pub mod grid;
pub mod image_store;
pub mod input;
pub mod nvim;

use gpui::{
    div, font, img, point, prelude::*, px, rgb, size, svg, App, Application, AssetSource, Bounds,
    Context, ElementInputHandler, Entity, FocusHandle, Focusable, Image, KeyDownEvent, MouseButton,
    Pixels, Render, SharedString, Subscription, Task, TitlebarOptions, Window, WindowBounds,
    WindowControlArea, WindowKind, WindowOptions,
};
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    env,
    ffi::OsString,
    fs,
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::Instant,
};

#[cfg(unix)]
use std::os::unix::fs::symlink;

use grid::GridElement;
use image_store::{GridId, ImageId, KittyEvent};
use input::{
    key_to_nvim_input, should_route_key_to_neovim, EntityInputHandler, InputRouter, InputTarget,
    SystemImeState,
};
use nvim::{NvimEvent, NvimProcess};

const BACKGROUND: u32 = 0x1e1e2e;
const SURFACE: u32 = 0x181825;
const SURFACE_BRIGHT: u32 = 0x313244;
const TEXT: u32 = 0xcdd6f4;
const MUTED_TEXT: u32 = 0x7f849c;
const ACCENT: u32 = 0x89b4fa;
const DEFAULT_GRID_WIDTH: u32 = 80;
const DEFAULT_GRID_HEIGHT: u32 = 24;
const DEFAULT_GRID_FONT_SIZE: f32 = 14.0;
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
const INITIAL_WINDOW_WIDTH: f32 = 800.0;
const INITIAL_WINDOW_HEIGHT: f32 = 600.0;
const MIN_WINDOW_WIDTH: f32 = 80.0;
const MIN_WINDOW_HEIGHT: f32 = 44.0;
const THEMED_TITLEBAR_HEIGHT: f32 = 32.0;
const DEFAULT_WINDOW_TITLE: &str = "gpvim";
const LOGO_ASSET: &str = "nvim-gpui.svg";

struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if path == LOGO_ASSET {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/nvim-gpui.svg"
            ))));
        }
        Ok(None)
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

#[derive(Debug, PartialEq, Eq)]
enum CliAction {
    Run(CliOptions),
    Help,
    Version,
}

#[derive(Debug, PartialEq, Eq)]
enum NvimConnection {
    Embed,
    Remote(String),
}

#[derive(Debug, PartialEq, Eq)]
struct CliOptions {
    debug_window: bool,
    connection: NvimConnection,
    nvim_command: Option<OsString>,
    working_directory: Option<OsString>,
    nvim_args: Vec<OsString>,
}

fn parse_cli<I>(args: I) -> Result<CliAction, String>
where
    I: IntoIterator<Item = OsString>,
{
    let mut debug_window = false;
    let mut connection = NvimConnection::Embed;
    let mut explicit_embed = false;
    let mut nvim_command = None;
    let mut working_directory = None;
    let mut nvim_args = Vec::new();
    let mut pass_through = false;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if !pass_through {
            match arg.to_str() {
                Some("--help") | Some("-h") => return Ok(CliAction::Help),
                Some("--version") | Some("-V") => return Ok(CliAction::Version),
                Some("--debug-window") => {
                    debug_window = true;
                    continue;
                }
                Some("--no-debug-window") => {
                    debug_window = false;
                    continue;
                }
                Some("--embed") => {
                    explicit_embed = true;
                    continue;
                }
                Some("--connect") => {
                    let address = args
                        .next()
                        .ok_or_else(|| "--connect requires an address".to_owned())?;
                    let address = address
                        .into_string()
                        .map_err(|_| "--connect address must be valid UTF-8".to_owned())?;
                    connection = NvimConnection::Remote(address);
                    continue;
                }
                Some(value) if value.starts_with("--connect=") => {
                    let address = value.trim_start_matches("--connect=");
                    if address.is_empty() {
                        return Err("--connect requires an address".to_owned());
                    }
                    connection = NvimConnection::Remote(address.to_owned());
                    continue;
                }
                Some("--nvim-command") => {
                    nvim_command = Some(
                        args.next()
                            .ok_or_else(|| "--nvim-command requires a path".to_owned())?,
                    );
                    continue;
                }
                Some(value) if value.starts_with("--nvim-command=") => {
                    let command = value.trim_start_matches("--nvim-command=");
                    if command.is_empty() {
                        return Err("--nvim-command requires a path".to_owned());
                    }
                    nvim_command = Some(OsString::from(command));
                    continue;
                }
                Some("--cwd") | Some("--working-directory") => {
                    working_directory = Some(
                        args.next()
                            .ok_or_else(|| "--cwd requires a path".to_owned())?,
                    );
                    continue;
                }
                Some(value) if value.starts_with("--cwd=") => {
                    let path = value.trim_start_matches("--cwd=");
                    if path.is_empty() {
                        return Err("--cwd requires a path".to_owned());
                    }
                    working_directory = Some(OsString::from(path));
                    continue;
                }
                Some(value) if value.starts_with("--working-directory=") => {
                    let path = value.trim_start_matches("--working-directory=");
                    if path.is_empty() {
                        return Err("--cwd requires a path".to_owned());
                    }
                    working_directory = Some(OsString::from(path));
                    continue;
                }
                Some("--") => {
                    pass_through = true;
                    continue;
                }
                _ => {}
            }
        }
        nvim_args.push(arg);
    }

    if explicit_embed && matches!(connection, NvimConnection::Remote(_)) {
        return Err("--embed and --connect cannot be used together".to_owned());
    }
    if matches!(connection, NvimConnection::Remote(_))
        && (nvim_command.is_some() || !nvim_args.is_empty())
    {
        return Err(
            "Neovim arguments and --nvim-command are only valid with embed mode".to_owned(),
        );
    }

    Ok(CliAction::Run(CliOptions {
        debug_window,
        connection,
        nvim_command,
        working_directory,
        nvim_args,
    }))
}

fn print_help() {
    println!(
        "Usage: gpvim [GPUI options] [--] [Neovim options]\n\n\
GPUI options:\n  --debug-window       Show the auxiliary debug window (opt-in)\n  --no-debug-window    Hide the auxiliary debug window\n  --embed              Start a local embedded Neovim (default)\n  --connect ADDRESS    Connect to a Neovim msgpack-rpc socket\n  --nvim-command PATH  Select the local Neovim executable for embed mode\n  --cwd PATH           Set the working directory for Neovim\n  -h, --help           Show this help\n  -V, --version        Show the GPUI version\n\n\
ADDRESS may be HOST:PORT, tcp:HOST:PORT, unix:/path, or a Unix socket path.\nAll other arguments are passed to embedded Neovim. Use -- to pass an argument\nthat would otherwise be interpreted as a GPUI option."
    );
}

fn gpvim_is_available_in_path() -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    let command_name = if cfg!(windows) { "gpvim.exe" } else { "gpvim" };
    env::split_paths(&path).any(|directory| is_executable_path(&directory.join(command_name)))
}

fn is_executable_path(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
}

fn bundled_gpvim_path() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(application) = env::var_os("NVIM_GPUI_APP") {
        candidates.push(PathBuf::from(application).join("Contents/Resources/gpvim"));
    }
    if let Ok(executable) = env::current_exe() {
        let executable = fs::canonicalize(&executable).unwrap_or(executable);
        candidates.extend(
            executable
                .ancestors()
                .filter(|path| {
                    path.extension().and_then(|extension| extension.to_str()) == Some("app")
                })
                .map(|application| application.join("Contents/Resources/gpvim")),
        );
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".cache/macos/nvim-gpui.app/Contents/Resources/gpvim"),
    );
    candidates.into_iter().find(|path| is_executable_path(path))
}

fn ensure_gpvim_helper() -> Result<(), String> {
    if gpvim_is_available_in_path() {
        return Ok(());
    }

    let Some(helper) = bundled_gpvim_path() else {
        return Ok(());
    };

    #[cfg(unix)]
    {
        let link = Path::new("/usr/local/bin/gpvim");
        if fs::symlink_metadata(link).is_ok() {
            return Err(format!(
                "gpvim is not executable from PATH and {} already exists",
                link.display()
            ));
        }
        if let Some(parent) = link.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "could not create gpvim helper directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        symlink(&helper, link).map_err(|error| {
            format!(
                "could not install gpvim symlink {} -> {}: {error}",
                link.display(),
                helper.display()
            )
        })?;
        Ok(())
    }

    #[cfg(not(unix))]
    {
        let _ = helper;
        Err(
            "gpvim is not executable from PATH; automatic helper links are only supported on Unix"
                .to_owned(),
        )
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
struct GridPlacement {
    row: i64,
    col: i64,
    width: u64,
    height: u64,
    compindex: i64,
    visible: bool,
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

impl Default for GridPlacement {
    fn default() -> Self {
        Self {
            row: 0,
            col: 0,
            width: 0,
            height: 0,
            compindex: -1,
            visible: false,
        }
    }
}

struct NvimGpui {
    focus_handle: Option<FocusHandle>,
    state: EditorState,
    grid: grid::GridModel,
    pending_grid: Option<grid::GridModel>,
    nvim: Option<NvimProcess>,
    input_router: InputRouter,
    system_ime: SystemImeState,
    rpc_status: String,
    api_level: Option<u64>,
    grid_size: Option<(u32, u32)>,
    guifont: Option<String>,
    guifontwide: Option<String>,
    window_title: String,
    window_icon: String,
    ui_options: HashMap<String, String>,
    linespace: f32,
    cursor_style_enabled: bool,
    cursor_modes: Vec<grid::CursorModeInfo>,
    cursor_mode_index: usize,
    cursor_blink_started_at: Instant,
    event_task: Option<Task<()>>,
    window_bounds_subscription: Option<Subscription>,
    last_resize: Option<(u32, u32)>,
    resolved_grid_font: Option<GuiFontSpec>,
    resolved_grid_wide_font: Option<GuiFontSpec>,
    shaping_cache: grid::SharedShapedLineCache,
    cursor_animation: Option<grid::CursorAnimation>,
    other_grids: HashMap<u64, grid::GridModel>,
    pending_other_grids: HashMap<u64, grid::GridModel>,
    grid_placements: HashMap<u64, GridPlacement>,
    pending_grid_placements: HashMap<u64, GridPlacement>,
    pending_destroyed_grids: HashSet<u64>,
    cursor_grid: u64,
    image_store: image_store::ImageStore,
    image_sources: HashMap<ImageId, Arc<Image>>,
}

impl Default for NvimGpui {
    fn default() -> Self {
        Self {
            focus_handle: None,
            state: EditorState::default(),
            grid: grid::demo_grid(),
            pending_grid: None,
            nvim: None,
            input_router: InputRouter::default(),
            system_ime: SystemImeState::default(),
            rpc_status: "rpc: starting".to_owned(),
            api_level: None,
            grid_size: None,
            guifont: None,
            guifontwide: None,
            window_title: DEFAULT_WINDOW_TITLE.to_owned(),
            window_icon: "nvim-gpui".to_owned(),
            ui_options: HashMap::new(),
            linespace: 0.0,
            cursor_style_enabled: false,
            cursor_modes: Vec::new(),
            cursor_mode_index: 0,
            cursor_blink_started_at: Instant::now(),
            event_task: None,
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
            cursor_grid: 1,
            image_store: image_store::ImageStore::new(),
            image_sources: HashMap::new(),
        }
    }
}

impl NvimGpui {
    fn new(nvim: Result<NvimProcess, String>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            focus_handle: Some(cx.focus_handle()),
            grid: grid::GridModel::new(DEFAULT_GRID_WIDTH as usize, DEFAULT_GRID_HEIGHT as usize),
            grid_size: Some((DEFAULT_GRID_WIDTH, DEFAULT_GRID_HEIGHT)),
            rpc_status: match &nvim {
                Ok(_) => "rpc: connecting".to_owned(),
                Err(error) => format!("rpc: {error}"),
            },
            nvim: nvim.ok(),
            ..Self::default()
        };

        if let Some(nvim) = this.nvim.as_ref() {
            let events = nvim.events();
            this.event_task = Some(cx.spawn(async move |weak, cx| {
                while let Ok(event) = events.recv().await {
                    let disconnected = matches!(&event, NvimEvent::Disconnected);
                    if weak
                        .update(cx, |this, cx| {
                            this.apply_nvim_event(event);
                            cx.notify();
                            if disconnected {
                                cx.quit();
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }

        this
    }

    fn current_grid_font(&mut self, window: &Window) -> GuiFontSpec {
        if let Some(font) = &self.resolved_grid_font {
            return font.clone();
        }

        let font = self
            .guifont
            .as_deref()
            .filter(|spec| !spec.trim().is_empty())
            .map(parse_guifont_spec)
            .unwrap_or_else(|| GuiFontSpec::system(window));
        self.resolved_grid_font = Some(font.clone());
        font
    }

    fn current_grid_wide_font(&mut self, window: &Window) -> GuiFontSpec {
        if let Some(font) = &self.resolved_grid_wide_font {
            return font.clone();
        }

        let font = if let Some(spec) = self
            .guifontwide
            .as_deref()
            .filter(|spec| !spec.trim().is_empty())
        {
            parse_guifont_spec(spec)
        } else {
            self.current_grid_font(window)
        };
        self.resolved_grid_wide_font = Some(font.clone());
        font
    }

    fn current_cursor_mode(&self) -> grid::CursorModeInfo {
        if !self.cursor_style_enabled {
            return grid::CursorModeInfo::default();
        }
        self.cursor_modes
            .get(self.cursor_mode_index)
            .copied()
            .unwrap_or_default()
    }

    fn sync_nvim_size(&mut self, window: &mut Window) {
        let gui_font = self.current_grid_font(window);
        let Some(nvim) = self.nvim.as_ref() else {
            return;
        };

        let cell_width = gui_font.cell_width(window);
        let line_height = gui_font.line_height(window, self.linespace);
        let viewport = window.viewport_size();
        let available_height = f32::from(viewport.height)
            - if themed_titlebar_enabled() {
                THEMED_TITLEBAR_HEIGHT
            } else {
                0.0
            };
        let width = (f32::from(viewport.width) / f32::from(cell_width))
            .floor()
            .max(1.0) as u32;
        let height = (available_height / f32::from(line_height)).floor().max(1.0) as u32;
        let size = (width, height);

        if self.last_resize == Some(size) {
            return;
        }

        match nvim.send_resize(width, height) {
            Ok(()) => self.last_resize = Some(size),
            Err(error) => self.rpc_status = format!("rpc resize error: {error}"),
        }
    }

    fn apply_nvim_event(&mut self, event: NvimEvent) {
        match event {
            NvimEvent::ApiReady { api_level } => {
                self.api_level = Some(api_level);
                self.rpc_status = format!("rpc: API {api_level}");
            }
            NvimEvent::UiAttached { width, height } => {
                self.rpc_status = format!("rpc: attached {width}×{height}");
            }
            NvimEvent::GridResized {
                grid,
                width,
                height,
            } => {
                if grid == 1 {
                    self.pending_grid = Some(self.new_styled_grid(width as usize, height as usize));
                    self.grid_size = Some((width, height));
                } else {
                    self.pending_grid_mut_for(grid)
                        .resize(width as usize, height as usize);
                }
                self.pending_destroyed_grids.remove(&grid);
            }
            NvimEvent::GridLine {
                grid,
                row,
                col_start,
                cells,
                wraps_to_next,
            } => {
                self.pending_grid_mut_for(grid).apply_grid_line(
                    row as usize,
                    col_start as usize,
                    &cells,
                    wraps_to_next,
                );
            }
            NvimEvent::GridClear { grid } => {
                self.pending_grid_mut_for(grid).clear();
            }
            NvimEvent::GridDestroy { grid } => {
                if grid == 1 {
                    self.pending_grid_mut().destroy();
                    self.grid_size = None;
                } else {
                    self.pending_other_grids.remove(&grid);
                    self.pending_destroyed_grids.insert(grid);
                }
            }
            NvimEvent::GridCursorGoto { grid, row, col } => {
                self.cursor_grid = grid;
                self.pending_grid_mut_for(grid)
                    .set_cursor(row as usize, col as usize);
            }
            NvimEvent::DefaultColorsSet {
                foreground,
                background,
                special,
            } => {
                self.set_default_colors_on_all_grids(foreground, background, special);
            }
            NvimEvent::HlAttrDefine { id, attrs } => {
                self.set_highlight_on_all_grids(id, attrs);
            }
            NvimEvent::GridScroll {
                grid,
                top,
                bot,
                left,
                right,
                rows,
                cols,
            } => {
                self.pending_grid_mut_for(grid).scroll(
                    top as usize,
                    bot as usize,
                    left as usize,
                    right as usize,
                    rows as isize,
                    cols as isize,
                );
            }
            NvimEvent::WinPos {
                grid,
                win: _,
                row,
                col,
                width,
                height,
            } => {
                self.set_grid_placement(
                    grid,
                    GridPlacement {
                        row: row as i64,
                        col: col as i64,
                        width,
                        height,
                        compindex: -1,
                        visible: true,
                    },
                );
            }
            NvimEvent::WinFloatPos {
                grid,
                win: _,
                anchor: _,
                anchor_grid: _,
                anchor_row: _,
                anchor_col: _,
                mouse_enabled: _,
                zindex: _,
                compindex,
                screen_row,
                screen_col,
            } => {
                let mut placement = self
                    .pending_grid_placements
                    .get(&grid)
                    .copied()
                    .or_else(|| self.grid_placements.get(&grid).copied())
                    .unwrap_or_default();
                placement.row = screen_row;
                placement.col = screen_col;
                placement.compindex = compindex;
                placement.visible = true;
                self.set_grid_placement(grid, placement);
            }
            NvimEvent::WinExternalPos { grid, win: _ } => {
                let mut placement = self
                    .pending_grid_placements
                    .get(&grid)
                    .copied()
                    .or_else(|| self.grid_placements.get(&grid).copied())
                    .unwrap_or_default();
                placement.visible = false;
                self.set_grid_placement(grid, placement);
            }
            NvimEvent::WinHide { grid } => {
                let mut placement = self
                    .pending_grid_placements
                    .get(&grid)
                    .copied()
                    .or_else(|| self.grid_placements.get(&grid).copied())
                    .unwrap_or_default();
                placement.visible = false;
                self.set_grid_placement(grid, placement);
            }
            NvimEvent::WinClose { grid } => {
                if grid == 1 {
                    self.pending_grid_mut().destroy();
                    self.grid_size = None;
                } else {
                    self.pending_other_grids.remove(&grid);
                    self.pending_destroyed_grids.insert(grid);
                }
            }
            NvimEvent::OptionSet { name, value } => {
                self.ui_options.insert(name.clone(), value.clone());
                match name.as_str() {
                    "guifont" => {
                        self.guifont = Some(value);
                        self.resolved_grid_font = None;
                        self.resolved_grid_wide_font = None;
                        self.last_resize = None;
                        self.shaping_cache.borrow_mut().clear();
                    }
                    "guifontwide" => {
                        self.guifontwide = Some(value);
                        self.resolved_grid_wide_font = None;
                        self.last_resize = None;
                        self.shaping_cache.borrow_mut().clear();
                    }
                    "linespace" => {
                        self.linespace = parse_non_negative_float(&value).unwrap_or(0.0);
                        self.last_resize = None;
                    }
                    "arabicshape" | "ambiwidth" | "emoji" | "termguicolors" => {
                        self.shaping_cache.borrow_mut().clear();
                    }
                    _ => {}
                }
            }
            NvimEvent::SetTitle { title } => {
                if !title.is_empty() {
                    self.window_title = title;
                }
            }
            NvimEvent::SetIcon { icon } => {
                self.window_icon = icon;
            }
            NvimEvent::ModeInfoSet {
                cursor_style_enabled,
                modes,
            } => {
                self.cursor_style_enabled = cursor_style_enabled;
                self.cursor_modes = modes;
                self.cursor_blink_started_at = Instant::now();
            }
            NvimEvent::ModeChanged { mode, mode_idx } => {
                self.input_router.set_nvim_mode(&mode);
                if self.input_router.target() != InputTarget::SystemIme {
                    self.system_ime.clear();
                }
                self.state.mode = mode.to_ascii_uppercase();
                self.cursor_mode_index = mode_idx as usize;
                self.cursor_blink_started_at = Instant::now();
            }
            NvimEvent::UiSend { data } => self.apply_ui_send(&data),
            NvimEvent::Flush => self.commit_pending_grid(),
            NvimEvent::Error(error) => {
                self.rpc_status = format!("rpc error: {error}");
            }
            NvimEvent::Disconnected => {
                self.rpc_status = "rpc: disconnected".to_owned();
            }
        }
    }

    fn apply_ui_send(&mut self, data: &str) {
        let events = self
            .image_store
            .consume_ui_data(data, GridId(self.cursor_grid));
        for event in events {
            match event {
                KittyEvent::AssetUpdated { image, .. } => {
                    if let Some(asset) = self.image_store.asset(image) {
                        let source =
                            Image::from_bytes(asset.format.gpui_format(), asset.encoded.clone());
                        self.image_sources.insert(image, Arc::new(source));
                    }
                }
                KittyEvent::AssetDeleted { image } => {
                    self.image_sources.remove(&image);
                }
                KittyEvent::AssetsCleared => {
                    self.image_sources.clear();
                }
                KittyEvent::TerminalResponse(response) => {
                    if let Some(nvim) = self.nvim.as_ref() {
                        if let Err(error) = nvim.send_term_event("termresponse", response) {
                            self.rpc_status = format!("rpc terminal response error: {error}");
                        }
                    }
                }
            }
        }
    }

    fn pending_grid_mut(&mut self) -> &mut grid::GridModel {
        self.pending_grid.get_or_insert_with(|| self.grid.clone())
    }

    fn new_styled_grid(&self, width: usize, height: usize) -> grid::GridModel {
        let source = self.pending_grid.as_ref().unwrap_or(&self.grid);
        let mut next_grid = grid::GridModel::new(width, height);
        for (id, attrs) in source.highlights() {
            next_grid.set_highlight(*id, attrs.clone());
        }
        let (foreground, background, special) = source.default_colors();
        next_grid.set_default_colors(foreground, background, special);
        next_grid
    }

    fn pending_grid_mut_for(&mut self, grid: u64) -> &mut grid::GridModel {
        if grid == 1 {
            return self.pending_grid_mut();
        }

        if !self.pending_other_grids.contains_key(&grid) {
            let model = self
                .other_grids
                .get(&grid)
                .cloned()
                .unwrap_or_else(|| self.new_styled_grid(0, 0));
            self.pending_other_grids.insert(grid, model);
        }

        self.pending_other_grids
            .get_mut(&grid)
            .expect("pending grid was inserted")
    }

    fn set_default_colors_on_all_grids(
        &mut self,
        foreground: Option<u32>,
        background: Option<u32>,
        special: Option<u32>,
    ) {
        self.pending_grid_mut()
            .set_default_colors(foreground, background, special);
        for model in self.other_grids.values_mut() {
            model.set_default_colors(foreground, background, special);
        }
        for model in self.pending_other_grids.values_mut() {
            model.set_default_colors(foreground, background, special);
        }
    }

    fn set_highlight_on_all_grids(&mut self, id: grid::HighlightId, attrs: grid::HighlightAttrs) {
        self.pending_grid_mut().set_highlight(id, attrs.clone());
        for model in self.other_grids.values_mut() {
            model.set_highlight(id, attrs.clone());
        }
        for model in self.pending_other_grids.values_mut() {
            model.set_highlight(id, attrs.clone());
        }
    }

    fn set_grid_placement(&mut self, grid: u64, placement: GridPlacement) {
        self.pending_grid_placements.insert(grid, placement);
        self.pending_destroyed_grids.remove(&grid);
    }

    fn commit_pending_grid(&mut self) {
        if let Some(grid) = self.pending_grid.take() {
            let previous_cursor = self.grid.cursor_visual_position();
            let next_cursor = grid.cursor_visual_position();
            if previous_cursor != next_cursor {
                self.cursor_animation = match (previous_cursor, next_cursor) {
                    (Some(from), Some(target)) => self
                        .cursor_animation
                        .map(|animation| animation.retarget(target))
                        .or_else(|| Some(grid::CursorAnimation::new(from, target))),
                    _ => None,
                };
            }

            if let Some(cursor) = grid.cursor() {
                self.state.line = cursor.row + 1;
                self.state.column = cursor.col + 1;
            }
            self.grid = grid;
        }

        for (grid, model) in std::mem::take(&mut self.pending_other_grids) {
            if !self.pending_destroyed_grids.contains(&grid) {
                self.other_grids.insert(grid, model);
            }
        }

        for (grid, placement) in std::mem::take(&mut self.pending_grid_placements) {
            if !self.pending_destroyed_grids.contains(&grid) {
                self.grid_placements.insert(grid, placement);
            }
        }

        for grid in std::mem::take(&mut self.pending_destroyed_grids) {
            self.other_grids.remove(&grid);
            self.grid_placements.remove(&grid);
        }
    }

    fn visible_grid_layers(&self) -> Vec<(u64, grid::GridModel, GridPlacement)> {
        let mut layers = self
            .other_grids
            .iter()
            .filter_map(|(grid, model)| {
                let placement = self.grid_placements.get(grid).copied()?;
                placement.visible.then(|| (*grid, model.clone(), placement))
            })
            .collect::<Vec<_>>();
        layers.sort_by(|left, right| {
            left.2
                .compindex
                .cmp(&right.2.compindex)
                .then_with(|| left.0.cmp(&right.0))
        });
        layers
    }

    fn visible_image_layers(&self) -> Vec<ImageLayer> {
        let mut layers = Vec::new();

        for placement in self.image_store.placements() {
            if placement.is_virtual_placeholder()
                || self.image_store.asset(placement.key.image).is_none()
                || !self.grid_is_visible(placement.anchor.grid.0)
            {
                continue;
            }
            layers.push(ImageLayer {
                image: placement.key.image,
                grid: placement.anchor.grid.0,
                row: placement.anchor.row as usize,
                column: placement.anchor.column as usize,
                columns: placement.columns,
                rows: placement.rows,
                z_index: placement.z_index,
            });
        }

        let mut models = vec![(1, &self.grid)];
        models.extend(
            self.other_grids
                .iter()
                .filter_map(|(grid, model)| self.grid_is_visible(*grid).then_some((*grid, model))),
        );

        for (grid, model) in models {
            for (row, grid_row) in model.rows().iter().enumerate() {
                for (column, cell) in grid_row.cells().iter().enumerate() {
                    let Some((row_offset, column_offset)) =
                        image_store::placeholder_position(&cell.text)
                    else {
                        continue;
                    };
                    if row_offset != 1 || column_offset != 1 {
                        continue;
                    }
                    let Some(image) = model
                        .highlight(cell.highlight)
                        .and_then(|attrs| attrs.foreground)
                        .map(ImageId)
                    else {
                        continue;
                    };
                    let Some(placement) = self.image_store.virtual_placements_for(image).next()
                    else {
                        continue;
                    };
                    layers.push(ImageLayer {
                        image,
                        grid,
                        row,
                        column,
                        columns: placement.columns,
                        rows: placement.rows,
                        z_index: placement.z_index,
                    });
                }
            }
        }

        layers.sort_by(|left, right| {
            left.z_index
                .cmp(&right.z_index)
                .then_with(|| left.grid.cmp(&right.grid))
                .then_with(|| left.row.cmp(&right.row))
                .then_with(|| left.column.cmp(&right.column))
        });
        layers
    }

    fn grid_is_visible(&self, grid: u64) -> bool {
        grid == 1
            || self
                .grid_placements
                .get(&grid)
                .is_some_and(|placement| placement.visible)
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, _cx: &mut Context<Self>) {
        let target = self.input_router.target();
        if !should_route_key_to_neovim(target, &event.keystroke) {
            return;
        }

        if let Some(nvim) = self.nvim.as_ref() {
            if let Err(error) = nvim.send_input(key_to_nvim_input(&event.keystroke)) {
                self.rpc_status = format!("rpc input error: {error}");
            }
        }
        // Prevent GPUI's default key action from competing with Neovim for
        // editor-owned shortcuts such as Ctrl-W, Cmd-W, and function keys.
        // This only applies after the event reaches this window; OS-global
        // shortcuts remain owned by the operating system.
        window.prevent_default();
    }
}

impl Focusable for NvimGpui {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle
            .clone()
            .expect("NvimGpui focus handle is initialized for app entities")
    }
}

impl EntityInputHandler for NvimGpui {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let (text, actual_range) = self.system_ime.text_for_range(range_utf16);
        adjusted_range.replace(actual_range);
        Some(text)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<gpui::UTF16Selection> {
        Some(self.system_ime.selected_text_range())
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.system_ime.marked_text_range()
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        // The local buffer only represents the active composition. Once the
        // platform cancels its marked range, there is no text to retain here.
        self.system_ime.clear();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_router.target() != InputTarget::SystemIme {
            return;
        }

        self.system_ime.replace_text(range, text);
        if !text.is_empty() {
            if let Some(nvim) = self.nvim.as_ref() {
                if let Err(error) = nvim.send_input(text.to_owned()) {
                    self.rpc_status = format!("rpc input error: {error}");
                }
            }
        }
        self.system_ime.clear();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_router.target() == InputTarget::SystemIme {
            self.system_ime
                .replace_and_mark_text(range, new_text, new_selected_range);
            cx.notify();
        }
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<gpui::Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<gpui::Pixels>> {
        let cursor = self
            .grid
            .cursor()
            .unwrap_or(grid::GridCursor { row: 0, col: 0 });
        let font_spec = self.current_grid_font(window);
        let cell_width = font_spec.cell_width(window);
        let line_height = font_spec.line_height(window, self.linespace);
        let origin = gpui::point(
            element_bounds.origin.x + cell_width * cursor.col,
            element_bounds.origin.y + line_height * cursor.row,
        );
        Some(Bounds::new(origin, size(cell_width, line_height)))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let font_spec = self.current_grid_font(window);
        let cell_width = font_spec.cell_width(window);
        let column = (f32::from(point.x) / f32::from(cell_width))
            .max(0.0)
            .floor() as usize;
        let byte_offset = self
            .system_ime
            .text()
            .char_indices()
            .nth(column)
            .map(|(offset, _)| offset)
            .unwrap_or(self.system_ime.text().len());
        Some(input::utf8_to_utf16_offset(
            self.system_ime.text(),
            byte_offset,
        ))
    }
}

impl Render for NvimGpui {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_window_title(&self.window_title);
        self.sync_nvim_size(window);

        let gui_font = self.current_grid_font(window);
        let gui_wide_font = self.current_grid_wide_font(window);
        let line_height = gui_font.line_height(window, self.linespace);
        let shaping_cache = Rc::clone(&self.shaping_cache);
        let cursor_mode = self.current_cursor_mode();
        let cursor_blink_started_at = self.cursor_blink_started_at;

        let entity = cx.entity();
        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(TEXT))
            .capture_key_down(cx.listener(Self::on_key_down));

        if let Some(focus_handle) = self.focus_handle.as_ref() {
            root = root.track_focus(focus_handle);
        }

        if themed_titlebar_enabled() {
            root = root.child(themed_titlebar(self.window_title.clone(), BACKGROUND));
        }

        let cell_width = gui_font.cell_width(window);
        let mut editor = div()
            .flex_1()
            .relative()
            .overflow_hidden()
            .font_family(gui_font.family.clone())
            .text_size(px(gui_font.size))
            .line_height(line_height)
            .child(
                GridElement::new(self.grid.clone())
                    .with_metrics(cell_width, line_height)
                    .with_wide_font(gui_wide_font.family.clone(), px(gui_wide_font.size))
                    .with_shaping_cache(Rc::clone(&shaping_cache))
                    .with_cursor_animation(self.cursor_animation)
                    .with_cursor_visible(self.cursor_grid == 1)
                    .with_cursor_mode(if self.cursor_grid == 1 {
                        cursor_mode
                    } else {
                        grid::CursorModeInfo::default()
                    })
                    .with_cursor_blink_started_at(cursor_blink_started_at)
                    .with_nerd_font_mode(true)
                    .with_input_handler(move |bounds, window, cx| {
                        let focus_handle = {
                            let view = entity.read(cx);
                            if view.input_router.target() == InputTarget::SystemIme {
                                view.focus_handle.clone()
                            } else {
                                None
                            }
                        };
                        if let Some(focus_handle) = focus_handle {
                            window.handle_input(
                                &focus_handle,
                                ElementInputHandler::new(bounds, entity.clone()),
                                cx,
                            );
                        }
                    }),
            );

        for (grid_id, model, placement) in self.visible_grid_layers() {
            let width = placement.width.max(model.width() as u64);
            let height = placement.height.max(model.height() as u64);
            let layer = div()
                .absolute()
                .left(px(placement.col as f32 * f32::from(cell_width)))
                .top(px(placement.row as f32 * f32::from(line_height)))
                .w(px(width as f32 * f32::from(cell_width)))
                .h(px(height as f32 * f32::from(line_height)))
                .child(
                    GridElement::new(model)
                        .with_metrics(cell_width, line_height)
                        .with_wide_font(gui_wide_font.family.clone(), px(gui_wide_font.size))
                        .with_shaping_cache(Rc::clone(&shaping_cache))
                        .with_cursor_visible(self.cursor_grid == grid_id)
                        .with_cursor_mode(if self.cursor_grid == grid_id {
                            cursor_mode
                        } else {
                            grid::CursorModeInfo::default()
                        })
                        .with_nerd_font_mode(true),
                );
            editor = editor.child(layer);
        }

        for layer in self.visible_image_layers() {
            let Some(source) = self.image_sources.get(&layer.image).cloned() else {
                continue;
            };
            let (grid_row, grid_col) = if layer.grid == 1 {
                (0, 0)
            } else {
                self.grid_placements
                    .get(&layer.grid)
                    .map(|placement| (placement.row.max(0), placement.col.max(0)))
                    .unwrap_or((0, 0))
            };
            editor = editor.child(
                img(source)
                    .absolute()
                    .left(px(
                        (layer.column as f32 + grid_col as f32) * f32::from(cell_width)
                    ))
                    .top(px(
                        (layer.row as f32 + grid_row as f32) * f32::from(line_height)
                    ))
                    .w(px(layer.columns as f32 * f32::from(cell_width)))
                    .h(px(layer.rows as f32 * f32::from(line_height)))
                    .object_fit(gpui::ObjectFit::Fill),
            );
        }

        root.child(editor)
    }
}

struct DebugWindow {
    source: Entity<NvimGpui>,
    _source_subscription: Subscription,
}

impl DebugWindow {
    fn new(source: Entity<NvimGpui>, cx: &mut Context<Self>) -> Self {
        let source_subscription = cx.observe(&source, |_, _, cx| cx.notify());
        Self {
            source,
            _source_subscription: source_subscription,
        }
    }
}

impl Render for DebugWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.source.read(cx);
        let guifont = view
            .resolved_grid_font
            .as_ref()
            .map(|font| format!("{}:h{}", font.family, font.size))
            .or_else(|| view.guifont.clone())
            .unwrap_or_else(|| "system monospace (resolving)".to_owned());
        let guifontwide = view
            .resolved_grid_wide_font
            .as_ref()
            .map(|font| format!("{}:h{}", font.family, font.size))
            .or_else(|| view.guifontwide.clone())
            .unwrap_or_else(|| "same as guifont (fallback)".to_owned());
        let grid_size = view
            .grid_size
            .map(|(width, height)| format!("{width}×{height}"))
            .unwrap_or_else(|| "pending".to_owned());
        let ime_status = if view.system_ime.is_empty() {
            "IME: system".to_owned()
        } else {
            format!("IME composing: {}", view.system_ime.text())
        };
        let debug_message = format!(
            "{}  ·  grid {grid_size}  ·  guifont {guifont}  ·  guifontwide {guifontwide}  ·  file {}  ·  {} {}:{}  ·  {ime_status}  ·  API {}",
            view.rpc_status,
            view.state.file,
            view.state.mode,
            view.state.line,
            view.state.column,
            view.api_level.unwrap_or_default()
        );

        let debug_content = div()
            .flex_1()
            .flex()
            .items_center()
            .overflow_hidden()
            .px_3()
            .bg(rgb(SURFACE))
            .text_color(rgb(MUTED_TEXT))
            .border_b_1()
            .border_color(rgb(SURFACE_BRIGHT))
            .child(div().text_color(rgb(ACCENT)).child("DEBUG  nvim-gpui  ·  "))
            .child(debug_message);
        let mut root = div().size_full().flex().flex_col().bg(rgb(SURFACE));
        if themed_titlebar_enabled() {
            root = root.child(themed_titlebar("nvim-gpui debug".to_owned(), SURFACE));
        }
        root.child(debug_content)
    }
}

fn is_monospace_family(window: &Window, family: &str, font_size: Pixels) -> bool {
    let text_system = window.text_system();
    let font_id = text_system.resolve_font(&font(family.to_owned()));
    let Some(reference) = text_system
        .advance(font_id, font_size, '0')
        .ok()
        .map(|advance| f32::from(advance.width))
    else {
        return false;
    };

    ['M', 'i', 'W', ' '].into_iter().all(|character| {
        text_system
            .advance(font_id, font_size, character)
            .ok()
            .map(|advance| (f32::from(advance.width) - reference).abs() <= 0.01)
            .unwrap_or(false)
    })
}

fn parse_guifont_spec(spec: &str) -> GuiFontSpec {
    let first_font = spec.split(',').next().unwrap_or(spec);
    let mut parts = first_font.split(':');
    let family = parts.next().unwrap_or_default().replace("\\:", ":");
    let family = if family.trim().is_empty() {
        GuiFontSpec::default().family
    } else {
        family
    };
    let size = parts
        .find_map(|part| part.strip_prefix('h'))
        .and_then(|size| size.parse::<f32>().ok())
        .filter(|size| *size > 0.0)
        .unwrap_or(DEFAULT_GRID_FONT_SIZE);

    GuiFontSpec { family, size }
}

fn line_height_from_metrics(glyph_height: Pixels, font_size: Pixels, linespace: f32) -> Pixels {
    let minimum_line_height = font_size * 1.2;

    // GPUI 0.2.2 does not expose the font's line-gap metric. Use the actual
    // glyph metrics and a compact 1.2em minimum cell height instead of
    // scaling a historical default ratio. Neovim's `linespace` remains the
    // only user-configured extra spacing.
    px(
        (f32::from(glyph_height.max(minimum_line_height)) + linespace)
            .ceil()
            .max(1.0),
    )
}

fn parse_non_negative_float(value: &str) -> Option<f32> {
    let value = value.parse::<f32>().ok()?;
    value.is_finite().then_some(value.max(0.0))
}

fn themed_titlebar_enabled() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

fn themed_titlebar_options(title: &'static str) -> TitlebarOptions {
    TitlebarOptions {
        title: Some(title.into()),
        appears_transparent: themed_titlebar_enabled(),
        ..Default::default()
    }
}

fn themed_titlebar(title: String, background: u32) -> impl IntoElement {
    let title_area = div()
        .flex_1()
        .h_full()
        .flex()
        .items_center()
        .justify_start()
        .pl(px(if cfg!(target_os = "macos") {
            76.0
        } else {
            12.0
        }))
        .text_color(rgb(TEXT))
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |event, window, _cx| {
            if event.click_count == 2 {
                // On macOS this forwards to AppKit's standard titlebar
                // double-click action (normally zoom/maximize). On Windows,
                // WindowControlArea::Drag lets the native caption handling do
                // the same job, so this is harmless there.
                window.titlebar_double_click();
            }
        })
        .child(svg().path(LOGO_ASSET).w(px(116.0)).h(px(28.0)))
        .child(div().w(px(8.0)))
        .child(title);

    let titlebar = div()
        .w_full()
        .h(px(THEMED_TITLEBAR_HEIGHT))
        .flex()
        .items_center()
        .bg(rgb(background))
        .child(title_area);

    #[cfg(target_os = "windows")]
    let titlebar = titlebar
        .child(window_control_button(
            "—",
            WindowControlArea::Min,
            background,
        ))
        .child(window_control_button(
            "□",
            WindowControlArea::Max,
            background,
        ))
        .child(window_control_button(
            "×",
            WindowControlArea::Close,
            background,
        ));

    titlebar
}

#[cfg(target_os = "windows")]
fn window_control_button(
    label: &'static str,
    area: WindowControlArea,
    background: u32,
) -> impl IntoElement {
    div()
        .w(px(46.0))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(background))
        .text_color(rgb(TEXT))
        .window_control_area(area)
        .child(label)
}

fn main() {
    let cli = match parse_cli(env::args_os().skip(1)) {
        Ok(CliAction::Run(options)) => options,
        Ok(CliAction::Help) => {
            print_help();
            return;
        }
        Ok(CliAction::Version) => {
            println!("nvim-gpui {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Err(error) => {
            eprintln!("gpvim: {error}");
            print_help();
            return;
        }
    };

    if let Err(error) = ensure_gpvim_helper() {
        eprintln!("[gpvim] {error}");
    }

    if let Some(path) = cli.working_directory.as_deref() {
        if let Err(error) = env::set_current_dir(path) {
            eprintln!("gpvim: failed to set working directory: {error}");
            return;
        }
    }

    let nvim = match cli.connection {
        NvimConnection::Embed => {
            let nvim_command = cli
                .nvim_command
                .or_else(|| env::var_os("NVIM_GPUI_NVIM"))
                .unwrap_or_else(|| OsString::from("nvim"));
            NvimProcess::spawn_with_command(
                DEFAULT_GRID_WIDTH,
                DEFAULT_GRID_HEIGHT,
                nvim_command,
                cli.nvim_args,
            )
        }
        NvimConnection::Remote(address) => {
            NvimProcess::connect(DEFAULT_GRID_WIDTH, DEFAULT_GRID_HEIGHT, &address)
        }
    };
    let show_debug_window = cli.debug_window;

    Application::new()
        .with_assets(AppAssets)
        .run(move |cx: &mut App| {
            // The debug window is auxiliary. Closing either top-level window ends
            // the session and drops the shared Neovim process.
            cx.on_window_closed(|cx| cx.quit()).detach();

            let main_bounds = Bounds::centered(
                None,
                size(px(INITIAL_WINDOW_WIDTH), px(INITIAL_WINDOW_HEIGHT)),
                cx,
            );
            let debug_y = if main_bounds.origin.y > px(104.0) {
                main_bounds.origin.y - px(96.0)
            } else {
                px(8.0)
            };
            let debug_bounds = Bounds::new(
                point(main_bounds.origin.x, debug_y),
                size(main_bounds.size.width, px(88.0)),
            );

            let nvim_view = cx.new(|cx| NvimGpui::new(nvim, cx));

            let main_window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(main_bounds)),
                        titlebar: Some(themed_titlebar_options(DEFAULT_WINDOW_TITLE)),
                        is_resizable: true,
                        window_min_size: Some(size(px(MIN_WINDOW_WIDTH), px(MIN_WINDOW_HEIGHT))),
                        ..Default::default()
                    },
                    |_, _| nvim_view.clone(),
                )
                .expect("failed to open nvim-gpui window");

            main_window
                .update(cx, |view, window, cx| {
                    view.window_bounds_subscription =
                        Some(cx.observe_window_bounds(window, |view, window, _cx| {
                            view.sync_nvim_size(window)
                        }));
                    view.sync_nvim_size(window);
                    if let Some(focus_handle) = view.focus_handle.as_ref() {
                        window.focus(focus_handle);
                    }
                })
                .expect("failed to focus nvim-gpui window");

            if show_debug_window {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(debug_bounds)),
                        titlebar: Some(themed_titlebar_options("nvim-gpui debug")),
                        kind: WindowKind::Floating,
                        focus: false,
                        is_resizable: false,
                        ..Default::default()
                    },
                    |_, cx| cx.new(|cx| DebugWindow::new(nvim_view.clone(), cx)),
                )
                .expect("failed to open nvim-gpui debug window");
            }

            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use super::{
        parse_cli, parse_guifont_spec, CliAction, CliOptions, EditorState, NvimConnection,
        NvimEvent, NvimGpui, DEFAULT_WINDOW_TITLE,
    };
    use crate::grid::{CursorModeInfo, CursorShape, GridLineCell, HighlightAttrs, HighlightId};
    use gpui::px;
    use std::ffi::OsString;

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
}
