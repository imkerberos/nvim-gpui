use super::super::*;
#[cfg(target_os = "windows")]
use crate::widgets::window_control_button;
use crate::{
    gui,
    widgets::{logo_image, titlebar_button},
};

pub(crate) fn themed_titlebar_enabled() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

pub(crate) fn themed_titlebar_options(title: &'static str) -> TitlebarOptions {
    TitlebarOptions {
        title: Some(title.into()),
        appears_transparent: themed_titlebar_enabled(),
        ..Default::default()
    }
}

pub(crate) fn themed_titlebar(
    title: String,
    background: u32,
    foreground: u32,
    source: Option<Entity<NvimGpui>>,
) -> impl IntoElement {
    let title_area = div()
        .flex_1()
        .h_full()
        .flex()
        .items_center()
        .justify_start()
        .pl(px(if cfg!(target_os = "macos") {
            76.0
        } else {
            12.0
        }))
        .text_color(rgb(foreground))
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |event, window, _cx| {
            if event.click_count == 2 {
                // On macOS this forwards to AppKit's standard titlebar
                // double-click action (normally zoom/maximize). On Windows,
                // WindowControlArea::Drag lets the native caption handling do
                // the same job, so this is harmless there.
                window.titlebar_double_click();
            }
        })
        .child(img(logo_image()).w(px(20.0)).h(px(20.0)))
        .child(div().w(px(6.0)))
        .child(title);

    let mut titlebar = div()
        .w_full()
        .h(px(THEMED_TITLEBAR_HEIGHT))
        .flex()
        .items_center()
        .bg(rgb(background))
        .child(title_area);

    if let Some(source) = source {
        let settings_source = source.clone();
        let about_source = source;
        let actions = div()
            .h_full()
            .flex()
            .items_center()
            .pr(px(8.0))
            .child(titlebar_button("Settings", foreground, move |cx| {
                gui::open_settings_window(settings_source.clone(), cx);
            }))
            .child(div().w(px(4.0)))
            .child(titlebar_button("About", foreground, move |cx| {
                gui::open_about_window(about_source.clone(), cx);
            }));
        titlebar = titlebar.child(actions);
    }

    #[cfg(target_os = "windows")]
    let titlebar = titlebar
        .child(window_control_button(
            "—",
            WindowControlArea::Min,
            background,
            foreground,
        ))
        .child(window_control_button(
            "□",
            WindowControlArea::Max,
            background,
            foreground,
        ))
        .child(window_control_button(
            "×",
            WindowControlArea::Close,
            background,
            foreground,
        ));

    titlebar
}
