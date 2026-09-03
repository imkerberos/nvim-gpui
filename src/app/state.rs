use super::*;

mod events;
mod grid_state;
mod layers;
mod lifecycle;

impl NvimGpui {
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

    pub(super) fn update_settings(&mut self, next: settings::Settings) {
        self.settings = next;
        self.apply_runtime_settings();
        self.settings_save_error = self.settings.save().err();
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

    pub(super) fn on_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings.paste_shortcut.matches(&event.keystroke) {
            self.paste_from_system_clipboard(cx);
            window.prevent_default();
            return;
        }

        let target = self.input_router.target();
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
