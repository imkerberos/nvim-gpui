use super::EditorState;
use crate::{
    app::{DEFAULT_GRID_HEIGHT, DEFAULT_GRID_WIDTH},
    editor::image_store::{GridId, ImageStore, KittyEvent},
    grid,
    input::{InputContext, InputRouter, InputRouterConfig},
    nvim::{
        DisconnectReason, NvimEvent, NvimFloatAnchor, NvimFloatPosition, NvimTheme, NvimVersion,
    },
};
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

/// The semantic layer owner used by both the protocol model and the editor
/// compositor. It deliberately contains no rendering data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GridLayerKind {
    Main,
    Window,
    Float,
    Message,
    External,
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
    pub(crate) z_index: i64,
    pub(crate) compindex: i64,
    pub(crate) float_position: Option<NvimFloatPosition>,
    pub(crate) mouse_enabled: bool,
    pub(crate) kind: GridLayerKind,
    pub(crate) visible: bool,
    pub(crate) viewport: Option<GridViewport>,
    pub(crate) viewport_margins: Option<GridViewportMargins>,
    pub(crate) message_scrolled: bool,
    pub(crate) message_separator: Option<char>,
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
            kind: GridLayerKind::Window,
            visible: false,
            viewport: None,
            viewport_margins: None,
            message_scrolled: false,
            message_separator: None,
        }
    }
}

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

/// Mutable screen model owned by the protocol reducer. GPUI images and
/// render snapshots intentionally do not belong here.
pub(crate) struct ProtocolScreenState {
    pub(crate) grid: Rc<grid::GridModel>,
    pub(crate) pending_grid: Option<Rc<grid::GridModel>>,
    pub(crate) grid_size: Option<(u32, u32)>,
    pub(crate) pending_grid_size: Option<Option<(u32, u32)>>,
    pub(crate) other_grids: HashMap<u64, Rc<grid::GridModel>>,
    pub(crate) pending_other_grids: HashMap<u64, Rc<grid::GridModel>>,
    pub(crate) grid_placements: HashMap<u64, GridPlacement>,
    pub(crate) pending_grid_placements: HashMap<u64, GridPlacement>,
    pub(crate) pending_destroyed_grids: HashSet<u64>,
    pub(crate) image_store: ImageStore,
    pub(crate) pending_ui_data: Vec<(String, GridId)>,
}

impl Default for ProtocolScreenState {
    fn default() -> Self {
        Self {
            grid: Rc::new(grid::GridModel::new(
                DEFAULT_GRID_WIDTH as usize,
                DEFAULT_GRID_HEIGHT as usize,
            )),
            pending_grid: None,
            grid_size: None,
            pending_grid_size: None,
            other_grids: HashMap::new(),
            pending_other_grids: HashMap::new(),
            grid_placements: HashMap::new(),
            pending_grid_placements: HashMap::new(),
            pending_destroyed_grids: HashSet::new(),
            image_store: ImageStore::new(),
            pending_ui_data: Vec::new(),
        }
    }
}

pub(crate) struct ProtocolCursorState {
    pub(crate) cursor_style_enabled: bool,
    pub(crate) cursor_modes: Vec<grid::CursorModeInfo>,
    pub(crate) cursor_mode_index: usize,
    pub(crate) multicursor_positions: HashMap<MultiCursorKey, MultiCursorPosition>,
    pub(crate) pending_multicursor_positions: Option<HashMap<MultiCursorKey, MultiCursorPosition>>,
    pub(crate) unresolved_multicursor_positions: HashMap<MultiCursorKey, MultiCursorPosition>,
    pub(crate) multicursor_namespace_ids: HashMap<String, u64>,
    pub(crate) cursor_grid: u64,
    pub(crate) pending_cursor_grid: Option<u64>,
}

impl Default for ProtocolCursorState {
    fn default() -> Self {
        Self {
            cursor_style_enabled: false,
            cursor_modes: Vec::new(),
            cursor_mode_index: 0,
            multicursor_positions: HashMap::new(),
            pending_multicursor_positions: None,
            unresolved_multicursor_positions: HashMap::new(),
            multicursor_namespace_ids: HashMap::new(),
            cursor_grid: 1,
            pending_cursor_grid: None,
        }
    }
}

/// State used while the first redraw is being sized and presented.
#[derive(Debug, Clone)]
pub(crate) struct StartupState {
    pub(crate) nvim_grid_ready: bool,
    pub(crate) resize_target: Option<(u32, u32)>,
    pub(crate) flush_seen: bool,
    pub(crate) grid_content_seen: bool,
    pub(crate) redraw_pending: bool,
    pub(crate) maximize_pending: bool,
}

impl Default for StartupState {
    fn default() -> Self {
        Self {
            nvim_grid_ready: true,
            resize_target: None,
            flush_seen: false,
            grid_content_seen: false,
            redraw_pending: false,
            maximize_pending: false,
        }
    }
}

pub(crate) struct PendingRedrawState {
    pub(crate) ui_options: HashMap<String, String>,
    pub(crate) display_options: grid::DisplayOptions,
    pub(crate) guifont: Option<String>,
    pub(crate) guifontwide: Option<String>,
    pub(crate) window_title: String,
    pub(crate) window_icon: String,
    pub(crate) mouse_option: String,
    pub(crate) mouse_enabled: bool,
    pub(crate) nvim_mode: String,
    pub(crate) linespace: f32,
    pub(crate) cursor_style_enabled: bool,
    pub(crate) cursor_modes: Vec<grid::CursorModeInfo>,
    pub(crate) cursor_mode_index: usize,
    pub(crate) input_router: InputRouter,
    pub(crate) editor_mode: String,
}

pub(crate) struct RedrawCommit {
    pub(crate) input_router: InputRouter,
    pub(crate) window_title: String,
    pub(crate) window_icon: String,
    pub(crate) mouse_option: String,
    pub(crate) mouse_enabled: bool,
    pub(crate) nvim_mode: String,
    pub(crate) font_changed: bool,
    pub(crate) font_wide_changed: bool,
    pub(crate) linespace_changed: bool,
    pub(crate) display_options_changed: bool,
    pub(crate) geometry_changed: bool,
    pub(crate) cursor_timing_reset: bool,
    pub(crate) kitty_events: Vec<KittyEvent>,
    pub(crate) grid_commits: Vec<GridCommit>,
    pub(crate) destroyed_grids: Vec<u64>,
}

pub(crate) struct GridCommit {
    pub(crate) grid: u64,
    pub(crate) previous_grid: Rc<grid::GridModel>,
    pub(crate) next_grid: Rc<grid::GridModel>,
    pub(crate) previous_placement: Option<GridPlacement>,
    pub(crate) next_placement: Option<GridPlacement>,
}

pub(crate) enum ProtocolOutcome {
    ApiReady(NvimVersion),
    UiAttached {
        width: u32,
        height: u32,
        redraw: RedrawCommit,
    },
    PendingChanged,
    Flushed(RedrawCommit),
    Error(String),
    Disconnected(DisconnectReason),
}

pub(crate) struct ProtocolState {
    pub(crate) state: EditorState,
    pub(crate) presentation: ProtocolScreenState,
    pub(crate) cursor: ProtocolCursorState,
    pub(crate) startup: StartupState,
    pub(crate) guifont: Option<String>,
    pub(crate) guifontwide: Option<String>,
    pub(crate) ui_options: HashMap<String, String>,
    pub(crate) display_options: grid::DisplayOptions,
    pub(crate) linespace: f32,
    pub(crate) window_title: String,
    pub(crate) window_icon: String,
    pub(crate) input_router: InputRouter,
    pub(crate) mouse_option: String,
    pub(crate) mouse_enabled: bool,
    pub(crate) nvim_mode: String,
    pub(crate) theme: NvimTheme,
    pub(crate) pending_theme: Option<NvimTheme>,
    pending_redraw: Option<PendingRedrawState>,
    pending_geometry_changed: bool,
    pending_cursor_timing_reset: bool,
}

impl Default for ProtocolState {
    fn default() -> Self {
        Self {
            state: EditorState::default(),
            presentation: ProtocolScreenState::default(),
            cursor: ProtocolCursorState::default(),
            startup: StartupState::default(),
            guifont: None,
            guifontwide: None,
            ui_options: HashMap::new(),
            display_options: grid::DisplayOptions::default(),
            linespace: 0.0,
            window_title: "gpvim".to_owned(),
            window_icon: "nvim-gpui".to_owned(),
            input_router: InputRouter::new(InputRouterConfig::default()),
            mouse_option: "nvi".to_owned(),
            mouse_enabled: true,
            nvim_mode: "n".to_owned(),
            theme: NvimTheme::default(),
            pending_theme: None,
            pending_redraw: None,
            pending_geometry_changed: false,
            pending_cursor_timing_reset: false,
        }
    }
}

impl ProtocolState {
    #[cfg(test)]
    pub(crate) fn with_startup(startup: StartupState) -> Self {
        Self {
            startup,
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn has_pending_redraw(&self) -> bool {
        self.pending_redraw.is_some()
    }

    pub(crate) fn set_rime_enabled_for(&mut self, context: InputContext, enabled: bool) {
        self.input_router.set_rime_enabled_for(context, enabled);
        if let Some(pending) = self.pending_redraw.as_mut() {
            pending.input_router.set_rime_enabled_for(context, enabled);
        }
    }

    pub(crate) fn disable_rime(&mut self) {
        self.input_router.disable_rime();
        if let Some(pending) = self.pending_redraw.as_mut() {
            pending.input_router.disable_rime();
        }
    }

    pub(crate) fn apply(&mut self, event: NvimEvent) -> ProtocolOutcome {
        if !matches!(
            &event,
            NvimEvent::ApiReady { .. }
                | NvimEvent::UiAttached { .. }
                | NvimEvent::Flush
                | NvimEvent::Error(_)
                | NvimEvent::Disconnected { .. }
                | NvimEvent::UiSend { .. }
        ) {
            self.begin_pending_redraw();
        }

        match event {
            NvimEvent::ApiReady {
                version,
                capabilities: _,
            } => ProtocolOutcome::ApiReady(version),
            NvimEvent::UiAttached { width, height } => {
                let redraw = self.commit_pending_redraw(Vec::new());
                ProtocolOutcome::UiAttached {
                    width,
                    height,
                    redraw,
                }
            }
            NvimEvent::GridResized {
                grid,
                width,
                height,
            } => {
                if grid == 1 {
                    if !self.startup.nvim_grid_ready {
                        self.startup.grid_content_seen = false;
                    }
                    self.presentation.pending_grid =
                        Some(self.new_styled_grid(width as usize, height as usize));
                    self.presentation.pending_grid_size = Some(Some((width, height)));
                } else {
                    self.pending_grid_mut_for(grid)
                        .resize(width as usize, height as usize);
                }
                self.refresh_float_position(grid);
                self.presentation.pending_destroyed_grids.remove(&grid);
                self.pending_geometry_changed = true;
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::GridLine {
                grid,
                row,
                col_start,
                cells,
                wraps_to_next,
            } => {
                if grid == 1 && !self.startup.nvim_grid_ready {
                    self.startup.grid_content_seen = true;
                }
                self.pending_grid_mut_for(grid).apply_grid_line(
                    row as usize,
                    col_start as usize,
                    &cells,
                    wraps_to_next,
                );
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::GridClear { grid } => {
                if grid == 1 && !self.startup.nvim_grid_ready {
                    self.startup.grid_content_seen = true;
                }
                self.pending_grid_mut_for(grid).clear();
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::GridDestroy { grid } => {
                self.clear_pending_multicursors_for_grid(grid);
                if grid == 1 {
                    self.pending_grid_mut().destroy();
                    self.presentation.pending_grid_size = Some(None);
                } else {
                    self.presentation.pending_other_grids.remove(&grid);
                    self.presentation.pending_destroyed_grids.insert(grid);
                }
                self.pending_geometry_changed = true;
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::GridCursorGoto { grid, row, col } => {
                self.cursor.pending_cursor_grid = Some(grid);
                self.pending_grid_mut_for(grid)
                    .set_cursor(row as usize, col as usize);
                self.pending_geometry_changed = true;
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::DefaultColorsSet {
                foreground,
                background,
                special,
            } => {
                let theme = self.pending_theme_mut();
                theme.default_foreground = foreground;
                theme.default_background = background;
                self.set_default_colors_on_all_grids(foreground, background, special);
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::HlAttrDefine { id, attrs } => {
                let theme = self.pending_theme_mut();
                match attrs.ui_name.as_deref() {
                    Some("Normal") => {
                        theme.normal_foreground = attrs.foreground;
                        theme.normal_background = attrs.background;
                    }
                    Some("NormalFloat") => {
                        theme.normal_float_background = attrs.background;
                    }
                    _ => {}
                }
                self.set_highlight_on_all_grids(id, attrs);
                ProtocolOutcome::PendingChanged
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
                self.pending_geometry_changed = true;
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::WinPos {
                grid,
                win: _,
                row,
                col,
                width,
                height,
            } => {
                let mut placement = self.grid_placement(grid);
                placement.row = row as i64;
                placement.col = col as i64;
                placement.width = width;
                placement.height = height;
                placement.z_index = 0;
                placement.compindex = -1;
                placement.float_position = None;
                placement.kind = GridLayerKind::Window;
                placement.mouse_enabled = true;
                placement.visible = true;
                self.set_grid_placement(grid, placement);
                self.refresh_anchored_float_positions(grid);
                self.pending_geometry_changed = true;
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::WinFloatPos {
                grid,
                win: _,
                position,
                mouse_enabled,
                zindex,
                compindex,
            } => {
                let (row, col) = self.resolve_float_position(grid, position);
                let mut placement = self.grid_placement(grid);
                placement.row = row;
                placement.col = col;
                placement.z_index = zindex;
                placement.compindex = compindex;
                placement.float_position = Some(position);
                placement.kind = GridLayerKind::Float;
                placement.mouse_enabled = mouse_enabled;
                placement.visible = true;
                self.set_grid_placement(grid, placement);
                self.refresh_anchored_float_positions(grid);
                self.pending_geometry_changed = true;
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::WinViewport {
                grid,
                win: _,
                topline,
                botline,
                curline,
                curcol,
                line_count,
                scroll_delta,
            } => {
                let mut placement = self.grid_placement(grid);
                placement.viewport = Some(GridViewport {
                    topline,
                    botline,
                    curline,
                    curcol,
                    line_count,
                    scroll_delta,
                });
                self.set_grid_placement(grid, placement);
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::WinViewportMargins {
                grid,
                win: _,
                top,
                bottom,
                left,
                right,
            } => {
                let mut placement = self.grid_placement(grid);
                placement.viewport_margins = Some(GridViewportMargins {
                    top,
                    bottom,
                    left,
                    right,
                });
                self.set_grid_placement(grid, placement);
                self.pending_geometry_changed = true;
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::WinExtmark {
                grid,
                win: _,
                ns_id,
                mark_id,
                row,
                col,
            } => {
                let key = MultiCursorKey {
                    grid,
                    ns_id,
                    mark_id,
                };
                self.cursor.unresolved_multicursor_positions.remove(&key);
                if row < 0 || col < 0 {
                    self.pending_multicursor_positions_mut().remove(&key);
                    return ProtocolOutcome::PendingChanged;
                }
                let position = MultiCursorPosition {
                    key,
                    row: row as usize,
                    col: col as usize,
                };
                if self
                    .cursor
                    .multicursor_namespace_ids
                    .values()
                    .any(|known_ns_id| *known_ns_id == ns_id)
                {
                    self.pending_multicursor_positions_mut()
                        .insert(key, position);
                } else {
                    self.cursor
                        .unresolved_multicursor_positions
                        .insert(key, position);
                }
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::MsgSetPos {
                grid,
                row,
                scrolled,
                sep_char,
                zindex,
                compindex,
            } => {
                let grid_width = self.pending_grid_mut_for(grid).width() as u64;
                let mut placement = self.grid_placement(grid);
                placement.row = row as i64;
                placement.col = 0;
                placement.width = grid_width;
                placement.z_index = zindex;
                placement.compindex = compindex;
                placement.kind = GridLayerKind::Message;
                placement.visible = true;
                placement.message_scrolled = scrolled;
                placement.message_separator = sep_char.chars().next();
                self.set_grid_placement(grid, placement);
                self.pending_geometry_changed = true;
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::WinExternalPos { grid, win: _ } => {
                let mut placement = self.grid_placement(grid);
                placement.kind = GridLayerKind::External;
                placement.visible = false;
                self.set_grid_placement(grid, placement);
                self.pending_geometry_changed = true;
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::WinHide { grid } => {
                self.clear_pending_multicursors_for_grid(grid);
                let mut placement = self.grid_placement(grid);
                placement.visible = false;
                self.set_grid_placement(grid, placement);
                self.pending_geometry_changed = true;
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::WinClose { grid } => {
                self.clear_pending_multicursors_for_grid(grid);
                if grid == 1 {
                    self.pending_grid_mut().destroy();
                    self.presentation.pending_grid_size = Some(None);
                } else {
                    self.presentation.pending_other_grids.remove(&grid);
                    self.presentation.pending_destroyed_grids.insert(grid);
                }
                self.pending_geometry_changed = true;
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::OptionSet { name, value } => {
                let pending = self.pending_redraw_mut();
                pending.ui_options.insert(name.clone(), value.clone());
                pending.display_options.apply_option(&name, &value);
                match name.as_str() {
                    "mouse" => {
                        pending.mouse_option = value;
                        pending.mouse_enabled = Self::mouse_option_allows_mode(
                            &pending.mouse_option,
                            &pending.nvim_mode,
                        );
                    }
                    "guifont" => pending.guifont = Some(value),
                    "guifontwide" => pending.guifontwide = Some(value),
                    "linespace" => {
                        pending.linespace = parse_non_negative_float(&value).unwrap_or(0.0)
                    }
                    _ => {}
                }
                if matches!(name.as_str(), "guifont" | "guifontwide" | "linespace") {
                    self.pending_geometry_changed = true;
                }
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::SetTitle { title } => {
                if !title.is_empty() {
                    self.pending_redraw_mut().window_title = title;
                }
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::SetIcon { icon } => {
                self.pending_redraw_mut().window_icon = icon;
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::ModeInfoSet {
                cursor_style_enabled,
                modes,
            } => {
                let pending = self.pending_redraw_mut();
                pending.cursor_style_enabled = cursor_style_enabled;
                pending.cursor_modes = modes;
                self.pending_cursor_timing_reset = true;
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::ModeChanged { mode, mode_idx } => {
                let pending = self.pending_redraw_mut();
                pending.input_router.set_nvim_mode(&mode);
                pending.editor_mode = mode.to_ascii_uppercase();
                pending.nvim_mode = mode;
                pending.mouse_enabled =
                    Self::mouse_option_allows_mode(&pending.mouse_option, &pending.nvim_mode);
                pending.cursor_mode_index = mode_idx as usize;
                self.pending_geometry_changed = true;
                self.pending_cursor_timing_reset = true;
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::UiSend { data } => {
                let grid = GridId(
                    self.cursor
                        .pending_cursor_grid
                        .unwrap_or(self.cursor.cursor_grid),
                );
                self.presentation.pending_ui_data.push((data, grid));
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::MouseEnabled(enabled) => {
                self.pending_redraw_mut().mouse_enabled = enabled;
                ProtocolOutcome::PendingChanged
            }
            NvimEvent::Flush => {
                let (grid_commits, destroyed_grids) = self.commit_pending_grid();
                self.commit_pending_theme();
                let kitty_events = self.commit_pending_ui_data();
                let mut redraw = self.commit_pending_redraw(kitty_events);
                redraw.geometry_changed |= self.pending_geometry_changed;
                redraw.cursor_timing_reset |= self.pending_cursor_timing_reset;
                redraw.grid_commits = grid_commits;
                redraw.destroyed_grids = destroyed_grids;
                self.pending_geometry_changed = false;
                self.pending_cursor_timing_reset = false;
                self.startup.flush_seen = true;
                self.update_startup_grid_ready();
                ProtocolOutcome::Flushed(redraw)
            }
            NvimEvent::Error(error) => ProtocolOutcome::Error(error),
            NvimEvent::Disconnected { reason } => ProtocolOutcome::Disconnected(reason),
        }
    }

    fn begin_pending_redraw(&mut self) {
        if self.pending_redraw.is_some() {
            return;
        }
        if self.cursor.pending_multicursor_positions.is_none() {
            self.cursor.pending_multicursor_positions =
                Some(self.cursor.multicursor_positions.clone());
        }
        self.pending_redraw = Some(PendingRedrawState {
            ui_options: self.ui_options.clone(),
            display_options: self.display_options,
            guifont: self.guifont.clone(),
            guifontwide: self.guifontwide.clone(),
            window_title: self.window_title.clone(),
            window_icon: self.window_icon.clone(),
            mouse_option: self.mouse_option.clone(),
            mouse_enabled: self.mouse_enabled,
            nvim_mode: self.nvim_mode.clone(),
            linespace: self.linespace,
            cursor_style_enabled: self.cursor.cursor_style_enabled,
            cursor_modes: self.cursor.cursor_modes.clone(),
            cursor_mode_index: self.cursor.cursor_mode_index,
            input_router: self.input_router,
            editor_mode: self.state.mode.clone(),
        });
    }

    fn pending_redraw_mut(&mut self) -> &mut PendingRedrawState {
        self.begin_pending_redraw();
        self.pending_redraw
            .as_mut()
            .expect("pending redraw was initialized")
    }

    fn commit_pending_redraw(&mut self, kitty_events: Vec<KittyEvent>) -> RedrawCommit {
        let Some(pending) = self.pending_redraw.take() else {
            return self.current_redraw_commit(kitty_events);
        };

        let result = RedrawCommit {
            input_router: pending.input_router,
            window_title: pending.window_title.clone(),
            window_icon: pending.window_icon.clone(),
            mouse_option: pending.mouse_option.clone(),
            mouse_enabled: pending.mouse_enabled,
            nvim_mode: pending.nvim_mode.clone(),
            font_changed: self.guifont != pending.guifont,
            font_wide_changed: self.guifontwide != pending.guifontwide,
            linespace_changed: (self.linespace - pending.linespace).abs() > f32::EPSILON,
            display_options_changed: self.display_options != pending.display_options,
            geometry_changed: false,
            cursor_timing_reset: false,
            kitty_events,
            grid_commits: Vec::new(),
            destroyed_grids: Vec::new(),
        };

        self.ui_options = pending.ui_options;
        self.display_options = pending.display_options;
        self.guifont = pending.guifont;
        self.guifontwide = pending.guifontwide;
        self.window_title = pending.window_title;
        self.window_icon = pending.window_icon;
        self.mouse_option = pending.mouse_option;
        self.mouse_enabled = pending.mouse_enabled;
        self.nvim_mode = pending.nvim_mode;
        self.linespace = pending.linespace;
        self.cursor.cursor_style_enabled = pending.cursor_style_enabled;
        self.cursor.cursor_modes = pending.cursor_modes;
        self.cursor.cursor_mode_index = pending.cursor_mode_index;
        self.input_router = result.input_router;
        self.state.mode = pending.editor_mode;
        result
    }

    fn current_redraw_commit(&self, kitty_events: Vec<KittyEvent>) -> RedrawCommit {
        RedrawCommit {
            input_router: self.input_router,
            window_title: self.window_title.clone(),
            window_icon: self.window_icon.clone(),
            mouse_option: self.mouse_option.clone(),
            mouse_enabled: self.mouse_enabled,
            nvim_mode: self.nvim_mode.clone(),
            font_changed: false,
            font_wide_changed: false,
            linespace_changed: false,
            display_options_changed: false,
            geometry_changed: false,
            cursor_timing_reset: false,
            kitty_events,
            grid_commits: Vec::new(),
            destroyed_grids: Vec::new(),
        }
    }

    fn commit_pending_ui_data(&mut self) -> Vec<KittyEvent> {
        std::mem::take(&mut self.presentation.pending_ui_data)
            .into_iter()
            .flat_map(|(data, grid)| self.presentation.image_store.consume_ui_data(&data, grid))
            .collect()
    }

    fn pending_multicursor_positions_mut(
        &mut self,
    ) -> &mut HashMap<MultiCursorKey, MultiCursorPosition> {
        self.cursor
            .pending_multicursor_positions
            .get_or_insert_with(|| self.cursor.multicursor_positions.clone())
    }

    fn pending_grid_mut(&mut self) -> &mut grid::GridModel {
        let pending = self
            .presentation
            .pending_grid
            .get_or_insert_with(|| Rc::clone(&self.presentation.grid));
        Rc::make_mut(pending)
    }

    fn new_styled_grid(&self, width: usize, height: usize) -> Rc<grid::GridModel> {
        let source = self
            .presentation
            .pending_grid
            .as_deref()
            .unwrap_or(self.presentation.grid.as_ref());
        let mut next_grid = grid::GridModel::new(width, height);
        for (id, attrs) in source.highlights() {
            next_grid.set_highlight(*id, attrs.clone());
        }
        let (foreground, background, special) = source.default_colors();
        next_grid.set_default_colors(foreground, background, special);
        Rc::new(next_grid)
    }

    fn pending_grid_mut_for(&mut self, grid: u64) -> &mut grid::GridModel {
        if grid == 1 {
            return self.pending_grid_mut();
        }
        if !self.presentation.pending_other_grids.contains_key(&grid) {
            let model = self
                .presentation
                .other_grids
                .get(&grid)
                .cloned()
                .unwrap_or_else(|| self.new_styled_grid(0, 0));
            self.presentation.pending_other_grids.insert(grid, model);
        }
        Rc::make_mut(
            self.presentation
                .pending_other_grids
                .get_mut(&grid)
                .expect("pending grid was inserted"),
        )
    }

    fn set_default_colors_on_all_grids(
        &mut self,
        foreground: Option<u32>,
        background: Option<u32>,
        special: Option<u32>,
    ) {
        self.pending_grid_mut()
            .set_default_colors(foreground, background, special);
        let grid_ids = self
            .presentation
            .other_grids
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for grid in grid_ids {
            self.pending_grid_mut_for(grid)
                .set_default_colors(foreground, background, special);
        }
        for (grid, model) in &mut self.presentation.pending_other_grids {
            if !self.presentation.other_grids.contains_key(grid) {
                Rc::make_mut(model).set_default_colors(foreground, background, special);
            }
        }
    }

    fn set_highlight_on_all_grids(&mut self, id: grid::HighlightId, attrs: grid::HighlightAttrs) {
        self.pending_grid_mut().set_highlight(id, attrs.clone());
        let grid_ids = self
            .presentation
            .other_grids
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for grid in grid_ids {
            self.pending_grid_mut_for(grid)
                .set_highlight(id, attrs.clone());
        }
        for (grid, model) in &mut self.presentation.pending_other_grids {
            if !self.presentation.other_grids.contains_key(grid) {
                Rc::make_mut(model).set_highlight(id, attrs.clone());
            }
        }
    }

    fn set_grid_placement(&mut self, grid: u64, placement: GridPlacement) {
        self.presentation
            .pending_grid_placements
            .insert(grid, placement);
        self.presentation.pending_destroyed_grids.remove(&grid);
    }

    pub(crate) fn grid_placement(&self, grid: u64) -> GridPlacement {
        self.presentation
            .pending_grid_placements
            .get(&grid)
            .copied()
            .or_else(|| self.presentation.grid_placements.get(&grid).copied())
            .unwrap_or_default()
    }

    fn resolve_float_position(&self, grid: u64, position: NvimFloatPosition) -> (i64, i64) {
        match position {
            NvimFloatPosition::Screen { row, col } => (row, col),
            NvimFloatPosition::Anchored {
                anchor,
                anchor_grid,
                row,
                col,
            } => {
                let anchor_placement = self.grid_placement(anchor_grid);
                let (width, height) = self.grid_dimensions(grid);
                let mut screen_row = anchor_placement.row.saturating_add(row);
                let mut screen_col = anchor_placement.col.saturating_add(col);
                if matches!(
                    anchor,
                    NvimFloatAnchor::SouthWest | NvimFloatAnchor::SouthEast
                ) {
                    screen_row = screen_row.saturating_sub(height.saturating_sub(1));
                }
                if matches!(
                    anchor,
                    NvimFloatAnchor::NorthEast | NvimFloatAnchor::SouthEast
                ) {
                    screen_col = screen_col.saturating_sub(width.saturating_sub(1));
                }
                (screen_row, screen_col)
            }
        }
    }

    fn refresh_float_position(&mut self, grid: u64) {
        let placement = self.grid_placement(grid);
        let Some(position) = placement.float_position else {
            return;
        };
        let (row, col) = self.resolve_float_position(grid, position);
        let mut placement = self.grid_placement(grid);
        placement.row = row;
        placement.col = col;
        self.set_grid_placement(grid, placement);
    }

    fn refresh_anchored_float_positions(&mut self, anchor_grid: u64) {
        let grids = self
            .presentation
            .grid_placements
            .keys()
            .chain(self.presentation.pending_grid_placements.keys())
            .copied()
            .collect::<HashSet<_>>();
        for grid in grids {
            let is_anchored_to_grid = matches!(
                self.grid_placement(grid).float_position,
                Some(NvimFloatPosition::Anchored {
                    anchor_grid: grid_id,
                    ..
                }) if grid_id == anchor_grid
            );
            if is_anchored_to_grid {
                self.refresh_float_position(grid);
            }
        }
    }

    fn grid_dimensions(&self, grid: u64) -> (i64, i64) {
        if grid == 1 {
            let model = self
                .presentation
                .pending_grid
                .as_ref()
                .unwrap_or(&self.presentation.grid);
            return (model.width() as i64, model.height() as i64);
        }
        self.presentation
            .pending_other_grids
            .get(&grid)
            .or_else(|| self.presentation.other_grids.get(&grid))
            .map(|model| (model.width() as i64, model.height() as i64))
            .unwrap_or((0, 0))
    }

    fn commit_pending_grid(&mut self) -> (Vec<GridCommit>, Vec<u64>) {
        let mut commits = Vec::new();
        if let Some(grid_size) = self.presentation.pending_grid_size.take() {
            self.presentation.grid_size = grid_size;
        }
        if let Some(grid) = self.presentation.pending_grid.take() {
            commits.push(GridCommit {
                grid: 1,
                previous_grid: Rc::clone(&self.presentation.grid),
                next_grid: Rc::clone(&grid),
                previous_placement: self.presentation.grid_placements.get(&1).copied(),
                next_placement: self.presentation.pending_grid_placements.get(&1).copied(),
            });
            if let Some(cursor) = grid.cursor() {
                self.state.line = cursor.row + 1;
                self.state.column = cursor.col + 1;
            }
            self.presentation.grid = grid;
        }
        for (grid, model) in std::mem::take(&mut self.presentation.pending_other_grids) {
            if !self.presentation.pending_destroyed_grids.contains(&grid) {
                if let Some(previous_grid) = self.presentation.other_grids.get(&grid).cloned() {
                    commits.push(GridCommit {
                        grid,
                        previous_grid,
                        next_grid: Rc::clone(&model),
                        previous_placement: self.presentation.grid_placements.get(&grid).copied(),
                        next_placement: self
                            .presentation
                            .pending_grid_placements
                            .get(&grid)
                            .copied(),
                    });
                }
                self.presentation.other_grids.insert(grid, model);
            }
        }
        for (grid, placement) in std::mem::take(&mut self.presentation.pending_grid_placements) {
            if !self.presentation.pending_destroyed_grids.contains(&grid) {
                self.presentation.grid_placements.insert(grid, placement);
            }
        }
        let destroyed_grids = std::mem::take(&mut self.presentation.pending_destroyed_grids);
        for grid in &destroyed_grids {
            self.presentation.other_grids.remove(grid);
            self.presentation.grid_placements.remove(grid);
            self.cursor
                .multicursor_positions
                .retain(|key, _| key.grid != *grid);
        }
        if let Some(positions) = self.cursor.pending_multicursor_positions.take() {
            self.cursor.multicursor_positions = positions;
        }
        if let Some(grid) = self.cursor.pending_cursor_grid.take() {
            self.cursor.cursor_grid = grid;
        }
        (commits, destroyed_grids.into_iter().collect())
    }

    fn commit_pending_theme(&mut self) {
        if let Some(theme) = self.pending_theme.take() {
            self.theme = theme;
        }
    }

    fn pending_theme_mut(&mut self) -> &mut NvimTheme {
        self.pending_theme.get_or_insert(self.theme)
    }

    fn clear_pending_multicursors_for_grid(&mut self, grid: u64) {
        self.pending_multicursor_positions_mut()
            .retain(|key, _| key.grid != grid);
        self.cursor
            .unresolved_multicursor_positions
            .retain(|key, _| key.grid != grid);
    }

    fn update_startup_grid_ready(&mut self) {
        if self.startup.nvim_grid_ready
            || !self.startup.flush_seen
            || !self.startup.grid_content_seen
        {
            return;
        }
        let Some(target) = self.startup.resize_target else {
            return;
        };
        let committed_size = (
            self.presentation.grid.width() as u32,
            self.presentation.grid.height() as u32,
        );
        if committed_size == target {
            self.startup.nvim_grid_ready = true;
        }
    }

    fn mouse_option_allows_mode(mouse_option: &str, mode: &str) -> bool {
        if mouse_option.is_empty() {
            return false;
        }
        match mode.chars().next() {
            Some('n' | 'v' | 'V' | '\u{16}') => mouse_option.contains('n'),
            Some('i' | 'R' | 'r' | 'c' | 't') => mouse_option.contains('i'),
            _ => true,
        }
    }

    pub(crate) fn discard_pending_redraw(&mut self) {
        self.presentation.pending_grid = None;
        self.presentation.pending_grid_size = None;
        self.presentation.pending_other_grids.clear();
        self.cursor.pending_multicursor_positions = None;
        self.presentation.pending_grid_placements.clear();
        self.presentation.pending_destroyed_grids.clear();
        self.cursor.pending_cursor_grid = None;
        self.pending_theme = None;
        self.pending_redraw = None;
        self.presentation.pending_ui_data.clear();
        self.pending_geometry_changed = false;
        self.pending_cursor_timing_reset = false;
    }

    pub(crate) fn grid_is_visible(&self, grid: u64) -> bool {
        grid == 1
            || self
                .presentation
                .grid_placements
                .get(&grid)
                .is_some_and(|placement| placement.visible)
    }
}

fn parse_non_negative_float(value: &str) -> Option<f32> {
    let value = value.parse::<f32>().ok()?;
    value.is_finite().then_some(value.max(0.0))
}
