use crate::{image_store::is_kitty_placeholder, settings::FallbackMode};
use gpui::{
    fill, font, point, px, rgb, size, App, Bounds, Corners, Element, ElementId, Font,
    FontFallbacks, GlobalElementId, Hsla, IntoElement, LayoutId, Pixels, ShapedLine, SharedString,
    StrikethroughStyle, Style, TextRun, UnderlineStyle, Window,
};
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::HashMap,
    f32::consts::PI,
    rc::Rc,
    time::{Duration, Instant},
};

const DEFAULT_FOREGROUND: u32 = 0xcdd6f4;
const DEFAULT_BACKGROUND: u32 = 0x1e1e2e;
const BLUE_FOREGROUND: u32 = 0x89b4fa;

mod cache;
mod element;
mod model;
mod visual;

pub use cache::{
    GlyphCoverageCache, ShapedLineCache, SharedGlyphCoverageCache, SharedShapedLineCache,
};
pub use element::{CursorElement, GridElement, GridPrepaintState};
pub use model::{
    CellKind, CursorAnimation, CursorModeInfo, CursorShape, CursorVisualPosition, GridCell,
    GridCursor, GridLineCell, GridModel, GridRow, HighlightAttrs, HighlightId, DEFAULT_HIGHLIGHT,
};
pub use visual::{VisualCell, VisualCellBuilder, VisualCellKind};

use cache::{ShapingStyle, StyledTextRun};
#[cfg(test)]
use element::cursor_bounds;
pub(crate) use element::cursor_colors;
use element::jelly_progress;
#[cfg(test)]
use model::cursor_geometry;
use model::{blink_visible, CursorVisualPositionF};
#[cfg(test)]
use visual::visual_cell_overlaps_cursor;
use visual::{highlight_colors, push_background};

#[cfg(test)]
mod tests;
