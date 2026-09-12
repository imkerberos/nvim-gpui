use crate::grid::{CursorModeInfo, GridLineCell, HighlightAttrs, HighlightId};

use async_channel::Sender;
use rmpv::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Instant;

use super::{NvimCapabilities, NvimVersion};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NvimFloatAnchor {
    NorthWest,
    NorthEast,
    SouthWest,
    SouthEast,
}

/// Normalized position information for a floating grid.
///
/// Neovim 0.10 and 0.11 send an anchor position, while newer versions also
/// send the final screen position and compositor index. The protocol adapter
/// converts both wire formats into this small internal representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NvimFloatPosition {
    Screen {
        row: i64,
        col: i64,
    },
    Anchored {
        anchor: NvimFloatAnchor,
        anchor_grid: u64,
        row: i64,
        col: i64,
    },
}

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
    Request {
        params: Value,
        token: u64,
        request: Arc<RequestState>,
    },
    TermEvent {
        event: String,
        value: String,
    },
    DrainCoalesced,
    Shutdown,
}

pub(super) struct RequestState {
    pub(super) method: String,
    pub(super) deadline: Instant,
    pub(super) response: Sender<Result<Value, String>>,
    pub(super) completed: AtomicBool,
}

impl RequestState {
    pub(super) fn new(
        method: String,
        deadline: Instant,
        response: Sender<Result<Value, String>>,
    ) -> Self {
        Self {
            method,
            deadline,
            response,
            completed: AtomicBool::new(false),
        }
    }

    pub(super) fn complete(&self, result: Result<Value, String>) {
        if !self.completed.swap(true, Ordering::AcqRel) {
            let _ = self.response.try_send(result);
        }
    }
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
        position: NvimFloatPosition,
        mouse_enabled: bool,
        zindex: i64,
        compindex: i64,
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
    WinExtmark {
        grid: u64,
        win: Vec<u8>,
        ns_id: u64,
        mark_id: u64,
        row: i64,
        col: i64,
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
    Disconnected {
        reason: DisconnectReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisconnectReason {
    Requested,
    CleanExit,
    TransportClosed,
    UnexpectedExit,
    ProtocolError(String),
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
