use super::dialog_overlay;
use crate::widgets::{ACCENT, BACKGROUND, SURFACE, SURFACE_BRIGHT, TEXT, WARNING};
use gpui::{div, prelude::*, px, rgb};

/// Render the application-level connection failure overlay.
pub(crate) fn startup_error_dialog(nvim_connected: bool, rpc_status: &str) -> Option<gpui::Div> {
    if nvim_connected {
        return None;
    }

    let error = rpc_status
        .strip_prefix("rpc: ")
        .unwrap_or(rpc_status)
        .to_owned();
    let ok = div()
        .id("startup-error-ok")
        .px_3()
        .py_2()
        .rounded_sm()
        .text_sm()
        .bg(rgb(ACCENT))
        .text_color(rgb(BACKGROUND))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0xa6c8ff)))
        .on_click(|_, _, cx| cx.quit())
        .child("OK");
    let panel = div()
        .id("startup-error-dialog")
        .w(px(520.0))
        .max_w(px(720.0))
        .p_4()
        .rounded_sm()
        .border_1()
        .border_color(rgb(SURFACE_BRIGHT))
        .bg(rgb(SURFACE))
        .text_color(rgb(TEXT))
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("Neovim connection failed"),
        )
        .child(
            div()
                .text_sm()
                .text_color(rgb(WARNING))
                .whitespace_normal()
                .child(error),
        )
        .child(div().w_full().flex().justify_end().mt_2().child(ok));

    Some(dialog_overlay("startup-error-backdrop", panel))
}
