use super::*;

pub(crate) fn run(options: CliOptions) {
    let app_settings = settings::Settings::load();
    let startup_maximized = app_settings.startup_maximized;
    let nvim = match options.connection {
        NvimConnection::Embed => {
            let nvim_command = options
                .nvim_command
                .or_else(nvim::configured_nvim_command)
                .unwrap_or_else(|| OsString::from("nvim"));
            NvimProcess::spawn_with_command(
                DEFAULT_GRID_WIDTH,
                DEFAULT_GRID_HEIGHT,
                nvim_command,
                options.nvim_args,
            )
        }
        NvimConnection::Remote(address) => {
            NvimProcess::connect(DEFAULT_GRID_WIDTH, DEFAULT_GRID_HEIGHT, &address)
        }
    };
    let show_debug_window = options.debug_window;
    let initial_theme = nvim.as_ref().ok().and_then(NvimProcess::startup_theme);

    Application::new()
        .with_assets(AppAssets)
        .run(move |cx: &mut App| {
            let nerd_font_registered = match platform::register_bundled_fonts(cx) {
                Ok(()) => true,
                Err(error) => {
                    eprintln!("[font] {error}");
                    false
                }
            };
            if let Err(error) = platform::install_dock_icon() {
                eprintln!("[platform] {error}");
            }

            let main_bounds = Bounds::centered(
                None,
                initial_window_size_for_grid(DEFAULT_GRID_WIDTH, DEFAULT_GRID_HEIGHT),
                cx,
            );
            let debug_height = px(DEBUG_WINDOW_HEIGHT);
            let debug_y = if main_bounds.origin.y > debug_height + px(8.0) {
                main_bounds.origin.y - debug_height - px(8.0)
            } else {
                px(8.0)
            };
            let debug_bounds = Bounds::new(
                point(main_bounds.origin.x, debug_y),
                size(main_bounds.size.width, debug_height),
            );

            let nvim_view = cx.new(|cx| {
                NvimGpui::new(
                    nvim,
                    cx,
                    nerd_font_registered,
                    app_settings.clone(),
                    initial_theme,
                )
            });

            let main_window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(if startup_maximized {
                            WindowBounds::Maximized(main_bounds)
                        } else {
                            WindowBounds::Windowed(main_bounds)
                        }),
                        titlebar: Some(themed_titlebar_options(DEFAULT_WINDOW_TITLE)),
                        is_resizable: true,
                        window_min_size: Some(size(px(MIN_WINDOW_WIDTH), px(MIN_WINDOW_HEIGHT))),
                        ..Default::default()
                    },
                    |_, _| nvim_view.clone(),
                )
                .expect("failed to open nvim-gpui window");

            main_window
                .update(cx, |view, window, cx| {
                    window.on_window_should_close(cx, |_, cx| {
                        cx.quit();
                        true
                    });
                    view.window_bounds_subscription =
                        Some(cx.observe_window_bounds(window, |view, window, _cx| {
                            view.sync_nvim_size(window)
                        }));
                    view.sync_nvim_size(window);
                    if let Some(focus_handle) = view.focus_handle.as_ref() {
                        window.focus(focus_handle);
                    }
                })
                .expect("failed to focus nvim-gpui window");

            if show_debug_window {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(debug_bounds)),
                        titlebar: Some(themed_titlebar_options("nvim-gpui debug")),
                        kind: WindowKind::Floating,
                        focus: false,
                        is_resizable: false,
                        ..Default::default()
                    },
                    |window, cx| {
                        window.on_window_should_close(cx, |_, cx| {
                            cx.quit();
                            true
                        });
                        cx.new(|cx| DebugWindow::new(nvim_view.clone(), cx))
                    },
                )
                .expect("failed to open nvim-gpui debug window");
            }

            cx.activate(true);
        });
}
