use super::*;

const MAX_SHAPED_LINE_CACHE_ENTRIES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FontRole {
    Normal,
    Wide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FontStyleKind {
    Normal,
    Bold,
    Italic,
    BoldItalic,
}

impl FontStyleKind {
    pub(crate) fn from_attributes(bold: bool, italic: bool) -> Self {
        match (bold, italic) {
            (false, false) => Self::Normal,
            (true, false) => Self::Bold,
            (false, true) => Self::Italic,
            (true, true) => Self::BoldItalic,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FontStyleCacheKey {
    role: FontRole,
    primary: Font,
    fallback_families: Vec<String>,
}

/// The four style variants for one primary font and one configured fallback
/// chain. Both the primary-only and cascading variants are retained so Linux
/// can keep splitting runs while CoreText and DirectWrite use one cascading
/// font for the whole cell.
#[derive(Clone)]
pub(crate) struct FontStyleVariants {
    primary: [Font; 4],
    fallback: [Font; 4],
    selected_fallbacks: Vec<[Font; 4]>,
}

impl FontStyleVariants {
    fn index(style: FontStyleKind) -> usize {
        match style {
            FontStyleKind::Normal => 0,
            FontStyleKind::Bold => 1,
            FontStyleKind::Italic => 2,
            FontStyleKind::BoldItalic => 3,
        }
    }

    pub(crate) fn font(&self, style: FontStyleKind, with_fallback: bool) -> Font {
        let index = Self::index(style);
        if with_fallback {
            self.fallback[index].clone()
        } else {
            self.primary[index].clone()
        }
    }

    pub(crate) fn selected_font(&self, style: FontStyleKind, selection: FontSelection) -> Font {
        let index = Self::index(style);
        match selection {
            FontSelection::Primary => self.primary[index].clone(),
            FontSelection::Fallback(fallback_index) => {
                self.selected_fallbacks[fallback_index][index].clone()
            }
            FontSelection::Unavailable => self.fallback[index].clone(),
        }
    }
}

#[derive(Default)]
pub(crate) struct FontStyleCache {
    entries: HashMap<FontStyleCacheKey, FontStyleVariants>,
}

pub(crate) type SharedFontStyleCache = Rc<RefCell<FontStyleCache>>;

impl FontStyleCache {
    pub(crate) fn shared() -> SharedFontStyleCache {
        Rc::new(RefCell::new(Self::default()))
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn get_or_insert(
        &mut self,
        role: FontRole,
        primary: Font,
        fallback_families: &[String],
    ) -> FontStyleVariants {
        let key = FontStyleCacheKey {
            role,
            primary: primary.clone(),
            fallback_families: fallback_families.to_vec(),
        };
        if let Some(variants) = self.entries.get(&key) {
            return variants.clone();
        }

        let build = |bold, italic| {
            let primary_font = styled_font(primary.clone(), bold, italic);
            let fallback_font = font_with_fallback(primary_font.clone(), fallback_families);
            (primary_font, fallback_font)
        };
        let [(normal_primary, normal_fallback), (bold_primary, bold_fallback), (italic_primary, italic_fallback), (bold_italic_primary, bold_italic_fallback)] = [
            build(false, false),
            build(true, false),
            build(false, true),
            build(true, true),
        ];
        let variants = FontStyleVariants {
            primary: [
                normal_primary,
                bold_primary,
                italic_primary,
                bold_italic_primary,
            ],
            fallback: [
                normal_fallback,
                bold_fallback,
                italic_fallback,
                bold_italic_fallback,
            ],
            selected_fallbacks: fallback_families
                .iter()
                .enumerate()
                .map(|(index, family)| {
                    let build_selected = |bold, italic| {
                        font_with_fallback(
                            styled_font(font(family.clone()), bold, italic),
                            &fallback_families[index + 1..],
                        )
                    };
                    [
                        build_selected(false, false),
                        build_selected(true, false),
                        build_selected(false, true),
                        build_selected(true, true),
                    ]
                })
                .collect(),
        };
        self.entries.insert(key, variants.clone());
        variants
    }
}

pub(super) fn font_with_fallback(mut base_font: Font, fallback_families: &[String]) -> Font {
    if fallback_families.is_empty() {
        return base_font;
    }
    let mut fallback_families = fallback_families.to_vec();
    if let Some(fallbacks) = base_font.fallbacks.as_ref() {
        let existing_fallbacks = fallbacks.fallback_list().to_vec();
        for family in existing_fallbacks {
            if !fallback_families.iter().any(|item| item == &family) {
                fallback_families.push(family);
            }
        }
    }
    base_font.fallbacks = Some(FontFallbacks::from_fonts(fallback_families));
    base_font
}

pub(super) fn styled_font(mut font: Font, bold: bool, italic: bool) -> Font {
    if italic {
        font = font.italic();
    }
    if bold {
        font = font.bold();
    }
    font
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FontSelection {
    Primary,
    Fallback(usize),
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FontSelectionKey {
    role: FontRole,
    font: Font,
    character: char,
}

/// Caches the explicit fallback family selected for a character.
///
/// The cache deliberately stores only a small selection index. It does not
/// retain visual cells, shaped lines, or rendered text. Font configuration
/// changes clear it because the same role may then refer to a different font
/// chain.
#[derive(Default)]
pub(crate) struct FontSelectionCache {
    entries: HashMap<FontSelectionKey, FontSelection>,
}

pub(crate) type SharedFontSelectionCache = Rc<RefCell<FontSelectionCache>>;

impl FontSelectionCache {
    pub(crate) fn shared() -> SharedFontSelectionCache {
        Rc::new(RefCell::new(Self::default()))
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn get(
        &self,
        role: FontRole,
        font: &Font,
        character: char,
    ) -> Option<FontSelection> {
        self.entries
            .get(&FontSelectionKey {
                role,
                font: font.clone(),
                character,
            })
            .copied()
    }

    pub(crate) fn insert(
        &mut self,
        role: FontRole,
        font: Font,
        character: char,
        selection: FontSelection,
    ) {
        self.entries.insert(
            FontSelectionKey {
                role,
                font,
                character,
            },
            selection,
        );
    }
}

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
        // `resolve_font` falls back to the platform's default stack when a
        // requested family is unavailable. Do not let that unrelated font
        // make an invalid configured family look like it contains the glyph.
        let contains = text_system
            .get_font_for_id(requested_font)
            .is_some_and(|resolved_font| {
                resolved_font
                    .family
                    .eq_ignore_ascii_case(font.family.as_ref())
                    && text_system
                        .typographic_bounds(requested_font, px(16.0), character)
                        .is_ok()
            });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_style_cache_keeps_four_styles_per_font_chain() {
        let mut cache = FontStyleCache::default();
        let fallback_families = vec!["Fallback Font".to_owned()];
        let variants =
            cache.get_or_insert(FontRole::Normal, font("Primary Font"), &fallback_families);

        assert_eq!(
            variants.font(FontStyleKind::Normal, false),
            font("Primary Font")
        );
        assert_eq!(
            variants.font(FontStyleKind::Bold, false),
            font("Primary Font").bold()
        );
        assert_eq!(
            variants.font(FontStyleKind::Italic, false),
            font("Primary Font").italic()
        );
        assert_eq!(
            variants.font(FontStyleKind::BoldItalic, false),
            font("Primary Font").italic().bold()
        );
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(
            variants
                .font(FontStyleKind::BoldItalic, true)
                .fallbacks
                .expect("fallback chain")
                .fallback_list(),
            fallback_families.as_slice()
        );
        let selected = variants.selected_font(FontStyleKind::Bold, FontSelection::Fallback(0));
        assert_eq!(selected.family.as_ref(), "Fallback Font");
        assert_eq!(selected.weight, font("Fallback Font").bold().weight);
    }
}
