mod about;
mod settings;

pub(crate) use about::AboutWindow;
pub(crate) use settings::SettingsWindow;

use crate::app::{themed_titlebar_options, NvimGpui};
use gpui::{
    prelude::*, size, App, Bounds, Entity, WindowBounds, WindowHandle, WindowKind, WindowOptions,
};

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
                kind: WindowKind::Floating,
                is_resizable: false,
                ..Default::default()
            },
            |_, cx| cx.new(|_| AboutWindow),
        )
        .expect("failed to open nvim-gpui about window");
    source.update(cx, |view, _| view.set_about_window_handle(handle));
}
