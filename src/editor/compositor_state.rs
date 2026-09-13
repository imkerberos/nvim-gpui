use super::*;
use super::{GridCommit, GridPlacement, MultiCursorPosition};
use std::collections::HashSet;

impl EditorRuntime {
    pub(crate) fn apply_viewport_commits(&mut self, commits: Vec<GridCommit>) {
        for commit in commits {
            let Some(previous_placement) = commit.previous_placement else {
                continue;
            };
            let Some(next_placement) = commit.next_placement else {
                continue;
            };
            let Some(viewport) = next_placement.viewport else {
                continue;
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
                || viewport.scroll_delta == 0
                || commit.previous_grid.width() != commit.next_grid.width()
                || commit.previous_grid.height() != commit.next_grid.height()
            {
                self.presentation.viewport_animations.remove(&commit.grid);
                continue;
            }

            self.presentation.viewport_animations.insert(
                commit.grid,
                ViewportAnimation {
                    previous_grid: commit.previous_grid,
                    scroll_delta: viewport.scroll_delta,
                    started_at: Instant::now(),
                    presented: false,
                },
            );
        }
    }

    pub(crate) fn visible_multicursor_positions(&self) -> Vec<MultiCursorPosition> {
        let tracking_ns = self
            .protocol
            .cursor
            .multicursor_namespace_ids
            .get("nvim.multicursor")
            .copied();
        let display_ns = self
            .protocol
            .cursor
            .multicursor_namespace_ids
            .get("nvim.multicursor.cursor")
            .copied();
        let display_grids = display_ns
            .into_iter()
            .flat_map(|ns_id| {
                self.protocol
                    .cursor
                    .multicursor_positions
                    .values()
                    .filter(move |position| position.key.ns_id == ns_id)
                    .map(|position| position.key.grid)
            })
            .collect::<HashSet<_>>();

        self.protocol
            .cursor
            .multicursor_positions
            .values()
            .copied()
            .filter(|position| {
                self.grid_is_visible(position.key.grid)
                    && match (tracking_ns, display_ns) {
                        (_, Some(ns_id)) if position.key.ns_id == ns_id => true,
                        (Some(ns_id), _) if position.key.ns_id == ns_id => {
                            !display_grids.contains(&position.key.grid)
                        }
                        _ => false,
                    }
            })
            .collect()
    }

    pub(crate) fn grid_placement(&self, grid: u64) -> GridPlacement {
        self.protocol.grid_placement(grid)
    }

    pub(crate) fn discard_pending_redraw(&mut self) {
        self.protocol.discard_pending_redraw();
    }

    pub(crate) fn grid_is_visible(&self, grid: u64) -> bool {
        self.protocol.grid_is_visible(grid)
    }

    pub(crate) fn current_cursor_mode(&self) -> grid::CursorModeInfo {
        if !self.protocol.cursor.cursor_style_enabled {
            return grid::CursorModeInfo::default();
        }
        self.protocol
            .cursor
            .cursor_modes
            .get(self.protocol.cursor.cursor_mode_index)
            .copied()
            .unwrap_or_default()
    }

    pub(crate) fn theme_background(&self) -> u32 {
        self.protocol
            .theme
            .normal_background
            .or(self.protocol.theme.default_background)
            .unwrap_or(crate::widgets::BACKGROUND)
    }

    pub(crate) fn theme_foreground(&self) -> u32 {
        self.protocol
            .theme
            .normal_foreground
            .or(self.protocol.theme.default_foreground)
            .unwrap_or(crate::widgets::TEXT)
    }

    pub(crate) fn update_startup_grid_ready(&mut self) {
        if self.protocol.startup.nvim_grid_ready
            || !self.protocol.startup.flush_seen
            || !self.protocol.startup.grid_content_seen
        {
            return;
        }

        let Some(target) = self.protocol.startup.resize_target else {
            return;
        };
        let committed_size = (
            self.protocol.presentation.grid.width() as u32,
            self.protocol.presentation.grid.height() as u32,
        );
        if committed_size == target {
            self.protocol.startup.nvim_grid_ready = true;
        }
    }

    pub(crate) fn complete_startup_maximize(&mut self) {
        self.protocol.startup.maximize_pending = false;
        self.protocol.startup.resize_target = None;
        self.protocol.startup.flush_seen = false;
        self.protocol.startup.grid_content_seen = false;
        self.protocol.startup.redraw_pending = true;
        log::debug!(
            target: "nvim_gpui::app",
            "startup window maximized; waiting for the final Neovim grid"
        );
    }

    pub(crate) fn current_cursor_screen_position(&self) -> Option<grid::CursorVisualPosition> {
        let model = self.active_cursor_model()?;
        let grid = self.protocol.cursor.cursor_grid;
        let placement = if grid == 1 {
            self.protocol
                .presentation
                .grid_placements
                .get(&grid)
                .copied()
                .unwrap_or_default()
        } else {
            self.protocol
                .presentation
                .grid_placements
                .get(&grid)
                .copied()?
        };
        Self::cursor_screen_position(&model, placement)
    }

    /// Return the cursor in the local coordinate system of the grid that owns
    /// the currently registered system IME handler.
    pub(crate) fn ime_cursor_position(&self) -> Option<grid::CursorVisualPosition> {
        let grid = self.input.ime_input_grid?;
        let model = if grid == 1 {
            self.protocol.presentation.grid.as_ref()
        } else {
            self.protocol.presentation.other_grids.get(&grid)?.as_ref()
        };
        model.cursor_visual_position()
    }

    pub(crate) fn active_cursor_model(&self) -> Option<Rc<grid::GridModel>> {
        let grid = self.protocol.cursor.cursor_grid;
        if grid == 1 {
            Some(Rc::clone(&self.protocol.presentation.grid))
        } else {
            self.protocol.presentation.other_grids.get(&grid).cloned()
        }
    }

    pub(crate) fn update_cursor_animation_from(
        &mut self,
        previous: Option<grid::CursorVisualPosition>,
    ) {
        let next = self.current_cursor_screen_position();
        self.cursor.cursor_animation = match (previous, next) {
            (Some(from), Some(target)) if from != target => self
                .cursor
                .cursor_animation
                .map(|animation| animation.retarget(target))
                .or_else(|| Some(grid::CursorAnimation::new(from, target))),
            _ => None,
        };
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
}
