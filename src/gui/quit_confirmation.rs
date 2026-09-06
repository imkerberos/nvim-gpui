use super::dialog_overlay;
use crate::{
    app::{NvimGpui, QuitDialogState},
    widgets::{ACCENT, BACKGROUND, MUTED_TEXT, SURFACE, SURFACE_BRIGHT, TEXT, WARNING},
};
use gpui::{div, prelude::*, px, rgb, Context};

/// Render the application-level quit confirmation overlay.
///
/// The quit state and its actions belong to `app`; this module only owns the
/// presentation and wires the buttons to the application callbacks.
pub(crate) fn quit_confirmation_dialog(
    state: &QuitDialogState,
    cx: &mut Context<NvimGpui>,
) -> Option<gpui::Div> {
    let state = match state {
        QuitDialogState::Hidden | QuitDialogState::Quitting => return None,
        state => state,
    };

    let mut body = div().w_full().flex().flex_col().gap_2();
    match state {
        QuitDialogState::Checking => {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED_TEXT))
                    .child("Checking for unsaved changes…"),
            );
        }
        QuitDialogState::Saving => {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED_TEXT))
                    .child("Saving files…"),
            );
        }
        QuitDialogState::Confirm {
            modified_buffers,
            error,
        } => {
            if let Some(error) = error {
                body = body.child(
                    div()
                        .text_sm()
                        .text_color(rgb(WARNING))
                        .whitespace_normal()
                        .child(error.clone()),
                );
            }
            if modified_buffers.is_empty() {
                body = body.child(
                    div()
                        .text_sm()
                        .text_color(rgb(MUTED_TEXT))
                        .child("Neovim may still have unsaved changes."),
                );
            } else {
                body = body.child(div().text_sm().text_color(rgb(MUTED_TEXT)).child(format!(
                    "The following {} {} unsaved changes:",
                    if modified_buffers.len() == 1 {
                        "file"
                    } else {
                        "files"
                    },
                    if modified_buffers.len() == 1 {
                        "has"
                    } else {
                        "have"
                    }
                )));
                let mut files = div()
                    .id("quit-dialog-files")
                    .w_full()
                    .max_h(px(180.0))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .rounded_sm()
                    .bg(rgb(BACKGROUND));
                for name in modified_buffers {
                    files = files.child(
                        div()
                            .w_full()
                            .text_sm()
                            .whitespace_normal()
                            .child(name.clone()),
                    );
                }
                body = body.child(files);
            }

            let cancel = div()
                .id("quit-dialog-cancel")
                .px_3()
                .py_2()
                .rounded_sm()
                .text_sm()
                .text_color(rgb(TEXT))
                .cursor_pointer()
                .hover(|style| style.bg(rgb(SURFACE_BRIGHT)))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.cancel_quit_dialog(cx);
                }))
                .child("Cancel");
            let discard = div()
                .id("quit-dialog-discard")
                .px_3()
                .py_2()
                .rounded_sm()
                .text_sm()
                .bg(rgb(0xf38ba8))
                .text_color(rgb(BACKGROUND))
                .cursor_pointer()
                .hover(|style| style.bg(rgb(0xf5bde6)))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.discard_and_quit(cx);
                }))
                .child("Discard & Quit");
            let save = div()
                .id("quit-dialog-save")
                .px_3()
                .py_2()
                .rounded_sm()
                .text_sm()
                .bg(rgb(ACCENT))
                .text_color(rgb(BACKGROUND))
                .cursor_pointer()
                .hover(|style| style.bg(rgb(0xa6c8ff)))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.save_and_quit(cx);
                }))
                .child("Save All & Quit");
            body = body.child(
                div()
                    .w_full()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .mt_2()
                    .child(cancel)
                    .child(discard)
                    .child(save),
            );
        }
        QuitDialogState::Hidden | QuitDialogState::Quitting => unreachable!(),
    }

    let title = match state {
        QuitDialogState::Checking => "Preparing to quit",
        QuitDialogState::Saving => "Saving changes",
        QuitDialogState::Confirm { .. } => "Unsaved changes",
        QuitDialogState::Hidden | QuitDialogState::Quitting => unreachable!(),
    };
    let panel = div()
        .id("quit-confirmation-dialog")
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
                .child(title),
        )
        .child(body);

    Some(dialog_overlay("quit-confirmation-backdrop", panel))
}
