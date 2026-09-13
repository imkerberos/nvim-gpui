use super::*;
use crate::input;

impl Focusable for NvimGpui {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.window
            .focus_handle
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
        let (text, actual_range) = self.editor.input.system_ime.text_for_range(range_utf16);
        adjusted_range.replace(actual_range);
        Some(text)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<gpui::UTF16Selection> {
        Some(self.editor.input.system_ime.selected_text_range())
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.editor.input.system_ime.marked_text_range()
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        // The local buffer only represents the active composition. Once the
        // platform cancels its marked range, there is no text to retain here.
        log::debug!(target: "nvim_gpui::ime", "IME composition unmarked");
        self.editor.input.system_ime.clear();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.input.input_router.target() != InputTarget::SystemIme {
            return;
        }

        log::debug!(
            target: "nvim_gpui::ime",
            "IME text committed: bytes={}, replacement_range={range:?}",
            text.len()
        );
        self.editor.input.system_ime.replace_text(range, text);
        if !text.is_empty() {
            if let Some(nvim) = self.app.session.nvim.as_ref() {
                if let Err(error) = nvim.send_input(text.to_owned()) {
                    log::error!(
                        target: "nvim_gpui::ime",
                        "failed to forward committed IME text: {error}"
                    );
                    self.app.session.rpc_status = format!("rpc input error: {error}");
                }
            }
        }
        self.editor.input.system_ime.clear();
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
        if self.editor.input.input_router.target() == InputTarget::SystemIme {
            log::debug!(
                target: "nvim_gpui::ime",
                "IME preedit updated: bytes={}, replacement_range={range:?}, selected_range={new_selected_range:?}",
                new_text.len()
            );
            self.editor
                .input
                .system_ime
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
        let cursor = self.editor.ime_cursor_position()?;
        log::trace!(
            target: "nvim_gpui::ime",
            "IME bounds requested: grid={:?}, range={range_utf16:?}, row={}, col={}",
            self.editor.input.ime_input_grid,
            cursor.row,
            cursor.col
        );
        let font_spec = self.editor.current_grid_font(window);
        let cell_width = font_spec.cell_width(window);
        let line_height = font_spec.line_height(window, self.editor.protocol.linespace);
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
        let font_spec = self.editor.current_grid_font(window);
        let cell_width = font_spec.cell_width(window);
        let column = (f32::from(point.x) / f32::from(cell_width))
            .max(0.0)
            .floor() as usize;
        let byte_offset = self
            .editor
            .input
            .system_ime
            .text()
            .char_indices()
            .nth(column)
            .map(|(offset, _)| offset)
            .unwrap_or(self.editor.input.system_ime.text().len());
        Some(input::utf8_to_utf16_offset(
            self.editor.input.system_ime.text(),
            byte_offset,
        ))
    }
}

impl NvimGpui {
    pub(super) fn with_ime_input_handler(
        &self,
        element: GridElement,
        grid: u64,
        entity: Entity<Self>,
        invalidate_coordinates: bool,
        composition: Option<&grid::ImeComposition>,
    ) -> GridElement {
        let owns_composition = self.editor.composition_grid() == Some(grid);
        let element = if owns_composition {
            element.with_ime_composition(composition.cloned())
        } else {
            element
        };
        if self.editor.input.ime_input_grid != Some(grid) {
            return element;
        }

        element.with_input_handler(move |bounds, window, cx| {
            let focus_handle = entity.read(cx).window.focus_handle.clone();
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
}

impl EditorRuntime {
    fn composition_grid(&self) -> Option<u64> {
        match self.input.input_router.target() {
            InputTarget::SystemIme => self.input.ime_input_grid,
            InputTarget::Rime => self
                .input
                .rime_context
                .as_ref()
                .filter(|context| !context.preedit.is_empty())
                .map(|_| self.protocol.cursor.cursor_grid),
            InputTarget::Neovim => None,
        }
    }

    fn system_ime_composition(&self) -> Option<grid::ImeComposition> {
        let marked_range = self.input.system_ime.marked_range_utf8()?;
        let cursor = self.ime_cursor_position()?;
        (!self.input.system_ime.is_empty()).then(|| {
            let text = self.input.system_ime.text().to_owned();
            grid::ImeComposition {
                row: cursor.row,
                col: cursor.col,
                grid_width: self.protocol.display_options.text_cell_width(&text).max(1),
                text: text.into(),
                marked_range,
                selected_range: self.input.system_ime.selected_range_utf8(),
            }
        })
    }

    fn rime_composition(&self) -> Option<grid::ImeComposition> {
        let context = self.input.rime_context.as_ref()?;
        if context.preedit.is_empty() {
            return None;
        }
        let cursor = self.active_cursor_model()?.cursor_visual_position()?;
        let text = context.preedit.clone();
        let text_len = text.len();
        let cursor_pos = context.cursor_pos.min(text_len);
        Some(grid::ImeComposition {
            row: cursor.row,
            col: cursor.col,
            grid_width: self.protocol.display_options.text_cell_width(&text).max(1),
            text: text.into(),
            marked_range: 0..text_len,
            selected_range: cursor_pos..cursor_pos,
        })
    }

    pub(super) fn active_ime_composition(&self) -> Option<grid::ImeComposition> {
        match self.input.input_router.target() {
            InputTarget::SystemIme => self.system_ime_composition(),
            InputTarget::Rime => self.rime_composition(),
            InputTarget::Neovim => None,
        }
    }

    pub(super) fn ime_cursor_position_for_composition(
        &self,
        composition: &grid::ImeComposition,
        screen_position: grid::CursorVisualPosition,
        local_position: grid::CursorVisualPosition,
    ) -> grid::CursorVisualPosition {
        let selected_start = composition.selected_range.start.min(composition.text.len());
        let prefix = &composition.text[..selected_start];
        let offset = grid::ime_text_cell_offset(prefix, self.protocol.display_options);
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

impl EditorRuntime {
    pub(super) fn rime_candidate_popup(
        &self,
        gui_font: &GuiFontSpec,
        gui_wide_font: &GuiFontSpec,
        cell_width: Pixels,
        line_height: Pixels,
        layout: settings::RimeCandidateLayout,
    ) -> Option<gpui::Div> {
        if self.input.input_router.target() != InputTarget::Rime {
            return None;
        }
        let context = self.input.rime_context.as_ref()?;
        if context.candidates.is_empty() {
            return None;
        }
        let position = self.current_cursor_screen_position()?;
        let popup_background = self
            .protocol
            .theme
            .normal_float_background
            .unwrap_or(SURFACE);
        let popup_foreground = self.theme_foreground();
        let horizontal = layout == settings::RimeCandidateLayout::Horizontal;
        let page = context.page_no.saturating_add(1);
        let page_label = page.to_string();
        let cell_width_px = f32::from(cell_width);
        let candidate_widths = context
            .candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| {
                let marker_width = self
                    .protocol
                    .display_options
                    .text_cell_width(candidate_marker(index));
                let text_width = self
                    .protocol
                    .display_options
                    .text_cell_width(&candidate.text);
                let mut width = 8.0 + (marker_width + 1 + text_width) as f32 * cell_width_px;
                if let Some(comment) = candidate.comment.as_deref() {
                    width += 8.0
                        + self.protocol.display_options.text_cell_width(comment) as f32
                            * cell_width_px;
                }
                width
            })
            .collect::<Vec<_>>();
        let page_indicator_width = 16.0 + (page_label.len() + 2) as f32 * cell_width_px;
        let content_width = if horizontal {
            candidate_widths.iter().sum::<f32>() + page_indicator_width
        } else {
            candidate_widths
                .iter()
                .copied()
                .reduce(f32::max)
                .unwrap_or_default()
                .max(page_indicator_width)
        };
        let popup_width = px(content_width.max(12.0 * cell_width_px));
        let row_count = if horizontal {
            1
        } else {
            context.candidates.len() + 1
        };
        let popup_height = px((row_count as f32 + 1.0) * f32::from(line_height));
        let viewport_width = self
            .protocol
            .presentation
            .grid_size
            .map(|(width, _)| width as f32 * f32::from(cell_width));
        let viewport_height = self
            .protocol
            .presentation
            .grid_size
            .map(|(_, height)| height as f32 * f32::from(line_height));
        let cursor_left = position.col as f32 * f32::from(cell_width);
        let cursor_top = position.row as f32 * f32::from(line_height);
        let left = viewport_width
            .map(|width| (cursor_left.min((width - f32::from(popup_width)).max(0.0))).max(0.0))
            .unwrap_or(cursor_left);
        let below_top = cursor_top + f32::from(line_height);
        let top = viewport_height
            .filter(|height| below_top + f32::from(popup_height) > *height)
            .map(|_| (cursor_top - f32::from(popup_height)).max(0.0))
            .unwrap_or(below_top);

        let mut candidate_font = font(gui_font.family.clone());
        if let Some(nerd_font_family) = self.nerd_font_family.as_ref() {
            candidate_font.fallbacks =
                Some(FontFallbacks::from_fonts(vec![nerd_font_family.clone()]));
        }
        let candidate_wide_font = if gui_wide_font.family == gui_font.family {
            candidate_font.clone()
        } else {
            font(gui_wide_font.family.clone())
        };

        let mut popup = div()
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(popup_width)
            .p_1()
            .border_1()
            .border_color(rgb(SURFACE_BRIGHT))
            .bg(rgb(popup_background))
            .text_color(rgb(popup_foreground))
            .font(candidate_font.clone())
            .text_size(px(gui_font.size))
            .when(horizontal, |popup| popup.flex().items_center());

        for (index, candidate) in context.candidates.iter().enumerate() {
            let selected = index as i32 == context.highlighted_candidate_index;
            let mut row = div()
                .when(!horizontal, |row| row.w_full())
                .flex_none()
                .h(line_height)
                .flex()
                .items_center()
                .px_1()
                .bg(rgb(if selected { ACCENT } else { popup_background }))
                .text_color(rgb(if selected {
                    BACKGROUND
                } else {
                    popup_foreground
                }))
                .child(format!("{} ", candidate_marker(index)));
            row = row.child(candidate_text(
                &candidate.text,
                self.protocol.display_options,
                &candidate_font,
                &candidate_wide_font,
                px(gui_font.size),
                px(gui_wide_font.size),
            ));
            if let Some(comment) = candidate.comment.as_deref() {
                row = row.child(
                    div()
                        .ml_2()
                        .text_color(rgb(if selected { BACKGROUND } else { MUTED_TEXT }))
                        .child(candidate_text(
                            comment,
                            self.protocol.display_options,
                            &candidate_font,
                            &candidate_wide_font,
                            px(gui_font.size),
                            px(gui_wide_font.size),
                        )),
                );
            }
            popup = popup.child(row);
        }

        let previous_color = if context.page_no > 0 {
            ACCENT
        } else {
            MUTED_TEXT
        };
        let next_color = if context.is_last_page {
            MUTED_TEXT
        } else {
            ACCENT
        };
        let previous_icon = if self.nerd_font_family.is_some() {
            // Nerd Font: angle-up (U+F0D9).
            "\u{f0d9}"
        } else {
            "‹"
        };
        let next_icon = if self.nerd_font_family.is_some() {
            // Nerd Font: angle-down (U+F0DA).
            "\u{f0da}"
        } else {
            "›"
        };
        let page_indicator = div()
            .h(line_height)
            .flex()
            .items_center()
            .justify_end()
            .px_1()
            .text_sm()
            .child(div().text_color(rgb(previous_color)).child(previous_icon))
            .child(
                div()
                    .mx_1()
                    .text_color(rgb(if context.page_no == 0 && context.is_last_page {
                        MUTED_TEXT
                    } else {
                        popup_foreground
                    }))
                    .child(page_label),
            )
            .child(div().text_color(rgb(next_color)).child(next_icon));
        if horizontal {
            popup = popup.child(page_indicator.w(px(page_indicator_width)));
        } else {
            popup = popup.child(page_indicator.w_full());
        }

        Some(popup)
    }
}

fn candidate_marker(index: usize) -> &'static str {
    ["①", "②", "③", "④", "⑤", "⑥", "⑦", "⑧", "⑨", "⓪"]
        .get(index)
        .copied()
        .unwrap_or("⓪")
}

fn candidate_text(
    text: &str,
    display_options: grid::DisplayOptions,
    normal_font: &gpui::Font,
    wide_font: &gpui::Font,
    normal_size: Pixels,
    wide_size: Pixels,
) -> gpui::Div {
    text.graphemes(true).fold(
        div().flex().items_center().flex_none(),
        |container, grapheme| {
            let is_wide = display_options.text_cell_width(grapheme) > 1;
            container.child(
                div()
                    .flex_none()
                    .font(if is_wide {
                        wide_font.clone()
                    } else {
                        normal_font.clone()
                    })
                    .text_size(if is_wide { wide_size } else { normal_size })
                    .child(grapheme.to_owned()),
            )
        },
    )
}

#[cfg(test)]
#[allow(dead_code)]
pub(super) fn nvim_mouse_position(
    position: gpui::Point<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
) -> (u64, u64) {
    let (row, col) =
        compositor::CompositorFrame::point_in_grid_space(position, cell_width, line_height);
    (row.max(0.0).floor() as u64, col.max(0.0).floor() as u64)
}

impl EditorRuntime {
    fn mouse_target_at(
        &mut self,
        position: gpui::Point<Pixels>,
        window: &mut Window,
    ) -> Option<compositor::MouseTarget> {
        let gui_font = self.current_grid_font(window);
        let cell_width = gui_font.cell_width(window);
        let line_height = gui_font.line_height(window, self.protocol.linespace);
        let presentation = self.presentation_snapshot();
        let target = presentation
            .compositor
            .hit_test(position, cell_width, line_height);
        self.map_mouse_target_through_ime(target)
    }

    fn mouse_target_for_grid(
        &mut self,
        grid_id: u64,
        position: gpui::Point<Pixels>,
        window: &mut Window,
    ) -> Option<compositor::MouseTarget> {
        let gui_font = self.current_grid_font(window);
        let cell_width = gui_font.cell_width(window);
        let line_height = gui_font.line_height(window, self.protocol.linespace);
        let presentation = self.presentation_snapshot();
        let target =
            presentation
                .compositor
                .target_for_grid(grid_id, position, cell_width, line_height);
        self.map_mouse_target_through_ime(target)
    }

    fn map_mouse_target_through_ime(
        &self,
        target: Option<compositor::MouseTarget>,
    ) -> Option<compositor::MouseTarget> {
        let mut target = target?;
        let Some(composition_grid) = self.composition_grid() else {
            return Some(target);
        };
        let Some(composition) = self.active_ime_composition() else {
            return Some(target);
        };
        let composition_row = u64::try_from(composition.row).unwrap_or(u64::MAX);
        if target.grid_id != composition_grid || target.row != composition_row {
            return Some(target);
        }

        let composition_col = u64::try_from(composition.col).unwrap_or(u64::MAX);
        let composition_width = u64::try_from(composition.grid_width).unwrap_or(u64::MAX);
        let gap_end = composition_col.saturating_add(composition_width);
        if target.col >= gap_end {
            target.col = target.col.saturating_sub(composition_width);
        } else if target.col >= composition_col {
            // A click inside the visual gap still means the Neovim cursor
            // position at which the composition is anchored.
            target.col = composition_col;
        }
        Some(target)
    }
}

impl NvimGpui {
    fn send_mouse(
        &mut self,
        button: &str,
        action: &str,
        modifiers: gpui::Modifiers,
        target: Option<compositor::MouseTarget>,
    ) {
        let Some(target) = target else {
            return;
        };
        if !self.editor.input.mouse_enabled {
            return;
        }
        let modifier = input::nvim_mouse_modifiers(modifiers);
        if let Some(nvim) = self.app.session.nvim.as_ref() {
            if let Err(error) = nvim.send_mouse(
                button,
                action,
                modifier,
                target.grid_id,
                target.row,
                target.col,
            ) {
                log::error!(
                    target: "nvim_gpui::input",
                    "mouse event failed: button={button}, action={action}, grid={}, row={}, col={}: {error}",
                    target.grid_id,
                    target.row,
                    target.col
                );
                self.app.session.rpc_status = format!("rpc mouse error: {error}");
            }
        }
    }

    pub(super) fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if cx.has_active_drag() {
            window.prevent_default();
            return;
        }

        if let Some(focus_handle) = self.window.focus_handle.as_ref() {
            window.focus(focus_handle);
        }
        let target = self.editor.mouse_target_at(event.position, window);
        self.editor.input.mouse_capture = target.map(|target| target.grid_id);
        self.send_mouse(
            input::nvim_mouse_button(event.button),
            "press",
            event.modifiers,
            target,
        );
        window.prevent_default();
    }

    pub(super) fn on_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if cx.has_active_drag() {
            window.prevent_default();
            return;
        }

        let target = self
            .editor
            .input
            .mouse_capture
            .take()
            .and_then(|grid_id| {
                self.editor
                    .mouse_target_for_grid(grid_id, event.position, window)
            })
            .or_else(|| self.editor.mouse_target_at(event.position, window));
        self.send_mouse(
            input::nvim_mouse_button(event.button),
            "release",
            event.modifiers,
            target,
        );
        window.prevent_default();
    }

    pub(super) fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if cx.has_active_drag() {
            window.prevent_default();
            return;
        }

        let (button, action) = event
            .pressed_button
            .map(|button| (input::nvim_mouse_button(button), "drag"))
            .unwrap_or(("move", "move"));
        let target = self
            .editor
            .input
            .mouse_capture
            .and_then(|grid_id| {
                self.editor
                    .mouse_target_for_grid(grid_id, event.position, window)
            })
            .or_else(|| self.editor.mouse_target_at(event.position, window));
        self.send_mouse(button, action, event.modifiers, target);
        window.prevent_default();
    }

    pub(super) fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        if !self.editor.input.mouse_enabled {
            return;
        }

        let gui_font = self.editor.current_grid_font(window);
        let cell_width = gui_font.cell_width(window);
        let line_height = gui_font.line_height(window, self.editor.protocol.linespace);
        let mut delta = input::scroll_delta_to_lines(event.delta, line_height);

        // Shift-wheel is conventionally horizontal. Windows' GPUI backend
        // already performs this conversion, while macOS/Linux may expose
        // the original vertical axis, so keep the behavior consistent.
        if event.modifiers.shift && delta.x.abs() < f32::EPSILON {
            delta.x = delta.y;
            delta.y = 0.0;
        }
        self.editor.input.scroll_remainder.x += delta.x;
        self.editor.input.scroll_remainder.y += delta.y;

        let x_steps = self.editor.input.scroll_remainder.x.trunc() as i32;
        let y_steps = self.editor.input.scroll_remainder.y.trunc() as i32;
        self.editor.input.scroll_remainder.x -= x_steps as f32;
        self.editor.input.scroll_remainder.y -= y_steps as f32;
        let modifier = input::nvim_mouse_modifiers(event.modifiers);
        let presentation = self.editor.presentation_snapshot();
        let target = presentation
            .compositor
            .hit_test(event.position, cell_width, line_height);

        if let Some(nvim) = self.app.session.nvim.as_ref() {
            let Some(target) = target else {
                window.prevent_default();
                return;
            };
            for _ in 0..x_steps.unsigned_abs() {
                let action = if x_steps > 0 { "right" } else { "left" };
                if let Err(error) = nvim.send_mouse(
                    "wheel",
                    action,
                    modifier.clone(),
                    target.grid_id,
                    target.row,
                    target.col,
                ) {
                    log::error!(
                        target: "nvim_gpui::input",
                        "horizontal wheel event failed: action={action}, grid={}, row={}, col={}: {error}",
                        target.grid_id,
                        target.row,
                        target.col
                    );
                    self.app.session.rpc_status = format!("rpc mouse error: {error}");
                    break;
                }
            }
            for _ in 0..y_steps.unsigned_abs() {
                let action = if y_steps > 0 { "up" } else { "down" };
                if let Err(error) = nvim.send_mouse(
                    "wheel",
                    action,
                    modifier.clone(),
                    target.grid_id,
                    target.row,
                    target.col,
                ) {
                    log::error!(
                        target: "nvim_gpui::input",
                        "vertical wheel event failed: action={action}, grid={}, row={}, col={}: {error}",
                        target.grid_id,
                        target.row,
                        target.col
                    );
                    self.app.session.rpc_status = format!("rpc mouse error: {error}");
                    break;
                }
            }
        }
        window.prevent_default();
    }
}
