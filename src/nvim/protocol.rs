use crate::grid::{CursorModeInfo, CursorShape, GridLineCell, HighlightAttrs, HighlightId};
use rmpv::Value;

use super::{NvimCapabilities, NvimEvent, CLIENT_NAME};

pub(super) fn client_info_params(methods: impl IntoIterator<Item = String>) -> Value {
    let methods = methods
        .into_iter()
        .map(|method| (Value::from(method), Value::Map(Vec::new())))
        .collect();
    Value::Array(vec![
        Value::from(CLIENT_NAME),
        Value::Map(vec![
            (Value::from("major"), Value::from(0)),
            (Value::from("minor"), Value::from(1)),
            (Value::from("patch"), Value::from(0)),
        ]),
        Value::from("ui"),
        Value::Map(Vec::new()),
        Value::Map(methods),
    ])
}

#[cfg(test)]
pub(crate) fn ui_attach_params(width: u32, height: u32) -> Value {
    ui_attach_params_for(width, height, &NvimCapabilities::default())
}

pub(super) fn ui_attach_params_for(
    width: u32,
    height: u32,
    capabilities: &NvimCapabilities,
) -> Value {
    let mut options = vec![
        (Value::from("rgb"), Value::Boolean(true)),
        (Value::from("ext_linegrid"), Value::Boolean(true)),
        (Value::from("ext_multigrid"), Value::Boolean(true)),
    ];

    if capabilities.supports_ui_option("ext_hlstate") {
        // Include semantic UI highlight names so the renderer can give
        // floating grids their NormalFloat surface when a cell falls
        // back to the default highlight id.
        options.push((Value::from("ext_hlstate"), Value::Boolean(true)));
    }
    // Neovim 0.12 exposes the TTY flags as attach options but omits them from
    // `api-metadata.ui_options`. Snacks Dashboard uses these flags to
    // distinguish an interactive GUI from a piped TUI, so the same fallback
    // used for `stdout_tty` must also cover `stdin_tty`.
    if capabilities.supports_ui_option("stdin_tty") || capabilities.supports_ui_event("ui_send") {
        // GPUI supplies interactive keyboard input through nvim_input.
        // Mark it as a TTY-like input so plugins such as Snacks Dashboard
        // do not mistake this UI for a non-interactive/piped frontend.
        options.push((Value::from("stdin_tty"), Value::Boolean(true)));
    }
    // Neovim 0.12 exposes `ui_send`, but does not advertise the embed-only
    // `stdout_tty` option in `api-metadata.ui_options`. Enable it from the
    // event capability as well so Kitty/terminal data reaches the UI.
    if capabilities.supports_ui_option("stdout_tty") || capabilities.supports_ui_event("ui_send") {
        options.push((Value::from("stdout_tty"), Value::Boolean(true)));
    }

    Value::Array(vec![
        Value::from(width),
        Value::from(height),
        Value::Map(options),
    ])
}

pub(super) fn resize_request_frame(id: u64, width: u32, height: u32) -> Value {
    Value::Array(vec![
        Value::from(0),
        Value::from(id),
        Value::from("nvim_ui_try_resize"),
        Value::Array(vec![Value::from(width), Value::from(height)]),
    ])
}

pub(super) fn mouse_event_notification_frame(
    button: String,
    action: String,
    modifier: String,
    grid: u64,
    row: u64,
    col: u64,
) -> Value {
    Value::Array(vec![
        Value::from(2),
        Value::from("nvim_input_mouse"),
        Value::Array(vec![
            Value::from(button),
            Value::from(action),
            Value::from(modifier),
            Value::from(grid),
            Value::from(row),
            Value::from(col),
        ]),
    ])
}

pub(super) fn parse_hl_attr_define(args: &[Value]) -> Result<NvimEvent, String> {
    if !(2..=4).contains(&args.len()) {
        return Err(format!(
            "hl_attr_define expects 2 to 4 arguments, got {}",
            args.len()
        ));
    }
    let id = args[0]
        .as_u64()
        .ok_or_else(|| "hl_attr_define has an invalid highlight id".to_owned())?;
    let mut attrs = parse_highlight_attrs(&args[1])?;
    attrs.ui_name = args.get(3).and_then(parse_ui_highlight_name);
    Ok(NvimEvent::HlAttrDefine {
        id: HighlightId(id),
        attrs,
    })
}

pub(super) fn parse_ui_highlight_name(value: &Value) -> Option<String> {
    value.as_array()?.iter().rev().find_map(|info| {
        (map_value(info, "kind").and_then(string_value).as_deref() == Some("ui"))
            .then(|| map_value(info, "ui_name").and_then(string_value))
            .flatten()
    })
}

pub(super) fn parse_mode_info_set(args: &[Value]) -> Result<NvimEvent, String> {
    if args.len() != 2 {
        return Err(format!(
            "mode_info_set expects 2 arguments, got {}",
            args.len()
        ));
    }
    let cursor_style_enabled = bool_value(&args[0])
        .ok_or_else(|| "mode_info_set has an invalid cursor_style_enabled flag".to_owned())?;
    let modes = args[1]
        .as_array()
        .ok_or_else(|| "mode_info_set modes are not an array".to_owned())?
        .iter()
        .map(parse_cursor_mode_info)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(NvimEvent::ModeInfoSet {
        cursor_style_enabled,
        modes,
    })
}

pub(super) fn parse_cursor_mode_info(value: &Value) -> Result<CursorModeInfo, String> {
    let entries = value
        .as_map()
        .ok_or_else(|| "mode_info_set mode is not a map".to_owned())?;
    let mut mode = CursorModeInfo::default();

    if let Some(shape) = map_value(value, "cursor_shape").and_then(string_value) {
        mode.shape = match shape.as_str() {
            "block" => CursorShape::Block,
            "horizontal" => CursorShape::Horizontal,
            "vertical" => CursorShape::Vertical,
            _ => CursorShape::Block,
        };
    }

    for (key, value) in entries {
        let Some(key) = string_value(key) else {
            continue;
        };
        match key.as_str() {
            "cell_percentage" => mode.cell_percentage = parse_percentage(value, "cell_percentage")?,
            "blinkwait" => mode.blink_wait = parse_u32(value, "blinkwait")?,
            "blinkon" => mode.blink_on = parse_u32(value, "blinkon")?,
            "blinkoff" => mode.blink_off = parse_u32(value, "blinkoff")?,
            "attr_id" => mode.attr_id = parse_optional_highlight_id(value, "attr_id")?,
            "attr_id_lm" => mode.attr_id_lm = parse_optional_highlight_id(value, "attr_id_lm")?,
            _ => {}
        }
    }

    Ok(mode)
}

pub(super) fn parse_percentage(value: &Value, name: &str) -> Result<u8, String> {
    let value = parse_u32(value, name)?;
    Ok(value.min(100) as u8)
}

pub(super) fn parse_u32(value: &Value, name: &str) -> Result<u32, String> {
    value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| format!("mode_info_set has an invalid {name}"))
}

pub(super) fn parse_optional_highlight_id(
    value: &Value,
    name: &str,
) -> Result<Option<HighlightId>, String> {
    value
        .as_u64()
        .map(|value| Some(HighlightId(value)))
        .ok_or_else(|| format!("mode_info_set has an invalid {name}"))
}

pub(super) fn parse_highlight_attrs(value: &Value) -> Result<HighlightAttrs, String> {
    let entries = value
        .as_map()
        .ok_or_else(|| "hl_attr_define RGB attributes are not a map".to_owned())?;
    let mut attrs = HighlightAttrs::default();

    for (key, value) in entries {
        let Some(key) = string_value(key) else {
            continue;
        };
        match key.as_str() {
            "foreground" => attrs.foreground = parse_color(value, "foreground")?,
            "background" => attrs.background = parse_color(value, "background")?,
            "special" => attrs.special = parse_color(value, "special")?,
            "reverse" => attrs.reverse = parse_bool(value, "reverse")?,
            "italic" => attrs.italic = parse_bool(value, "italic")?,
            "bold" => attrs.bold = parse_bool(value, "bold")?,
            "strikethrough" => attrs.strikethrough = parse_bool(value, "strikethrough")?,
            "underline" => attrs.underline = parse_bool(value, "underline")?,
            "undercurl" => attrs.undercurl = parse_bool(value, "undercurl")?,
            "underdouble" => attrs.underdouble = parse_bool(value, "underdouble")?,
            "underdotted" => attrs.underdotted = parse_bool(value, "underdotted")?,
            "underdashed" => attrs.underdashed = parse_bool(value, "underdashed")?,
            "dim" => attrs.dim = parse_bool(value, "dim")?,
            "blink" => attrs.blink = parse_bool(value, "blink")?,
            "conceal" => attrs.conceal = parse_bool(value, "conceal")?,
            "overline" => attrs.overline = parse_bool(value, "overline")?,
            "altfont" => {
                attrs.altfont = Some(
                    value
                        .as_u64()
                        .and_then(|value| u32::try_from(value).ok())
                        .ok_or_else(|| "hl_attr_define has an invalid altfont".to_owned())?,
                )
            }
            "url" => {
                attrs.url = Some(
                    string_value(value)
                        .ok_or_else(|| "hl_attr_define has an invalid url".to_owned())?,
                )
            }
            "blend" => {
                // Neovim uses -1 internally for "no explicit blend" before
                // applying a floating window's winblend. If it reaches a UI,
                // preserve that sentinel as None instead of turning it into
                // opaque blend=0. Positive values are the final per-cell
                // blend level (winblend is already folded into the attr).
                attrs.blend = if value.as_i64() == Some(-1) {
                    None
                } else {
                    Some(parse_percentage(value, "blend")?)
                };
            }
            _ => {}
        }
    }

    Ok(attrs)
}

pub(super) fn parse_color(value: &Value, name: &str) -> Result<Option<u32>, String> {
    if value.as_i64() == Some(-1) {
        return Ok(None);
    }

    value
        .as_u64()
        .and_then(|color| u32::try_from(color).ok())
        .map(Some)
        .ok_or_else(|| format!("hl_attr_define has an invalid {name} color"))
}

pub(super) fn parse_bool(value: &Value, name: &str) -> Result<bool, String> {
    bool_value(value).ok_or_else(|| format!("hl_attr_define has an invalid {name} flag"))
}

pub(super) fn parse_grid_line(args: &[Value]) -> Result<NvimEvent, String> {
    if !(4..=5).contains(&args.len()) {
        return Err(format!(
            "grid_line expects 4 to 5 arguments, got {}",
            args.len()
        ));
    }
    let grid = args[0]
        .as_u64()
        .ok_or_else(|| "grid_line has an invalid grid id".to_owned())?;
    let row = args[1]
        .as_u64()
        .ok_or_else(|| "grid_line has an invalid row".to_owned())?;
    let col_start = args[2]
        .as_u64()
        .ok_or_else(|| "grid_line has an invalid column".to_owned())?;
    let raw_cells = args[3]
        .as_array()
        .ok_or_else(|| "grid_line cells are not an array".to_owned())?;
    let mut highlight = HighlightId(0);
    let mut cells = Vec::with_capacity(raw_cells.len());

    for raw_cell in raw_cells {
        let values = raw_cell
            .as_array()
            .ok_or_else(|| "grid_line cell is not an array".to_owned())?;
        let text = values
            .first()
            .and_then(string_value)
            .ok_or_else(|| "grid_line cell has no text".to_owned())?;

        if let Some(value) = values.get(1) {
            highlight = HighlightId(
                value
                    .as_u64()
                    .ok_or_else(|| "grid_line cell has an invalid highlight id".to_owned())?,
            );
        }

        let repeat = values
            .get(2)
            .map(|value| {
                value
                    .as_u64()
                    .ok_or_else(|| "grid_line cell has an invalid repeat count".to_owned())
            })
            .transpose()?
            .unwrap_or(1);

        cells.push(GridLineCell::new(text, highlight, repeat as usize));
    }

    Ok(NvimEvent::GridLine {
        grid,
        row,
        col_start,
        cells,
        wraps_to_next: args
            .get(4)
            .map(|value| {
                bool_value(value)
                    .ok_or_else(|| "grid_line has an invalid wraps_to_next flag".to_owned())
            })
            .transpose()?
            .unwrap_or(false),
    })
}

pub(super) fn parse_win_pos(args: &[Value]) -> Result<NvimEvent, String> {
    Ok(NvimEvent::WinPos {
        grid: parse_u64_value(&args[0], "win_pos grid")?,
        win: parse_window_id(&args[1])?,
        row: parse_u64_value(&args[2], "win_pos row")?,
        col: parse_u64_value(&args[3], "win_pos column")?,
        width: parse_u64_value(&args[4], "win_pos width")?,
        height: parse_u64_value(&args[5], "win_pos height")?,
    })
}

pub(super) fn parse_win_float_pos(args: &[Value]) -> Result<NvimEvent, String> {
    Ok(NvimEvent::WinFloatPos {
        grid: parse_u64_value(&args[0], "win_float_pos grid")?,
        win: parse_window_id(&args[1])?,
        anchor: string_value(&args[2]).unwrap_or_default(),
        anchor_grid: parse_u64_value(&args[3], "win_float_pos anchor grid")?,
        anchor_row: parse_i64_value(&args[4], "win_float_pos anchor row")?,
        anchor_col: parse_i64_value(&args[5], "win_float_pos anchor column")?,
        mouse_enabled: bool_value(&args[6])
            .ok_or_else(|| "win_float_pos has an invalid mouse flag".to_owned())?,
        zindex: parse_i64_value(&args[7], "win_float_pos z-index")?,
        compindex: parse_i64_value(&args[8], "win_float_pos composition index")?,
        screen_row: parse_i64_value(&args[9], "win_float_pos screen row")?,
        screen_col: parse_i64_value(&args[10], "win_float_pos screen column")?,
    })
}

pub(super) fn parse_win_viewport(args: &[Value]) -> Result<NvimEvent, String> {
    Ok(NvimEvent::WinViewport {
        grid: parse_u64_value(&args[0], "win_viewport grid")?,
        win: parse_window_id(&args[1])?,
        topline: parse_u64_value(&args[2], "win_viewport topline")?,
        botline: parse_u64_value(&args[3], "win_viewport botline")?,
        curline: parse_u64_value(&args[4], "win_viewport curline")?,
        curcol: parse_u64_value(&args[5], "win_viewport curcol")?,
        line_count: parse_u64_value(&args[6], "win_viewport line_count")?,
        scroll_delta: parse_i64_value(&args[7], "win_viewport scroll delta")?,
    })
}

pub(super) fn parse_win_viewport_margins(args: &[Value]) -> Result<NvimEvent, String> {
    Ok(NvimEvent::WinViewportMargins {
        grid: parse_u64_value(&args[0], "win_viewport_margins grid")?,
        win: parse_window_id(&args[1])?,
        top: parse_u64_value(&args[2], "win_viewport_margins top")?,
        bottom: parse_u64_value(&args[3], "win_viewport_margins bottom")?,
        left: parse_u64_value(&args[4], "win_viewport_margins left")?,
        right: parse_u64_value(&args[5], "win_viewport_margins right")?,
    })
}

pub(super) fn parse_msg_set_pos(args: &[Value]) -> Result<NvimEvent, String> {
    Ok(NvimEvent::MsgSetPos {
        grid: parse_u64_value(&args[0], "msg_set_pos grid")?,
        row: parse_u64_value(&args[1], "msg_set_pos row")?,
        scrolled: bool_value(&args[2])
            .ok_or_else(|| "msg_set_pos has an invalid scrolled flag".to_owned())?,
        sep_char: string_value(&args[3])
            .ok_or_else(|| "msg_set_pos has an invalid separator character".to_owned())?,
        zindex: parse_i64_value(&args[4], "msg_set_pos z-index")?,
        compindex: parse_i64_value(&args[5], "msg_set_pos composition index")?,
    })
}

pub(super) fn parse_u64_value(value: &Value, name: &str) -> Result<u64, String> {
    value.as_u64().ok_or_else(|| format!("{name} is invalid"))
}

pub(super) fn parse_window_id(value: &Value) -> Result<Vec<u8>, String> {
    match value {
        Value::Ext(_, bytes) => Ok(bytes.clone()),
        _ => Err(format!(
            "window id is not a MessagePack extension: {value:?}"
        )),
    }
}

pub(super) fn parse_i64_value(value: &Value, name: &str) -> Result<i64, String> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        .or_else(|| {
            value.as_f64().and_then(|value| {
                value
                    .is_finite()
                    .then_some(value.round())
                    .and_then(|value| i64::try_from(value as i128).ok())
            })
        })
        .ok_or_else(|| format!("{name} is invalid"))
}

pub(super) fn term_event_notification_frame(event: String, value: String) -> Value {
    Value::Array(vec![
        Value::from(2),
        Value::from("nvim_ui_term_event"),
        Value::Array(vec![Value::from(event), Value::from(value)]),
    ])
}

pub(super) fn display_value(value: &Value) -> String {
    string_value(value).unwrap_or_else(|| format!("{value:?}"))
}

pub(super) fn bool_value(value: &Value) -> Option<bool> {
    match value {
        Value::Boolean(value) => Some(*value),
        _ => None,
    }
}

pub(super) fn map_value<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let Value::Map(entries) = value else {
        return None;
    };

    entries.iter().find_map(|(entry_key, entry_value)| {
        (string_value(entry_key).as_deref() == Some(key)).then_some(entry_value)
    })
}

pub(super) fn string_value(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => value.as_str().map(str::to_owned),
        _ => None,
    }
}
