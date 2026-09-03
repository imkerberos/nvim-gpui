use super::*;

pub(super) struct DebugWindow {
    source: Entity<NvimGpui>,
    _source_subscription: Subscription,
}

impl DebugWindow {
    pub(super) fn new(source: Entity<NvimGpui>, cx: &mut Context<Self>) -> Self {
        let source_subscription = cx.observe(&source, |_, _, cx| cx.notify());
        Self {
            source,
            _source_subscription: source_subscription,
        }
    }
}

impl Render for DebugWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.source.read(cx);
        let guifont = view
            .resolved_grid_font
            .as_ref()
            .map(|font| format!("{}:h{}", font.family, font.size))
            .or_else(|| view.guifont.clone())
            .unwrap_or_else(|| "system monospace (resolving)".to_owned());
        let guifontwide = view
            .resolved_grid_wide_font
            .as_ref()
            .map(|font| format!("{}:h{}", font.family, font.size))
            .or_else(|| view.guifontwide.clone())
            .unwrap_or_else(|| "same as guifont (fallback)".to_owned());
        let grid_size = view
            .grid_size
            .map(|(width, height)| format!("{width}×{height}"))
            .unwrap_or_else(|| "pending".to_owned());
        let ime_status = if view.system_ime.is_empty() {
            "IME: system".to_owned()
        } else {
            format!("IME composing: {}", view.system_ime.text())
        };
        let debug_row = |label: &'static str, value: String| {
            div()
                .w_full()
                .flex()
                .items_start()
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(rgb(ACCENT))
                        .child(format!("{label}: ")),
                )
                .child(div().flex_1().whitespace_normal().child(value))
        };

        let debug_content = div()
            .flex_1()
            .flex()
            .flex_col()
            .justify_start()
            .overflow_hidden()
            .px_3()
            .py_2()
            .bg(rgb(SURFACE))
            .text_color(rgb(MUTED_TEXT))
            .border_b_1()
            .border_color(rgb(SURFACE_BRIGHT))
            .child(
                div()
                    .w_full()
                    .text_color(rgb(ACCENT))
                    .child("DEBUG  nvim-gpui"),
            )
            .child(debug_row("RPC", view.rpc_status.clone()))
            .child(debug_row("Grid", grid_size))
            .child(debug_row("guifont", guifont))
            .child(debug_row("guifontwide", guifontwide))
            .child(debug_row("File", view.state.file.to_owned()))
            .child(debug_row(
                "State",
                format!(
                    "{} {}:{}",
                    view.state.mode, view.state.line, view.state.column
                ),
            ))
            .child(debug_row("Input", ime_status))
            .child(debug_row(
                "API",
                view.api_level.unwrap_or_default().to_string(),
            ));
        let mut root = div().size_full().flex().flex_col().bg(rgb(SURFACE));
        if themed_titlebar_enabled() {
            root = root.child(themed_titlebar(
                "nvim-gpui debug".to_owned(),
                SURFACE,
                TEXT,
                None,
            ));
        }
        root.child(debug_content)
    }
}

pub(super) struct SettingsWindow {
    source: Entity<NvimGpui>,
    _source_subscription: Subscription,
}

impl SettingsWindow {
    fn new(source: Entity<NvimGpui>, cx: &mut Context<Self>) -> Self {
        let source_subscription = cx.observe(&source, |_, _, cx| cx.notify());
        Self {
            source,
            _source_subscription: source_subscription,
        }
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (current, save_error, cli_install_error) = {
            let view = self.source.read(cx);
            (
                view.settings.clone(),
                view.settings_save_error.clone(),
                view.cli_install_error.clone(),
            )
        };
        let source = self.source.clone();
        let cli_available = helper::is_available_in_path();

        let mut nerd_font_options = div().w_full().flex().items_center();
        for (id, choice) in [
            ("settings-nerd-symbols", settings::NerdFontChoice::Symbols),
            (
                "settings-nerd-symbols-mono",
                settings::NerdFontChoice::SymbolsMono,
            ),
        ] {
            let source = source.clone();
            nerd_font_options = nerd_font_options.child(setting_option_button(
                id,
                choice.label(),
                current.nerd_font == choice,
                move |cx| {
                    source.update(cx, |view, cx| {
                        let mut next = view.settings.clone();
                        next.nerd_font = choice;
                        view.update_settings(next);
                        cx.notify();
                    });
                },
            ));
        }

        let mut fallback_options = div().w_full().flex().items_center();
        for (id, mode) in [
            ("settings-fallback-none", settings::FallbackMode::None),
            ("settings-fallback-auto", settings::FallbackMode::Auto),
            ("settings-fallback-force", settings::FallbackMode::Force),
        ] {
            let source = source.clone();
            fallback_options = fallback_options.child(setting_option_button(
                id,
                mode.label(),
                current.fallback_mode == mode,
                move |cx| {
                    source.update(cx, |view, cx| {
                        let mut next = view.settings.clone();
                        next.fallback_mode = mode;
                        view.update_settings(next);
                        cx.notify();
                    });
                },
            ));
        }

        let mut cache_options = div().w_full().flex().items_center();
        for megabytes in settings::IMAGE_CACHE_SIZE_OPTIONS_MB {
            let source = source.clone();
            let label = format!("{megabytes} MB");
            cache_options = cache_options.child(setting_option_button(
                ("settings-cache", *megabytes),
                label,
                current.image_cache_size_mb == *megabytes,
                move |cx| {
                    source.update(cx, |view, cx| {
                        let mut next = view.settings.clone();
                        next.image_cache_size_mb = *megabytes;
                        view.update_settings(next);
                        cx.notify();
                    });
                },
            ));
        }

        let mut paste_shortcut_options = div().w_full().flex().items_center();
        for (index, shortcut) in [
            settings::PasteShortcut::CmdV,
            settings::PasteShortcut::CtrlV,
            settings::PasteShortcut::Disabled,
        ]
        .into_iter()
        .enumerate()
        {
            let source = source.clone();
            paste_shortcut_options = paste_shortcut_options.child(setting_option_button(
                ("settings-paste-shortcut", index),
                shortcut.label(),
                current.paste_shortcut == shortcut,
                move |cx| {
                    source.update(cx, |view, cx| {
                        let mut next = view.settings.clone();
                        next.paste_shortcut = shortcut;
                        view.update_settings(next);
                        cx.notify();
                    });
                },
            ));
        }

        let source = self.source.clone();
        let startup_options = div()
            .w_full()
            .flex()
            .items_center()
            .child(setting_option_button(
                "settings-startup-on",
                "On",
                current.startup_maximized,
                {
                    let source = source.clone();
                    move |cx| {
                        source.update(cx, |view, cx| {
                            let mut next = view.settings.clone();
                            next.startup_maximized = true;
                            view.update_settings(next);
                            cx.notify();
                        });
                    }
                },
            ))
            .child(setting_option_button(
                "settings-startup-off",
                "Off",
                !current.startup_maximized,
                move |cx| {
                    source.update(cx, |view, cx| {
                        let mut next = view.settings.clone();
                        next.startup_maximized = false;
                        view.update_settings(next);
                        cx.notify();
                    });
                },
            ));

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
                            view.cli_install_error = result.err();
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
            .child(div().mt_1().text_sm().text_color(rgb(MUTED_TEXT)).child(
                "Changes apply immediately. Startup maximization applies on the next launch.",
            ))
            .child(div().h(px(12.0)))
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
                "Startup maximized",
                "Open the main editor window in its maximized state.",
                startup_options,
            ))
            .child(setting_row(
                "Image cache size",
                "Maximum unplaced Kitty Graphics Protocol image data kept in memory.",
                cache_options,
            ))
            .child(setting_row(
                "Paste shortcut",
                "Read the local system clipboard and paste it through Neovim.",
                paste_shortcut_options,
            ))
            .child(setting_row(
                "Command-line helper",
                "Install gpvim and gpvimdiff so files can be opened or compared from a terminal.",
                cli_options,
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

pub(super) struct AboutWindow;

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

fn setting_option_button(
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

fn logo_image() -> Arc<Image> {
    Arc::new(Image::from_bytes(
        gpui::ImageFormat::Png,
        include_bytes!("../../assets/icons/neovim-gpui.png").to_vec(),
    ))
}

fn setting_row(
    label: &'static str,
    description: &'static str,
    controls: impl IntoElement,
) -> impl IntoElement {
    div()
        .w_full()
        .flex()
        .flex_col()
        .items_start()
        .py_3()
        .border_b_1()
        .border_color(rgb(SURFACE_BRIGHT))
        .child(
            div()
                .flex_1()
                .pr_4()
                .child(div().text_base().child(label))
                .child(
                    div()
                        .mt_1()
                        .text_sm()
                        .text_color(rgb(MUTED_TEXT))
                        .child(description),
                ),
        )
        .child(div().w_full().mt_2().child(controls))
}

fn open_settings_window(source: Entity<NvimGpui>, cx: &mut App) {
    let existing = source.read(cx).settings_window;
    if let Some(handle) = existing {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }

    let bounds = Bounds::centered(None, size(px(720.0), px(560.0)), cx);
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(themed_titlebar_options("nvim-gpui settings")),
                kind: WindowKind::Floating,
                is_resizable: true,
                window_min_size: Some(size(px(560.0), px(420.0))),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| SettingsWindow::new(source.clone(), cx)),
        )
        .expect("failed to open nvim-gpui settings window");
    source.update(cx, |view, _| view.settings_window = Some(handle));
}

fn open_about_window(source: Entity<NvimGpui>, cx: &mut App) {
    let existing = source.read(cx).about_window;
    if let Some(handle) = existing {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }

    let bounds = Bounds::centered(None, size(px(440.0), px(320.0)), cx);
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(themed_titlebar_options("About nvim-gpui")),
                kind: WindowKind::Floating,
                is_resizable: false,
                ..Default::default()
            },
            |_, cx| cx.new(|_| AboutWindow),
        )
        .expect("failed to open nvim-gpui about window");
    source.update(cx, |view, _| view.about_window = Some(handle));
}

pub(super) fn is_monospace_family(window: &Window, family: &str, font_size: Pixels) -> bool {
    let text_system = window.text_system();
    let font_id = text_system.resolve_font(&font(family.to_owned()));
    let Some(reference) = text_system
        .advance(font_id, font_size, '0')
        .ok()
        .map(|advance| f32::from(advance.width))
    else {
        return false;
    };

    ['M', 'i', 'W', ' '].into_iter().all(|character| {
        text_system
            .advance(font_id, font_size, character)
            .ok()
            .map(|advance| (f32::from(advance.width) - reference).abs() <= 0.01)
            .unwrap_or(false)
    })
}

pub(super) fn parse_guifont_spec(spec: &str) -> GuiFontSpec {
    let first_font = spec.split(',').next().unwrap_or(spec);
    let mut parts = first_font.split(':');
    let family = parts.next().unwrap_or_default().replace("\\:", ":");
    let family = if family.trim().is_empty() {
        GuiFontSpec::default().family
    } else {
        family
    };
    let size = parts
        .find_map(|part| part.strip_prefix('h'))
        .and_then(|size| size.parse::<f32>().ok())
        .filter(|size| *size > 0.0)
        .unwrap_or(DEFAULT_GRID_FONT_SIZE);

    GuiFontSpec { family, size }
}

pub(super) fn line_height_from_metrics(
    glyph_height: Pixels,
    font_size: Pixels,
    linespace: f32,
) -> Pixels {
    let minimum_line_height = font_size * 1.2;

    // GPUI 0.2.2 does not expose the font's line-gap metric. Use the actual
    // glyph metrics and a compact 1.2em minimum cell height instead of
    // scaling a historical default ratio. Neovim's `linespace` remains the
    // only user-configured extra spacing.
    px(
        (f32::from(glyph_height.max(minimum_line_height)) + linespace)
            .ceil()
            .max(1.0),
    )
}

pub(super) fn parse_non_negative_float(value: &str) -> Option<f32> {
    let value = value.parse::<f32>().ok()?;
    value.is_finite().then_some(value.max(0.0))
}

pub(super) fn initial_window_size_for_grid(width: u32, height: u32) -> gpui::Size<Pixels> {
    let titlebar_height = if themed_titlebar_enabled() {
        THEMED_TITLEBAR_HEIGHT
    } else {
        0.0
    };
    size(
        px((width as f32 * DEFAULT_GRID_CELL_WIDTH).max(MIN_WINDOW_WIDTH)),
        px((height as f32 * DEFAULT_GRID_LINE_HEIGHT + titlebar_height).max(MIN_WINDOW_HEIGHT)),
    )
}

pub(super) fn themed_titlebar_enabled() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

pub(super) fn themed_titlebar_options(title: &'static str) -> TitlebarOptions {
    TitlebarOptions {
        title: Some(title.into()),
        appears_transparent: themed_titlebar_enabled(),
        ..Default::default()
    }
}

pub(super) fn themed_titlebar(
    title: String,
    background: u32,
    foreground: u32,
    source: Option<Entity<NvimGpui>>,
) -> impl IntoElement {
    let title_area = div()
        .flex_1()
        .h_full()
        .flex()
        .items_center()
        .justify_start()
        .pl(px(if cfg!(target_os = "macos") {
            76.0
        } else {
            12.0
        }))
        .text_color(rgb(foreground))
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |event, window, _cx| {
            if event.click_count == 2 {
                // On macOS this forwards to AppKit's standard titlebar
                // double-click action (normally zoom/maximize). On Windows,
                // WindowControlArea::Drag lets the native caption handling do
                // the same job, so this is harmless there.
                window.titlebar_double_click();
            }
        })
        .child(img(logo_image()).w(px(20.0)).h(px(20.0)))
        .child(div().w(px(6.0)))
        .child(title);

    let mut titlebar = div()
        .w_full()
        .h(px(THEMED_TITLEBAR_HEIGHT))
        .flex()
        .items_center()
        .bg(rgb(background))
        .child(title_area);

    if let Some(source) = source {
        let settings_source = source.clone();
        let about_source = source;
        let actions = div()
            .h_full()
            .flex()
            .items_center()
            .pr(px(8.0))
            .child(titlebar_button("Settings", foreground, move |cx| {
                open_settings_window(settings_source.clone(), cx);
            }))
            .child(div().w(px(4.0)))
            .child(titlebar_button("About", foreground, move |cx| {
                open_about_window(about_source.clone(), cx);
            }));
        titlebar = titlebar.child(actions);
    }

    #[cfg(target_os = "windows")]
    let titlebar = titlebar
        .child(window_control_button(
            "—",
            WindowControlArea::Min,
            background,
            foreground,
        ))
        .child(window_control_button(
            "□",
            WindowControlArea::Max,
            background,
            foreground,
        ))
        .child(window_control_button(
            "×",
            WindowControlArea::Close,
            background,
            foreground,
        ));

    titlebar
}

fn titlebar_button(
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
fn window_control_button(
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
