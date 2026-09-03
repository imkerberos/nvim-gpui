use super::*;

struct PendingText {
    row: usize,
    grid_start: usize,
    grid_end: usize,
    text: String,
    runs: Vec<StyledTextRun>,
    // A Nerd/wide cell must remain a shaping boundary on both sides. Its
    // fallback glyph advance is not necessarily the terminal cell advance,
    // so ordinary text after it must start a fresh shaped line.
    mergeable: bool,
    in_viewport: bool,
}

struct PaintedText {
    line: ShapedLine,
    origin: gpui::Point<Pixels>,
    in_viewport: bool,
}

pub struct GridPrepaintState {
    backgrounds: Vec<(Bounds<Pixels>, Hsla, bool)>,
    overlines: Vec<(Bounds<Pixels>, Hsla, bool)>,
    texts: Vec<PaintedText>,
    viewport_bounds: Option<Bounds<Pixels>>,
}

type InputHandlerRegistrar = Box<dyn FnMut(Bounds<Pixels>, &mut Window, &mut App)>;

pub struct GridElement {
    model: Rc<GridModel>,
    nerd_font_mode: bool,
    nerd_fallback_mode: FallbackMode,
    cell_width: Pixels,
    line_height: Pixels,
    shaping_cache: SharedShapedLineCache,
    wide_font: Option<(String, Pixels)>,
    nerd_fallback_font: Option<(String, Pixels)>,
    default_background_override: Option<u32>,
    viewport_margins: (usize, usize, usize, usize),
    viewport_offset: gpui::Point<Pixels>,
    glyph_coverage_cache: SharedGlyphCoverageCache,
    cursor_blink_started_at: Instant,
    input_handler: Option<InputHandlerRegistrar>,
}

impl GridElement {
    pub fn with_shared_model(model: Rc<GridModel>) -> Self {
        Self {
            model,
            nerd_font_mode: false,
            nerd_fallback_mode: FallbackMode::Auto,
            cell_width: px(10.0),
            line_height: px(22.0),
            shaping_cache: ShapedLineCache::shared(),
            wide_font: None,
            nerd_fallback_font: None,
            default_background_override: None,
            viewport_margins: (0, 0, 0, 0),
            viewport_offset: point(px(0.0), px(0.0)),
            glyph_coverage_cache: GlyphCoverageCache::shared(),
            cursor_blink_started_at: Instant::now(),
            input_handler: None,
        }
    }

    pub fn with_nerd_font_mode(mut self, enabled: bool) -> Self {
        self.nerd_font_mode = enabled;
        self
    }

    pub fn with_nerd_fallback_mode(mut self, mode: FallbackMode) -> Self {
        self.nerd_fallback_mode = mode;
        self
    }

    pub fn with_metrics(mut self, cell_width: Pixels, line_height: Pixels) -> Self {
        self.cell_width = cell_width;
        self.line_height = line_height;
        self
    }

    pub fn with_wide_font(mut self, family: impl Into<String>, size: Pixels) -> Self {
        self.wide_font = Some((family.into(), size));
        self
    }

    pub fn with_nerd_fallback_font(mut self, family: impl Into<String>, size: Pixels) -> Self {
        let family = family.into();
        if !family.is_empty() {
            self.nerd_fallback_font = Some((family, size));
        }
        self
    }

    /// Use this background for cells in this grid whose highlight does not
    /// specify one. Multigrid floating windows need this because Neovim can
    /// represent their implicit `NormalFloat` fill with the default highlight
    /// id while still expecting the window surface to remain opaque.
    pub fn with_default_background(mut self, background: Option<u32>) -> Self {
        self.default_background_override = background;
        self
    }

    /// Keep the window's non-viewport margins fixed while the viewport is
    /// moved. Neovim uses these margins for elements such as winbars and
    /// floating-window borders.
    pub fn with_viewport_margins(mut self, top: u64, bottom: u64, left: u64, right: u64) -> Self {
        self.viewport_margins = (
            usize::try_from(top).unwrap_or(usize::MAX),
            usize::try_from(bottom).unwrap_or(usize::MAX),
            usize::try_from(left).unwrap_or(usize::MAX),
            usize::try_from(right).unwrap_or(usize::MAX),
        );
        self
    }

    /// Offset only the viewport portion of this grid. The outer grid remains
    /// stationary so borders and winbars do not move during smooth scrolling.
    pub fn with_viewport_offset(mut self, offset: gpui::Point<Pixels>) -> Self {
        self.viewport_offset = offset;
        self
    }

    pub fn with_glyph_coverage_cache(mut self, cache: SharedGlyphCoverageCache) -> Self {
        self.glyph_coverage_cache = cache;
        self
    }

    pub fn with_shaping_cache(mut self, cache: SharedShapedLineCache) -> Self {
        self.shaping_cache = cache;
        self
    }

    pub fn with_cursor_blink_started_at(mut self, started_at: Instant) -> Self {
        self.cursor_blink_started_at = started_at;
        self
    }

    pub fn with_input_handler(
        mut self,
        registrar: impl FnMut(Bounds<Pixels>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.input_handler = Some(Box::new(registrar));
        self
    }

    fn effective_cell_width(&self, window: &Window) -> Pixels {
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let font = text_style.font();

        window
            .text_system()
            .ch_advance(window.text_system().resolve_font(&font), font_size)
            .map(|advance| advance.max(px(1.0)))
            .unwrap_or(self.cell_width)
    }

    fn font_for_cell(
        &mut self,
        window: &Window,
        cell: &VisualCell,
        normal_font: &Font,
        normal_font_size: Pixels,
    ) -> (Font, Pixels) {
        let (base_font, size) = if cell.kind == VisualCellKind::WideCharacter {
            self.wide_font
                .as_ref()
                .map(|(family, size)| (font(family.clone()), *size))
                .unwrap_or_else(|| (normal_font.clone(), normal_font_size))
        } else {
            (normal_font.clone(), normal_font_size)
        };

        if cell.kind != VisualCellKind::NerdSymbol {
            return (base_font, size);
        }

        let Some(character) = cell.text.chars().next() else {
            return (base_font, size);
        };
        let Some((fallback_family, fallback_size)) = self.nerd_fallback_font.as_ref() else {
            return (base_font, size);
        };

        match self.nerd_fallback_mode {
            FallbackMode::None => return (base_font, size),
            FallbackMode::Auto => {
                if self
                    .glyph_coverage_cache
                    .borrow_mut()
                    .contains(window, &base_font, character)
                {
                    return (base_font, size);
                }
            }
            FallbackMode::Force => {}
        }

        // Keep the primary font as the requested face. GPUI's macOS and
        // Windows backends then pass this explicit cascade to CoreText or
        // DirectWrite, allowing the symbol-only font to supply just the
        // missing glyph. Resolving Symbols Nerd Font as a standalone font
        // would fail on GPUI 0.2.2 because that font intentionally has no
        // ordinary `m` glyph.
        let mut fallback_font = base_font;
        fallback_font.fallbacks = Some(FontFallbacks::from_fonts(vec![fallback_family.clone()]));
        (fallback_font, *fallback_size)
    }

    /// Shape the active cell again using the cursor foreground color.
    ///
    /// The cursor is painted by the editor-wide overlay so it can travel
    /// between grids. Repainting just this glyph after the cursor background
    /// keeps block cursors faithful to Neovim's foreground/background swap
    /// without putting a second cursor back into every GridElement.
    pub(crate) fn cursor_glyph(
        &mut self,
        window: &Window,
        position: CursorVisualPosition,
        foreground: Hsla,
    ) -> Option<CursorGlyph> {
        let row = self.model.rows().get(position.row)?;
        let cell = VisualCellBuilder::new(self.nerd_font_mode)
            .build_row(position.row, row)
            .into_iter()
            .find(|cell| {
                (cell.grid_start..cell.grid_start + cell.grid_len).contains(&position.col)
            })?;
        let attrs = self
            .model
            .highlight_ref(cell.highlight)
            .cloned()
            .unwrap_or_default();
        if cell.text.is_empty() || is_kitty_placeholder(&cell.text) || attrs.conceal {
            return None;
        }

        let text_style = window.text_style();
        let normal_font_size = text_style.font_size.to_pixels(window.rem_size());
        let normal_font = text_style.font();
        let (cell_font, cell_font_size) =
            self.font_for_cell(window, &cell, &normal_font, normal_font_size);
        let underline = (attrs.underline
            || attrs.undercurl
            || attrs.underdouble
            || attrs.underdotted
            || attrs.underdashed)
            .then(|| UnderlineStyle {
                thickness: px(1.0),
                color: Some(foreground),
                wavy: attrs.undercurl,
            });
        let strikethrough = attrs.strikethrough.then(|| StrikethroughStyle {
            thickness: px(1.0),
            color: Some(foreground),
        });
        let text_len = cell.text.len();
        let line = self.shaping_cache.borrow_mut().shape_line(
            window,
            cell.text,
            vec![StyledTextRun {
                len: text_len,
                style: ShapingStyle {
                    font: cell_font,
                    font_size: cell_font_size,
                    foreground,
                    underline,
                    strikethrough,
                },
            }],
        );
        Some(CursorGlyph { line })
    }

    fn viewport_row_range(&self) -> (usize, usize) {
        let height = self.model.height();
        let top = self.viewport_margins.0.min(height);
        let bottom = self.viewport_margins.1.min(height.saturating_sub(top));
        (top, height.saturating_sub(bottom))
    }

    fn viewport_column_range(&self) -> (usize, usize) {
        let width = self.model.width();
        let left = self.viewport_margins.2.min(width);
        let right = self.viewport_margins.3.min(width.saturating_sub(left));
        (left, width.saturating_sub(right))
    }

    fn cell_is_in_viewport(&self, row: usize, column: usize) -> bool {
        let (top, bottom) = self.viewport_row_range();
        let (left, right) = self.viewport_column_range();
        (top..bottom).contains(&row) && (left..right).contains(&column)
    }

    fn offset_for_cell(&self, row: usize, column: usize) -> gpui::Point<Pixels> {
        if self.cell_is_in_viewport(row, column) {
            self.viewport_offset
        } else {
            point(px(0.0), px(0.0))
        }
    }

    fn viewport_bounds(
        &self,
        bounds: Bounds<Pixels>,
        cell_width: Pixels,
    ) -> Option<Bounds<Pixels>> {
        let (top, bottom) = self.viewport_row_range();
        let (left, right) = self.viewport_column_range();
        (top < bottom && left < right).then(|| {
            Bounds::new(
                point(
                    bounds.origin.x + cell_width * left,
                    bounds.origin.y + self.line_height * top,
                ),
                size(
                    cell_width * (right - left),
                    self.line_height * (bottom - top),
                ),
            )
        })
    }
}

impl IntoElement for GridElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for GridElement {
    type RequestLayoutState = ();
    type PrepaintState = GridPrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = (self.effective_cell_width(window) * self.model.width()).into();
        style.size.height = (self.line_height * self.model.height()).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let text_style = window.text_style();
        let normal_font_size = text_style.font_size.to_pixels(window.rem_size());
        let normal_font = text_style.font();
        let cell_width = self.effective_cell_width(window);
        let builder = VisualCellBuilder::new(self.nerd_font_mode);
        let model = Rc::clone(&self.model);
        let now = Instant::now();
        let mut has_blinking_text = false;
        let mut backgrounds = Vec::new();
        let mut overlines = Vec::new();
        let mut text_groups = Vec::new();
        let mut pending_text: Option<PendingText> = None;

        builder.for_each_cell(model.as_ref(), |cell| {
            let in_viewport = self.cell_is_in_viewport(cell.row, cell.grid_start);
            let attrs = model
                .highlight_ref(cell.highlight)
                .map(Cow::Borrowed)
                .unwrap_or_else(|| Cow::Owned(HighlightAttrs::default()));
            let (foreground, background) = highlight_colors(
                model.as_ref(),
                cell.highlight,
                self.default_background_override,
            );
            if attrs.blink {
                has_blinking_text = true;
            }
            let style = if cell.text.is_empty()
                || is_kitty_placeholder(&cell.text)
                || attrs.conceal
                || (attrs.blink && !blink_visible(self.cursor_blink_started_at, now, 0, 500, 500))
            {
                None
            } else {
                let (cell_font, cell_font_size) =
                    self.font_for_cell(window, &cell, &normal_font, normal_font_size);
                let cell_font = if attrs.italic {
                    cell_font.italic()
                } else {
                    cell_font
                };
                let cell_font = if attrs.bold {
                    cell_font.bold()
                } else {
                    cell_font
                };
                let underline = (attrs.underline
                    || attrs.undercurl
                    || attrs.underdouble
                    || attrs.underdotted
                    || attrs.underdashed)
                    .then(|| UnderlineStyle {
                        thickness: px(1.0),
                        color: attrs
                            .special
                            .or(attrs.foreground)
                            .map(|color| rgb(color).into()),
                        wavy: attrs.undercurl,
                    });
                let strikethrough = attrs.strikethrough.then(|| StrikethroughStyle {
                    thickness: px(1.0),
                    color: attrs
                        .special
                        .or(attrs.foreground)
                        .map(|color| rgb(color).into()),
                });
                Some(ShapingStyle {
                    font: cell_font,
                    font_size: cell_font_size,
                    foreground,
                    underline,
                    strikethrough,
                })
            };
            let origin = point(
                bounds.origin.x + cell_width * cell.grid_start,
                bounds.origin.y + self.line_height * cell.row,
            ) + self.offset_for_cell(cell.row, cell.grid_start);
            let cell_bounds =
                Bounds::new(origin, size(cell_width * cell.grid_len, self.line_height));
            let overline = attrs.overline.then(|| {
                let overline_color = attrs
                    .special
                    .map(|color| rgb(color).into())
                    .unwrap_or(foreground);
                (
                    Bounds::new(
                        point(cell_bounds.origin.x, cell_bounds.origin.y + px(1.0)),
                        size(cell_bounds.size.width, px(1.0)),
                    ),
                    overline_color,
                )
            });

            if let Some(style) = style {
                let can_merge = cell.kind == VisualCellKind::Text
                    && cell.grid_len == 1
                    && pending_text.as_ref().is_some_and(|pending| {
                        pending.mergeable
                            && pending.in_viewport == in_viewport
                            && pending.row == cell.row
                            && pending.grid_end == cell.grid_start
                    });
                if can_merge {
                    let pending = pending_text
                        .as_mut()
                        .expect("a mergeable cell must have pending text");
                    pending.text.push_str(&cell.text);
                    pending.grid_end = cell.grid_start + cell.grid_len;
                    let text_len = cell.text.len();
                    match pending.runs.last_mut() {
                        Some(last) if last.style == style => last.len += text_len,
                        _ => pending.runs.push(StyledTextRun {
                            len: text_len,
                            style,
                        }),
                    }
                } else {
                    if let Some(pending) = pending_text.take() {
                        text_groups.push(pending);
                    }
                    pending_text = Some(PendingText {
                        row: cell.row,
                        grid_start: cell.grid_start,
                        grid_end: cell.grid_start + cell.grid_len,
                        text: cell.text.to_string(),
                        runs: vec![StyledTextRun {
                            len: cell.text.len(),
                            style,
                        }],
                        mergeable: cell.kind == VisualCellKind::Text,
                        in_viewport,
                    });
                }
            } else if let Some(pending) = pending_text.take() {
                text_groups.push(pending);
            }

            // Terminal cells are positioned from their leading edge. Do
            // not center a shaped glyph inside the cell: the extra
            // padding creates visible gaps between adjacent ASCII-art
            // glyphs, whose raster width is often smaller than the cell
            // advance.
            if let Some(background) = background {
                push_background(&mut backgrounds, cell_bounds, background, in_viewport);
            }
            if let Some(overline) = overline {
                overlines.push((overline.0, overline.1, in_viewport));
            }
        });

        if let Some(pending) = pending_text {
            text_groups.push(pending);
        }

        let texts = text_groups
            .into_iter()
            .map(|pending| {
                let origin = point(
                    bounds.origin.x + cell_width * pending.grid_start,
                    bounds.origin.y + self.line_height * pending.row,
                ) + self.offset_for_cell(pending.row, pending.grid_start);
                let text: SharedString = pending.text.into();
                let line = self
                    .shaping_cache
                    .borrow_mut()
                    .shape_line(window, text, pending.runs);
                PaintedText {
                    line,
                    origin,
                    in_viewport: pending.in_viewport,
                }
            })
            .collect();

        if has_blinking_text {
            window.request_animation_frame();
        }
        if self.viewport_offset.x != px(0.0) || self.viewport_offset.y != px(0.0) {
            window.request_animation_frame();
        }

        GridPrepaintState {
            backgrounds,
            overlines,
            texts,
            viewport_bounds: self.viewport_bounds(bounds, cell_width),
        }
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(input_handler) = self.input_handler.as_mut() {
            input_handler(bounds, window, cx);
        }

        for (bounds, background, in_viewport) in &prepaint.backgrounds {
            let mask = (*in_viewport).then(|| gpui::ContentMask {
                bounds: prepaint.viewport_bounds.unwrap_or(*bounds),
            });
            window.with_content_mask(mask, |window| {
                window.paint_quad(fill(*bounds, *background));
            });
        }
        for (bounds, color, in_viewport) in &prepaint.overlines {
            let mask = (*in_viewport).then(|| gpui::ContentMask {
                bounds: prepaint.viewport_bounds.unwrap_or(*bounds),
            });
            window.with_content_mask(mask, |window| {
                window.paint_quad(fill(*bounds, *color));
            });
        }

        // Keep the terminal's cell coordinates for placement, but do not clip
        // every glyph to its individual cell. GPUI's glyph raster bounds can
        // extend past the logical cell (especially for ASCII art, Nerd Font
        // symbols, and fonts with a generous ascent/descent). Per-cell masks
        // turn that overhang into visible seams at grid boundaries. The Grid
        // itself remains clipped so text and an elastic cursor cannot escape
        // the Neovim viewport.
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for painted_text in prepaint.texts.drain(..) {
                let mask = painted_text.in_viewport.then(|| gpui::ContentMask {
                    bounds: prepaint.viewport_bounds.unwrap_or(bounds),
                });
                window.with_content_mask(mask, |window| {
                    painted_text
                        .line
                        .paint(painted_text.origin, self.line_height, window, cx)
                        .expect("failed to paint grid text");
                });
            }
        });
    }
}

pub(crate) struct CursorGlyph {
    line: ShapedLine,
}

/// A cursor that is positioned in editor-wide screen coordinates.
///
/// GridElement keeps the cursor attached to the grid that owns it, which is
/// correct for a stationary cursor. During a cross-window move, however, the
/// cursor must travel between grid bounds. This small overlay lets the app
/// render that transition without duplicating a cursor in either grid.
pub struct CursorElement {
    position: CursorVisualPosition,
    local_position: CursorVisualPosition,
    animation: Option<CursorAnimation>,
    color: Hsla,
    glyph_foreground: Hsla,
    glyph_source: Option<GridElement>,
    cell_width: Pixels,
    line_height: Pixels,
    width: usize,
    height: usize,
    cursor_mode: CursorModeInfo,
    blink_started_at: Instant,
}

impl CursorElement {
    pub(crate) fn new(
        position: CursorVisualPosition,
        color: Hsla,
        cursor_mode: CursorModeInfo,
    ) -> Self {
        Self {
            position,
            local_position: position,
            animation: None,
            color,
            glyph_foreground: color,
            glyph_source: None,
            cell_width: px(10.0),
            line_height: px(22.0),
            width: 0,
            height: 0,
            cursor_mode,
            blink_started_at: Instant::now(),
        }
    }

    pub(crate) fn with_animation(mut self, animation: Option<CursorAnimation>) -> Self {
        self.animation = animation;
        self
    }

    pub(crate) fn with_local_position(mut self, position: CursorVisualPosition) -> Self {
        self.local_position = position;
        self
    }

    pub(crate) fn with_glyph_foreground(mut self, foreground: Hsla) -> Self {
        self.glyph_foreground = foreground;
        self
    }

    pub(crate) fn with_glyph_source(mut self, source: Option<GridElement>) -> Self {
        self.glyph_source = source;
        self
    }

    pub(crate) fn with_metrics(mut self, cell_width: Pixels, line_height: Pixels) -> Self {
        self.cell_width = cell_width;
        self.line_height = line_height;
        self
    }

    pub(crate) fn with_grid_size(mut self, width: usize, height: usize) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    pub(crate) fn with_blink_started_at(mut self, started_at: Instant) -> Self {
        self.blink_started_at = started_at;
        self
    }
}

pub struct CursorTrail {
    bounds: Bounds<Pixels>,
    opacity: f32,
}

pub struct CursorPrepaintState {
    trails: Vec<CursorTrail>,
    glyph_position: Option<CursorVisualPositionF>,
    glyph: Option<ShapedLine>,
}

impl IntoElement for CursorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for CursorElement {
    type RequestLayoutState = ();
    type PrepaintState = CursorPrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = (self.cell_width * self.width).into();
        style.size.height = (self.line_height * self.height).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let now = Instant::now();
        if self.cursor_mode.blink_enabled()
            && !self.cursor_mode.visible_at(self.blink_started_at, now)
        {
            window.request_animation_frame();
            return CursorPrepaintState {
                trails: Vec::new(),
                glyph_position: None,
                glyph: None,
            };
        }

        let (trails, glyph_position) =
            if let Some(animation) = self.animation.filter(|animation| animation.is_active(now)) {
                window.request_animation_frame();

                const TRAIL: [(u64, f32); 5] =
                    [(56, 0.05), (42, 0.08), (28, 0.13), (14, 0.22), (0, 1.0)];
                let trails = TRAIL
                    .into_iter()
                    .map(|(age_ms, opacity)| {
                        let sample_time = now
                            .checked_sub(Duration::from_millis(age_ms))
                            .unwrap_or(animation.started_at);
                        CursorTrail {
                            bounds: animated_cursor_bounds(
                                bounds,
                                self.cell_width,
                                self.line_height,
                                animation,
                                self.cursor_mode,
                                sample_time,
                            ),
                            opacity,
                        }
                    })
                    .collect::<Vec<_>>();
                (trails, Some(animation.position_at(now)))
            } else {
                (
                    vec![CursorTrail {
                        bounds: cursor_bounds_at(
                            bounds,
                            self.cell_width,
                            self.line_height,
                            self.position.into(),
                            self.cursor_mode,
                        ),
                        opacity: 1.0,
                    }],
                    Some(self.position.into()),
                )
            };

        let glyph = (self.cursor_mode.shape == CursorShape::Block)
            .then(|| {
                self.glyph_source.as_mut()?.cursor_glyph(
                    window,
                    self.local_position,
                    self.glyph_foreground,
                )
            })
            .flatten()
            .map(|glyph| glyph.line);

        CursorPrepaintState {
            trails,
            glyph_position,
            glyph,
        }
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for trail in prepaint.trails.drain(..) {
                let radius = px((f32::from(trail.bounds.size.width)
                    .min(f32::from(trail.bounds.size.height))
                    .mul_add(0.18, 0.0))
                .clamp(2.0, 6.0));
                window.paint_quad(
                    fill(trail.bounds, self.color.opacity(trail.opacity))
                        .corner_radii(Corners::all(radius)),
                );
            }

            if let (Some(glyph), Some(position)) = (prepaint.glyph.take(), prepaint.glyph_position)
            {
                let origin = point(
                    bounds.origin.x + self.cell_width * position.col,
                    bounds.origin.y + self.line_height * position.row,
                );
                glyph
                    .paint(origin, self.line_height, window, _cx)
                    .expect("failed to paint cursor glyph");
            }
        });
    }
}

#[cfg(test)]
pub(super) fn cursor_bounds(
    grid_bounds: Bounds<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    position: CursorVisualPosition,
    mode: CursorModeInfo,
) -> Bounds<Pixels> {
    cursor_bounds_at(grid_bounds, cell_width, line_height, position.into(), mode)
}

pub(crate) fn cursor_colors(
    model: &GridModel,
    position: CursorVisualPosition,
    mode: CursorModeInfo,
) -> (Hsla, Hsla) {
    let default_colors = highlight_colors(model, DEFAULT_HIGHLIGHT, None);
    let default_background = default_colors
        .1
        .unwrap_or_else(|| rgb(DEFAULT_BACKGROUND).into());
    let cell_highlight = model
        .rows()
        .get(position.row)
        .and_then(|row| row.cells().get(position.col))
        .map(|cell| cell.highlight)
        .unwrap_or(DEFAULT_HIGHLIGHT);

    match mode.attr_id {
        // Neovim defines attr_id 0 as a request to swap the current cell's
        // foreground and background, rather than as a normal highlight id.
        Some(DEFAULT_HIGHLIGHT) => {
            let (cell_foreground, cell_background) = highlight_colors(model, cell_highlight, None);
            (cell_background.unwrap_or(default_colors.0), cell_foreground)
        }
        Some(attr_id) => {
            let (foreground, background) = highlight_colors(model, attr_id, None);
            (foreground, background.unwrap_or(default_background))
        }
        None => (default_background, rgb(BLUE_FOREGROUND).into()),
    }
}

fn animated_cursor_bounds(
    grid_bounds: Bounds<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    animation: CursorAnimation,
    mode: CursorModeInfo,
    now: Instant,
) -> Bounds<Pixels> {
    let progress = animation.progress(now);
    let position = animation.position_at(now);
    let from = animation.from;
    let to = animation.to;

    let base = cursor_bounds_at(grid_bounds, cell_width, line_height, position, mode);
    if progress >= 1.0 {
        return base;
    }

    // Estimate the instantaneous velocity from two nearby animation samples.
    // The velocity, rather than only the total distance, makes a short key
    // press feel soft and makes a large jump visibly stretch at launch.
    let previous_time = now
        .checked_sub(Duration::from_millis(8))
        .unwrap_or(animation.started_at);
    let previous_position = animation.position_at(previous_time);
    let interval = 0.008;
    let velocity_col = (position.col - previous_position.col) / interval;
    let velocity_row = (position.row - previous_position.row) / interval;
    let delta_x = (to.col - from.col) * f32::from(cell_width);
    let delta_y = (to.row - from.row) * f32::from(line_height);
    let distance = (delta_x / f32::from(cell_width))
        .abs()
        .max((delta_y / f32::from(line_height)).abs());
    if distance == 0.0 && velocity_col == 0.0 && velocity_row == 0.0 {
        return base;
    }

    // A jelly cursor stretches with its current speed and relaxes towards the
    // target. The distance term keeps a jump between split windows readable,
    // while the cap prevents a redraw storm from producing a huge cursor.
    let velocity = velocity_col.hypot(velocity_row);
    let speed_factor = (velocity / 12.0).clamp(0.0, 1.0);
    let distance_factor = (distance / 8.0).clamp(0.0, 1.0);
    let settle = (PI * progress).sin().max(0.0);
    let stretch_ratio =
        (0.055 + 0.22 * speed_factor + 0.06 * distance_factor + 0.025 * settle).min(0.4);

    let base_width = f32::from(base.size.width);
    let base_height = f32::from(base.size.height);
    let horizontal = velocity_col.abs().max(delta_x.abs()) >= velocity_row.abs().max(delta_y.abs());
    let (x, y, width, height) = if horizontal {
        let extra = f32::from(cell_width) * stretch_ratio;
        let height = (base_height * (1.0 - stretch_ratio * 0.42)).max(1.0);
        let direction = if velocity_col.abs() > 0.001 {
            velocity_col
        } else {
            delta_x
        };
        (
            f32::from(base.origin.x) - if direction > 0.0 { extra } else { 0.0 },
            f32::from(base.origin.y) + (base_height - height) / 2.0,
            base_width + extra,
            height,
        )
    } else {
        let extra = f32::from(line_height) * stretch_ratio;
        let width = (base_width * (1.0 - stretch_ratio * 0.42)).max(1.0);
        let direction = if velocity_row.abs() > 0.001 {
            velocity_row
        } else {
            delta_y
        };
        (
            f32::from(base.origin.x) + (base_width - width) / 2.0,
            f32::from(base.origin.y) - if direction > 0.0 { extra } else { 0.0 },
            width,
            base_height + extra,
        )
    };

    Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
}

pub(super) fn jelly_progress(progress: f32) -> f32 {
    if progress >= 1.0 {
        return 1.0;
    }

    // A restrained ease-out-back curve: the cursor settles a few percent
    // past its destination and returns, which reads as a soft jelly motion
    // rather than a rigid linear slide. The animation is still clamped to a
    // small overshoot so a large cursor jump cannot leave the viewport.
    let x = progress - 1.0;
    let overshoot = 0.75;
    let curve = 1.0 + (overshoot + 1.0) * x.powi(3) + overshoot * x.powi(2);
    curve.clamp(0.0, 1.025)
}

fn cursor_bounds_at(
    grid_bounds: Bounds<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    position: CursorVisualPositionF,
    mode: CursorModeInfo,
) -> Bounds<Pixels> {
    let percentage = f32::from(mode.cell_percentage) / 100.0;
    let origin = point(
        grid_bounds.origin.x + cell_width * position.col,
        grid_bounds.origin.y + line_height * position.row,
    );
    let full_width = cell_width * position.width.max(1.0);

    let (origin, size) = match mode.shape {
        CursorShape::Block => (origin, size(full_width, line_height)),
        CursorShape::Horizontal => (
            point(origin.x, origin.y + line_height * (1.0 - percentage)),
            size(full_width, line_height * percentage),
        ),
        CursorShape::Vertical => (origin, size(full_width * percentage, line_height)),
    };

    Bounds::new(origin, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_margins_keep_outer_cells_fixed() {
        let element = GridElement::with_shared_model(Rc::new(GridModel::new(8, 4)))
            .with_viewport_margins(1, 1, 2, 2)
            .with_viewport_offset(point(px(0.0), px(10.0)));

        assert_eq!(element.viewport_row_range(), (1, 3));
        assert_eq!(element.viewport_column_range(), (2, 6));
        assert!(element.cell_is_in_viewport(1, 2));
        assert!(!element.cell_is_in_viewport(0, 2));
        assert!(!element.cell_is_in_viewport(1, 1));
        assert_eq!(element.offset_for_cell(1, 2), point(px(0.0), px(10.0)));
        assert_eq!(element.offset_for_cell(0, 2), point(px(0.0), px(0.0)));
    }

    #[test]
    fn moving_cursor_stretches_towards_its_previous_position() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(400.0), px(200.0)));
        let mode = CursorModeInfo::default();
        let animation = CursorAnimation::new(
            CursorVisualPosition {
                row: 0,
                col: 1,
                width: 1,
            },
            CursorVisualPosition {
                row: 0,
                col: 8,
                width: 1,
            },
        );
        let now = animation.started_at + Duration::from_millis(24);
        let position = animation.position_at(now);
        let base = cursor_bounds_at(bounds, px(10.0), px(20.0), position, mode);
        let stretched = animated_cursor_bounds(bounds, px(10.0), px(20.0), animation, mode, now);

        assert!(stretched.size.width > base.size.width);
        assert!(stretched.size.height < base.size.height);
        assert!(stretched.origin.x < base.origin.x);
    }
}
