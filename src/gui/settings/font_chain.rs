use super::*;
use crate::editor::{format_guifont_families, parse_guifont_families};
use crate::widgets::{
    token_edit, TextInputState, TokenEditCandidate, TokenEditConfig, TokenEditEvent,
};

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

fn fuzzy_font_score(query: &str, family: &str) -> Option<i32> {
    let query: Vec<char> = query
        .chars()
        .flat_map(|character| character.to_lowercase())
        .collect();
    if query.is_empty() {
        return Some(0);
    }
    let family: Vec<char> = family
        .chars()
        .flat_map(|character| character.to_lowercase())
        .collect();

    let mut score = 0;
    let mut next_index = 0;
    let mut previous_index = None;
    for (query_index, query_character) in query.into_iter().enumerate() {
        let relative_index = family[next_index..]
            .iter()
            .position(|character| *character == query_character)?;
        let index = next_index + relative_index;
        let boundary = index == 0 || !family[index - 1].is_alphanumeric();
        if query_index == 0 {
            score += if index == 0 {
                120
            } else if boundary {
                80
            } else {
                0
            };
        } else if previous_index == Some(index - 1) {
            score += 60;
        } else {
            score += if boundary { 24 } else { 0 };
            score -= relative_index as i32 * 5;
        }
        score -= index as i32;
        previous_index = Some(index);
        next_index = index + 1;
    }

    score -= family.len().saturating_sub(next_index) as i32;
    if next_index == family.len() {
        score += 300;
    }
    Some(score)
}

fn filtered_font_families(query: &str, candidates: &[String]) -> Vec<String> {
    if query.is_empty() {
        return candidates.to_vec();
    }

    let mut matches = candidates
        .iter()
        .enumerate()
        .filter_map(|(index, family)| {
            fuzzy_font_score(query, family).map(|score| (index, score, family.clone()))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    matches.into_iter().map(|(_, _, family)| family).collect()
}

fn insert_font_family(
    mut tokens: Vec<String>,
    cursor: usize,
    family: String,
) -> (Vec<String>, usize) {
    let mut cursor = cursor.min(tokens.len());
    if let Some(existing) = tokens
        .iter()
        .position(|token| token.eq_ignore_ascii_case(&family))
    {
        tokens.remove(existing);
        cursor -= usize::from(existing < cursor);
    }
    tokens.insert(cursor, family);
    (tokens, cursor + 1)
}

fn next_candidate_index(current: usize, count: usize, down: bool) -> usize {
    if count == 0 {
        return 0;
    }
    if down {
        (current + 1) % count
    } else {
        (current + count - 1) % count
    }
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
        edit.state.input = TextInputState::new(String::new());
        edit.state.highlighted_candidate = 0;
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
                input: TextInputState::new(String::new()),
                highlighted_candidate: 0,
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
        let token_count = font_chain_tokens(&value).len();
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
            edit.state.cursor = cursor.min(token_count);
        }
        cx.notify();
    }

    fn font_families(&self, field: FontChainField) -> Option<&[String]> {
        match field {
            FontChainField::GuiFont => self.monospace_families.as_deref(),
            FontChainField::GuiFontWide => self.unicode_families.as_deref(),
        }
    }

    fn commit_font_candidate(
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
        let settings = self.source.read(cx).app.settings_value();
        let tokens = font_chain_tokens(field.value(&settings));
        let (tokens, cursor) = insert_font_family(tokens, edit.state.cursor, family);
        self.set_font_chain(field, tokens, cursor, cx);
        if let Some(edit) = self
            .font_chain_editing
            .as_mut()
            .filter(|edit| edit.field == field)
        {
            edit.state.input = TextInputState::new(String::new());
            edit.state.highlighted_candidate = 0;
            edit.state.list_open = false;
        }
        cx.notify();
    }

    fn accept_font_candidate(&mut self, field: FontChainField, cx: &mut Context<Self>) -> bool {
        let Some(edit) = self
            .font_chain_editing
            .as_ref()
            .filter(|edit| edit.field == field)
        else {
            return false;
        };
        if edit.state.input.value.is_empty() || !edit.state.list_open {
            return false;
        }
        let query = edit.state.input.value.clone();
        let highlighted = edit.state.highlighted_candidate;
        let candidates = self.font_families(field).map(<[String]>::to_vec);
        let Some(family) = candidates
            .as_deref()
            .map(|candidates| filtered_font_families(&query, candidates))
            .and_then(|candidates| candidates.into_iter().nth(highlighted))
        else {
            return false;
        };

        self.commit_font_candidate(field, family, cx);
        true
    }

    fn reset_font_candidate_highlight(&mut self, field: FontChainField) {
        if let Some(edit) = self
            .font_chain_editing
            .as_mut()
            .filter(|edit| edit.field == field)
        {
            edit.state.highlighted_candidate = 0;
            edit.state.list_open = true;
            self.font_chain_scroll_handles[field.index()].scroll_to_item(0);
        }
    }

    fn move_font_candidate_highlight(
        &mut self,
        field: FontChainField,
        down: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self
            .font_chain_editing
            .as_ref()
            .filter(|edit| edit.field == field)
        else {
            return;
        };
        if edit.state.input.value.is_empty() || !edit.state.list_open {
            return;
        }
        let count = self
            .font_families(field)
            .map(|families| filtered_font_families(&edit.state.input.value, families).len())
            .unwrap_or(0);
        if count == 0 {
            return;
        }
        let next = next_candidate_index(edit.state.highlighted_candidate, count, down);
        if let Some(edit) = self
            .font_chain_editing
            .as_mut()
            .filter(|edit| edit.field == field)
        {
            edit.state.highlighted_candidate = next;
        }
        self.font_chain_scroll_handles[field.index()].scroll_to_item(next);
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
        let input_empty = edit.state.input.value.is_empty();
        let settings = self.source.read(cx).app.settings_value();
        let mut tokens = font_chain_tokens(field.value(&settings));
        match event.keystroke.key.as_str() {
            "escape" => {
                if !self.close_font_chain_list() {
                    self.font_chain_editing = None;
                }
                cx.notify();
            }
            "enter" | "return" => {
                if input_empty {
                    self.font_chain_editing = None;
                    cx.notify();
                } else if !self.accept_font_candidate(field, cx) {
                    cx.notify();
                }
            }
            "up" => self.move_font_candidate_highlight(field, false, cx),
            "down" => self.move_font_candidate_highlight(field, true, cx),
            "left" => {
                if let Some(edit) = self
                    .font_chain_editing
                    .as_mut()
                    .filter(|edit| edit.field == field)
                {
                    if edit.state.input.has_selection() || edit.state.input.cursor > 0 {
                        edit.state.input.move_left(false);
                    } else {
                        edit.state.cursor = cursor.saturating_sub(1);
                        edit.state.input = TextInputState::new(String::new());
                        edit.state.highlighted_candidate = 0;
                    }
                }
                cx.notify();
            }
            "right" => {
                if let Some(edit) = self
                    .font_chain_editing
                    .as_mut()
                    .filter(|edit| edit.field == field)
                {
                    if edit.state.input.has_selection()
                        || edit.state.input.cursor < edit.state.input.value.len()
                    {
                        edit.state.input.move_right(false);
                    } else {
                        edit.state.cursor = (cursor + 1).min(tokens.len());
                        edit.state.input = TextInputState::new(String::new());
                        edit.state.highlighted_candidate = 0;
                    }
                }
                cx.notify();
            }
            "home" => {
                if let Some(edit) = self.font_chain_editing.as_mut() {
                    if edit.state.input.value.is_empty() {
                        edit.state.cursor = 0;
                    } else {
                        edit.state.input.move_home(false);
                    }
                }
                cx.notify();
            }
            "end" => {
                if let Some(edit) = self.font_chain_editing.as_mut() {
                    if edit.state.input.value.is_empty() {
                        edit.state.cursor = tokens.len();
                    } else {
                        edit.state.input.move_end(false);
                    }
                }
                cx.notify();
            }
            "backspace" => {
                let has_input = self
                    .font_chain_editing
                    .as_ref()
                    .is_some_and(|edit| !edit.state.input.value.is_empty());
                if has_input {
                    if let Some(edit) = self.font_chain_editing.as_mut() {
                        edit.state.input.backspace();
                    }
                    self.reset_font_candidate_highlight(field);
                    cx.notify();
                } else if cursor > 0 && cursor <= tokens.len() {
                    tokens.remove(cursor - 1);
                    self.set_font_chain(field, tokens, cursor - 1, cx);
                }
            }
            "delete" => {
                let has_input = self
                    .font_chain_editing
                    .as_ref()
                    .is_some_and(|edit| !edit.state.input.value.is_empty());
                let input_at_end = self
                    .font_chain_editing
                    .as_ref()
                    .is_some_and(|edit| edit.state.input.cursor >= edit.state.input.value.len());
                let input_has_selection = self
                    .font_chain_editing
                    .as_ref()
                    .is_some_and(|edit| edit.state.input.has_selection());
                if has_input && (input_has_selection || !input_at_end) {
                    if let Some(edit) = self.font_chain_editing.as_mut() {
                        edit.state.input.delete();
                    }
                    self.reset_font_candidate_highlight(field);
                    cx.notify();
                } else if !has_input && cursor < tokens.len() {
                    tokens.remove(cursor);
                    self.set_font_chain(field, tokens, cursor, cx);
                }
            }
            _ if !event.keystroke.modifiers.control
                && !event.keystroke.modifiers.alt
                && !event.keystroke.modifiers.platform
                && !event.keystroke.modifiers.function =>
            {
                if let Some(key_char) = event.keystroke.key_char.as_ref() {
                    if let Some(edit) = self
                        .font_chain_editing
                        .as_mut()
                        .filter(|edit| edit.field == field)
                    {
                        edit.state.input.insert_text(key_char);
                    }
                    self.reset_font_candidate_highlight(field);
                    cx.notify();
                }
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
        let has_input = !edit.state.input.value.is_empty();
        if has_input {
            self.commit_font_candidate(field, family, cx);
            return;
        }
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

    fn handle_font_chain_input_mouse(
        &mut self,
        field: FontChainField,
        event: TextInputMouseEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self
            .font_chain_editing
            .as_mut()
            .filter(|edit| edit.field == field)
        else {
            return;
        };
        match event {
            TextInputMouseEvent::Down { index, shift } => {
                edit.state.input.begin_mouse_selection(index, shift);
                self.font_chain_focus_handles[field.index()].focus(window);
            }
            TextInputMouseEvent::Drag { index } => edit.state.input.extend_mouse_selection(index),
            TextInputMouseEvent::Up => edit.state.input.end_mouse_selection(),
        }
        cx.notify();
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
            edit.state.input = TextInputState::new(String::new());
            edit.state.highlighted_candidate = 0;
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
        let state = editing.map(|edit| edit.state.clone());
        let focus_handle = self.font_chain_focus_handles[field.index()].clone();
        let query = editing
            .map(|edit| edit.state.input.value.as_str())
            .unwrap_or_default();
        let candidate_families = candidates
            .map(|candidates| filtered_font_families(query, candidates))
            .unwrap_or_default();
        let token_candidates = candidates.map(|_| {
            candidate_families
                .iter()
                .enumerate()
                .map(|(index, family)| TokenEditCandidate {
                    label: family.clone().into(),
                    selected: contains_font_family(&tokens, family),
                    highlighted: !query.is_empty()
                        && editing.is_some_and(|edit| edit.state.highlighted_candidate == index),
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
                TokenEditEvent::Input(mouse_event) => {
                    this.handle_font_chain_input_mouse(field, mouse_event, window, cx);
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
            .scroll_handle(self.font_chain_scroll_handles[field.index()].clone())
            .empty_candidates_label("No additional fonts available")
            .loading_label("Scanning fonts…")
            .on_event(on_event)
            .on_key_down(on_key_down),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{filtered_font_families, insert_font_family, next_candidate_index};

    fn families(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn fuzzy_font_filter_matches_subsequences_and_ranks_the_best_match_first() {
        let candidates = families(&["JetBrains Mono", "Fira Code", "FiraCode", "Cascadia Code"]);

        assert_eq!(
            filtered_font_families("fc", &candidates),
            families(&["Fira Code", "FiraCode"])
        );
        assert_eq!(
            filtered_font_families("jbm", &candidates),
            families(&["JetBrains Mono"])
        );
    }

    #[test]
    fn fuzzy_font_filter_is_case_insensitive_and_keeps_all_fonts_without_a_query() {
        let candidates = families(&["Fira Code", "JetBrains Mono"]);

        assert_eq!(
            filtered_font_families("FIC", &candidates),
            families(&["Fira Code"])
        );
        assert_eq!(filtered_font_families("", &candidates), candidates);
        assert!(filtered_font_families("zzz", &candidates).is_empty());
    }

    #[test]
    fn accepting_an_existing_font_moves_it_without_duplicating_or_losing_the_cursor() {
        let (tokens, cursor) = insert_font_family(
            families(&["Fira Code", "JetBrains Mono", "Cascadia Code"]),
            3,
            "fira code".to_owned(),
        );
        assert_eq!(
            tokens,
            families(&["JetBrains Mono", "Cascadia Code", "fira code"])
        );
        assert_eq!(cursor, tokens.len());

        let (tokens, cursor) = insert_font_family(tokens, 0, "Cascadia Code".to_owned());
        assert_eq!(
            tokens,
            families(&["Cascadia Code", "JetBrains Mono", "fira code"])
        );
        assert_eq!(cursor, 1);
    }

    #[test]
    fn candidate_navigation_wraps_in_both_directions() {
        assert_eq!(next_candidate_index(0, 3, true), 1);
        assert_eq!(next_candidate_index(2, 3, true), 0);
        assert_eq!(next_candidate_index(0, 3, false), 2);
        assert_eq!(next_candidate_index(1, 3, false), 0);
        assert_eq!(next_candidate_index(0, 0, true), 0);
    }
}
