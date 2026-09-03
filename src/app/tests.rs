use super::{
    initial_window_size_for_grid, parse_guifont_spec, themed_titlebar_enabled, EditorState,
    GridPlacement, GridViewport, GridViewportMargins, NvimGpui, DEFAULT_GRID_CELL_WIDTH,
    DEFAULT_GRID_LINE_HEIGHT, DEFAULT_WINDOW_TITLE, THEMED_TITLEBAR_HEIGHT,
};
use crate::{
    grid::{
        CursorModeInfo, CursorShape, CursorVisualPosition, GridLineCell, HighlightAttrs,
        HighlightId,
    },
    image_store::{GridAnchor, GridId, ImageFormatKind, ImageId, ImagePlacement, PlacementKey},
    nvim::NvimEvent,
    parse_cli, CliAction, CliOptions, NvimConnection,
};
use gpui::{point, px};
use std::ffi::OsString;
use std::rc::Rc;
use std::time::Instant;

#[test]
fn cli_keeps_unknown_arguments_for_neovim() {
    let action = parse_cli([
        OsString::from("--no-debug-window"),
        OsString::from("--clean"),
        OsString::from("+set number"),
        OsString::from("README.md"),
    ])
    .expect("CLI should parse");

    assert_eq!(
        action,
        CliAction::Run(CliOptions {
            debug_window: false,
            connection: NvimConnection::Embed,
            nvim_command: None,
            working_directory: None,
            nvim_args: vec![
                OsString::from("--clean"),
                OsString::from("+set number"),
                OsString::from("README.md")
            ],
        })
    );
}

#[test]
fn cli_only_shows_the_debug_window_when_requested() {
    let action = parse_cli([OsString::from("--debug-window")]).expect("CLI should parse");

    assert_eq!(
        action,
        CliAction::Run(CliOptions {
            debug_window: true,
            connection: NvimConnection::Embed,
            nvim_command: None,
            working_directory: None,
            nvim_args: Vec::new(),
        })
    );
}

#[test]
fn cli_separator_forwards_gpui_named_arguments_to_neovim() {
    let action = parse_cli([OsString::from("--"), OsString::from("--no-debug-window")])
        .expect("CLI should parse");

    assert_eq!(
        action,
        CliAction::Run(CliOptions {
            debug_window: false,
            connection: NvimConnection::Embed,
            nvim_command: None,
            working_directory: None,
            nvim_args: vec![OsString::from("--no-debug-window")],
        })
    );
}

#[test]
fn cli_selects_a_remote_neovim_without_forwarding_remote_arguments() {
    let action = parse_cli([
        OsString::from("--no-debug-window"),
        OsString::from("--connect"),
        OsString::from("unix:/tmp/nvim.sock"),
    ])
    .expect("CLI should parse");

    assert_eq!(
        action,
        CliAction::Run(CliOptions {
            debug_window: false,
            connection: NvimConnection::Remote("unix:/tmp/nvim.sock".to_owned()),
            nvim_command: None,
            working_directory: None,
            nvim_args: Vec::new(),
        })
    );
}

#[test]
fn cli_selects_a_wrapped_nvim_command_for_embed_mode() {
    let action = parse_cli([
        OsString::from("--nvim-command"),
        OsString::from("/nix/store/example/bin/nvim"),
        OsString::from("--clean"),
    ])
    .expect("CLI should parse");

    assert_eq!(
        action,
        CliAction::Run(CliOptions {
            debug_window: false,
            connection: NvimConnection::Embed,
            nvim_command: Some(OsString::from("/nix/store/example/bin/nvim")),
            working_directory: None,
            nvim_args: vec![OsString::from("--clean")],
        })
    );
}

#[test]
fn cli_preserves_a_working_directory_for_app_bundle_launches() {
    let action = parse_cli([
        OsString::from("--cwd=/Users/example/project"),
        OsString::from("README.md"),
    ])
    .expect("CLI should parse");

    assert_eq!(
        action,
        CliAction::Run(CliOptions {
            debug_window: false,
            connection: NvimConnection::Embed,
            nvim_command: None,
            working_directory: Some(OsString::from("/Users/example/project")),
            nvim_args: vec![OsString::from("README.md")],
        })
    );
}

#[test]
fn cli_rejects_neovim_arguments_in_remote_mode() {
    let error = parse_cli([
        OsString::from("--connect=127.0.0.1:6666"),
        OsString::from("--clean"),
    ])
    .expect_err("remote mode should reject local Neovim arguments");

    assert!(error.contains("only valid with embed mode"));
}

#[test]
fn editor_starts_in_normal_mode() {
    let state = EditorState::default();

    assert_eq!(state.mode, "NORMAL");
    assert_eq!(state.file, "src/main.rs");
    assert_eq!((state.line, state.column), (1, 1));
}

#[test]
fn initial_window_size_is_derived_from_the_attached_grid() {
    let window_size = initial_window_size_for_grid(80, 24);
    let expected_titlebar = if themed_titlebar_enabled() {
        THEMED_TITLEBAR_HEIGHT
    } else {
        0.0
    };

    assert_eq!(f32::from(window_size.width), 80.0 * DEFAULT_GRID_CELL_WIDTH);
    assert_eq!(
        f32::from(window_size.height),
        24.0 * DEFAULT_GRID_LINE_HEIGHT + expected_titlebar
    );
}

#[test]
fn mouse_position_converts_window_pixels_to_grid_cells() {
    let titlebar = if themed_titlebar_enabled() {
        THEMED_TITLEBAR_HEIGHT
    } else {
        0.0
    };
    let position = point(px(35.9), px(titlebar + 45.9));

    assert_eq!(
        NvimGpui::nvim_mouse_position(position, px(10.0), px(15.0)),
        (3, 3)
    );
}

#[test]
fn startup_keeps_grid_hidden_until_matching_resize_is_flushed() {
    let mut app = NvimGpui {
        nvim_grid_ready: false,
        startup_resize_target: Some((4, 2)),
        ..Default::default()
    };

    app.apply_nvim_event(NvimEvent::GridResized {
        grid: 1,
        width: 3,
        height: 2,
    });
    app.apply_nvim_event(NvimEvent::Flush);
    assert!(!app.nvim_grid_ready);

    app.apply_nvim_event(NvimEvent::GridResized {
        grid: 1,
        width: 4,
        height: 2,
    });
    app.apply_nvim_event(NvimEvent::Flush);
    assert!(app.nvim_grid_ready);
}

#[test]
fn nvim_title_updates_the_window_title_model() {
    let mut app = NvimGpui::default();

    app.apply_nvim_event(NvimEvent::SetTitle {
        title: "nvim — README.md".to_owned(),
    });

    assert_eq!(app.window_title, "nvim — README.md");
}

#[test]
fn default_window_title_is_gpvim() {
    assert_eq!(NvimGpui::default().window_title, DEFAULT_WINDOW_TITLE);
}

#[test]
fn nvim_icon_and_ui_options_update_the_client_model() {
    let mut app = NvimGpui::default();

    app.apply_nvim_event(NvimEvent::SetIcon {
        icon: "nvim-document".to_owned(),
    });
    app.apply_nvim_event(NvimEvent::OptionSet {
        name: "linespace".to_owned(),
        value: "3".to_owned(),
    });
    app.apply_nvim_event(NvimEvent::OptionSet {
        name: "ambiwidth".to_owned(),
        value: "single".to_owned(),
    });

    assert_eq!(app.window_icon, "nvim-document");
    assert_eq!(app.linespace, 3.0);
    assert_eq!(app.ui_options.get("ambiwidth"), Some(&"single".to_owned()));
}

#[test]
fn nvim_mode_info_and_mode_change_select_the_cursor_style() {
    let mut app = NvimGpui::default();
    let mode = CursorModeInfo {
        shape: CursorShape::Vertical,
        cell_percentage: 20,
        blink_wait: 700,
        blink_on: 400,
        blink_off: 250,
        attr_id: Some(HighlightId(8)),
        attr_id_lm: Some(HighlightId(9)),
    };

    app.apply_nvim_event(NvimEvent::ModeInfoSet {
        cursor_style_enabled: true,
        modes: vec![mode],
    });
    app.apply_nvim_event(NvimEvent::ModeChanged {
        mode: "i".to_owned(),
        mode_idx: 0,
    });

    assert_eq!(app.current_cursor_mode(), mode);
    assert_eq!(app.state.mode, "I");
}

#[test]
fn cursor_grid_is_committed_only_at_flush() {
    let mut app = NvimGpui::default();

    app.apply_nvim_event(NvimEvent::GridResized {
        grid: 2,
        width: 4,
        height: 1,
    });
    app.apply_nvim_event(NvimEvent::GridCursorGoto {
        grid: 2,
        row: 0,
        col: 1,
    });

    assert_eq!(app.cursor_grid, 1);
    assert_eq!(app.pending_cursor_grid, Some(2));

    app.apply_nvim_event(NvimEvent::Flush);

    assert_eq!(app.cursor_grid, 2);
    assert_eq!(app.pending_cursor_grid, None);
}

#[test]
fn ime_cursor_position_uses_the_registered_grid() {
    let mut app = NvimGpui::default();

    app.apply_nvim_event(NvimEvent::GridResized {
        grid: 1,
        width: 4,
        height: 2,
    });
    app.apply_nvim_event(NvimEvent::GridCursorGoto {
        grid: 1,
        row: 1,
        col: 3,
    });
    app.apply_nvim_event(NvimEvent::Flush);

    app.apply_nvim_event(NvimEvent::GridResized {
        grid: 2,
        width: 4,
        height: 2,
    });
    app.apply_nvim_event(NvimEvent::GridCursorGoto {
        grid: 2,
        row: 0,
        col: 1,
    });
    app.apply_nvim_event(NvimEvent::Flush);

    app.ime_input_grid = Some(2);
    assert_eq!(
        app.ime_cursor_position(),
        Some(CursorVisualPosition {
            row: 0,
            col: 1,
            width: 1,
        })
    );

    app.ime_input_grid = Some(1);
    assert_eq!(
        app.ime_cursor_position(),
        Some(CursorVisualPosition {
            row: 1,
            col: 3,
            width: 1,
        })
    );
}

#[test]
fn cursor_move_between_grids_uses_one_screen_animation() {
    let mut app = NvimGpui::default();

    app.apply_nvim_event(NvimEvent::GridResized {
        grid: 1,
        width: 4,
        height: 2,
    });
    app.apply_nvim_event(NvimEvent::GridCursorGoto {
        grid: 1,
        row: 0,
        col: 1,
    });
    app.apply_nvim_event(NvimEvent::Flush);

    app.apply_nvim_event(NvimEvent::GridResized {
        grid: 2,
        width: 4,
        height: 2,
    });
    app.apply_nvim_event(NvimEvent::WinPos {
        grid: 2,
        win: Vec::new(),
        row: 2,
        col: 5,
        width: 4,
        height: 2,
    });
    app.apply_nvim_event(NvimEvent::GridCursorGoto {
        grid: 2,
        row: 0,
        col: 2,
    });
    app.apply_nvim_event(NvimEvent::Flush);

    assert_eq!(app.cursor_grid, 2);
    assert!(app
        .cursor_animation
        .is_some_and(|animation| animation.is_active(Instant::now())));
}

#[test]
fn guifont_family_and_size_are_parsed_for_grid_metrics() {
    let spec = parse_guifont_spec("FiraCode Nerd Font Mono:h16");

    assert_eq!(spec.family, "FiraCode Nerd Font Mono");
    assert_eq!(spec.size, 16.0);
}

#[test]
fn empty_guifont_falls_back_to_a_safe_grid_font() {
    let spec = parse_guifont_spec("");

    assert_eq!(spec.family, "Menlo");
    assert_eq!(spec.size, 14.0);
}

#[test]
fn grid_line_height_keeps_a_terminal_sized_cell_and_explicit_linespace() {
    assert_eq!(
        f32::from(super::line_height_from_metrics(px(15.0), px(16.0), 0.0)),
        20.0
    );
    assert_eq!(
        f32::from(super::line_height_from_metrics(px(15.0), px(16.0), 2.0)),
        22.0
    );
    assert_eq!(
        f32::from(super::line_height_from_metrics(px(19.0), px(16.0), 0.0)),
        20.0
    );
}

#[test]
fn grid_updates_become_visible_at_flush() {
    let mut app = NvimGpui::default();

    app.apply_nvim_event(NvimEvent::GridResized {
        grid: 1,
        width: 4,
        height: 1,
    });
    app.apply_nvim_event(NvimEvent::GridLine {
        grid: 1,
        row: 0,
        col_start: 0,
        cells: vec![GridLineCell::new("界", HighlightId(1), 1)],
        wraps_to_next: false,
    });

    assert_ne!(app.grid.width(), 4);
    assert!(app.pending_grid.is_some());

    app.apply_nvim_event(NvimEvent::Flush);

    assert_eq!(app.grid.width(), 4);
    assert_eq!(app.grid.height(), 1);
    assert_eq!(app.grid.rows()[0].cells()[0].text, "界");
    assert!(app.pending_grid.is_none());
}

#[test]
fn theme_changes_become_visible_at_flush() {
    let mut app = NvimGpui::default();

    app.apply_nvim_event(NvimEvent::DefaultColorsSet {
        foreground: Some(0x101010),
        background: Some(0xf0f0f0),
        special: None,
    });
    app.apply_nvim_event(NvimEvent::HlAttrDefine {
        id: HighlightId(1),
        attrs: HighlightAttrs {
            foreground: Some(0x202020),
            background: Some(0xe0e0e0),
            ui_name: Some("Normal".to_owned()),
            ..Default::default()
        },
    });

    assert_eq!(app.theme_background(), super::BACKGROUND);
    assert_eq!(app.theme_foreground(), super::TEXT);

    app.apply_nvim_event(NvimEvent::Flush);

    assert_eq!(app.theme_background(), 0xe0e0e0);
    assert_eq!(app.theme_foreground(), 0x202020);
}

#[test]
fn highlight_definitions_are_applied_before_the_next_flush() {
    let mut app = NvimGpui::default();
    let attrs = HighlightAttrs {
        foreground: Some(0xabcdef),
        bold: true,
        ..Default::default()
    };

    app.apply_nvim_event(NvimEvent::HlAttrDefine {
        id: HighlightId(9),
        attrs: attrs.clone(),
    });

    app.apply_nvim_event(NvimEvent::GridResized {
        grid: 1,
        width: 1,
        height: 1,
    });

    assert!(app.grid.highlight(HighlightId(9)).is_none());
    assert_eq!(
        app.pending_grid.as_ref().unwrap().highlight(HighlightId(9)),
        Some(attrs.clone())
    );

    app.apply_nvim_event(NvimEvent::Flush);

    assert_eq!(app.grid.highlight(HighlightId(9)), Some(attrs));
}

#[test]
fn default_colors_are_applied_to_the_pending_grid() {
    let mut app = NvimGpui::default();

    app.apply_nvim_event(NvimEvent::DefaultColorsSet {
        foreground: Some(0x101010),
        background: Some(0xf0f0f0),
        special: Some(0xff0000),
    });

    assert_eq!(
        app.pending_grid.as_ref().unwrap().default_colors(),
        (Some(0x101010), Some(0xf0f0f0), Some(0xff0000))
    );
}

#[test]
fn multigrid_layers_keep_window_positions_and_visibility() {
    let mut app = NvimGpui::default();

    app.apply_nvim_event(NvimEvent::GridResized {
        grid: 2,
        width: 20,
        height: 10,
    });
    app.apply_nvim_event(NvimEvent::WinPos {
        grid: 2,
        win: Vec::new(),
        row: 3,
        col: 4,
        width: 20,
        height: 10,
    });
    app.apply_nvim_event(NvimEvent::GridResized {
        grid: 3,
        width: 8,
        height: 4,
    });
    app.apply_nvim_event(NvimEvent::GridLine {
        grid: 3,
        row: 0,
        col_start: 0,
        cells: vec![GridLineCell::new("│", HighlightId(4), 1)],
        wraps_to_next: false,
    });
    app.apply_nvim_event(NvimEvent::WinFloatPos {
        grid: 3,
        win: Vec::new(),
        anchor: "NW".to_owned(),
        anchor_grid: 1,
        anchor_row: 0,
        anchor_col: 0,
        mouse_enabled: true,
        zindex: 50,
        compindex: 7,
        screen_row: 5,
        screen_col: 6,
    });
    app.apply_nvim_event(NvimEvent::Flush);

    assert_eq!(app.other_grids.get(&2).unwrap().width(), 20);
    assert_eq!(
        app.other_grids.get(&3).unwrap().rows()[0].cells()[0].text,
        "│"
    );
    let layers = app.visible_grid_layers();
    assert_eq!(
        layers.iter().map(|(grid, _, _)| *grid).collect::<Vec<_>>(),
        vec![2, 3]
    );
    assert_eq!(layers[0].2.row, 3);
    assert_eq!(layers[0].2.col, 4);
    assert_eq!(layers[1].2.row, 5);
    assert_eq!(layers[1].2.col, 6);
    assert!(layers[1].2.mouse_enabled);

    app.apply_nvim_event(NvimEvent::WinHide { grid: 3 });
    app.apply_nvim_event(NvimEvent::Flush);
    assert_eq!(
        app.visible_grid_layers()
            .iter()
            .map(|(grid, _, _)| *grid)
            .collect::<Vec<_>>(),
        vec![2]
    );

    app.apply_nvim_event(NvimEvent::WinClose { grid: 2 });
    app.apply_nvim_event(NvimEvent::Flush);
    assert!(!app.other_grids.contains_key(&2));
}

#[test]
fn multigrid_keeps_zindex_and_viewport_state_in_protocol_order() {
    let mut app = NvimGpui::default();

    for grid in [2, 3, 4] {
        app.apply_nvim_event(NvimEvent::GridResized {
            grid,
            width: 8,
            height: 3,
        });
    }
    app.apply_nvim_event(NvimEvent::WinFloatPos {
        grid: 2,
        win: Vec::new(),
        anchor: "NW".to_owned(),
        anchor_grid: 1,
        anchor_row: 0,
        anchor_col: 0,
        mouse_enabled: false,
        zindex: 100,
        compindex: 2,
        screen_row: 1,
        screen_col: 1,
    });
    app.apply_nvim_event(NvimEvent::WinFloatPos {
        grid: 3,
        win: Vec::new(),
        anchor: "NW".to_owned(),
        anchor_grid: 1,
        anchor_row: 0,
        anchor_col: 0,
        mouse_enabled: false,
        zindex: 40,
        compindex: 1,
        screen_row: 2,
        screen_col: 2,
    });

    // Margins can arrive before win_pos (as they do during initial
    // multigrid setup), so applying win_pos must merge with the existing
    // window state instead of replacing it.
    app.apply_nvim_event(NvimEvent::WinViewportMargins {
        grid: 4,
        win: Vec::new(),
        top: 1,
        bottom: 2,
        left: 3,
        right: 4,
    });
    app.apply_nvim_event(NvimEvent::WinViewport {
        grid: 4,
        win: Vec::new(),
        topline: 10,
        botline: 30,
        curline: 12,
        curcol: 5,
        line_count: 100,
        scroll_delta: -3,
    });
    app.apply_nvim_event(NvimEvent::WinPos {
        grid: 4,
        win: Vec::new(),
        row: 4,
        col: 5,
        width: 8,
        height: 3,
    });
    app.apply_nvim_event(NvimEvent::Flush);

    let layers = app.visible_grid_layers();
    assert_eq!(
        layers.iter().map(|(grid, _, _)| *grid).collect::<Vec<_>>(),
        vec![4, 3, 2]
    );
    assert_eq!(layers[0].2.z_index, 0);
    assert_eq!(layers[1].2.z_index, 40);
    assert_eq!(layers[2].2.z_index, 100);

    let placement = app.grid_placements.get(&4).expect("grid 4 placement");
    assert_eq!(
        placement.viewport,
        Some(GridViewport {
            topline: 10,
            botline: 30,
            curline: 12,
            curcol: 5,
            line_count: 100,
            scroll_delta: -3,
        })
    );
    assert_eq!(
        placement.viewport_margins,
        Some(GridViewportMargins {
            top: 1,
            bottom: 2,
            left: 3,
            right: 4,
        })
    );
    assert_eq!((placement.row, placement.col), (4, 5));
}

#[test]
fn viewport_scroll_keeps_the_previous_grid_for_the_transition() {
    let mut app = NvimGpui::default();

    app.apply_nvim_event(NvimEvent::GridResized {
        grid: 2,
        width: 8,
        height: 3,
    });
    app.apply_nvim_event(NvimEvent::GridLine {
        grid: 2,
        row: 0,
        col_start: 0,
        cells: vec![GridLineCell::new("old", HighlightId(1), 1)],
        wraps_to_next: false,
    });
    app.apply_nvim_event(NvimEvent::WinPos {
        grid: 2,
        win: Vec::new(),
        row: 0,
        col: 0,
        width: 8,
        height: 3,
    });
    app.apply_nvim_event(NvimEvent::WinViewportMargins {
        grid: 2,
        win: Vec::new(),
        top: 1,
        bottom: 1,
        left: 0,
        right: 0,
    });
    app.apply_nvim_event(NvimEvent::WinViewport {
        grid: 2,
        win: Vec::new(),
        topline: 0,
        botline: 1,
        curline: 0,
        curcol: 0,
        line_count: 10,
        scroll_delta: 0,
    });
    app.apply_nvim_event(NvimEvent::Flush);

    app.apply_nvim_event(NvimEvent::GridLine {
        grid: 2,
        row: 0,
        col_start: 0,
        cells: vec![GridLineCell::new("new", HighlightId(1), 1)],
        wraps_to_next: false,
    });
    app.apply_nvim_event(NvimEvent::WinViewport {
        grid: 2,
        win: Vec::new(),
        topline: 1,
        botline: 2,
        curline: 1,
        curcol: 0,
        line_count: 10,
        scroll_delta: 1,
    });
    app.apply_nvim_event(NvimEvent::Flush);

    let animation = app
        .viewport_animations
        .get(&2)
        .expect("viewport scroll should start an animation");
    assert_eq!(animation.scroll_delta, 1);
    assert_eq!(animation.previous_grid.rows()[0].cells()[0].text, "old");
    assert_eq!(app.other_grids[&2].rows()[0].cells()[0].text, "new");
    assert_eq!(app.grid_placements[&2].viewport_margins.unwrap().top, 1);
}

#[test]
fn viewport_margins_define_the_inner_render_area() {
    let placement = GridPlacement {
        viewport_margins: Some(GridViewportMargins {
            top: 1,
            bottom: 2,
            left: 3,
            right: 4,
        }),
        ..Default::default()
    };

    assert_eq!(NvimGpui::viewport_rect(placement, 100, 40), (3, 1, 93, 37));
}

#[test]
fn message_grid_position_makes_native_cmdline_grid_visible() {
    let mut app = NvimGpui::default();

    app.apply_nvim_event(NvimEvent::GridResized {
        grid: 3,
        width: 80,
        height: 4,
    });
    app.apply_nvim_event(NvimEvent::GridLine {
        grid: 3,
        row: 0,
        col_start: 0,
        cells: vec![GridLineCell::new(":echo", HighlightId(1), 1)],
        wraps_to_next: false,
    });
    app.apply_nvim_event(NvimEvent::MsgSetPos {
        grid: 3,
        row: 20,
        scrolled: false,
        sep_char: " ".to_owned(),
        zindex: 200,
        compindex: 11,
    });
    app.apply_nvim_event(NvimEvent::Flush);

    let layers = app.visible_grid_layers();
    assert_eq!(
        layers.iter().map(|(grid, _, _)| *grid).collect::<Vec<_>>(),
        vec![3]
    );
    assert_eq!(layers[0].2.row, 20);
    assert_eq!(layers[0].2.col, 0);
    assert_eq!(layers[0].2.width, 80);
    assert_eq!(layers[0].2.z_index, 200);
    assert_eq!(layers[0].2.compindex, 11);
    assert!(!layers[0].2.message_scrolled);
    assert_eq!(layers[0].2.message_separator, Some(' '));
    assert_eq!(app.other_grids[&3].rows()[0].cells()[0].text, ":echo");
}

#[test]
fn image_layer_recovers_from_a_covered_first_placeholder_cell() {
    let image = ImageId(17);
    let mut app = NvimGpui::default();
    app.image_store
        .insert_asset_with_format(image, vec![1, 2, 3], ImageFormatKind::Png);
    app.image_store.place(ImagePlacement {
        key: PlacementKey {
            image,
            placement: 4,
        },
        anchor: GridAnchor {
            grid: GridId(0),
            row: 0,
            column: 0,
        },
        columns: 3,
        rows: 2,
        z_index: 0,
        virtual_placeholder: true,
    });

    let mut model = crate::grid::GridModel::new(6, 3);
    let highlight = HighlightId(1839);
    model.set_highlight(
        highlight,
        HighlightAttrs {
            foreground: Some(image.0),
            ..Default::default()
        },
    );
    // The (1, 1) cell is covered by another decoration. The following
    // marker still identifies the image and encodes its (1, 2) offset.
    let marker = format!("{}{}{}", '\u{10eeee}', '\u{0305}', '\u{030d}');
    model.apply_grid_line(1, 1, &[GridLineCell::new(marker, highlight, 1)], false);
    model.set_cursor(2, 0);
    app.other_grids.insert(2, Rc::new(model));
    app.grid_placements.insert(
        2,
        GridPlacement {
            width: 6,
            height: 3,
            visible: true,
            ..Default::default()
        },
    );
    app.cursor_grid = 2;

    let layers = app.visible_image_layers();
    assert_eq!(layers.len(), 1);
    assert_eq!(
        (
            layers[0].image,
            layers[0].grid,
            layers[0].row,
            layers[0].column
        ),
        (image, 2, 1, 0)
    );

    let mut hidden_model = (*app.other_grids[&2]).clone();
    hidden_model.set_cursor(0, 0);
    app.other_grids.insert(2, Rc::new(hidden_model));
    assert!(app.visible_image_layers().is_empty());
}

#[test]
fn grid_destroy_removes_the_visible_grid_at_flush() {
    let mut app = NvimGpui::default();
    let attrs = HighlightAttrs {
        foreground: Some(0xabcdef),
        ..Default::default()
    };

    app.apply_nvim_event(NvimEvent::GridResized {
        grid: 1,
        width: 2,
        height: 1,
    });
    app.apply_nvim_event(NvimEvent::HlAttrDefine {
        id: HighlightId(7),
        attrs,
    });
    app.apply_nvim_event(NvimEvent::GridCursorGoto {
        grid: 1,
        row: 0,
        col: 1,
    });
    app.apply_nvim_event(NvimEvent::Flush);
    assert_eq!(app.grid.width(), 2);

    app.apply_nvim_event(NvimEvent::GridDestroy { grid: 1 });
    app.apply_nvim_event(NvimEvent::Flush);

    assert_eq!(app.grid.width(), 0);
    assert_eq!(app.grid.height(), 0);
    assert_eq!(app.grid.cursor(), None);
    assert!(app.grid.highlights().is_empty());
    assert_eq!(app.grid_size, None);
}
