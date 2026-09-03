use super::*;

impl NvimGpui {
    pub(crate) fn apply_nvim_event(&mut self, event: NvimEvent) {
        match event {
            NvimEvent::ApiReady {
                version,
                capabilities: _,
            } => {
                log::info!(
                    target: "nvim_gpui::state",
                    "Neovim API ready: version={version}, api_level={}",
                    version.api_level
                );
                self.api_level = Some(version.api_level);
                self.nvim_version = Some(version);
                self.rpc_status = format!("rpc: Neovim {version} / API {}", version.api_level);
            }
            NvimEvent::UiAttached { width, height } => {
                log::info!(
                    target: "nvim_gpui::state",
                    "Neovim UI attached: width={width}, height={height}"
                );
                self.rpc_status = format!("rpc: attached {width}×{height}");
            }
            NvimEvent::GridResized {
                grid,
                width,
                height,
            } => {
                log::debug!(
                    target: "nvim_gpui::state",
                    "grid resized: grid={grid}, width={width}, height={height}"
                );
                self.ime_coordinates_dirty = true;
                if grid == 1 {
                    self.pending_grid = Some(self.new_styled_grid(width as usize, height as usize));
                    self.grid_size = Some((width, height));
                } else {
                    self.pending_grid_mut_for(grid)
                        .resize(width as usize, height as usize);
                }
                self.pending_destroyed_grids.remove(&grid);
            }
            NvimEvent::GridLine {
                grid,
                row,
                col_start,
                cells,
                wraps_to_next,
            } => {
                self.pending_grid_mut_for(grid).apply_grid_line(
                    row as usize,
                    col_start as usize,
                    &cells,
                    wraps_to_next,
                );
            }
            NvimEvent::GridClear { grid } => {
                self.pending_grid_mut_for(grid).clear();
            }
            NvimEvent::GridDestroy { grid } => {
                log::debug!(target: "nvim_gpui::state", "grid destroyed: grid={grid}");
                if grid == 1 {
                    self.pending_grid_mut().destroy();
                    self.grid_size = None;
                } else {
                    self.pending_other_grids.remove(&grid);
                    self.pending_destroyed_grids.insert(grid);
                }
            }
            NvimEvent::GridCursorGoto { grid, row, col } => {
                // `grid_cursor_goto` belongs to the current redraw batch. Do
                // not expose it until `flush`, otherwise a partial redraw can
                // paint the cursor over a different, already committed grid.
                self.ime_coordinates_dirty = true;
                self.pending_cursor_grid = Some(grid);
                self.pending_grid_mut_for(grid)
                    .set_cursor(row as usize, col as usize);
            }
            NvimEvent::DefaultColorsSet {
                foreground,
                background,
                special,
            } => {
                let theme = self.pending_theme_mut();
                theme.default_foreground = foreground;
                theme.default_background = background;
                self.set_default_colors_on_all_grids(foreground, background, special);
            }
            NvimEvent::HlAttrDefine { id, attrs } => {
                let theme = self.pending_theme_mut();
                match attrs.ui_name.as_deref() {
                    Some("Normal") => {
                        theme.normal_foreground = attrs.foreground;
                        theme.normal_background = attrs.background;
                    }
                    Some("NormalFloat") => {
                        theme.normal_float_background = attrs.background;
                    }
                    _ => {}
                }
                self.set_highlight_on_all_grids(id, attrs);
            }
            NvimEvent::GridScroll {
                grid,
                top,
                bot,
                left,
                right,
                rows,
                cols,
            } => {
                self.ime_coordinates_dirty = true;
                self.pending_grid_mut_for(grid).scroll(
                    top as usize,
                    bot as usize,
                    left as usize,
                    right as usize,
                    rows as isize,
                    cols as isize,
                );
            }
            NvimEvent::WinPos {
                grid,
                win: _,
                row,
                col,
                width,
                height,
            } => {
                self.ime_coordinates_dirty = true;
                let mut placement = self.grid_placement(grid);
                placement.row = row as i64;
                placement.col = col as i64;
                placement.width = width;
                placement.height = height;
                placement.z_index = 0;
                placement.compindex = -1;
                placement.mouse_enabled = true;
                placement.visible = true;
                self.set_grid_placement(grid, placement);
            }
            NvimEvent::WinFloatPos {
                grid,
                win: _,
                anchor: _,
                anchor_grid: _,
                anchor_row: _,
                anchor_col: _,
                mouse_enabled,
                zindex,
                compindex,
                screen_row,
                screen_col,
            } => {
                self.ime_coordinates_dirty = true;
                let mut placement = self.grid_placement(grid);
                placement.row = screen_row;
                placement.col = screen_col;
                placement.z_index = zindex;
                placement.compindex = compindex;
                placement.mouse_enabled = mouse_enabled;
                placement.visible = true;
                self.set_grid_placement(grid, placement);
            }
            NvimEvent::WinViewport {
                grid,
                win: _,
                topline,
                botline,
                curline,
                curcol,
                line_count,
                scroll_delta,
            } => {
                let mut placement = self.grid_placement(grid);
                placement.viewport = Some(GridViewport {
                    topline,
                    botline,
                    curline,
                    curcol,
                    line_count,
                    scroll_delta,
                });
                self.set_grid_placement(grid, placement);
            }
            NvimEvent::WinViewportMargins {
                grid,
                win: _,
                top,
                bottom,
                left,
                right,
            } => {
                self.ime_coordinates_dirty = true;
                let mut placement = self.grid_placement(grid);
                placement.viewport_margins = Some(GridViewportMargins {
                    top,
                    bottom,
                    left,
                    right,
                });
                self.set_grid_placement(grid, placement);
            }
            NvimEvent::MsgSetPos {
                grid,
                row,
                scrolled,
                sep_char,
                zindex,
                compindex,
            } => {
                self.ime_coordinates_dirty = true;
                // A message grid is not associated with a normal window, so
                // Neovim positions it with msg_set_pos instead of win_pos.
                // Keep it in the same placement table as window grids so its
                // grid_line updates become visible and participate in the
                // protocol compositing order.
                let grid_width = self.pending_grid_mut_for(grid).width() as u64;
                let mut placement = self.grid_placement(grid);
                placement.row = row as i64;
                placement.col = 0;
                placement.width = grid_width;
                placement.z_index = zindex;
                placement.compindex = compindex;
                placement.visible = true;
                placement.message_scrolled = scrolled;
                placement.message_separator = sep_char.chars().next();
                self.set_grid_placement(grid, placement);
            }
            NvimEvent::WinExternalPos { grid, win: _ } => {
                self.ime_coordinates_dirty = true;
                let mut placement = self.grid_placement(grid);
                placement.visible = false;
                self.set_grid_placement(grid, placement);
            }
            NvimEvent::WinHide { grid } => {
                self.ime_coordinates_dirty = true;
                let mut placement = self.grid_placement(grid);
                placement.visible = false;
                self.set_grid_placement(grid, placement);
            }
            NvimEvent::WinClose { grid } => {
                self.ime_coordinates_dirty = true;
                if grid == 1 {
                    self.pending_grid_mut().destroy();
                    self.grid_size = None;
                } else {
                    self.pending_other_grids.remove(&grid);
                    self.pending_destroyed_grids.insert(grid);
                }
            }
            NvimEvent::OptionSet { name, value } => {
                self.ui_options.insert(name.clone(), value.clone());
                match name.as_str() {
                    "mouse" => {
                        self.mouse_option = value;
                        self.mouse_enabled = self.mouse_option_allows_current_mode();
                    }
                    "guifont" => {
                        self.ime_coordinates_dirty = true;
                        self.guifont = Some(value);
                        self.resolved_grid_font = None;
                        self.resolved_grid_wide_font = None;
                        self.last_resize = None;
                        self.shaping_cache.borrow_mut().clear();
                    }
                    "guifontwide" => {
                        self.ime_coordinates_dirty = true;
                        self.guifontwide = Some(value);
                        self.resolved_grid_wide_font = None;
                        self.last_resize = None;
                        self.shaping_cache.borrow_mut().clear();
                    }
                    "linespace" => {
                        self.ime_coordinates_dirty = true;
                        self.linespace = parse_non_negative_float(&value).unwrap_or(0.0);
                        self.last_resize = None;
                    }
                    "arabicshape" | "ambiwidth" | "emoji" | "termguicolors" => {
                        self.shaping_cache.borrow_mut().clear();
                    }
                    _ => {}
                }
            }
            NvimEvent::SetTitle { title } => {
                if !title.is_empty() {
                    self.window_title = title;
                }
            }
            NvimEvent::SetIcon { icon } => {
                self.window_icon = icon;
            }
            NvimEvent::ModeInfoSet {
                cursor_style_enabled,
                modes,
            } => {
                self.cursor_style_enabled = cursor_style_enabled;
                self.cursor_modes = modes;
                self.cursor_blink_started_at = Instant::now();
            }
            NvimEvent::ModeChanged { mode, mode_idx } => {
                self.ime_coordinates_dirty = true;
                self.input_router.set_nvim_mode(&mode);
                log::info!(
                    target: "nvim_gpui::state",
                    "Neovim mode changed: mode={mode}, mode_idx={mode_idx}, input_target={:?}",
                    self.input_router.target()
                );
                if self.input_router.target() != InputTarget::SystemIme {
                    self.system_ime.clear();
                }
                self.state.mode = mode.to_ascii_uppercase();
                self.nvim_mode = mode;
                self.mouse_enabled = self.mouse_option_allows_current_mode();
                self.cursor_mode_index = mode_idx as usize;
                self.cursor_blink_started_at = Instant::now();
            }
            NvimEvent::UiSend { data } => self.apply_ui_send(&data),
            NvimEvent::MouseEnabled(enabled) => {
                log::debug!(
                    target: "nvim_gpui::state",
                    "Neovim mouse input enabled: {enabled}"
                );
                self.mouse_enabled = enabled;
            }
            NvimEvent::Flush => {
                self.commit_pending_grid();
                self.commit_pending_theme();
                self.ime_coordinates_dirty = true;
                self.startup_flush_seen = true;
                self.update_startup_grid_ready();
            }
            NvimEvent::Error(error) => {
                log::error!(target: "nvim_gpui::state", "Neovim event error: {error}");
                self.rpc_status = format!("rpc error: {error}");
            }
            NvimEvent::Disconnected { reason } => {
                log::info!(
                    target: "nvim_gpui::state",
                    "Neovim disconnected: reason={reason:?}"
                );
                self.rpc_status = "rpc: disconnected".to_owned();
            }
        }
    }

    pub(super) fn apply_ui_send(&mut self, data: &str) {
        let events = self.image_store.consume_ui_data(
            data,
            GridId(self.pending_cursor_grid.unwrap_or(self.cursor_grid)),
        );
        for event in events {
            match event {
                KittyEvent::AssetUpdated { image, .. } => {
                    if let Some(asset) = self.image_store.asset(image) {
                        let source =
                            Image::from_bytes(asset.format.gpui_format(), asset.encoded.clone());
                        self.image_sources.insert(image, Arc::new(source));
                    }
                }
                KittyEvent::AssetDeleted { image } => {
                    self.image_sources.remove(&image);
                }
                KittyEvent::AssetsCleared => {
                    self.image_sources.clear();
                }
                KittyEvent::TerminalResponse(response) => {
                    if let Some(nvim) = self.nvim.as_ref() {
                        if let Err(error) = nvim.send_term_event("termresponse", response) {
                            log::error!(
                                target: "nvim_gpui::nvim",
                                "failed to forward Neovim terminal response: {error}"
                            );
                            self.rpc_status = format!("rpc terminal response error: {error}");
                        }
                    }
                }
            }
        }
    }
}
