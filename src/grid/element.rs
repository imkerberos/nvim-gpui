use super::cursor::CursorGlyph;
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

struct ImePaintedText {
    row: usize,
    col: usize,
    cell_end: usize,
    line: ShapedLine,
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
            let cell_len = ((f32::from(line.width) / f32::from(cell_width)).ceil() as usize).max(1);

            Some(ImePaintedText {
                row: composition.row,
                col: composition.col,
                cell_end: composition.col.saturating_add(cell_len),
                line,
                in_viewport: composition.col < model.width()
                    && self.cell_is_in_viewport(composition.row, composition.col),
            })
        });
        let ime_span = ime_paint
            .as_ref()
            .map(|ime| (ime.row, ime.col, ime.cell_end));

        builder.for_each_cell(model.as_ref(), |cell| {
            let ime_overlaps = ime_span.is_some_and(|(row, start, end)| {
                cell.row == row && cell.grid_start < end && start < cell.grid_start + cell.grid_len
            });
            let in_viewport = self.cell_is_in_viewport(cell.row, cell.grid_start);
            let resolved =
                resolve_highlight(model.as_ref(), cell.highlight, self.highlight_context);
            let attrs = &resolved.attrs;
            let foreground = resolved.foreground;
            let background = resolved.background;
            if attrs.blink {
                has_blinking_text = true;
            }
            let style = if ime_overlaps
                || cell.text.is_empty()
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
            if !ime_overlaps {
                if let Some(overline) = overline {
                    overlines.push((overline.0, overline.1, in_viewport));
                }
            }
        });

        if let Some(pending) = pending_text {
            text_groups.push(pending);
        }

        let mut texts: Vec<PaintedText> = text_groups
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
}
