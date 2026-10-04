use super::*;
use super::{GridCommit, GridPlacement, MultiCursorPosition};
use std::collections::HashSet;

impl EditorRuntime {
    pub(crate) fn apply_viewport_commits(&mut self, commits: Vec<GridCommit>) {
        for commit in commits {
            self.presentation
                .grid_dirty_regions
                .entry(commit.grid)
                .or_default()
                .merge(&commit.dirty_region);
            if self
                .presentation
                .viewport_animations
                .get(&commit.grid)
                .is_some_and(|animation| !animation.presented)
            {
                // The old animation's previous grid was never displayed.
                // Starting another transition from the intermediate commit
                // would put an unseen, already-obsolete frame on screen.
                self.presentation.viewport_animations.remove(&commit.grid);
                self.presentation
                    .scroll_animation_suppressed
                    .insert(commit.grid);
            }
            if self
                .presentation
                .scroll_animation_suppressed
                .contains(&commit.grid)
            {
                continue;
            }
            if !self.scrolling_animation_enabled {
                self.presentation.viewport_animations.remove(&commit.grid);
                continue;
            }
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
                    previous_images: self
                        .presentation
                        .last_presented_snapshot
                        .as_ref()
                        .map(|snapshot| {
                            snapshot
                                .image_layers
                                .iter()
                                .copied()
                                .filter(|image| image.grid == commit.grid)
                                .collect()
                        })
                        .unwrap_or_default(),
                    previous_image_sources: self
                        .presentation
                        .last_presented_snapshot
                        .as_ref()
                        .map(|snapshot| snapshot.image_sources.clone())
                        .unwrap_or_default(),
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

    #[cfg(test)]
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
        if !self.cursor.cursor_animation_enabled {
            self.cursor.cursor_animation = None;
            return;
        }

        let now = Instant::now();
        let next = self.current_cursor_screen_position();
        self.cursor.cursor_animation = match (previous, next) {
            (Some(from), Some(target)) if from != target => self
                .cursor
                .cursor_animation
                .filter(|animation| animation.is_recent(now))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::protocol::GridViewport;
    use std::rc::Rc;

    #[test]
    fn back_to_back_scroll_commits_skip_an_unpresented_old_frame() {
        let mut editor = EditorRuntime::default();
        let grid = Rc::new(crate::grid::GridModel::new(8, 8));
        let placement = GridPlacement {
            viewport: Some(GridViewport {
                topline: 1,
                botline: 9,
                curline: 1,
                curcol: 0,
                line_count: 100,
                scroll_delta: 1,
            }),
            ..GridPlacement::default()
        };
        let commit = || GridCommit {
            grid: 2,
            previous_grid: Rc::clone(&grid),
            next_grid: Rc::clone(&grid),
            previous_placement: Some(placement),
            next_placement: Some(placement),
            dirty_region: GridDirtyRegion::default(),
        };

        let image = ImageLayer {
            image: ImageId(9),
            grid: 2,
            row: 3,
            column: 1,
            columns: 4,
            rows: 2,
            pixel_width: None,
            pixel_height: None,
            z_index: 0,
        };
        let source = Arc::new(Image::from_bytes(gpui::ImageFormat::Png, Vec::new()));
        editor.presentation.last_presented_snapshot =
            Some(Rc::new(compositor::PresentationSnapshot {
                compositor: editor.compositor_frame(),
                image_layers: vec![image],
                image_sources: HashMap::from([(image.image, Arc::clone(&source))]),
            }));

        editor.apply_viewport_commits(vec![commit()]);
        assert!(editor.presentation.viewport_animations.contains_key(&2));
        let animation = &editor.presentation.viewport_animations[&2];
        assert_eq!(animation.previous_images, vec![image]);
        assert!(Arc::ptr_eq(
            &animation.previous_image_sources[&image.image],
            &source
        ));
        editor.apply_viewport_commits(vec![commit()]);
        assert!(!editor.presentation.viewport_animations.contains_key(&2));
        assert!(editor.presentation.scroll_animation_suppressed.contains(&2));
        editor.apply_viewport_commits(vec![commit()]);
        assert!(!editor.presentation.viewport_animations.contains_key(&2));

        editor.presentation.scroll_animation_suppressed.clear();
        editor.apply_viewport_commits(vec![commit()]);
        assert!(editor.presentation.viewport_animations.contains_key(&2));
    }

    #[test]
    fn viewport_commits_accumulate_dirty_regions_for_the_same_grid() {
        let mut editor = EditorRuntime::default();
        let grid = Rc::new(crate::grid::GridModel::new(8, 8));

        let commit = |top| GridCommit {
            grid: 2,
            previous_grid: Rc::clone(&grid),
            next_grid: Rc::clone(&grid),
            previous_placement: None,
            next_placement: None,
            dirty_region: GridDirtyRegion {
                full: false,
                rects: vec![crate::editor::protocol::GridDirtyRect {
                    top,
                    bottom: top + 1,
                    left: 0,
                    right: 4,
                }],
            },
        };

        editor.apply_viewport_commits(vec![commit(1), commit(6)]);

        assert_eq!(
            editor
                .presentation
                .grid_dirty_regions
                .get(&2)
                .expect("dirty region should be retained")
                .rects,
            vec![
                crate::editor::protocol::GridDirtyRect {
                    top: 1,
                    bottom: 2,
                    left: 0,
                    right: 4,
                },
                crate::editor::protocol::GridDirtyRect {
                    top: 6,
                    bottom: 7,
                    left: 0,
                    right: 4,
                },
            ]
        );
    }
}
