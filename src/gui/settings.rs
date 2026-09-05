use crate::{
    app::{themed_titlebar, themed_titlebar_enabled, NvimGpui},
    helper, settings,
    widgets::{
        setting_checkbox, setting_combo_box, setting_combo_option, setting_option_button,
        setting_row, setting_section, setting_text_input, SettingTextInputConfig,
        SettingTextInputMouseEvent, SettingTextInputState, ACCENT, BACKGROUND, MUTED_TEXT, SURFACE,
        SURFACE_BRIGHT, TEXT,
    },
};
use gpui::{
    div, prelude::*, px, rgb, Context, Entity, FocusHandle, FontFallbacks, KeyDownEvent, Render,
    SharedString, Subscription, Window,
};
use nvim_gpui::rime::RimeRuntimeResolver;
use std::env;

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
    open_combo: Option<SettingsCombo>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsCombo {
    NerdFont,
    FallbackMode,
    StartupMaximized,
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
            open_combo: None,
        }
    }

    fn toggle_combo(&mut self, combo: SettingsCombo, cx: &mut Context<Self>) {
        self.open_combo = (self.open_combo != Some(combo)).then_some(combo);
        self.recording_paste_shortcut = false;
        self.recording_rime_toggle_shortcut = false;
        self.commit_rime_path_edit(cx);
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
            let mut next = view.settings_value();
            update(&mut next);
            view.update_settings(next);
            cx.notify();
        });
        self.open_combo = None;
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
        let value = Self::rime_path_value(&self.source.read(cx).settings_value(), field);
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
            let mut next = view.settings_value();
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
        let text = match crate::clipboard::paste_text(cx) {
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
        let settings = self.source.read(cx).settings_value();
        if let Some(reason) = Self::rime_test_block_reason(&settings) {
            self.rime_test_status = Some(RimeTestStatus::Blocked(reason));
            cx.notify();
            return;
        }
        self.rime_test_status = Some(RimeTestStatus::Testing);
        cx.notify();

        let task = cx.background_spawn(async move {
            NvimGpui::test_rime_configuration_with_settings(settings)
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
            let mut next = view.settings_value();
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
            let mut next = view.settings_value();
            next.rime_toggle_shortcut = shortcut;
            view.update_settings(next);
            cx.notify();
        });
        self.recording_rime_toggle_shortcut = false;
        cx.notify();
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_rime_path_blur_subscriptions(window, cx);
        let (current, save_error, cli_install_error) = self.source.read(cx).settings_snapshot();
        let cli_available = helper::is_available_in_path();
        let mut paste_shortcut_icon_font = window.text_style().font();
        paste_shortcut_icon_font.fallbacks = Some(FontFallbacks::from_fonts(vec![current
            .nerd_font
            .family()
            .to_owned()]));

        let mut nerd_font_options = div().w_full().flex().flex_col();
        for (id, choice) in [
            ("settings-nerd-symbols", settings::NerdFontChoice::Symbols),
            (
                "settings-nerd-symbols-mono",
                settings::NerdFontChoice::SymbolsMono,
            ),
        ] {
            nerd_font_options = nerd_font_options.child(setting_combo_option(
                id,
                choice.label(),
                current.nerd_font == choice,
                cx.listener(move |this, _, _, cx| {
                    this.apply_setting(|settings| settings.nerd_font = choice, cx);
                }),
            ));
        }
        let nerd_font_options = setting_combo_box(
            "settings-nerd-font-combo",
            current.nerd_font.label(),
            self.open_combo == Some(SettingsCombo::NerdFont),
            nerd_font_options,
            paste_shortcut_icon_font.clone(),
            cx.listener(|this, _, _, cx| this.toggle_combo(SettingsCombo::NerdFont, cx)),
        );

        let mut fallback_options = div().w_full().flex().flex_col();
        for (id, mode) in [
            ("settings-fallback-none", settings::FallbackMode::None),
            ("settings-fallback-auto", settings::FallbackMode::Auto),
            ("settings-fallback-force", settings::FallbackMode::Force),
        ] {
            fallback_options = fallback_options.child(setting_combo_option(
                id,
                mode.label(),
                current.fallback_mode == mode,
                cx.listener(move |this, _, _, cx| {
                    this.apply_setting(|settings| settings.fallback_mode = mode, cx);
                }),
            ));
        }
        let fallback_options = setting_combo_box(
            "settings-fallback-combo",
            current.fallback_mode.label(),
            self.open_combo == Some(SettingsCombo::FallbackMode),
            fallback_options,
            paste_shortcut_icon_font.clone(),
            cx.listener(|this, _, _, cx| this.toggle_combo(SettingsCombo::FallbackMode, cx)),
        );

        let mut cache_options = div().w_full().flex().flex_col();
        for megabytes in settings::IMAGE_CACHE_SIZE_OPTIONS_MB {
            let label = format!("{megabytes} MB");
            cache_options = cache_options.child(setting_combo_option(
                ("settings-cache", *megabytes),
                label,
                current.image_cache_size_mb == *megabytes,
                cx.listener(move |this, _, _, cx| {
                    this.apply_setting(|settings| settings.image_cache_size_mb = *megabytes, cx);
                }),
            ));
        }
        let cache_options = setting_combo_box(
            "settings-cache-combo",
            format!("{} MB", current.image_cache_size_mb),
            self.open_combo == Some(SettingsCombo::ImageCacheSize),
            cache_options,
            paste_shortcut_icon_font.clone(),
            cx.listener(|this, _, _, cx| this.toggle_combo(SettingsCombo::ImageCacheSize, cx)),
        );

        let mut ime_backend_options = div().w_full().flex().flex_col();
        for (id, backend) in [
            ("settings-ime-system", settings::ImeBackend::System),
            ("settings-ime-rime", settings::ImeBackend::Rime),
        ] {
            ime_backend_options = ime_backend_options.child(setting_combo_option(
                id,
                backend.label(),
                current.ime_backend == backend,
                cx.listener(move |this, _, _, cx| {
                    this.apply_setting(|settings| settings.ime_backend = backend, cx);
                }),
            ));
        }
        let ime_backend_combo = setting_combo_box(
            "settings-ime-backend-combo",
            current.ime_backend.label(),
            self.open_combo == Some(SettingsCombo::ImeBackend),
            ime_backend_options,
            paste_shortcut_icon_font.clone(),
            cx.listener(|this, _, _, cx| this.toggle_combo(SettingsCombo::ImeBackend, cx)),
        );

        let mut rime_layout_options = div().w_full().flex().flex_col();
        for (id, layout) in [
            (
                "settings-rime-layout-vertical",
                settings::RimeCandidateLayout::Vertical,
            ),
            (
                "settings-rime-layout-horizontal",
                settings::RimeCandidateLayout::Horizontal,
            ),
        ] {
            rime_layout_options = rime_layout_options.child(setting_combo_option(
                id,
                layout.label(),
                current.rime_candidate_layout == layout,
                cx.listener(move |this, _, _, cx| {
                    this.apply_setting(|settings| settings.rime_candidate_layout = layout, cx);
                }),
            ));
        }
        let rime_layout_combo = setting_combo_box(
            "settings-rime-layout-combo",
            current.rime_candidate_layout.label(),
            self.open_combo == Some(SettingsCombo::RimeCandidateLayout),
            rime_layout_options,
            paste_shortcut_icon_font.clone(),
            cx.listener(|this, _, _, cx| this.toggle_combo(SettingsCombo::RimeCandidateLayout, cx)),
        );

        let paste_shortcut_label: SharedString = if self.recording_paste_shortcut {
            "Press a key combination…".into()
        } else {
            current.paste_shortcut.label().into()
        };
        let paste_shortcut_focus_handle = self.paste_shortcut_focus_handle.clone();
        let paste_shortcut_input = div()
            .id("settings-paste-shortcut-input")
            .w_full()
            .h(px(36.0))
            .flex()
            .items_center()
            .px_3()
            .rounded_sm()
            .border_1()
            .border_color(rgb(if self.recording_paste_shortcut {
                ACCENT
            } else {
                SURFACE_BRIGHT
            }))
            .bg(rgb(SURFACE))
            .track_focus(&paste_shortcut_focus_handle)
            .focus(|style| style.border_color(rgb(ACCENT)))
            .hover(|style| style.border_color(rgb(ACCENT)))
            .on_click(cx.listener(|this, _, window, cx| {
                this.recording_paste_shortcut = true;
                this.paste_shortcut_focus_handle.focus(window);
                cx.notify();
            }))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if !this.recording_paste_shortcut {
                    return;
                }

                window.prevent_default();
                cx.stop_propagation();

                match event.keystroke.key.as_str() {
                    "escape" => {
                        this.recording_paste_shortcut = false;
                        cx.notify();
                    }
                    "backspace" | "delete" => {
                        this.set_paste_shortcut(settings::PasteShortcut::Disabled, cx);
                    }
                    _ => {
                        if let Some(shortcut) =
                            settings::PasteShortcut::from_keystroke(&event.keystroke)
                        {
                            this.set_paste_shortcut(shortcut, cx);
                        }
                    }
                }
            }))
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(rgb(if self.recording_paste_shortcut {
                        ACCENT
                    } else if current.paste_shortcut.is_disabled() {
                        MUTED_TEXT
                    } else {
                        TEXT
                    }))
                    .child(paste_shortcut_label),
            )
            .child(
                div()
                    .id("settings-paste-shortcut-clear")
                    .w(px(28.0))
                    .h(px(28.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .font(paste_shortcut_icon_font.clone())
                    .text_color(rgb(MUTED_TEXT))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(SURFACE_BRIGHT)).text_color(rgb(TEXT)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.set_paste_shortcut(settings::PasteShortcut::Disabled, cx);
                    }))
                    .child(""),
            );

        let rime_toggle_shortcut_label: SharedString = if self.recording_rime_toggle_shortcut {
            "Press a key combination…".into()
        } else {
            current.rime_toggle_shortcut.label().into()
        };
        let rime_toggle_shortcut_focus_handle = self.rime_toggle_shortcut_focus_handle.clone();
        let rime_toggle_shortcut_input = div()
            .id("settings-rime-toggle-shortcut-input")
            .w_full()
            .h(px(36.0))
            .flex()
            .items_center()
            .px_3()
            .rounded_sm()
            .border_1()
            .border_color(rgb(if self.recording_rime_toggle_shortcut {
                ACCENT
            } else {
                SURFACE_BRIGHT
            }))
            .bg(rgb(SURFACE))
            .track_focus(&rime_toggle_shortcut_focus_handle)
            .focus(|style| style.border_color(rgb(ACCENT)))
            .hover(|style| style.border_color(rgb(ACCENT)))
            .on_click(cx.listener(|this, _, window, cx| {
                this.recording_rime_toggle_shortcut = true;
                this.rime_toggle_shortcut_focus_handle.focus(window);
                cx.notify();
            }))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if !this.recording_rime_toggle_shortcut {
                    return;
                }

                window.prevent_default();
                cx.stop_propagation();

                match event.keystroke.key.as_str() {
                    "escape" => {
                        this.recording_rime_toggle_shortcut = false;
                        cx.notify();
                    }
                    "backspace" | "delete" => {
                        this.set_rime_toggle_shortcut(settings::RimeToggleShortcut::Disabled, cx);
                    }
                    _ => {
                        if let Some(shortcut) =
                            settings::RimeToggleShortcut::from_keystroke(&event.keystroke)
                        {
                            this.set_rime_toggle_shortcut(shortcut, cx);
                        }
                    }
                }
            }))
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(rgb(if self.recording_rime_toggle_shortcut {
                        ACCENT
                    } else if current.rime_toggle_shortcut.is_disabled() {
                        MUTED_TEXT
                    } else {
                        TEXT
                    }))
                    .child(rime_toggle_shortcut_label),
            )
            .child(
                div()
                    .id("settings-rime-toggle-shortcut-clear")
                    .w(px(28.0))
                    .h(px(28.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .font(paste_shortcut_icon_font.clone())
                    .text_color(rgb(MUTED_TEXT))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(SURFACE_BRIGHT)).text_color(rgb(TEXT)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.set_rime_toggle_shortcut(settings::RimeToggleShortcut::Disabled, cx);
                    }))
                    .child(""),
            );

        let bundled_rime_runtime = Self::uses_bundled_rime_runtime();
        let rime_library_input = self.rime_path_input(
            RimePathField::Library,
            Self::rime_path_value(&current, RimePathField::Library),
            if bundled_rime_runtime {
                "Bundled librime"
            } else {
                "Auto-detect librime"
            },
            bundled_rime_runtime,
            cx,
        );
        let rime_data_input = self.rime_path_input(
            RimePathField::Data,
            Self::rime_path_value(&current, RimePathField::Data),
            if bundled_rime_runtime {
                "Bundled Rime data"
            } else {
                "Use NVIM_GPUI_RIME_SHARED_DIR"
            },
            bundled_rime_runtime,
            cx,
        );
        let rime_user_data_input = self.rime_path_input(
            RimePathField::UserData,
            Self::rime_path_value(&current, RimePathField::UserData),
            "Application support/rime",
            true,
            cx,
        );
        let rime_source = self.source.clone();
        let rime_library_detect = setting_option_button(
            "settings-rime-library-auto-detect",
            "Detect",
            current.rime_library_auto_detect,
            move |cx| {
                rime_source.update(cx, |view, cx| {
                    let mut next = view.settings_value();
                    next.rime_library_dir.clear();
                    next.rime_library_auto_detect = true;
                    view.update_settings(next);
                    cx.notify();
                });
            },
        );
        let mut rime_library_control = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_1()
            .items_center()
            .gap_2()
            .child(rime_library_input);
        if !bundled_rime_runtime {
            rime_library_control = rime_library_control.child(rime_library_detect);
        }
        let rime_user_data_open = div()
            .id("settings-open-rime-user-data")
            .px_3()
            .py_2()
            .rounded_sm()
            .text_sm()
            .flex_shrink_0()
            .bg(rgb(SURFACE_BRIGHT))
            .text_color(rgb(TEXT))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(0x45475a)))
            .on_click(cx.listener(|this, _, _, cx| {
                this.open_rime_user_data_directory(cx);
            }))
            .child("Open");
        let mut rime_user_data_control = div().w_full().flex().flex_col().gap_1().child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_2()
                .child(rime_user_data_input)
                .child(rime_user_data_open),
        );
        if let Some(error) = self.rime_user_data_error.as_ref() {
            rime_user_data_control = rime_user_data_control.child(
                div()
                    .text_sm()
                    .text_color(rgb(0xf38ba8))
                    .child(format!("Could not open Rime data: {error}")),
            );
        }
        let rime_test_block_reason = Self::rime_test_block_reason(&current);
        let rime_test_ready = rime_test_block_reason.is_none();
        let rime_test_running = matches!(self.rime_test_status, Some(RimeTestStatus::Testing));
        let rime_test_enabled = rime_test_ready && !rime_test_running;
        let rime_test_button = div()
            .id("settings-rime-test")
            .mx(px(2.0))
            .px_3()
            .py_2()
            .rounded_sm()
            .text_sm()
            .flex_shrink_0()
            .bg(rgb(if rime_test_enabled {
                SURFACE_BRIGHT
            } else {
                SURFACE
            }))
            .text_color(rgb(if rime_test_enabled { TEXT } else { MUTED_TEXT }))
            .when(rime_test_enabled, |element| {
                element
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0x45475a)))
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.test_rime_configuration(cx);
            }))
            .child(if rime_test_running {
                "Testing…"
            } else {
                "Test"
            });
        let rime_test_status = self.rime_test_status.as_ref().map(|status| match status {
            RimeTestStatus::Testing => (MUTED_TEXT, "Testing librime…".to_owned()),
            RimeTestStatus::Complete(Ok(())) => (
                ACCENT,
                "Success: Rime configuration loaded; session created.".to_owned(),
            ),
            RimeTestStatus::Complete(Err(error)) => (
                0xf38ea8,
                format!("Failed: Rime configuration failed: {error}"),
            ),
            RimeTestStatus::Blocked(reason) => (MUTED_TEXT, format!("Not ready: {reason}")),
        });
        let rime_test_requirement = rime_test_block_reason
            .as_ref()
            .map(|reason| (MUTED_TEXT, format!("Not ready: {reason}")));

        let mut ime_content = div().w_full().child(setting_row(
            "Input method",
            "Choose the text input backend used while editing.",
            ime_backend_combo,
        ));
        if current.ime_backend == settings::ImeBackend::Rime {
            let mut rime_content = div()
                .w_full()
                .child(setting_row(
                    "Candidate layout",
                    "Show Rime candidates in a vertical list or a horizontal row.",
                    rime_layout_combo,
                ))
                .child(setting_row(
                    "Activation shortcut",
                    "Toggle the built-in Rime backend. Changes to this shortcut are saved immediately.",
                    rime_toggle_shortcut_input,
                ));
            if let Some((color, status)) = rime_test_requirement {
                rime_content = rime_content.child(
                    div()
                        .mx_3()
                        .mb_2()
                        .text_sm()
                        .text_color(rgb(color))
                        .child(status),
                );
            }
            rime_content = rime_content
                .child(setting_row(
                    "librime directory",
                    if bundled_rime_runtime {
                        "Read-only path to the librime library shipped in the application bundle."
                    } else {
                        "Library path or directory. Automatic detection checks NVIM_GPUI_RIME_LIBRARY, then bundled and system paths."
                    },
                    rime_library_control,
                ))
                .child(setting_row(
                    "Rime data directory",
                    if bundled_rime_runtime {
                        "Read-only schema and dictionary data shipped in the application bundle."
                    } else {
                        "Shared schema and dictionary data. Leave empty to use NVIM_GPUI_RIME_SHARED_DIR."
                    },
                    rime_data_input,
                ))
                .child(setting_row(
                    "User data directory",
                    "Writable Rime user data in the nvim-gpui application-support directory.",
                    rime_user_data_control,
                ))
                .child(
                    div()
                        .mx_3()
                        .mb_3()
                        .text_sm()
                        .text_color(rgb(MUTED_TEXT))
                        .child(if bundled_rime_runtime {
                            "librime and Rime data are fixed by the application bundle; user data is kept in the displayed directory."
                        } else {
                            "librime and Rime data directory changes take effect after restart."
                        }),
                )
                .child(setting_row(
                    "Test Rime configuration",
                    "Load the configured librime and create a session.",
                    rime_test_button,
                ));
            if let Some((color, status)) = rime_test_status {
                rime_content = rime_content.child(
                    div()
                        .mx_3()
                        .mb_3()
                        .mt_1()
                        .text_sm()
                        .text_color(rgb(color))
                        .child(status),
                );
            }
            ime_content = ime_content.child(rime_content);
        }

        let startup_options = div()
            .w_full()
            .flex()
            .flex_col()
            .child(setting_combo_option(
                "settings-startup-on",
                "On",
                current.startup_maximized,
                cx.listener(|this, _, _, cx| {
                    this.apply_setting(|settings| settings.startup_maximized = true, cx);
                }),
            ))
            .child(setting_combo_option(
                "settings-startup-off",
                "Off",
                !current.startup_maximized,
                cx.listener(|this, _, _, cx| {
                    this.apply_setting(|settings| settings.startup_maximized = false, cx);
                }),
            ));
        let startup_options = setting_combo_box(
            "settings-startup-combo",
            if current.startup_maximized {
                "On"
            } else {
                "Off"
            },
            self.open_combo == Some(SettingsCombo::StartupMaximized),
            startup_options,
            paste_shortcut_icon_font.clone(),
            cx.listener(|this, _, _, cx| this.toggle_combo(SettingsCombo::StartupMaximized, cx)),
        );

        let quit_on_window_close = setting_checkbox(
            "settings-quit-on-window-close",
            "Quit when the main window closes",
            current.quit_on_window_close,
            cx.listener(|this, _, _, cx| {
                this.apply_setting(
                    |settings| {
                        settings.quit_on_window_close = !settings.quit_on_window_close;
                    },
                    cx,
                );
            }),
        );

        let allow_multiple_instances = setting_checkbox(
            "settings-allow-multiple-instances",
            "Allow multiple instances",
            current.allow_multiple_instances,
            cx.listener(|this, _, _, cx| {
                this.apply_setting(
                    |settings| {
                        settings.allow_multiple_instances = !settings.allow_multiple_instances;
                    },
                    cx,
                );
            }),
        );

        let mut log_options = div().w_full().flex().flex_col();
        for (index, level) in [
            settings::LogLevel::Off,
            settings::LogLevel::Error,
            settings::LogLevel::Warn,
            settings::LogLevel::Info,
            settings::LogLevel::Debug,
            settings::LogLevel::Trace,
        ]
        .into_iter()
        .enumerate()
        {
            log_options = log_options.child(setting_combo_option(
                ("settings-log-level", index),
                level.label(),
                current.log_level == level,
                cx.listener(move |this, _, _, cx| {
                    this.apply_setting(|settings| settings.log_level = level, cx);
                }),
            ));
        }
        let log_options = setting_combo_box(
            "settings-log-level-combo",
            current.log_level.label(),
            self.open_combo == Some(SettingsCombo::LogLevel),
            log_options,
            paste_shortcut_icon_font,
            cx.listener(|this, _, _, cx| this.toggle_combo(SettingsCombo::LogLevel, cx)),
        );
        let log_directory_label = crate::logging::log_directory()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "Log directory unavailable".to_owned());
        let mut log_directory_control = div().w_full().flex().flex_col().gap_1().child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .h(px(36.0))
                        .flex()
                        .items_center()
                        .px_3()
                        .rounded_sm()
                        .border_1()
                        .border_color(rgb(SURFACE_BRIGHT))
                        .bg(rgb(SURFACE))
                        .text_sm()
                        .text_color(rgb(TEXT))
                        .child(
                            div()
                                .w_full()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(log_directory_label),
                        ),
                )
                .child(
                    div()
                        .id("settings-open-log-directory")
                        .px_3()
                        .py_2()
                        .rounded_sm()
                        .text_sm()
                        .bg(rgb(SURFACE_BRIGHT))
                        .text_color(rgb(TEXT))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(0x45475a)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_log_directory(cx);
                        }))
                        .child("Open"),
                ),
        );
        if let Some(error) = self.log_directory_error.as_ref() {
            log_directory_control = log_directory_control.child(
                div()
                    .text_sm()
                    .text_color(rgb(0xf38ba8))
                    .child(format!("Could not open logs: {error}")),
            );
        }

        let source = self.source.clone();
        let cli_options = div()
            .w_full()
            .flex()
            .items_center()
            .child(setting_option_button(
                "settings-cli-install",
                if cli_available {
                    "Installed"
                } else {
                    "Install CLI (gpvim)"
                },
                cli_available,
                move |cx| {
                    let source = source.clone();
                    let task = cx.background_spawn(async move { helper::install() });
                    cx.spawn(async move |cx| {
                        let result = task.await;
                        let _ = source.update(cx, |view, cx| {
                            view.set_cli_install_error(result.err());
                            cx.notify();
                        });
                    })
                    .detach();
                },
            ));

        let mut content = div()
            .id("settings-scroll")
            .flex_1()
            .overflow_y_scroll()
            .px_6()
            .py_5()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(TEXT))
            .child(div().text_lg().child("Settings"))
            .child(setting_section(
                "Application behavior",
                div()
                    .w_full()
                    .child(setting_row(
                        "Startup maximized",
                        "Open the main editor window in its maximized state.",
                        startup_options,
                    ))
                    .child(setting_row(
                        "Quit behavior",
                        "Choose whether closing the main editor window also exits nvim-gpui.",
                        quit_on_window_close,
                    ))
                    .child(setting_row(
                        "Instance behavior",
                        "Allow another nvim-gpui process to run at the same time.",
                        allow_multiple_instances,
                    ))
                    .child(setting_row(
                        "Log level",
                        "Write runtime logs at the selected level. Logging is disabled by default.",
                        log_options,
                    ))
                    .child(setting_row(
                        "Log directory",
                        "Read-only location used for runtime log files.",
                        log_directory_control,
                    )),
            ))
            .child(setting_section(
                "Font and image",
                div()
                    .w_full()
                    .child(setting_row(
                        "Nerd font",
                        "Font used for bundled Nerd Font fallback glyphs.",
                        nerd_font_options,
                    ))
                    .child(setting_row(
                        "Fallback mode",
                        "Choose whether missing Nerd glyphs use the selected fallback font.",
                        fallback_options,
                    ))
                    .child(setting_row(
                        "Image cache size",
                        "Maximum unplaced Kitty Graphics Protocol image data kept in memory.",
                        cache_options,
                    )),
            ))
            .child(setting_section(
                "IME",
                ime_content,
            ))
            .child(setting_section(
                "Clipboard",
                setting_row(
                    "Paste shortcut",
                    "Read the local system clipboard and paste it through Neovim.",
                    paste_shortcut_input,
                ),
            ))
            .child(setting_section(
                "Utils",
                setting_row(
                    "Command-line helper",
                    "Install gpvim and gpvimdiff so files can be opened or compared from a terminal.",
                    cli_options,
                ),
            ));

        if let Some(error) = save_error {
            content = content.child(
                div()
                    .mt_4()
                    .text_sm()
                    .text_color(rgb(0xf38ba8))
                    .child(format!("Settings could not be saved: {error}")),
            );
        }

        if let Some(error) = cli_install_error {
            content = content.child(
                div()
                    .mt_4()
                    .text_sm()
                    .text_color(rgb(0xf38ba8))
                    .child(format!("gpvim could not be installed: {error}")),
            );
        }

        let mut root = div().size_full().flex().flex_col().bg(rgb(BACKGROUND));
        root = root.on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.commit_rime_path_edit(cx);
            if this.open_combo.take().is_some() {
                cx.notify();
            }
        }));
        if themed_titlebar_enabled() {
            root = root.child(themed_titlebar(
                "nvim-gpui settings".to_owned(),
                BACKGROUND,
                TEXT,
                None,
                None,
            ));
        }
        root.child(content)
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
