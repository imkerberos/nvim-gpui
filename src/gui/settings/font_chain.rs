use super::*;
use crate::editor::{format_guifont_families, parse_guifont_families};
use crate::widgets::{token_edit, TokenEditCandidate, TokenEditConfig, TokenEditEvent};

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
    state: TokenEditState,
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
        if !edit.state.list_open {
            return false;
        }
        edit.state.list_open = false;
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
            state: TokenEditState {
                cursor: cursor.min(token_count),
                list_open: true,
            },
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
            edit.state.cursor = cursor.min(tokens.len());
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
        let cursor = edit.state.cursor;
        let settings = self.source.read(cx).app.settings_value();
        let mut tokens = font_chain_tokens(field.value(&settings));
        match event.keystroke.key.as_str() {
            "escape" => {
                if let Some(edit) = self
                    .font_chain_editing
                    .as_mut()
                    .filter(|edit| edit.field == field)
                {
                    if edit.state.list_open {
                        edit.state.list_open = false;
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
                    edit.state.cursor = cursor - 1;
                }
                cx.notify();
            }
            "right" if cursor < tokens.len() => {
                if let Some(edit) = self.font_chain_editing.as_mut() {
                    edit.state.cursor = cursor + 1;
                }
                cx.notify();
            }
            "home" => {
                if let Some(edit) = self.font_chain_editing.as_mut() {
                    edit.state.cursor = 0;
                }
                cx.notify();
            }
            "end" => {
                if let Some(edit) = self.font_chain_editing.as_mut() {
                    edit.state.cursor = tokens.len();
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
        let cursor = edit.state.cursor;
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

    fn handle_font_chain_click(
        &mut self,
        field: FontChainField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let token_count = {
            let settings = self.source.read(cx).app.settings_value();
            font_chain_tokens(field.value(&settings)).len()
        };
        if let Some(edit) = self
            .font_chain_editing
            .as_mut()
            .filter(|edit| edit.field == field)
        {
            edit.state.cursor = token_count;
            edit.state.list_open = !edit.state.list_open;
            cx.notify();
        } else {
            self.begin_font_chain_edit(field, token_count, window, cx);
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
        let state = editing.map(|edit| edit.state);
        let focus_handle = self.font_chain_focus_handles[field.index()].clone();
        let candidate_families = candidates.map(<[String]>::to_vec).unwrap_or_default();
        let token_candidates = candidates.map(|candidates| {
            candidates
                .iter()
                .map(|family| TokenEditCandidate {
                    label: family.clone().into(),
                    selected: contains_font_family(&tokens, family),
                })
                .collect()
        });
        let placeholder = default_family
            .map(|family| format!("System default ({family})"))
            .unwrap_or_else(|| "System default".to_owned());
        let on_event = cx.listener(
            move |this, event: &TokenEditEvent, window, cx| match *event {
                TokenEditEvent::Surface => this.handle_font_chain_click(field, window, cx),
                TokenEditEvent::Token(index) => {
                    this.begin_font_chain_edit(field, index + 1, window, cx);
                }
                TokenEditEvent::Candidate(index) => {
                    if let Some(family) = candidate_families.get(index).cloned() {
                        this.toggle_font_candidate(field, family, cx);
                    }
                }
            },
        );
        let on_key_down = cx.listener(move |this, event, window, cx| {
            this.handle_font_chain_key(field, event, window, cx);
        });

        token_edit(
            TokenEditConfig::new(
                ("settings-font-chain", field.index() as u32),
                focus_handle,
                tokens.into_iter().map(Into::into).collect(),
                placeholder,
                state,
            )
            .candidates(token_candidates)
            .empty_candidates_label("No additional fonts available")
            .loading_label("Scanning fonts…")
            .on_event(on_event)
            .on_key_down(on_key_down),
        )
    }
}
