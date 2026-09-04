use super::*;

impl NvimGpui {
    pub(crate) fn pending_grid_mut(&mut self) -> &mut grid::GridModel {
        let pending = self
            .pending_grid
            .get_or_insert_with(|| Rc::clone(&self.grid));
        Rc::make_mut(pending)
    }

    pub(crate) fn new_styled_grid(&self, width: usize, height: usize) -> Rc<grid::GridModel> {
        let source = self.pending_grid.as_deref().unwrap_or(self.grid.as_ref());
        let mut next_grid = grid::GridModel::new(width, height);
        for (id, attrs) in source.highlights() {
            next_grid.set_highlight(*id, attrs.clone());
        }
        let (foreground, background, special) = source.default_colors();
        next_grid.set_default_colors(foreground, background, special);
        Rc::new(next_grid)
    }

    pub(crate) fn pending_grid_mut_for(&mut self, grid: u64) -> &mut grid::GridModel {
        if grid == 1 {
            return self.pending_grid_mut();
        }

        if !self.pending_other_grids.contains_key(&grid) {
            let model = self
                .other_grids
                .get(&grid)
                .cloned()
                .unwrap_or_else(|| self.new_styled_grid(0, 0));
            self.pending_other_grids.insert(grid, model);
        }

        let pending = self
            .pending_other_grids
            .get_mut(&grid)
            .expect("pending grid was inserted");
        Rc::make_mut(pending)
    }

    pub(crate) fn set_default_colors_on_all_grids(
        &mut self,
        foreground: Option<u32>,
        background: Option<u32>,
        special: Option<u32>,
    ) {
        self.pending_grid_mut()
            .set_default_colors(foreground, background, special);
        let grid_ids = self.other_grids.keys().copied().collect::<Vec<_>>();
        for grid in grid_ids {
            self.pending_grid_mut_for(grid)
                .set_default_colors(foreground, background, special);
        }
        for (grid, model) in &mut self.pending_other_grids {
            if !self.other_grids.contains_key(grid) {
                Rc::make_mut(model).set_default_colors(foreground, background, special);
            }
        }
    }

    pub(crate) fn set_highlight_on_all_grids(
        &mut self,
        id: grid::HighlightId,
        attrs: grid::HighlightAttrs,
    ) {
        self.pending_grid_mut().set_highlight(id, attrs.clone());
        let grid_ids = self.other_grids.keys().copied().collect::<Vec<_>>();
        for grid in grid_ids {
            self.pending_grid_mut_for(grid)
                .set_highlight(id, attrs.clone());
        }
        for (grid, model) in &mut self.pending_other_grids {
            if !self.other_grids.contains_key(grid) {
                Rc::make_mut(model).set_highlight(id, attrs.clone());
            }
        }
    }

    pub(crate) fn discard_pending_redraw(&mut self) {
        self.pending_grid = None;
        self.pending_other_grids.clear();
        self.pending_grid_placements.clear();
        self.pending_destroyed_grids.clear();
        self.pending_cursor_grid = None;
        self.pending_theme = None;
        self.pending_redraw = None;
    }

    pub(crate) fn set_grid_placement(&mut self, grid: u64, placement: GridPlacement) {
        self.pending_grid_placements.insert(grid, placement);
        self.pending_destroyed_grids.remove(&grid);
    }

    pub(crate) fn grid_placement(&self, grid: u64) -> GridPlacement {
        self.pending_grid_placements
            .get(&grid)
            .copied()
            .or_else(|| self.grid_placements.get(&grid).copied())
            .unwrap_or_default()
    }

    pub(crate) fn commit_pending_grid(&mut self) {
        if self.pending_cursor_grid.is_some() {
            self.update_cursor_animation();
        }

        if let Some(grid) = self.pending_grid.take() {
            self.start_viewport_animation(1, Rc::clone(&self.grid), Rc::clone(&grid));

            if let Some(cursor) = grid.cursor() {
                self.state.line = cursor.row + 1;
                self.state.column = cursor.col + 1;
            }
            self.grid = grid;
        }

        for (grid, model) in std::mem::take(&mut self.pending_other_grids) {
            if !self.pending_destroyed_grids.contains(&grid) {
                if let Some(previous) = self.other_grids.get(&grid).cloned() {
                    self.start_viewport_animation(grid, previous, Rc::clone(&model));
                }
                self.other_grids.insert(grid, model);
            }
        }

        for (grid, placement) in std::mem::take(&mut self.pending_grid_placements) {
            if !self.pending_destroyed_grids.contains(&grid) {
                self.grid_placements.insert(grid, placement);
            }
        }

        for grid in std::mem::take(&mut self.pending_destroyed_grids) {
            self.other_grids.remove(&grid);
            self.grid_placements.remove(&grid);
            self.viewport_animations.remove(&grid);
        }

        if let Some(grid) = self.pending_cursor_grid.take() {
            self.cursor_grid = grid;
        }
    }

    fn update_cursor_animation(&mut self) {
        let previous = self.current_cursor_screen_position();
        let next = self.pending_cursor_screen_position();

        self.cursor_animation = match (previous, next) {
            (Some(from), Some(target)) if from != target => self
                .cursor_animation
                .map(|animation| animation.retarget(target))
                .or_else(|| Some(grid::CursorAnimation::new(from, target))),
            _ => None,
        };
    }

    pub(crate) fn current_cursor_screen_position(&self) -> Option<grid::CursorVisualPosition> {
        let model = self.active_cursor_model()?;
        let placement = if self.cursor_grid == 1 {
            self.grid_placements
                .get(&self.cursor_grid)
                .copied()
                .unwrap_or_default()
        } else {
            self.grid_placements.get(&self.cursor_grid).copied()?
        };
        Self::cursor_screen_position(&model, placement)
    }

    /// Return the cursor in the local coordinate system of the grid that owns
    /// the currently registered system IME handler. The handler's
    /// `element_bounds` already includes the grid's screen placement, so the
    /// caller must add only this local position.
    pub(crate) fn ime_cursor_position(&self) -> Option<grid::CursorVisualPosition> {
        let grid = self.ime_input_grid?;
        let model = if grid == 1 {
            self.grid.as_ref()
        } else {
            self.other_grids.get(&grid)?.as_ref()
        };
        model.cursor_visual_position()
    }

    fn pending_cursor_screen_position(&self) -> Option<grid::CursorVisualPosition> {
        let grid = self.pending_cursor_grid.unwrap_or(self.cursor_grid);
        let model = if grid == 1 {
            self.pending_grid.as_ref().unwrap_or(&self.grid)
        } else {
            self.pending_other_grids
                .get(&grid)
                .or_else(|| self.other_grids.get(&grid))?
        };
        let placement = self.grid_placement(grid);
        Self::cursor_screen_position(model, placement)
    }

    fn cursor_screen_position(
        model: &grid::GridModel,
        placement: GridPlacement,
    ) -> Option<grid::CursorVisualPosition> {
        let position = model.cursor_visual_position()?;
        let row = placement.row.checked_add(position.row as i64)?;
        let col = placement.col.checked_add(position.col as i64)?;
        (row >= 0 && col >= 0).then_some(grid::CursorVisualPosition {
            row: row as usize,
            col: col as usize,
            width: position.width,
        })
    }

    pub(crate) fn active_cursor_model(&self) -> Option<Rc<grid::GridModel>> {
        if self.cursor_grid == 1 {
            Some(Rc::clone(&self.grid))
        } else {
            self.other_grids.get(&self.cursor_grid).cloned()
        }
    }

    fn start_viewport_animation(
        &mut self,
        grid: u64,
        previous_grid: Rc<grid::GridModel>,
        next_grid: Rc<grid::GridModel>,
    ) {
        let Some(viewport) = self
            .pending_grid_placements
            .get(&grid)
            .and_then(|placement| placement.viewport)
        else {
            return;
        };
        let Some(previous_placement) = self.grid_placements.get(&grid).copied() else {
            return;
        };
        let Some(next_placement) = self.pending_grid_placements.get(&grid).copied() else {
            return;
        };

        if previous_placement.viewport.is_none()
            || previous_placement.viewport_margins != next_placement.viewport_margins
            || previous_placement.row != next_placement.row
            || previous_placement.col != next_placement.col
            || previous_placement.width != next_placement.width
            || previous_placement.height != next_placement.height
            || previous_placement.z_index != next_placement.z_index
            || previous_placement.compindex != next_placement.compindex
            || previous_placement.visible != next_placement.visible
        {
            self.viewport_animations.remove(&grid);
            return;
        }

        if viewport.scroll_delta == 0
            || previous_grid.width() != next_grid.width()
            || previous_grid.height() != next_grid.height()
        {
            self.viewport_animations.remove(&grid);
            return;
        }

        self.viewport_animations.insert(
            grid,
            ViewportAnimation {
                previous_grid,
                scroll_delta: viewport.scroll_delta,
                started_at: Instant::now(),
                presented: false,
            },
        );
    }
}
