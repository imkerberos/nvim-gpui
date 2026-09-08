use super::cursor::CursorGlyph;
use super::*;

struct PendingText {
    row: usize,
    render_start: usize,
    render_end: usize,
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

struct ImePaintedText {
    row: usize,
    col: usize,
    grid_width: usize,
    line: ShapedLine,
    in_viewport: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ImeGap {
    row: usize,
    col: usize,
    width: usize,
}

impl ImeGap {
    fn shifted_column(self, row: usize, column: usize) -> usize {
        if row == self.row && column >= self.col {
            column.saturating_add(self.width)
        } else {
            column
        }
    }

    fn end(self) -> usize {
        self.col.saturating_add(self.width)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GridCellRange {
    rows: (usize, usize),
    columns: (usize, usize),
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
    highlight_context: HighlightContext,
    viewport_margins: (usize, usize, usize, usize),
    viewport_offset: gpui::Point<Pixels>,
    glyph_coverage_cache: SharedGlyphCoverageCache,
    cursor_blink_started_at: Instant,
    input_handler: Option<InputHandlerRegistrar>,
    ime_composition: Option<ImeComposition>,
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
            highlight_context: HighlightContext::Main,
            viewport_margins: (0, 0, 0, 0),
            viewport_offset: point(px(0.0), px(0.0)),
            glyph_coverage_cache: GlyphCoverageCache::shared(),
            cursor_blink_started_at: Instant::now(),
            input_handler: None,
            ime_composition: None,
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

    /// Resolve highlights using the semantic layer that owns this grid.
    pub fn with_highlight_context(mut self, context: HighlightContext) -> Self {
        self.highlight_context = context;
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

    pub fn with_ime_composition(mut self, composition: Option<ImeComposition>) -> Self {
        self.ime_composition = composition;
        self
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
        let resolved = resolve_highlight(&self.model, cell.highlight, self.highlight_context);
        let attrs = resolved.attrs;
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

    /// Convert the active GPUI content mask into logical grid coordinates.
    /// Keep one cell of overscan on each side so glyph overhang and a wide
    /// character whose lead starts just outside the clip are still prepared.
    fn render_range(
        &self,
        bounds: Bounds<Pixels>,
        content_bounds: Bounds<Pixels>,
    ) -> GridCellRange {
        let clipped = bounds.intersect(&content_bounds);
        GridCellRange {
            rows: clipped_cell_range(
                bounds.origin.y,
                clipped.origin.y,
                clipped.size.height,
                self.line_height,
                self.model.height(),
            ),
            columns: clipped_cell_range(
                bounds.origin.x,
                clipped.origin.x,
                clipped.size.width,
                self.cell_width,
                self.model.width(),
            ),
        }
    }
}

fn clipped_cell_range(
    grid_origin: Pixels,
    clipped_origin: Pixels,
    clipped_size: Pixels,
    cell_size: Pixels,
    cell_count: usize,
) -> (usize, usize) {
    if cell_count == 0 || f32::from(cell_size) <= 0.0 || f32::from(clipped_size) <= 0.0 {
        return (0, 0);
    }

    let start = f32::from(clipped_origin - grid_origin);
    let end = start + f32::from(clipped_size);
    let cell_size = f32::from(cell_size);
    let first = (start / cell_size).floor() as isize - 1;
    let last = (end / cell_size).ceil() as isize + 1;
    let max = cell_count as isize;
    let first = first.clamp(0, max) as usize;
    let last = last.clamp(first as isize, max) as usize;
    (first, last)
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
        style.size.width = (self.cell_width * self.model.width()).into();
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
        let cell_width = self.cell_width;
        let builder = VisualCellBuilder::new(self.nerd_font_mode);
        let model = Rc::clone(&self.model);
        let now = Instant::now();
        let render_range = self.render_range(bounds, window.content_mask().bounds);
        let mut resolved_highlights = HashMap::new();
        let mut has_blinking_text = false;
        let mut backgrounds = Vec::new();
        let mut ime_gap_backgrounds = Vec::new();
        let mut overlines = Vec::new();
        let mut text_groups = Vec::new();
        let mut pending_text: Option<PendingText> = None;
        // Inline composition contributes glyphs only. Its background must
        // remain the background of the underlying grid cell (for example,
        // CursorLine), including after the composition is cleared.
        let ime_style =
            resolve_highlight(model.as_ref(), DEFAULT_HIGHLIGHT, self.highlight_context);
        let ime_foreground = ime_style.foreground;
        let ime_paint = self.ime_composition.as_ref().and_then(|composition| {
            if composition.text.is_empty()
                || composition.row >= model.height()
                || composition.col > model.width()
            {
                return None;
            }

            // The cell under the cursor may be Neovim virtual text and carry
            // a decoration-specific highlight. IME preedit is client-side
            // input, so it must use the grid's normal text attributes instead
            // of inheriting that cell's virtual-text color.
            let attrs = ime_style.attrs.clone();
            let text = composition.text.clone();
            let marked_start = composition.marked_range.start.min(text.len());
            let marked_end = composition
                .marked_range
                .end
                .min(text.len())
                .max(marked_start);
            let make_style = |underline| ShapingStyle {
                font: normal_font.clone(),
                font_size: normal_font_size,
                foreground: ime_foreground,
                underline,
                strikethrough: attrs.strikethrough.then(|| StrikethroughStyle {
                    thickness: px(1.0),
                    color: Some(ime_style.special),
                }),
            };
            let marked_style = Some(UnderlineStyle {
                thickness: px(1.0),
                color: Some(ime_foreground),
                wavy: false,
            });
            let plain_style = make_style(None);
            let marked_style = make_style(marked_style);
            let mut runs = Vec::with_capacity(3);
            if marked_start > 0 {
                runs.push(StyledTextRun {
                    len: marked_start,
                    style: plain_style.clone(),
                });
            }
            runs.push(StyledTextRun {
                len: marked_end - marked_start,
                style: marked_style,
            });
            if marked_end < text.len() {
                runs.push(StyledTextRun {
                    len: text.len() - marked_end,
                    style: plain_style,
                });
            }
            let line = self
                .shaping_cache
                .borrow_mut()
                .shape_line(window, text, runs);
            Some(ImePaintedText {
                row: composition.row,
                col: composition.col,
                grid_width: composition.grid_width.max(1),
                line,
                in_viewport: composition.col < model.width()
                    && self.cell_is_in_viewport(composition.row, composition.col),
            })
        });
        let ime_gap = ime_paint.as_ref().map(|ime| ImeGap {
            row: ime.row,
            col: ime.col,
            width: ime.grid_width,
        });
        let mut ime_gap_background_covered = ime_gap
            .map(|gap| vec![false; gap.width])
            .unwrap_or_default();

        let mut paint_cell = |cell: VisualCell| {
            let render_start = ime_gap
                .map(|gap| gap.shifted_column(cell.row, cell.grid_start))
                .unwrap_or(cell.grid_start);
            let in_viewport = self.cell_is_in_viewport(cell.row, render_start);
            let resolved = resolved_highlights
                .entry(cell.highlight)
                .or_insert_with(|| {
                    resolve_highlight(model.as_ref(), cell.highlight, self.highlight_context)
                });
            let attrs = &resolved.attrs;
            let foreground = resolved.foreground;
            let background = resolved.background;
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
                        color: Some(resolved.special),
                        wavy: attrs.undercurl,
                    });
                let strikethrough = attrs.strikethrough.then(|| StrikethroughStyle {
                    thickness: px(1.0),
                    color: Some(resolved.special),
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
                bounds.origin.x + cell_width * render_start,
                bounds.origin.y + self.line_height * cell.row,
            ) + self.offset_for_cell(cell.row, render_start);
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
                            && pending.render_end == render_start
                    });
                if can_merge {
                    let pending = pending_text
                        .as_mut()
                        .expect("a mergeable cell must have pending text");
                    pending.text.push_str(&cell.text);
                    pending.render_end = render_start + cell.grid_len;
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
                        render_start,
                        render_end: render_start + cell.grid_len,
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
                if let Some(gap) = ime_gap {
                    let cell_end = cell.grid_start.saturating_add(cell.grid_len);
                    let gap_start = cell.grid_start.max(gap.col);
                    let gap_end = cell_end.min(gap.end());
                    if cell.row == gap.row && gap_start < gap_end {
                        let gap_in_viewport = self.cell_is_in_viewport(cell.row, gap_start);
                        let gap_origin = point(
                            bounds.origin.x + cell_width * gap_start,
                            bounds.origin.y + self.line_height * cell.row,
                        ) + self.offset_for_cell(cell.row, gap_start);
                        let gap_bounds = Bounds::new(
                            gap_origin,
                            size(cell_width * (gap_end - gap_start), self.line_height),
                        );
                        push_background(
                            &mut ime_gap_backgrounds,
                            gap_bounds,
                            background,
                            gap_in_viewport,
                        );
                        for column in gap_start..gap_end {
                            if let Some(covered) =
                                ime_gap_background_covered.get_mut(column.saturating_sub(gap.col))
                            {
                                *covered = true;
                            }
                        }
                    }
                }
            }
            if let Some(overline) = overline {
                overlines.push((overline.0, overline.1, in_viewport));
            }
        };
        builder.for_each_cell_in_range(
            model.as_ref(),
            render_range.rows.0..render_range.rows.1,
            render_range.columns.0..render_range.columns.1,
            &mut paint_cell,
        );

        if let Some(gap) = ime_gap {
            // Usually every gap cell gets its background from the original
            // grid cell that was moved to the right. At the end of a line, or
            // while a clipped render range is active, there may be no source
            // cell; use the normal grid background for that uncovered part.
            let fallback_background = ime_style
                .background
                .unwrap_or_else(|| rgb(DEFAULT_BACKGROUND).into());
            for (offset, covered) in ime_gap_background_covered.iter().enumerate() {
                if *covered {
                    continue;
                }
                let column = gap.col.saturating_add(offset);
                let in_viewport = self.cell_is_in_viewport(gap.row, column);
                let origin = point(
                    bounds.origin.x + cell_width * column,
                    bounds.origin.y + self.line_height * gap.row,
                ) + self.offset_for_cell(gap.row, column);
                ime_gap_backgrounds.push((
                    Bounds::new(origin, size(cell_width, self.line_height)),
                    fallback_background,
                    in_viewport,
                ));
            }
        }
        backgrounds.extend(ime_gap_backgrounds);

        if let Some(pending) = pending_text {
            text_groups.push(pending);
        }

        let mut texts: Vec<PaintedText> = text_groups
            .into_iter()
            .map(|pending| {
                let origin = point(
                    bounds.origin.x + cell_width * pending.render_start,
                    bounds.origin.y + self.line_height * pending.row,
                ) + self.offset_for_cell(pending.row, pending.render_start);
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

        if let Some(ime) = ime_paint {
            let origin = point(
                bounds.origin.x + cell_width * ime.col,
                bounds.origin.y + self.line_height * ime.row,
            ) + self.offset_for_cell(ime.row, ime.col);
            texts.push(PaintedText {
                line: ime.line,
                origin,
                in_viewport: ime.in_viewport,
            });
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ime_gap_shifts_only_the_composition_row_after_its_anchor() {
        let gap = ImeGap {
            row: 2,
            col: 4,
            width: 3,
        };

        assert_eq!(gap.shifted_column(1, 7), 7);
        assert_eq!(gap.shifted_column(2, 3), 3);
        assert_eq!(gap.shifted_column(2, 4), 7);
        assert_eq!(gap.shifted_column(2, 9), 12);
    }

    #[test]
    fn ime_gap_end_saturates_for_large_compositions() {
        let gap = ImeGap {
            row: 0,
            col: usize::MAX - 1,
            width: 4,
        };

        assert_eq!(gap.end(), usize::MAX);
        assert_eq!(gap.shifted_column(0, usize::MAX), usize::MAX);
    }

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
    fn clipped_cell_range_keeps_partial_cells_with_one_cell_overscan() {
        assert_eq!(
            clipped_cell_range(px(0.0), px(25.0), px(50.0), px(10.0), 10),
            (1, 9)
        );
        assert_eq!(
            clipped_cell_range(px(0.0), px(0.0), px(0.0), px(10.0), 10),
            (0, 0)
        );
    }

    #[test]
    fn render_range_intersects_grid_and_content_bounds() {
        let element = GridElement::with_shared_model(Rc::new(GridModel::new(10, 8)))
            .with_metrics(px(10.0), px(20.0));
        let bounds = Bounds::new(point(px(100.0), px(50.0)), size(px(100.0), px(160.0)));
        let content_bounds = Bounds::new(point(px(120.0), px(90.0)), size(px(50.0), px(60.0)));

        assert_eq!(
            element.render_range(bounds, content_bounds),
            GridCellRange {
                rows: (1, 6),
                columns: (1, 8),
            }
        );
    }
}
