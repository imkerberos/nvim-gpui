use super::*;

/// The semantic owner of a grid layer.
///
/// Keeping this separate from the raw Neovim placement values lets the
/// compositor make decisions from context instead of inferring that a grid is
/// floating from `zindex` or `compindex`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GridLayerKind {
    Main,
    Window,
    Float,
    Message,
    External,
}

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
pub(super) struct GridRect {
    pub(super) row: i64,
    pub(super) col: i64,
    pub(super) width: u64,
    pub(super) height: u64,
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
}

/// One complete logical layer in a compositor frame.
///
/// `content_rect` describes the dimensions of the model. `surface_rect` and
/// `clip_rect` describe the actual GPUI surface currently used by the
/// renderer. Keeping the three values explicit is intentional: a float may
/// receive its position before its `grid_resize`, and a placement may omit a
/// width or height altogether.
#[derive(Debug, Clone)]
pub(super) struct CompositorLayer {
    pub(super) grid_id: u64,
    pub(super) kind: GridLayerKind,
    pub(super) model: Rc<grid::GridModel>,
    pub(super) placement: GridPlacement,
    pub(super) content_rect: GridRect,
    pub(super) surface_rect: GridRect,
    pub(super) clip_rect: GridRect,
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
pub(super) struct CompositorFrame {
    pub(super) layers: Vec<CompositorLayer>,
}

impl NvimGpui {
    /// Build the committed multigrid state in the order in which the current
    /// renderer paints it. This is deliberately pure data construction; the
    /// GPUI element tree will consume it in a later compositor step.
    pub(super) fn compositor_frame(&self) -> CompositorFrame {
        let main_model = Rc::clone(&self.grid);
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

        for (grid_id, model) in &self.other_grids {
            let Some(placement) = self.grid_placements.get(grid_id).copied() else {
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
                .then_with(|| left.placement.compindex.cmp(&right.placement.compindex))
                .then_with(|| left.placement.z_index.cmp(&right.placement.z_index))
                .then_with(|| left.grid_id.cmp(&right.grid_id))
        });

        CompositorFrame { layers }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compositor_frame_contains_main_and_visible_grids_in_paint_order() {
        let mut app = NvimGpui::default();
        app.other_grids
            .insert(2, Rc::new(grid::GridModel::new(4, 2)));
        app.other_grids
            .insert(3, Rc::new(grid::GridModel::new(6, 3)));
        app.other_grids
            .insert(4, Rc::new(grid::GridModel::new(2, 1)));
        app.grid_placements.insert(
            2,
            GridPlacement {
                kind: GridLayerKind::Float,
                visible: true,
                compindex: 10,
                ..Default::default()
            },
        );
        app.grid_placements.insert(
            3,
            GridPlacement {
                kind: GridLayerKind::Message,
                visible: true,
                compindex: 3,
                ..Default::default()
            },
        );
        app.grid_placements.insert(
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
}
