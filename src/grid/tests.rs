use super::{
    blink_visible, cursor_bounds, cursor_colors, cursor_geometry, highlight_colors, jelly_progress,
    CellKind, CursorAnimation, CursorModeInfo, CursorShape, CursorVisualPosition, GridCell,
    GridLineCell, GridModel, GridRow, HighlightAttrs, HighlightId, VisualCell, VisualCellBuilder,
    VisualCellKind, DEFAULT_HIGHLIGHT,
};
use gpui::{point, px, size, Bounds};
use std::time::{Duration, Instant};

#[test]
fn wide_character_occupies_two_grid_cells() {
    let row = GridRow::new(vec![
        GridCell::wide_lead("界", DEFAULT_HIGHLIGHT),
        GridCell::wide_continuation(DEFAULT_HIGHLIGHT),
    ]);

    let cells = VisualCellBuilder::new(false).build_row(4, &row);

    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0].row, 4);
    assert_eq!(cells[0].grid_start, 0);
    assert_eq!(cells[0].grid_len, 2);
    assert_eq!(cells[0].text, "界");
    assert_eq!(cells[0].kind, VisualCellKind::WideCharacter);
}

#[test]
fn cursor_highlight_only_applies_to_the_cursor_row() {
    let cell = VisualCell {
        row: 3,
        grid_start: 5,
        grid_len: 1,
        text: "x".into(),
        highlight: DEFAULT_HIGHLIGHT,
        kind: VisualCellKind::Text,
    };
    let cursor = CursorVisualPosition {
        row: 4,
        col: 5,
        width: 1,
    };

    assert!(!super::visual_cell_overlaps_cursor(&cell, cursor));
    assert!(super::visual_cell_overlaps_cursor(
        &VisualCell { row: 4, ..cell },
        cursor
    ));
}

#[test]
fn nerd_symbol_and_following_space_share_a_two_cell_visual_span() {
    let row = GridRow::new(vec![
        GridCell::text("\u{f0239}", DEFAULT_HIGHLIGHT),
        GridCell::text(" ", DEFAULT_HIGHLIGHT),
    ]);

    let cells = VisualCellBuilder::new(true).build_row(0, &row);

    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0].grid_start, 0);
    assert_eq!(cells[0].grid_len, 2);
    assert_eq!(cells[0].kind, VisualCellKind::NerdSymbol);
}

#[test]
fn nerd_symbol_does_not_consume_a_differently_highlighted_space() {
    let row = GridRow::new(vec![
        GridCell::text("\u{f0239}", DEFAULT_HIGHLIGHT),
        GridCell::text(" ", HighlightId(1)),
    ]);

    let cells = VisualCellBuilder::new(true).build_row(0, &row);

    assert_eq!(cells.len(), 2);
    assert_eq!(cells[0].grid_len, 1);
    assert_eq!(cells[1].grid_len, 1);
}

#[test]
fn wide_nerd_symbol_uses_the_main_font_visual_kind() {
    let row = GridRow::new(vec![
        GridCell::wide_lead("\u{f0239}", DEFAULT_HIGHLIGHT),
        GridCell::wide_continuation(DEFAULT_HIGHLIGHT),
    ]);

    let cells = VisualCellBuilder::new(true).build_row(0, &row);

    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0].grid_len, 2);
    assert_eq!(cells[0].kind, VisualCellKind::NerdSymbol);
}

#[test]
fn one_visual_cell_keeps_its_grapheme_cluster_intact() {
    let combining = "e\u{301}";
    let emoji = "👩‍💻";
    let row = GridRow::new(vec![
        GridCell::text(combining, DEFAULT_HIGHLIGHT),
        GridCell::text(emoji, DEFAULT_HIGHLIGHT),
    ]);

    let cells = VisualCellBuilder::new(false).build_row(0, &row);

    assert_eq!(cells.len(), 2);
    assert_eq!(cells[0].text, combining);
    assert_eq!(cells[0].grid_len, 1);
    assert_eq!(cells[1].text, emoji);
    assert_eq!(cells[1].grid_len, 1);
}

#[test]
fn adjacent_cells_are_never_combined_for_text_layout() {
    let row = GridRow::new(vec![
        GridCell::text("a", DEFAULT_HIGHLIGHT),
        GridCell::text("c", DEFAULT_HIGHLIGHT),
        GridCell::text("d", HighlightId(1)),
    ]);

    let cells = VisualCellBuilder::new(false).build_row(0, &row);

    assert_eq!(cells.len(), 3);
    assert_eq!(cells[0].text, "a");
    assert_eq!(cells[0].grid_start, 0);
    assert_eq!(cells[1].text, "c");
    assert_eq!(cells[1].grid_start, 1);
    assert_eq!(cells[2].text, "d");
    assert_eq!(cells[2].grid_start, 2);
}

#[test]
fn model_pads_rows_to_a_stable_grid_width() {
    let model = GridModel::from_rows(vec![
        GridRow::new(vec![GridCell::text("a", DEFAULT_HIGHLIGHT)]),
        GridRow::new(vec![
            GridCell::text("b", DEFAULT_HIGHLIGHT),
            GridCell::blank(DEFAULT_HIGHLIGHT),
        ]),
    ]);

    assert_eq!(model.width(), 2);
    assert_eq!(model.height(), 2);
    assert_eq!(model.rows()[0].cells().len(), 2);
    assert_eq!(model.rows()[0].cells()[1].kind, CellKind::Blank);
    assert_eq!(model.rows()[1].cells().len(), 2);
    assert_eq!(model.rows()[1].cells()[1].kind, CellKind::Blank);
}

#[test]
fn grid_line_updates_unicode_cells_repeats_and_wrap_state() {
    let mut model = GridModel::new(6, 2);

    model.apply_grid_line(
        0,
        1,
        &[
            GridLineCell::new("界", HighlightId(7), 1),
            GridLineCell::new("", HighlightId(7), 1),
            GridLineCell::new("x", HighlightId(8), 2),
        ],
        true,
    );

    assert_eq!(model.rows()[0].cells()[0].kind, CellKind::Blank);
    assert_eq!(model.rows()[0].cells()[1].text, "界");
    assert_eq!(model.rows()[0].cells()[1].kind, CellKind::WideLead);
    assert_eq!(model.rows()[0].cells()[2].kind, CellKind::WideContinuation);
    assert_eq!(model.rows()[0].cells()[3].text, "x");
    assert_eq!(model.rows()[0].cells()[4].text, "x");
    assert!(model.rows()[0].wraps_to_next);
}

#[test]
fn grid_line_repeat_zero_does_not_consume_a_cell() {
    let mut model = GridModel::new(3, 1);

    model.apply_grid_line(
        0,
        0,
        &[
            GridLineCell::new("a", HighlightId(1), 1),
            GridLineCell::new(" ", DEFAULT_HIGHLIGHT, 0),
            GridLineCell::new("b", HighlightId(2), 1),
        ],
        false,
    );

    assert_eq!(model.rows()[0].cells()[0].text, "a");
    assert_eq!(model.rows()[0].cells()[0].highlight, HighlightId(1));
    assert_eq!(model.rows()[0].cells()[1].text, "b");
    assert_eq!(model.rows()[0].cells()[1].highlight, HighlightId(2));
    assert_eq!(model.rows()[0].cells()[2].text, " ");
    assert_eq!(model.rows()[0].cells()[2].highlight, DEFAULT_HIGHLIGHT);
}

#[test]
fn mixed_ascii_and_wide_cells_keep_their_grid_columns() {
    let mut model = GridModel::new(6, 1);

    model.apply_grid_line(
        0,
        0,
        &[
            GridLineCell::new("a", DEFAULT_HIGHLIGHT, 1),
            GridLineCell::new("中", DEFAULT_HIGHLIGHT, 1),
            GridLineCell::new("", DEFAULT_HIGHLIGHT, 1),
            GridLineCell::new("b", DEFAULT_HIGHLIGHT, 1),
        ],
        false,
    );

    let cells = VisualCellBuilder::new(false).build_row(0, &model.rows()[0]);

    assert_eq!(cells[0].grid_start, 0);
    assert_eq!(cells[0].grid_len, 1);
    assert_eq!(cells[1].grid_start, 1);
    assert_eq!(cells[1].grid_len, 2);
    assert_eq!(cells[2].grid_start, 3);
    assert_eq!(cells[2].text.chars().next(), Some('b'));
}

#[test]
fn empty_grid_line_cell_is_the_protocol_wide_continuation() {
    let mut model = GridModel::new(4, 1);

    // The empty entry is the protocol marker for the second cell. The
    // renderer must not reinterpret that marker from the code point's
    // local Unicode-width classification (for example, box-drawing
    // characters can be affected by Nvim's `ambiwidth`).
    model.apply_grid_line(
        0,
        0,
        &[
            GridLineCell::new("│", DEFAULT_HIGHLIGHT, 1),
            GridLineCell::new("", DEFAULT_HIGHLIGHT, 1),
            GridLineCell::new("x", DEFAULT_HIGHLIGHT, 1),
        ],
        false,
    );

    assert_eq!(model.rows()[0].cells()[0].kind, CellKind::WideLead);
    assert_eq!(model.rows()[0].cells()[1].kind, CellKind::WideContinuation);

    let cells = VisualCellBuilder::new(false).build_row(0, &model.rows()[0]);
    assert_eq!(cells[0].grid_start, 0);
    assert_eq!(cells[0].grid_len, 2);
    assert_eq!(cells[1].grid_start, 2);
    assert_eq!(cells[1].text, "x");
}

#[test]
fn grid_scroll_moves_rows_and_clears_the_scrolled_in_area() {
    let mut model = GridModel::from_rows(vec![
        GridRow::new(vec![GridCell::text("a", DEFAULT_HIGHLIGHT)]),
        GridRow::new(vec![GridCell::text("b", DEFAULT_HIGHLIGHT)]),
        GridRow::new(vec![GridCell::text("c", DEFAULT_HIGHLIGHT)]),
    ]);

    model.scroll(0, 3, 0, 1, 1, 0);

    assert_eq!(model.rows()[0].cells()[0].text, "b");
    assert_eq!(model.rows()[1].cells()[0].text, "c");
    assert_eq!(model.rows()[2].cells()[0].kind, CellKind::Blank);
}

#[test]
fn cursor_is_kept_in_the_grid_model() {
    let mut model = GridModel::new(4, 2);

    model.set_cursor(1, 3);

    assert_eq!(model.cursor(), Some(super::GridCursor { row: 1, col: 3 }));
}

#[test]
fn cursor_can_arrive_before_the_grid_resize() {
    let mut model = GridModel::new(0, 0);

    model.set_cursor(4, 7);
    model.resize(10, 5);

    assert_eq!(model.cursor(), Some(super::GridCursor { row: 4, col: 7 }));
}

#[test]
fn cursor_covers_a_wide_character_from_either_grid_cell() {
    let row = GridRow::new(vec![
        GridCell::wide_lead("界", DEFAULT_HIGHLIGHT),
        GridCell::wide_continuation(DEFAULT_HIGHLIGHT),
    ]);

    assert_eq!(cursor_geometry(&row, 0), (0, 2));
    assert_eq!(cursor_geometry(&row, 1), (0, 2));
}

#[test]
fn cursor_animation_interpolates_to_its_target() {
    let animation = CursorAnimation::new(
        CursorVisualPosition {
            row: 2,
            col: 3,
            width: 1,
        },
        CursorVisualPosition {
            row: 5,
            col: 8,
            width: 2,
        },
    );

    let start = animation.position_at(animation.started_at);
    let middle = animation.position_at(animation.started_at + Duration::from_millis(72));
    let end = animation.position_at(animation.started_at + animation.duration);

    assert_eq!(start.row, 2.0);
    assert_eq!(start.col, 3.0);
    assert!(middle.row > 2.0 && middle.row < 5.0);
    assert!(middle.col > 3.0 && middle.col < 8.0);
    assert!(middle.width > 1.0 && middle.width < 2.0);
    assert_eq!(end.row, 5.0);
    assert_eq!(end.col, 8.0);
    assert_eq!(end.width, 2.0);
}

#[test]
fn cursor_animation_has_a_small_elastic_settle() {
    assert_eq!(jelly_progress(0.0), 0.0);
    assert!(jelly_progress(0.72) > 1.0);
    assert!(jelly_progress(0.72) <= 1.025);
    assert!(jelly_progress(0.92) < jelly_progress(0.72));
    assert_eq!(jelly_progress(1.0), 1.0);
}

#[test]
fn model_stores_neovim_highlight_attributes() {
    let mut model = GridModel::new(1, 1);
    let attrs = HighlightAttrs {
        foreground: Some(0xabcdef),
        reverse: true,
        ..Default::default()
    };

    model.set_highlight(HighlightId(42), attrs.clone());

    assert_eq!(model.highlight(HighlightId(42)), Some(attrs));
}

#[test]
fn missing_highlight_background_inherits_the_grid_default() {
    let mut model = GridModel::new(1, 1);
    model.set_default_colors(Some(0xffffff), Some(0x112233), None);
    model.set_highlight(HighlightId(42), HighlightAttrs::default());

    let (_, background) = highlight_colors(&model, HighlightId(42), None);

    assert_eq!(background, Some(gpui::rgb(0x112233).into()));
}

#[test]
fn floating_grid_background_override_fills_implicit_cells() {
    let mut model = GridModel::new(1, 1);
    model.set_default_colors(Some(0xffffff), Some(0x000000), None);
    model.set_highlight(HighlightId(42), HighlightAttrs::default());

    let (_, background) = highlight_colors(&model, HighlightId(42), Some(0x001419));

    assert_eq!(background, Some(gpui::rgb(0x001419).into()));
}

#[test]
fn floating_grid_background_override_replaces_explicit_default_background() {
    let mut model = GridModel::new(1, 1);
    model.set_default_colors(Some(0xffffff), Some(0x000000), None);
    model.set_highlight(
        DEFAULT_HIGHLIGHT,
        HighlightAttrs {
            background: Some(0x000000),
            ..Default::default()
        },
    );

    let (_, background) = highlight_colors(&model, DEFAULT_HIGHLIGHT, Some(0x001419));

    assert_eq!(background, Some(gpui::rgb(0x001419).into()));
}

#[test]
fn highlight_blend_uses_neovims_zero_to_hundred_transparency_scale() {
    let mut model = GridModel::new(1, 1);
    model.set_highlight(
        HighlightId(42),
        HighlightAttrs {
            foreground: Some(0xffffff),
            background: Some(0x112233),
            blend: Some(25),
            ..Default::default()
        },
    );

    let (foreground, background) = highlight_colors(&model, HighlightId(42), None);

    assert!((foreground.a - 0.75).abs() < f32::EPSILON);
    assert!((background.expect("highlight background").a - 0.75).abs() < f32::EPSILON);
}

#[test]
fn default_cursor_attribute_swaps_the_current_cell_colors() {
    let mut model = GridModel::new(1, 1);
    model.set_default_colors(Some(0xffffff), Some(0x112233), None);
    model.set_highlight(
        HighlightId(42),
        HighlightAttrs {
            foreground: Some(0xaabbcc),
            background: Some(0x445566),
            ..Default::default()
        },
    );
    model.apply_grid_line(0, 0, &[GridLineCell::new("x", HighlightId(42), 1)], false);

    let normal = highlight_colors(&model, HighlightId(42), None);
    let cursor = cursor_colors(
        &model,
        CursorVisualPosition {
            row: 0,
            col: 0,
            width: 1,
        },
        CursorModeInfo {
            attr_id: Some(DEFAULT_HIGHLIGHT),
            ..Default::default()
        },
    );

    assert_eq!(cursor.0, normal.1.expect("cell background should exist"));
    assert_eq!(cursor.1, normal.0);
}

#[test]
fn destroy_clears_grid_contents_cursor_highlights_and_defaults() {
    let mut model = GridModel::new(2, 1);
    model.set_cursor(0, 1);
    model.set_highlight(
        HighlightId(42),
        HighlightAttrs {
            foreground: Some(0xabcdef),
            ..Default::default()
        },
    );
    model.set_default_colors(Some(1), Some(2), Some(3));

    model.destroy();

    assert_eq!(model.width(), 0);
    assert_eq!(model.height(), 0);
    assert_eq!(model.cursor(), None);
    assert!(model.highlights().is_empty());
    assert_eq!(model.default_colors(), (None, None, None));
}

#[test]
fn cursor_shapes_use_the_neovim_cell_percentage() {
    let bounds = Bounds::new(point(px(10.0), px(20.0)), size(px(80.0), px(88.0)));
    let position = super::CursorVisualPosition {
        row: 1,
        col: 2,
        width: 2,
    };

    let horizontal = cursor_bounds(
        bounds,
        px(10.0),
        px(22.0),
        position,
        CursorModeInfo {
            shape: CursorShape::Horizontal,
            cell_percentage: 25,
            ..Default::default()
        },
    );
    assert_eq!(f32::from(horizontal.origin.x), 30.0);
    assert_eq!(f32::from(horizontal.origin.y), 58.5);
    assert_eq!(f32::from(horizontal.size.width), 20.0);
    assert_eq!(f32::from(horizontal.size.height), 5.5);

    let vertical = cursor_bounds(
        bounds,
        px(10.0),
        px(22.0),
        position,
        CursorModeInfo {
            shape: CursorShape::Vertical,
            cell_percentage: 20,
            ..Default::default()
        },
    );
    assert_eq!(f32::from(vertical.origin.x), 30.0);
    assert_eq!(f32::from(vertical.origin.y), 42.0);
    assert_eq!(f32::from(vertical.size.width), 4.0);
    assert_eq!(f32::from(vertical.size.height), 22.0);
}

#[test]
fn cursor_blink_respects_wait_on_and_off_intervals() {
    let started_at = Instant::now();

    assert!(blink_visible(
        started_at,
        started_at + Duration::from_millis(100),
        200,
        400,
        250
    ));
    assert!(blink_visible(
        started_at,
        started_at + Duration::from_millis(500),
        200,
        400,
        250
    ));
    assert!(!blink_visible(
        started_at,
        started_at + Duration::from_millis(650),
        200,
        400,
        250
    ));
    assert!(blink_visible(
        started_at,
        started_at + Duration::from_millis(900),
        200,
        400,
        250
    ));
}

#[test]
fn keyword_highlight_is_distinct_from_default() {
    let row = GridRow::new(vec![
        GridCell::text("fn", HighlightId(2)),
        GridCell::text(" main", DEFAULT_HIGHLIGHT),
    ]);

    let cells = VisualCellBuilder::new(false).build_row(0, &row);

    assert_eq!(cells[0].highlight, HighlightId(2));
    assert_eq!(cells[1].highlight, DEFAULT_HIGHLIGHT);
}
