use crate::{
    grid,
    gui::{AboutWindow, SettingsWindow},
    image_store,
    image_store::{GridId, ImageId, KittyEvent},
    input::{InputRouter, InputRouterConfig, InputTarget, SystemImeState},
    nvim::{
        self, DisconnectReason, NvimEvent, NvimFloatAnchor, NvimFloatPosition, NvimProcess,
        NvimTheme, NvimVersion, SessionId,
    },
    platform, settings, update_check,
    widgets::{ACCENT, BACKGROUND, MUTED_TEXT, SURFACE, SURFACE_BRIGHT, TEXT},
    CliOptions, NvimConnection,
};
use gpui::{
    div, font, img, point, prelude::*, px, rgb, size, App, Application, AssetSource, Bounds,
    Context, Entity, FocusHandle, Image, KeyDownEvent, MouseButton, Pixels, Point, Render,
    SharedString, Subscription, Task, TitlebarOptions, Window, WindowBounds, WindowControlArea,
    WindowDecorations, WindowHandle, WindowKind, WindowOptions,
};
use nvim_gpui::rime::{RimeContextSnapshot, RimeService};
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    ffi::OsString,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

pub(crate) const DEFAULT_GRID_WIDTH: u32 = 80;
pub(crate) const DEFAULT_GRID_HEIGHT: u32 = 24;
pub(crate) const DEFAULT_GRID_FONT_SIZE: f32 = 14.0;
pub(crate) const DEFAULT_GRID_CELL_WIDTH: f32 = DEFAULT_GRID_FONT_SIZE * 0.6;
pub(crate) const DEFAULT_GRID_LINE_HEIGHT: f32 = 20.0;
pub(crate) const PREFERRED_SYSTEM_MONOSPACE_FONTS: &[&str] = &[
    "Menlo",
    "SF Mono",
    "Monaco",
    "Cascadia Mono",
    "Consolas",
    "Noto Sans Mono",
    "DejaVu Sans Mono",
    "Liberation Mono",
    "Courier New",
];
pub(crate) const MIN_WINDOW_WIDTH: f32 = 80.0;
pub(crate) const MIN_WINDOW_HEIGHT: f32 = 44.0;
pub(crate) const THEMED_TITLEBAR_HEIGHT: f32 = 32.0;
pub(crate) const DEFAULT_WINDOW_TITLE: &str = "gpvim";
pub(crate) const LOGO_ASSET: &str = "neovim-gpui.png";
pub(crate) const WINDOW_CONTROL_MINIMIZE_ASSET: &str = "window-controls/minimize.svg";
pub(crate) const WINDOW_CONTROL_MAXIMIZE_ASSET: &str = "window-controls/maximize.svg";
pub(crate) const WINDOW_CONTROL_RESTORE_ASSET: &str = "window-controls/restore.svg";
pub(crate) const WINDOW_CONTROL_CLOSE_ASSET: &str = "window-controls/close.svg";
pub(crate) const DEBUG_WINDOW_HEIGHT: f32 = 240.0;
pub(crate) const MAX_EVENTS_PER_UI_UPDATE: usize = 2048;
pub(crate) const VIEWPORT_SCROLL_DURATION: Duration = Duration::from_millis(140);

struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let asset: &'static [u8] = match path {
            LOGO_ASSET => include_bytes!("../assets/icons/neovim-gpui.png"),
            WINDOW_CONTROL_MINIMIZE_ASSET => {
                include_bytes!("../assets/icons/window-controls/minimize.svg")
            }
            WINDOW_CONTROL_MAXIMIZE_ASSET => {
                include_bytes!("../assets/icons/window-controls/maximize.svg")
            }
            WINDOW_CONTROL_RESTORE_ASSET => {
                include_bytes!("../assets/icons/window-controls/restore.svg")
            }
            WINDOW_CONTROL_CLOSE_ASSET => {
                include_bytes!("../assets/icons/window-controls/close.svg")
            }
            _ => return Ok(None),
        };
        Ok(Some(Cow::Borrowed(asset)))
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EditorState {
    pub(crate) mode: String,
    pub(crate) file: &'static str,
    pub(crate) line: usize,
    pub(crate) column: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum QuitDialogState {
    #[default]
    Hidden,
    Checking,
    Confirm {
        modified_buffers: Vec<String>,
        error: Option<String>,
    },
    Saving,
    Quitting,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GuiFontSpec {
    pub(crate) family: String,
    pub(crate) size: f32,
}

impl Default for GuiFontSpec {
    fn default() -> Self {
        Self {
            family: "Menlo".to_owned(),
            size: DEFAULT_GRID_FONT_SIZE,
        }
    }
}

impl GuiFontSpec {
    pub(crate) fn system(window: &Window) -> Self {
        let available_fonts = window.text_system().all_font_names();
        let font_size = px(DEFAULT_GRID_FONT_SIZE);
        let family = PREFERRED_SYSTEM_MONOSPACE_FONTS
            .iter()
            .find_map(|preferred| {
                let installed = available_fonts
                    .iter()
                    .find(|name| name.eq_ignore_ascii_case(preferred))?;
                is_monospace_family(window, installed, font_size).then(|| installed.clone())
            })
            .or_else(|| {
                available_fonts
                    .iter()
                    .find(|name| is_monospace_family(window, name, font_size))
                    .cloned()
            })
            // GPUI normally exposes at least one system monospace font. Keep
            // a last-resort value for unusual platforms with incomplete font
            // enumeration; the normal path above is runtime-selected.
            .unwrap_or_else(|| Self::default().family);

        Self {
            family,
            size: DEFAULT_GRID_FONT_SIZE,
        }
    }

    pub(crate) fn line_height(&self, window: &Window, linespace: f32) -> Pixels {
        let font = font(self.family.clone());
        let font_size = px(self.size);
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font);
        let glyph_height =
            text_system.ascent(font_id, font_size) + text_system.descent(font_id, font_size);

        line_height_from_metrics(glyph_height, font_size, linespace)
    }

    pub(crate) fn cell_width(&self, window: &Window) -> Pixels {
        let font = font(self.family.clone());
        let font_size = px(self.size);
        window
            .text_system()
            .ch_advance(window.text_system().resolve_font(&font), font_size)
            .map(|advance| advance.max(px(1.0)))
            .unwrap_or_else(|_| px(self.size * 0.6))
    }
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            mode: "NORMAL".to_owned(),
            file: "src/main.rs",
            line: 1,
            column: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GridViewport {
    pub(crate) topline: u64,
    pub(crate) botline: u64,
    pub(crate) curline: u64,
    pub(crate) curcol: u64,
    pub(crate) line_count: u64,
    pub(crate) scroll_delta: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GridViewportMargins {
    pub(crate) top: u64,
    pub(crate) bottom: u64,
    pub(crate) left: u64,
    pub(crate) right: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GridPlacement {
    pub(crate) row: i64,
    pub(crate) col: i64,
    pub(crate) width: u64,
    pub(crate) height: u64,
    /// Configured float stacking level. `compindex` remains the primary
    /// render key because Neovim computes it as the exact compositing order;
    /// `z_index` is retained for the protocol's same-order/group semantics.
    pub(crate) z_index: i64,
    pub(crate) compindex: i64,
    pub(crate) float_position: Option<NvimFloatPosition>,
    /// Whether Neovim allows this floating grid to receive mouse input.
    /// Neovim uses this when the client sends `nvim_input_mouse` with grid 0.
    pub(crate) mouse_enabled: bool,
    pub(crate) kind: compositor::GridLayerKind,
    pub(crate) visible: bool,
    pub(crate) viewport: Option<GridViewport>,
    pub(crate) viewport_margins: Option<GridViewportMargins>,
    pub(crate) message_scrolled: bool,
    pub(crate) message_separator: Option<char>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ImageLayer {
    pub(crate) image: ImageId,
    pub(crate) grid: u64,
    pub(crate) row: usize,
    pub(crate) column: usize,
    pub(crate) columns: u32,
    pub(crate) rows: u32,
    pub(crate) z_index: i32,
}

#[derive(Clone)]
pub(crate) struct ViewportAnimation {
    pub(crate) previous_grid: Rc<grid::GridModel>,
    pub(crate) scroll_delta: i64,
    pub(crate) started_at: Instant,
    pub(crate) presented: bool,
}

impl ViewportAnimation {
    fn progress(&self, now: Instant) -> f32 {
        (now.saturating_duration_since(self.started_at).as_secs_f32()
            / VIEWPORT_SCROLL_DURATION.as_secs_f32())
        .min(1.0)
    }

    pub(crate) fn is_active(&self, now: Instant) -> bool {
        self.progress(now) < 1.0
    }

    pub(crate) fn mark_presented(&mut self, now: Instant) {
        if !self.presented {
            self.started_at = now;
            self.presented = true;
        }
    }

    pub(crate) fn offsets(
        &self,
        now: Instant,
        max_delta: usize,
        line_height: Pixels,
    ) -> (Pixels, Pixels) {
        let progress = self.progress(now);
        let progress = progress * progress * (3.0 - 2.0 * progress);
        let delta = self
            .scroll_delta
            .clamp(-(max_delta as i64), max_delta as i64) as f32;
        (
            px(-delta * progress * f32::from(line_height)),
            px(delta * (1.0 - progress) * f32::from(line_height)),
        )
    }
}

/// Identity of a Neovim extmark that is being presented as a multicursor.
/// The namespace and mark identifiers are session-local; the grid keeps the
/// screen coordinate local to the window that emitted the event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct MultiCursorKey {
    pub(crate) grid: u64,
    pub(crate) ns_id: u64,
    pub(crate) mark_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MultiCursorPosition {
    pub(crate) key: MultiCursorKey,
    pub(crate) row: usize,
    pub(crate) col: usize,
}

impl Default for GridPlacement {
    fn default() -> Self {
        Self {
            row: 0,
            col: 0,
            width: 0,
            height: 0,
            z_index: 0,
            compindex: -1,
            float_position: None,
            mouse_enabled: true,
            kind: compositor::GridLayerKind::Window,
            visible: false,
            viewport: None,
            viewport_margins: None,
            message_scrolled: false,
            message_separator: None,
        }
    }
}

pub(crate) struct NvimGpui {
    pub(crate) focus_handle: Option<FocusHandle>,
    pub(crate) state: EditorState,
    pub(crate) grid: Rc<grid::GridModel>,
    pub(crate) pending_grid: Option<Rc<grid::GridModel>>,
    pub(crate) nvim: Option<NvimProcess>,
    /// Identity of the Neovim connection whose events and asynchronous
    /// responses are allowed to mutate this view.
    pub(crate) nvim_session_id: Option<SessionId>,
    pub(crate) input_router: InputRouter,
    pub(crate) last_modifiers: gpui::Modifiers,
    pub(crate) rime_service: Option<RimeService>,
    pub(crate) rime_context: Option<RimeContextSnapshot>,
    pub(crate) rime_menu_open: bool,
    pub(crate) rime_menu_message: Option<String>,
    pub(crate) system_ime: SystemImeState,
    pub(crate) rpc_status: String,
    pub(crate) api_level: Option<u64>,
    pub(crate) nvim_version: Option<NvimVersion>,
    pub(crate) grid_size: Option<(u32, u32)>,
    pub(crate) guifont: Option<String>,
    pub(crate) guifontwide: Option<String>,
    pub(crate) window_title: String,
    pub(crate) window_icon: String,
    pub(crate) ui_options: HashMap<String, String>,
    pub(crate) display_options: grid::DisplayOptions,
    pub(crate) mouse_option: String,
    pub(crate) mouse_enabled: bool,
    pub(crate) mouse_capture: Option<u64>,
    pub(crate) nvim_mode: String,
    pub(crate) quit_dialog: QuitDialogState,
    pub(crate) scroll_remainder: gpui::Point<f32>,
    pub(crate) linespace: f32,
    pub(crate) cursor_style_enabled: bool,
    pub(crate) cursor_modes: Vec<grid::CursorModeInfo>,
    pub(crate) cursor_mode_index: usize,
    pub(crate) cursor_blink_started_at: Instant,
    pub(crate) event_task: Option<Task<()>>,
    pub(crate) rime_init_task: Option<Task<()>>,
    pub(crate) rime_deploy_task: Option<Task<()>>,
    pub(crate) open_urls_task: Option<Task<()>>,
    pub(crate) file_drop_notice: Option<String>,
    pub(crate) file_drop_notice_generation: u64,
    pub(crate) file_drop_notice_task: Option<Task<()>>,
    pub(crate) clipboard_task: Option<Task<()>>,
    pub(crate) reconnect_task: Option<Task<()>>,
    pub(crate) reconnect_attempt: u32,
    pub(crate) window_bounds_subscription: Option<Subscription>,
    pub(crate) last_resize: Option<(u32, u32)>,
    pub(crate) resolved_grid_font: Option<GuiFontSpec>,
    pub(crate) resolved_grid_wide_font: Option<GuiFontSpec>,
    pub(crate) shaping_cache: grid::SharedShapedLineCache,
    pub(crate) cursor_animation: Option<grid::CursorAnimation>,
    pub(crate) multicursor_positions: HashMap<MultiCursorKey, MultiCursorPosition>,
    pub(crate) pending_multicursor_positions: Option<HashMap<MultiCursorKey, MultiCursorPosition>>,
    pub(crate) unresolved_multicursor_positions: HashMap<MultiCursorKey, MultiCursorPosition>,
    pub(crate) multicursor_namespace_ids: HashMap<String, u64>,
    pub(crate) multicursor_namespace_task: Option<Task<()>>,
    pub(crate) multicursor_reconcile_task: Option<Task<()>>,
    pub(crate) multicursor_reconcile_dirty: bool,
    pub(crate) other_grids: HashMap<u64, Rc<grid::GridModel>>,
    pub(crate) pending_other_grids: HashMap<u64, Rc<grid::GridModel>>,
    pub(crate) grid_placements: HashMap<u64, GridPlacement>,
    pub(crate) pending_grid_placements: HashMap<u64, GridPlacement>,
    pub(crate) pending_destroyed_grids: HashSet<u64>,
    pub(crate) viewport_animations: HashMap<u64, ViewportAnimation>,
    pub(crate) cursor_grid: u64,
    pub(crate) pending_cursor_grid: Option<u64>,
    /// Grid whose element owns the currently registered system IME handler.
    /// This is separate from `cursor_grid` because the platform input handler
    /// lives for the painted frame, while Neovim cursor state can change
    /// between frames.
    pub(crate) ime_input_grid: Option<u64>,
    pub(crate) ime_coordinates_dirty: bool,
    pub(crate) image_store: image_store::ImageStore,
    pub(crate) image_sources: HashMap<ImageId, Arc<Image>>,
    pub(crate) nerd_font_family: Option<String>,
    pub(crate) glyph_coverage_cache: grid::SharedGlyphCoverageCache,
    pub(crate) settings: settings::Settings,
    pub(crate) update_http_client: Option<Arc<dyn gpui::http_client::HttpClient>>,
    pub(crate) update_status: update_check::Status,
    pub(crate) update_check_task: Option<Task<()>>,
    pub(crate) logger: Option<flexi_logger::LoggerHandle>,
    pub(crate) bundled_nerd_font_registered: bool,
    pub(crate) settings_save_error: Option<String>,
    pub(crate) cli_install_error: Option<String>,
    pub(crate) settings_window: Option<WindowHandle<SettingsWindow>>,
    pub(crate) about_window: Option<WindowHandle<AboutWindow>>,
    pub(crate) theme: NvimTheme,
    pub(crate) pending_theme: Option<NvimTheme>,
    pub(crate) pending_redraw: Option<state::PendingRedrawState>,
    pub(crate) nvim_grid_ready: bool,
    pub(crate) startup_resize_target: Option<(u32, u32)>,
    pub(crate) startup_flush_seen: bool,
    pub(crate) startup_grid_content_seen: bool,
    pub(crate) startup_redraw_pending: bool,
    pub(crate) startup_maximize_pending: bool,
}

impl NvimGpui {
    pub(crate) fn settings_snapshot(&self) -> (settings::Settings, Option<String>, Option<String>) {
        (
            self.settings.clone(),
            self.settings_save_error.clone(),
            self.cli_install_error.clone(),
        )
    }

    pub(crate) fn update_status(&self) -> update_check::Status {
        self.update_status.clone()
    }

    pub(crate) fn settings_value(&self) -> settings::Settings {
        self.settings.clone()
    }

    pub(crate) fn set_cli_install_error(&mut self, error: Option<String>) {
        self.cli_install_error = error;
    }

    pub(crate) fn settings_window_handle(&self) -> Option<WindowHandle<SettingsWindow>> {
        self.settings_window
    }

    pub(crate) fn set_settings_window_handle(&mut self, handle: WindowHandle<SettingsWindow>) {
        self.settings_window = Some(handle);
    }

    pub(crate) fn about_window_handle(&self) -> Option<WindowHandle<AboutWindow>> {
        self.about_window
    }

    pub(crate) fn set_about_window_handle(&mut self, handle: WindowHandle<AboutWindow>) {
        self.about_window = Some(handle);
    }
}

impl Default for NvimGpui {
    fn default() -> Self {
        Self {
            focus_handle: None,
            state: EditorState::default(),
            grid: Rc::new(grid::GridModel::new(
                DEFAULT_GRID_WIDTH as usize,
                DEFAULT_GRID_HEIGHT as usize,
            )),
            pending_grid: None,
            nvim: None,
            nvim_session_id: None,
            input_router: InputRouter::default(),
            last_modifiers: gpui::Modifiers::none(),
            rime_service: None,
            rime_context: None,
            rime_menu_open: false,
            rime_menu_message: None,
            system_ime: SystemImeState::default(),
            rpc_status: "rpc: starting".to_owned(),
            api_level: None,
            nvim_version: None,
            grid_size: None,
            guifont: None,
            guifontwide: None,
            window_title: DEFAULT_WINDOW_TITLE.to_owned(),
            window_icon: "nvim-gpui".to_owned(),
            ui_options: HashMap::new(),
            display_options: grid::DisplayOptions::default(),
            mouse_option: "nvi".to_owned(),
            mouse_enabled: true,
            mouse_capture: None,
            nvim_mode: "n".to_owned(),
            quit_dialog: QuitDialogState::default(),
            scroll_remainder: point(0.0, 0.0),
            linespace: 0.0,
            cursor_style_enabled: false,
            cursor_modes: Vec::new(),
            cursor_mode_index: 0,
            cursor_blink_started_at: Instant::now(),
            event_task: None,
            rime_init_task: None,
            rime_deploy_task: None,
            open_urls_task: None,
            file_drop_notice: None,
            file_drop_notice_generation: 0,
            file_drop_notice_task: None,
            clipboard_task: None,
            reconnect_task: None,
            reconnect_attempt: 0,
            window_bounds_subscription: None,
            last_resize: None,
            resolved_grid_font: None,
            resolved_grid_wide_font: None,
            shaping_cache: grid::ShapedLineCache::shared(),
            cursor_animation: None,
            multicursor_positions: HashMap::new(),
            pending_multicursor_positions: None,
            unresolved_multicursor_positions: HashMap::new(),
            multicursor_namespace_ids: HashMap::new(),
            multicursor_namespace_task: None,
            multicursor_reconcile_task: None,
            multicursor_reconcile_dirty: false,
            other_grids: HashMap::new(),
            pending_other_grids: HashMap::new(),
            grid_placements: HashMap::new(),
            pending_grid_placements: HashMap::new(),
            pending_destroyed_grids: HashSet::new(),
            viewport_animations: HashMap::new(),
            cursor_grid: 1,
            pending_cursor_grid: None,
            ime_input_grid: None,
            ime_coordinates_dirty: true,
            image_store: image_store::ImageStore::new(),
            image_sources: HashMap::new(),
            nerd_font_family: None,
            glyph_coverage_cache: grid::GlyphCoverageCache::shared(),
            settings: settings::Settings::default(),
            update_http_client: None,
            update_status: update_check::Status::default(),
            update_check_task: None,
            logger: None,
            bundled_nerd_font_registered: false,
            settings_save_error: None,
            cli_install_error: None,
            settings_window: None,
            about_window: None,
            theme: NvimTheme::default(),
            pending_theme: None,
            pending_redraw: None,
            nvim_grid_ready: true,
            startup_resize_target: None,
            startup_flush_seen: false,
            startup_grid_content_seen: false,
            startup_redraw_pending: false,
            startup_maximize_pending: false,
        }
    }
}

pub(crate) mod compositor;
mod startup;
mod state;
pub(crate) mod windows;
mod workspace;

pub(crate) use startup::run;
#[cfg(target_os = "linux")]
pub(crate) use windows::themed_resize_handles;
use windows::DebugWindow;
pub(crate) use windows::{
    initial_window_size_for_grid, is_monospace_family, line_height_from_metrics,
    parse_guifont_spec, parse_non_negative_float,
};
pub(crate) use windows::{
    themed_titlebar, themed_titlebar_enabled, themed_titlebar_options, themed_window_decorations,
};
