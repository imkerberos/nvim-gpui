use super::*;

impl NvimGpui {
    pub(crate) fn new(
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
                let mut reached_flush = matches!(&batch[0], NvimEvent::Flush);
                while !reached_flush && batch.len() < MAX_EVENTS_PER_UI_UPDATE {
                    match events.try_recv() {
                        Ok(event) => {
                            reached_flush = matches!(&event, NvimEvent::Flush);
                            batch.push(event);
                        }
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
                let should_notify = batch.iter().any(|event| {
                    matches!(
                        event,
                        NvimEvent::ApiReady { .. }
                            | NvimEvent::UiAttached { .. }
                            | NvimEvent::Flush
                            | NvimEvent::Error(_)
                            | NvimEvent::Disconnected { .. }
                    )
                });
                if weak
                    .update(cx, |this, cx| {
                        for event in batch {
                            this.apply_nvim_event(event);
                        }
                        if let Some(reason) = disconnect_reason {
                            this.handle_disconnect(reason, cx);
                        }
                        if should_notify {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
                // Let GPUI present the committed frame before processing the
                // next queued redraw. Without an executor yield, a rapid
                // sequence of page updates can keep this task runnable and
                // make all intermediate viewport animations invisible.
                if should_notify {
                    cx.background_executor()
                        .timer(Duration::from_millis(1))
                        .await;
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
        self.discard_pending_redraw();
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
        self.pending_redraw = None;
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_nvim_session_discards_old_committed_and_pending_state() {
        let mut app = NvimGpui::default();
        app.state.mode = "INSERT".to_owned();
        app.other_grids
            .insert(2, Rc::new(grid::GridModel::new(4, 2)));
        app.pending_grid = Some(Rc::new(grid::GridModel::new(3, 1)));
        app.pending_other_grids
            .insert(3, Rc::new(grid::GridModel::new(2, 1)));
        app.pending_grid_placements
            .insert(3, GridPlacement::default());
        app.pending_destroyed_grids.insert(4);
        app.pending_cursor_grid = Some(3);
        app.pending_theme = Some(NvimTheme {
            default_background: Some(0x101010),
            ..Default::default()
        });
        app.begin_pending_redraw();
        app.system_ime.replace_and_mark_text(None, "compose", None);

        let next_theme = NvimTheme {
            normal_background: Some(0x202020),
            ..Default::default()
        };
        app.reset_nvim_session(next_theme);

        assert!(app.other_grids.is_empty());
        assert!(app.pending_grid.is_none());
        assert!(app.pending_other_grids.is_empty());
        assert!(app.pending_grid_placements.is_empty());
        assert!(app.pending_destroyed_grids.is_empty());
        assert!(app.pending_cursor_grid.is_none());
        assert!(app.pending_theme.is_none());
        assert!(app.pending_redraw.is_none());
        assert_eq!(
            app.grid_size,
            Some((DEFAULT_GRID_WIDTH, DEFAULT_GRID_HEIGHT))
        );
        assert_eq!(app.theme.normal_background, Some(0x202020));
        assert_eq!(app.state.mode, "NORMAL");
        assert!(app.system_ime.is_empty());
    }
}
