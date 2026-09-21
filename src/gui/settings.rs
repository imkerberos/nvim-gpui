use crate::app::NvimGpui;
#[cfg(target_os = "macos")]
use crate::widgets::setting_checkbox;
use crate::{
    app::{themed_titlebar, themed_titlebar_enabled, MD_ICON_CLOSE_ASSET},
    editor::{system_font_families, EditorRuntime},
    helper, settings, update_check,
    widgets::{
        setting_combo_box, setting_combo_option, setting_option_button, setting_row,
        setting_section, setting_text_input, SettingTextInputConfig, SettingTextInputMouseEvent,
        SettingTextInputState, ACCENT, BACKGROUND, MUTED_TEXT, SURFACE, SURFACE_BRIGHT, TEXT,
        WARNING,
    },
};
use gpui::{
    div, prelude::*, px, rgb, svg, Context, Entity, FocusHandle, FontWeight, KeyDownEvent,
    SharedString, Subscription, Task, Window,
};
use nvim_gpui::rime::RimeRuntimeResolver;
use std::env;

#[path = "settings/font_chain.rs"]
mod font_chain;
#[path = "settings/render.rs"]
mod render;
use font_chain::FontChainField;

pub(crate) struct SettingsWindow {
    source: Entity<NvimGpui>,
    _source_subscription: Subscription,
    paste_shortcut_focus_handle: FocusHandle,
    recording_paste_shortcut: bool,
    rime_toggle_shortcut_focus_handle: FocusHandle,
    recording_rime_toggle_shortcut: bool,
    rime_path_focus_handles: [FocusHandle; 3],
    _rime_path_blur_subscriptions: Vec<Subscription>,
    rime_path_editing: Option<RimePathEdit>,
    rime_test_status: Option<RimeTestStatus>,
    rime_user_data_error: Option<String>,
    log_directory_error: Option<String>,
    monospace_families: Option<Vec<String>>,
    unicode_families: Option<Vec<String>>,
    font_scan_task: Option<Task<()>>,
    font_chain_focus_handles: [FocusHandle; 2],
    font_chain_editing: Option<font_chain::FontChainEdit>,
    open_combo: Option<SettingsCombo>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsCombo {
    FontSize,
    NerdFont,
    FallbackMode,
    CursorAnimation,
    StartupMaximized,
    UpdateChecks,
    LogLevel,
    ImageCacheSize,
    ImeBackend,
    RimeCandidateLayout,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RimePathField {
    Library,
    Data,
    UserData,
}

impl RimePathField {
    fn index(self) -> usize {
        match self {
            Self::Library => 0,
            Self::Data => 1,
            Self::UserData => 2,
        }
    }
}

struct RimePathEdit {
    field: RimePathField,
    input: SettingTextInputState,
}

enum RimeTestStatus {
    Testing,
    Complete(Result<(), String>),
    Blocked(String),
}

fn is_path_paste_key(event: &KeyDownEvent) -> bool {
    if event.keystroke.key != "v" {
        return false;
    }

    #[cfg(target_os = "macos")]
    {
        event.keystroke.modifiers.platform && !event.keystroke.modifiers.control
    }
    #[cfg(not(target_os = "macos"))]
    {
        event.keystroke.modifiers.control && !event.keystroke.modifiers.platform
    }
}

impl SettingsWindow {
    pub(crate) fn new(source: Entity<NvimGpui>, cx: &mut Context<Self>) -> Self {
        let source_subscription = cx.observe(&source, |_, _, cx| cx.notify());
        Self {
            source,
            _source_subscription: source_subscription,
            paste_shortcut_focus_handle: cx.focus_handle().tab_stop(true),
            recording_paste_shortcut: false,
            rime_toggle_shortcut_focus_handle: cx.focus_handle().tab_stop(true),
            recording_rime_toggle_shortcut: false,
            rime_path_focus_handles: [
                cx.focus_handle().tab_stop(true),
                cx.focus_handle().tab_stop(true),
                cx.focus_handle().tab_stop(true),
            ],
            _rime_path_blur_subscriptions: Vec::new(),
            rime_path_editing: None,
            rime_test_status: None,
            rime_user_data_error: None,
            log_directory_error: None,
            monospace_families: None,
            unicode_families: None,
            font_scan_task: None,
            font_chain_focus_handles: [
                cx.focus_handle().tab_stop(true),
                cx.focus_handle().tab_stop(true),
            ],
            font_chain_editing: None,
            open_combo: None,
        }
    }

    fn toggle_combo(&mut self, combo: SettingsCombo, cx: &mut Context<Self>) {
        self.open_combo = (self.open_combo != Some(combo)).then_some(combo);
        self.recording_paste_shortcut = false;
        self.recording_rime_toggle_shortcut = false;
        self.commit_rime_path_edit(cx);
        self.font_chain_editing = None;
        cx.notify();
    }

    fn ensure_rime_path_blur_subscriptions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self._rime_path_blur_subscriptions.is_empty() {
            return;
        }

        for field in [
            RimePathField::Library,
            RimePathField::Data,
            RimePathField::UserData,
        ] {
            let focus_handle = self.rime_path_focus_handles[field.index()].clone();
            self._rime_path_blur_subscriptions.push(cx.on_blur(
                &focus_handle,
                window,
                move |this, _, cx| {
                    if matches!(
                        this.rime_path_editing.as_ref(),
                        Some(edit) if edit.field == field
                    ) {
                        this.commit_rime_path_edit(cx);
                    }
                },
            ));
        }
    }

    fn apply_setting(
        &mut self,
        update: impl FnOnce(&mut settings::Settings),
        cx: &mut Context<Self>,
    ) {
        self.commit_rime_path_edit(cx);
        self.source.update(cx, |view, cx| {
            let mut next = view.app.settings_value();
            update(&mut next);
            view.update_settings(next);
            cx.notify();
        });
        self.open_combo = None;
        self.font_chain_editing = None;
        cx.notify();
    }

    fn uses_bundled_rime_runtime() -> bool {
        cfg!(any(target_os = "macos", target_os = "windows"))
    }

    fn bundled_rime_path(field: RimePathField) -> Option<String> {
        let resolver = RimeRuntimeResolver::default();
        let path = match field {
            RimePathField::Library => resolver.resolve_library_directory(None).ok()?,
            RimePathField::Data => resolver.resolve_shared_data(None).ok()?,
            RimePathField::UserData => return None,
        };
        Some(path.display().to_string())
    }

    fn rime_path_value(settings: &settings::Settings, field: RimePathField) -> String {
        if field == RimePathField::UserData {
            return settings::rime_user_data_directory()
                .map(|path| path.display().to_string())
                .unwrap_or_default();
        }

        if Self::uses_bundled_rime_runtime() {
            return Self::bundled_rime_path(field).unwrap_or_default();
        }

        let configured = match field {
            RimePathField::Library => settings.rime_library_dir.clone(),
            RimePathField::Data => settings.rime_data_dir.clone(),
            RimePathField::UserData => unreachable!("user data path handled above"),
        };
        if !configured.is_empty() {
            return configured;
        }

        let environment = match field {
            RimePathField::Library => "NVIM_GPUI_RIME_LIBRARY",
            RimePathField::Data => "NVIM_GPUI_RIME_SHARED_DIR",
            RimePathField::UserData => unreachable!("user data path handled above"),
        };
        env::var_os(environment)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn set_rime_path_value(settings: &mut settings::Settings, field: RimePathField, value: String) {
        match field {
            RimePathField::Library => {
                settings.rime_library_dir = value;
                settings.rime_library_auto_detect = false;
            }
            RimePathField::Data => settings.rime_data_dir = value,
            RimePathField::UserData => {}
        }
    }

    fn begin_rime_path_edit(
        &mut self,
        field: RimePathField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_rime_path_edit_at(field, None, window, cx);
    }

    fn begin_rime_path_edit_at(
        &mut self,
        field: RimePathField,
        cursor: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_rime_path_edit(cx);
        let value = Self::rime_path_value(&self.source.read(cx).app.settings_value(), field);
        let mut input = SettingTextInputState::new(value);
        input.move_to(cursor.unwrap_or(input.value.len()), false);
        self.rime_path_editing = Some(RimePathEdit { field, input });
        self.rime_test_status = None;
        self.rime_path_focus_handles[field.index()].focus(window);
        cx.notify();
    }

    fn commit_rime_path_edit(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.rime_path_editing.take() else {
            return;
        };
        self.source.update(cx, |view, cx| {
            let mut next = view.app.settings_value();
            Self::set_rime_path_value(&mut next, edit.field, edit.input.value);
            view.update_settings(next);
            cx.notify();
        });
    }

    fn cancel_rime_path_edit(&mut self, cx: &mut Context<Self>) {
        self.rime_path_editing = None;
        cx.notify();
    }

    fn paste_rime_path(&mut self, field: RimePathField, cx: &mut Context<Self>) {
        let text = match crate::app::clipboard::paste_text(cx) {
            Ok(text) => text,
            Err(error) => {
                log::warn!(
                    target: "nvim_gpui::settings",
                    "could not read clipboard for Rime path: {error}"
                );
                return;
            }
        };
        let text: String = text
            .chars()
            .filter(|character| !matches!(character, '\r' | '\n'))
            .collect();
        let Some(edit) = self
            .rime_path_editing
            .as_mut()
            .filter(|edit| edit.field == field)
        else {
            return;
        };
        if text.is_empty() {
            return;
        }
        edit.input.insert_text(&text);
        cx.notify();
    }

    fn test_rime_configuration(&mut self, cx: &mut Context<Self>) {
        self.commit_rime_path_edit(cx);
        if matches!(self.rime_test_status, Some(RimeTestStatus::Testing)) {
            return;
        }
        let settings = self.source.read(cx).app.settings_value();
        if let Some(reason) = Self::rime_test_block_reason(&settings) {
            self.rime_test_status = Some(RimeTestStatus::Blocked(reason));
            cx.notify();
            return;
        }
        self.rime_test_status = Some(RimeTestStatus::Testing);
        cx.notify();

        let task = cx.background_spawn(async move {
            EditorRuntime::test_rime_configuration_with_settings(settings)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.rime_test_status = Some(RimeTestStatus::Complete(result));
                cx.notify();
            });
        })
        .detach();
    }

    fn rime_test_block_reason(settings: &settings::Settings) -> Option<String> {
        if Self::uses_bundled_rime_runtime() {
            let resolver = RimeRuntimeResolver::default();
            if resolver.resolve_library(None).is_err() {
                return Some("Bundled librime is unavailable.".to_owned());
            }
            if resolver.resolve_shared_data(None).is_err() {
                return Some("Bundled Rime data is unavailable.".to_owned());
            }
        }

        let data_configured = !Self::rime_path_value(settings, RimePathField::Data)
            .trim()
            .is_empty();
        let data_auto_detected = settings.rime_library_auto_detect
            && RimeRuntimeResolver::default()
                .resolve_shared_data(None)
                .is_ok();
        if !Self::uses_bundled_rime_runtime() && !data_configured && !data_auto_detected {
            return Some(
                "Rime data directory is required; set it here or provide NVIM_GPUI_RIME_SHARED_DIR."
                    .to_owned(),
            );
        }

        if !Self::uses_bundled_rime_runtime()
            && !settings.rime_library_auto_detect
            && Self::rime_path_value(settings, RimePathField::Library)
                .trim()
                .is_empty()
        {
            return Some(
                "librime directory is required, or enable automatic detection first.".to_owned(),
            );
        }

        if settings::rime_user_data_directory().is_none() {
            return Some("User data directory is not available.".to_owned());
        }

        None
    }

    fn open_log_directory(&mut self, cx: &mut Context<Self>) {
        self.log_directory_error = crate::logging::open_log_directory().err();
        cx.notify();
    }

    fn open_rime_user_data_directory(&mut self, cx: &mut Context<Self>) {
        let result = settings::rime_user_data_directory()
            .ok_or_else(|| "could not determine the Rime user data directory".to_owned())
            .and_then(|path| crate::logging::open_directory(&path));
        self.rime_user_data_error = result.err();
        cx.notify();
    }

    fn handle_rime_path_key(
        &mut self,
        field: RimePathField,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if is_path_paste_key(event) {
            window.prevent_default();
            cx.stop_propagation();
            self.paste_rime_path(field, cx);
            return;
        }

        let Some(edit) = self.rime_path_editing.as_mut() else {
            return;
        };
        if edit.field != field {
            return;
        }

        window.prevent_default();
        cx.stop_propagation();
        let extend_selection = event.keystroke.modifiers.shift;
        if event.keystroke.key == "a"
            && (event.keystroke.modifiers.platform || event.keystroke.modifiers.control)
        {
            edit.input.select_all();
            cx.notify();
            return;
        }
        match event.keystroke.key.as_str() {
            "escape" => self.cancel_rime_path_edit(cx),
            "enter" | "return" => self.commit_rime_path_edit(cx),
            "left" => {
                edit.input.move_left(extend_selection);
                cx.notify();
            }
            "right" => {
                edit.input.move_right(extend_selection);
                cx.notify();
            }
            "home" => {
                edit.input.move_home(extend_selection);
                cx.notify();
            }
            "end" => {
                edit.input.move_end(extend_selection);
                cx.notify();
            }
            "backspace" => {
                edit.input.backspace();
                cx.notify();
            }
            "delete" => {
                edit.input.delete();
                cx.notify();
            }
            _ if !event.keystroke.modifiers.control
                && !event.keystroke.modifiers.alt
                && !event.keystroke.modifiers.platform
                && !event.keystroke.modifiers.function =>
            {
                if let Some(key_char) = event.keystroke.key_char.as_ref() {
                    edit.input.insert_text(key_char);
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    fn handle_rime_path_mouse(
        &mut self,
        field: RimePathField,
        event: SettingTextInputMouseEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SettingTextInputMouseEvent::Down { index, .. } = event {
            if !matches!(
                self.rime_path_editing.as_ref(),
                Some(edit) if edit.field == field
            ) {
                self.begin_rime_path_edit_at(field, Some(index), window, cx);
            }
        }

        let Some(edit) = self
            .rime_path_editing
            .as_mut()
            .filter(|edit| edit.field == field)
        else {
            return;
        };
        match event {
            SettingTextInputMouseEvent::Down { index, shift } => {
                edit.input.begin_mouse_selection(index, shift);
            }
            SettingTextInputMouseEvent::Drag { index } => {
                edit.input.extend_mouse_selection(index);
            }
            SettingTextInputMouseEvent::Up => edit.input.end_mouse_selection(),
        }
        cx.notify();
    }

    fn rime_path_input(
        &self,
        field: RimePathField,
        value: String,
        placeholder: &'static str,
        read_only: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let editing_state = (!read_only).then(|| {
            self.rime_path_editing
                .as_ref()
                .filter(|edit| edit.field == field)
                .map(|edit| edit.input.clone())
        });
        let editing_state = editing_state.flatten();
        let editing = editing_state.is_some();
        let input_state = editing_state
            .clone()
            .unwrap_or_else(|| SettingTextInputState::new(value));
        let focus_handle = self.rime_path_focus_handles[field.index()].clone();
        let config = SettingTextInputConfig::new(
            ("settings-rime-path", field as u32),
            input_state,
            placeholder,
            editing,
            focus_handle,
        );
        if read_only {
            return setting_text_input(config.read_only());
        }

        setting_text_input(
            config
                .on_click(cx.listener(move |this, _, window, cx| {
                    if !matches!(
                        this.rime_path_editing.as_ref(),
                        Some(edit) if edit.field == field
                    ) {
                        this.begin_rime_path_edit(field, window, cx);
                    }
                }))
                .on_key_down(cx.listener(move |this, event, window, cx| {
                    this.handle_rime_path_key(field, event, window, cx);
                }))
                .on_mouse(cx.processor(move |this, event, window, cx| {
                    this.handle_rime_path_mouse(field, event, window, cx);
                })),
        )
    }

    fn set_paste_shortcut(&mut self, shortcut: settings::PasteShortcut, cx: &mut Context<Self>) {
        self.source.update(cx, |view, cx| {
            let mut next = view.app.settings_value();
            next.paste_shortcut = shortcut;
            view.update_settings(next);
            cx.notify();
        });
        self.recording_paste_shortcut = false;
        cx.notify();
    }

    fn set_rime_toggle_shortcut(
        &mut self,
        shortcut: settings::RimeToggleShortcut,
        cx: &mut Context<Self>,
    ) {
        self.source.update(cx, |view, cx| {
            let mut next = view.app.settings_value();
            next.rime_toggle_shortcut = shortcut;
            view.update_settings(next);
            cx.notify();
        });
        self.recording_rime_toggle_shortcut = false;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::SettingTextInputState;

    #[test]
    fn text_input_moves_on_grapheme_boundaries() {
        let mut input = SettingTextInputState::new("a👩‍💻b".to_owned());
        input.move_to(input.value.len(), false);
        input.move_left(false);
        assert_eq!(&input.value[input.cursor..], "b");

        input.move_left(false);
        assert_eq!(&input.value[input.cursor..], "👩‍💻b");
    }

    #[test]
    fn text_input_mouse_selection_replaces_selected_text() {
        let mut input = SettingTextInputState::new("/tmp/rime-data".to_owned());
        input.move_to(5, false);
        input.begin_mouse_selection(5, false);
        input.extend_mouse_selection(14);

        assert_eq!(input.selected_range(), 5..14);
        input.insert_text("shared");
        assert_eq!(input.value, "/tmp/shared");
        assert_eq!(input.cursor, "/tmp/shared".len());
        assert!(!input.has_selection());
    }

    #[test]
    fn text_input_shift_selection_can_be_reversed_and_collapsed() {
        let mut input = SettingTextInputState::new("abcdef".to_owned());
        input.move_to(5, false);
        input.begin_mouse_selection(5, false);
        input.extend_mouse_selection(2);

        assert_eq!(input.selected_range(), 2..5);
        input.move_right(false);
        assert_eq!(input.cursor, 5);
        assert!(!input.has_selection());
    }

    #[test]
    fn text_input_select_all_and_backspace_clear_the_value() {
        let mut input = SettingTextInputState::new("/tmp/rime".to_owned());
        input.select_all();
        input.backspace();

        assert!(input.value.is_empty());
        assert_eq!(input.cursor, 0);
        assert!(!input.has_selection());
    }
}
