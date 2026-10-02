use gpui::{div, prelude::*, rgb, AlignItems, IntoElement};

use super::{ACCENT, MUTED_TEXT};

pub(crate) fn section(title: &'static str, content: impl IntoElement) -> impl IntoElement {
    div()
        .w_full()
        .mt_4()
        .child(div().text_sm().text_color(rgb(ACCENT)).child(title))
        .child(div().w_full().mt_2().pl_3().child(content))
}

pub(crate) fn row(
    label: &'static str,
    description: &'static str,
    controls: impl IntoElement,
) -> impl IntoElement {
    let mut controls_wrapper = div().min_w_0().mt_2().flex().flex_row();
    controls_wrapper.style().align_self = Some(AlignItems::Stretch);

    div()
        .w_full()
        .flex()
        .flex_col()
        .px_3()
        .py_3()
        .child(
            div().text_base().child(label).child(
                div()
                    .mt_1()
                    .text_sm()
                    .text_color(rgb(MUTED_TEXT))
                    .child(description),
            ),
        )
        .child(controls_wrapper.child(controls))
}
