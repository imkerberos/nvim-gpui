use super::{
    AppState, NvimGpui, QuitDialogState, Session, WindowRuntime, DEFAULT_GRID_HEIGHT,
    DEFAULT_GRID_WIDTH, DEFAULT_WINDOW_TITLE, MAX_EVENTS_PER_UI_UPDATE,
};
use crate::editor::ProtocolState;
use crate::input::{InputRouter, InputRouterConfig};
use crate::nvim::{DisconnectReason, NvimEvent, NvimProcess, NvimTheme, SessionId};
#[cfg(test)]
use crate::{editor::GridPlacement, grid};
use crate::{settings, update_check};
use gpui::{point, AppContext, Context};
#[cfg(test)]
use std::rc::Rc;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

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
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        nvim: Result<NvimProcess, String>,
        cx: &mut Context<Self>,
        nerd_font_registered: bool,
        app_settings: settings::Settings,
        initial_theme: Option<NvimTheme>,
        startup_maximized: bool,
        logger: Option<flexi_logger::LoggerHandle>,
        update_http_client: Arc<dyn gpui::http_client::HttpClient>,
    ) -> Self {
        let nvim_available = nvim.is_ok();
        match &nvim {
            Ok(_) => log::info!(target: "nvim_gpui::app", "Neovim connection initialized"),
            Err(error) => log::error!(
                target: "nvim_gpui::app",
                "Neovim connection unavailable: {error}"
            ),
        }
        let initial_theme = initial_theme.unwrap_or_default();
        let mut this = Self {
            window: WindowRuntime {
                focus_handle: Some(cx.focus_handle()),
                ..WindowRuntime::default()
            },
            app: AppState {
                session: Session {
                    rpc_status: match &nvim {
                        Ok(_) => "rpc: connecting".to_owned(),
                        Err(error) => format!("rpc: {error}"),
                    },
                    nvim: nvim.ok(),
                    ..Session::default()
                },
                settings: app_settings,
                update_http_client: Some(update_http_client),
                logger,
                ..AppState::default()
            },
            ..Self::default()
        };

        this.editor.bundled_nerd_font_registered = nerd_font_registered;
        this.editor.protocol.theme = initial_theme;
        this.editor.protocol.presentation.grid_size =
            Some((DEFAULT_GRID_WIDTH, DEFAULT_GRID_HEIGHT));
        this.editor.protocol.startup.nvim_grid_ready = !nvim_available;
        this.editor.protocol.startup.redraw_pending = nvim_available;
        this.editor.protocol.startup.maximize_pending = nvim_available && startup_maximized;
        this.editor.apply_runtime_settings(&this.app.settings);
        // librime deployment can rebuild a large data set. Start the backend
        // away from the UI thread so the first window remains responsive.
        this.start_rime_initialization(cx);

        if this
            .app
            .session
            .nvim
            .as_ref()
            .is_some_and(NvimProcess::is_remote)
        {
            this.start_remote_clipboard_bridge(cx);
        }
        if let Some(nvim) = this.app.session.nvim.as_ref() {
            this.app.session.session_id = Some(nvim.session_id());
            this.start_event_task(nvim.events(), nvim.session_id(), cx);
        }

        this
    }

    pub(crate) fn start_update_check_if_due(&mut self, cx: &mut Context<Self>) {
        if self.app.settings.check_for_updates
            && update_check::is_due(self.app.settings.last_update_check)
        {
            self.start_update_check(cx);
        }
    }

    pub(crate) fn start_update_check(&mut self, cx: &mut Context<Self>) {
        if self.app.update_check_task.is_some() {
            return;
        }

        self.app.settings.last_update_check = update_check::unix_timestamp();
        self.app.settings_save_error = self.app.settings.save().err();
        self.app.update_status = update_check::Status::Checking;
        cx.notify();

        let Some(http) = self.app.update_http_client.clone() else {
            self.app.update_status = update_check::Status::Failed(
                "update checks are unavailable in this application context".to_owned(),
            );
            cx.notify();
            return;
        };

        log::debug!(target: "nvim_gpui::update_check", "checking for updates");
        let request = cx.background_spawn(async move { update_check::check_latest(http).await });
        self.app.update_check_task = Some(cx.spawn(async move |weak, cx| {
            let status = request
                .await
                .unwrap_or_else(update_check::Status::Failed);
            let _ = weak.update(cx, |view, cx| {
                if matches!(&status, update_check::Status::Available { .. }) {
                    log::info!(target: "nvim_gpui::update_check", "a newer nvim-gpui release is available");
                }
                view.app.update_status = status;
                view.app.update_check_task = None;
                cx.notify();
            });
        }));
    }

    fn start_event_task(
        &mut self,
        events: async_channel::Receiver<NvimEvent>,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) {
        self.app.session.event_task = Some(cx.spawn(async move |weak, cx| {
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
                        target: "nvim_gpui::app",
                        "processing Neovim event batch: {} events",
                        batch.len()
                    );
                }
                let has_win_extmark = batch
                    .iter()
                    .any(|event| matches!(event, NvimEvent::WinExtmark { .. }));
                let should_reconcile_multicursors = batch.iter().any(|event| {
                    matches!(
                        event,
                        NvimEvent::GridLine { .. }
                            | NvimEvent::GridClear { .. }
                            | NvimEvent::WinExtmark { .. }
                            | NvimEvent::Flush
                    )
                });
                let should_notify = batch.iter().any(|event| {
                    matches!(
                        event,
                        NvimEvent::ApiReady { .. }
                            | NvimEvent::UiAttached { .. }
                            | NvimEvent::Flush
                            | NvimEvent::WinExtmark { .. }
                            | NvimEvent::Error(_)
                            | NvimEvent::Disconnected { .. }
                    )
                });
                let accepted = weak
                    .update(cx, |this, cx| {
                        if this.app.session.session_id != Some(session_id) {
                            return false;
                        }
                        for event in batch {
                            this.handle_nvim_event(event, cx);
                        }
                        if has_win_extmark
                            || !this
                                .editor
                                .protocol
                                .cursor
                                .unresolved_multicursor_positions
                                .is_empty()
                        {
                            this.schedule_multicursor_namespace_query(cx);
                        }
                        if should_reconcile_multicursors {
                            this.schedule_multicursor_reconcile(cx);
                        }
                        if should_notify {
                            cx.notify();
                        }
                        true
                    })
                    .unwrap_or(false);
                if !accepted {
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
        let Some(nvim) = self.app.session.nvim.as_ref() else {
            return;
        };
        if !nvim.is_remote() {
            return;
        }

        let (requests, request_queue) = crate::app::clipboard::channel();
        let get_requests = requests.clone();
        let set_requests = requests;
        self.app.session.clipboard_task = Some(cx.spawn(async move |_weak, cx| {
            while let Ok(request) = request_queue.recv().await {
                if cx
                    .update(|app| crate::app::clipboard::handle_on_ui_thread(app, request))
                    .is_err()
                {
                    break;
                }
            }
        }));

        let registration = {
            let nvim = self
                .app
                .session
                .nvim
                .as_ref()
                .expect("remote Neovim should be present");
            nvim.register_request_handler(
                crate::app::clipboard::CLIPBOARD_GET_METHOD,
                crate::app::clipboard::get_request_handler(get_requests),
            )
            .and_then(|_| {
                nvim.register_request_handler(
                    crate::app::clipboard::CLIPBOARD_SET_METHOD,
                    crate::app::clipboard::set_request_handler(set_requests),
                )
            })
        };
        if let Err(error) = registration {
            log::error!(
                target: "nvim_gpui::clipboard",
                "failed to register remote clipboard handlers: {error}"
            );
            self.app.session.clipboard_task = None;
            return;
        }

        let response = self
            .app
            .session
            .nvim
            .as_ref()
            .expect("remote Neovim should be present")
            .protocol()
            .map(|protocol| protocol.channel_id);
        let channel_id = response.unwrap_or_default();
        let response = self
            .app
            .session
            .nvim
            .as_ref()
            .expect("remote Neovim should be present")
            .request(
                "nvim_exec_lua",
                rmpv::Value::Array(vec![
                    rmpv::Value::from(crate::app::clipboard::remote_provider_lua(channel_id)),
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
                self.app.session.clipboard_task = None;
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

    pub(crate) fn handle_disconnect(&mut self, reason: DisconnectReason, cx: &mut Context<Self>) {
        self.editor.discard_pending_redraw();
        // Invalidate the disconnected connection before scheduling any
        // replacement. Late events and responses from its workers must not
        // be allowed to mutate state while reconnecting.
        self.app.session.session_id = None;
        log::info!(target: "nvim_gpui::app", "Neovim disconnected: reason={reason:?}");
        match reason {
            DisconnectReason::Requested => {}
            DisconnectReason::CleanExit => cx.quit(),
            reason => self.schedule_reconnect(reason, cx),
        }
    }

    fn schedule_reconnect(&mut self, reason: DisconnectReason, cx: &mut Context<Self>) {
        if self.app.session.reconnect_task.is_some() {
            return;
        }

        let Some(nvim) = self.app.session.nvim.as_ref() else {
            return;
        };
        let connection = nvim.connection_spec();
        let size = self
            .app
            .last_resize
            .or(self.editor.protocol.presentation.grid_size)
            .unwrap_or((DEFAULT_GRID_WIDTH, DEFAULT_GRID_HEIGHT));
        self.app.session.reconnect_attempt = self.app.session.reconnect_attempt.saturating_add(1);
        let attempt = self.app.session.reconnect_attempt;
        let backoff = 250_u64 * (1_u64 << attempt.saturating_sub(1).min(4));
        log::warn!(
            target: "nvim_gpui::app",
            "scheduling Neovim reconnect: attempt={}, backoff_ms={}, reason={reason:?}",
            attempt,
            backoff
        );
        self.app.session.rpc_status = format!("rpc: reconnecting (attempt {attempt})");

        let task = cx.background_spawn(async move {
            std::thread::sleep(Duration::from_millis(backoff));
            NvimProcess::connect_from_spec(&connection, size.0, size.1)
        });
        self.app.session.reconnect_task = Some(cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.app.session.reconnect_task = None;
                match result {
                    Ok(nvim) => {
                        log::info!(target: "nvim_gpui::app", "Neovim reconnected");
                        this.app.session.reconnect_attempt = 0;
                        this.install_reconnected_nvim(nvim, cx);
                    }
                    Err(error) => {
                        log::warn!(
                            target: "nvim_gpui::app",
                            "Neovim reconnect failed: {error}"
                        );
                        this.app.session.rpc_status = format!("rpc reconnect failed: {error}");
                        this.schedule_reconnect(reason.clone(), cx);
                    }
                }
                cx.notify();
            });
        }));
    }

    fn install_reconnected_nvim(&mut self, nvim: NvimProcess, cx: &mut Context<Self>) {
        let events = nvim.events();
        let session_id = nvim.session_id();
        let initial_theme = nvim.startup_theme().unwrap_or_default();
        let protocol = nvim.protocol().cloned();

        self.reset_nvim_session(initial_theme);
        self.app.session.nvim = Some(nvim);
        self.app.session.session_id = Some(session_id);
        self.app.session.api_level = protocol.as_ref().map(|protocol| protocol.version.api_level);
        self.app.session.nvim_version = protocol.map(|protocol| protocol.version);
        self.app.session.rpc_status = "rpc: reconnected".to_owned();
        log::info!(target: "nvim_gpui::app", "installed reconnected Neovim session");
        self.start_event_task(events, session_id, cx);
        self.start_remote_clipboard_bridge(cx);
        self.editor.protocol.startup.redraw_pending = true;
    }

    fn reset_nvim_session(&mut self, initial_theme: NvimTheme) {
        self.app.session.session_id = None;
        self.editor.protocol = ProtocolState::default();
        self.editor.protocol.theme = initial_theme;
        self.editor.protocol.presentation.grid_size =
            Some((DEFAULT_GRID_WIDTH, DEFAULT_GRID_HEIGHT));
        self.editor.protocol.startup.nvim_grid_ready = false;
        self.app.last_resize = None;
        self.editor.presentation.viewport_animations.clear();
        self.editor.cursor.multicursor_namespace_task = None;
        self.editor.cursor.multicursor_reconcile_task = None;
        self.editor.cursor.multicursor_reconcile_dirty = false;
        self.editor.input.ime_input_grid = None;
        self.editor.input.ime_coordinates_dirty = true;
        self.editor.protocol.presentation.image_store.clear();
        self.editor.presentation.image_sources.clear();
        self.editor.protocol.presentation.pending_ui_data.clear();
        self.editor.resolved_grid_font = None;
        self.editor.resolved_grid_wide_font = None;
        self.editor.shaping_cache.borrow_mut().clear();
        self.editor.glyph_coverage_cache.borrow_mut().clear();
        self.editor.invalidate_presentation_snapshot();
        self.editor.input.mouse_option = "nvi".to_owned();
        self.editor.input.mouse_enabled = true;
        self.editor.input.mouse_capture = None;
        self.editor.input.nvim_mode = "n".to_owned();
        self.editor.input.input_router = InputRouter::new(InputRouterConfig {
            rime_enabled: false,
        });
        self.editor.protocol.input_router = self.editor.input.input_router;
        self.editor.protocol.mouse_option = "nvi".to_owned();
        self.editor.protocol.mouse_enabled = true;
        self.editor.protocol.nvim_mode = "n".to_owned();
        self.editor.reset_rime_composition();
        self.editor.input.rime_menu_open = false;
        self.editor.input.rime_menu_message = None;
        self.editor.input.system_ime.clear();
        self.editor.input.scroll_remainder = point(0.0, 0.0);
        self.editor.cursor.cursor_blink_started_at = Instant::now();
        self.app.session.clipboard_task = None;
        self.window.window_title = DEFAULT_WINDOW_TITLE.to_owned();
        self.window.window_icon = "nvim-gpui".to_owned();
    }

    /// Intercept the native window close request for embedded Neovim long
    /// enough to report modified buffers. A remote session is only a client
    /// connection, so closing it must not inspect or modify server buffers.
    pub(crate) fn request_window_close(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.app.settings.quit_on_window_close {
            return true;
        }
        if self
            .app
            .session
            .nvim
            .as_ref()
            .is_some_and(NvimProcess::is_remote)
        {
            return true;
        }
        if matches!(self.window.quit_dialog, QuitDialogState::Quitting) {
            return true;
        }
        if !matches!(self.window.quit_dialog, QuitDialogState::Hidden) {
            return false;
        }
        let Some(nvim) = self.app.session.nvim.as_ref() else {
            return true;
        };
        let session_id = nvim.session_id();
        let response = match nvim.request(
            "nvim_exec_lua",
            rmpv::Value::Array(vec![
                rmpv::Value::from(MODIFIED_BUFFERS_LUA),
                rmpv::Value::Array(Vec::new()),
            ]),
        ) {
            Ok(response) => response,
            Err(error) => {
                self.window.quit_dialog = QuitDialogState::Confirm {
                    modified_buffers: Vec::new(),
                    error: Some(format!("Could not check for unsaved changes: {error}")),
                };
                cx.notify();
                return false;
            }
        };

        self.window.quit_dialog = QuitDialogState::Checking;
        cx.spawn(async move |weak, cx| {
            let result = response.recv().await;
            let _ = weak.update(cx, |this, cx| {
                if this.app.session.session_id != Some(session_id) {
                    return;
                }
                match result {
                    Ok(Ok(value)) => match parse_modified_buffers(value) {
                        Ok(modified_buffers) if modified_buffers.is_empty() => {
                            if this
                                .app
                                .session
                                .nvim
                                .as_ref()
                                .is_some_and(NvimProcess::is_remote)
                            {
                                this.window.quit_dialog = QuitDialogState::Quitting;
                                cx.quit();
                            } else {
                                this.begin_quit_command("qa", Vec::new(), cx);
                            }
                        }
                        Ok(modified_buffers) => {
                            this.window.quit_dialog = QuitDialogState::Confirm {
                                modified_buffers,
                                error: None,
                            };
                        }
                        Err(error) => {
                            this.window.quit_dialog = QuitDialogState::Confirm {
                                modified_buffers: Vec::new(),
                                error: Some(format!(
                                    "Could not check for unsaved changes: {error}"
                                )),
                            };
                        }
                    },
                    Ok(Err(error)) => {
                        this.window.quit_dialog = QuitDialogState::Confirm {
                            modified_buffers: Vec::new(),
                            error: Some(format!("Could not check for unsaved changes: {error}")),
                        };
                    }
                    Err(error) => {
                        this.window.quit_dialog = QuitDialogState::Confirm {
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
        if matches!(self.window.quit_dialog, QuitDialogState::Confirm { .. }) {
            self.window.quit_dialog = QuitDialogState::Hidden;
            cx.notify();
        }
    }

    pub(crate) fn save_and_quit(&mut self, cx: &mut Context<Self>) {
        if self
            .app
            .session
            .nvim
            .as_ref()
            .is_some_and(NvimProcess::is_remote)
        {
            self.window.quit_dialog = QuitDialogState::Quitting;
            cx.quit();
            return;
        }
        let modified_buffers = match &self.window.quit_dialog {
            QuitDialogState::Confirm {
                modified_buffers, ..
            } => modified_buffers.clone(),
            _ => return,
        };
        if modified_buffers
            .iter()
            .any(|buffer| buffer == UNNAMED_BUFFER_LABEL)
        {
            self.window.quit_dialog = QuitDialogState::Confirm {
                modified_buffers,
                error: Some(format!(
                    "Cannot save {UNNAMED_BUFFER_LABEL}: it has no file name. Save it with :write first, or choose Discard & Quit."
                )),
            };
            cx.notify();
            return;
        }
        self.window.quit_dialog = QuitDialogState::Saving;
        self.begin_quit_command("wall | qa", modified_buffers, cx);
        cx.notify();
    }

    pub(crate) fn discard_and_quit(&mut self, cx: &mut Context<Self>) {
        if self
            .app
            .session
            .nvim
            .as_ref()
            .is_some_and(NvimProcess::is_remote)
        {
            self.window.quit_dialog = QuitDialogState::Quitting;
            cx.quit();
            return;
        }
        let modified_buffers = match &self.window.quit_dialog {
            QuitDialogState::Confirm {
                modified_buffers, ..
            } => modified_buffers.clone(),
            _ => return,
        };
        self.begin_quit_command("qa!", modified_buffers, cx);
    }

    fn begin_quit_command(
        &mut self,
        command: &'static str,
        modified_buffers: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        self.window.quit_dialog = QuitDialogState::Quitting;
        let Some(nvim) = self.app.session.nvim.as_ref() else {
            cx.quit();
            return;
        };
        let session_id = nvim.session_id();
        let response = match nvim.request(
            "nvim_command",
            rmpv::Value::Array(vec![rmpv::Value::from(command)]),
        ) {
            Ok(response) => response,
            Err(error) => {
                self.window.quit_dialog = QuitDialogState::Confirm {
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
            let _ = weak.update(cx, |this, cx| {
                if this.app.session.session_id != Some(session_id) {
                    return;
                }
                match result {
                    Ok(Ok(_)) if remote => {
                        this.window.quit_dialog = QuitDialogState::Quitting;
                        cx.quit();
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => {
                        this.window.quit_dialog = QuitDialogState::Confirm {
                            modified_buffers,
                            error: Some(format!("Could not quit Neovim: {error}")),
                        };
                        cx.notify();
                    }
                    Err(error) => {
                        this.window.quit_dialog = QuitDialogState::Confirm {
                            modified_buffers,
                            error: Some(format!("Could not quit Neovim: {error}")),
                        };
                        cx.notify();
                    }
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
        app.editor.protocol.state.mode = "INSERT".to_owned();
        app.editor
            .protocol
            .presentation
            .other_grids
            .insert(2, Rc::new(grid::GridModel::new(4, 2)));
        app.editor.protocol.presentation.pending_grid = Some(Rc::new(grid::GridModel::new(3, 1)));
        app.editor
            .protocol
            .presentation
            .pending_other_grids
            .insert(3, Rc::new(grid::GridModel::new(2, 1)));
        app.editor
            .protocol
            .presentation
            .pending_grid_placements
            .insert(3, GridPlacement::default());
        app.editor
            .protocol
            .presentation
            .pending_destroyed_grids
            .insert(4);
        app.editor.protocol.cursor.pending_cursor_grid = Some(3);
        app.editor.protocol.pending_theme = Some(NvimTheme {
            default_background: Some(0x101010),
            ..Default::default()
        });
        app.editor.protocol.apply(NvimEvent::OptionSet {
            name: "linespace".to_owned(),
            value: "1".to_owned(),
        });
        app.editor
            .input
            .system_ime
            .replace_and_mark_text(None, "compose", None);

        let next_theme = NvimTheme {
            normal_background: Some(0x202020),
            ..Default::default()
        };
        app.reset_nvim_session(next_theme);

        assert!(app.app.session.session_id.is_none());
        assert!(app.editor.protocol.presentation.other_grids.is_empty());
        assert!(app.editor.protocol.presentation.pending_grid.is_none());
        assert!(app.editor.protocol.presentation.pending_grid_size.is_none());
        assert!(app
            .editor
            .protocol
            .presentation
            .pending_other_grids
            .is_empty());
        assert!(app
            .editor
            .protocol
            .presentation
            .pending_grid_placements
            .is_empty());
        assert!(app
            .editor
            .protocol
            .presentation
            .pending_destroyed_grids
            .is_empty());
        assert!(app.editor.protocol.cursor.pending_cursor_grid.is_none());
        assert!(app.editor.protocol.pending_theme.is_none());
        assert!(!app.editor.protocol.has_pending_redraw());
        assert!(app.editor.protocol.presentation.pending_ui_data.is_empty());
        assert!(app.editor.presentation.presentation_snapshot.is_none());
        assert_eq!(
            app.editor.protocol.presentation.grid_size,
            Some((DEFAULT_GRID_WIDTH, DEFAULT_GRID_HEIGHT))
        );
        assert!(!app.editor.protocol.startup.grid_content_seen);
        assert!(!app.editor.protocol.startup.redraw_pending);
        assert_eq!(app.editor.protocol.theme.normal_background, Some(0x202020));
        assert_eq!(app.editor.protocol.state.mode, "NORMAL");
        assert!(app.editor.input.system_ime.is_empty());
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
