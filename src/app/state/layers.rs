use super::*;

impl NvimGpui {
    pub(crate) fn visible_grid_layers(&self) -> Vec<(u64, Rc<grid::GridModel>, GridPlacement)> {
        let mut layers = self
            .other_grids
            .iter()
            .filter_map(|(grid, model)| {
                let placement = self.grid_placements.get(grid).copied()?;
                placement
                    .visible
                    .then(|| (*grid, Rc::clone(model), placement))
            })
            .collect::<Vec<_>>();
        layers.sort_by(|left, right| {
            left.2
                .compindex
                .cmp(&right.2.compindex)
                .then_with(|| left.2.z_index.cmp(&right.2.z_index))
                .then_with(|| left.0.cmp(&right.0))
        });
        layers
    }

    pub(crate) fn visible_image_layers(&self) -> Vec<ImageLayer> {
        let mut layers = Vec::new();

        for placement in self.image_store.placements() {
            if placement.is_virtual_placeholder()
                || self.image_store.asset(placement.key.image).is_none()
                || !self.grid_is_visible(placement.anchor.grid.0)
            {
                continue;
            }
            layers.push(ImageLayer {
                image: placement.key.image,
                grid: placement.anchor.grid.0,
                row: placement.anchor.row as usize,
                column: placement.anchor.column as usize,
                columns: placement.columns,
                rows: placement.rows,
                z_index: placement.z_index,
            });
        }

        // Most frames have no Kitty placeholder at all. Avoid walking every
        // visible grid in that common case. Build the lookup once as well so
        // placeholder cells do not rescan every image placement individually.
        if !self.image_store.has_virtual_placements() {
            layers.sort_by(|left, right| {
                left.z_index
                    .cmp(&right.z_index)
                    .then_with(|| left.grid.cmp(&right.grid))
                    .then_with(|| left.row.cmp(&right.row))
                    .then_with(|| left.column.cmp(&right.column))
            });
            return layers;
        }

        let virtual_image_sizes = self
            .image_store
            .virtual_placements()
            .filter_map(|placement| {
                self.image_store
                    .asset(placement.key.image)
                    .is_some()
                    .then_some((
                        placement.key.image,
                        (placement.columns, placement.rows, placement.z_index),
                    ))
            })
            .collect::<HashMap<_, _>>();
        let mut models = vec![(1, self.grid.as_ref())];
        models.extend(self.other_grids.iter().filter_map(|(grid, model)| {
            self.grid_is_visible(*grid)
                .then_some((*grid, model.as_ref()))
        }));

        let mut virtual_layer_keys = HashSet::new();

        for (grid, model) in &models {
            for (row, grid_row) in model.rows().iter().enumerate() {
                for (column, cell) in grid_row.cells().iter().enumerate() {
                    let Some((row_offset, column_offset)) =
                        image_store::placeholder_position(&cell.text)
                    else {
                        continue;
                    };
                    let Some(image) = model
                        .highlight(cell.highlight)
                        .and_then(|attrs| attrs.foreground)
                        .map(ImageId)
                    else {
                        continue;
                    };
                    let Some(&(columns, rows, z_index)) = virtual_image_sizes.get(&image) else {
                        continue;
                    };

                    // A placeholder is rendered through a Neovim virtual
                    // text/line. Another decoration (for example Markview's
                    // concealed title text) can cover the first placeholder
                    // cell while leaving the rest of the image intact. Do
                    // not require the (1, 1) marker: every marker encodes its
                    // own offset, so any visible cell can recover the image
                    // anchor.
                    let Some(row) = row.checked_sub(row_offset.saturating_sub(1) as usize) else {
                        continue;
                    };
                    let Some(column) = column.checked_sub(column_offset.saturating_sub(1) as usize)
                    else {
                        continue;
                    };
                    // Snacks intentionally hides an inline image while the
                    // cursor is on the source line (hybrid/conceal mode).
                    // With `virt_lines`, the Kitty placeholder begins on the
                    // line immediately below that source line.
                    // The placeholder cells can remain in the redraw model
                    // because another decoration may cover only their first
                    // cell, so use Neovim's cursor row as the visibility
                    // signal instead of treating a partial placeholder as a
                    // complete preview.
                    let source_row = row.saturating_sub(1);
                    if self.cursor_grid == *grid
                        && model
                            .cursor()
                            .is_some_and(|cursor| cursor.row == source_row)
                    {
                        continue;
                    }
                    if !virtual_layer_keys.insert((image, *grid, row, column)) {
                        continue;
                    }
                    layers.push(ImageLayer {
                        image,
                        grid: *grid,
                        row,
                        column,
                        columns,
                        rows,
                        z_index,
                    });
                }
            }
        }

        layers.sort_by(|left, right| {
            left.z_index
                .cmp(&right.z_index)
                .then_with(|| left.grid.cmp(&right.grid))
                .then_with(|| left.row.cmp(&right.row))
                .then_with(|| left.column.cmp(&right.column))
        });
        layers
    }

    pub(crate) fn grid_is_visible(&self, grid: u64) -> bool {
        grid == 1
            || self
                .grid_placements
                .get(&grid)
                .is_some_and(|placement| placement.visible)
    }
}
