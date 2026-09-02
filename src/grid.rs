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
const MUTED_FOREGROUND: u32 = 0x7f849c;
const BLUE_FOREGROUND: u32 = 0x89b4fa;
const GREEN_FOREGROUND: u32 = 0xa6e3a1;
const STRING_BACKGROUND: u32 = 0x263238;
const LONG_TEXT_CHAR_COUNT: usize = 2048;

mod cache;
mod demo;
mod element;
mod model;
mod visual;

pub use cache::{
    GlyphCoverageCache, ShapedLineCache, SharedGlyphCoverageCache, SharedShapedLineCache,
};
pub use demo::demo_grid;
pub use element::{GridElement, GridPrepaintState};
pub use model::{
    CellKind, CursorAnimation, CursorModeInfo, CursorShape, CursorVisualPosition, GridCell,
    GridCursor, GridLineCell, GridModel, GridRow, HighlightAttrs, HighlightId, COMMENT_HIGHLIGHT,
    DEFAULT_HIGHLIGHT, KEYWORD_HIGHLIGHT, STRING_HIGHLIGHT,
};
pub use visual::{VisualCell, VisualCellBuilder, VisualCellKind};

use cache::{ShapingStyle, StyledTextRun};
use element::jelly_progress;
#[cfg(test)]
use element::{cursor_bounds, cursor_colors};
#[cfg(test)]
use model::cursor_geometry;
use model::{blink_visible, CursorVisualPositionF};
use visual::{
    demo_highlight_attrs, highlight_colors, push_background, visual_cell_overlaps_cursor,
};

#[cfg(test)]
mod tests;
