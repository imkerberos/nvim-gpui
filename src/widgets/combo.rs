use crate::app::{MD_ICON_ARROW_DROP_DOWN_ASSET, MD_ICON_ARROW_DROP_UP_ASSET};
use gpui::{
    deferred, div, prelude::*, px, rgb, svg, App, ClickEvent, ElementId, IntoElement, SharedString,
    Window,
};

use super::{SURFACE, SURFACE_BRIGHT, TEXT};

pub(crate) fn combo_box(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    open: bool,
    options: impl IntoElement,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let id = id.into();
    let label = label.into();
    let mut combo = div().relative().w_full().flex().flex_col();
    combo = combo.child(
        div()
            .id(id.clone())
            .w_full()
            .h(px(36.0))
            .flex()
            .items_center()
            .px_3()
            .rounded_sm()
            .border_1()
            .border_color(rgb(if open { super::ACCENT } else { SURFACE_BRIGHT }))
            .bg(rgb(SURFACE))
            .text_sm()
            .text_color(rgb(TEXT))
            .cursor_pointer()
            .hover(|style| style.border_color(rgb(super::ACCENT)))
            .on_click(on_click)
            .child(div().flex_1().child(label))
            .child(
                svg()
                    .path(if open {
                        MD_ICON_ARROW_DROP_UP_ASSET
                    } else {
                        MD_ICON_ARROW_DROP_DOWN_ASSET
                    })
                    .w(px(18.0))
                    .h(px(18.0))
                    .text_color(rgb(super::MUTED_TEXT)),
            ),
    );

    if open {
        combo = combo.child(
            deferred(
                div()
                    .absolute()
                    .left(px(0.0))
                    .top(px(40.0))
                    .w_full()
                    .p_1()
                    .rounded_sm()
                    .border_1()
                    .border_color(rgb(SURFACE_BRIGHT))
                    .bg(rgb(SURFACE))
                    .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .id(ElementId::NamedChild(Box::new(id), "options".into()))
                            .w_full()
                            .max_h(px(320.0))
                            .overflow_y_scroll()
                            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                            .child(options),
                    ),
            )
            .with_priority(1),
        );
    }

    combo
}

pub(crate) fn combo_option(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label = label.into();
    div()
        .id(id)
        .w_full()
        .flex()
        .items_center()
        .px_3()
        .py_2()
        .rounded_sm()
        .text_sm()
        .text_color(rgb(TEXT))
        .bg(rgb(if selected { SURFACE_BRIGHT } else { SURFACE }))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(SURFACE_BRIGHT)))
        .on_click(on_click)
        .child(div().flex_1().child(label))
        .child(
            div()
                .w(px(20.0))
                .text_right()
                .text_color(rgb(super::ACCENT))
                .child(if selected { "✓" } else { "" }),
        )
}
