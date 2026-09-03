use crate::grid::{CursorModeInfo, GridLineCell, HighlightAttrs, HighlightId};

use super::{NvimCapabilities, NvimVersion};

pub(super) enum NvimCommand {
    Input(String),
    Mouse {
        button: String,
        action: String,
        modifier: String,
        grid: u64,
        row: u64,
        col: u64,
    },
    Resize {
        width: u32,
        height: u32,
    },
    TermEvent {
        event: String,
        value: String,
    },
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NvimEvent {
    ApiReady {
        version: NvimVersion,
        capabilities: NvimCapabilities,
    },
    UiAttached {
        width: u32,
        height: u32,
    },
    GridResized {
        grid: u64,
        width: u32,
        height: u32,
    },
    GridLine {
        grid: u64,
        row: u64,
        col_start: u64,
        cells: Vec<GridLineCell>,
        wraps_to_next: bool,
    },
    DefaultColorsSet {
        foreground: Option<u32>,
        background: Option<u32>,
        special: Option<u32>,
    },
    HlAttrDefine {
        id: HighlightId,
        attrs: HighlightAttrs,
    },
    GridClear {
        grid: u64,
    },
    GridDestroy {
        grid: u64,
    },
    GridCursorGoto {
        grid: u64,
        row: u64,
        col: u64,
    },
    GridScroll {
        grid: u64,
        top: u64,
        bot: u64,
        left: u64,
        right: u64,
        rows: i64,
        cols: i64,
    },
    WinPos {
        grid: u64,
        win: Vec<u8>,
        row: u64,
        col: u64,
        width: u64,
        height: u64,
    },
    WinFloatPos {
        grid: u64,
        win: Vec<u8>,
        anchor: String,
        anchor_grid: u64,
        anchor_row: i64,
        anchor_col: i64,
        mouse_enabled: bool,
        zindex: i64,
        compindex: i64,
        screen_row: i64,
        screen_col: i64,
    },
    WinViewport {
        grid: u64,
        win: Vec<u8>,
        topline: u64,
        botline: u64,
        curline: u64,
        curcol: u64,
        line_count: u64,
        scroll_delta: i64,
    },
    WinViewportMargins {
        grid: u64,
        win: Vec<u8>,
        top: u64,
        bottom: u64,
        left: u64,
        right: u64,
    },
    MsgSetPos {
        grid: u64,
        row: u64,
        scrolled: bool,
        sep_char: String,
        zindex: i64,
        compindex: i64,
    },
    WinExternalPos {
        grid: u64,
        win: Vec<u8>,
    },
    WinHide {
        grid: u64,
    },
    WinClose {
        grid: u64,
    },
    OptionSet {
        name: String,
        value: String,
    },
    MouseEnabled(bool),
    SetTitle {
        title: String,
    },
    SetIcon {
        icon: String,
    },
    ModeInfoSet {
        cursor_style_enabled: bool,
        modes: Vec<CursorModeInfo>,
    },
    ModeChanged {
        mode: String,
        mode_idx: u64,
    },
    UiSend {
        data: String,
    },
    Flush,
    Error(String),
    Disconnected,
}

/// Effective theme colors collected from Neovim's initial UI redraw.
///
/// The default colors are the fallback for highlight groups that omit an
/// explicit color. `Normal` is kept separately because most colorschemes use
/// it to override the editor surface, while `NormalFloat` supplies the
/// surface for native floating-grid backgrounds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NvimTheme {
    pub default_foreground: Option<u32>,
    pub default_background: Option<u32>,
    pub normal_foreground: Option<u32>,
    pub normal_background: Option<u32>,
    pub normal_float_background: Option<u32>,
}
