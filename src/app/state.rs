use super::*;

mod events;
mod grid_state;
mod layers;
mod lifecycle;

pub(super) struct PendingRedrawState {
    ui_options: HashMap<String, String>,
    display_options: grid::DisplayOptions,
    guifont: Option<String>,
    guifontwide: Option<String>,
    window_title: String,
    window_icon: String,
    mouse_option: String,
    mouse_enabled: bool,
    nvim_mode: String,
    linespace: f32,
    cursor_style_enabled: bool,
    cursor_modes: Vec<grid::CursorModeInfo>,
    cursor_mode_index: usize,
    cursor_blink_started_at: Instant,
    input_router: InputRouter,
    editor_mode: String,
}

impl NvimGpui {
    fn begin_pending_redraw(&mut self) {
        if self.pending_redraw.is_some() {
            return;
        }

        self.pending_redraw = Some(PendingRedrawState {
            ui_options: self.ui_options.clone(),
            display_options: self.display_options,
            guifont: self.guifont.clone(),
            guifontwide: self.guifontwide.clone(),
            window_title: self.window_title.clone(),
            window_icon: self.window_icon.clone(),
            mouse_option: self.mouse_option.clone(),
            mouse_enabled: self.mouse_enabled,
            nvim_mode: self.nvim_mode.clone(),
            linespace: self.linespace,
            cursor_style_enabled: self.cursor_style_enabled,
            cursor_modes: self.cursor_modes.clone(),
            cursor_mode_index: self.cursor_mode_index,
            cursor_blink_started_at: self.cursor_blink_started_at,
            input_router: self.input_router,
            editor_mode: self.state.mode.clone(),
        });
    }

    fn pending_redraw_mut(&mut self) -> &mut PendingRedrawState {
        self.begin_pending_redraw();
        self.pending_redraw
            .as_mut()
            .expect("pending redraw was initialized")
    }

    pub(super) fn commit_pending_redraw(&mut self) {
        let Some(pending) = self.pending_redraw.take() else {
            return;
        };

        let guifont_changed = self.guifont != pending.guifont;
        let guifontwide_changed = self.guifontwide != pending.guifontwide;
        let linespace_changed = (self.linespace - pending.linespace).abs() > f32::EPSILON;
        let display_options_changed = self.display_options != pending.display_options;

        self.ui_options = pending.ui_options;
        self.display_options = pending.display_options;
        self.guifont = pending.guifont;
        self.guifontwide = pending.guifontwide;
        self.window_title = pending.window_title;
        self.window_icon = pending.window_icon;
        self.mouse_option = pending.mouse_option;
        self.mouse_enabled = pending.mouse_enabled;
        self.nvim_mode = pending.nvim_mode;
        self.linespace = pending.linespace;
        self.cursor_style_enabled = pending.cursor_style_enabled;
        self.cursor_modes = pending.cursor_modes;
        self.cursor_mode_index = pending.cursor_mode_index;
        self.cursor_blink_started_at = pending.cursor_blink_started_at;
        self.input_router = pending.input_router;
        self.state.mode = pending.editor_mode;

        if self.input_router.target() != InputTarget::Rime {
            self.reset_rime_composition();
        }
        if self.input_router.target() != InputTarget::SystemIme {
            self.system_ime.clear();
        }
        if guifont_changed {
            self.resolved_grid_font = None;
            self.resolved_grid_wide_font = None;
            self.ime_coordinates_dirty = true;
            self.last_resize = None;
        }
        if guifontwide_changed {
            self.resolved_grid_wide_font = None;
            self.ime_coordinates_dirty = true;
            self.last_resize = None;
        }
        if linespace_changed {
            self.ime_coordinates_dirty = true;
            self.last_resize = None;
        }
        if display_options_changed {
            self.shaping_cache.borrow_mut().clear();
        }
    }

    pub(super) fn apply_runtime_settings(&mut self) {
        self.nerd_font_family = self
            .bundled_nerd_font_registered
            .then(|| self.settings.nerd_font.family().to_owned());
        self.shaping_cache.borrow_mut().clear();
        self.glyph_coverage_cache.borrow_mut().clear();
        for image in self
            .image_store
            .set_cache_size_mb(self.settings.image_cache_size_mb)
        {
            self.image_sources.remove(&image);
        }
    }

    pub(crate) fn update_settings(&mut self, next: settings::Settings) {
        let ime_backend_changed = self.settings.ime_backend != next.ime_backend;
        if self.settings.log_level != next.log_level {
            if let Some(logger) = self.logger.as_ref() {
                crate::logging::set_level(logger, next.log_level);
            }
        }
        self.settings = next;
        self.apply_runtime_settings();
        if ime_backend_changed {
            self.apply_ime_backend_setting();
        }
        self.settings_save_error = self.settings.save().err();
    }

    pub(super) fn apply_ime_backend_setting(&mut self) {
        let rime_enabled =
            self.settings.ime_backend == settings::ImeBackend::Rime && self.rime_backend.is_some();
        let mut config = self.input_router.config();
        config.rime_enabled = rime_enabled;
        self.input_router.set_config(config);
        if let Some(pending) = self.pending_redraw.as_mut() {
            pending.input_router.set_config(config);
        }
        if !rime_enabled {
            self.reset_rime_composition();
            self.rime_menu_open = false;
            self.rime_menu_message = None;
        }
        self.system_ime.clear();
    }

    pub(super) fn current_grid_font(&mut self, window: &Window) -> GuiFontSpec {
        if let Some(font) = &self.resolved_grid_font {
            return font.clone();
        }

        let font = self
            .guifont
            .as_deref()
            .filter(|spec| !spec.trim().is_empty())
            .map(parse_guifont_spec)
            .unwrap_or_else(|| GuiFontSpec::system(window));
        self.resolved_grid_font = Some(font.clone());
        font
    }

    pub(super) fn current_grid_wide_font(&mut self, window: &Window) -> GuiFontSpec {
        if let Some(font) = &self.resolved_grid_wide_font {
            return font.clone();
        }

        let font = if let Some(spec) = self
            .guifontwide
            .as_deref()
            .filter(|spec| !spec.trim().is_empty())
        {
            parse_guifont_spec(spec)
        } else {
            self.current_grid_font(window)
        };
        self.resolved_grid_wide_font = Some(font.clone());
        font
    }

    pub(super) fn current_cursor_mode(&self) -> grid::CursorModeInfo {
        if !self.cursor_style_enabled {
            return grid::CursorModeInfo::default();
        }
        self.cursor_modes
            .get(self.cursor_mode_index)
            .copied()
            .unwrap_or_default()
    }

    pub(super) fn theme_background(&self) -> u32 {
        self.theme
            .normal_background
            .or(self.theme.default_background)
            .unwrap_or(BACKGROUND)
    }

    pub(super) fn theme_foreground(&self) -> u32 {
        self.theme
            .normal_foreground
            .or(self.theme.default_foreground)
            .unwrap_or(TEXT)
    }

    pub(super) fn pending_theme_mut(&mut self) -> &mut NvimTheme {
        self.pending_theme.get_or_insert(self.theme)
    }

    pub(super) fn commit_pending_theme(&mut self) {
        if let Some(theme) = self.pending_theme.take() {
            self.theme = theme;
        }
    }

    pub(super) fn update_startup_grid_ready(&mut self) {
        if self.nvim_grid_ready || !self.startup_flush_seen {
            return;
        }

        let Some(target) = self.startup_resize_target else {
            return;
        };
        let committed_size = (self.grid.width() as u32, self.grid.height() as u32);
        if committed_size == target {
            self.nvim_grid_ready = true;
        }
    }

    pub(super) fn sync_nvim_size(&mut self, window: &mut Window) {
        let gui_font = self.current_grid_font(window);
        let cell_width = gui_font.cell_width(window);
        let line_height = gui_font.line_height(window, self.linespace);
        let viewport = window.viewport_size();
        let available_height = f32::from(viewport.height)
            - if themed_titlebar_enabled() {
                THEMED_TITLEBAR_HEIGHT
            } else {
                0.0
            };
        let width = (f32::from(viewport.width) / f32::from(cell_width))
            .floor()
            .max(1.0) as u32;
        let height = (available_height / f32::from(line_height)).floor().max(1.0) as u32;
        let size = (width, height);

        if !self.nvim_grid_ready {
            self.startup_resize_target = Some(size);
            self.update_startup_grid_ready();
            if self.nvim_grid_ready {
                return;
            }
        }

        let Some(nvim) = self.nvim.as_ref() else {
            return;
        };

        if self.last_resize == Some(size) {
            return;
        }

        match nvim.send_resize(width, height) {
            Ok(()) => {
                log::debug!(
                    target: "nvim_gpui::state",
                    "sent Neovim resize: width={}, height={}",
                    width,
                    height
                );
                self.last_resize = Some(size);
            }
            Err(error) => {
                log::error!(target: "nvim_gpui::state", "Neovim resize failed: {error}");
                self.rpc_status = format!("rpc resize error: {error}");
            }
        }
    }

    fn paste_from_system_clipboard(&mut self, cx: &mut Context<Self>) {
        let text = match crate::clipboard::paste_text(cx) {
            Ok(text) => text,
            Err(error) => {
                log::warn!(
                    target: "nvim_gpui::clipboard",
                    "could not read system clipboard for paste: {error}"
                );
                return;
            }
        };
        let Some(nvim) = self.nvim.as_ref() else {
            log::warn!(
                target: "nvim_gpui::clipboard",
                "ignoring paste because Neovim is unavailable"
            );
            return;
        };
        let response = match nvim.send_paste(text) {
            Ok(response) => response,
            Err(error) => {
                log::error!(
                    target: "nvim_gpui::clipboard",
                    "failed to queue system paste: {error}"
                );
                self.rpc_status = format!("rpc paste error: {error}");
                return;
            }
        };
        cx.spawn(async move |_weak, _cx| match response.recv().await {
            Ok(Ok(rmpv::Value::Boolean(false))) => log::warn!(
                target: "nvim_gpui::clipboard",
                "Neovim rejected system paste"
            ),
            Ok(Ok(_)) => {}
            Ok(Err(error)) => log::error!(
                target: "nvim_gpui::clipboard",
                "system paste failed in Neovim: {error}"
            ),
            Err(error) => log::error!(
                target: "nvim_gpui::clipboard",
                "system paste response was lost: {error}"
            ),
        })
        .detach();
    }

    fn reset_rime_composition(&mut self) {
        self.rime_context = None;
        if let Some(backend) = self.rime_backend.as_ref() {
            if let Err(error) = backend.clear_composition() {
                log::debug!(target: "nvim_gpui::rime", "could not clear Rime composition: {error}");
            }
        }
    }

    fn disable_rime(&mut self, reason: &str) {
        self.reset_rime_composition();
        self.rime_backend = None;
        self.rime_menu_open = false;
        self.rime_menu_message = None;
        let mut config = self.input_router.config();
        config.rime_enabled = false;
        self.input_router.set_config(config);
        log::warn!(target: "nvim_gpui::rime", "Rime disabled: {reason}");
    }

    pub(super) fn toggle_rime(&mut self, cx: &mut Context<Self>) {
        if self.rime_backend.is_none() {
            log::debug!(target: "nvim_gpui::rime", "Rime toggle ignored because the backend is unavailable");
            return;
        }

        self.rime_menu_open = false;
        self.rime_menu_message = None;
        let enabled = !self.input_router.config().rime_enabled;
        let mut config = self.input_router.config();
        config.rime_enabled = enabled;
        self.input_router.set_config(config);
        if let Some(pending) = self.pending_redraw.as_mut() {
            pending.input_router.set_config(config);
        }
        self.reset_rime_composition();
        self.system_ime.clear();
        log::info!(target: "nvim_gpui::rime", "Rime {}", if enabled { "enabled" } else { "disabled" });
        cx.notify();
    }

    pub(super) fn open_rime_menu(&mut self, cx: &mut Context<Self>) {
        if self.rime_backend.is_none() {
            return;
        }
        self.rime_menu_open = true;
        self.rime_menu_message = None;
        cx.notify();
    }

    pub(super) fn close_rime_menu(&mut self, cx: &mut Context<Self>) {
        if !self.rime_menu_open && self.rime_menu_message.is_none() {
            return;
        }
        self.rime_menu_open = false;
        self.rime_menu_message = None;
        cx.notify();
    }

    fn send_rime_commit(&mut self, text: String, cx: &mut Context<Self>) -> Result<(), String> {
        let Some(nvim) = self.nvim.as_ref() else {
            return Err("Neovim is unavailable".to_owned());
        };
        let bytes = text.len();
        let response = nvim.send_paste(text)?;
        log::debug!(
            target: "nvim_gpui::rime",
            "forwarding Rime commit through nvim_paste: bytes={bytes}"
        );
        cx.spawn(async move |_weak, _cx| match response.recv().await {
            Ok(Ok(rmpv::Value::Boolean(false))) => log::warn!(
                target: "nvim_gpui::rime",
                "Neovim rejected Rime commit"
            ),
            Ok(Ok(value)) => log::debug!(
                target: "nvim_gpui::rime",
                "Neovim accepted Rime commit: {value:?}"
            ),
            Ok(Err(error)) => log::error!(
                target: "nvim_gpui::rime",
                "Rime commit failed in Neovim: {error}"
            ),
            Err(error) => log::error!(
                target: "nvim_gpui::rime",
                "Rime commit response was lost: {error}"
            ),
        })
        .detach();
        Ok(())
    }

    fn handle_rime_keycode(
        &mut self,
        keycode: i32,
        modifiers: i32,
        reset_on_unconsumed: bool,
        cx: &mut Context<Self>,
    ) -> Result<bool, String> {
        let Some(backend) = self.rime_backend.as_ref() else {
            return Ok(false);
        };
        let consumed = backend.process_key(keycode, modifiers)?;
        let context = backend.context()?;
        let commit = backend.take_commit()?;
        log::debug!(
            target: "nvim_gpui::rime",
            "processed key: keycode={keycode}, consumed={consumed}, has_commit={}",
            commit.is_some()
        );
        if !consumed {
            // A key can be left unconsumed while librime still has commit
            // text buffered (for example, when punctuation commits the
            // candidate). Forward that commit first, then return false so
            // the original key continues through Neovim's normal path.
            if let Some(text) = commit {
                log::debug!(
                    target: "nvim_gpui::rime",
                    "Rime produced a commit before forwarding an unconsumed key: bytes={}",
                    text.len()
                );
                self.send_rime_commit(text, cx)?;
            }
            if reset_on_unconsumed {
                self.reset_rime_composition();
            }
            cx.notify();
            return Ok(false);
        }
        self.rime_context =
            (!context.preedit.is_empty() || !context.candidates.is_empty()).then_some(context);
        if let Some(text) = commit {
            log::debug!(
                target: "nvim_gpui::rime",
                "Rime produced a commit: bytes={}",
                text.len()
            );
            self.rime_context = None;
            self.send_rime_commit(text, cx)?;
        }
        cx.notify();
        Ok(true)
    }

    fn handle_rime_key(
        &mut self,
        keystroke: &gpui::Keystroke,
        cx: &mut Context<Self>,
    ) -> Result<bool, String> {
        let Some((keycode, modifiers)) = rime_key_event(keystroke) else {
            log::debug!(
                target: "nvim_gpui::rime",
                "Rime key mapping rejected: key={:?}, key_char={:?}, modifiers={:?}",
                keystroke.key,
                keystroke.key_char,
                keystroke.modifiers
            );
            self.reset_rime_composition();
            return Ok(false);
        };
        self.handle_rime_keycode(keycode, modifiers, true, cx)
    }

    pub(super) fn on_modifiers_changed(
        &mut self,
        event: &gpui::ModifiersChangedEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = self.last_modifiers;
        self.last_modifiers = event.modifiers;

        if self.input_router.target() != InputTarget::Rime || self.rime_backend.is_none() {
            return;
        }

        // GPUI represents modifier-only input as an aggregate state change.
        // Reconstruct the individual press/release events so Rime can run
        // bindings such as its press-and-release Shift ASCII switcher.
        let transitions = [
            ("shift", previous.shift, event.modifiers.shift),
            ("control", previous.control, event.modifiers.control),
            ("alt", previous.alt, event.modifiers.alt),
            ("platform", previous.platform, event.modifiers.platform),
        ];
        for (key, was_pressed, is_pressed) in transitions {
            if was_pressed == is_pressed {
                continue;
            }
            let Some((keycode, modifiers)) =
                rime_modifier_transition(key, previous, event.modifiers)
            else {
                continue;
            };
            match self.handle_rime_keycode(keycode, modifiers, false, cx) {
                Ok(true) => window.prevent_default(),
                Ok(false) => log::debug!(
                    target: "nvim_gpui::rime",
                    "Rime did not consume modifier transition: key={key}, pressed={is_pressed}"
                ),
                Err(error) => {
                    self.disable_rime(&error);
                    break;
                }
            }
        }
    }

    pub(super) fn on_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(
            &self.quit_dialog,
            QuitDialogState::Hidden | QuitDialogState::Quitting
        ) {
            match (&self.quit_dialog, event.keystroke.key.as_str()) {
                (QuitDialogState::Confirm { .. }, "escape") => {
                    self.cancel_quit_dialog(cx);
                }
                (QuitDialogState::Confirm { .. }, "enter" | "return") => {
                    self.save_and_quit(cx);
                }
                _ => {}
            }
            window.prevent_default();
            cx.stop_propagation();
            return;
        }

        if self.settings.ime_backend == settings::ImeBackend::Rime
            && self.settings.rime_toggle_shortcut.matches(&event.keystroke)
            && self.rime_backend.is_some()
        {
            self.toggle_rime(cx);
            window.prevent_default();
            cx.stop_propagation();
            return;
        }

        if self.settings.paste_shortcut.matches(&event.keystroke) {
            self.paste_from_system_clipboard(cx);
            window.prevent_default();
            return;
        }

        let mut target = self.input_router.target();
        log::debug!(
            target: "nvim_gpui::input",
            "key down: key={:?}, key_char={:?}, modifiers={:?}, target={target:?}",
            event.keystroke.key,
            event.keystroke.key_char,
            event.keystroke.modifiers
        );
        if target == InputTarget::Rime {
            match self.handle_rime_key(&event.keystroke, cx) {
                Ok(true) => {
                    window.prevent_default();
                    return;
                }
                Ok(false) => target = InputTarget::Neovim,
                Err(error) => {
                    self.disable_rime(&error);
                    target = InputTarget::Neovim;
                }
            }
        }
        if !should_route_key_to_neovim(target, &event.keystroke) {
            return;
        }

        if let Some(nvim) = self.nvim.as_ref() {
            if let Err(error) = nvim.send_input(key_to_nvim_input(&event.keystroke)) {
                log::error!(target: "nvim_gpui::input", "key event failed: {error}");
                self.rpc_status = format!("rpc input error: {error}");
            }
        }
        // Prevent GPUI's default key action from competing with Neovim for
        // editor-owned shortcuts such as Ctrl-W, Cmd-W, and function keys.
        // This only applies after the event reaches this window; OS-global
        // shortcuts remain owned by the operating system.
        window.prevent_default();
    }
}
