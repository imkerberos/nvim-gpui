use super::{themed_titlebar, themed_titlebar_enabled, NvimGpui};
use crate::{app::windows::RimeTitlebarState, gui, input::InputTarget, settings};
use gpui::{div, prelude::*, rgb, Context, Render, Window};

impl Render for NvimGpui {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_window_title(&self.window_title);

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
            .on_modifiers_changed(cx.listener(Self::on_modifiers_changed));

        if let Some(focus_handle) = self.focus_handle.as_ref() {
            workspace = workspace.track_focus(focus_handle);
        }

        if themed_titlebar_enabled() {
            workspace = workspace.child(themed_titlebar(
                self.window_title.clone(),
                theme_background,
                theme_foreground,
                Some(entity),
                (self.settings.ime_backend == settings::ImeBackend::Rime
                    && self.rime_backend.is_some())
                .then_some(RimeTitlebarState {
                    enabled: self.input_router.config().rime_enabled,
                    active: self.input_router.target() == InputTarget::Rime,
                    menu_open: self.rime_menu_open,
                    menu_message: self.rime_menu_message.clone(),
                }),
            ));
        }

        workspace = workspace.child(self.render_editor_surface(window, cx));

        #[cfg(target_os = "linux")]
        if let Some(resize_handles) = super::themed_resize_handles(window) {
            workspace = workspace.child(resize_handles);
        }

        if let Some(quit_dialog) = gui::quit_confirmation_dialog(&self.quit_dialog, cx) {
            workspace = workspace.child(quit_dialog);
        }
        if let Some(startup_error_dialog) =
            gui::startup_error_dialog(self.nvim.is_some(), &self.rpc_status)
        {
            workspace = workspace.child(startup_error_dialog);
        }

        workspace
    }
}
