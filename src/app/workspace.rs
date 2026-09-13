#[cfg(target_os = "linux")]
use super::themed_resize_handles;
use super::{
    themed_titlebar, themed_titlebar_enabled, NvimGpui, QuitDialogState, RimeTitlebarState,
    THEMED_TITLEBAR_HEIGHT,
};
use crate::{
    gui,
    input::{key_to_nvim_input, should_route_key_to_neovim, InputTarget},
    settings,
};
use gpui::{div, prelude::*, px, rgb, Context, IntoElement, KeyDownEvent, Render, Window};

impl Render for NvimGpui {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_window_title(&self.window.window_title);

        let theme_background = self.editor.theme_background();
        let theme_foreground = self.editor.theme_foreground();
        let entity = cx.entity();
        let mut workspace = div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(rgb(theme_background))
            .text_color(rgb(theme_foreground))
            .capture_key_down(cx.listener(Self::on_key_down))
            .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
            .on_drag_move::<gpui::ExternalPaths>(cx.listener(Self::on_file_drag_move))
            .on_drop::<gpui::ExternalPaths>(cx.listener(Self::on_file_drop));

        if let Some(focus_handle) = self.window.focus_handle.as_ref() {
            workspace = workspace.track_focus(focus_handle);
        }

        if themed_titlebar_enabled() {
            workspace = workspace.child(themed_titlebar(
                self.window.window_title.clone(),
                theme_background,
                theme_foreground,
                Some(entity),
                (self.app.settings.ime_backend == settings::ImeBackend::Rime
                    && (self.editor.input.rime_service.is_some()
                        || self.editor.input.rime_deploy_task.is_some()))
                .then_some(RimeTitlebarState {
                    enabled: self
                        .editor
                        .input
                        .input_router
                        .rime_enabled_for_toggle_context(),
                    active: self.editor.input.rime_service.is_some()
                        && self.editor.input.input_router.target() == InputTarget::Rime,
                    menu_open: self.editor.input.rime_menu_open,
                    menu_message: self.editor.input.rime_menu_message.clone(),
                }),
                window.is_maximized(),
            ));
        }

        workspace = workspace.child(self.render_editor_surface(window, cx));

        if let Some(message) = self.window.file_drop_notice.as_ref() {
            workspace = workspace.child(
                div()
                    .absolute()
                    .top(px(12.0))
                    .left(px(12.0))
                    .right(px(12.0))
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .rounded_sm()
                            .border_1()
                            .border_color(rgb(crate::widgets::WARNING))
                            .bg(rgb(crate::widgets::SURFACE))
                            .text_color(rgb(crate::widgets::WARNING))
                            .text_sm()
                            .px_3()
                            .py_2()
                            .child(message.clone()),
                    ),
            );
        }

        #[cfg(target_os = "linux")]
        if let Some(resize_handles) = themed_resize_handles(window) {
            workspace = workspace.child(resize_handles);
        }

        if let Some(quit_dialog) = gui::quit_confirmation_dialog(&self.window.quit_dialog, cx) {
            workspace = workspace.child(quit_dialog);
        }
        if let Some(startup_error_dialog) = gui::startup_error_dialog(
            self.app.session.nvim.is_some(),
            &self.app.session.rpc_status,
        ) {
            workspace = workspace.child(startup_error_dialog);
        }

        workspace
    }
}

impl NvimGpui {
    pub(crate) fn update_settings(&mut self, next: settings::Settings) {
        let ime_backend_changed = self.app.settings.ime_backend != next.ime_backend;
        if self.app.settings.log_level != next.log_level {
            if let Some(logger) = self.app.logger.as_ref() {
                crate::logging::set_level(logger, next.log_level);
            }
        }
        self.app.settings = next;
        self.editor.apply_runtime_settings(&self.app.settings);
        if ime_backend_changed {
            self.editor
                .apply_ime_backend_setting(self.app.settings.ime_backend);
        }
        self.app.settings_save_error = self.app.settings.save().err();
    }

    pub(crate) fn sync_nvim_size(&mut self, window: &mut Window) {
        let gui_font = self.editor.current_grid_font(window);
        let cell_width = gui_font.cell_width(window);
        let line_height = gui_font.line_height(window, self.editor.protocol.linespace);
        let viewport = window.viewport_size();
        let available_height = f32::from(viewport.height)
            - if cfg!(any(
                target_os = "linux",
                target_os = "macos",
                target_os = "windows"
            )) {
                THEMED_TITLEBAR_HEIGHT
            } else {
                0.0
            };
        let width = (f32::from(viewport.width) / f32::from(cell_width))
            .floor()
            .max(1.0) as u32;
        let height = (available_height / f32::from(line_height)).floor().max(1.0) as u32;
        let size = (width, height);

        if self.editor.protocol.startup.maximize_pending {
            if !window.is_maximized() {
                return;
            }
            self.editor.complete_startup_maximize();
        }

        if !self.editor.protocol.startup.nvim_grid_ready {
            if self.app.last_resize != Some(size) {
                self.editor.protocol.startup.redraw_pending = true;
            }
            self.editor.protocol.startup.resize_target = Some(size);
            self.editor.update_startup_grid_ready();
            if self.editor.protocol.startup.nvim_grid_ready
                && !self.editor.protocol.startup.redraw_pending
            {
                return;
            }
        }

        let Some(nvim) = self.app.session.nvim.as_ref() else {
            return;
        };

        let resize_succeeded = if self.app.last_resize == Some(size) {
            true
        } else {
            match nvim.send_resize(width, height) {
                Ok(()) => {
                    log::debug!(
                        target: "nvim_gpui::app",
                        "sent Neovim resize: width={}, height={}",
                        width,
                        height
                    );
                    self.app.last_resize = Some(size);
                    true
                }
                Err(error) => {
                    log::error!(target: "nvim_gpui::app", "Neovim resize failed: {error}");
                    self.app.session.rpc_status = format!("rpc resize error: {error}");
                    false
                }
            }
        };

        if resize_succeeded && self.editor.protocol.startup.redraw_pending {
            self.editor.protocol.startup.redraw_pending = false;
            self.app.session.request_startup_redraw();
        }
    }

    fn paste_from_system_clipboard(&mut self, cx: &mut Context<Self>) {
        let text = match crate::app::clipboard::paste_text(cx) {
            Ok(text) => text,
            Err(error) => {
                log::warn!(
                    target: "nvim_gpui::clipboard",
                    "could not read system clipboard for paste: {error}"
                );
                return;
            }
        };
        let Some(nvim) = self.app.session.nvim.as_ref() else {
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
                self.app.session.rpc_status = format!("rpc paste error: {error}");
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

    pub(crate) fn on_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(
            &self.window.quit_dialog,
            QuitDialogState::Hidden | QuitDialogState::Quitting
        ) {
            match (&self.window.quit_dialog, event.keystroke.key.as_str()) {
                (QuitDialogState::Confirm { .. }, "escape") => {
                    self.cancel_quit_dialog(cx);
                }
                (QuitDialogState::Confirm { .. }, "enter" | "return") => {
                    self.save_and_quit(cx);
                }
                _ => {}
            }
            window.prevent_default();
            cx.stop_propagation();
            return;
        }

        if self.app.settings.ime_backend == settings::ImeBackend::Rime
            && self
                .app
                .settings
                .rime_toggle_shortcut
                .matches(&event.keystroke)
            && self.editor.input.rime_service.is_some()
        {
            if self.editor.toggle_rime() {
                cx.notify();
            }
            window.prevent_default();
            cx.stop_propagation();
            return;
        }

        if self.app.settings.paste_shortcut.matches(&event.keystroke) {
            self.paste_from_system_clipboard(cx);
            window.prevent_default();
            return;
        }

        let mut target = self.editor.input.input_router.target();
        // The deployer uses librime's process-global data APIs. Keep the live
        // session out of that API while a background redeploy is in progress.
        if target == InputTarget::Rime && self.editor.input.rime_deploy_task.is_some() {
            target = InputTarget::Neovim;
        }
        log::debug!(
            target: "nvim_gpui::input",
            "key down: key={:?}, key_char={:?}, modifiers={:?}, target={target:?}",
            event.keystroke.key,
            event.keystroke.key_char,
            event.keystroke.modifiers
        );
        if target == InputTarget::Rime {
            match self.handle_rime_key(&event.keystroke, cx) {
                Ok(true) => {
                    window.prevent_default();
                    return;
                }
                Ok(false) => target = InputTarget::Neovim,
                Err(error) => {
                    self.editor.disable_rime(&error);
                    target = InputTarget::Neovim;
                }
            }
        }
        if !should_route_key_to_neovim(target, &event.keystroke) {
            return;
        }

        if let Some(nvim) = self.app.session.nvim.as_ref() {
            if let Err(error) = nvim.send_input(key_to_nvim_input(&event.keystroke)) {
                log::error!(target: "nvim_gpui::input", "key event failed: {error}");
                self.app.session.rpc_status = format!("rpc input error: {error}");
            }
        }
        // Prevent GPUI's default key action from competing with Neovim for
        // editor-owned shortcuts such as Ctrl-W, Cmd-W, and function keys.
        // This only applies after the event reaches this window; OS-global
        // shortcuts remain owned by the operating system.
        window.prevent_default();
    }
}
