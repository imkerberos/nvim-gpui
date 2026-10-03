use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualCellKind {
    Text,
    WideCharacter,
    NerdSymbol,
}

/// One independently positioned visual unit in the Neovim grid.
///
/// Normal cells contain one grapheme cluster. A wide-character lead is the
/// only unit that may cover more than one logical cell; its continuation is
/// kept in the model but does not paint a second glyph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualCell {
    pub row: usize,
    pub grid_start: usize,
    pub grid_len: usize,
    pub text: SharedString,
    pub highlight: HighlightId,
    pub kind: VisualCellKind,
}

#[cfg(test)]
pub(super) fn visual_cell_overlaps_cursor(cell: &VisualCell, cursor: CursorVisualPosition) -> bool {
    if cell.row != cursor.row {
        return false;
    }

    let cell_end = cell.grid_start + cell.grid_len;
    let cursor_end = cursor.col + cursor.width;
    cell.grid_start < cursor_end && cursor.col < cell_end
}

pub struct VisualCellBuilder {
    nerd_font_mode: bool,
}

impl VisualCellBuilder {
    pub fn new(nerd_font_mode: bool) -> Self {
        Self { nerd_font_mode }
    }

    pub fn for_each_cell(&self, model: &GridModel, f: impl FnMut(VisualCell)) {
        self.for_each_cell_in_range(model, 0..model.height(), 0..model.width(), f);
    }

    /// Visit only visual cells that intersect the requested logical grid
    /// rectangle. The one-cell overlap handling is important for wide leads:
    /// a two-cell glyph that starts just before the range still belongs in the
    /// rendered output.
    pub fn for_each_cell_in_range(
        &self,
        model: &GridModel,
        rows: std::ops::Range<usize>,
        columns: std::ops::Range<usize>,
        mut f: impl FnMut(VisualCell),
    ) {
        for row in rows {
            let Some(grid_row) = model.rows().get(row) else {
                continue;
            };
            self.for_each_row(row, grid_row, columns.clone(), &mut f);
        }
    }

    pub fn build_row(&self, row: usize, grid_row: &GridRow) -> Vec<VisualCell> {
        let mut visual_cells = Vec::new();
        self.for_each_row(row, grid_row, 0..grid_row.cells().len(), &mut |cell| {
            visual_cells.push(cell)
        });
        visual_cells
    }

    /// Build only the visual cell that contains a logical grid column.
    ///
    /// Cursor rendering uses this path for every repaint. Avoiding a full row
    /// allocation keeps cursor movement independent of the width of the row.
    pub fn build_cell_at(
        &self,
        row: usize,
        grid_row: &GridRow,
        column: usize,
    ) -> Option<VisualCell> {
        let mut result = None;
        let end = column.saturating_add(1);
        self.for_each_row(row, grid_row, column..end, &mut |cell| {
            if (cell.grid_start..cell.grid_start.saturating_add(cell.grid_len)).contains(&column) {
                result = Some(cell);
            }
        });
        result
    }

    fn for_each_row(
        &self,
        row: usize,
        grid_row: &GridRow,
        columns: std::ops::Range<usize>,
        f: &mut impl FnMut(VisualCell),
    ) {
        let cells = grid_row.cells();
        let start = columns.start.min(cells.len());
        let end = columns.end.min(cells.len());
        if start >= end {
            return;
        }

        // A clipped range may begin on the continuation half of a wide
        // character. Start at the lead so the visual unit is never split.
        let mut col = if start > 0
            && cells[start].kind == CellKind::WideContinuation
            && cells[start - 1].kind == CellKind::WideLead
        {
            start - 1
        } else {
            start
        };

        while col < cells.len() {
            let cell = &cells[col];

            if cell.kind == CellKind::WideContinuation {
                if col >= end {
                    break;
                }
                f(VisualCell {
                    row,
                    grid_start: col,
                    grid_len: 1,
                    text: SharedString::new_static(" "),
                    highlight: cell.highlight,
                    kind: VisualCellKind::Text,
                });
                col += 1;
                continue;
            }

            if cell.kind == CellKind::WideLead {
                let has_continuation = cells
                    .get(col + 1)
                    .is_some_and(|next| next.kind == CellKind::WideContinuation);
                let is_nerd_symbol =
                    self.nerd_font_mode && is_nerd_symbol(&cell.text) && has_continuation;
                let grid_len = usize::from(has_continuation) + 1;
                if col >= end {
                    break;
                }

                f(VisualCell {
                    row,
                    grid_start: col,
                    grid_len,
                    text: cell.text.clone(),
                    highlight: cell.highlight,
                    kind: if is_nerd_symbol {
                        VisualCellKind::NerdSymbol
                    } else if has_continuation {
                        VisualCellKind::WideCharacter
                    } else {
                        VisualCellKind::Text
                    },
                });
                col += grid_len;
                continue;
            }

            if col >= end {
                break;
            }
            f(VisualCell {
                row,
                grid_start: col,
                grid_len: 1,
                text: cell.text.clone(),
                highlight: cell.highlight,
                kind: if self.nerd_font_mode && is_nerd_symbol(&cell.text) {
                    VisualCellKind::NerdSymbol
                } else {
                    VisualCellKind::Text
                },
            });
            col += 1;
        }
    }
}

fn is_nerd_symbol(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(character) = chars.next() else {
        return false;
    };

    chars.next().is_none()
        && (('\u{e000}'..='\u{f8ff}').contains(&character)
            || ('\u{f0000}'..='\u{ffffd}').contains(&character)
            || ('\u{100000}'..='\u{10fffd}').contains(&character))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightContext {
    Main,
    Floating { background: Option<u32> },
    Message { background: Option<u32> },
}

impl HighlightContext {
    pub fn background_override(self) -> Option<u32> {
        match self {
            Self::Main => None,
            Self::Floating { background } | Self::Message { background } => background,
        }
    }
}

/// Highlight attributes resolved against grid defaults and the owning layer.
///
/// Keeping the original attributes alongside resolved colors lets the paint
/// pass use one authoritative result for conceal, font flags, decorations,
/// reverse, dim, and blend.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedHighlight {
    pub attrs: HighlightAttrs,
    pub foreground: Hsla,
    pub background: Option<Hsla>,
    pub special: Hsla,
}

pub fn resolve_highlight(
    model: &GridModel,
    highlight: HighlightId,
    context: HighlightContext,
) -> ResolvedHighlight {
    resolve_highlight_from_parts(
        model.highlights(),
        model.default_colors(),
        highlight,
        context,
    )
}

pub(crate) fn resolve_highlight_from_parts(
    highlights: &HashMap<HighlightId, HighlightAttrs>,
    defaults: (Option<u32>, Option<u32>, Option<u32>),
    highlight: HighlightId,
    context: HighlightContext,
) -> ResolvedHighlight {
    let attrs = highlights.get(&highlight).cloned().unwrap_or_default();
    let (default_foreground, default_background, default_special) = defaults;
    let foreground = attrs
        .foreground
        .or(default_foreground)
        .unwrap_or(DEFAULT_FOREGROUND);
    let background = if highlight == DEFAULT_HIGHLIGHT {
        context
            .background_override()
            .or(attrs.background)
            .or(default_background)
    } else {
        attrs
            .background
            .or(context.background_override())
            .or(default_background)
    }
    .unwrap_or(DEFAULT_BACKGROUND);
    let special = attrs
        .special
        .or(default_special)
        .or(attrs.foreground)
        .or(default_foreground)
        .unwrap_or(DEFAULT_FOREGROUND);
    let mut foreground: Hsla = rgb(foreground).into();
    let mut background: Hsla = rgb(background).into();
    if attrs.dim {
        foreground.a *= 0.6;
    }
    let alpha = blend_alpha(attrs.blend);
    foreground.a *= alpha;
    background.a *= alpha;

    if attrs.reverse {
        ResolvedHighlight {
            attrs,
            foreground: background,
            background: Some(foreground),
            special: rgb(special).into(),
        }
    } else {
        ResolvedHighlight {
            attrs,
            foreground,
            background: Some(background),
            special: rgb(special).into(),
        }
    }
}

#[cfg(test)]
pub(super) fn highlight_colors(
    model: &GridModel,
    highlight: HighlightId,
    background_override: Option<u32>,
) -> (Hsla, Option<Hsla>) {
    let context = background_override
        .map(|background| HighlightContext::Floating {
            background: Some(background),
        })
        .unwrap_or(HighlightContext::Main);
    let style = resolve_highlight(model, highlight, context);
    (style.foreground, style.background)
}

fn blend_alpha(blend: Option<u8>) -> f32 {
    blend
        .map(|blend| 1.0 - f32::from(blend.min(100)) / 100.0)
        .unwrap_or(1.0)
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct BackgroundSpan {
    pub row: usize,
    pub columns: Range<usize>,
    pub offset: gpui::Point<Pixels>,
}

pub(super) fn push_background(
    backgrounds: &mut Vec<(Bounds<Pixels>, Hsla, bool)>,
    previous_span: &mut Option<BackgroundSpan>,
    span: BackgroundSpan,
    bounds: Bounds<Pixels>,
    color: Hsla,
    in_viewport: bool,
) {
    if let Some((previous_bounds, previous_color, previous_in_viewport)) = backgrounds.last_mut() {
        if *previous_color == color
            && *previous_in_viewport == in_viewport
            && previous_bounds.origin.y == bounds.origin.y
            && previous_bounds.size.height == bounds.size.height
            && previous_span.as_ref().is_some_and(|previous| {
                previous.row == span.row
                    && previous.columns.end == span.columns.start
                    && previous.offset == span.offset
            })
        {
            // Adjacency is a grid property, not a floating-point comparison.
            // Derive the right edge from the new cell instead of repeatedly
            // adding fractional advances (e.g. 0xProto's 9.92px cells).
            previous_bounds.size.width =
                bounds.origin.x + bounds.size.width - previous_bounds.origin.x;
            *previous_span = Some(span);
            return;
        }
    }

    backgrounds.push((bounds, color, in_viewport));
    *previous_span = Some(span);
}

#[cfg(test)]
mod background_tests {
    use super::*;

    fn append(
        backgrounds: &mut Vec<(Bounds<Pixels>, Hsla, bool)>,
        previous: &mut Option<BackgroundSpan>,
        span: BackgroundSpan,
        color: Hsla,
        in_viewport: bool,
    ) {
        let bounds = Bounds::new(
            point(
                px(13.25) + px(9.92) * span.columns.start,
                px(20.0) * span.row,
            ) + span.offset,
            size(px(9.92) * span.columns.len(), px(20.0)),
        );
        push_background(backgrounds, previous, span, bounds, color, in_viewport);
    }

    fn span(row: usize, columns: Range<usize>) -> BackgroundSpan {
        BackgroundSpan {
            row,
            columns,
            offset: point(px(0.0), px(0.0)),
        }
    }

    #[test]
    fn fractional_cell_backgrounds_merge_at_any_grid_width() {
        for columns in [64, 160, 320, 1000] {
            let mut backgrounds = Vec::new();
            let mut previous = None;
            for column in 0..columns {
                append(
                    &mut backgrounds,
                    &mut previous,
                    span(0, column..column + 1),
                    rgb(0x123456).into(),
                    true,
                );
            }
            assert_eq!(backgrounds.len(), 1, "{columns} columns");
            let bounds = backgrounds[0].0;
            let expected_right = px(13.25) + px(9.92) * (columns - 1) + px(9.92);
            assert_eq!(bounds.origin.x + bounds.size.width, expected_right);
        }
    }

    #[test]
    fn wide_cells_merge_but_gaps_rows_colors_masks_and_offsets_do_not() {
        let mut backgrounds = Vec::new();
        let mut previous = None;
        let color = rgb(0x123456).into();
        for columns in [0..1, 1..3, 3..4] {
            append(
                &mut backgrounds,
                &mut previous,
                span(0, columns),
                color,
                true,
            );
        }
        assert_eq!(backgrounds.len(), 1);
        append(&mut backgrounds, &mut previous, span(0, 5..6), color, true);
        append(&mut backgrounds, &mut previous, span(1, 6..7), color, true);
        append(
            &mut backgrounds,
            &mut previous,
            span(1, 7..8),
            rgb(0x654321).into(),
            true,
        );
        append(
            &mut backgrounds,
            &mut previous,
            span(1, 8..9),
            rgb(0x654321).into(),
            false,
        );
        append(
            &mut backgrounds,
            &mut previous,
            BackgroundSpan {
                offset: point(px(0.25), px(0.0)),
                ..span(1, 9..10)
            },
            rgb(0x654321).into(),
            false,
        );
        assert_eq!(backgrounds.len(), 6);
    }
}
