use crate::{image_store::is_kitty_placeholder, settings::FallbackMode};
use gpui::{
    fill, font, point, px, rgb, size, App, Bounds, Corners, Element, ElementId, Font,
    FontFallbacks, GlobalElementId, Hsla, IntoElement, LayoutId, Pixels, ShapedLine, SharedString,
    StrikethroughStyle, Style, TextRun, UnderlineStyle, Window,
};
use std::{
    cell::RefCell,
    collections::HashMap,
    f32::consts::PI,
    ops::Range,
    rc::Rc,
    time::{Duration, Instant},
};

const DEFAULT_FOREGROUND: u32 = 0xcdd6f4;
const DEFAULT_BACKGROUND: u32 = 0x1e1e2e;
const BLUE_FOREGROUND: u32 = 0x89b4fa;

mod cache;
mod cursor;
mod element;
mod model;
mod visual;

pub use cache::{
    GlyphCoverageCache, ShapedLineCache, SharedGlyphCoverageCache, SharedShapedLineCache,
};
pub use cursor::CursorElement;
pub use element::{GridElement, GridPrepaintState};
pub use model::{
    AmbiguousWidth, CellKind, CursorAnimation, CursorModeInfo, CursorShape, CursorVisualPosition,
    DisplayOptions, EmojiWidth, GridCell, GridCursor, GridLineCell, GridModel, GridRow,
    HighlightAttrs, HighlightId, DEFAULT_HIGHLIGHT,
};
pub use visual::{HighlightContext, ResolvedHighlight};
pub use visual::{VisualCell, VisualCellBuilder, VisualCellKind};

/// Transient text supplied by the platform IME.
///
/// This is deliberately separate from [`GridModel`]. Neovim remains the
/// authority for the grid contents; the element merges this composition into
/// the cell paint pass for the current frame only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImeComposition {
    pub row: usize,
    pub col: usize,
    pub text: SharedString,
    /// Byte range in `text` that is still marked by the IME.
    pub marked_range: Range<usize>,
    /// Byte range in `text` containing the IME caret/selection.
    pub selected_range: Range<usize>,
}

/// Convert a prefix of client-side IME text to terminal cells.
pub fn ime_text_cell_offset(text: &str, display_options: DisplayOptions) -> usize {
    display_options.text_cell_width(text)
}

use cache::{ShapingStyle, StyledTextRun};
#[cfg(test)]
use cursor::cursor_bounds;
#[cfg(test)]
pub(crate) use cursor::cursor_colors;
pub(crate) use cursor::cursor_colors_with_context;
use cursor::jelly_progress;
#[cfg(test)]
use model::cursor_geometry;
use model::{blink_visible, CursorVisualPositionF};
#[cfg(test)]
use visual::highlight_colors;
#[cfg(test)]
use visual::visual_cell_overlaps_cursor;
use visual::{push_background, resolve_highlight};

#[cfg(test)]
mod tests;
