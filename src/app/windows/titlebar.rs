use super::super::*;
use crate::{
    gui,
    widgets::{logo_image, titlebar_button, IME_ACTIVE, MUTED_TEXT, SURFACE, SURFACE_BRIGHT, TEXT},
};
use gpui::deferred;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use gpui::Window;
#[cfg(target_os = "linux")]
use gpui::{CursorStyle, Decorations, Div, ResizeEdge};

#[derive(Clone)]
pub(crate) struct RimeTitlebarState {
    pub enabled: bool,
    pub active: bool,
    pub menu_open: bool,
    pub menu_message: Option<String>,
}

pub(crate) fn themed_titlebar_enabled() -> bool {
    cfg!(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows"
    ))
}

pub(crate) fn themed_window_decorations() -> Option<WindowDecorations> {
    #[cfg(target_os = "linux")]
    {
        Some(WindowDecorations::Client)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn themed_resize_handles(window: &Window) -> Option<impl IntoElement> {
    matches!(window.window_decorations(), Decorations::Client { .. })
        .then_some(window_resize_handles())
}

pub(crate) fn themed_titlebar_options(title: &'static str) -> TitlebarOptions {
    TitlebarOptions {
        title: Some(title.into()),
        appears_transparent: cfg!(any(target_os = "macos", target_os = "windows")),
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
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let close_source = source.clone();

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
        .child(img(logo_image()).w(px(20.0)).h(px(20.0)))
        .child(div().w(px(6.0)))
        .child(title);

    #[cfg(target_os = "macos")]
    let title_area = title_area.on_mouse_down(MouseButton::Left, |event, window, _cx| {
        if event.click_count == 2 {
            window.titlebar_double_click();
        }
    });

    #[cfg(target_os = "linux")]
    let title_area = title_area.on_mouse_down(MouseButton::Left, |event, window, _cx| {
        if event.click_count == 2 {
            window.zoom_window();
        } else {
            window.start_window_move();
        }
    });

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

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let titlebar = titlebar
        .child(window_control_button(
            "—",
            WindowControlArea::Min,
            background,
            foreground,
            |window, _cx| window.minimize_window(),
        ))
        .child(window_control_button(
            "□",
            WindowControlArea::Max,
            background,
            foreground,
            |window, _cx| window.zoom_window(),
        ))
        .child(window_control_button(
            "×",
            WindowControlArea::Close,
            background,
            foreground,
            move |window, cx| {
                let should_close = close_source
                    .as_ref()
                    .map(|view| view.update(cx, |view, cx| view.request_window_close(cx)))
                    .unwrap_or(true);
                if should_close {
                    if close_source.is_some() {
                        cx.quit();
                    } else {
                        window.remove_window();
                    }
                }
            },
        ));

    titlebar
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn window_control_button(
    label: &'static str,
    area: WindowControlArea,
    background: u32,
    foreground: u32,
    action: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(label)
        .w(px(46.0))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(background))
        .text_color(rgb(foreground))
        .window_control_area(area)
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            window.prevent_default();
            action(window, cx);
        })
        .child(
            div()
                .text_size(px(if label == "×" { 20.0 } else { 16.0 }))
                .child(label),
        )
}

#[cfg(target_os = "linux")]
const WINDOW_RESIZE_EDGE_SIZE: f32 = 6.0;

#[cfg(target_os = "linux")]
const WINDOW_RESIZE_CORNER_SIZE: f32 = 12.0;

#[cfg(target_os = "linux")]
fn window_resize_handles() -> impl IntoElement {
    div()
        .absolute()
        .left(px(0.0))
        .top(px(0.0))
        .right(px(0.0))
        .bottom(px(0.0))
        .child(window_resize_handle(
            "window-resize-top",
            ResizeEdge::Top,
            |handle| {
                handle
                    .left(px(WINDOW_RESIZE_CORNER_SIZE))
                    .top(px(0.0))
                    .right(px(WINDOW_RESIZE_CORNER_SIZE))
                    .h(px(WINDOW_RESIZE_EDGE_SIZE))
            },
        ))
        .child(window_resize_handle(
            "window-resize-right",
            ResizeEdge::Right,
            |handle| {
                handle
                    .top(px(WINDOW_RESIZE_CORNER_SIZE))
                    .right(px(0.0))
                    .bottom(px(WINDOW_RESIZE_CORNER_SIZE))
                    .w(px(WINDOW_RESIZE_EDGE_SIZE))
            },
        ))
        .child(window_resize_handle(
            "window-resize-bottom",
            ResizeEdge::Bottom,
            |handle| {
                handle
                    .left(px(WINDOW_RESIZE_CORNER_SIZE))
                    .right(px(WINDOW_RESIZE_CORNER_SIZE))
                    .bottom(px(0.0))
                    .h(px(WINDOW_RESIZE_EDGE_SIZE))
            },
        ))
        .child(window_resize_handle(
            "window-resize-left",
            ResizeEdge::Left,
            |handle| {
                handle
                    .left(px(0.0))
                    .top(px(WINDOW_RESIZE_CORNER_SIZE))
                    .bottom(px(WINDOW_RESIZE_CORNER_SIZE))
                    .w(px(WINDOW_RESIZE_EDGE_SIZE))
            },
        ))
        .child(window_resize_handle(
            "window-resize-top-left",
            ResizeEdge::TopLeft,
            |handle| {
                handle
                    .left(px(0.0))
                    .top(px(0.0))
                    .w(px(WINDOW_RESIZE_CORNER_SIZE))
                    .h(px(WINDOW_RESIZE_CORNER_SIZE))
            },
        ))
        .child(window_resize_handle(
            "window-resize-top-right",
            ResizeEdge::TopRight,
            |handle| {
                handle
                    .top(px(0.0))
                    .right(px(0.0))
                    .w(px(WINDOW_RESIZE_CORNER_SIZE))
                    .h(px(WINDOW_RESIZE_CORNER_SIZE))
            },
        ))
        .child(window_resize_handle(
            "window-resize-bottom-right",
            ResizeEdge::BottomRight,
            |handle| {
                handle
                    .right(px(0.0))
                    .bottom(px(0.0))
                    .w(px(WINDOW_RESIZE_CORNER_SIZE))
                    .h(px(WINDOW_RESIZE_CORNER_SIZE))
            },
        ))
        .child(window_resize_handle(
            "window-resize-bottom-left",
            ResizeEdge::BottomLeft,
            |handle| {
                handle
                    .left(px(0.0))
                    .bottom(px(0.0))
                    .w(px(WINDOW_RESIZE_CORNER_SIZE))
                    .h(px(WINDOW_RESIZE_CORNER_SIZE))
            },
        ))
}

#[cfg(target_os = "linux")]
fn window_resize_handle(
    id: &'static str,
    edge: ResizeEdge,
    place: impl FnOnce(Div) -> Div,
) -> impl IntoElement {
    place(div().absolute().cursor(resize_cursor(edge)))
        .id(id)
        .on_mouse_down(MouseButton::Left, move |_, window, _cx| {
            window.prevent_default();
            window.start_window_resize(edge);
        })
}

#[cfg(target_os = "linux")]
fn resize_cursor(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
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
