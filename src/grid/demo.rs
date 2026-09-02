use super::*;

pub fn demo_grid() -> GridModel {
    let mut unicode_row = text_cells("Unicode: ", COMMENT_HIGHLIGHT);
    unicode_row.push(GridCell::wide_lead("界", DEFAULT_HIGHLIGHT));
    unicode_row.push(GridCell::wide_continuation(DEFAULT_HIGHLIGHT));
    unicode_row.extend(text_cells(" ", DEFAULT_HIGHLIGHT));
    unicode_row.push(GridCell::wide_lead("你", DEFAULT_HIGHLIGHT));
    unicode_row.push(GridCell::wide_continuation(DEFAULT_HIGHLIGHT));
    unicode_row.push(GridCell::wide_lead("好", DEFAULT_HIGHLIGHT));
    unicode_row.push(GridCell::wide_continuation(DEFAULT_HIGHLIGHT));

    let mut combining_row = text_cells("Combining: ", STRING_HIGHLIGHT);
    combining_row.push(GridCell::text("e\u{301}", STRING_HIGHLIGHT));
    combining_row.extend(text_cells("  emoji: ", STRING_HIGHLIGHT));
    combining_row.push(GridCell::wide_lead("👩‍💻", DEFAULT_HIGHLIGHT));
    combining_row.push(GridCell::wide_continuation(DEFAULT_HIGHLIGHT));

    let mut nerd_row = text_cells("Nerd Font: ", COMMENT_HIGHLIGHT);
    nerd_row.push(GridCell::text("\u{f0239}", DEFAULT_HIGHLIGHT));
    nerd_row.push(GridCell::blank(DEFAULT_HIGHLIGHT));
    nerd_row.extend(text_cells(
        "symbol + space -> one visual span",
        DEFAULT_HIGHLIGHT,
    ));

    let long_ascii_row = long_ascii_cells("Long ASCII (2048 chars): ");
    let long_unicode_row = long_unicode_cells("Long Unicode (2048 chars): ");

    GridModel::from_rows(vec![
        GridRow::new(unicode_row),
        GridRow::new(combining_row),
        GridRow::new(nerd_row),
        GridRow::new(
            [
                text_cells("highlight ", DEFAULT_HIGHLIGHT),
                text_cells("changes", KEYWORD_HIGHLIGHT),
                text_cells(" at cell boundaries", DEFAULT_HIGHLIGHT),
            ]
            .into_iter()
            .flatten()
            .collect(),
        ),
        GridRow::new(text_cells(
            "This row reports a wrap boundary",
            COMMENT_HIGHLIGHT,
        ))
        .wrapped(),
        GridRow::new(long_ascii_row),
        GridRow::new(long_unicode_row),
    ])
}

fn text_cells(text: &str, highlight: HighlightId) -> Vec<GridCell> {
    text.chars()
        .map(|character| GridCell::text(character.to_string(), highlight))
        .collect()
}

fn long_ascii_cells(prefix: &str) -> Vec<GridCell> {
    let mut cells = text_cells(prefix, COMMENT_HIGHLIGHT);

    for index in 0..LONG_TEXT_CHAR_COUNT {
        let character = char::from(b'a' + (index % 26) as u8);
        cells.push(GridCell::text(character.to_string(), DEFAULT_HIGHLIGHT));
    }

    cells
}

fn long_unicode_cells(prefix: &str) -> Vec<GridCell> {
    let mut cells = text_cells(prefix, COMMENT_HIGHLIGHT);
    let pattern = ['a', '界', 'b', '你', 'c', '好', 'd', 'e'];

    for index in 0..LONG_TEXT_CHAR_COUNT {
        let character = pattern[index % pattern.len()];
        if matches!(character, '界' | '你' | '好') {
            cells.push(GridCell::wide_lead(
                character.to_string(),
                DEFAULT_HIGHLIGHT,
            ));
            cells.push(GridCell::wide_continuation(DEFAULT_HIGHLIGHT));
        } else {
            cells.push(GridCell::text(character.to_string(), DEFAULT_HIGHLIGHT));
        }
    }

    cells
}
