use gpui::{div, prelude::*, px, rgb, App, ClickEvent, Font, Image, SharedString, Window};
use std::sync::Arc;

#[cfg(target_os = "windows")]
use gpui::WindowControlArea;

pub(crate) const BACKGROUND: u32 = 0x1e1e2e;
pub(crate) const SURFACE: u32 = 0x181825;
pub(crate) const SURFACE_BRIGHT: u32 = 0x313244;
pub(crate) const TEXT: u32 = 0xcdd6f4;
pub(crate) const MUTED_TEXT: u32 = 0x7f849c;
pub(crate) const ACCENT: u32 = 0x89b4fa;

pub(crate) fn setting_combo_box(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    open: bool,
    options: impl IntoElement,
    icon_font: Font,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label = label.into();
    let mut combo = div().w_full().flex().flex_col();
    combo = combo.child(
        div()
            .id(id)
            .w_full()
            .h(px(36.0))
            .flex()
            .items_center()
            .px_3()
            .rounded_sm()
            .border_1()
            .border_color(rgb(if open { ACCENT } else { SURFACE_BRIGHT }))
            .bg(rgb(SURFACE))
            .text_sm()
            .text_color(rgb(TEXT))
            .cursor_pointer()
            .hover(|style| style.border_color(rgb(ACCENT)))
            .on_click(on_click)
            .child(div().flex_1().child(label))
            .child(
                div()
                    .font(icon_font)
                    .text_color(rgb(MUTED_TEXT))
                    .child(if open { "" } else { "" }),
            ),
    );

    if open {
        combo = combo.child(
            div()
                .w_full()
                .mt_1()
                .p_1()
                .rounded_sm()
                .border_1()
                .border_color(rgb(SURFACE_BRIGHT))
                .bg(rgb(SURFACE))
                .child(options),
        );
    }

    combo
}

pub(crate) fn setting_combo_option(
    id: impl Into<gpui::ElementId>,
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
                .text_color(rgb(ACCENT))
                .child(if selected { "✓" } else { "" }),
        )
}

pub(crate) fn setting_option_button(
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
        .bg(rgb(if selected { ACCENT } else { SURFACE_BRIGHT }))
        .text_color(rgb(if selected { BACKGROUND } else { TEXT }))
        .hover(|style| style.bg(rgb(if selected { 0xa6c8ff } else { 0x45475a })))
        .on_click(move |_, _, cx| on_click(cx))
        .child(label)
}

pub(crate) fn logo_image() -> Arc<Image> {
    Arc::new(Image::from_bytes(
        gpui::ImageFormat::Png,
        include_bytes!("../assets/icons/neovim-gpui.png").to_vec(),
    ))
}

pub(crate) fn setting_section(title: &'static str, content: impl IntoElement) -> impl IntoElement {
    div()
        .w_full()
        .mt_4()
        .child(div().text_sm().text_color(rgb(ACCENT)).child(title))
        .child(div().w_full().mt_2().pl_3().child(content))
}

pub(crate) fn setting_row(
    label: &'static str,
    description: &'static str,
    controls: impl IntoElement,
) -> impl IntoElement {
    div()
        .w_full()
        .flex()
        .flex_col()
        .items_start()
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
        .child(div().w_full().mt_2().child(controls))
}

pub(crate) fn titlebar_button(
    label: &'static str,
    foreground: u32,
    on_click: impl Fn(&mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(label)
        .h(px(24.0))
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

#[cfg(target_os = "windows")]
pub(crate) fn window_control_button(
    label: &'static str,
    area: WindowControlArea,
    background: u32,
    foreground: u32,
) -> impl IntoElement {
    div()
        .w(px(46.0))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(background))
        .text_color(rgb(foreground))
        .window_control_area(area)
        .child(label)
}
