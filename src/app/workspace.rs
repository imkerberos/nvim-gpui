#[cfg(target_os = "linux")]
use super::themed_resize_handles;
use super::{
    themed_titlebar, themed_titlebar_enabled, NvimGpui, QuitDialogState, RimeTitlebarState,
    THEMED_TITLEBAR_HEIGHT,
};
use crate::{
    editor::{parse_guifont_spec, GuiFontSpec},
    grid, gui,
    input::{key_to_nvim_input, should_route_key_to_neovim, InputTarget},
    settings,
    widgets::{BACKGROUND, TEXT},
};
use gpui::{div, prelude::*, px, rgb, Context, IntoElement, KeyDownEvent, Render, Window};

impl Render for NvimGpui {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_window_title(&self.window.window_title);

        let theme_background = self.theme_background();
        let theme_foreground = self.theme_foreground();
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
    pub(crate) fn apply_runtime_settings(&mut self) {
        self.editor.nerd_font_family = self
            .editor
            .bundled_nerd_font_registered
            .then(|| self.app.settings.nerd_font.family().to_owned());
        self.editor.shaping_cache.borrow_mut().clear();
        self.editor.glyph_coverage_cache.borrow_mut().clear();
        for image in self
            .editor
            .protocol
            .presentation
            .image_store
            .set_cache_size_mb(self.app.settings.image_cache_size_mb)
        {
            self.editor.presentation.image_sources.remove(&image);
        }
    }

    pub(crate) fn update_settings(&mut self, next: settings::Settings) {
        let ime_backend_changed = self.app.settings.ime_backend != next.ime_backend;
        if self.app.settings.log_level != next.log_level {
            if let Some(logger) = self.logger.as_ref() {
                crate::logging::set_level(logger, next.log_level);
            }
        }
        self.app.settings = next;
        self.apply_runtime_settings();
        if ime_backend_changed {
            self.apply_ime_backend_setting();
        }
        self.app.settings_save_error = self.app.settings.save().err();
    }

    pub(crate) fn current_grid_font(&mut self, window: &Window) -> GuiFontSpec {
        if let Some(font) = &self.editor.resolved_grid_font {
            return font.clone();
        }

        let font = self
            .editor
            .protocol
            .guifont
            .as_deref()
            .filter(|spec| !spec.trim().is_empty())
            .map(parse_guifont_spec)
            .unwrap_or_else(|| GuiFontSpec::system(window));
        self.editor.resolved_grid_font = Some(font.clone());
        font
    }

    pub(crate) fn current_grid_wide_font(&mut self, window: &Window) -> GuiFontSpec {
        if let Some(font) = &self.editor.resolved_grid_wide_font {
            return font.clone();
        }

        let font = if let Some(spec) = self
            .editor
            .protocol
            .guifontwide
            .as_deref()
            .filter(|spec| !spec.trim().is_empty())
        {
            parse_guifont_spec(spec)
        } else {
            self.current_grid_font(window)
        };
        self.editor.resolved_grid_wide_font = Some(font.clone());
        font
    }

    pub(crate) fn current_cursor_mode(&self) -> grid::CursorModeInfo {
        if !self.editor.protocol.cursor.cursor_style_enabled {
            return grid::CursorModeInfo::default();
        }
        self.editor
            .protocol
            .cursor
            .cursor_modes
            .get(self.editor.protocol.cursor.cursor_mode_index)
            .copied()
            .unwrap_or_default()
    }

    pub(crate) fn theme_background(&self) -> u32 {
        self.editor
            .protocol
            .theme
            .normal_background
            .or(self.editor.protocol.theme.default_background)
            .unwrap_or(BACKGROUND)
    }

    pub(crate) fn theme_foreground(&self) -> u32 {
        self.editor
            .protocol
            .theme
            .normal_foreground
            .or(self.editor.protocol.theme.default_foreground)
            .unwrap_or(TEXT)
    }

    pub(super) fn update_startup_grid_ready(&mut self) {
        if self.editor.protocol.startup.nvim_grid_ready
            || !self.editor.protocol.startup.flush_seen
            || !self.editor.protocol.startup.grid_content_seen
        {
            return;
        }

        let Some(target) = self.editor.protocol.startup.resize_target else {
            return;
        };
        let committed_size = (
            self.editor.protocol.presentation.grid.width() as u32,
            self.editor.protocol.presentation.grid.height() as u32,
        );
        if committed_size == target {
            self.editor.protocol.startup.nvim_grid_ready = true;
        }
    }

    pub(crate) fn complete_startup_maximize(&mut self) {
        self.editor.protocol.startup.maximize_pending = false;
        self.editor.protocol.startup.resize_target = None;
        self.editor.protocol.startup.flush_seen = false;
        self.editor.protocol.startup.grid_content_seen = false;
        self.editor.protocol.startup.redraw_pending = true;
        log::debug!(
            target: "nvim_gpui::app",
            "startup window maximized; waiting for the final Neovim grid"
        );
    }

    pub(crate) fn sync_nvim_size(&mut self, window: &mut Window) {
        let gui_font = self.current_grid_font(window);
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
            self.complete_startup_maximize();
        }

        if !self.editor.protocol.startup.nvim_grid_ready {
            if self.app.last_resize != Some(size) {
                self.editor.protocol.startup.redraw_pending = true;
            }
            self.editor.protocol.startup.resize_target = Some(size);
            self.update_startup_grid_ready();
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
            self.request_startup_redraw();
        }
    }

    pub(super) fn request_startup_redraw(&self) {
        let Some(nvim) = self.app.session.nvim.as_ref() else {
            return;
        };
        match nvim.request(
            "nvim_command",
            rmpv::Value::Array(vec![rmpv::Value::from("redraw!")]),
        ) {
            Ok(_) => log::debug!(
                target: "nvim_gpui::app",
                "requested a complete redraw after Neovim startup/reconnect"
            ),
            Err(error) => log::warn!(
                target: "nvim_gpui::app",
                "could not request a complete redraw after Neovim startup/reconnect: {error}"
            ),
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
            self.toggle_rime(cx);
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
                    self.disable_rime(&error);
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
