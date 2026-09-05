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
    let attrs = model.highlight_ref(highlight).cloned().unwrap_or_default();
    let (default_foreground, default_background, default_special) = model.default_colors();
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

pub(super) fn push_background(
    backgrounds: &mut Vec<(Bounds<Pixels>, Hsla, bool)>,
    bounds: Bounds<Pixels>,
    color: Hsla,
    in_viewport: bool,
) {
    if let Some((previous_bounds, previous_color, previous_in_viewport)) = backgrounds.last_mut() {
        let previous_right = previous_bounds.origin.x + previous_bounds.size.width;
        if *previous_color == color
            && *previous_in_viewport == in_viewport
            && previous_bounds.origin.y == bounds.origin.y
            && previous_bounds.size.height == bounds.size.height
            && previous_right == bounds.origin.x
        {
            previous_bounds.size.width += bounds.size.width;
            return;
        }
    }

    backgrounds.push((bounds, color, in_viewport));
}
