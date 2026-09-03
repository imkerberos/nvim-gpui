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

    pub fn for_each_cell(&self, model: &GridModel, mut f: impl FnMut(VisualCell)) {
        for (row, grid_row) in model.rows().iter().enumerate() {
            self.for_each_row(row, grid_row, &mut f);
        }
    }

    pub fn build_row(&self, row: usize, grid_row: &GridRow) -> Vec<VisualCell> {
        let mut visual_cells = Vec::new();
        self.for_each_row(row, grid_row, &mut |cell| visual_cells.push(cell));
        visual_cells
    }

    fn for_each_row(&self, row: usize, grid_row: &GridRow, f: &mut impl FnMut(VisualCell)) {
        let cells = grid_row.cells();
        let mut col = 0;

        while col < cells.len() {
            let cell = &cells[col];

            if cell.kind == CellKind::WideContinuation {
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

                f(VisualCell {
                    row,
                    grid_start: col,
                    grid_len: if has_continuation { 2 } else { 1 },
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
                col += usize::from(has_continuation) + 1;
                continue;
            }

            let is_nerd_symbol = self.nerd_font_mode && is_nerd_symbol(&cell.text);
            let has_symbol_padding = is_nerd_symbol
                && cells.get(col + 1).is_some_and(|next| {
                    next.highlight == cell.highlight
                        && next.kind != CellKind::WideContinuation
                        && next.text == " "
                });

            if has_symbol_padding {
                f(VisualCell {
                    row,
                    grid_start: col,
                    grid_len: 2,
                    text: cell.text.clone(),
                    highlight: cell.highlight,
                    kind: VisualCellKind::NerdSymbol,
                });
                col += 2;
                continue;
            }

            f(VisualCell {
                row,
                grid_start: col,
                grid_len: 1,
                text: cell.text.clone(),
                highlight: cell.highlight,
                kind: if is_nerd_symbol {
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

pub(super) fn highlight_colors(
    model: &GridModel,
    highlight: HighlightId,
    background_override: Option<u32>,
) -> (Hsla, Option<Hsla>) {
    let attrs = model
        .highlight_ref(highlight)
        .map(Cow::Borrowed)
        .unwrap_or_else(|| Cow::Owned(HighlightAttrs::default()));
    let (default_foreground, default_background, _) = model.default_colors();
    let foreground = attrs
        .foreground
        .or(default_foreground)
        .unwrap_or(DEFAULT_FOREGROUND);
    // A terminal paints every cell, including a blank cell. In a multigrid
    // float, Neovim can use the default highlight id for an implicit cell
    // even though that id already contains the main grid's explicit
    // background. Prefer the float surface for that id; explicit non-default
    // highlights still retain their own background.
    let background = if highlight == DEFAULT_HIGHLIGHT {
        background_override
            .or(attrs.background)
            .or(default_background)
    } else {
        attrs
            .background
            .or(background_override)
            .or(default_background)
    }
    .unwrap_or(DEFAULT_BACKGROUND);
    let mut foreground: Hsla = rgb(foreground).into();
    if attrs.dim {
        foreground.a *= 0.6;
    }
    foreground.a *= blend_alpha(attrs.blend);

    if attrs.reverse {
        let mut background_color: Hsla = rgb(background).into();
        background_color.a *= blend_alpha(attrs.blend);
        (background_color, Some(foreground))
    } else {
        let mut background: Hsla = rgb(background).into();
        background.a *= blend_alpha(attrs.blend);
        (foreground, Some(background))
    }
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
