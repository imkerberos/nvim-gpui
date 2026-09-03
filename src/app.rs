use crate::{
    grid,
    grid::GridElement,
    helper, image_store,
    image_store::{GridId, ImageId, KittyEvent},
    input,
    input::{
        key_to_nvim_input, should_route_key_to_neovim, EntityInputHandler, InputRouter,
        InputTarget, SystemImeState,
    },
    nvim::{self, DisconnectReason, NvimEvent, NvimProcess, NvimTheme, NvimVersion},
    platform, settings, CliOptions, NvimConnection,
};
use gpui::{
    div, font, img, point, prelude::*, px, rgb, size, App, Application, AssetSource, Bounds,
    Context, ElementInputHandler, Entity, FocusHandle, Focusable, Image, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Render, ScrollWheelEvent, SharedString,
    Subscription, Task, TitlebarOptions, Window, WindowBounds, WindowControlArea, WindowHandle,
    WindowKind, WindowOptions,
};
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    ffi::OsString,
    ops::Range,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

const BACKGROUND: u32 = 0x1e1e2e;
const SURFACE: u32 = 0x181825;
const SURFACE_BRIGHT: u32 = 0x313244;
const TEXT: u32 = 0xcdd6f4;
const MUTED_TEXT: u32 = 0x7f849c;
const ACCENT: u32 = 0x89b4fa;
const DEFAULT_GRID_WIDTH: u32 = 80;
const DEFAULT_GRID_HEIGHT: u32 = 24;
const DEFAULT_GRID_FONT_SIZE: f32 = 14.0;
const DEFAULT_GRID_CELL_WIDTH: f32 = DEFAULT_GRID_FONT_SIZE * 0.6;
const DEFAULT_GRID_LINE_HEIGHT: f32 = 20.0;
const PREFERRED_SYSTEM_MONOSPACE_FONTS: &[&str] = &[
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
const MIN_WINDOW_WIDTH: f32 = 80.0;
const MIN_WINDOW_HEIGHT: f32 = 44.0;
const THEMED_TITLEBAR_HEIGHT: f32 = 32.0;
const DEFAULT_WINDOW_TITLE: &str = "gpvim";
const REPOSITORY_URL: &str = env!("CARGO_PKG_REPOSITORY");
const LOGO_ASSET: &str = "neovim-gpui.png";
const DEBUG_WINDOW_HEIGHT: f32 = 240.0;
const MAX_EVENTS_PER_UI_UPDATE: usize = 2048;
const VIEWPORT_SCROLL_DURATION: Duration = Duration::from_millis(140);

struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if path == LOGO_ASSET {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/neovim-gpui.png"
            ))));
        }
        Ok(None)
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorState {
    mode: String,
    file: &'static str,
    line: usize,
    column: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct GuiFontSpec {
    family: String,
    size: f32,
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
    fn system(window: &Window) -> Self {
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

    fn line_height(&self, window: &Window, linespace: f32) -> Pixels {
        let font = font(self.family.clone());
        let font_size = px(self.size);
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font);
        let glyph_height =
            text_system.ascent(font_id, font_size) + text_system.descent(font_id, font_size);

        line_height_from_metrics(glyph_height, font_size, linespace)
    }

    fn cell_width(&self, window: &Window) -> Pixels {
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
struct GridViewport {
    topline: u64,
    botline: u64,
    curline: u64,
    curcol: u64,
    line_count: u64,
    scroll_delta: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GridViewportMargins {
    top: u64,
    bottom: u64,
    left: u64,
    right: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GridPlacement {
    row: i64,
    col: i64,
    width: u64,
    height: u64,
    /// Configured float stacking level. `compindex` remains the primary
    /// render key because Neovim computes it as the exact compositing order;
    /// `z_index` is retained for the protocol's same-order/group semantics.
    z_index: i64,
    compindex: i64,
    /// Whether Neovim allows this floating grid to receive mouse input.
    /// Neovim uses this when the client sends `nvim_input_mouse` with grid 0.
    mouse_enabled: bool,
    visible: bool,
    viewport: Option<GridViewport>,
    viewport_margins: Option<GridViewportMargins>,
    message_scrolled: bool,
    message_separator: Option<char>,
}

#[derive(Debug, Clone, Copy)]
struct ImageLayer {
    image: ImageId,
    grid: u64,
    row: usize,
    column: usize,
    columns: u32,
    rows: u32,
    z_index: i32,
}

#[derive(Clone)]
struct ViewportAnimation {
    previous_grid: Rc<grid::GridModel>,
    scroll_delta: i64,
    started_at: Instant,
}

impl ViewportAnimation {
    fn progress(&self, now: Instant) -> f32 {
        (now.saturating_duration_since(self.started_at).as_secs_f32()
            / VIEWPORT_SCROLL_DURATION.as_secs_f32())
        .min(1.0)
    }

    fn is_active(&self, now: Instant) -> bool {
        self.progress(now) < 1.0
    }

    fn offsets(&self, now: Instant, max_delta: usize, line_height: Pixels) -> (Pixels, Pixels) {
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

impl Default for GridPlacement {
    fn default() -> Self {
        Self {
            row: 0,
            col: 0,
            width: 0,
            height: 0,
            z_index: 0,
            compindex: -1,
            mouse_enabled: true,
            visible: false,
            viewport: None,
            viewport_margins: None,
            message_scrolled: false,
            message_separator: None,
        }
    }
}

struct NvimGpui {
    focus_handle: Option<FocusHandle>,
    state: EditorState,
    grid: Rc<grid::GridModel>,
    pending_grid: Option<Rc<grid::GridModel>>,
    nvim: Option<NvimProcess>,
    input_router: InputRouter,
    system_ime: SystemImeState,
    rpc_status: String,
    api_level: Option<u64>,
    nvim_version: Option<NvimVersion>,
    grid_size: Option<(u32, u32)>,
    guifont: Option<String>,
    guifontwide: Option<String>,
    window_title: String,
    window_icon: String,
    ui_options: HashMap<String, String>,
    mouse_option: String,
    mouse_enabled: bool,
    nvim_mode: String,
    scroll_remainder: gpui::Point<f32>,
    linespace: f32,
    cursor_style_enabled: bool,
    cursor_modes: Vec<grid::CursorModeInfo>,
    cursor_mode_index: usize,
    cursor_blink_started_at: Instant,
    event_task: Option<Task<()>>,
    reconnect_task: Option<Task<()>>,
    reconnect_attempt: u32,
    window_bounds_subscription: Option<Subscription>,
    last_resize: Option<(u32, u32)>,
    resolved_grid_font: Option<GuiFontSpec>,
    resolved_grid_wide_font: Option<GuiFontSpec>,
    shaping_cache: grid::SharedShapedLineCache,
    cursor_animation: Option<grid::CursorAnimation>,
    other_grids: HashMap<u64, Rc<grid::GridModel>>,
    pending_other_grids: HashMap<u64, Rc<grid::GridModel>>,
    grid_placements: HashMap<u64, GridPlacement>,
    pending_grid_placements: HashMap<u64, GridPlacement>,
    pending_destroyed_grids: HashSet<u64>,
    viewport_animations: HashMap<u64, ViewportAnimation>,
    cursor_grid: u64,
    pending_cursor_grid: Option<u64>,
    /// Grid whose element owns the currently registered system IME handler.
    /// This is separate from `cursor_grid` because the platform input handler
    /// lives for the painted frame, while Neovim cursor state can change
    /// between frames.
    ime_input_grid: Option<u64>,
    ime_coordinates_dirty: bool,
    image_store: image_store::ImageStore,
    image_sources: HashMap<ImageId, Arc<Image>>,
    nerd_font_family: Option<String>,
    glyph_coverage_cache: grid::SharedGlyphCoverageCache,
    settings: settings::Settings,
    bundled_nerd_font_registered: bool,
    settings_save_error: Option<String>,
    cli_install_error: Option<String>,
    settings_window: Option<WindowHandle<SettingsWindow>>,
    about_window: Option<WindowHandle<AboutWindow>>,
    theme: NvimTheme,
    pending_theme: Option<NvimTheme>,
    nvim_grid_ready: bool,
    startup_resize_target: Option<(u32, u32)>,
    startup_flush_seen: bool,
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
            input_router: InputRouter::default(),
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
            mouse_option: "nvi".to_owned(),
            mouse_enabled: true,
            nvim_mode: "n".to_owned(),
            scroll_remainder: point(0.0, 0.0),
            linespace: 0.0,
            cursor_style_enabled: false,
            cursor_modes: Vec::new(),
            cursor_mode_index: 0,
            cursor_blink_started_at: Instant::now(),
            event_task: None,
            reconnect_task: None,
            reconnect_attempt: 0,
            window_bounds_subscription: None,
            last_resize: None,
            resolved_grid_font: None,
            resolved_grid_wide_font: None,
            shaping_cache: grid::ShapedLineCache::shared(),
            cursor_animation: None,
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
            bundled_nerd_font_registered: false,
            settings_save_error: None,
            cli_install_error: None,
            settings_window: None,
            about_window: None,
            theme: NvimTheme::default(),
            pending_theme: None,
            nvim_grid_ready: true,
            startup_resize_target: None,
            startup_flush_seen: false,
        }
    }
}

mod editor;
mod startup;
mod state;
mod windows;

pub(crate) use startup::run;
use windows::{
    initial_window_size_for_grid, is_monospace_family, line_height_from_metrics,
    parse_guifont_spec, parse_non_negative_float, themed_titlebar, themed_titlebar_enabled,
    themed_titlebar_options, AboutWindow, DebugWindow, SettingsWindow,
};

#[cfg(test)]
mod tests;
