use super::*;

impl NvimGpui {
    pub(super) fn new(
        nvim: Result<NvimProcess, String>,
        cx: &mut Context<Self>,
        nerd_font_registered: bool,
        app_settings: settings::Settings,
        initial_theme: Option<NvimTheme>,
    ) -> Self {
        let nvim_available = nvim.is_ok();
        match &nvim {
            Ok(_) => log::info!(target: "nvim_gpui::state", "Neovim connection initialized"),
            Err(error) => log::error!(
                target: "nvim_gpui::state",
                "Neovim connection unavailable: {error}"
            ),
        }
        let mut this = Self {
            focus_handle: Some(cx.focus_handle()),
            grid: Rc::new(grid::GridModel::new(
                DEFAULT_GRID_WIDTH as usize,
                DEFAULT_GRID_HEIGHT as usize,
            )),
            grid_size: Some((DEFAULT_GRID_WIDTH, DEFAULT_GRID_HEIGHT)),
            rpc_status: match &nvim {
                Ok(_) => "rpc: connecting".to_owned(),
                Err(error) => format!("rpc: {error}"),
            },
            nvim: nvim.ok(),
            settings: app_settings,
            theme: initial_theme.unwrap_or_default(),
            ..Self::default()
        };

        this.bundled_nerd_font_registered = nerd_font_registered;
        this.nvim_grid_ready = !nvim_available;
        this.apply_runtime_settings();

        if this.nvim.as_ref().is_some_and(NvimProcess::is_remote) {
            this.start_remote_clipboard_bridge(cx);
        }
        if let Some(nvim) = this.nvim.as_ref() {
            this.start_event_task(nvim.events(), cx);
        }

        this
    }

    fn start_event_task(
        &mut self,
        events: async_channel::Receiver<NvimEvent>,
        cx: &mut Context<Self>,
    ) {
        self.event_task = Some(cx.spawn(async move |weak, cx| {
            while let Ok(event) = events.recv().await {
                // A single Neovim redraw notification can contain hundreds
                // of typed events. Drain the events that are already
                // available and invalidate GPUI once for the batch instead
                // of once per cell/window/style update.
                let mut batch = Vec::with_capacity(64);
                batch.push(event);
                while batch.len() < MAX_EVENTS_PER_UI_UPDATE {
                    match events.try_recv() {
                        Ok(event) => batch.push(event),
                        Err(async_channel::TryRecvError::Empty)
                        | Err(async_channel::TryRecvError::Closed) => break,
                    }
                }
                if batch.len() > 1 {
                    log::debug!(
                        target: "nvim_gpui::state",
                        "processing Neovim event batch: {} events",
                        batch.len()
                    );
                }
                let disconnect_reason = batch.iter().find_map(|event| match event {
                    NvimEvent::Disconnected { reason } => Some(reason.clone()),
                    _ => None,
                });
                if weak
                    .update(cx, |this, cx| {
                        for event in batch {
                            this.apply_nvim_event(event);
                        }
                        if let Some(reason) = disconnect_reason {
                            this.handle_disconnect(reason, cx);
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn start_remote_clipboard_bridge(&mut self, cx: &mut Context<Self>) {
        let Some(nvim) = self.nvim.as_ref() else {
            return;
        };
        if !nvim.is_remote() {
            return;
        }

        let (requests, request_queue) = crate::clipboard::channel();
        let get_requests = requests.clone();
        let set_requests = requests;
        self.clipboard_task = Some(cx.spawn(async move |_weak, cx| {
            while let Ok(request) = request_queue.recv().await {
                if cx
                    .update(|app| crate::clipboard::handle_on_ui_thread(app, request))
                    .is_err()
                {
                    break;
                }
            }
        }));

        let registration = {
            let nvim = self.nvim.as_ref().expect("remote Neovim should be present");
            nvim.register_request_handler(
                crate::clipboard::CLIPBOARD_GET_METHOD,
                crate::clipboard::get_request_handler(get_requests),
            )
            .and_then(|_| {
                nvim.register_request_handler(
                    crate::clipboard::CLIPBOARD_SET_METHOD,
                    crate::clipboard::set_request_handler(set_requests),
                )
            })
        };
        if let Err(error) = registration {
            log::error!(
                target: "nvim_gpui::clipboard",
                "failed to register remote clipboard handlers: {error}"
            );
            self.clipboard_task = None;
            return;
        }

        let response = self
            .nvim
            .as_ref()
            .expect("remote Neovim should be present")
            .protocol()
            .map(|protocol| protocol.channel_id);
        let channel_id = response.unwrap_or_default();
        let response = self
            .nvim
            .as_ref()
            .expect("remote Neovim should be present")
            .request(
                "nvim_exec_lua",
                rmpv::Value::Array(vec![
                    rmpv::Value::from(crate::clipboard::remote_provider_lua(channel_id)),
                    rmpv::Value::Array(Vec::new()),
                ]),
            );
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                log::error!(
                    target: "nvim_gpui::clipboard",
                    "failed to install remote clipboard provider: {error}"
                );
                self.clipboard_task = None;
                return;
            }
        };
        cx.spawn(async move |_weak, _cx| match response.recv().await {
            Ok(Ok(_)) => log::info!(
                target: "nvim_gpui::clipboard",
                "remote clipboard provider installed"
            ),
            Ok(Err(error)) => log::error!(
                target: "nvim_gpui::clipboard",
                "remote clipboard provider failed: {error}"
            ),
            Err(error) => log::error!(
                target: "nvim_gpui::clipboard",
                "remote clipboard provider response was lost: {error}"
            ),
        })
        .detach();
    }

    fn handle_disconnect(&mut self, reason: DisconnectReason, cx: &mut Context<Self>) {
        log::info!(target: "nvim_gpui::state", "Neovim disconnected: reason={reason:?}");
        match reason {
            DisconnectReason::Requested => {}
            DisconnectReason::CleanExit => cx.quit(),
            reason => self.schedule_reconnect(reason, cx),
        }
    }

    fn schedule_reconnect(&mut self, reason: DisconnectReason, cx: &mut Context<Self>) {
        if self.reconnect_task.is_some() {
            return;
        }

        let Some(nvim) = self.nvim.as_ref() else {
            return;
        };
        let connection = nvim.connection_spec();
        let size = self
            .last_resize
            .or(self.grid_size)
            .unwrap_or((DEFAULT_GRID_WIDTH, DEFAULT_GRID_HEIGHT));
        self.reconnect_attempt = self.reconnect_attempt.saturating_add(1);
        let attempt = self.reconnect_attempt;
        let backoff = 250_u64 * (1_u64 << attempt.saturating_sub(1).min(4));
        log::warn!(
            target: "nvim_gpui::state",
            "scheduling Neovim reconnect: attempt={}, backoff_ms={}, reason={reason:?}",
            attempt,
            backoff
        );
        self.rpc_status = format!("rpc: reconnecting (attempt {attempt})");

        let task = cx.background_spawn(async move {
            std::thread::sleep(Duration::from_millis(backoff));
            NvimProcess::connect_from_spec(&connection, size.0, size.1)
        });
        self.reconnect_task = Some(cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.reconnect_task = None;
                match result {
                    Ok(nvim) => {
                        log::info!(target: "nvim_gpui::state", "Neovim reconnected");
                        this.reconnect_attempt = 0;
                        this.install_reconnected_nvim(nvim, cx);
                    }
                    Err(error) => {
                        log::warn!(
                            target: "nvim_gpui::state",
                            "Neovim reconnect failed: {error}"
                        );
                        this.rpc_status = format!("rpc reconnect failed: {error}");
                        this.schedule_reconnect(reason.clone(), cx);
                    }
                }
                cx.notify();
            });
        }));
    }

    fn install_reconnected_nvim(&mut self, nvim: NvimProcess, cx: &mut Context<Self>) {
        let events = nvim.events();
        let initial_theme = nvim.startup_theme().unwrap_or_default();
        let protocol = nvim.protocol().cloned();

        self.reset_nvim_session(initial_theme);
        self.nvim = Some(nvim);
        self.api_level = protocol.as_ref().map(|protocol| protocol.version.api_level);
        self.nvim_version = protocol.map(|protocol| protocol.version);
        self.rpc_status = "rpc: reconnected".to_owned();
        log::info!(target: "nvim_gpui::state", "installed reconnected Neovim session");
        self.start_remote_clipboard_bridge(cx);
        self.start_event_task(events, cx);
    }

    fn reset_nvim_session(&mut self, initial_theme: NvimTheme) {
        self.state = EditorState::default();
        self.grid = Rc::new(grid::GridModel::new(
            DEFAULT_GRID_WIDTH as usize,
            DEFAULT_GRID_HEIGHT as usize,
        ));
        self.pending_grid = None;
        self.grid_size = Some((DEFAULT_GRID_WIDTH, DEFAULT_GRID_HEIGHT));
        self.last_resize = None;
        self.nvim_grid_ready = false;
        self.startup_resize_target = None;
        self.startup_flush_seen = false;
        self.theme = initial_theme;
        self.pending_theme = None;
        self.ui_options.clear();
        self.other_grids.clear();
        self.pending_other_grids.clear();
        self.grid_placements.clear();
        self.pending_grid_placements.clear();
        self.pending_destroyed_grids.clear();
        self.viewport_animations.clear();
        self.cursor_grid = 1;
        self.pending_cursor_grid = None;
        self.ime_input_grid = None;
        self.ime_coordinates_dirty = true;
        self.image_store.clear();
        self.image_sources.clear();
        self.mouse_option = "nvi".to_owned();
        self.mouse_enabled = true;
        self.nvim_mode = "n".to_owned();
        self.input_router = InputRouter::default();
        self.system_ime.clear();
        self.scroll_remainder = point(0.0, 0.0);
        self.cursor_style_enabled = false;
        self.cursor_modes.clear();
        self.cursor_mode_index = 0;
        self.cursor_blink_started_at = Instant::now();
        self.clipboard_task = None;
        self.window_title = DEFAULT_WINDOW_TITLE.to_owned();
        self.window_icon = "nvim-gpui".to_owned();
    }

    pub(super) fn apply_runtime_settings(&mut self) {
        self.nerd_font_family = self
            .bundled_nerd_font_registered
            .then(|| self.settings.nerd_font.family().to_owned());
        self.shaping_cache.borrow_mut().clear();
        self.glyph_coverage_cache.borrow_mut().clear();
        for image in self
            .image_store
            .set_cache_size_mb(self.settings.image_cache_size_mb)
        {
            self.image_sources.remove(&image);
        }
    }

    pub(super) fn update_settings(&mut self, next: settings::Settings) {
        self.settings = next;
        self.apply_runtime_settings();
        self.settings_save_error = self.settings.save().err();
    }

    pub(super) fn current_grid_font(&mut self, window: &Window) -> GuiFontSpec {
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

    pub(super) fn current_grid_wide_font(&mut self, window: &Window) -> GuiFontSpec {
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

    pub(super) fn current_cursor_mode(&self) -> grid::CursorModeInfo {
        if !self.cursor_style_enabled {
            return grid::CursorModeInfo::default();
        }
        self.cursor_modes
            .get(self.cursor_mode_index)
            .copied()
            .unwrap_or_default()
    }

    pub(super) fn theme_background(&self) -> u32 {
        self.theme
            .normal_background
            .or(self.theme.default_background)
            .unwrap_or(BACKGROUND)
    }

    pub(super) fn theme_foreground(&self) -> u32 {
        self.theme
            .normal_foreground
            .or(self.theme.default_foreground)
            .unwrap_or(TEXT)
    }

    pub(super) fn pending_theme_mut(&mut self) -> &mut NvimTheme {
        self.pending_theme.get_or_insert(self.theme)
    }

    pub(super) fn commit_pending_theme(&mut self) {
        if let Some(theme) = self.pending_theme.take() {
            self.theme = theme;
        }
    }

    pub(super) fn update_startup_grid_ready(&mut self) {
        if self.nvim_grid_ready || !self.startup_flush_seen {
            return;
        }

        let Some(target) = self.startup_resize_target else {
            return;
        };
        let committed_size = (self.grid.width() as u32, self.grid.height() as u32);
        if committed_size == target {
            self.nvim_grid_ready = true;
        }
    }

    pub(super) fn sync_nvim_size(&mut self, window: &mut Window) {
        let gui_font = self.current_grid_font(window);
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

        if !self.nvim_grid_ready {
            self.startup_resize_target = Some(size);
            self.update_startup_grid_ready();
            if self.nvim_grid_ready {
                return;
            }
        }

        let Some(nvim) = self.nvim.as_ref() else {
            return;
        };

        if self.last_resize == Some(size) {
            return;
        }

        match nvim.send_resize(width, height) {
            Ok(()) => {
                log::debug!(
                    target: "nvim_gpui::state",
                    "sent Neovim resize: width={}, height={}",
                    width,
                    height
                );
                self.last_resize = Some(size);
            }
            Err(error) => {
                log::error!(target: "nvim_gpui::state", "Neovim resize failed: {error}");
                self.rpc_status = format!("rpc resize error: {error}");
            }
        }
    }

    pub(super) fn apply_nvim_event(&mut self, event: NvimEvent) {
        match event {
            NvimEvent::ApiReady {
                version,
                capabilities: _,
            } => {
                log::info!(
                    target: "nvim_gpui::state",
                    "Neovim API ready: version={version}, api_level={}",
                    version.api_level
                );
                self.api_level = Some(version.api_level);
                self.nvim_version = Some(version);
                self.rpc_status = format!("rpc: Neovim {version} / API {}", version.api_level);
            }
            NvimEvent::UiAttached { width, height } => {
                log::info!(
                    target: "nvim_gpui::state",
                    "Neovim UI attached: width={width}, height={height}"
                );
                self.rpc_status = format!("rpc: attached {width}×{height}");
            }
            NvimEvent::GridResized {
                grid,
                width,
                height,
            } => {
                log::debug!(
                    target: "nvim_gpui::state",
                    "grid resized: grid={grid}, width={width}, height={height}"
                );
                self.ime_coordinates_dirty = true;
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
                log::debug!(target: "nvim_gpui::state", "grid destroyed: grid={grid}");
                if grid == 1 {
                    self.pending_grid_mut().destroy();
                    self.grid_size = None;
                } else {
                    self.pending_other_grids.remove(&grid);
                    self.pending_destroyed_grids.insert(grid);
                }
            }
            NvimEvent::GridCursorGoto { grid, row, col } => {
                // `grid_cursor_goto` belongs to the current redraw batch. Do
                // not expose it until `flush`, otherwise a partial redraw can
                // paint the cursor over a different, already committed grid.
                self.ime_coordinates_dirty = true;
                self.pending_cursor_grid = Some(grid);
                self.pending_grid_mut_for(grid)
                    .set_cursor(row as usize, col as usize);
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
                self.ime_coordinates_dirty = true;
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
                self.ime_coordinates_dirty = true;
                let mut placement = self.grid_placement(grid);
                placement.row = row as i64;
                placement.col = col as i64;
                placement.width = width;
                placement.height = height;
                placement.z_index = 0;
                placement.compindex = -1;
                placement.mouse_enabled = true;
                placement.visible = true;
                self.set_grid_placement(grid, placement);
            }
            NvimEvent::WinFloatPos {
                grid,
                win: _,
                anchor: _,
                anchor_grid: _,
                anchor_row: _,
                anchor_col: _,
                mouse_enabled,
                zindex,
                compindex,
                screen_row,
                screen_col,
            } => {
                self.ime_coordinates_dirty = true;
                let mut placement = self.grid_placement(grid);
                placement.row = screen_row;
                placement.col = screen_col;
                placement.z_index = zindex;
                placement.compindex = compindex;
                placement.mouse_enabled = mouse_enabled;
                placement.visible = true;
                self.set_grid_placement(grid, placement);
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
            }
            NvimEvent::WinViewportMargins {
                grid,
                win: _,
                top,
                bottom,
                left,
                right,
            } => {
                self.ime_coordinates_dirty = true;
                let mut placement = self.grid_placement(grid);
                placement.viewport_margins = Some(GridViewportMargins {
                    top,
                    bottom,
                    left,
                    right,
                });
                self.set_grid_placement(grid, placement);
            }
            NvimEvent::MsgSetPos {
                grid,
                row,
                scrolled,
                sep_char,
                zindex,
                compindex,
            } => {
                self.ime_coordinates_dirty = true;
                // A message grid is not associated with a normal window, so
                // Neovim positions it with msg_set_pos instead of win_pos.
                // Keep it in the same placement table as window grids so its
                // grid_line updates become visible and participate in the
                // protocol compositing order.
                let grid_width = self.pending_grid_mut_for(grid).width() as u64;
                let mut placement = self.grid_placement(grid);
                placement.row = row as i64;
                placement.col = 0;
                placement.width = grid_width;
                placement.z_index = zindex;
                placement.compindex = compindex;
                placement.visible = true;
                placement.message_scrolled = scrolled;
                placement.message_separator = sep_char.chars().next();
                self.set_grid_placement(grid, placement);
            }
            NvimEvent::WinExternalPos { grid, win: _ } => {
                self.ime_coordinates_dirty = true;
                let mut placement = self.grid_placement(grid);
                placement.visible = false;
                self.set_grid_placement(grid, placement);
            }
            NvimEvent::WinHide { grid } => {
                self.ime_coordinates_dirty = true;
                let mut placement = self.grid_placement(grid);
                placement.visible = false;
                self.set_grid_placement(grid, placement);
            }
            NvimEvent::WinClose { grid } => {
                self.ime_coordinates_dirty = true;
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
                    "mouse" => {
                        self.mouse_option = value;
                        self.mouse_enabled = self.mouse_option_allows_current_mode();
                    }
                    "guifont" => {
                        self.ime_coordinates_dirty = true;
                        self.guifont = Some(value);
                        self.resolved_grid_font = None;
                        self.resolved_grid_wide_font = None;
                        self.last_resize = None;
                        self.shaping_cache.borrow_mut().clear();
                    }
                    "guifontwide" => {
                        self.ime_coordinates_dirty = true;
                        self.guifontwide = Some(value);
                        self.resolved_grid_wide_font = None;
                        self.last_resize = None;
                        self.shaping_cache.borrow_mut().clear();
                    }
                    "linespace" => {
                        self.ime_coordinates_dirty = true;
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
                self.ime_coordinates_dirty = true;
                self.input_router.set_nvim_mode(&mode);
                log::info!(
                    target: "nvim_gpui::state",
                    "Neovim mode changed: mode={mode}, mode_idx={mode_idx}, input_target={:?}",
                    self.input_router.target()
                );
                if self.input_router.target() != InputTarget::SystemIme {
                    self.system_ime.clear();
                }
                self.state.mode = mode.to_ascii_uppercase();
                self.nvim_mode = mode;
                self.mouse_enabled = self.mouse_option_allows_current_mode();
                self.cursor_mode_index = mode_idx as usize;
                self.cursor_blink_started_at = Instant::now();
            }
            NvimEvent::UiSend { data } => self.apply_ui_send(&data),
            NvimEvent::MouseEnabled(enabled) => {
                log::debug!(
                    target: "nvim_gpui::state",
                    "Neovim mouse input enabled: {enabled}"
                );
                self.mouse_enabled = enabled;
            }
            NvimEvent::Flush => {
                self.commit_pending_grid();
                self.commit_pending_theme();
                self.ime_coordinates_dirty = true;
                self.startup_flush_seen = true;
                self.update_startup_grid_ready();
            }
            NvimEvent::Error(error) => {
                log::error!(target: "nvim_gpui::state", "Neovim event error: {error}");
                self.rpc_status = format!("rpc error: {error}");
            }
            NvimEvent::Disconnected { reason } => {
                log::info!(
                    target: "nvim_gpui::state",
                    "Neovim disconnected: reason={reason:?}"
                );
                self.rpc_status = "rpc: disconnected".to_owned();
            }
        }
    }

    pub(super) fn apply_ui_send(&mut self, data: &str) {
        let events = self.image_store.consume_ui_data(
            data,
            GridId(self.pending_cursor_grid.unwrap_or(self.cursor_grid)),
        );
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
                            log::error!(
                                target: "nvim_gpui::nvim",
                                "failed to forward Neovim terminal response: {error}"
                            );
                            self.rpc_status = format!("rpc terminal response error: {error}");
                        }
                    }
                }
            }
        }
    }

    pub(super) fn pending_grid_mut(&mut self) -> &mut grid::GridModel {
        let pending = self
            .pending_grid
            .get_or_insert_with(|| Rc::clone(&self.grid));
        Rc::make_mut(pending)
    }

    pub(super) fn new_styled_grid(&self, width: usize, height: usize) -> Rc<grid::GridModel> {
        let source = self.pending_grid.as_deref().unwrap_or(self.grid.as_ref());
        let mut next_grid = grid::GridModel::new(width, height);
        for (id, attrs) in source.highlights() {
            next_grid.set_highlight(*id, attrs.clone());
        }
        let (foreground, background, special) = source.default_colors();
        next_grid.set_default_colors(foreground, background, special);
        Rc::new(next_grid)
    }

    pub(super) fn pending_grid_mut_for(&mut self, grid: u64) -> &mut grid::GridModel {
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

        let pending = self
            .pending_other_grids
            .get_mut(&grid)
            .expect("pending grid was inserted");
        Rc::make_mut(pending)
    }

    pub(super) fn set_default_colors_on_all_grids(
        &mut self,
        foreground: Option<u32>,
        background: Option<u32>,
        special: Option<u32>,
    ) {
        self.pending_grid_mut()
            .set_default_colors(foreground, background, special);
        for model in self.other_grids.values_mut() {
            Rc::make_mut(model).set_default_colors(foreground, background, special);
        }
        for model in self.pending_other_grids.values_mut() {
            Rc::make_mut(model).set_default_colors(foreground, background, special);
        }
    }

    pub(super) fn set_highlight_on_all_grids(
        &mut self,
        id: grid::HighlightId,
        attrs: grid::HighlightAttrs,
    ) {
        self.pending_grid_mut().set_highlight(id, attrs.clone());
        for model in self.other_grids.values_mut() {
            Rc::make_mut(model).set_highlight(id, attrs.clone());
        }
        for model in self.pending_other_grids.values_mut() {
            Rc::make_mut(model).set_highlight(id, attrs.clone());
        }
    }

    pub(super) fn set_grid_placement(&mut self, grid: u64, placement: GridPlacement) {
        self.pending_grid_placements.insert(grid, placement);
        self.pending_destroyed_grids.remove(&grid);
    }

    pub(super) fn grid_placement(&self, grid: u64) -> GridPlacement {
        self.pending_grid_placements
            .get(&grid)
            .copied()
            .or_else(|| self.grid_placements.get(&grid).copied())
            .unwrap_or_default()
    }

    pub(super) fn commit_pending_grid(&mut self) {
        if self.pending_cursor_grid.is_some() {
            self.update_cursor_animation();
        }

        if let Some(grid) = self.pending_grid.take() {
            self.start_viewport_animation(1, Rc::clone(&self.grid), Rc::clone(&grid));

            if let Some(cursor) = grid.cursor() {
                self.state.line = cursor.row + 1;
                self.state.column = cursor.col + 1;
            }
            self.grid = grid;
        }

        for (grid, model) in std::mem::take(&mut self.pending_other_grids) {
            if !self.pending_destroyed_grids.contains(&grid) {
                if let Some(previous) = self.other_grids.get(&grid).cloned() {
                    self.start_viewport_animation(grid, previous, Rc::clone(&model));
                }
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
            self.viewport_animations.remove(&grid);
        }

        if let Some(grid) = self.pending_cursor_grid.take() {
            self.cursor_grid = grid;
        }
    }

    fn update_cursor_animation(&mut self) {
        let previous = self.current_cursor_screen_position();
        let next = self.pending_cursor_screen_position();

        self.cursor_animation = match (previous, next) {
            (Some(from), Some(target)) if from != target => self
                .cursor_animation
                .map(|animation| animation.retarget(target))
                .or_else(|| Some(grid::CursorAnimation::new(from, target))),
            _ => None,
        };
    }

    pub(super) fn current_cursor_screen_position(&self) -> Option<grid::CursorVisualPosition> {
        let model = self.active_cursor_model()?;
        let placement = if self.cursor_grid == 1 {
            self.grid_placements
                .get(&self.cursor_grid)
                .copied()
                .unwrap_or_default()
        } else {
            self.grid_placements.get(&self.cursor_grid).copied()?
        };
        Self::cursor_screen_position(&model, placement)
    }

    /// Return the cursor in the local coordinate system of the grid that owns
    /// the currently registered system IME handler. The handler's
    /// `element_bounds` already includes the grid's screen placement, so the
    /// caller must add only this local position.
    pub(super) fn ime_cursor_position(&self) -> Option<grid::CursorVisualPosition> {
        let grid = self.ime_input_grid?;
        let model = if grid == 1 {
            self.grid.as_ref()
        } else {
            self.other_grids.get(&grid)?.as_ref()
        };
        model.cursor_visual_position()
    }

    fn pending_cursor_screen_position(&self) -> Option<grid::CursorVisualPosition> {
        let grid = self.pending_cursor_grid.unwrap_or(self.cursor_grid);
        let model = if grid == 1 {
            self.pending_grid.as_ref().unwrap_or(&self.grid)
        } else {
            self.pending_other_grids
                .get(&grid)
                .or_else(|| self.other_grids.get(&grid))?
        };
        let placement = self.grid_placement(grid);
        Self::cursor_screen_position(model, placement)
    }

    fn cursor_screen_position(
        model: &grid::GridModel,
        placement: GridPlacement,
    ) -> Option<grid::CursorVisualPosition> {
        let position = model.cursor_visual_position()?;
        let row = placement.row.checked_add(position.row as i64)?;
        let col = placement.col.checked_add(position.col as i64)?;
        (row >= 0 && col >= 0).then_some(grid::CursorVisualPosition {
            row: row as usize,
            col: col as usize,
            width: position.width,
        })
    }

    pub(super) fn active_cursor_model(&self) -> Option<Rc<grid::GridModel>> {
        if self.cursor_grid == 1 {
            Some(Rc::clone(&self.grid))
        } else {
            self.other_grids.get(&self.cursor_grid).cloned()
        }
    }

    fn start_viewport_animation(
        &mut self,
        grid: u64,
        previous_grid: Rc<grid::GridModel>,
        next_grid: Rc<grid::GridModel>,
    ) {
        let Some(viewport) = self
            .pending_grid_placements
            .get(&grid)
            .and_then(|placement| placement.viewport)
        else {
            return;
        };
        let Some(previous_placement) = self.grid_placements.get(&grid).copied() else {
            return;
        };
        let Some(next_placement) = self.pending_grid_placements.get(&grid).copied() else {
            return;
        };

        if previous_placement.viewport.is_none()
            || previous_placement.viewport_margins != next_placement.viewport_margins
            || previous_placement.row != next_placement.row
            || previous_placement.col != next_placement.col
            || previous_placement.width != next_placement.width
            || previous_placement.height != next_placement.height
            || previous_placement.z_index != next_placement.z_index
            || previous_placement.compindex != next_placement.compindex
            || previous_placement.visible != next_placement.visible
        {
            self.viewport_animations.remove(&grid);
            return;
        }

        if viewport.scroll_delta == 0
            || previous_grid.width() != next_grid.width()
            || previous_grid.height() != next_grid.height()
        {
            self.viewport_animations.remove(&grid);
            return;
        }

        self.viewport_animations.insert(
            grid,
            ViewportAnimation {
                previous_grid,
                scroll_delta: viewport.scroll_delta,
                started_at: Instant::now(),
            },
        );
    }

    pub(super) fn visible_grid_layers(&self) -> Vec<(u64, Rc<grid::GridModel>, GridPlacement)> {
        let mut layers = self
            .other_grids
            .iter()
            .filter_map(|(grid, model)| {
                let placement = self.grid_placements.get(grid).copied()?;
                placement
                    .visible
                    .then(|| (*grid, Rc::clone(model), placement))
            })
            .collect::<Vec<_>>();
        layers.sort_by(|left, right| {
            left.2
                .compindex
                .cmp(&right.2.compindex)
                .then_with(|| left.2.z_index.cmp(&right.2.z_index))
                .then_with(|| left.0.cmp(&right.0))
        });
        layers
    }

    pub(super) fn visible_image_layers(&self) -> Vec<ImageLayer> {
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

        // Most frames have no Kitty placeholder at all. Avoid walking every
        // visible grid in that common case. Build the lookup once as well so
        // placeholder cells do not rescan every image placement individually.
        if !self.image_store.has_virtual_placements() {
            layers.sort_by(|left, right| {
                left.z_index
                    .cmp(&right.z_index)
                    .then_with(|| left.grid.cmp(&right.grid))
                    .then_with(|| left.row.cmp(&right.row))
                    .then_with(|| left.column.cmp(&right.column))
            });
            return layers;
        }

        let virtual_image_sizes = self
            .image_store
            .virtual_placements()
            .filter_map(|placement| {
                self.image_store
                    .asset(placement.key.image)
                    .is_some()
                    .then_some((
                        placement.key.image,
                        (placement.columns, placement.rows, placement.z_index),
                    ))
            })
            .collect::<HashMap<_, _>>();
        let mut models = vec![(1, self.grid.as_ref())];
        models.extend(self.other_grids.iter().filter_map(|(grid, model)| {
            self.grid_is_visible(*grid)
                .then_some((*grid, model.as_ref()))
        }));

        let mut virtual_layer_keys = HashSet::new();

        for (grid, model) in &models {
            for (row, grid_row) in model.rows().iter().enumerate() {
                for (column, cell) in grid_row.cells().iter().enumerate() {
                    let Some((row_offset, column_offset)) =
                        image_store::placeholder_position(&cell.text)
                    else {
                        continue;
                    };
                    let Some(image) = model
                        .highlight(cell.highlight)
                        .and_then(|attrs| attrs.foreground)
                        .map(ImageId)
                    else {
                        continue;
                    };
                    let Some(&(columns, rows, z_index)) = virtual_image_sizes.get(&image) else {
                        continue;
                    };

                    // A placeholder is rendered through a Neovim virtual
                    // text/line. Another decoration (for example Markview's
                    // concealed title text) can cover the first placeholder
                    // cell while leaving the rest of the image intact. Do
                    // not require the (1, 1) marker: every marker encodes its
                    // own offset, so any visible cell can recover the image
                    // anchor.
                    let Some(row) = row.checked_sub(row_offset.saturating_sub(1) as usize) else {
                        continue;
                    };
                    let Some(column) = column.checked_sub(column_offset.saturating_sub(1) as usize)
                    else {
                        continue;
                    };
                    // Snacks intentionally hides an inline image while the
                    // cursor is on the source line (hybrid/conceal mode).
                    // With `virt_lines`, the Kitty placeholder begins on the
                    // line immediately below that source line.
                    // The placeholder cells can remain in the redraw model
                    // because another decoration may cover only their first
                    // cell, so use Neovim's cursor row as the visibility
                    // signal instead of treating a partial placeholder as a
                    // complete preview.
                    let source_row = row.saturating_sub(1);
                    if self.cursor_grid == *grid
                        && model
                            .cursor()
                            .is_some_and(|cursor| cursor.row == source_row)
                    {
                        continue;
                    }
                    if !virtual_layer_keys.insert((image, *grid, row, column)) {
                        continue;
                    }
                    layers.push(ImageLayer {
                        image,
                        grid: *grid,
                        row,
                        column,
                        columns,
                        rows,
                        z_index,
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

    pub(super) fn grid_is_visible(&self, grid: u64) -> bool {
        grid == 1
            || self
                .grid_placements
                .get(&grid)
                .is_some_and(|placement| placement.visible)
    }

    fn paste_from_system_clipboard(&mut self, cx: &mut Context<Self>) {
        let text = match crate::clipboard::paste_text(cx) {
            Ok(text) => text,
            Err(error) => {
                log::warn!(
                    target: "nvim_gpui::clipboard",
                    "could not read system clipboard for paste: {error}"
                );
                return;
            }
        };
        let Some(nvim) = self.nvim.as_ref() else {
            log::warn!(
                target: "nvim_gpui::clipboard",
                "ignoring paste because Neovim is unavailable"
            );
            return;
        };
        let response = match nvim.send_paste(text) {
            Ok(response) => response,
            Err(error) => {
                log::error!(
                    target: "nvim_gpui::clipboard",
                    "failed to queue system paste: {error}"
                );
                self.rpc_status = format!("rpc paste error: {error}");
                return;
            }
        };
        cx.spawn(async move |_weak, _cx| match response.recv().await {
            Ok(Ok(rmpv::Value::Boolean(false))) => log::warn!(
                target: "nvim_gpui::clipboard",
                "Neovim rejected system paste"
            ),
            Ok(Ok(_)) => {}
            Ok(Err(error)) => log::error!(
                target: "nvim_gpui::clipboard",
                "system paste failed in Neovim: {error}"
            ),
            Err(error) => log::error!(
                target: "nvim_gpui::clipboard",
                "system paste response was lost: {error}"
            ),
        })
        .detach();
    }

    pub(super) fn on_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings.paste_shortcut.matches(&event.keystroke) {
            self.paste_from_system_clipboard(cx);
            window.prevent_default();
            return;
        }

        let target = self.input_router.target();
        if !should_route_key_to_neovim(target, &event.keystroke) {
            return;
        }

        if let Some(nvim) = self.nvim.as_ref() {
            if let Err(error) = nvim.send_input(key_to_nvim_input(&event.keystroke)) {
                log::error!(target: "nvim_gpui::input", "key event failed: {error}");
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
