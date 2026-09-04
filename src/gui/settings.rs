use crate::{
    app::{themed_titlebar, themed_titlebar_enabled, NvimGpui},
    helper, settings,
    widgets::{
        setting_checkbox, setting_combo_box, setting_combo_option, setting_option_button,
        setting_row, setting_section, ACCENT, BACKGROUND, MUTED_TEXT, SURFACE, SURFACE_BRIGHT,
        TEXT,
    },
};
use gpui::{
    div, prelude::*, px, rgb, Context, Entity, FocusHandle, FontFallbacks, KeyDownEvent,
    MouseButton, Render, SharedString, Subscription, Window,
};

pub(crate) struct SettingsWindow {
    source: Entity<NvimGpui>,
    _source_subscription: Subscription,
    paste_shortcut_focus_handle: FocusHandle,
    recording_paste_shortcut: bool,
    open_combo: Option<SettingsCombo>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsCombo {
    NerdFont,
    FallbackMode,
    StartupMaximized,
    LogLevel,
    ImageCacheSize,
}

impl SettingsWindow {
    pub(crate) fn new(source: Entity<NvimGpui>, cx: &mut Context<Self>) -> Self {
        let source_subscription = cx.observe(&source, |_, _, cx| cx.notify());
        Self {
            source,
            _source_subscription: source_subscription,
            paste_shortcut_focus_handle: cx.focus_handle().tab_stop(true),
            recording_paste_shortcut: false,
            open_combo: None,
        }
    }

    fn toggle_combo(&mut self, combo: SettingsCombo, cx: &mut Context<Self>) {
        self.open_combo = (self.open_combo != Some(combo)).then_some(combo);
        self.recording_paste_shortcut = false;
        cx.notify();
    }

    fn apply_setting(
        &mut self,
        update: impl FnOnce(&mut settings::Settings),
        cx: &mut Context<Self>,
    ) {
        self.source.update(cx, |view, cx| {
            let mut next = view.settings_value();
            update(&mut next);
            view.update_settings(next);
            cx.notify();
        });
        self.open_combo = None;
        cx.notify();
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
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                "Keyboard",
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
        root = root.on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                if this.open_combo.take().is_some() {
                    cx.notify();
                }
            }),
        );
        if themed_titlebar_enabled() {
            root = root.child(themed_titlebar(
                "nvim-gpui settings".to_owned(),
                BACKGROUND,
                TEXT,
                None,
            ));
        }
        root.child(content)
    }
}
