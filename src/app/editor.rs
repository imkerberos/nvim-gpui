use super::*;

impl Focusable for NvimGpui {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle
            .clone()
            .expect("NvimGpui focus handle is initialized for app entities")
    }
}

impl EntityInputHandler for NvimGpui {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let (text, actual_range) = self.system_ime.text_for_range(range_utf16);
        adjusted_range.replace(actual_range);
        Some(text)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<gpui::UTF16Selection> {
        Some(self.system_ime.selected_text_range())
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.system_ime.marked_text_range()
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        // The local buffer only represents the active composition. Once the
        // platform cancels its marked range, there is no text to retain here.
        self.system_ime.clear();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_router.target() != InputTarget::SystemIme {
            return;
        }

        self.system_ime.replace_text(range, text);
        if !text.is_empty() {
            if let Some(nvim) = self.nvim.as_ref() {
                if let Err(error) = nvim.send_input(text.to_owned()) {
                    self.rpc_status = format!("rpc input error: {error}");
                }
            }
        }
        self.system_ime.clear();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_router.target() == InputTarget::SystemIme {
            self.system_ime
                .replace_and_mark_text(range, new_text, new_selected_range);
            cx.notify();
        }
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<gpui::Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<gpui::Pixels>> {
        let cursor = self
            .grid
            .cursor()
            .unwrap_or(grid::GridCursor { row: 0, col: 0 });
        let font_spec = self.current_grid_font(window);
        let cell_width = font_spec.cell_width(window);
        let line_height = font_spec.line_height(window, self.linespace);
        let origin = gpui::point(
            element_bounds.origin.x + cell_width * cursor.col,
            element_bounds.origin.y + line_height * cursor.row,
        );
        Some(Bounds::new(origin, size(cell_width, line_height)))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let font_spec = self.current_grid_font(window);
        let cell_width = font_spec.cell_width(window);
        let column = (f32::from(point.x) / f32::from(cell_width))
            .max(0.0)
            .floor() as usize;
        let byte_offset = self
            .system_ime
            .text()
            .char_indices()
            .nth(column)
            .map(|(offset, _)| offset)
            .unwrap_or(self.system_ime.text().len());
        Some(input::utf8_to_utf16_offset(
            self.system_ime.text(),
            byte_offset,
        ))
    }
}

impl Render for NvimGpui {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_window_title(&self.window_title);
        self.sync_nvim_size(window);

        let gui_font = self.current_grid_font(window);
        let gui_wide_font = self.current_grid_wide_font(window);
        let line_height = gui_font.line_height(window, self.linespace);
        let shaping_cache = Rc::clone(&self.shaping_cache);
        let cursor_mode = self.current_cursor_mode();
        let cursor_blink_started_at = self.cursor_blink_started_at;
        let theme_background = self.theme_background();
        let theme_foreground = self.theme_foreground();

        let entity = cx.entity();
        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(theme_background))
            .text_color(rgb(theme_foreground))
            .capture_key_down(cx.listener(Self::on_key_down));

        if let Some(focus_handle) = self.focus_handle.as_ref() {
            root = root.track_focus(focus_handle);
        }

        if themed_titlebar_enabled() {
            root = root.child(themed_titlebar(
                self.window_title.clone(),
                theme_background,
                theme_foreground,
                Some(entity.clone()),
            ));
        }

        let cell_width = gui_font.cell_width(window);
        let grid_ready = self.nvim_grid_ready;
        let mut editor = div()
            .flex_1()
            .relative()
            .overflow_hidden()
            .font_family(gui_font.family.clone())
            .text_size(px(gui_font.size))
            .line_height(line_height);

        if grid_ready {
            // A grid and its Kitty placements must share one compositing
            // layer. Keeping images as siblings of all grids lets a later
            // floating grid paint over an image that belongs to an earlier
            // grid, which is not how Neovim's multigrid compositor behaves.
            let mut main_layer = div()
                .absolute()
                .left(px(0.0))
                .top(px(0.0))
                .w_full()
                .h_full()
                .overflow_hidden()
                .child(
                    GridElement::with_shared_model(Rc::clone(&self.grid))
                        .with_metrics(cell_width, line_height)
                        .with_wide_font(gui_wide_font.family.clone(), px(gui_wide_font.size))
                        .with_nerd_fallback_font(
                            self.nerd_font_family.clone().unwrap_or_default(),
                            px(gui_font.size),
                        )
                        .with_glyph_coverage_cache(Rc::clone(&self.glyph_coverage_cache))
                        .with_shaping_cache(Rc::clone(&shaping_cache))
                        .with_nerd_fallback_mode(self.settings.fallback_mode)
                        .with_cursor_animation(self.cursor_animation)
                        .with_cursor_visible(self.cursor_grid == 1)
                        .with_cursor_mode(if self.cursor_grid == 1 {
                            cursor_mode
                        } else {
                            grid::CursorModeInfo::default()
                        })
                        .with_cursor_blink_started_at(cursor_blink_started_at)
                        .with_nerd_font_mode(true)
                        .with_input_handler(move |bounds, window, cx| {
                            let focus_handle = {
                                let view = entity.read(cx);
                                if view.input_router.target() == InputTarget::SystemIme {
                                    view.focus_handle.clone()
                                } else {
                                    None
                                }
                            };
                            if let Some(focus_handle) = focus_handle {
                                window.handle_input(
                                    &focus_handle,
                                    ElementInputHandler::new(bounds, entity.clone()),
                                    cx,
                                );
                            }
                        }),
                );

            let image_layers = self.visible_image_layers();
            for layer in image_layers.iter().filter(|layer| layer.grid == 1) {
                let Some(source) = self.image_sources.get(&layer.image).cloned() else {
                    continue;
                };
                main_layer = main_layer.child(
                    img(source)
                        .absolute()
                        .left(px(layer.column as f32 * f32::from(cell_width)))
                        .top(px(layer.row as f32 * f32::from(line_height)))
                        .w(px(layer.columns as f32 * f32::from(cell_width)))
                        .h(px(layer.rows as f32 * f32::from(line_height)))
                        .object_fit(gpui::ObjectFit::Fill),
                );
            }
            editor = editor.child(main_layer);

            for (grid_id, model, placement) in self.visible_grid_layers() {
                let width = placement.width.max(model.width() as u64);
                let height = placement.height.max(model.height() as u64);
                let mut layer = div()
                    .absolute()
                    .left(px(placement.col as f32 * f32::from(cell_width)))
                    .top(px(placement.row as f32 * f32::from(line_height)))
                    .w(px(width as f32 * f32::from(cell_width)))
                    .h(px(height as f32 * f32::from(line_height)))
                    // Kitty images are children of their owning grid. Keep
                    // an oversized preview inside that grid's compositor
                    // bounds so it cannot cover a neighbouring picker pane
                    // or its separator.
                    .overflow_hidden()
                    .child(
                        GridElement::with_shared_model(model)
                            .with_metrics(cell_width, line_height)
                            .with_default_background(
                                (placement.compindex >= 0)
                                    .then_some(self.theme.normal_float_background)
                                    .flatten(),
                            )
                            .with_wide_font(gui_wide_font.family.clone(), px(gui_wide_font.size))
                            .with_nerd_fallback_font(
                                self.nerd_font_family.clone().unwrap_or_default(),
                                px(gui_font.size),
                            )
                            .with_glyph_coverage_cache(Rc::clone(&self.glyph_coverage_cache))
                            .with_shaping_cache(Rc::clone(&shaping_cache))
                            .with_nerd_fallback_mode(self.settings.fallback_mode)
                            .with_cursor_visible(self.cursor_grid == grid_id)
                            .with_cursor_mode(if self.cursor_grid == grid_id {
                                cursor_mode
                            } else {
                                grid::CursorModeInfo::default()
                            })
                            .with_nerd_font_mode(true),
                    );
                for image_layer in image_layers.iter().filter(|layer| layer.grid == grid_id) {
                    let Some(source) = self.image_sources.get(&image_layer.image).cloned() else {
                        continue;
                    };
                    layer = layer.child(
                        img(source)
                            .absolute()
                            .left(px(image_layer.column as f32 * f32::from(cell_width)))
                            .top(px(image_layer.row as f32 * f32::from(line_height)))
                            .w(px(image_layer.columns as f32 * f32::from(cell_width)))
                            .h(px(image_layer.rows as f32 * f32::from(line_height)))
                            .object_fit(gpui::ObjectFit::Fill),
                    );
                }
                editor = editor.child(layer);
            }
        }

        root.child(editor)
    }
}
