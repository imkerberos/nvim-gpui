use super::super::*;

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

    GuiFontSpec { family, size }
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

pub(crate) fn parse_non_negative_float(value: &str) -> Option<f32> {
    let value = value.parse::<f32>().ok()?;
    value.is_finite().then_some(value.max(0.0))
}

pub(crate) fn initial_window_size_for_grid(width: u32, height: u32) -> gpui::Size<Pixels> {
    let titlebar_height = if themed_titlebar_enabled() {
        THEMED_TITLEBAR_HEIGHT
    } else {
        0.0
    };
    size(
        px((width as f32 * DEFAULT_GRID_CELL_WIDTH).max(MIN_WINDOW_WIDTH)),
        px((height as f32 * DEFAULT_GRID_LINE_HEIGHT + titlebar_height).max(MIN_WINDOW_HEIGHT)),
    )
}
