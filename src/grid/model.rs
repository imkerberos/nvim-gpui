use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HighlightId(pub u64);

pub const DEFAULT_HIGHLIGHT: HighlightId = HighlightId(0);

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
    /// The semantic UI highlight name sent when `ext_hlstate` is enabled.
    /// This is metadata, not a styling attribute; it lets multigrid render a
    /// floating window's implicit background correctly.
    pub ui_name: Option<String>,
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
    pub text: SharedString,
    pub highlight: HighlightId,
    pub kind: CellKind,
}

impl GridCell {
    pub fn text(text: impl Into<String>, highlight: HighlightId) -> Self {
        Self {
            text: text.into().into(),
            highlight,
            kind: CellKind::Text,
        }
    }

    pub fn blank(highlight: HighlightId) -> Self {
        Self {
            text: SharedString::new_static(" "),
            highlight,
            kind: CellKind::Blank,
        }
    }

    pub fn wide_lead(text: impl Into<String>, highlight: HighlightId) -> Self {
        Self {
            text: text.into().into(),
            highlight,
            kind: CellKind::WideLead,
        }
    }

    pub fn wide_continuation(highlight: HighlightId) -> Self {
        Self {
            text: SharedString::new_static(""),
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
    pub(super) from: CursorVisualPositionF,
    pub(super) to: CursorVisualPositionF,
    pub(super) target: CursorVisualPosition,
    pub(super) started_at: Instant,
    pub(super) duration: Duration,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct CursorVisualPositionF {
    pub(super) row: f32,
    pub(super) col: f32,
    pub(super) width: f32,
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
    // Keep this short enough that normal cursor movement still feels direct,
    // while leaving enough frames for the elastic shape and two tail layers
    // to be visible at 60 Hz.
    const DURATION: Duration = Duration::from_millis(180);

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

    pub(super) fn targets(&self, target: CursorVisualPosition) -> bool {
        self.target == target
    }

    pub(super) fn progress(&self, now: Instant) -> f32 {
        (now.saturating_duration_since(self.started_at).as_secs_f32() / self.duration.as_secs_f32())
            .min(1.0)
    }

    pub(super) fn position_at(&self, now: Instant) -> CursorVisualPositionF {
        let progress = jelly_progress(self.progress(now));
        CursorVisualPositionF {
            row: lerp(self.from.row, self.to.row, progress),
            col: lerp(self.from.col, self.to.col, progress),
            width: lerp(self.from.width, self.to.width, progress),
        }
    }

    pub(super) fn is_active(&self, now: Instant) -> bool {
        self.progress(now) < 1.0
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
                    // Neovim's line-grid protocol reserves an empty text
                    // entry for the right half of the preceding double-width
                    // character. Do not infer this from Unicode width: Nvim
                    // has already applied its `ambiwidth` and display-width
                    // rules before emitting the event.
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

    pub(super) fn highlight_ref(&self, id: HighlightId) -> Option<&HighlightAttrs> {
        self.highlights.get(&id)
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

fn lerp(from: f32, to: f32, progress: f32) -> f32 {
    from + (to - from) * progress
}

pub(super) fn blink_visible(
    started_at: Instant,
    now: Instant,
    wait_ms: u32,
    on_ms: u32,
    off_ms: u32,
) -> bool {
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

pub(super) fn cursor_geometry(row: &GridRow, cursor_col: usize) -> (usize, usize) {
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
