use super::EditorRuntime;
use crate::app::{
    DEFAULT_GRID_CELL_WIDTH, DEFAULT_GRID_FONT_FAMILY, DEFAULT_GRID_FONT_SIZE,
    DEFAULT_GRID_LINE_HEIGHT, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH,
    PREFERRED_SYSTEM_MONOSPACE_FONTS, PREFERRED_SYSTEM_WIDE_FONTS, THEMED_TITLEBAR_HEIGHT,
};
use gpui::{font, px, size, Pixels, Window};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GuiFontSpec {
    pub(crate) family: String,
    pub(crate) size: f32,
    pub(crate) fallback_family: Option<String>,
}

impl Default for GuiFontSpec {
    fn default() -> Self {
        Self {
            family: DEFAULT_GRID_FONT_FAMILY.to_owned(),
            size: DEFAULT_GRID_FONT_SIZE,
            fallback_family: None,
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
            fallback_family: None,
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
            fallback_family: None,
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
            .as_deref()
            .map(|family| GuiFontSpec {
                family: family.to_owned(),
                size: font_size,
                fallback_family: fallback.as_ref().map(|font| font.family.clone()),
            })
            .unwrap_or_else(|| GuiFontSpec::system(window));
        if self.configured_grid_font.is_none() {
            font.size = font_size;
            font.fallback_family = fallback.map(|fallback| fallback.family);
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
            .as_deref()
            .map(|family| GuiFontSpec {
                family: family.to_owned(),
                size: font_size,
                fallback_family: fallback.as_ref().map(|font| font.family.clone()),
            })
            .unwrap_or_else(|| GuiFontSpec::system_wide(window));
        if self.configured_grid_wide_font.is_none() {
            font.size = font_size;
            font.fallback_family = fallback.map(|fallback| fallback.family);
        }
        self.resolved_grid_wide_font = Some(font.clone());
        font
    }
}

pub(crate) fn system_monospace_families(window: &Window) -> Vec<String> {
    let font_size = px(DEFAULT_GRID_FONT_SIZE);
    let mut families = window
        .text_system()
        .all_font_names()
        .into_iter()
        .filter(|family| !family.starts_with('.') && is_monospace_family(window, family, font_size))
        .collect::<Vec<_>>();
    order_system_font_families(&mut families, PREFERRED_SYSTEM_MONOSPACE_FONTS);
    families
}

pub(crate) fn system_unicode_families(window: &Window) -> Vec<String> {
    let font_size = px(DEFAULT_GRID_FONT_SIZE);
    let mut families = window
        .text_system()
        .all_font_names()
        .into_iter()
        .filter(|family| {
            !family.starts_with('.') && supports_unicode_family(window, family, font_size)
        })
        .collect::<Vec<_>>();
    order_system_font_families(&mut families, PREFERRED_SYSTEM_WIDE_FONTS);
    families
}

const UNICODE_FONT_SAMPLE: &[char] = &['中', '文', '日', '本', '한', '🙂'];

fn supports_unicode_family(window: &Window, family: &str, font_size: Pixels) -> bool {
    let text_system = window.text_system();
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
    let text_system = window.text_system();
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
    let first_font = spec.split(',').next().unwrap_or(spec);
    let mut parts = first_font.split(':');
    let family = parts.next().unwrap_or_default().replace("\\:", ":");
    let family = if family.trim().is_empty() {
        GuiFontSpec::default().family
    } else {
        family
    };
    let size = parts
        .find_map(|part| part.strip_prefix('h'))
        .and_then(|size| size.parse::<f32>().ok())
        .filter(|size| *size > 0.0)
        .unwrap_or(DEFAULT_GRID_FONT_SIZE);

    GuiFontSpec {
        family,
        size,
        fallback_family: None,
    }
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
