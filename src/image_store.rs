//! Protocol-neutral image state.
//!
//! Kitty parsing and GPUI image decoding will be added later. Keeping image
//! identity and placement separate from the grid lets the renderer support
//! both normal Kitty placements and Unicode-placeholder based placements.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GridId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlacementKey {
    pub image: ImageId,
    pub placement: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridAnchor {
    pub grid: GridId,
    pub row: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageAsset {
    pub encoded: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePlacement {
    pub key: PlacementKey,
    pub anchor: GridAnchor,
    pub columns: u32,
    pub rows: u32,
    pub z_index: i32,
}

#[derive(Debug, Default)]
pub struct ImageStore {
    assets: HashMap<ImageId, ImageAsset>,
    placements: HashMap<PlacementKey, ImagePlacement>,
}

impl ImageStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_asset(&mut self, id: ImageId, encoded: Vec<u8>) {
        self.assets.insert(id, ImageAsset { encoded });
    }

    pub fn asset(&self, id: ImageId) -> Option<&ImageAsset> {
        self.assets.get(&id)
    }

    pub fn place(&mut self, placement: ImagePlacement) {
        self.placements.insert(placement.key, placement);
    }

    pub fn placement(&self, key: PlacementKey) -> Option<&ImagePlacement> {
        self.placements.get(&key)
    }

    pub fn remove_placement(&mut self, key: PlacementKey) -> Option<ImagePlacement> {
        self.placements.remove(&key)
    }

    pub fn clear(&mut self) {
        self.assets.clear();
        self.placements.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{GridAnchor, GridId, ImageId, ImagePlacement, ImageStore, PlacementKey};

    #[test]
    fn one_asset_can_have_multiple_grid_placements() {
        let image = ImageId(7);
        let first_key = PlacementKey {
            image,
            placement: 1,
        };
        let second_key = PlacementKey {
            image,
            placement: 2,
        };
        let mut store = ImageStore::new();

        store.insert_asset(image, vec![1, 2, 3]);
        store.place(ImagePlacement {
            key: first_key,
            anchor: GridAnchor {
                grid: GridId(1),
                row: 2,
                column: 3,
            },
            columns: 4,
            rows: 5,
            z_index: -1,
        });
        store.place(ImagePlacement {
            key: second_key,
            anchor: GridAnchor {
                grid: GridId(2),
                row: 8,
                column: 13,
            },
            columns: 2,
            rows: 2,
            z_index: 0,
        });

        assert_eq!(store.asset(image).map(|asset| asset.encoded.len()), Some(3));
        assert_eq!(store.placement(first_key).map(|item| item.rows), Some(5));
        assert_eq!(
            store.placement(second_key).map(|item| item.anchor.grid),
            Some(GridId(2))
        );
        assert!(store.remove_placement(first_key).is_some());
        assert!(store.placement(first_key).is_none());
        assert!(store.placement(second_key).is_some());
    }
}
