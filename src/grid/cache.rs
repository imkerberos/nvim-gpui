use super::*;

const MAX_SHAPED_LINE_CACHE_ENTRIES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GlyphCoverageKey {
    font: Font,
    character: char,
}

/// Caches whether a primary GPUI font contains a glyph. Coverage is
/// independent of font size, color, and grid position.
#[derive(Default)]
pub struct GlyphCoverageCache {
    entries: HashMap<GlyphCoverageKey, bool>,
}

pub type SharedGlyphCoverageCache = Rc<RefCell<GlyphCoverageCache>>;

impl GlyphCoverageCache {
    pub fn shared() -> SharedGlyphCoverageCache {
        Rc::new(RefCell::new(Self::default()))
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub(super) fn contains(&mut self, window: &Window, font: &Font, character: char) -> bool {
        let key = GlyphCoverageKey {
            font: font.clone(),
            character,
        };
        if let Some(contains) = self.entries.get(&key) {
            return *contains;
        }

        let text_system = window.text_system();
        let requested_font = text_system.resolve_font(font);
        // A shaping result can still contain the primary font's missing-glyph
        // box, so comparing shaped font ids is not a reliable coverage test.
        // `typographic_bounds` asks the platform font directly for the glyph
        // and therefore distinguishes a real glyph from a replacement box.
        let contains = text_system
            .typographic_bounds(requested_font, px(16.0), character)
            .is_ok();
        self.entries.insert(key, contains);
        contains
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ShapingKey {
    text: SharedString,
    runs: Vec<StyledTextRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct ShapingStyle {
    pub(super) font: Font,
    pub(super) font_size: Pixels,
    pub(super) foreground: Hsla,
    pub(super) underline: Option<UnderlineStyle>,
    pub(super) strikethrough: Option<StrikethroughStyle>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct StyledTextRun {
    pub(super) len: usize,
    pub(super) style: ShapingStyle,
}

#[derive(Default)]
pub struct ShapedLineCache {
    lines: HashMap<ShapingKey, CachedShapedLine>,
    usage_clock: u64,
}

struct CachedShapedLine {
    line: ShapedLine,
    last_used: u64,
}

pub type SharedShapedLineCache = Rc<RefCell<ShapedLineCache>>;

impl ShapedLineCache {
    pub fn shared() -> SharedShapedLineCache {
        Rc::new(RefCell::new(Self::default()))
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.usage_clock = 0;
    }

    pub(super) fn shape_line(
        &mut self,
        window: &Window,
        text: SharedString,
        runs: Vec<StyledTextRun>,
    ) -> ShapedLine {
        let key = ShapingKey {
            text: text.clone(),
            runs: runs.clone(),
        };

        self.usage_clock = self.usage_clock.saturating_add(1);
        if let Some(entry) = self.lines.get_mut(&key) {
            entry.last_used = self.usage_clock;
            return entry.line.clone();
        }

        if self.lines.len() >= MAX_SHAPED_LINE_CACHE_ENTRIES {
            if let Some(oldest_key) = self
                .lines
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            {
                self.lines.remove(&oldest_key);
            }
        }

        let font_size = runs
            .first()
            .map(|run| run.style.font_size)
            .unwrap_or(px(1.0));
        let text_runs = runs
            .into_iter()
            .map(|run| TextRun {
                len: run.len,
                font: run.style.font,
                color: run.style.foreground,
                background_color: None,
                underline: run.style.underline,
                strikethrough: run.style.strikethrough,
            })
            .collect::<Vec<_>>();
        let line = window
            .text_system()
            .shape_line(text, font_size, &text_runs, None);
        self.lines.insert(
            key,
            CachedShapedLine {
                line: line.clone(),
                last_used: self.usage_clock,
            },
        );
        line
    }
}
