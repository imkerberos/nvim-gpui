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

#[derive(Clone, Copy)]
struct GridRenderOptions<'a> {
    placement: GridPlacement,
    width: usize,
    height: usize,
    cell_width: Pixels,
    line_height: Pixels,
    gui_font: &'a GuiFontSpec,
    gui_wide_font: &'a GuiFontSpec,
    cursor_mode: grid::CursorModeInfo,
    cursor_blink_started_at: Instant,
    cursor_animation: Option<grid::CursorAnimation>,
    cursor_visible: bool,
    viewport_offset: Pixels,
}

impl NvimGpui {
    fn grid_element(
        &self,
        model: Rc<grid::GridModel>,
        options: GridRenderOptions<'_>,
    ) -> GridElement {
        let mut element = GridElement::with_shared_model(model)
            .with_metrics(options.cell_width, options.line_height)
            .with_default_background(
                (options.placement.compindex >= 0)
                    .then_some(self.theme.normal_float_background)
                    .flatten(),
            )
            .with_wide_font(
                options.gui_wide_font.family.clone(),
                px(options.gui_wide_font.size),
            )
            .with_nerd_fallback_font(
                self.nerd_font_family.clone().unwrap_or_default(),
                px(options.gui_font.size),
            )
            .with_glyph_coverage_cache(Rc::clone(&self.glyph_coverage_cache))
            .with_shaping_cache(Rc::clone(&self.shaping_cache))
            .with_nerd_fallback_mode(self.settings.fallback_mode)
            .with_cursor_animation(options.cursor_animation)
            .with_cursor_visible(options.cursor_visible)
            .with_cursor_mode(if options.cursor_visible {
                options.cursor_mode
            } else {
                grid::CursorModeInfo::default()
            })
            .with_cursor_blink_started_at(options.cursor_blink_started_at)
            .with_viewport_offset(point(px(0.0), options.viewport_offset))
            .with_nerd_font_mode(true);

        if let Some(margins) = options.placement.viewport_margins {
            element = element.with_viewport_margins(
                margins.top,
                margins.bottom,
                margins.left,
                margins.right,
            );
        }

        element
    }

    fn grid_surface(element: GridElement, options: GridRenderOptions<'_>) -> gpui::Div {
        div()
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .w(px(options.width as f32 * f32::from(options.cell_width)))
            .h(px(options.height as f32 * f32::from(options.line_height)))
            .child(element)
    }

    pub(super) fn viewport_rect(
        placement: GridPlacement,
        width: usize,
        height: usize,
    ) -> (usize, usize, usize, usize) {
        let margins = placement
            .viewport_margins
            .map(|margins| {
                (
                    usize::try_from(margins.top).unwrap_or(usize::MAX),
                    usize::try_from(margins.bottom).unwrap_or(usize::MAX),
                    usize::try_from(margins.left).unwrap_or(usize::MAX),
                    usize::try_from(margins.right).unwrap_or(usize::MAX),
                )
            })
            .unwrap_or_default();
        let top = margins.0.min(height);
        let bottom = margins.1.min(height.saturating_sub(top));
        let left = margins.2.min(width);
        let right = margins.3.min(width.saturating_sub(left));
        (
            left,
            top,
            width.saturating_sub(left + right),
            height.saturating_sub(top + bottom),
        )
    }

    fn image_surface(
        &self,
        grid_id: u64,
        image_layers: &[ImageLayer],
        options: GridRenderOptions<'_>,
    ) -> gpui::Div {
        let (left, top, viewport_width, viewport_height) =
            Self::viewport_rect(options.placement, options.width, options.height);
        let mut surface = div()
            .absolute()
            .left(px(left as f32 * f32::from(options.cell_width)))
            .top(px(top as f32 * f32::from(options.line_height)))
            .w(px(viewport_width as f32 * f32::from(options.cell_width)))
            .h(px(viewport_height as f32 * f32::from(options.line_height)))
            .overflow_hidden();

        for image_layer in image_layers.iter().filter(|layer| layer.grid == grid_id) {
            let Some(source) = self.image_sources.get(&image_layer.image).cloned() else {
                continue;
            };
            surface = surface.child(
                img(source)
                    .absolute()
                    .left(px(
                        (image_layer.column as f32 - left as f32) * f32::from(options.cell_width)
                    ))
                    .top(px(
                        (image_layer.row as f32 - top as f32) * f32::from(options.line_height)
                    ))
                    .w(px(
                        image_layer.columns as f32 * f32::from(options.cell_width)
                    ))
                    .h(px(image_layer.rows as f32 * f32::from(options.line_height)))
                    .object_fit(gpui::ObjectFit::Fill),
            );
        }

        surface
    }
}

impl Render for NvimGpui {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_window_title(&self.window_title);
        self.sync_nvim_size(window);

        let gui_font = self.current_grid_font(window);
        let gui_wide_font = self.current_grid_wide_font(window);
        let line_height = gui_font.line_height(window, self.linespace);
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
        let now = Instant::now();
        self.viewport_animations
            .retain(|_, animation| animation.is_active(now));
        let viewport_animations = self.viewport_animations.clone();
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
            let main_placement = self.grid_placements.get(&1).copied().unwrap_or_default();
            let main_width = self.grid.width();
            let main_height = self.grid.height();
            let main_animation = viewport_animations.get(&1);
            let (old_offset, current_offset) = main_animation
                .map(|animation| animation.offsets(now, main_height, line_height))
                .unwrap_or((px(0.0), px(0.0)));
            let main_options = GridRenderOptions {
                placement: main_placement,
                width: main_width,
                height: main_height,
                cell_width,
                line_height,
                gui_font: &gui_font,
                gui_wide_font: &gui_wide_font,
                cursor_mode,
                cursor_blink_started_at,
                cursor_animation: self.cursor_animation,
                cursor_visible: self.cursor_grid == 1,
                viewport_offset: current_offset,
            };
            let mut main_layer = div()
                .absolute()
                .left(px(0.0))
                .top(px(0.0))
                .w_full()
                .h_full()
                .overflow_hidden();

            if let Some(animation) = main_animation {
                let old_options = GridRenderOptions {
                    cursor_animation: None,
                    cursor_visible: false,
                    viewport_offset: old_offset,
                    ..main_options
                };
                main_layer = main_layer.child(Self::grid_surface(
                    self.grid_element(Rc::clone(&animation.previous_grid), old_options),
                    old_options,
                ));
            }

            let mut main_element = self.grid_element(Rc::clone(&self.grid), main_options);
            main_element = main_element.with_input_handler(move |bounds, window, cx| {
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
            });
            main_layer = main_layer.child(Self::grid_surface(main_element, main_options));

            let image_layers = self.visible_image_layers();
            main_layer = main_layer.child(self.image_surface(1, &image_layers, main_options));
            editor = editor.child(main_layer);

            for (grid_id, model, placement) in self.visible_grid_layers() {
                let width = placement.width.max(model.width() as u64);
                let height = placement.height.max(model.height() as u64);
                let model_width = model.width();
                let model_height = model.height();
                let animation = viewport_animations.get(&grid_id);
                let (old_offset, current_offset) = animation
                    .map(|animation| animation.offsets(now, model_height, line_height))
                    .unwrap_or((px(0.0), px(0.0)));
                let options = GridRenderOptions {
                    placement,
                    width: model_width,
                    height: model_height,
                    cell_width,
                    line_height,
                    gui_font: &gui_font,
                    gui_wide_font: &gui_wide_font,
                    cursor_mode,
                    cursor_blink_started_at,
                    cursor_animation: None,
                    cursor_visible: self.cursor_grid == grid_id,
                    viewport_offset: current_offset,
                };
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
                    .overflow_hidden();
                if let Some(animation) = animation {
                    let old_options = GridRenderOptions {
                        cursor_visible: false,
                        viewport_offset: old_offset,
                        ..options
                    };
                    layer = layer.child(Self::grid_surface(
                        self.grid_element(Rc::clone(&animation.previous_grid), old_options),
                        old_options,
                    ));
                }
                layer = layer.child(Self::grid_surface(
                    self.grid_element(model, options),
                    options,
                ));
                layer = layer.child(self.image_surface(grid_id, &image_layers, options));
                editor = editor.child(layer);
            }
        }

        root.child(editor)
    }
}
