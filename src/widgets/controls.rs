use gpui::{div, prelude::*, px, rgb, App, ClickEvent, IntoElement, SharedString};

use super::{ACCENT, BACKGROUND, SURFACE, SURFACE_BRIGHT, TEXT};

#[cfg(target_os = "macos")]
pub(crate) fn checkbox(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
    on_click: impl Fn(&ClickEvent, &mut gpui::Window, &mut App) + 'static,
) -> impl IntoElement {
    let label = label.into();
    div()
        .id(id)
        .w_full()
        .h(px(36.0))
        .flex()
        .items_center()
        .gap_2()
        .text_sm()
        .text_color(rgb(TEXT))
        .cursor_pointer()
        .on_click(on_click)
        .child(
            div()
                .w(px(18.0))
                .h(px(18.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .border_1()
                .border_color(rgb(if checked { ACCENT } else { SURFACE_BRIGHT }))
                .bg(rgb(if checked { ACCENT } else { SURFACE }))
                .text_sm()
                .text_color(rgb(BACKGROUND))
                .child(if checked { "✓" } else { "" }),
        )
        .child(label)
}

pub(crate) fn option_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    on_click: impl Fn(&mut App) + 'static,
) -> impl IntoElement {
    let label = label.into();
    div()
        .id(id)
        .mx(px(2.0))
        .px_3()
        .py_2()
        .rounded_sm()
        .text_sm()
        .flex_shrink_0()
        .bg(rgb(if selected { ACCENT } else { SURFACE_BRIGHT }))
        .text_color(rgb(if selected { BACKGROUND } else { TEXT }))
        .hover(|style| style.bg(rgb(if selected { 0xa6c8ff } else { 0x45475a })))
        .on_click(move |_, _, cx| on_click(cx))
        .child(label)
}

pub(crate) fn titlebar_button(
    label: &'static str,
    foreground: u32,
    on_click: impl Fn(&mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(label)
        .h(px(24.0))
        .flex_shrink_0()
        .px_2()
        .flex()
        .items_center()
        .rounded_sm()
        .text_sm()
        .text_color(rgb(foreground))
        .hover(|style| style.bg(rgb(SURFACE_BRIGHT)).text_color(rgb(foreground)))
        .on_click(move |_, _, cx| on_click(cx))
        .child(label)
}
