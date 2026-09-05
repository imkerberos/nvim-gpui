use crate::{
    app::{themed_titlebar, themed_titlebar_enabled},
    widgets::{logo_image, ACCENT, BACKGROUND, MUTED_TEXT, TEXT},
};
use gpui::{div, img, prelude::*, px, rgb, Context, Render, Window};

const REPOSITORY_URL: &str = env!("CARGO_PKG_REPOSITORY");

pub(crate) struct AboutWindow;

impl Render for AboutWindow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(TEXT));
        if themed_titlebar_enabled() {
            root = div()
                .size_full()
                .flex()
                .flex_col()
                .bg(rgb(BACKGROUND))
                .child(themed_titlebar(
                    "About nvim-gpui".to_owned(),
                    BACKGROUND,
                    TEXT,
                    None,
                    None,
                ))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .text_color(rgb(TEXT))
                        .child(img(logo_image()).w(px(96.0)).h(px(96.0)))
                        .child(div().text_lg().child("nvim-gpui"))
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(MUTED_TEXT))
                                .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
                        )
                        .child(
                            div()
                                .mt_2()
                                .text_sm()
                                .text_color(rgb(MUTED_TEXT))
                                .child("A GPUI graphical frontend for Neovim."),
                        )
                        .child(repository_link()),
                );
        } else {
            root = root
                .child(img(logo_image()).w(px(96.0)).h(px(96.0)))
                .child(div().text_lg().child("nvim-gpui"))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(MUTED_TEXT))
                        .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
                )
                .child(
                    div()
                        .mt_2()
                        .text_sm()
                        .text_color(rgb(MUTED_TEXT))
                        .child("A GPUI graphical frontend for Neovim."),
                )
                .child(repository_link());
        }
        root
    }
}

fn repository_link() -> impl IntoElement {
    div()
        .id("about-repository")
        .mt_1()
        .text_sm()
        .text_color(rgb(ACCENT))
        .cursor_pointer()
        .hover(|style| style.text_color(rgb(0xb4befe)))
        .on_click(|_, _, cx| cx.open_url(REPOSITORY_URL))
        .child(REPOSITORY_URL)
}
