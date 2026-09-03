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
    CellKind, CursorAnimation, CursorModeInfo, CursorShape, CursorVisualPosition, GridCell,
    GridCursor, GridLineCell, GridModel, GridRow, HighlightAttrs, HighlightId, DEFAULT_HIGHLIGHT,
};
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

/// Convert a prefix of IME text to the number of terminal cells it occupies.
///
/// The IME text is not part of Neovim's grid, so its width has to be measured
/// locally. Using the same text system and font metrics as the grid keeps the
/// transient cursor aligned with the rendered preedit.
pub fn ime_text_cell_offset(
    window: &Window,
    font_family: &str,
    font_size: Pixels,
    text: &str,
    cell_width: Pixels,
) -> usize {
    if text.is_empty() {
        return 0;
    }

    let text: SharedString = text.to_owned().into();
    let line = window.text_system().shape_line(
        text.clone(),
        font_size,
        &[TextRun {
            len: text.len(),
            font: font(font_family.to_owned()),
            color: rgb(DEFAULT_FOREGROUND).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
        None,
    );
    (f32::from(line.width) / f32::from(cell_width)).ceil() as usize
}

use cache::{ShapingStyle, StyledTextRun};
#[cfg(test)]
use cursor::cursor_bounds;
pub(crate) use cursor::cursor_colors;
use cursor::jelly_progress;
#[cfg(test)]
use model::cursor_geometry;
use model::{blink_visible, CursorVisualPositionF};
#[cfg(test)]
use visual::visual_cell_overlaps_cursor;
use visual::{highlight_colors, push_background};

#[cfg(test)]
mod tests;
