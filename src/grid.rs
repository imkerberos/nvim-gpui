use crate::image_store::is_kitty_placeholder;
use gpui::{
    fill, font, point, px, rgb, size, App, Bounds, Corners, Element, ElementId, Font,
    GlobalElementId, Hsla, IntoElement, LayoutId, Pixels, ShapedLine, SharedString,
    StrikethroughStyle, Style, TextRun, UnderlineStyle, Window,
};
use std::{
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HighlightId(pub u64);

pub const DEFAULT_HIGHLIGHT: HighlightId = HighlightId(0);
pub const COMMENT_HIGHLIGHT: HighlightId = HighlightId(1);
pub const KEYWORD_HIGHLIGHT: HighlightId = HighlightId(2);
pub const STRING_HIGHLIGHT: HighlightId = HighlightId(3);

/// The RGB attributes announced by Neovim's `hl_attr_define` event.
///
/// `None` is intentional: Neovim uses absent colors to mean "use the current
/// default", so the model must not eagerly replace them with a fixed color.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HighlightAttrs {
    pub foreground: Option<u32>,
    pub background: Option<u32>,
    pub special: Option<u32>,
    pub reverse: bool,
    pub italic: bool,
    pub bold: bool,
    pub strikethrough: bool,
    pub underline: bool,
    pub undercurl: bool,
    pub underdouble: bool,
    pub underdotted: bool,
    pub underdashed: bool,
    pub dim: bool,
    pub blink: bool,
    pub conceal: bool,
    pub overline: bool,
    pub blend: Option<u8>,
    pub altfont: Option<u32>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Block,
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorModeInfo {
    pub shape: CursorShape,
    pub cell_percentage: u8,
    pub blink_wait: u32,
    pub blink_on: u32,
    pub blink_off: u32,
    pub attr_id: Option<HighlightId>,
    pub attr_id_lm: Option<HighlightId>,
}

impl Default for CursorModeInfo {
    fn default() -> Self {
        Self {
            shape: CursorShape::Block,
            cell_percentage: 100,
            blink_wait: 0,
            blink_on: 0,
            blink_off: 0,
            attr_id: None,
            attr_id_lm: None,
        }
    }
}

impl CursorModeInfo {
    pub fn blink_enabled(self) -> bool {
        self.blink_on > 0 || self.blink_off > 0
    }

    pub fn visible_at(self, started_at: Instant, now: Instant) -> bool {
        blink_visible(
            started_at,
            now,
            self.blink_wait,
            self.blink_on,
            self.blink_off,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    Text,
    Blank,
    WideLead,
    WideContinuation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridCell {
    pub text: String,
    pub highlight: HighlightId,
    pub kind: CellKind,
}

impl GridCell {
    pub fn text(text: impl Into<String>, highlight: HighlightId) -> Self {
        Self {
            text: text.into(),
            highlight,
            kind: CellKind::Text,
        }
    }

    pub fn blank(highlight: HighlightId) -> Self {
        Self {
            text: " ".to_owned(),
            highlight,
            kind: CellKind::Blank,
        }
    }

    pub fn wide_lead(text: impl Into<String>, highlight: HighlightId) -> Self {
        Self {
            text: text.into(),
            highlight,
            kind: CellKind::WideLead,
        }
    }

    pub fn wide_continuation(highlight: HighlightId) -> Self {
        Self {
            text: String::new(),
            highlight,
            kind: CellKind::WideContinuation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridLineCell {
    pub text: String,
    pub highlight: HighlightId,
    pub repeat: usize,
}

impl GridLineCell {
    pub fn new(text: impl Into<String>, highlight: HighlightId, repeat: usize) -> Self {
        Self {
            text: text.into(),
            highlight,
            repeat: repeat.max(1),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridRow {
    cells: Vec<GridCell>,
    pub wraps_to_next: bool,
}

impl GridRow {
    pub fn new(cells: Vec<GridCell>) -> Self {
        Self {
            cells,
            wraps_to_next: false,
        }
    }

    pub fn wrapped(mut self) -> Self {
        self.wraps_to_next = true;
        self
    }

    pub fn cells(&self) -> &[GridCell] {
        &self.cells
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridModel {
    rows: Vec<GridRow>,
    width: usize,
    cursor: Option<GridCursor>,
    highlights: std::collections::HashMap<HighlightId, HighlightAttrs>,
    default_foreground: Option<u32>,
    default_background: Option<u32>,
    default_special: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridCursor {
    pub row: usize,
    pub col: usize,
}

/// The cursor's visible footprint in grid coordinates.
///
/// The width is normally one cell, but is two cells when Neovim places the
/// cursor on either half of a wide character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorVisualPosition {
    pub row: usize,
    pub col: usize,
    pub width: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct CursorAnimation {
    from: CursorVisualPositionF,
    to: CursorVisualPositionF,
    target: CursorVisualPosition,
    started_at: Instant,
    duration: Duration,
}

#[derive(Debug, Clone, Copy)]
struct CursorVisualPositionF {
    row: f32,
    col: f32,
    width: f32,
}

impl From<CursorVisualPosition> for CursorVisualPositionF {
    fn from(position: CursorVisualPosition) -> Self {
        Self {
            row: position.row as f32,
            col: position.col as f32,
            width: position.width as f32,
        }
    }
}

impl CursorAnimation {
    const DURATION: Duration = Duration::from_millis(145);

    pub fn new(from: CursorVisualPosition, target: CursorVisualPosition) -> Self {
        Self {
            from: from.into(),
            to: target.into(),
            target,
            started_at: Instant::now(),
            duration: Self::DURATION,
        }
    }

    /// Retarget an in-flight animation from its current interpolated position.
    /// This prevents fast cursor movement from jumping back to the previous
    /// cell whenever Neovim sends another redraw before the animation ends.
    pub fn retarget(&self, target: CursorVisualPosition) -> Self {
        let now = Instant::now();
        let from = self.position_at(now);
        Self {
            from,
            to: target.into(),
            target,
            started_at: now,
            duration: Self::DURATION,
        }
    }

    fn targets(&self, target: CursorVisualPosition) -> bool {
        self.target == target
    }

    fn progress(&self, now: Instant) -> f32 {
        (now.saturating_duration_since(self.started_at).as_secs_f32() / self.duration.as_secs_f32())
            .min(1.0)
    }

    fn position_at(&self, now: Instant) -> CursorVisualPositionF {
        let progress = ease_out_cubic(self.progress(now));
        CursorVisualPositionF {
            row: lerp(self.from.row, self.to.row, progress),
            col: lerp(self.from.col, self.to.col, progress),
            width: lerp(self.from.width, self.to.width, progress),
        }
    }
}

impl GridModel {
    pub fn from_rows(mut rows: Vec<GridRow>) -> Self {
        let width = rows
            .iter()
            .map(|row| row.cells.len())
            .max()
            .unwrap_or_default();

        for row in &mut rows {
            row.cells
                .resize_with(width, || GridCell::blank(DEFAULT_HIGHLIGHT));
        }

        Self {
            rows,
            width,
            cursor: None,
            highlights: std::collections::HashMap::new(),
            default_foreground: None,
            default_background: None,
            default_special: None,
        }
    }

    pub fn new(width: usize, height: usize) -> Self {
        Self::from_rows(
            (0..height)
                .map(|_| GridRow::new(Vec::new()))
                .map(|mut row| {
                    row.cells
                        .resize_with(width, || GridCell::blank(DEFAULT_HIGHLIGHT));
                    row
                })
                .collect(),
        )
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        let mut rows = Vec::with_capacity(height);
        for row_index in 0..height {
            let mut row = self
                .rows
                .get(row_index)
                .cloned()
                .unwrap_or_else(|| GridRow::new(Vec::new()));
            row.cells
                .resize_with(width, || GridCell::blank(DEFAULT_HIGHLIGHT));
            row.cells.truncate(width);
            rows.push(row);
        }

        self.rows = rows;
        self.width = width;
        self.cursor = self.cursor.map(|cursor| GridCursor {
            row: cursor.row.min(height.saturating_sub(1)),
            col: cursor.col.min(width.saturating_sub(1)),
        });
    }

    pub fn clear(&mut self) {
        for row in &mut self.rows {
            for cell in &mut row.cells {
                *cell = GridCell::blank(DEFAULT_HIGHLIGHT);
            }
            row.wraps_to_next = false;
        }
    }

    pub fn destroy(&mut self) {
        self.rows.clear();
        self.width = 0;
        self.cursor = None;
        self.highlights.clear();
        self.default_foreground = None;
        self.default_background = None;
        self.default_special = None;
    }

    pub fn apply_grid_line(
        &mut self,
        row: usize,
        col_start: usize,
        cells: &[GridLineCell],
        wraps_to_next: bool,
    ) {
        if row >= self.rows.len() {
            return;
        }

        let mut col = col_start;
        for (update_index, update) in cells.iter().enumerate() {
            let is_wide_lead = update.repeat == 1
                && !update.text.is_empty()
                && cells
                    .get(update_index + 1)
                    .is_some_and(|next| next.repeat == 1 && next.text.is_empty());

            for _ in 0..update.repeat {
                if col >= self.width {
                    break;
                }

                let cell = if update.text.is_empty() {
                    GridCell::wide_continuation(update.highlight)
                } else if is_wide_lead {
                    GridCell::wide_lead(update.text.clone(), update.highlight)
                } else {
                    GridCell::text(update.text.clone(), update.highlight)
                };
                self.replace_cell(row, col, cell);
                col += 1;
            }
        }

        self.rows[row].wraps_to_next = wraps_to_next;
    }

    pub fn set_cursor(&mut self, row: usize, col: usize) {
        // A multigrid redraw may announce the cursor before the first
        // `grid_resize` for a newly-created window grid. Keep the coordinates
        // and let `resize` clamp them once the grid dimensions are known.
        self.cursor = Some(GridCursor { row, col });
    }

    pub fn cursor(&self) -> Option<GridCursor> {
        self.cursor
    }

    pub fn cursor_visual_position(&self) -> Option<CursorVisualPosition> {
        let cursor = self.cursor?;
        let row = self.rows.get(cursor.row)?;
        let (col, width) = cursor_geometry(row, cursor.col);
        Some(CursorVisualPosition {
            row: cursor.row,
            col,
            width,
        })
    }

    pub fn set_highlight(&mut self, id: HighlightId, attrs: HighlightAttrs) {
        self.highlights.insert(id, attrs);
    }

    pub fn highlight(&self, id: HighlightId) -> Option<HighlightAttrs> {
        self.highlights.get(&id).cloned()
    }

    pub fn highlights(&self) -> &std::collections::HashMap<HighlightId, HighlightAttrs> {
        &self.highlights
    }

    pub fn set_default_colors(
        &mut self,
        foreground: Option<u32>,
        background: Option<u32>,
        special: Option<u32>,
    ) {
        self.default_foreground = foreground;
        self.default_background = background;
        self.default_special = special;
    }

    pub fn default_colors(&self) -> (Option<u32>, Option<u32>, Option<u32>) {
        (
            self.default_foreground,
            self.default_background,
            self.default_special,
        )
    }

    pub fn scroll(
        &mut self,
        top: usize,
        bot: usize,
        left: usize,
        right: usize,
        rows: isize,
        cols: isize,
    ) {
        let top = top.min(self.height());
        let bot = bot.min(self.height());
        let left = left.min(self.width());
        let right = right.min(self.width());
        if top >= bot || left >= right || (rows == 0 && cols == 0) {
            return;
        }

        let original = self.rows.clone();
        for row in top..bot {
            for col in left..right {
                let source_row = row as isize + rows;
                let source_col = col as isize + cols;
                let cell = if (top as isize..bot as isize).contains(&source_row)
                    && (left as isize..right as isize).contains(&source_col)
                {
                    original[source_row as usize].cells[source_col as usize].clone()
                } else {
                    GridCell::blank(DEFAULT_HIGHLIGHT)
                };
                self.replace_cell(row, col, cell);
            }
        }

        for row in top..bot {
            self.rows[row].wraps_to_next =
                if (top as isize..bot as isize).contains(&(row as isize + rows)) {
                    original[(row as isize + rows) as usize].wraps_to_next
                } else {
                    false
                };
        }
    }

    pub fn rows(&self) -> &[GridRow] {
        &self.rows
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.rows.len()
    }

    fn replace_cell(&mut self, row: usize, col: usize, cell: GridCell) {
        if self.rows[row].cells[col].kind == CellKind::WideContinuation
            && col > 0
            && self.rows[row].cells[col - 1].kind == CellKind::WideLead
        {
            self.rows[row].cells[col - 1] = GridCell::blank(DEFAULT_HIGHLIGHT);
        }

        if self.rows[row].cells[col].kind == CellKind::WideLead
            && self.rows[row]
                .cells
                .get(col + 1)
                .is_some_and(|next| next.kind == CellKind::WideContinuation)
        {
            self.rows[row].cells[col + 1] = GridCell::blank(DEFAULT_HIGHLIGHT);
        }

        if cell.kind != CellKind::WideContinuation
            && self.rows[row]
                .cells
                .get(col + 1)
                .is_some_and(|next| next.kind == CellKind::WideContinuation)
        {
            self.rows[row].cells[col + 1] = GridCell::blank(DEFAULT_HIGHLIGHT);
        }

        self.rows[row].cells[col] = cell;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualCellKind {
    Text,
    WideCharacter,
    NerdSymbol,
}

/// One independently positioned visual unit in the Neovim grid.
///
/// Normal cells contain one grapheme cluster. A wide-character lead is the
/// only unit that may cover more than one logical cell; its continuation is
/// kept in the model but does not paint a second glyph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualCell {
    pub row: usize,
    pub grid_start: usize,
    pub grid_len: usize,
    pub text: String,
    pub highlight: HighlightId,
    pub kind: VisualCellKind,
}

fn visual_cell_overlaps_cursor(cell: &VisualCell, cursor: CursorVisualPosition) -> bool {
    if cell.row != cursor.row {
        return false;
    }

    let cell_end = cell.grid_start + cell.grid_len;
    let cursor_end = cursor.col + cursor.width;
    cell.grid_start < cursor_end && cursor.col < cell_end
}

pub struct VisualCellBuilder {
    nerd_font_mode: bool,
}

impl VisualCellBuilder {
    pub fn new(nerd_font_mode: bool) -> Self {
        Self { nerd_font_mode }
    }

    pub fn build_grid(&self, model: &GridModel) -> Vec<VisualCell> {
        model
            .rows()
            .iter()
            .enumerate()
            .flat_map(|(row, grid_row)| self.build_row(row, grid_row))
            .collect()
    }

    pub fn build_row(&self, row: usize, grid_row: &GridRow) -> Vec<VisualCell> {
        let cells = grid_row.cells();
        let mut visual_cells = Vec::new();
        let mut col = 0;

        while col < cells.len() {
            let cell = &cells[col];

            if cell.kind == CellKind::WideContinuation {
                visual_cells.push(VisualCell {
                    row,
                    grid_start: col,
                    grid_len: 1,
                    text: " ".to_owned(),
                    highlight: cell.highlight,
                    kind: VisualCellKind::Text,
                });
                col += 1;
                continue;
            }

            if cell.kind == CellKind::WideLead {
                let has_continuation = cells
                    .get(col + 1)
                    .is_some_and(|next| next.kind == CellKind::WideContinuation);
                let is_nerd_symbol =
                    self.nerd_font_mode && is_nerd_symbol(&cell.text) && has_continuation;

                visual_cells.push(VisualCell {
                    row,
                    grid_start: col,
                    grid_len: if has_continuation { 2 } else { 1 },
                    text: cell.text.clone(),
                    highlight: cell.highlight,
                    kind: if is_nerd_symbol {
                        VisualCellKind::NerdSymbol
                    } else if has_continuation {
                        VisualCellKind::WideCharacter
                    } else {
                        VisualCellKind::Text
                    },
                });
                col += usize::from(has_continuation) + 1;
                continue;
            }

            let is_nerd_symbol = self.nerd_font_mode && is_nerd_symbol(&cell.text);
            let has_symbol_padding = is_nerd_symbol
                && cells.get(col + 1).is_some_and(|next| {
                    next.highlight == cell.highlight
                        && next.kind != CellKind::WideContinuation
                        && next.text == " "
                });

            if has_symbol_padding {
                visual_cells.push(VisualCell {
                    row,
                    grid_start: col,
                    grid_len: 2,
                    text: cell.text.clone(),
                    highlight: cell.highlight,
                    kind: VisualCellKind::NerdSymbol,
                });
                col += 2;
                continue;
            }

            visual_cells.push(VisualCell {
                row,
                grid_start: col,
                grid_len: 1,
                text: cell.text.clone(),
                highlight: cell.highlight,
                kind: if is_nerd_symbol {
                    VisualCellKind::NerdSymbol
                } else {
                    VisualCellKind::Text
                },
            });
            col += 1;
        }

        visual_cells
    }
}

fn is_nerd_symbol(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(character) = chars.next() else {
        return false;
    };

    chars.next().is_none()
        && (('\u{e000}'..='\u{f8ff}').contains(&character)
            || ('\u{f0000}'..='\u{ffffd}').contains(&character)
            || ('\u{100000}'..='\u{10fffd}').contains(&character))
}

fn demo_highlight_attrs(highlight: HighlightId) -> HighlightAttrs {
    match highlight {
        COMMENT_HIGHLIGHT => HighlightAttrs {
            foreground: Some(MUTED_FOREGROUND),
            ..Default::default()
        },
        KEYWORD_HIGHLIGHT => HighlightAttrs {
            foreground: Some(BLUE_FOREGROUND),
            ..Default::default()
        },
        STRING_HIGHLIGHT => HighlightAttrs {
            foreground: Some(GREEN_FOREGROUND),
            background: Some(STRING_BACKGROUND),
            ..Default::default()
        },
        _ => HighlightAttrs::default(),
    }
}

fn highlight_colors(model: &GridModel, highlight: HighlightId) -> (Hsla, Option<Hsla>) {
    let attrs = model
        .highlight(highlight)
        .unwrap_or_else(|| demo_highlight_attrs(highlight));
    let (default_foreground, default_background, _) = model.default_colors();
    let foreground = attrs
        .foreground
        .or(default_foreground)
        .unwrap_or(DEFAULT_FOREGROUND);
    // A terminal paints every cell, including a blank cell whose highlight
    // does not specify an explicit background. Keeping this as `None` makes
    // a multigrid float transparent, so the text behind a Noice/cmdline
    // window leaks through. Neovim's missing background means the current
    // default background, not transparent pixels.
    let background = attrs
        .background
        .or(default_background)
        .unwrap_or(DEFAULT_BACKGROUND);
    let mut foreground: Hsla = rgb(foreground).into();
    if attrs.dim {
        foreground.a *= 0.6;
    }
    foreground.a *= blend_alpha(attrs.blend);

    if attrs.reverse {
        let mut background_color: Hsla = rgb(background).into();
        background_color.a *= blend_alpha(attrs.blend);
        (background_color, Some(foreground))
    } else {
        let mut background: Hsla = rgb(background).into();
        background.a *= blend_alpha(attrs.blend);
        (foreground, Some(background))
    }
}

fn blend_alpha(blend: Option<u8>) -> f32 {
    blend
        .map(|blend| 1.0 - f32::from(blend.min(100)) / 100.0)
        .unwrap_or(1.0)
}

const MAX_SHAPED_LINE_CACHE_ENTRIES: usize = 8192;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ShapingKey {
    text: SharedString,
    style: ShapingStyle,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ShapingStyle {
    font: Font,
    font_size: Pixels,
    foreground: Hsla,
    underline: Option<UnderlineStyle>,
    strikethrough: Option<StrikethroughStyle>,
}

#[derive(Default)]
pub struct ShapedLineCache {
    lines: HashMap<ShapingKey, ShapedLine>,
}

pub type SharedShapedLineCache = Rc<RefCell<ShapedLineCache>>;

impl ShapedLineCache {
    pub fn shared() -> SharedShapedLineCache {
        Rc::new(RefCell::new(Self::default()))
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    fn shape_line(
        &mut self,
        window: &Window,
        text: SharedString,
        style: ShapingStyle,
    ) -> ShapedLine {
        let key = ShapingKey {
            text: text.clone(),
            style: style.clone(),
        };

        if let Some(line) = self.lines.get(&key) {
            return line.clone();
        }

        if self.lines.len() >= MAX_SHAPED_LINE_CACHE_ENTRIES {
            self.lines.clear();
        }

        let text_run = TextRun {
            len: text.len(),
            font: style.font,
            color: style.foreground,
            background_color: None,
            underline: style.underline,
            strikethrough: style.strikethrough,
        };
        let line = window
            .text_system()
            .shape_line(text, style.font_size, &[text_run], None);
        self.lines.insert(key, line.clone());
        line
    }
}

pub struct PaintedCell {
    line: Option<ShapedLine>,
    origin: gpui::Point<Pixels>,
    background: Option<(Bounds<Pixels>, Hsla)>,
    overline: Option<(Bounds<Pixels>, Hsla)>,
}

pub struct GridPrepaintState {
    cells: Vec<PaintedCell>,
    cursor: Option<PaintedCursor>,
}

struct PaintedCursor {
    bounds: Bounds<Pixels>,
    color: Hsla,
}

type InputHandlerRegistrar = Box<dyn FnMut(Bounds<Pixels>, &mut Window, &mut App)>;

pub struct GridElement {
    model: GridModel,
    nerd_font_mode: bool,
    cell_width: Pixels,
    line_height: Pixels,
    shaping_cache: SharedShapedLineCache,
    wide_font: Option<(String, Pixels)>,
    cursor_animation: Option<CursorAnimation>,
    cursor_mode: CursorModeInfo,
    cursor_visible: bool,
    cursor_blink_started_at: Instant,
    input_handler: Option<InputHandlerRegistrar>,
}

impl GridElement {
    pub fn new(model: GridModel) -> Self {
        Self {
            model,
            nerd_font_mode: false,
            cell_width: px(10.0),
            line_height: px(22.0),
            shaping_cache: ShapedLineCache::shared(),
            wide_font: None,
            cursor_animation: None,
            cursor_mode: CursorModeInfo::default(),
            cursor_visible: false,
            cursor_blink_started_at: Instant::now(),
            input_handler: None,
        }
    }

    pub fn with_nerd_font_mode(mut self, enabled: bool) -> Self {
        self.nerd_font_mode = enabled;
        self
    }

    pub fn with_metrics(mut self, cell_width: Pixels, line_height: Pixels) -> Self {
        self.cell_width = cell_width;
        self.line_height = line_height;
        self
    }

    pub fn with_wide_font(mut self, family: impl Into<String>, size: Pixels) -> Self {
        self.wide_font = Some((family.into(), size));
        self
    }

    pub fn with_shaping_cache(mut self, cache: SharedShapedLineCache) -> Self {
        self.shaping_cache = cache;
        self
    }

    pub fn with_cursor_animation(mut self, animation: Option<CursorAnimation>) -> Self {
        self.cursor_animation = animation;
        self
    }

    pub fn with_cursor_mode(mut self, mode: CursorModeInfo) -> Self {
        self.cursor_mode = mode;
        self
    }

    pub fn with_cursor_visible(mut self, visible: bool) -> Self {
        self.cursor_visible = visible;
        self
    }

    pub fn with_cursor_blink_started_at(mut self, started_at: Instant) -> Self {
        self.cursor_blink_started_at = started_at;
        self
    }

    pub fn with_input_handler(
        mut self,
        registrar: impl FnMut(Bounds<Pixels>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.input_handler = Some(Box::new(registrar));
        self
    }

    fn effective_cell_width(&self, window: &Window) -> Pixels {
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let font = text_style.font();

        window
            .text_system()
            .ch_advance(window.text_system().resolve_font(&font), font_size)
            .map(|advance| advance.max(px(1.0)))
            .unwrap_or(self.cell_width)
    }
}

impl IntoElement for GridElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for GridElement {
    type RequestLayoutState = ();
    type PrepaintState = GridPrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = (self.effective_cell_width(window) * self.model.width()).into();
        style.size.height = (self.line_height * self.model.height()).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let text_style = window.text_style();
        let normal_font_size = text_style.font_size.to_pixels(window.rem_size());
        let normal_font = text_style.font();
        let cell_width = self.effective_cell_width(window);
        let builder = VisualCellBuilder::new(self.nerd_font_mode);
        let now = Instant::now();
        let mut has_blinking_text = false;
        let cursor_position = self
            .cursor_visible
            .then(|| self.model.cursor_visual_position())
            .flatten();
        let block_cursor_colors = (self.cursor_visible
            && self.cursor_mode.shape == CursorShape::Block)
            .then(|| {
                cursor_position
                    .map(|position| cursor_colors(&self.model, position, self.cursor_mode))
            })
            .flatten();
        let mut shaping_cache = self.shaping_cache.borrow_mut();

        let cells = builder
            .build_grid(&self.model)
            .into_iter()
            .map(|cell| {
                let attrs = self
                    .model
                    .highlight(cell.highlight)
                    .unwrap_or_else(|| demo_highlight_attrs(cell.highlight));
                let (mut foreground, mut background) =
                    highlight_colors(&self.model, cell.highlight);
                let is_cursor_cell = block_cursor_colors.is_some_and(|_| {
                    cursor_position
                        .is_some_and(|position| visual_cell_overlaps_cursor(&cell, position))
                });
                if is_cursor_cell {
                    let (cursor_foreground, cursor_background) =
                        block_cursor_colors.expect("cursor colors are available");
                    foreground = cursor_foreground;
                    background = Some(cursor_background);
                }
                if attrs.blink {
                    has_blinking_text = true;
                }
                let line = if cell.text.is_empty()
                    || cell.text == " "
                    || is_kitty_placeholder(&cell.text)
                    || attrs.conceal
                    || (attrs.blink
                        && !blink_visible(self.cursor_blink_started_at, now, 0, 500, 500))
                {
                    None
                } else {
                    let text: SharedString = cell.text.clone().into();
                    let (cell_font, cell_font_size) = if cell.kind == VisualCellKind::WideCharacter
                    {
                        self.wide_font
                            .as_ref()
                            .map(|(family, size)| (font(family.clone()), *size))
                            .unwrap_or_else(|| (normal_font.clone(), normal_font_size))
                    } else {
                        (normal_font.clone(), normal_font_size)
                    };
                    let cell_font = if attrs.italic {
                        cell_font.italic()
                    } else {
                        cell_font
                    };
                    let cell_font = if attrs.bold {
                        cell_font.bold()
                    } else {
                        cell_font
                    };
                    let underline = (attrs.underline
                        || attrs.undercurl
                        || attrs.underdouble
                        || attrs.underdotted
                        || attrs.underdashed)
                        .then(|| UnderlineStyle {
                            thickness: px(1.0),
                            color: attrs
                                .special
                                .or(attrs.foreground)
                                .map(|color| rgb(color).into()),
                            wavy: attrs.undercurl,
                        });
                    let strikethrough = attrs.strikethrough.then(|| StrikethroughStyle {
                        thickness: px(1.0),
                        color: attrs
                            .special
                            .or(attrs.foreground)
                            .map(|color| rgb(color).into()),
                    });
                    Some(shaping_cache.shape_line(
                        window,
                        text,
                        ShapingStyle {
                            font: cell_font,
                            font_size: cell_font_size,
                            foreground,
                            underline,
                            strikethrough,
                        },
                    ))
                };
                let origin = point(
                    bounds.origin.x + cell_width * cell.grid_start,
                    bounds.origin.y + self.line_height * cell.row,
                );
                let cell_bounds =
                    Bounds::new(origin, size(cell_width * cell.grid_len, self.line_height));
                let overline = attrs.overline.then(|| {
                    let overline_color = attrs
                        .special
                        .map(|color| rgb(color).into())
                        .unwrap_or(foreground);
                    (
                        Bounds::new(
                            point(cell_bounds.origin.x, cell_bounds.origin.y + px(1.0)),
                            size(cell_bounds.size.width, px(1.0)),
                        ),
                        overline_color,
                    )
                });
                // Terminal cells are positioned from their leading edge. Do
                // not center a shaped glyph inside the cell: the extra
                // padding creates visible gaps between adjacent ASCII-art
                // glyphs, whose raster width is often smaller than the cell
                // advance.
                PaintedCell {
                    line,
                    origin,
                    background: background.map(|color| (cell_bounds, color)),
                    overline,
                }
            })
            .collect();

        if has_blinking_text {
            window.request_animation_frame();
        }

        let cursor = cursor_position.and_then(|target| {
            if self.cursor_mode.blink_enabled()
                && !self
                    .cursor_mode
                    .visible_at(self.cursor_blink_started_at, now)
            {
                window.request_animation_frame();
                return None;
            }

            let target_bounds = cursor_bounds(
                bounds,
                cell_width,
                self.line_height,
                target,
                self.cursor_mode,
            );
            let cursor_bounds = if self.cursor_mode.shape == CursorShape::Block {
                let Some(animation) = self
                    .cursor_animation
                    .filter(|animation| animation.targets(target))
                else {
                    return Some(PaintedCursor {
                        bounds: target_bounds,
                        color: cursor_colors(&self.model, target, self.cursor_mode).1,
                    });
                };

                if animation.progress(now) < 1.0 {
                    window.request_animation_frame();
                }
                animated_cursor_bounds(bounds, cell_width, self.line_height, animation, now)
            } else {
                target_bounds
            };

            Some(PaintedCursor {
                bounds: cursor_bounds,
                color: cursor_colors(&self.model, target, self.cursor_mode).1,
            })
        });

        if self.cursor_mode.blink_enabled() {
            window.request_animation_frame();
        }

        GridPrepaintState { cells, cursor }
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(input_handler) = self.input_handler.as_mut() {
            input_handler(bounds, window, cx);
        }

        for painted_cell in &prepaint.cells {
            if let Some((bounds, background)) = painted_cell.background {
                window.paint_quad(fill(bounds, background));
            }
            if let Some((bounds, color)) = painted_cell.overline {
                window.paint_quad(fill(bounds, color));
            }
        }

        if let Some(cursor) = prepaint.cursor.take() {
            let radius = px((f32::from(cursor.bounds.size.width)
                .min(f32::from(cursor.bounds.size.height))
                .mul_add(0.18, 0.0))
            .clamp(2.0, 6.0));
            window.paint_quad(fill(cursor.bounds, cursor.color).corner_radii(Corners::all(radius)));
        }

        // Keep the terminal's cell coordinates for placement, but do not clip
        // every glyph to its individual cell. GPUI's glyph raster bounds can
        // extend past the logical cell (especially for ASCII art, Nerd Font
        // symbols, and fonts with a generous ascent/descent). Per-cell masks
        // turn that overhang into visible seams at grid boundaries. The Grid
        // itself remains clipped so text cannot escape the Neovim viewport.
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for painted_cell in prepaint.cells.drain(..) {
                let Some(line) = painted_cell.line else {
                    continue;
                };
                line.paint(painted_cell.origin, self.line_height, window, cx)
                    .expect("failed to paint grid text");
            }
        });
    }
}

fn cursor_bounds(
    grid_bounds: Bounds<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    position: CursorVisualPosition,
    mode: CursorModeInfo,
) -> Bounds<Pixels> {
    let percentage = f32::from(mode.cell_percentage) / 100.0;
    let origin = point(
        grid_bounds.origin.x + cell_width * position.col,
        grid_bounds.origin.y + line_height * position.row,
    );
    let full_width = cell_width * position.width;

    let (origin, size) = match mode.shape {
        CursorShape::Block => (origin, size(full_width, line_height)),
        CursorShape::Horizontal => (
            point(origin.x, origin.y + line_height * (1.0 - percentage)),
            size(full_width, line_height * percentage),
        ),
        CursorShape::Vertical => (origin, size(full_width * percentage, line_height)),
    };

    Bounds::new(origin, size)
}

fn cursor_colors(
    model: &GridModel,
    position: CursorVisualPosition,
    mode: CursorModeInfo,
) -> (Hsla, Hsla) {
    let default_colors = highlight_colors(model, DEFAULT_HIGHLIGHT);
    let default_background = default_colors
        .1
        .unwrap_or_else(|| rgb(DEFAULT_BACKGROUND).into());
    let cell_highlight = model
        .rows()
        .get(position.row)
        .and_then(|row| row.cells().get(position.col))
        .map(|cell| cell.highlight)
        .unwrap_or(DEFAULT_HIGHLIGHT);

    match mode.attr_id {
        // Neovim defines attr_id 0 as a request to swap the current cell's
        // foreground and background, rather than as a normal highlight id.
        Some(DEFAULT_HIGHLIGHT) => {
            let (cell_foreground, cell_background) = highlight_colors(model, cell_highlight);
            (cell_background.unwrap_or(default_colors.0), cell_foreground)
        }
        Some(attr_id) => {
            let (foreground, background) = highlight_colors(model, attr_id);
            (foreground, background.unwrap_or(default_background))
        }
        None => (default_background, rgb(BLUE_FOREGROUND).into()),
    }
}

fn blink_visible(started_at: Instant, now: Instant, wait_ms: u32, on_ms: u32, off_ms: u32) -> bool {
    if on_ms == 0 && off_ms == 0 {
        return true;
    }

    let elapsed = now.saturating_duration_since(started_at);
    let wait = Duration::from_millis(u64::from(wait_ms));
    if elapsed < wait {
        return true;
    }

    let cycle_ms = u64::from(on_ms) + u64::from(off_ms);
    if cycle_ms == 0 {
        return true;
    }
    elapsed
        .saturating_sub(wait)
        .as_millis()
        .checked_rem(u128::from(cycle_ms))
        .map(|phase| phase < u128::from(on_ms))
        .unwrap_or(true)
}

fn animated_cursor_bounds(
    grid_bounds: Bounds<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    animation: CursorAnimation,
    now: Instant,
) -> Bounds<Pixels> {
    let progress = animation.progress(now);
    let position = animation.position_at(now);
    let from = animation.from;
    let to = animation.to;
    let delta_x = to.col - from.col;
    let delta_y = to.row - from.row;
    let distance = delta_x.abs().max(delta_y.abs());
    let pulse = (PI * progress).sin();
    let stretch_ratio = if distance > 0.0 {
        (0.10 + distance.min(4.0) * 0.05).min(0.30) * pulse
    } else {
        0.0
    };

    let cell_width = f32::from(cell_width);
    let line_height = f32::from(line_height);
    let base_x = f32::from(grid_bounds.origin.x) + cell_width * position.col;
    let base_y = f32::from(grid_bounds.origin.y) + line_height * position.row;
    let base_width = cell_width * position.width.max(1.0);
    let base_height = line_height;

    let (x, y, width, height) = if delta_x.abs() >= delta_y.abs() && delta_x != 0.0 {
        let extra = cell_width * stretch_ratio;
        let height = base_height * (1.0 - stretch_ratio * 0.12);
        (
            base_x - if delta_x > 0.0 { extra } else { 0.0 },
            base_y + (base_height - height) / 2.0,
            base_width + extra,
            height,
        )
    } else if delta_y != 0.0 {
        let extra = line_height * stretch_ratio;
        let width = base_width * (1.0 - stretch_ratio * 0.12);
        (
            base_x + (base_width - width) / 2.0,
            base_y - if delta_y > 0.0 { extra } else { 0.0 },
            width,
            base_height + extra,
        )
    } else {
        (base_x, base_y, base_width, base_height)
    };

    Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
}

fn ease_out_cubic(progress: f32) -> f32 {
    1.0 - (1.0 - progress).powi(3)
}

fn lerp(from: f32, to: f32, progress: f32) -> f32 {
    from + (to - from) * progress
}

fn cursor_geometry(row: &GridRow, cursor_col: usize) -> (usize, usize) {
    let Some(cell) = row.cells().get(cursor_col) else {
        return (cursor_col, 1);
    };

    if cell.kind == CellKind::WideContinuation
        && cursor_col > 0
        && row.cells()[cursor_col - 1].kind == CellKind::WideLead
    {
        return (cursor_col - 1, 2);
    }

    if cell.kind == CellKind::WideLead
        && row
            .cells()
            .get(cursor_col + 1)
            .is_some_and(|next| next.kind == CellKind::WideContinuation)
    {
        return (cursor_col, 2);
    }

    (cursor_col, 1)
}

pub fn demo_grid() -> GridModel {
    let mut unicode_row = text_cells("Unicode: ", COMMENT_HIGHLIGHT);
    unicode_row.push(GridCell::wide_lead("界", DEFAULT_HIGHLIGHT));
    unicode_row.push(GridCell::wide_continuation(DEFAULT_HIGHLIGHT));
    unicode_row.extend(text_cells(" ", DEFAULT_HIGHLIGHT));
    unicode_row.push(GridCell::wide_lead("你", DEFAULT_HIGHLIGHT));
    unicode_row.push(GridCell::wide_continuation(DEFAULT_HIGHLIGHT));
    unicode_row.push(GridCell::wide_lead("好", DEFAULT_HIGHLIGHT));
    unicode_row.push(GridCell::wide_continuation(DEFAULT_HIGHLIGHT));

    let mut combining_row = text_cells("Combining: ", STRING_HIGHLIGHT);
    combining_row.push(GridCell::text("e\u{301}", STRING_HIGHLIGHT));
    combining_row.extend(text_cells("  emoji: ", STRING_HIGHLIGHT));
    combining_row.push(GridCell::wide_lead("👩‍💻", DEFAULT_HIGHLIGHT));
    combining_row.push(GridCell::wide_continuation(DEFAULT_HIGHLIGHT));

    let mut nerd_row = text_cells("Nerd Font: ", COMMENT_HIGHLIGHT);
    nerd_row.push(GridCell::text("\u{f0239}", DEFAULT_HIGHLIGHT));
    nerd_row.push(GridCell::blank(DEFAULT_HIGHLIGHT));
    nerd_row.extend(text_cells(
        "symbol + space -> one visual span",
        DEFAULT_HIGHLIGHT,
    ));

    let long_ascii_row = long_ascii_cells("Long ASCII (2048 chars): ");
    let long_unicode_row = long_unicode_cells("Long Unicode (2048 chars): ");

    GridModel::from_rows(vec![
        GridRow::new(unicode_row),
        GridRow::new(combining_row),
        GridRow::new(nerd_row),
        GridRow::new(
            [
                text_cells("highlight ", DEFAULT_HIGHLIGHT),
                text_cells("changes", KEYWORD_HIGHLIGHT),
                text_cells(" at cell boundaries", DEFAULT_HIGHLIGHT),
            ]
            .into_iter()
            .flatten()
            .collect(),
        ),
        GridRow::new(text_cells(
            "This row reports a wrap boundary",
            COMMENT_HIGHLIGHT,
        ))
        .wrapped(),
        GridRow::new(long_ascii_row),
        GridRow::new(long_unicode_row),
    ])
}

fn text_cells(text: &str, highlight: HighlightId) -> Vec<GridCell> {
    text.chars()
        .map(|character| GridCell::text(character.to_string(), highlight))
        .collect()
}

fn long_ascii_cells(prefix: &str) -> Vec<GridCell> {
    let mut cells = text_cells(prefix, COMMENT_HIGHLIGHT);

    for index in 0..LONG_TEXT_CHAR_COUNT {
        let character = char::from(b'a' + (index % 26) as u8);
        cells.push(GridCell::text(character.to_string(), DEFAULT_HIGHLIGHT));
    }

    cells
}

fn long_unicode_cells(prefix: &str) -> Vec<GridCell> {
    let mut cells = text_cells(prefix, COMMENT_HIGHLIGHT);
    let pattern = ['a', '界', 'b', '你', 'c', '好', 'd', 'e'];

    for index in 0..LONG_TEXT_CHAR_COUNT {
        let character = pattern[index % pattern.len()];
        if matches!(character, '界' | '你' | '好') {
            cells.push(GridCell::wide_lead(
                character.to_string(),
                DEFAULT_HIGHLIGHT,
            ));
            cells.push(GridCell::wide_continuation(DEFAULT_HIGHLIGHT));
        } else {
            cells.push(GridCell::text(character.to_string(), DEFAULT_HIGHLIGHT));
        }
    }

    cells
}

#[cfg(test)]
mod tests {
    use super::{
        blink_visible, cursor_bounds, cursor_colors, cursor_geometry, highlight_colors, CellKind,
        CursorAnimation, CursorModeInfo, CursorShape, CursorVisualPosition, GridCell, GridLineCell,
        GridModel, GridRow, HighlightAttrs, HighlightId, VisualCell, VisualCellBuilder,
        VisualCellKind, COMMENT_HIGHLIGHT, DEFAULT_HIGHLIGHT, KEYWORD_HIGHLIGHT,
        LONG_TEXT_CHAR_COUNT,
    };
    use gpui::{point, px, size, Bounds};
    use std::time::{Duration, Instant};

    #[test]
    fn wide_character_occupies_two_grid_cells() {
        let row = GridRow::new(vec![
            GridCell::wide_lead("界", DEFAULT_HIGHLIGHT),
            GridCell::wide_continuation(DEFAULT_HIGHLIGHT),
        ]);

        let cells = VisualCellBuilder::new(false).build_row(4, &row);

        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].row, 4);
        assert_eq!(cells[0].grid_start, 0);
        assert_eq!(cells[0].grid_len, 2);
        assert_eq!(cells[0].text, "界");
        assert_eq!(cells[0].kind, VisualCellKind::WideCharacter);
    }

    #[test]
    fn cursor_highlight_only_applies_to_the_cursor_row() {
        let cell = VisualCell {
            row: 3,
            grid_start: 5,
            grid_len: 1,
            text: "x".to_owned(),
            highlight: DEFAULT_HIGHLIGHT,
            kind: VisualCellKind::Text,
        };
        let cursor = CursorVisualPosition {
            row: 4,
            col: 5,
            width: 1,
        };

        assert!(!super::visual_cell_overlaps_cursor(&cell, cursor));
        assert!(super::visual_cell_overlaps_cursor(
            &VisualCell { row: 4, ..cell },
            cursor
        ));
    }

    #[test]
    fn nerd_symbol_and_following_space_share_a_two_cell_visual_span() {
        let row = GridRow::new(vec![
            GridCell::text("\u{f0239}", DEFAULT_HIGHLIGHT),
            GridCell::text(" ", DEFAULT_HIGHLIGHT),
        ]);

        let cells = VisualCellBuilder::new(true).build_row(0, &row);

        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].grid_start, 0);
        assert_eq!(cells[0].grid_len, 2);
        assert_eq!(cells[0].kind, VisualCellKind::NerdSymbol);
    }

    #[test]
    fn nerd_symbol_does_not_consume_a_differently_highlighted_space() {
        let row = GridRow::new(vec![
            GridCell::text("\u{f0239}", DEFAULT_HIGHLIGHT),
            GridCell::text(" ", COMMENT_HIGHLIGHT),
        ]);

        let cells = VisualCellBuilder::new(true).build_row(0, &row);

        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].grid_len, 1);
        assert_eq!(cells[1].grid_len, 1);
    }

    #[test]
    fn wide_nerd_symbol_uses_the_main_font_visual_kind() {
        let row = GridRow::new(vec![
            GridCell::wide_lead("\u{f0239}", DEFAULT_HIGHLIGHT),
            GridCell::wide_continuation(DEFAULT_HIGHLIGHT),
        ]);

        let cells = VisualCellBuilder::new(true).build_row(0, &row);

        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].grid_len, 2);
        assert_eq!(cells[0].kind, VisualCellKind::NerdSymbol);
    }

    #[test]
    fn one_visual_cell_keeps_its_grapheme_cluster_intact() {
        let combining = "e\u{301}";
        let emoji = "👩‍💻";
        let row = GridRow::new(vec![
            GridCell::text(combining, DEFAULT_HIGHLIGHT),
            GridCell::text(emoji, DEFAULT_HIGHLIGHT),
        ]);

        let cells = VisualCellBuilder::new(false).build_row(0, &row);

        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].text, combining);
        assert_eq!(cells[0].grid_len, 1);
        assert_eq!(cells[1].text, emoji);
        assert_eq!(cells[1].grid_len, 1);
    }

    #[test]
    fn adjacent_cells_are_never_combined_for_text_layout() {
        let row = GridRow::new(vec![
            GridCell::text("a", DEFAULT_HIGHLIGHT),
            GridCell::text("c", DEFAULT_HIGHLIGHT),
            GridCell::text("d", COMMENT_HIGHLIGHT),
        ]);

        let cells = VisualCellBuilder::new(false).build_row(0, &row);

        assert_eq!(cells.len(), 3);
        assert_eq!(cells[0].text, "a");
        assert_eq!(cells[0].grid_start, 0);
        assert_eq!(cells[1].text, "c");
        assert_eq!(cells[1].grid_start, 1);
        assert_eq!(cells[2].text, "d");
        assert_eq!(cells[2].grid_start, 2);
    }

    #[test]
    fn model_pads_rows_to_a_stable_grid_width() {
        let model = GridModel::from_rows(vec![
            GridRow::new(vec![GridCell::text("a", DEFAULT_HIGHLIGHT)]),
            GridRow::new(vec![
                GridCell::text("b", DEFAULT_HIGHLIGHT),
                GridCell::blank(DEFAULT_HIGHLIGHT),
            ]),
        ]);

        assert_eq!(model.width(), 2);
        assert_eq!(model.height(), 2);
        assert_eq!(model.rows()[0].cells().len(), 2);
        assert_eq!(model.rows()[0].cells()[1].kind, CellKind::Blank);
        assert_eq!(model.rows()[1].cells().len(), 2);
        assert_eq!(model.rows()[1].cells()[1].kind, CellKind::Blank);
    }

    #[test]
    fn grid_line_updates_unicode_cells_repeats_and_wrap_state() {
        let mut model = GridModel::new(6, 2);

        model.apply_grid_line(
            0,
            1,
            &[
                GridLineCell::new("界", HighlightId(7), 1),
                GridLineCell::new("", HighlightId(7), 1),
                GridLineCell::new("x", HighlightId(8), 2),
            ],
            true,
        );

        assert_eq!(model.rows()[0].cells()[0].kind, CellKind::Blank);
        assert_eq!(model.rows()[0].cells()[1].text, "界");
        assert_eq!(model.rows()[0].cells()[1].kind, CellKind::WideLead);
        assert_eq!(model.rows()[0].cells()[2].kind, CellKind::WideContinuation);
        assert_eq!(model.rows()[0].cells()[3].text, "x");
        assert_eq!(model.rows()[0].cells()[4].text, "x");
        assert!(model.rows()[0].wraps_to_next);
    }

    #[test]
    fn mixed_ascii_and_wide_cells_keep_their_grid_columns() {
        let mut model = GridModel::new(6, 1);

        model.apply_grid_line(
            0,
            0,
            &[
                GridLineCell::new("a", DEFAULT_HIGHLIGHT, 1),
                GridLineCell::new("中", DEFAULT_HIGHLIGHT, 1),
                GridLineCell::new("", DEFAULT_HIGHLIGHT, 1),
                GridLineCell::new("b", DEFAULT_HIGHLIGHT, 1),
            ],
            false,
        );

        let cells = VisualCellBuilder::new(false).build_row(0, &model.rows()[0]);

        assert_eq!(cells[0].grid_start, 0);
        assert_eq!(cells[0].grid_len, 1);
        assert_eq!(cells[1].grid_start, 1);
        assert_eq!(cells[1].grid_len, 2);
        assert_eq!(cells[2].grid_start, 3);
        assert_eq!(cells[2].text.chars().next(), Some('b'));
    }

    #[test]
    fn grid_scroll_moves_rows_and_clears_the_scrolled_in_area() {
        let mut model = GridModel::from_rows(vec![
            GridRow::new(vec![GridCell::text("a", DEFAULT_HIGHLIGHT)]),
            GridRow::new(vec![GridCell::text("b", DEFAULT_HIGHLIGHT)]),
            GridRow::new(vec![GridCell::text("c", DEFAULT_HIGHLIGHT)]),
        ]);

        model.scroll(0, 3, 0, 1, 1, 0);

        assert_eq!(model.rows()[0].cells()[0].text, "b");
        assert_eq!(model.rows()[1].cells()[0].text, "c");
        assert_eq!(model.rows()[2].cells()[0].kind, CellKind::Blank);
    }

    #[test]
    fn cursor_is_kept_in_the_grid_model() {
        let mut model = GridModel::new(4, 2);

        model.set_cursor(1, 3);

        assert_eq!(model.cursor(), Some(super::GridCursor { row: 1, col: 3 }));
    }

    #[test]
    fn cursor_can_arrive_before_the_grid_resize() {
        let mut model = GridModel::new(0, 0);

        model.set_cursor(4, 7);
        model.resize(10, 5);

        assert_eq!(model.cursor(), Some(super::GridCursor { row: 4, col: 7 }));
    }

    #[test]
    fn cursor_covers_a_wide_character_from_either_grid_cell() {
        let row = GridRow::new(vec![
            GridCell::wide_lead("界", DEFAULT_HIGHLIGHT),
            GridCell::wide_continuation(DEFAULT_HIGHLIGHT),
        ]);

        assert_eq!(cursor_geometry(&row, 0), (0, 2));
        assert_eq!(cursor_geometry(&row, 1), (0, 2));
    }

    #[test]
    fn cursor_animation_interpolates_to_its_target() {
        let animation = CursorAnimation::new(
            CursorVisualPosition {
                row: 2,
                col: 3,
                width: 1,
            },
            CursorVisualPosition {
                row: 5,
                col: 8,
                width: 2,
            },
        );

        let start = animation.position_at(animation.started_at);
        let middle = animation.position_at(animation.started_at + Duration::from_millis(72));
        let end = animation.position_at(animation.started_at + animation.duration);

        assert_eq!(start.row, 2.0);
        assert_eq!(start.col, 3.0);
        assert!(middle.row > 2.0 && middle.row < 5.0);
        assert!(middle.col > 3.0 && middle.col < 8.0);
        assert!(middle.width > 1.0 && middle.width < 2.0);
        assert_eq!(end.row, 5.0);
        assert_eq!(end.col, 8.0);
        assert_eq!(end.width, 2.0);
    }

    #[test]
    fn model_stores_neovim_highlight_attributes() {
        let mut model = GridModel::new(1, 1);
        let attrs = HighlightAttrs {
            foreground: Some(0xabcdef),
            reverse: true,
            ..Default::default()
        };

        model.set_highlight(HighlightId(42), attrs.clone());

        assert_eq!(model.highlight(HighlightId(42)), Some(attrs));
    }

    #[test]
    fn missing_highlight_background_inherits_the_grid_default() {
        let mut model = GridModel::new(1, 1);
        model.set_default_colors(Some(0xffffff), Some(0x112233), None);
        model.set_highlight(HighlightId(42), HighlightAttrs::default());

        let (_, background) = highlight_colors(&model, HighlightId(42));

        assert_eq!(background, Some(gpui::rgb(0x112233).into()));
    }

    #[test]
    fn default_cursor_attribute_swaps_the_current_cell_colors() {
        let mut model = GridModel::new(1, 1);
        model.set_default_colors(Some(0xffffff), Some(0x112233), None);
        model.set_highlight(
            HighlightId(42),
            HighlightAttrs {
                foreground: Some(0xaabbcc),
                background: Some(0x445566),
                ..Default::default()
            },
        );
        model.apply_grid_line(0, 0, &[GridLineCell::new("x", HighlightId(42), 1)], false);

        let normal = highlight_colors(&model, HighlightId(42));
        let cursor = cursor_colors(
            &model,
            CursorVisualPosition {
                row: 0,
                col: 0,
                width: 1,
            },
            CursorModeInfo {
                attr_id: Some(DEFAULT_HIGHLIGHT),
                ..Default::default()
            },
        );

        assert_eq!(cursor.0, normal.1.expect("cell background should exist"));
        assert_eq!(cursor.1, normal.0);
    }

    #[test]
    fn destroy_clears_grid_contents_cursor_highlights_and_defaults() {
        let mut model = GridModel::new(2, 1);
        model.set_cursor(0, 1);
        model.set_highlight(
            HighlightId(42),
            HighlightAttrs {
                foreground: Some(0xabcdef),
                ..Default::default()
            },
        );
        model.set_default_colors(Some(1), Some(2), Some(3));

        model.destroy();

        assert_eq!(model.width(), 0);
        assert_eq!(model.height(), 0);
        assert_eq!(model.cursor(), None);
        assert!(model.highlights().is_empty());
        assert_eq!(model.default_colors(), (None, None, None));
    }

    #[test]
    fn cursor_shapes_use_the_neovim_cell_percentage() {
        let bounds = Bounds::new(point(px(10.0), px(20.0)), size(px(80.0), px(88.0)));
        let position = super::CursorVisualPosition {
            row: 1,
            col: 2,
            width: 2,
        };

        let horizontal = cursor_bounds(
            bounds,
            px(10.0),
            px(22.0),
            position,
            CursorModeInfo {
                shape: CursorShape::Horizontal,
                cell_percentage: 25,
                ..Default::default()
            },
        );
        assert_eq!(f32::from(horizontal.origin.x), 30.0);
        assert_eq!(f32::from(horizontal.origin.y), 58.5);
        assert_eq!(f32::from(horizontal.size.width), 20.0);
        assert_eq!(f32::from(horizontal.size.height), 5.5);

        let vertical = cursor_bounds(
            bounds,
            px(10.0),
            px(22.0),
            position,
            CursorModeInfo {
                shape: CursorShape::Vertical,
                cell_percentage: 20,
                ..Default::default()
            },
        );
        assert_eq!(f32::from(vertical.origin.x), 30.0);
        assert_eq!(f32::from(vertical.origin.y), 42.0);
        assert_eq!(f32::from(vertical.size.width), 4.0);
        assert_eq!(f32::from(vertical.size.height), 22.0);
    }

    #[test]
    fn cursor_blink_respects_wait_on_and_off_intervals() {
        let started_at = Instant::now();

        assert!(blink_visible(
            started_at,
            started_at + Duration::from_millis(100),
            200,
            400,
            250
        ));
        assert!(blink_visible(
            started_at,
            started_at + Duration::from_millis(500),
            200,
            400,
            250
        ));
        assert!(!blink_visible(
            started_at,
            started_at + Duration::from_millis(650),
            200,
            400,
            250
        ));
        assert!(blink_visible(
            started_at,
            started_at + Duration::from_millis(900),
            200,
            400,
            250
        ));
    }

    #[test]
    fn keyword_highlight_is_distinct_from_default() {
        let row = GridRow::new(vec![
            GridCell::text("fn", KEYWORD_HIGHLIGHT),
            GridCell::text(" main", DEFAULT_HIGHLIGHT),
        ]);

        let cells = VisualCellBuilder::new(false).build_row(0, &row);

        assert_eq!(cells[0].highlight, KEYWORD_HIGHLIGHT);
        assert_eq!(cells[1].highlight, DEFAULT_HIGHLIGHT);
    }

    #[test]
    fn demo_contains_2048_character_ascii_and_unicode_rows() {
        let model = super::demo_grid();
        let ascii_row = &model.rows()[5];
        let unicode_row = &model.rows()[6];
        let ascii_prefix_len = "Long ASCII (2048 chars): ".chars().count();
        let unicode_prefix_len = "Long Unicode (2048 chars): ".chars().count();

        let ascii_char_count = ascii_row
            .cells()
            .iter()
            .filter(|cell| cell.kind != CellKind::Blank)
            .map(|cell| cell.text.chars().count())
            .sum::<usize>();
        let unicode_char_count = unicode_row
            .cells()
            .iter()
            .filter(|cell| cell.kind != CellKind::Blank)
            .map(|cell| cell.text.chars().count())
            .sum::<usize>();

        assert_eq!(ascii_char_count, ascii_prefix_len + LONG_TEXT_CHAR_COUNT);
        assert_eq!(
            unicode_char_count,
            unicode_prefix_len + LONG_TEXT_CHAR_COUNT
        );
        assert!(unicode_row
            .cells()
            .iter()
            .any(|cell| cell.kind == CellKind::WideContinuation));
    }
}
