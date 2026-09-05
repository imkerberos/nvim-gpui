use super::*;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

use nvim_gpui::rime::{RimeBackend, RimeConfig, RimeRuntimeResolver};

const MODIFIED_BUFFERS_LUA: &str = r#"
local modified = {}
for _, buffer in ipairs(vim.api.nvim_list_bufs()) do
  if vim.api.nvim_buf_is_valid(buffer)
      and vim.api.nvim_buf_get_option(buffer, "modified") then
    local name = vim.api.nvim_buf_get_name(buffer)
    table.insert(modified, name == "" and "[No Name]" or name)
  end
end
return modified
"#;
const UNNAMED_BUFFER_LABEL: &str = "[No Name]";

impl NvimGpui {
    pub(crate) fn test_rime_configuration_with_settings(
        app_settings: settings::Settings,
    ) -> Result<(), String> {
        let config = rime_config_from_settings(&app_settings, Some(false))?;
        let backend = RimeBackend::new(config)?;
        backend.context().map(|_| ()).map_err(|error| {
            format!("Rime session was created but context loading failed: {error}")
        })
    }

    pub(crate) fn new(
        nvim: Result<NvimProcess, String>,
        cx: &mut Context<Self>,
        nerd_font_registered: bool,
        app_settings: settings::Settings,
        initial_theme: Option<NvimTheme>,
        logger: Option<flexi_logger::LoggerHandle>,
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
            logger,
            theme: initial_theme.unwrap_or_default(),
            ..Self::default()
        };

        this.bundled_nerd_font_registered = nerd_font_registered;
        this.nvim_grid_ready = !nvim_available;
        this.apply_runtime_settings();
        this.rime_backend = initialize_rime_backend(&this.settings);
        if this.rime_backend.is_some() {
            // Keep the runtime toggle off for every fresh process. Selecting
            // Rime in Settings or using the activation shortcut enables it
            // explicitly; the first Insert mode therefore remains on the
            // system IME.
            log::info!(target: "nvim_gpui::rime", "Rime backend available");
        }

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
                // A redraw Flush is a Neovim transaction boundary, not a
                // required GPUI presentation boundary. Keep all events in
                // order, including events after Flush, and present the latest
                // complete state once for this batch. This is important for
                // key repeat: an invalid motion can still produce a Flush,
                // but there is no useful intermediate frame to present.
                let batch = collect_event_batch(event, &events);
                let has_queued_events = !events.is_empty();
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

                // Do not sleep for a millisecond after every Flush. If more
                // redraw data is already queued, yield once so GPUI can run
                // its frame and dispatch pending input, then continue with
                // the next losslessly coalesced batch. When the queue is
                // empty, the next recv().await already yields naturally.
                if has_queued_events {
                    futures_lite::future::yield_now().await;
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
        self.mouse_capture = None;
        self.nvim_mode = "n".to_owned();
        self.input_router = InputRouter::new(InputRouterConfig {
            rime_enabled: false,
        });
        self.reset_rime_composition();
        self.rime_menu_open = false;
        self.rime_menu_message = None;
        self.system_ime.clear();
        self.scroll_remainder = point(0.0, 0.0);
        self.cursor_style_enabled = false;
        self.cursor_modes.clear();
        self.cursor_mode_index = 0;
        self.cursor_blink_started_at = Instant::now();
        self.clipboard_task = None;
        self.window_title = DEFAULT_WINDOW_TITLE.to_owned();
        self.window_icon = "nvim-gpui".to_owned();
        self.display_options = grid::DisplayOptions::default();
    }

    pub(crate) fn redeploy_rime(&mut self, cx: &mut Context<Self>) {
        self.reset_rime_composition();
        let result = self
            .rime_backend
            .as_mut()
            .ok_or_else(|| "Rime backend is unavailable".to_owned())
            .and_then(RimeBackend::redeploy);

        self.rime_menu_message = Some(match result {
            Ok(()) => {
                log::info!(
                    target: "nvim_gpui::rime",
                    "Rime data redeployed from titlebar menu"
                );
                "Rime data redeployed successfully.".to_owned()
            }
            Err(error) => {
                log::error!(target: "nvim_gpui::rime", "Rime data redeploy failed: {error}");
                format!("Rime redeploy failed: {error}")
            }
        });
        self.rime_menu_open = true;
        cx.notify();
    }

    pub(crate) fn open_rime_user_data_directory(&mut self, cx: &mut Context<Self>) {
        let result = settings::rime_user_data_directory()
            .ok_or_else(|| "could not determine the Rime user data directory".to_owned())
            .and_then(|path| crate::logging::open_directory(&path));

        match result {
            Ok(()) => {
                log::info!(target: "nvim_gpui::rime", "opened Rime user data directory");
                self.rime_menu_open = false;
                self.rime_menu_message = None;
            }
            Err(error) => {
                log::error!(
                    target: "nvim_gpui::rime",
                    "could not open Rime user data directory: {error}"
                );
                self.rime_menu_message = Some(format!("Could not open user settings: {error}"));
                self.rime_menu_open = true;
            }
        }
        cx.notify();
    }

    /// Intercept the native window close request long enough to let Neovim
    /// report modified buffers. The confirmation UI is rendered by GPUI, so
    /// it does not require Neovim's external-window protocol.
    pub(crate) fn request_window_close(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.settings.quit_on_window_close {
            return true;
        }
        if matches!(self.quit_dialog, QuitDialogState::Quitting) {
            return true;
        }
        if !matches!(self.quit_dialog, QuitDialogState::Hidden) {
            return false;
        }
        let Some(nvim) = self.nvim.as_ref() else {
            return true;
        };
        let response = match nvim.request(
            "nvim_exec_lua",
            rmpv::Value::Array(vec![
                rmpv::Value::from(MODIFIED_BUFFERS_LUA),
                rmpv::Value::Array(Vec::new()),
            ]),
        ) {
            Ok(response) => response,
            Err(error) => {
                self.quit_dialog = QuitDialogState::Confirm {
                    modified_buffers: Vec::new(),
                    error: Some(format!("Could not check for unsaved changes: {error}")),
                };
                cx.notify();
                return false;
            }
        };

        self.quit_dialog = QuitDialogState::Checking;
        cx.spawn(async move |weak, cx| {
            let result = response.recv().await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok(Ok(value)) => match parse_modified_buffers(value) {
                        Ok(modified_buffers) if modified_buffers.is_empty() => {
                            if this.nvim.as_ref().is_some_and(NvimProcess::is_remote) {
                                this.quit_dialog = QuitDialogState::Quitting;
                                cx.quit();
                            } else {
                                this.begin_quit_command("qa", Vec::new(), cx);
                            }
                        }
                        Ok(modified_buffers) => {
                            this.quit_dialog = QuitDialogState::Confirm {
                                modified_buffers,
                                error: None,
                            };
                        }
                        Err(error) => {
                            this.quit_dialog = QuitDialogState::Confirm {
                                modified_buffers: Vec::new(),
                                error: Some(format!(
                                    "Could not check for unsaved changes: {error}"
                                )),
                            };
                        }
                    },
                    Ok(Err(error)) => {
                        this.quit_dialog = QuitDialogState::Confirm {
                            modified_buffers: Vec::new(),
                            error: Some(format!("Could not check for unsaved changes: {error}")),
                        };
                    }
                    Err(error) => {
                        this.quit_dialog = QuitDialogState::Confirm {
                            modified_buffers: Vec::new(),
                            error: Some(format!("Could not check for unsaved changes: {error}")),
                        };
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
        false
    }

    pub(crate) fn cancel_quit_dialog(&mut self, cx: &mut Context<Self>) {
        if matches!(self.quit_dialog, QuitDialogState::Confirm { .. }) {
            self.quit_dialog = QuitDialogState::Hidden;
            cx.notify();
        }
    }

    pub(crate) fn save_and_quit(&mut self, cx: &mut Context<Self>) {
        let modified_buffers = match &self.quit_dialog {
            QuitDialogState::Confirm {
                modified_buffers, ..
            } => modified_buffers.clone(),
            _ => return,
        };
        if modified_buffers
            .iter()
            .any(|buffer| buffer == UNNAMED_BUFFER_LABEL)
        {
            self.quit_dialog = QuitDialogState::Confirm {
                modified_buffers,
                error: Some(format!(
                    "Cannot save {UNNAMED_BUFFER_LABEL}: it has no file name. Save it with :write first, or choose Discard & Quit."
                )),
            };
            cx.notify();
            return;
        }
        self.quit_dialog = QuitDialogState::Saving;
        let command = if self.nvim.as_ref().is_some_and(NvimProcess::is_remote) {
            "wall"
        } else {
            "wall | qa"
        };
        self.begin_quit_command(command, modified_buffers, cx);
        cx.notify();
    }

    pub(crate) fn discard_and_quit(&mut self, cx: &mut Context<Self>) {
        let modified_buffers = match &self.quit_dialog {
            QuitDialogState::Confirm {
                modified_buffers, ..
            } => modified_buffers.clone(),
            _ => return,
        };
        if self.nvim.as_ref().is_some_and(NvimProcess::is_remote) {
            self.quit_dialog = QuitDialogState::Quitting;
            cx.quit();
        } else {
            self.begin_quit_command("qa!", modified_buffers, cx);
        }
    }

    fn begin_quit_command(
        &mut self,
        command: &'static str,
        modified_buffers: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        self.quit_dialog = QuitDialogState::Quitting;
        let Some(nvim) = self.nvim.as_ref() else {
            cx.quit();
            return;
        };
        let response = match nvim.request(
            "nvim_command",
            rmpv::Value::Array(vec![rmpv::Value::from(command)]),
        ) {
            Ok(response) => response,
            Err(error) => {
                self.quit_dialog = QuitDialogState::Confirm {
                    modified_buffers,
                    error: Some(format!("Could not quit Neovim: {error}")),
                };
                cx.notify();
                return;
            }
        };
        let remote = nvim.is_remote();
        cx.spawn(async move |weak, cx| {
            let result = response.recv().await;
            let _ = weak.update(cx, |this, cx| match result {
                Ok(Ok(_)) if remote => {
                    this.quit_dialog = QuitDialogState::Quitting;
                    cx.quit();
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    this.quit_dialog = QuitDialogState::Confirm {
                        modified_buffers,
                        error: Some(format!("Could not quit Neovim: {error}")),
                    };
                    cx.notify();
                }
                Err(error) => {
                    this.quit_dialog = QuitDialogState::Confirm {
                        modified_buffers,
                        error: Some(format!("Could not quit Neovim: {error}")),
                    };
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

fn parse_modified_buffers(value: rmpv::Value) -> Result<Vec<String>, String> {
    let buffers = value
        .as_array()
        .ok_or_else(|| "Neovim returned an invalid modified-buffer list".to_owned())?;
    buffers
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value.as_str().map(str::to_owned).ok_or_else(|| {
                format!("Neovim returned an invalid modified-buffer name at index {index}")
            })
        })
        .collect()
}

fn initialize_rime_backend(app_settings: &settings::Settings) -> Option<RimeBackend> {
    let config = rime_config_from_settings(app_settings, None).ok()?;
    match RimeBackend::new(config) {
        Ok(backend) => Some(backend),
        Err(error) => {
            log::warn!(target: "nvim_gpui::rime", "Rime backend unavailable: {error}");
            None
        }
    }
}

fn rime_config_from_settings(
    app_settings: &settings::Settings,
    deploy_override: Option<bool>,
) -> Result<RimeConfig, String> {
    let bundled_runtime = cfg!(any(target_os = "macos", target_os = "windows"));
    let resolver = RimeRuntimeResolver::default();
    let shared_data = if bundled_runtime {
        resolver.resolve_shared_data(None)?
    } else if !app_settings.rime_data_dir.trim().is_empty() {
        PathBuf::from(app_settings.rime_data_dir.trim())
    } else if app_settings.rime_library_auto_detect {
        resolver.resolve_shared_data(None)?
    } else {
        env::var_os("NVIM_GPUI_RIME_SHARED_DIR")
            .map(PathBuf::from)
            .ok_or_else(|| "Rime shared data directory is not configured".to_owned())?
    };
    let user_data = settings::rime_user_data_directory()
        .ok_or_else(|| "Rime user data directory is not available".to_owned())?;
    // Keep librime's writable staging output under its default user-data
    // location without exposing the internal directory as a setting.
    let staging_data = user_data.join("build");
    let deploy = deploy_override.unwrap_or_else(|| {
        env::var("NVIM_GPUI_RIME_DEPLOY")
            .map(|value| matches!(value.as_str(), "1" | "true" | "yes"))
            .unwrap_or_else(|_| !directory_has_entries(&staging_data))
    });

    Ok(RimeConfig {
        library: if bundled_runtime {
            Some(resolver.resolve_library_directory(None)?)
        } else if app_settings.rime_library_auto_detect {
            env::var_os("NVIM_GPUI_RIME_LIBRARY")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
        } else {
            configured_path(&app_settings.rime_library_dir, "NVIM_GPUI_RIME_LIBRARY")
        },
        shared_data,
        user_data,
        staging_data: Some(staging_data),
        deploy,
    })
}

fn configured_path(setting: &str, environment: &str) -> Option<PathBuf> {
    (!setting.trim().is_empty())
        .then(|| PathBuf::from(setting.trim()))
        .or_else(|| env::var_os(environment).map(PathBuf::from))
}

fn directory_has_entries(path: &Path) -> bool {
    fs::read_dir(path)
        .ok()
        .and_then(|mut entries| entries.next())
        .is_some()
}

fn collect_event_batch(
    first: NvimEvent,
    events: &async_channel::Receiver<NvimEvent>,
) -> Vec<NvimEvent> {
    let mut batch = Vec::with_capacity(64);
    batch.push(first);

    while batch.len() < MAX_EVENTS_PER_UI_UPDATE {
        match events.try_recv() {
            Ok(event) => batch.push(event),
            Err(async_channel::TryRecvError::Empty) | Err(async_channel::TryRecvError::Closed) => {
                break
            }
        }
    }

    batch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redraw_batch_does_not_stop_at_first_flush() {
        let (sender, receiver) = async_channel::unbounded();
        sender.try_send(NvimEvent::Flush).unwrap();
        sender.try_send(NvimEvent::GridClear { grid: 1 }).unwrap();
        sender.try_send(NvimEvent::Flush).unwrap();

        let batch = collect_event_batch(NvimEvent::GridClear { grid: 1 }, &receiver);

        assert_eq!(batch.len(), 4);
        assert!(matches!(batch[1], NvimEvent::Flush));
        assert!(matches!(batch[2], NvimEvent::GridClear { grid: 1 }));
        assert!(matches!(batch[3], NvimEvent::Flush));
    }

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

    #[test]
    fn modified_buffer_list_accepts_named_and_unnamed_buffers() {
        let value = rmpv::Value::Array(vec![
            rmpv::Value::from("src/main.rs"),
            rmpv::Value::from("[No Name]"),
        ]);

        assert_eq!(
            parse_modified_buffers(value).unwrap(),
            vec!["src/main.rs".to_owned(), "[No Name]".to_owned()]
        );
    }

    #[test]
    fn modified_buffer_list_rejects_non_string_entries() {
        let value = rmpv::Value::Array(vec![rmpv::Value::Integer(1.into())]);

        let error = parse_modified_buffers(value).unwrap_err();

        assert!(error.contains("invalid modified-buffer name"));
    }

    #[test]
    fn modified_buffer_list_rejects_non_array_values() {
        let error = parse_modified_buffers(rmpv::Value::Boolean(false)).unwrap_err();

        assert!(error.contains("invalid modified-buffer list"));
    }
}
