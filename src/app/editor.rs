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
        log::debug!(target: "nvim_gpui::ime", "IME composition unmarked");
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

        log::debug!(
            target: "nvim_gpui::ime",
            "IME text committed: bytes={}, replacement_range={range:?}",
            text.len()
        );
        self.system_ime.replace_text(range, text);
        if !text.is_empty() {
            if let Some(nvim) = self.nvim.as_ref() {
                if let Err(error) = nvim.send_input(text.to_owned()) {
                    log::error!(
                        target: "nvim_gpui::ime",
                        "failed to forward committed IME text: {error}"
                    );
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
            log::debug!(
                target: "nvim_gpui::ime",
                "IME preedit updated: bytes={}, replacement_range={range:?}, selected_range={new_selected_range:?}",
                new_text.len()
            );
            self.system_ime
                .replace_and_mark_text(range, new_text, new_selected_range);
            cx.notify();
        }
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<gpui::Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<gpui::Pixels>> {
        let cursor = self.ime_cursor_position()?;
        log::trace!(
            target: "nvim_gpui::ime",
            "IME bounds requested: grid={:?}, range={range_utf16:?}, row={}, col={}",
            self.ime_input_grid,
            cursor.row,
            cursor.col
        );
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

impl NvimGpui {
    fn with_system_ime_input_handler(
        &self,
        element: GridElement,
        grid: u64,
        entity: Entity<Self>,
        invalidate_coordinates: bool,
        composition: Option<&grid::ImeComposition>,
    ) -> GridElement {
        if self.ime_input_grid != Some(grid) {
            return element;
        }

        let element = element.with_ime_composition(composition.cloned());
        element.with_input_handler(move |bounds, window, cx| {
            let focus_handle = entity.read(cx).focus_handle.clone();
            if let Some(focus_handle) = focus_handle {
                window.handle_input(
                    &focus_handle,
                    ElementInputHandler::new(bounds, entity.clone()),
                    cx,
                );
                if invalidate_coordinates {
                    log::debug!(
                        target: "nvim_gpui::ime",
                        "registered system IME input handler: grid={grid}"
                    );
                    // Schedule this after the current paint so the platform
                    // observes the input handler for this grid, rather than
                    // the handler from the previous frame.
                    window.invalidate_character_coordinates();
                }
            }
        })
    }

    fn system_ime_composition(&self) -> Option<grid::ImeComposition> {
        let marked_range = self.system_ime.marked_range_utf8()?;
        let cursor = self.ime_cursor_position()?;
        (!self.system_ime.is_empty()).then(|| grid::ImeComposition {
            row: cursor.row,
            col: cursor.col,
            text: self.system_ime.text().to_owned().into(),
            marked_range,
            selected_range: self.system_ime.selected_range_utf8(),
        })
    }

    fn system_ime_cursor_position(
        &self,
        composition: &grid::ImeComposition,
        screen_position: grid::CursorVisualPosition,
        local_position: grid::CursorVisualPosition,
        window: &Window,
        gui_font: &GuiFontSpec,
        cell_width: Pixels,
    ) -> grid::CursorVisualPosition {
        let selected_start = composition.selected_range.start.min(composition.text.len());
        let prefix = &composition.text[..selected_start];
        let offset = grid::ime_text_cell_offset(
            window,
            &gui_font.family,
            px(gui_font.size),
            prefix,
            cell_width,
        );
        let screen_row = screen_position
            .row
            .saturating_sub(local_position.row)
            .saturating_add(composition.row);
        let screen_col = screen_position
            .col
            .saturating_sub(local_position.col)
            .saturating_add(composition.col)
            .saturating_add(offset);
        grid::CursorVisualPosition {
            row: screen_row,
            col: screen_col,
            width: 1,
        }
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
    cursor_blink_started_at: Instant,
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

        let active_ime_grid =
            (self.input_router.target() == InputTarget::SystemIme).then_some(self.cursor_grid);
        if self.ime_input_grid != active_ime_grid {
            log::debug!(
                target: "nvim_gpui::ime",
                "IME input grid changed: from={:?}, to={active_ime_grid:?}",
                self.ime_input_grid
            );
            self.ime_input_grid = active_ime_grid;
            self.ime_coordinates_dirty = true;
        }
        let invalidate_ime_coordinates = self.ime_coordinates_dirty;
        if invalidate_ime_coordinates && self.ime_input_grid.is_some() {
            self.ime_coordinates_dirty = false;
        }
        let ime_composition = self.system_ime_composition();

        let cursor_element =
            grid_ready.then(|| {
                let model = self.active_cursor_model()?;
                let local_position = model.cursor_visual_position()?;
                let position = self.current_cursor_screen_position()?;
                let position = ime_composition
                    .as_ref()
                    .map(|composition| {
                        self.system_ime_cursor_position(
                            composition,
                            position,
                            local_position,
                            window,
                            &gui_font,
                            cell_width,
                        )
                    })
                    .unwrap_or(position);
                let (cursor_foreground, cursor_background) =
                    grid::cursor_colors(&model, local_position, cursor_mode);
                let glyph_source = (cursor_mode.shape == grid::CursorShape::Block).then(|| {
                    self.grid_element(
                        Rc::clone(&model),
                        GridRenderOptions {
                            placement: self.grid_placement(self.cursor_grid),
                            width: model.width(),
                            height: model.height(),
                            cell_width,
                            line_height,
                            gui_font: &gui_font,
                            gui_wide_font: &gui_wide_font,
                            cursor_blink_started_at,
                            viewport_offset: px(0.0),
                        },
                    )
                });
                Some(
                    grid::CursorElement::new(position, cursor_background, cursor_mode)
                        .with_local_position(local_position)
                        .with_glyph_foreground(cursor_foreground)
                        .with_glyph_source(glyph_source)
                        .with_animation(self.cursor_animation.filter(|animation| {
                            ime_composition.is_none() && animation.is_active(now)
                        }))
                        .with_metrics(cell_width, line_height)
                        .with_grid_size(self.grid.width(), self.grid.height())
                        .with_blink_started_at(cursor_blink_started_at),
                )
            });
        let cursor_element = cursor_element.flatten();
        let mut editor = div()
            .flex_1()
            .relative()
            .overflow_hidden()
            .font_family(gui_font.family.clone())
            .text_size(px(gui_font.size))
            .line_height(line_height)
            .on_any_mouse_down(cx.listener(Self::on_mouse_down))
            .capture_any_mouse_up(cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel));

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
                cursor_blink_started_at,
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
                    viewport_offset: old_offset,
                    ..main_options
                };
                main_layer = main_layer.child(Self::grid_surface(
                    self.grid_element(Rc::clone(&animation.previous_grid), old_options),
                    old_options,
                ));
            }

            let main_element = self.with_system_ime_input_handler(
                self.grid_element(Rc::clone(&self.grid), main_options),
                1,
                entity.clone(),
                invalidate_ime_coordinates,
                ime_composition.as_ref(),
            );
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
                    cursor_blink_started_at,
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
                        viewport_offset: old_offset,
                        ..options
                    };
                    layer = layer.child(Self::grid_surface(
                        self.grid_element(Rc::clone(&animation.previous_grid), old_options),
                        old_options,
                    ));
                }
                layer = layer.child(Self::grid_surface(
                    self.with_system_ime_input_handler(
                        self.grid_element(model, options),
                        grid_id,
                        entity.clone(),
                        invalidate_ime_coordinates,
                        ime_composition.as_ref(),
                    ),
                    options,
                ));
                layer = layer.child(self.image_surface(grid_id, &image_layers, options));
                editor = editor.child(layer);
            }

            if let Some(cursor_element) = cursor_element {
                editor = editor.child(
                    div()
                        .absolute()
                        .left(px(0.0))
                        .top(px(0.0))
                        .w_full()
                        .h_full()
                        .child(cursor_element),
                );
            }
        }

        root.child(editor)
    }
}

impl NvimGpui {
    pub(super) fn mouse_option_allows_current_mode(&self) -> bool {
        let mode = self.nvim_mode.chars().next().unwrap_or('n');
        let required = match mode {
            'i' | 'R' | 's' | 'S' => 'i',
            'v' | 'V' | '\u{16}' => 'v',
            'c' => 'c',
            'r' => 'r',
            // Terminal-mode mouse input is enabled by `a`, which is the
            // useful GUI behavior even though the option predates terminal
            // mode as a separate mode code.
            't' => return self.mouse_option.contains('a'),
            _ => 'n',
        };
        self.mouse_option.contains('a') || self.mouse_option.contains(required)
    }

    pub(super) fn nvim_mouse_position(
        position: gpui::Point<Pixels>,
        cell_width: Pixels,
        line_height: Pixels,
    ) -> (u64, u64) {
        let editor_y = f32::from(position.y)
            - if themed_titlebar_enabled() {
                THEMED_TITLEBAR_HEIGHT
            } else {
                0.0
            };
        (
            (editor_y / f32::from(line_height)).max(0.0).floor() as u64,
            (f32::from(position.x) / f32::from(cell_width))
                .max(0.0)
                .floor() as u64,
        )
    }

    fn send_mouse(
        &mut self,
        button: &str,
        action: &str,
        modifiers: gpui::Modifiers,
        position: gpui::Point<Pixels>,
        window: &Window,
    ) {
        if !self.mouse_enabled {
            return;
        }
        let gui_font = self.current_grid_font(window);
        let cell_width = gui_font.cell_width(window);
        let line_height = gui_font.line_height(window, self.linespace);
        let (row, col) = Self::nvim_mouse_position(position, cell_width, line_height);
        let modifier = input::nvim_mouse_modifiers(modifiers);
        if let Some(nvim) = self.nvim.as_ref() {
            if let Err(error) = nvim.send_mouse(button, action, modifier, 0, row, col) {
                log::error!(
                    target: "nvim_gpui::input",
                    "mouse event failed: button={button}, action={action}, row={row}, col={col}: {error}"
                );
                self.rpc_status = format!("rpc mouse error: {error}");
            }
        }
    }

    pub(super) fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        if let Some(focus_handle) = self.focus_handle.as_ref() {
            window.focus(focus_handle);
        }
        self.send_mouse(
            input::nvim_mouse_button(event.button),
            "press",
            event.modifiers,
            event.position,
            window,
        );
        window.prevent_default();
    }

    pub(super) fn on_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.send_mouse(
            input::nvim_mouse_button(event.button),
            "release",
            event.modifiers,
            event.position,
            window,
        );
        window.prevent_default();
    }

    pub(super) fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        let (button, action) = event
            .pressed_button
            .map(|button| (input::nvim_mouse_button(button), "drag"))
            .unwrap_or(("move", "move"));
        self.send_mouse(button, action, event.modifiers, event.position, window);
        window.prevent_default();
    }

    pub(super) fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        if !self.mouse_enabled {
            return;
        }

        let gui_font = self.current_grid_font(window);
        let cell_width = gui_font.cell_width(window);
        let line_height = gui_font.line_height(window, self.linespace);
        let mut delta = input::scroll_delta_to_lines(event.delta, line_height);

        // Shift-wheel is conventionally horizontal. Windows' GPUI backend
        // already performs this conversion, while macOS/Linux may expose
        // the original vertical axis, so keep the behavior consistent.
        if event.modifiers.shift && delta.x.abs() < f32::EPSILON {
            delta.x = delta.y;
            delta.y = 0.0;
        }
        self.scroll_remainder.x += delta.x;
        self.scroll_remainder.y += delta.y;

        let x_steps = self.scroll_remainder.x.trunc() as i32;
        let y_steps = self.scroll_remainder.y.trunc() as i32;
        self.scroll_remainder.x -= x_steps as f32;
        self.scroll_remainder.y -= y_steps as f32;
        let (row, col) = Self::nvim_mouse_position(event.position, cell_width, line_height);
        let modifier = input::nvim_mouse_modifiers(event.modifiers);

        if let Some(nvim) = self.nvim.as_ref() {
            for _ in 0..x_steps.unsigned_abs() {
                let action = if x_steps > 0 { "right" } else { "left" };
                if let Err(error) = nvim.send_mouse("wheel", action, modifier.clone(), 0, row, col)
                {
                    log::error!(
                        target: "nvim_gpui::input",
                        "horizontal wheel event failed: action={action}, row={row}, col={col}: {error}"
                    );
                    self.rpc_status = format!("rpc mouse error: {error}");
                    break;
                }
            }
            for _ in 0..y_steps.unsigned_abs() {
                let action = if y_steps > 0 { "up" } else { "down" };
                if let Err(error) = nvim.send_mouse("wheel", action, modifier.clone(), 0, row, col)
                {
                    log::error!(
                        target: "nvim_gpui::input",
                        "vertical wheel event failed: action={action}, row={row}, col={col}: {error}"
                    );
                    self.rpc_status = format!("rpc mouse error: {error}");
                    break;
                }
            }
        }
        window.prevent_default();
    }
}
