use super::*;
use crate::editor::{format_guifont_families, parse_guifont_families};
use gpui::{deferred, div, px, rgb};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FontChainField {
    GuiFont,
    GuiFontWide,
}

impl FontChainField {
    fn index(self) -> usize {
        match self {
            Self::GuiFont => 0,
            Self::GuiFontWide => 1,
        }
    }

    fn value(self, settings: &settings::Settings) -> &str {
        match self {
            Self::GuiFont => &settings.guifont,
            Self::GuiFontWide => &settings.guifontwide,
        }
    }

    fn set(self, settings: &mut settings::Settings, value: String) {
        match self {
            Self::GuiFont => settings.guifont = value,
            Self::GuiFontWide => settings.guifontwide = value,
        }
    }
}

pub(super) struct FontChainEdit {
    field: FontChainField,
    cursor: usize,
    list_open: bool,
}

fn font_chain_tokens(value: &str) -> Vec<String> {
    parse_guifont_families(value)
}

fn contains_font_family(families: &[String], family: &str) -> bool {
    families
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(family))
}

impl SettingsWindow {
    pub(super) fn close_font_chain_list(&mut self) -> bool {
        let Some(edit) = self.font_chain_editing.as_mut() else {
            return false;
        };
        if !edit.list_open {
            return false;
        }
        edit.list_open = false;
        true
    }

    fn begin_font_chain_edit(
        &mut self,
        field: FontChainField,
        cursor: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_rime_path_edit(cx);
        let settings = self.source.read(cx).app.settings_value();
        let token_count = font_chain_tokens(field.value(&settings)).len();
        self.font_chain_editing = Some(FontChainEdit {
            field,
            cursor: cursor.min(token_count),
            list_open: true,
        });
        self.open_combo = None;
        self.start_font_scan(window, cx);
        self.font_chain_focus_handles[field.index()].focus(window);
        cx.notify();
    }

    fn start_font_scan(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.monospace_families.is_some() && self.unicode_families.is_some()
            || self.font_scan_task.is_some()
        {
            return;
        }

        let text_system = window.text_system().clone();
        let scan = cx.background_spawn(async move { system_font_families(&text_system) });
        self.font_scan_task = Some(cx.spawn(async move |this, cx| {
            let (monospace_families, unicode_families) = scan.await;
            let _ = this.update(cx, |this, cx| {
                this.monospace_families = Some(monospace_families);
                this.unicode_families = Some(unicode_families);
                this.font_scan_task = None;
                cx.notify();
            });
        }));
    }

    fn set_font_chain(
        &mut self,
        field: FontChainField,
        tokens: Vec<String>,
        cursor: usize,
        cx: &mut Context<Self>,
    ) {
        let value = format_guifont_families(&tokens);
        self.source.update(cx, |view, cx| {
            let mut next = view.app.settings_value();
            field.set(&mut next, value);
            view.update_settings(next);
            cx.notify();
        });
        if let Some(edit) = self
            .font_chain_editing
            .as_mut()
            .filter(|edit| edit.field == field)
        {
            edit.cursor = cursor.min(tokens.len());
        }
        cx.notify();
    }

    fn handle_font_chain_key(
        &mut self,
        field: FontChainField,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self
            .font_chain_editing
            .as_ref()
            .filter(|edit| edit.field == field)
        else {
            return;
        };

        window.prevent_default();
        cx.stop_propagation();
        let cursor = edit.cursor;
        let settings = self.source.read(cx).app.settings_value();
        let mut tokens = font_chain_tokens(field.value(&settings));
        match event.keystroke.key.as_str() {
            "escape" => {
                if let Some(edit) = self
                    .font_chain_editing
                    .as_mut()
                    .filter(|edit| edit.field == field)
                {
                    if edit.list_open {
                        edit.list_open = false;
                    } else {
                        self.font_chain_editing = None;
                    }
                }
                cx.notify();
            }
            "enter" | "return" => {
                self.font_chain_editing = None;
                cx.notify();
            }
            "left" if cursor > 0 => {
                if let Some(edit) = self.font_chain_editing.as_mut() {
                    edit.cursor = cursor - 1;
                }
                cx.notify();
            }
            "right" if cursor < tokens.len() => {
                if let Some(edit) = self.font_chain_editing.as_mut() {
                    edit.cursor = cursor + 1;
                }
                cx.notify();
            }
            "home" => {
                if let Some(edit) = self.font_chain_editing.as_mut() {
                    edit.cursor = 0;
                }
                cx.notify();
            }
            "end" => {
                if let Some(edit) = self.font_chain_editing.as_mut() {
                    edit.cursor = tokens.len();
                }
                cx.notify();
            }
            "backspace" if cursor > 0 && cursor <= tokens.len() => {
                tokens.remove(cursor - 1);
                self.set_font_chain(field, tokens, cursor - 1, cx);
            }
            "delete" if cursor < tokens.len() => {
                tokens.remove(cursor);
                self.set_font_chain(field, tokens, cursor, cx);
            }
            _ => {}
        }
    }

    fn toggle_font_candidate(
        &mut self,
        field: FontChainField,
        family: String,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self
            .font_chain_editing
            .as_ref()
            .filter(|edit| edit.field == field)
        else {
            return;
        };
        let cursor = edit.cursor;
        let settings = self.source.read(cx).app.settings_value();
        let mut tokens = font_chain_tokens(field.value(&settings));
        if let Some(index) = tokens
            .iter()
            .position(|token| token.eq_ignore_ascii_case(&family))
        {
            tokens.remove(index);
            let cursor = cursor.saturating_sub(usize::from(index < cursor));
            self.set_font_chain(field, tokens, cursor, cx);
        } else {
            tokens.insert(cursor.min(tokens.len()), family);
            self.set_font_chain(field, tokens, cursor + 1, cx);
        }
    }

    pub(super) fn font_chain_editor(
        &self,
        field: FontChainField,
        value: &str,
        default_family: Option<&str>,
        candidates: Option<&[String]>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tokens = font_chain_tokens(value);
        let editing = self
            .font_chain_editing
            .as_ref()
            .filter(|edit| edit.field == field);
        let cursor = editing.map(|edit| edit.cursor).unwrap_or(tokens.len());
        let list_open = editing.is_some_and(|edit| edit.list_open);
        let focus_handle = self.font_chain_focus_handles[field.index()].clone();
        let mut token_field = div()
            .id(("settings-font-chain", field.index() as u32))
            .w_full()
            .min_w_0()
            .h(px(36.0))
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .rounded_sm()
            .border_1()
            .border_color(rgb(if editing.is_some() {
                ACCENT
            } else {
                SURFACE_BRIGHT
            }))
            .bg(rgb(SURFACE))
            .track_focus(&focus_handle)
            .focus(|style| style.border_color(rgb(ACCENT)))
            .hover(|style| style.border_color(rgb(ACCENT)))
            .cursor_pointer()
            .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| {
                let token_count = {
                    let settings = this.source.read(cx).app.settings_value();
                    font_chain_tokens(field.value(&settings)).len()
                };
                if let Some(edit) = this
                    .font_chain_editing
                    .as_mut()
                    .filter(|edit| edit.field == field)
                {
                    edit.cursor = token_count;
                    edit.list_open = !edit.list_open;
                    cx.notify();
                } else {
                    this.begin_font_chain_edit(field, token_count, window, cx);
                }
            }))
            .capture_key_down(cx.listener(move |this, event, window, cx| {
                this.handle_font_chain_key(field, event, window, cx);
            }));

        for (index, family) in tokens.iter().cloned().enumerate() {
            if editing.is_some() && cursor == index {
                token_field =
                    token_field.child(div().w(px(2.0)).h(px(22.0)).rounded_sm().bg(rgb(ACCENT)));
            }
            token_field = token_field.child(
                div()
                    .id((
                        "settings-font-token",
                        field.index().saturating_mul(10_000).saturating_add(index),
                    ))
                    .max_w(px(240.0))
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(rgb(SURFACE_BRIGHT))
                    .text_sm()
                    .text_color(rgb(TEXT))
                    .cursor_pointer()
                    .child(family)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.begin_font_chain_edit(field, index + 1, window, cx);
                    })),
            );
        }
        if editing.is_some() && cursor == tokens.len() {
            token_field =
                token_field.child(div().w(px(2.0)).h(px(22.0)).rounded_sm().bg(rgb(ACCENT)));
        }
        if tokens.is_empty() {
            token_field = token_field.child(
                div().flex_1().text_sm().text_color(rgb(MUTED_TEXT)).child(
                    default_family
                        .map(|family| format!("System default ({family})"))
                        .unwrap_or_else(|| "System default".to_owned()),
                ),
            );
        }

        let mut editor = div()
            .relative()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(token_field);

        if list_open {
            let mut options = div().w_full().flex().flex_col();
            match candidates {
                Some(candidates) => {
                    for (index, family) in candidates.iter().cloned().enumerate() {
                        options = options.child(setting_combo_option(
                            (
                                "settings-font-candidate",
                                field.index().saturating_mul(10_000).saturating_add(index),
                            ),
                            family.clone(),
                            contains_font_family(&tokens, &family),
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_font_candidate(field, family.clone(), cx);
                            }),
                        ));
                    }
                    if candidates.is_empty() {
                        options = options.child(
                            div()
                                .px_3()
                                .py_2()
                                .text_sm()
                                .text_color(rgb(MUTED_TEXT))
                                .child("No additional fonts available"),
                        );
                    }
                }
                None => {
                    options = options.child(
                        div()
                            .px_3()
                            .py_2()
                            .text_sm()
                            .text_color(rgb(MUTED_TEXT))
                            .child("Scanning fonts…"),
                    );
                }
            }
            editor = editor.child(
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
                                .id(("settings-font-candidates", field.index()))
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

        editor
    }
}
