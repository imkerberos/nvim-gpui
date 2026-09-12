use super::*;
use gpui::{Pixels, Point};

impl GridLayerKind {
    fn paint_rank(self) -> u8 {
        match self {
            Self::Main => 0,
            Self::Window | Self::Float | Self::Message | Self::External => 1,
        }
    }
}

/// A rectangle expressed in Neovim grid cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GridRect {
    pub(crate) row: i64,
    pub(crate) col: i64,
    pub(crate) width: u64,
    pub(crate) height: u64,
}

impl GridRect {
    fn from_placement(placement: GridPlacement, width: u64, height: u64) -> Self {
        Self {
            row: placement.row,
            col: placement.col,
            width,
            height,
        }
    }

    fn contains(self, row: f32, col: f32) -> bool {
        row >= self.row as f32
            && col >= self.col as f32
            && row < self.row as f32 + self.height as f32
            && col < self.col as f32 + self.width as f32
    }
}

/// One complete logical layer in a compositor frame.
///
/// `content_rect` describes the dimensions of the model. `surface_rect` and
/// `clip_rect` describe the actual GPUI surface currently used by the
/// renderer. Keeping the three values explicit is intentional: a float may
/// receive its position before its `grid_resize`, and a placement may omit a
/// width or height altogether.
#[derive(Debug, Clone)]
pub(crate) struct CompositorLayer {
    pub(crate) grid_id: u64,
    pub(crate) kind: GridLayerKind,
    pub(crate) model: Rc<grid::GridModel>,
    pub(crate) placement: GridPlacement,
    pub(crate) content_rect: GridRect,
    pub(crate) surface_rect: GridRect,
    pub(crate) clip_rect: GridRect,
}

impl CompositorLayer {
    fn new(
        grid_id: u64,
        kind: GridLayerKind,
        model: Rc<grid::GridModel>,
        placement: GridPlacement,
    ) -> Self {
        let content_width = model.width() as u64;
        let content_height = model.height() as u64;
        // A float's win_float_pos event carries position and stacking data,
        // while its dimensions arrive through grid_resize. Resolve that
        // protocol split once here instead of making every renderer decide
        // how to combine the two values.
        let surface_width = placement.width.max(content_width);
        let surface_height = placement.height.max(content_height);
        let content_rect = GridRect::from_placement(placement, content_width, content_height);
        let surface_rect = GridRect::from_placement(placement, surface_width, surface_height);

        Self {
            grid_id,
            kind,
            model,
            placement,
            content_rect,
            // Preserve the current renderer's effective surface dimensions in
            // this first extraction. A later compositor change can alter the
            // clipping policy independently and prove it with its own tests.
            clip_rect: surface_rect,
            surface_rect,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CompositorFrame {
    pub(crate) layers: Vec<CompositorLayer>,
}

/// The immutable presentation view published from the last completed
/// Neovim redraw transaction.
///
/// The protocol reducer owns the mutable committed state. Rendering and
/// pointer routing consume this view so they cannot independently rebuild
/// layer geometry from different pieces of application state.
#[derive(Clone)]
pub(crate) struct PresentationSnapshot {
    pub(crate) compositor: CompositorFrame,
    pub(crate) image_layers: Vec<ImageLayer>,
    pub(crate) image_sources: HashMap<ImageId, Arc<Image>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MouseTarget {
    pub(crate) grid_id: u64,
    pub(crate) row: u64,
    pub(crate) col: u64,
}

impl CompositorFrame {
    /// Convert a GPUI point into the screen grid coordinate space used by
    /// multigrid placements. Keep this next to hit testing so mouse routing
    /// and the legacy main-grid coordinate helper cannot drift apart.
    pub(crate) fn point_in_grid_space(
        position: Point<Pixels>,
        cell_width: Pixels,
        line_height: Pixels,
    ) -> (f32, f32) {
        let editor_y = f32::from(position.y)
            - if cfg!(any(
                target_os = "linux",
                target_os = "macos",
                target_os = "windows"
            )) {
                THEMED_TITLEBAR_HEIGHT
            } else {
                0.0
            };
        (
            editor_y / f32::from(line_height),
            f32::from(position.x) / f32::from(cell_width),
        )
    }

    /// Return the topmost visible layer that is allowed to receive mouse
    /// input at this screen point. Layers are stored in paint order, so the
    /// reverse traversal mirrors the visual hit-test order.
    pub(crate) fn hit_test(
        &self,
        position: Point<Pixels>,
        cell_width: Pixels,
        line_height: Pixels,
    ) -> Option<MouseTarget> {
        let (row, col) = Self::point_in_grid_space(position, cell_width, line_height);
        self.layers.iter().rev().find_map(|layer| {
            (layer.placement.mouse_enabled && layer.surface_rect.contains(row, col)).then(|| {
                MouseTarget {
                    grid_id: layer.grid_id,
                    row: (row - layer.surface_rect.row as f32).floor().max(0.0) as u64,
                    col: (col - layer.surface_rect.col as f32).floor().max(0.0) as u64,
                }
            })
        })
    }

    /// Resolve a point against a previously captured grid. Drag release and
    /// move events must continue going to the grid that received the press,
    /// even after the pointer leaves its visual rectangle.
    pub(crate) fn target_for_grid(
        &self,
        grid_id: u64,
        position: Point<Pixels>,
        cell_width: Pixels,
        line_height: Pixels,
    ) -> Option<MouseTarget> {
        let layer = self.layers.iter().find(|layer| layer.grid_id == grid_id)?;
        let (row, col) = Self::point_in_grid_space(position, cell_width, line_height);
        Some(MouseTarget {
            grid_id,
            row: (row - layer.surface_rect.row as f32).floor().max(0.0) as u64,
            col: (col - layer.surface_rect.col as f32).floor().max(0.0) as u64,
        })
    }
}

impl NvimGpui {
    pub(crate) fn presentation_snapshot(&mut self) -> Rc<PresentationSnapshot> {
        if let Some(snapshot) = &self.editor.presentation.presentation_snapshot {
            return Rc::clone(snapshot);
        }

        let snapshot = Rc::new(PresentationSnapshot {
            compositor: self.compositor_frame(),
            image_layers: self.visible_image_layers(),
            image_sources: self.editor.presentation.image_sources.clone(),
        });
        self.editor.presentation.presentation_snapshot = Some(Rc::clone(&snapshot));
        snapshot
    }

    pub(crate) fn invalidate_presentation_snapshot(&mut self) {
        self.editor.presentation.presentation_snapshot = None;
    }

    /// Build the committed multigrid state in the order in which the current
    /// renderer paints it. This is deliberately pure data construction; the
    /// GPUI element tree will consume it in a later compositor step.
    pub(crate) fn compositor_frame(&self) -> CompositorFrame {
        let main_model = Rc::clone(&self.editor.protocol.presentation.grid);
        let main_width = main_model.width() as u64;
        let main_height = main_model.height() as u64;
        let main_placement = GridPlacement {
            row: 0,
            col: 0,
            width: main_width,
            height: main_height,
            kind: GridLayerKind::Main,
            visible: true,
            ..GridPlacement::default()
        };

        let mut layers = vec![CompositorLayer::new(
            1,
            GridLayerKind::Main,
            main_model,
            main_placement,
        )];

        for (grid_id, model) in &self.editor.protocol.presentation.other_grids {
            let Some(placement) = self
                .editor
                .protocol
                .presentation
                .grid_placements
                .get(grid_id)
                .copied()
            else {
                continue;
            };
            if !placement.visible {
                continue;
            }
            layers.push(CompositorLayer::new(
                *grid_id,
                placement.kind,
                Rc::clone(model),
                placement,
            ));
        }

        layers.sort_by(|left, right| {
            left.kind
                .paint_rank()
                .cmp(&right.kind.paint_rank())
                .then_with(|| {
                    let left_has_compindex = left.placement.compindex >= 0;
                    let right_has_compindex = right.placement.compindex >= 0;
                    if left_has_compindex && right_has_compindex {
                        left.placement
                            .compindex
                            .cmp(&right.placement.compindex)
                            .then_with(|| left.placement.z_index.cmp(&right.placement.z_index))
                    } else {
                        // Neovim 0.10/0.11 do not send compindex. Their
                        // compositor orders layers by z-index, so use that
                        // legacy ordering whenever either side lacks the
                        // newer exact composition index.
                        left.placement
                            .z_index
                            .cmp(&right.placement.z_index)
                            .then_with(|| left.placement.compindex.cmp(&right.placement.compindex))
                    }
                })
                .then_with(|| left.grid_id.cmp(&right.grid_id))
        });

        CompositorFrame { layers }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nvim::NvimEvent;

    #[test]
    fn compositor_frame_contains_main_and_visible_grids_in_paint_order() {
        let mut app = NvimGpui::default();
        app.editor
            .protocol
            .presentation
            .other_grids
            .insert(2, Rc::new(grid::GridModel::new(4, 2)));
        app.editor
            .protocol
            .presentation
            .other_grids
            .insert(3, Rc::new(grid::GridModel::new(6, 3)));
        app.editor
            .protocol
            .presentation
            .other_grids
            .insert(4, Rc::new(grid::GridModel::new(2, 1)));
        app.editor.protocol.presentation.grid_placements.insert(
            2,
            GridPlacement {
                kind: GridLayerKind::Float,
                visible: true,
                compindex: 10,
                ..Default::default()
            },
        );
        app.editor.protocol.presentation.grid_placements.insert(
            3,
            GridPlacement {
                kind: GridLayerKind::Message,
                visible: true,
                compindex: 3,
                ..Default::default()
            },
        );
        app.editor.protocol.presentation.grid_placements.insert(
            4,
            GridPlacement {
                kind: GridLayerKind::Window,
                visible: false,
                compindex: 0,
                ..Default::default()
            },
        );

        let frame = app.compositor_frame();

        assert_eq!(
            frame
                .layers
                .iter()
                .map(|layer| layer.grid_id)
                .collect::<Vec<_>>(),
            vec![1, 3, 2]
        );
        assert_eq!(frame.layers[0].kind, GridLayerKind::Main);
        assert_eq!(frame.layers[1].kind, GridLayerKind::Message);
        assert_eq!(frame.layers[2].kind, GridLayerKind::Float);
    }

    #[test]
    fn compositor_uses_legacy_zindex_when_compindex_is_unavailable() {
        let mut app = NvimGpui::default();
        app.editor
            .protocol
            .presentation
            .other_grids
            .insert(2, Rc::new(grid::GridModel::new(4, 2)));
        app.editor
            .protocol
            .presentation
            .other_grids
            .insert(3, Rc::new(grid::GridModel::new(6, 3)));
        app.editor.protocol.presentation.grid_placements.insert(
            2,
            GridPlacement {
                kind: GridLayerKind::Float,
                visible: true,
                z_index: 50,
                compindex: -1,
                ..Default::default()
            },
        );
        app.editor.protocol.presentation.grid_placements.insert(
            3,
            GridPlacement {
                kind: GridLayerKind::Float,
                visible: true,
                z_index: 100,
                compindex: -1,
                ..Default::default()
            },
        );

        let frame = app.compositor_frame();

        assert_eq!(
            frame
                .layers
                .iter()
                .map(|layer| layer.grid_id)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn compositor_resolves_content_and_surface_sizes_once() {
        let model = Rc::new(grid::GridModel::new(8, 4));
        let placement = GridPlacement {
            row: 5,
            col: 7,
            width: 3,
            height: 2,
            visible: true,
            ..Default::default()
        };

        let layer = CompositorLayer::new(2, GridLayerKind::Float, model, placement);

        assert_eq!(
            layer.content_rect,
            GridRect {
                row: 5,
                col: 7,
                width: 8,
                height: 4,
            }
        );
        assert_eq!(layer.surface_rect, layer.content_rect);
        assert_eq!(layer.clip_rect, layer.surface_rect);
    }

    #[test]
    fn compositor_preserves_explicitly_larger_surface_dimensions() {
        let model = Rc::new(grid::GridModel::new(3, 2));
        let placement = GridPlacement {
            width: 10,
            height: 6,
            visible: true,
            ..Default::default()
        };

        let layer = CompositorLayer::new(2, GridLayerKind::Float, model, placement);

        assert_eq!(layer.content_rect.width, 3);
        assert_eq!(layer.content_rect.height, 2);
        assert_eq!(layer.surface_rect.width, 10);
        assert_eq!(layer.surface_rect.height, 6);
        assert_eq!(layer.clip_rect, layer.surface_rect);
    }

    #[test]
    fn hit_test_chooses_the_topmost_mouse_enabled_layer() {
        let mut app = NvimGpui::default();
        app.editor
            .protocol
            .presentation
            .other_grids
            .insert(2, Rc::new(grid::GridModel::new(10, 5)));
        app.editor
            .protocol
            .presentation
            .other_grids
            .insert(3, Rc::new(grid::GridModel::new(4, 3)));
        app.editor.protocol.presentation.grid_placements.insert(
            2,
            GridPlacement {
                row: 2,
                col: 3,
                width: 10,
                height: 5,
                kind: GridLayerKind::Float,
                visible: true,
                compindex: 1,
                ..Default::default()
            },
        );
        app.editor.protocol.presentation.grid_placements.insert(
            3,
            GridPlacement {
                row: 3,
                col: 4,
                width: 4,
                height: 3,
                kind: GridLayerKind::Float,
                visible: true,
                compindex: 2,
                ..Default::default()
            },
        );

        let frame = app.compositor_frame();
        let titlebar = if cfg!(any(
            target_os = "linux",
            target_os = "macos",
            target_os = "windows"
        )) {
            THEMED_TITLEBAR_HEIGHT
        } else {
            0.0
        };
        let target = frame
            .hit_test(point(px(45.0), px(titlebar + 60.0)), px(10.0), px(15.0))
            .expect("overlapping float should receive the click");

        assert_eq!(
            target,
            MouseTarget {
                grid_id: 3,
                row: 1,
                col: 0
            }
        );
    }

    #[test]
    fn presentation_snapshot_only_publishes_after_flush() {
        let mut app = NvimGpui::default();
        let initial = app.presentation_snapshot();
        let initial_width = initial.compositor.layers[0].content_rect.width;

        assert!(Rc::ptr_eq(&initial, &app.presentation_snapshot()));

        app.apply_nvim_event_for_test(NvimEvent::GridResized {
            grid: 1,
            width: 12,
            height: 3,
        });

        assert_eq!(
            app.presentation_snapshot().compositor.layers[0]
                .content_rect
                .width,
            initial_width
        );

        app.apply_nvim_event_for_test(NvimEvent::Flush);

        assert_eq!(
            app.presentation_snapshot().compositor.layers[0]
                .content_rect
                .width,
            12
        );
    }

    #[test]
    fn hit_test_skips_a_mouse_disabled_float() {
        let mut app = NvimGpui::default();
        app.editor
            .protocol
            .presentation
            .other_grids
            .insert(2, Rc::new(grid::GridModel::new(5, 3)));
        app.editor.protocol.presentation.grid_placements.insert(
            2,
            GridPlacement {
                row: 1,
                col: 1,
                width: 5,
                height: 3,
                kind: GridLayerKind::Float,
                mouse_enabled: false,
                visible: true,
                compindex: 2,
                ..Default::default()
            },
        );

        let frame = app.compositor_frame();
        let titlebar = if cfg!(any(
            target_os = "linux",
            target_os = "macos",
            target_os = "windows"
        )) {
            THEMED_TITLEBAR_HEIGHT
        } else {
            0.0
        };
        let target = frame
            .hit_test(point(px(15.0), px(titlebar + 30.0)), px(10.0), px(15.0))
            .expect("main grid should remain the fallback target");

        assert_eq!(target.grid_id, 1);
        assert_eq!((target.row, target.col), (2, 1));
    }

    #[test]
    fn captured_grid_keeps_receiving_pointer_coordinates_outside_its_rect() {
        let mut app = NvimGpui::default();
        app.editor
            .protocol
            .presentation
            .other_grids
            .insert(2, Rc::new(grid::GridModel::new(4, 2)));
        app.editor.protocol.presentation.grid_placements.insert(
            2,
            GridPlacement {
                row: 2,
                col: 3,
                width: 4,
                height: 2,
                kind: GridLayerKind::Float,
                visible: true,
                compindex: 1,
                ..Default::default()
            },
        );

        let frame = app.compositor_frame();
        let titlebar = if cfg!(any(
            target_os = "linux",
            target_os = "macos",
            target_os = "windows"
        )) {
            THEMED_TITLEBAR_HEIGHT
        } else {
            0.0
        };
        let target = frame
            .target_for_grid(
                2,
                point(px(100.0), px(titlebar + 120.0)),
                px(10.0),
                px(15.0),
            )
            .expect("captured visible grid should resolve outside its bounds");

        assert_eq!(
            target,
            MouseTarget {
                grid_id: 2,
                row: 6,
                col: 7
            }
        );
    }
}
