mod about;
mod quit_confirmation;
mod settings;
mod startup_error;

pub(crate) use about::AboutWindow;
pub(crate) use quit_confirmation::quit_confirmation_dialog;
pub(crate) use settings::SettingsWindow;
pub(crate) use startup_error::startup_error_dialog;

use crate::app::{themed_titlebar_options, themed_window_decorations, NvimGpui};
use gpui::{
    prelude::*, size, App, Bounds, Entity, WindowBounds, WindowHandle, WindowKind, WindowOptions,
};

pub(crate) fn dialog_overlay(id: &'static str, panel: impl IntoElement) -> gpui::Div {
    let backdrop = gpui::div()
        .id(id)
        .absolute()
        .left(gpui::px(0.0))
        .top(gpui::px(0.0))
        .w_full()
        .h_full()
        .bg(gpui::rgba(0x00000099))
        .on_any_mouse_down(|_, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
        });

    gpui::div()
        .absolute()
        .left(gpui::px(0.0))
        .top(gpui::px(0.0))
        .w_full()
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .child(backdrop)
        .child(panel)
}

pub(crate) fn open_settings_window(source: Entity<NvimGpui>, cx: &mut App) {
    let existing = source.read(cx).settings_window_handle();
    if let Some(handle) = existing {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }

    let bounds = Bounds::centered(None, size(gpui::px(720.0), gpui::px(560.0)), cx);
    let handle: WindowHandle<SettingsWindow> = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(themed_titlebar_options("nvim-gpui settings")),
                window_decorations: themed_window_decorations(),
                kind: WindowKind::Floating,
                is_resizable: true,
                window_min_size: Some(size(gpui::px(560.0), gpui::px(420.0))),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| SettingsWindow::new(source.clone(), cx)),
        )
        .expect("failed to open nvim-gpui settings window");
    source.update(cx, |view, _| view.set_settings_window_handle(handle));
}

pub(crate) fn open_about_window(source: Entity<NvimGpui>, cx: &mut App) {
    let existing = source.read(cx).about_window_handle();
    if let Some(handle) = existing {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }

    let bounds = Bounds::centered(None, size(gpui::px(440.0), gpui::px(320.0)), cx);
    let handle: WindowHandle<AboutWindow> = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(themed_titlebar_options("About nvim-gpui")),
                window_decorations: themed_window_decorations(),
                kind: WindowKind::Floating,
                is_resizable: false,
                ..Default::default()
            },
            |_, cx| cx.new(|_| AboutWindow),
        )
        .expect("failed to open nvim-gpui about window");
    source.update(cx, |view, _| view.set_about_window_handle(handle));
}
