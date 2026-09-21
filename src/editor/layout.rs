use super::EditorRuntime;
use crate::app::{
    DEFAULT_GRID_CELL_WIDTH, DEFAULT_GRID_FONT_FAMILY, DEFAULT_GRID_FONT_SIZE,
    DEFAULT_GRID_LINE_HEIGHT, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH,
    PREFERRED_SYSTEM_MONOSPACE_FONTS, PREFERRED_SYSTEM_WIDE_FONTS, THEMED_TITLEBAR_HEIGHT,
};
use gpui::{font, px, size, Pixels, Window, WindowTextSystem};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GuiFontSpec {
    pub(crate) family: String,
    pub(crate) size: f32,
    pub(crate) fallback_families: Vec<String>,
}

impl Default for GuiFontSpec {
    fn default() -> Self {
        Self {
            family: DEFAULT_GRID_FONT_FAMILY.to_owned(),
            size: DEFAULT_GRID_FONT_SIZE,
            fallback_families: Vec::new(),
        }
    }
}

impl GuiFontSpec {
    pub(crate) fn system(window: &Window) -> Self {
        let available_fonts = window.text_system().all_font_names();
        let font_size = px(DEFAULT_GRID_FONT_SIZE);
        let family = PREFERRED_SYSTEM_MONOSPACE_FONTS
            .iter()
            .find_map(|preferred| {
                let installed = available_fonts
                    .iter()
                    .find(|name| name.eq_ignore_ascii_case(preferred))?;
                is_monospace_family(window, installed, font_size).then(|| installed.clone())
            })
            .or_else(|| {
                available_fonts
                    .iter()
                    .find(|name| is_monospace_family(window, name, font_size))
                    .cloned()
            })
            // GPUI normally exposes at least one system monospace font. Keep
            // a last-resort value for unusual platforms with incomplete font
            // enumeration; the normal path above is runtime-selected.
            .unwrap_or_else(|| Self::default().family);

        Self {
            family,
            size: DEFAULT_GRID_FONT_SIZE,
            fallback_families: Vec::new(),
        }
    }

    pub(crate) fn system_wide(window: &Window) -> Self {
        let available_fonts = window.text_system().all_font_names();
        let family = PREFERRED_SYSTEM_WIDE_FONTS
            .iter()
            .find_map(|preferred| {
                available_fonts
                    .iter()
                    .find(|name| name.eq_ignore_ascii_case(preferred))
                    .cloned()
            })
            .unwrap_or_else(|| Self::system(window).family);

        Self {
            family,
            size: DEFAULT_GRID_FONT_SIZE,
            fallback_families: Vec::new(),
        }
    }

    pub(crate) fn line_height(&self, window: &Window, linespace: f32) -> Pixels {
        let font = font(self.family.clone());
        let font_size = px(self.size);
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font);
        let glyph_height =
            text_system.ascent(font_id, font_size) + text_system.descent(font_id, font_size);

        line_height_from_metrics(glyph_height, font_size, linespace)
    }

    pub(crate) fn cell_width(&self, window: &Window) -> Pixels {
        let font = font(self.family.clone());
        let font_size = px(self.size);
        window
            .text_system()
            .ch_advance(window.text_system().resolve_font(&font), font_size)
            .map(|advance| advance.max(px(1.0)))
            .unwrap_or_else(|_| px(self.size * 0.6))
    }
}

impl EditorRuntime {
    pub(crate) fn current_grid_font(&mut self, window: &Window) -> GuiFontSpec {
        if let Some(font) = &self.resolved_grid_font {
            return font.clone();
        }

        let fallback = self
            .protocol
            .guifont
            .as_deref()
            .filter(|spec| !spec.trim().is_empty())
            .map(parse_guifont_spec);
        let font_size = self.configured_grid_font_size.unwrap_or_else(|| {
            fallback
                .as_ref()
                .map(|font| font.size)
                .unwrap_or(DEFAULT_GRID_FONT_SIZE)
        });
        let mut font = self
            .configured_grid_font
            .as_ref()
            .map(|families| {
                let mut families = families.clone();
                append_font_families(
                    &mut families,
                    fallback.as_ref().map(|font| {
                        std::iter::once(font.family.clone())
                            .chain(font.fallback_families.iter().cloned())
                            .collect::<Vec<_>>()
                    }),
                );
                GuiFontSpec::from_families(families, font_size)
            })
            .unwrap_or_else(|| GuiFontSpec::system(window));
        if self.configured_grid_font.is_none() {
            font.size = font_size;
            if let Some(fallback) = fallback {
                font.fallback_families = std::iter::once(fallback.family)
                    .chain(fallback.fallback_families)
                    .collect();
            }
        }
        self.resolved_grid_font = Some(font.clone());
        font
    }

    pub(crate) fn current_grid_wide_font(&mut self, window: &Window) -> GuiFontSpec {
        if let Some(font) = &self.resolved_grid_wide_font {
            return font.clone();
        }

        let fallback = self
            .protocol
            .guifontwide
            .as_deref()
            .filter(|spec| !spec.trim().is_empty())
            .or_else(|| {
                self.protocol
                    .guifont
                    .as_deref()
                    .filter(|spec| !spec.trim().is_empty())
            })
            .map(parse_guifont_spec);
        let font_size = self.configured_grid_font_size.unwrap_or_else(|| {
            fallback
                .as_ref()
                .map(|font| font.size)
                .unwrap_or(DEFAULT_GRID_FONT_SIZE)
        });
        let mut font = self
            .configured_grid_wide_font
            .as_ref()
            .map(|families| {
                let mut families = families.clone();
                append_font_families(
                    &mut families,
                    fallback.as_ref().map(|font| {
                        std::iter::once(font.family.clone())
                            .chain(font.fallback_families.iter().cloned())
                            .collect::<Vec<_>>()
                    }),
                );
                GuiFontSpec::from_families(families, font_size)
            })
            .unwrap_or_else(|| GuiFontSpec::system_wide(window));
        if self.configured_grid_wide_font.is_none() {
            font.size = font_size;
            if let Some(fallback) = fallback {
                font.fallback_families = std::iter::once(fallback.family)
                    .chain(fallback.fallback_families)
                    .collect();
            }
        }
        self.resolved_grid_wide_font = Some(font.clone());
        font
    }
}

pub(crate) fn system_font_families(text_system: &WindowTextSystem) -> (Vec<String>, Vec<String>) {
    let font_size = px(DEFAULT_GRID_FONT_SIZE);
    let mut monospace_families = Vec::new();
    let mut unicode_families = Vec::new();

    for family in text_system.all_font_names() {
        if family.starts_with('.') {
            continue;
        }
        if is_monospace_family_with_text_system(text_system, &family, font_size) {
            monospace_families.push(family.clone());
        }
        if supports_unicode_family(text_system, &family, font_size) {
            unicode_families.push(family);
        }
    }

    order_system_font_families(&mut monospace_families, PREFERRED_SYSTEM_MONOSPACE_FONTS);
    order_system_font_families(&mut unicode_families, PREFERRED_SYSTEM_WIDE_FONTS);
    (monospace_families, unicode_families)
}

const UNICODE_FONT_SAMPLE: &[char] = &['中', '文', '日', '本', '한', '🙂'];

fn supports_unicode_family(
    text_system: &WindowTextSystem,
    family: &str,
    font_size: Pixels,
) -> bool {
    let requested_font = font(family.to_owned());
    let font_id = text_system.resolve_font(&requested_font);

    // `resolve_font` silently falls back when a family cannot be loaded. Do
    // not let that fallback make an unrelated font look like a Unicode font.
    let Some(resolved_font) = text_system.get_font_for_id(font_id) else {
        return false;
    };
    if !resolved_font
        .family
        .eq_ignore_ascii_case(requested_font.family.as_ref())
    {
        return false;
    }

    // Test glyph coverage directly. Family names and naming conventions are
    // not reliable indicators of whether a font contains CJK or other wide
    // Unicode characters (for example, LXGW WenKai).
    UNICODE_FONT_SAMPLE.iter().any(|&character| {
        text_system
            .typographic_bounds(font_id, font_size, character)
            .is_ok()
    })
}

fn order_system_font_families(families: &mut Vec<String>, preferred: &[&str]) {
    families.sort_by_key(|family| {
        (
            preferred
                .iter()
                .position(|name| name.eq_ignore_ascii_case(family))
                .unwrap_or(usize::MAX),
            family.to_ascii_lowercase(),
        )
    });
    families.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
}

pub(crate) fn is_monospace_family(window: &Window, family: &str, font_size: Pixels) -> bool {
    is_monospace_family_with_text_system(window.text_system(), family, font_size)
}

fn is_monospace_family_with_text_system(
    text_system: &WindowTextSystem,
    family: &str,
    font_size: Pixels,
) -> bool {
    let font_id = text_system.resolve_font(&font(family.to_owned()));
    let Some(reference) = text_system
        .advance(font_id, font_size, '0')
        .ok()
        .map(|advance| f32::from(advance.width))
    else {
        return false;
    };

    ['M', 'i', 'W', ' '].into_iter().all(|character| {
        text_system
            .advance(font_id, font_size, character)
            .ok()
            .map(|advance| (f32::from(advance.width) - reference).abs() <= 0.01)
            .unwrap_or(false)
    })
}

pub(crate) fn parse_guifont_spec(spec: &str) -> GuiFontSpec {
    let mut families = Vec::new();
    let mut size = None;
    for entry in split_escaped(spec, ',') {
        let parts = split_escaped(&entry, ':');
        let family = parts
            .first()
            .map(|part| unescape_font_text(part).trim().to_owned())
            .unwrap_or_default();
        if family.is_empty() {
            continue;
        }
        if size.is_none() {
            size = parts.iter().skip(1).find_map(|part| {
                let part = unescape_font_text(part);
                part.strip_prefix('h')
                    .and_then(|size| size.parse::<f32>().ok())
                    .filter(|size| *size > 0.0)
            });
        }
        families.push(family);
    }

    GuiFontSpec::from_families(families, size.unwrap_or(DEFAULT_GRID_FONT_SIZE))
}

impl GuiFontSpec {
    fn from_families(mut families: Vec<String>, size: f32) -> Self {
        deduplicate_font_families(&mut families);
        let family = families
            .first()
            .cloned()
            .unwrap_or_else(|| GuiFontSpec::default().family);
        Self {
            family,
            size,
            fallback_families: families.into_iter().skip(1).collect(),
        }
    }
}

pub(crate) fn parse_guifont_families(spec: &str) -> Vec<String> {
    if spec.trim().is_empty() {
        return Vec::new();
    }
    let parsed = parse_guifont_spec(spec);
    std::iter::once(parsed.family)
        .chain(parsed.fallback_families)
        .collect()
}

pub(crate) fn format_guifont_families(families: &[String]) -> String {
    let mut families = families.to_vec();
    deduplicate_font_families(&mut families);
    families
        .iter()
        .map(|family| escape_font_text(family))
        .collect::<Vec<_>>()
        .join(",")
}

fn append_font_families(target: &mut Vec<String>, additional: Option<Vec<String>>) {
    let Some(additional) = additional else {
        return;
    };
    target.extend(additional);
    deduplicate_font_families(target);
}

fn deduplicate_font_families(families: &mut Vec<String>) {
    families.retain(|family| !family.trim().is_empty());
    let mut unique = Vec::with_capacity(families.len());
    for family in families.drain(..) {
        if !unique
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(&family))
        {
            unique.push(family);
        }
    }
    *families = unique;
}

fn split_escaped(value: &str, delimiter: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            current.push('\\');
            current.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == delimiter {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(character);
        }
    }
    if escaped {
        current.push('\\');
    }
    parts.push(current);
    parts
}

fn unescape_font_text(value: &str) -> String {
    let mut unescaped = String::new();
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            unescaped.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else {
            unescaped.push(character);
        }
    }
    if escaped {
        unescaped.push('\\');
    }
    unescaped
}

fn escape_font_text(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| {
            if matches!(character, '\\' | ',' | ':') {
                Some('\\')
            } else {
                None
            }
            .into_iter()
            .chain(std::iter::once(character))
        })
        .collect()
}

pub(crate) fn line_height_from_metrics(
    glyph_height: Pixels,
    font_size: Pixels,
    linespace: f32,
) -> Pixels {
    let minimum_line_height = font_size * 1.2;

    // GPUI 0.2.2 does not expose the font's line-gap metric. Use the actual
    // glyph metrics and a compact 1.2em minimum cell height instead of
    // scaling a historical default ratio. Neovim's `linespace` remains the
    // only user-configured extra spacing.
    px(
        (f32::from(glyph_height.max(minimum_line_height)) + linespace)
            .ceil()
            .max(1.0),
    )
}

pub(crate) fn initial_window_size_for_grid(width: u32, height: u32) -> gpui::Size<Pixels> {
    let titlebar_height = if cfg!(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows"
    )) {
        THEMED_TITLEBAR_HEIGHT
    } else {
        0.0
    };
    size(
        px((width as f32 * DEFAULT_GRID_CELL_WIDTH).max(MIN_WINDOW_WIDTH)),
        px((height as f32 * DEFAULT_GRID_LINE_HEIGHT + titlebar_height).max(MIN_WINDOW_HEIGHT)),
    )
}
