use super::*;
use std::cell::RefCell;

pub(crate) fn run(
    options: CliOptions,
    app_settings: settings::Settings,
    logger: Option<flexi_logger::LoggerHandle>,
) {
    let startup_maximized = app_settings.startup_maximized;
    log::debug!(
        target: "nvim_gpui::startup",
        "loaded settings: nerd_font={}, fallback_mode={}, startup_maximized={}, quit_on_window_close={}, allow_multiple_instances={}, log_level={}, image_cache_size_mb={}",
        app_settings.nerd_font.key(),
        app_settings.fallback_mode.key(),
        app_settings.startup_maximized,
        app_settings.quit_on_window_close,
        app_settings.allow_multiple_instances,
        app_settings.log_level.key(),
        app_settings.image_cache_size_mb
    );
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
    if let Err(error) = &nvim {
        log::error!(target: "nvim_gpui::startup", "Neovim initialization failed: {error}");
    }
    let show_debug_window = options.debug_window;
    let initial_theme = nvim.as_ref().ok().and_then(NvimProcess::startup_theme);
    let reopen_view = Rc::new(RefCell::new(None));
    let reopen_view_for_handler = reopen_view.clone();

    log::info!(target: "nvim_gpui::startup", "starting GPUI application");
    let application = Application::new().with_assets(AppAssets);
    application.on_reopen(move |cx| {
        let Some(view) = reopen_view_for_handler.borrow().clone() else {
            return;
        };
        if let Err(error) = open_main_window(view, startup_maximized, cx) {
            log::error!(target: "nvim_gpui::startup", "failed to reopen main window: {error}");
            return;
        }
        log::info!(target: "nvim_gpui::startup", "main window reopened");
        cx.activate(true);
    });
    application.run(move |cx: &mut App| {
            let nerd_font_registered = match platform::register_bundled_fonts(cx) {
                Ok(()) => true,
                Err(error) => {
                    log::error!(target: "nvim_gpui::startup", "bundled font registration failed: {error}");
                    eprintln!("[font] {error}");
                    false
                }
            };
            if let Err(error) = platform::install_dock_icon() {
                log::warn!(target: "nvim_gpui::startup", "dock icon installation failed: {error}");
                eprintln!("[platform] {error}");
            }

            let nvim_view = cx.new(|cx| {
                NvimGpui::new(
                    nvim,
                    cx,
                    nerd_font_registered,
                    app_settings.clone(),
                    initial_theme,
                    logger,
                )
            });
            *reopen_view.borrow_mut() = Some(nvim_view.clone());

            open_main_window(nvim_view.clone(), startup_maximized, cx)
                .expect("failed to open nvim-gpui window");

            if show_debug_window {
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

                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(debug_bounds)),
                        titlebar: Some(themed_titlebar_options("nvim-gpui debug")),
                        kind: WindowKind::Floating,
                        focus: false,
                        is_resizable: false,
                        ..Default::default()
                    },
                    |_, cx| cx.new(|cx| DebugWindow::new(nvim_view.clone(), cx)),
                )
                .expect("failed to open nvim-gpui debug window");
            }

        cx.activate(true);
    });
}

fn open_main_window(
    nvim_view: Entity<NvimGpui>,
    startup_maximized: bool,
    cx: &mut App,
) -> gpui::Result<WindowHandle<NvimGpui>> {
    let bounds = Bounds::centered(
        None,
        initial_window_size_for_grid(DEFAULT_GRID_WIDTH, DEFAULT_GRID_HEIGHT),
        cx,
    );
    let close_guard_view = nvim_view.downgrade();
    let main_window = cx.open_window(
        WindowOptions {
            window_bounds: Some(if startup_maximized {
                WindowBounds::Maximized(bounds)
            } else {
                WindowBounds::Windowed(bounds)
            }),
            titlebar: Some(themed_titlebar_options(DEFAULT_WINDOW_TITLE)),
            is_resizable: true,
            window_min_size: Some(size(px(MIN_WINDOW_WIDTH), px(MIN_WINDOW_HEIGHT))),
            ..Default::default()
        },
        move |_, _| nvim_view.clone(),
    )?;

    main_window.update(cx, move |view, window, cx| {
        window.on_window_should_close(cx, move |_, cx| {
            let should_quit = close_guard_view
                .upgrade()
                .map(|view| view.read(cx).should_quit_on_window_close())
                .unwrap_or(true);
            if should_quit {
                cx.quit();
            }
            true
        });
        view.window_bounds_subscription =
            Some(cx.observe_window_bounds(window, |view, window, _cx| view.sync_nvim_size(window)));
        view.sync_nvim_size(window);
        if let Some(focus_handle) = view.focus_handle.as_ref() {
            window.focus(focus_handle);
        }
    })?;

    log::info!(target: "nvim_gpui::startup", "main window opened");
    Ok(main_window)
}
