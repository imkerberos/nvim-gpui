use super::super::*;
#[cfg(target_os = "windows")]
use crate::widgets::window_control_button;
use crate::{
    gui,
    widgets::{logo_image, titlebar_button, IME_ACTIVE, MUTED_TEXT, SURFACE, SURFACE_BRIGHT, TEXT},
};
use gpui::deferred;

#[derive(Clone)]
pub(crate) struct RimeTitlebarState {
    pub enabled: bool,
    pub active: bool,
    pub menu_open: bool,
    pub menu_message: Option<String>,
}

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
    rime_state: Option<RimeTitlebarState>,
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
        let rime_source = source.clone();
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
            }))
            .when_some(rime_state, |actions, state| {
                let label = if state.enabled { "㞢" } else { "En" };
                let color = if state.enabled && state.active {
                    IME_ACTIVE
                } else {
                    MUTED_TEXT
                };
                actions.child(div().w(px(4.0))).child(rime_indicator(
                    label,
                    color,
                    state,
                    rime_source,
                ))
            });
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

fn rime_indicator(
    label: &'static str,
    foreground: u32,
    state: RimeTitlebarState,
    source: Entity<NvimGpui>,
) -> impl IntoElement {
    let toggle_source = source.clone();
    let open_menu_source = source.clone();
    let mut indicator = div()
        .id("titlebar-rime-indicator")
        .relative()
        .w(px(28.0))
        .h(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .text_sm()
        .text_color(rgb(foreground))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(SURFACE_BRIGHT)).text_color(rgb(IME_ACTIVE)))
        .on_click(move |_, _, cx| {
            toggle_source.update(cx, |view, cx| view.toggle_rime(cx));
        })
        .on_mouse_down(MouseButton::Right, move |_, window, cx| {
            open_menu_source.update(cx, |view, cx| view.open_rime_menu(cx));
            window.prevent_default();
        })
        .child(label);

    if state.menu_open {
        let close_source = source.clone();
        let deploy_source = source.clone();
        let user_data_source = source;
        let mut menu = div()
            .id("titlebar-rime-menu")
            .absolute()
            .right(px(0.0))
            .top(px(28.0))
            .w(px(240.0))
            .p_1()
            .rounded_sm()
            .border_1()
            .border_color(rgb(SURFACE_BRIGHT))
            .bg(rgb(SURFACE))
            .text_color(rgb(TEXT))
            .on_mouse_down_out(move |_, _, cx| {
                close_source.update(cx, |view, cx| view.close_rime_menu(cx));
            })
            .child(rime_menu_item(
                "titlebar-rime-deploy",
                "Deploy / Redeploy Rime data",
                move |cx| {
                    deploy_source.update(cx, |view, cx| view.redeploy_rime(cx));
                },
            ))
            .child(rime_menu_item(
                "titlebar-rime-user-settings",
                "Open Rime user settings",
                move |cx| {
                    user_data_source.update(cx, |view, cx| view.open_rime_user_data_directory(cx));
                },
            ));

        if let Some(message) = state.menu_message {
            menu = menu.child(
                div()
                    .mt_1()
                    .px_2()
                    .text_xs()
                    .text_color(rgb(MUTED_TEXT))
                    .child(message),
            );
        }

        indicator = indicator.child(deferred(menu).with_priority(2));
    }

    indicator
}

fn rime_menu_item(
    id: &'static str,
    label: &'static str,
    on_click: impl Fn(&mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .w_full()
        .px_2()
        .py_2()
        .rounded_sm()
        .text_sm()
        .text_color(rgb(TEXT))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(SURFACE_BRIGHT)))
        .on_click(move |_, _, cx| on_click(cx))
        .child(label)
}
