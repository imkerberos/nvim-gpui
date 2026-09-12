use super::*;

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
    fn highlight_context_for_layer(&self, kind: GridLayerKind) -> grid::HighlightContext {
        match kind {
            GridLayerKind::Float => grid::HighlightContext::Floating {
                background: self.editor.protocol.theme.normal_float_background,
            },
            GridLayerKind::Message => grid::HighlightContext::Message {
                background: self.editor.protocol.theme.normal_float_background,
            },
            GridLayerKind::Main | GridLayerKind::Window | GridLayerKind::External => {
                grid::HighlightContext::Main
            }
        }
    }

    fn grid_element(
        &self,
        model: Rc<grid::GridModel>,
        options: GridRenderOptions<'_>,
    ) -> GridElement {
        let highlight_context = self.highlight_context_for_layer(options.placement.kind);
        let mut element = GridElement::with_shared_model(model)
            .with_metrics(options.cell_width, options.line_height)
            .with_highlight_context(highlight_context)
            .with_wide_font(
                options.gui_wide_font.family.clone(),
                px(options.gui_wide_font.size),
            )
            .with_nerd_fallback_font(
                self.editor.nerd_font_family.clone().unwrap_or_default(),
                px(options.gui_font.size),
            )
            .with_glyph_coverage_cache(Rc::clone(&self.editor.glyph_coverage_cache))
            .with_shaping_cache(Rc::clone(&self.editor.shaping_cache))
            .with_nerd_fallback_mode(self.app.settings.fallback_mode)
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

    fn multicursor_elements(
        &self,
        cell_width: Pixels,
        line_height: Pixels,
        gui_font: &GuiFontSpec,
        gui_wide_font: &GuiFontSpec,
        cursor_blink_started_at: Instant,
    ) -> Vec<grid::CursorElement> {
        let mut positions = self.visible_multicursor_positions();
        positions.sort_by_key(|position| {
            (
                position.key.grid,
                position.row,
                position.col,
                position.key.ns_id,
                position.key.mark_id,
            )
        });

        positions
            .into_iter()
            .filter_map(|position| {
                let model = if position.key.grid == 1 {
                    Rc::clone(&self.editor.protocol.presentation.grid)
                } else {
                    self.editor
                        .protocol
                        .presentation
                        .other_grids
                        .get(&position.key.grid)
                        .cloned()?
                };
                let local_position = model.visual_position_at(position.row, position.col)?;
                let placement = self.grid_placement(position.key.grid);
                let row = placement.row.checked_add(local_position.row as i64)?;
                let col = placement.col.checked_add(local_position.col as i64)?;
                if row < 0 || col < 0 {
                    return None;
                }
                let screen_position = grid::CursorVisualPosition {
                    row: row as usize,
                    col: col as usize,
                    width: local_position.width,
                };
                let context = self.highlight_context_for_layer(placement.kind);
                let (foreground, background) =
                    grid::multicursor_colors_with_context(&model, local_position, context);
                let glyph_source = self.grid_element(
                    Rc::clone(&model),
                    GridRenderOptions {
                        placement,
                        width: model.width(),
                        height: model.height(),
                        cell_width,
                        line_height,
                        gui_font,
                        gui_wide_font,
                        cursor_blink_started_at,
                        viewport_offset: px(0.0),
                    },
                );
                Some(
                    grid::CursorElement::new(
                        screen_position,
                        background,
                        grid::CursorModeInfo::default(),
                    )
                    .with_local_position(local_position)
                    .with_glyph_foreground(foreground)
                    .with_glyph_source(Some(glyph_source))
                    .with_metrics(cell_width, line_height)
                    .with_grid_size(model.width(), model.height())
                    .with_blink_started_at(cursor_blink_started_at)
                    .with_rounded_corners(false),
                )
            })
            .collect()
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
        image_sources: &HashMap<ImageId, Arc<Image>>,
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
            let Some(source) = image_sources.get(&image_layer.image).cloned() else {
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

impl NvimGpui {
    pub(crate) fn render_editor_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        self.sync_nvim_size(window);

        let gui_font = self.current_grid_font(window);
        let gui_wide_font = self.current_grid_wide_font(window);
        let line_height = gui_font.line_height(window, self.editor.protocol.linespace);
        let cursor_mode = self.current_cursor_mode();
        let cursor_blink_started_at = self.editor.cursor.cursor_blink_started_at;

        let entity = cx.entity();

        let cell_width = gui_font.cell_width(window);
        let grid_ready = self.editor.protocol.startup.nvim_grid_ready;
        let now = Instant::now();
        for animation in self.editor.presentation.viewport_animations.values_mut() {
            // Redraw processing can take longer than the animation duration,
            // especially for a large screen update. Start the clock when this
            // frame is actually about to be rendered so the first visible
            // frame cannot consume the whole animation.
            animation.mark_presented(now);
        }
        self.editor
            .presentation
            .viewport_animations
            .retain(|_, animation| animation.is_active(now));
        let viewport_animations = self.editor.presentation.viewport_animations.clone();

        let active_ime_grid = (self.editor.input.input_router.target() == InputTarget::SystemIme)
            .then_some(self.editor.protocol.cursor.cursor_grid);
        if self.editor.input.ime_input_grid != active_ime_grid {
            log::debug!(
                target: "nvim_gpui::ime",
                "IME input grid changed: from={:?}, to={active_ime_grid:?}",
                self.editor.input.ime_input_grid
            );
            self.editor.input.ime_input_grid = active_ime_grid;
            self.editor.input.ime_coordinates_dirty = true;
        }
        let invalidate_ime_coordinates = self.editor.input.ime_coordinates_dirty;
        if invalidate_ime_coordinates && self.editor.input.ime_input_grid.is_some() {
            self.editor.input.ime_coordinates_dirty = false;
        }
        let ime_composition = self.active_ime_composition();

        let cursor_element = grid_ready.then(|| {
            let model = self.active_cursor_model()?;
            let local_position = model.cursor_visual_position()?;
            let position = self.current_cursor_screen_position()?;
            let position = ime_composition
                .as_ref()
                .map(|composition| {
                    self.ime_cursor_position_for_composition(composition, position, local_position)
                })
                .unwrap_or(position);
            let cursor_placement = self.grid_placement(self.editor.protocol.cursor.cursor_grid);
            let cursor_context = self.highlight_context_for_layer(cursor_placement.kind);
            let (cursor_foreground, cursor_background) = grid::cursor_colors_with_context(
                &model,
                local_position,
                cursor_mode,
                cursor_context,
            );
            let glyph_source = (cursor_mode.shape == grid::CursorShape::Block).then(|| {
                self.grid_element(
                    Rc::clone(&model),
                    GridRenderOptions {
                        placement: cursor_placement,
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
                    .with_animation(
                        self.editor.cursor.cursor_animation.filter(|animation| {
                            ime_composition.is_none() && animation.is_active(now)
                        }),
                    )
                    .with_metrics(cell_width, line_height)
                    .with_grid_size(
                        self.editor.protocol.presentation.grid.width(),
                        self.editor.protocol.presentation.grid.height(),
                    )
                    .with_blink_started_at(cursor_blink_started_at),
            )
        });
        let cursor_element = cursor_element.flatten();
        let multicursor_elements = if grid_ready {
            self.multicursor_elements(
                cell_width,
                line_height,
                &gui_font,
                &gui_wide_font,
                cursor_blink_started_at,
            )
        } else {
            Vec::new()
        };
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
            let presentation = self.presentation_snapshot();
            let compositor_frame = &presentation.compositor;
            let image_layers = &presentation.image_layers;
            let image_sources = &presentation.image_sources;
            // A grid and its Kitty placements must share one compositing
            // layer. Keeping images as siblings of all grids lets a later
            // floating grid paint over an image that belongs to an earlier
            // grid, which is not how Neovim's multigrid compositor behaves.
            let main_layer = &compositor_frame.layers[0];
            let main_model = Rc::clone(&main_layer.model);
            let main_placement = main_layer.placement;
            let main_width = main_layer.content_rect.width as usize;
            let main_height = main_layer.content_rect.height as usize;
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

            let main_element = self.with_ime_input_handler(
                self.grid_element(main_model, main_options),
                1,
                entity.clone(),
                invalidate_ime_coordinates,
                ime_composition.as_ref(),
            );
            main_layer = main_layer.child(Self::grid_surface(main_element, main_options));

            main_layer =
                main_layer.child(self.image_surface(1, image_layers, image_sources, main_options));
            editor = editor.child(main_layer);

            for compositor_layer in compositor_frame.layers.iter().skip(1) {
                let grid_id = compositor_layer.grid_id;
                let model = Rc::clone(&compositor_layer.model);
                let placement = compositor_layer.placement;
                let content_rect = compositor_layer.content_rect;
                let surface_rect = compositor_layer.surface_rect;
                let clip_rect = compositor_layer.clip_rect;
                let model_width = content_rect.width as usize;
                let model_height = content_rect.height as usize;
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
                    .left(px(surface_rect.col as f32 * f32::from(cell_width)))
                    .top(px(surface_rect.row as f32 * f32::from(line_height)))
                    .w(px(clip_rect.width as f32 * f32::from(cell_width)))
                    .h(px(clip_rect.height as f32 * f32::from(line_height)))
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
                    self.with_ime_input_handler(
                        self.grid_element(model, options),
                        grid_id,
                        entity.clone(),
                        invalidate_ime_coordinates,
                        ime_composition.as_ref(),
                    ),
                    options,
                ));
                layer =
                    layer.child(self.image_surface(grid_id, image_layers, image_sources, options));
                editor = editor.child(layer);
            }

            if cursor_element.is_some() || !multicursor_elements.is_empty() {
                let mut cursor_layer = div()
                    .absolute()
                    .left(px(0.0))
                    .top(px(0.0))
                    .w_full()
                    .h_full();
                for multicursor_element in multicursor_elements {
                    cursor_layer = cursor_layer.child(multicursor_element);
                }
                if let Some(cursor_element) = cursor_element {
                    cursor_layer = cursor_layer.child(cursor_element);
                }
                editor = editor.child(cursor_layer);
            }

            if let Some(rime_popup) =
                self.rime_candidate_popup(&gui_font, &gui_wide_font, cell_width, line_height)
            {
                editor = editor.child(rime_popup);
            }
        }

        editor
    }
}
