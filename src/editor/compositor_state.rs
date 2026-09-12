use super::*;
use super::{GridCommit, GridPlacement, MultiCursorPosition};
use std::collections::HashSet;

impl NvimGpui {
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
                self.editor
                    .presentation
                    .viewport_animations
                    .remove(&commit.grid);
                continue;
            }

            self.editor.presentation.viewport_animations.insert(
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
            .editor
            .protocol
            .cursor
            .multicursor_namespace_ids
            .get("nvim.multicursor")
            .copied();
        let display_ns = self
            .editor
            .protocol
            .cursor
            .multicursor_namespace_ids
            .get("nvim.multicursor.cursor")
            .copied();
        let display_grids = display_ns
            .into_iter()
            .flat_map(|ns_id| {
                self.editor
                    .protocol
                    .cursor
                    .multicursor_positions
                    .values()
                    .filter(move |position| position.key.ns_id == ns_id)
                    .map(|position| position.key.grid)
            })
            .collect::<HashSet<_>>();

        self.editor
            .protocol
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
        self.editor.protocol.grid_placement(grid)
    }

    pub(crate) fn discard_pending_redraw(&mut self) {
        self.editor.protocol.discard_pending_redraw();
    }

    pub(crate) fn current_cursor_screen_position(&self) -> Option<grid::CursorVisualPosition> {
        let model = self.active_cursor_model()?;
        let grid = self.editor.protocol.cursor.cursor_grid;
        let placement = if grid == 1 {
            self.editor
                .protocol
                .presentation
                .grid_placements
                .get(&grid)
                .copied()
                .unwrap_or_default()
        } else {
            self.editor
                .protocol
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
        let grid = self.editor.input.ime_input_grid?;
        let model = if grid == 1 {
            self.editor.protocol.presentation.grid.as_ref()
        } else {
            self.editor
                .protocol
                .presentation
                .other_grids
                .get(&grid)?
                .as_ref()
        };
        model.cursor_visual_position()
    }

    pub(crate) fn active_cursor_model(&self) -> Option<Rc<grid::GridModel>> {
        let grid = self.editor.protocol.cursor.cursor_grid;
        if grid == 1 {
            Some(Rc::clone(&self.editor.protocol.presentation.grid))
        } else {
            self.editor
                .protocol
                .presentation
                .other_grids
                .get(&grid)
                .cloned()
        }
    }

    pub(crate) fn update_cursor_animation_from(
        &mut self,
        previous: Option<grid::CursorVisualPosition>,
    ) {
        let next = self.current_cursor_screen_position();
        self.editor.cursor.cursor_animation = match (previous, next) {
            (Some(from), Some(target)) if from != target => self
                .editor
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

    pub(crate) fn grid_is_visible(&self, grid: u64) -> bool {
        self.editor.protocol.grid_is_visible(grid)
    }
}
