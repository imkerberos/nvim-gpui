use async_channel::Sender;
use rmpv::Value;
use std::io::{BufReader, Read};
use std::sync::mpsc::SyncSender;

use super::protocol::*;
use super::transport::{read_message, write_shared_message, RpcReader, SharedWriter};
use super::{
    parse_protocol_info, NvimEvent, NvimProtocolInfo, NvimTheme, PendingRequests,
    RpcRequestHandlers,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn run_session(
    writer: SharedWriter,
    reader: Box<dyn Read + Send>,
    width: u32,
    height: u32,
    events: &Sender<NvimEvent>,
    rpc_ready: &Sender<()>,
    startup_theme_sender: &std::sync::mpsc::SyncSender<NvimTheme>,
    protocol_sender: &SyncSender<NvimProtocolInfo>,
    pending_requests: &PendingRequests,
    request_handlers: &RpcRequestHandlers,
) -> Result<(), String> {
    let mut reader = BufReader::new(reader);
    let mut request_id = 1;

    let api_info = request(
        &writer,
        &mut reader,
        request_id,
        "nvim_get_api_info",
        Value::Array(Vec::new()),
        events,
        request_handlers,
    )?;
    request_id += 1;

    let protocol = parse_protocol_info(&api_info)?;
    send_event(
        events,
        NvimEvent::ApiReady {
            version: protocol.version,
            capabilities: protocol.capabilities.clone(),
        },
    )?;
    protocol_sender
        .send(protocol.clone())
        .map_err(|_| "GPUI stopped receiving Neovim protocol metadata".to_owned())?;

    for option in ["rgb", "ext_linegrid", "ext_multigrid"] {
        if !protocol.capabilities.supports_ui_option(option) {
            return Err(format!(
                "Neovim does not support required UI option {option}"
            ));
        }
    }

    let request_methods = request_handlers
        .lock()
        .map_err(|_| "RPC request handler registry is poisoned".to_owned())?
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    request(
        &writer,
        &mut reader,
        request_id,
        "nvim_set_client_info",
        client_info_params(request_methods),
        events,
        request_handlers,
    )?;
    request_id += 1;

    // `mouse` is a global option and is not part of the redraw stream until
    // it changes. Read it once before attaching so the GUI starts with the
    // same mode gate as Neovim, including user configuration loaded at boot.
    let mouse = request(
        &writer,
        &mut reader,
        request_id,
        "nvim_get_option",
        Value::Array(vec![Value::from("mouse")]),
        events,
        request_handlers,
    )?;
    request_id += 1;
    if let Some(value) = string_value(&mouse) {
        send_event(
            events,
            NvimEvent::OptionSet {
                name: "mouse".to_owned(),
                value,
            },
        )?;
    }

    request(
        &writer,
        &mut reader,
        request_id,
        "nvim_ui_attach",
        ui_attach_params_for(width, height, &protocol.capabilities),
        events,
        request_handlers,
    )?;
    let _ = rpc_ready.send_blocking(());
    send_event(events, NvimEvent::UiAttached { width, height })?;

    let mut startup_theme = NvimTheme::default();
    let mut startup_theme_sent = false;

    loop {
        let message = read_message(&mut reader)?;
        if !startup_theme_sent && observe_startup_theme(&message, &mut startup_theme) {
            let _ = startup_theme_sender.send(startup_theme);
            startup_theme_sent = true;
        }
        dispatch_message(&writer, message, events, pending_requests, request_handlers)?;
    }
}

pub(crate) fn observe_startup_theme(message: &Value, theme: &mut NvimTheme) -> bool {
    let Some(values) = message.as_array() else {
        return false;
    };
    if values.first().and_then(Value::as_u64) != Some(2)
        || values.get(1).and_then(string_value).as_deref() != Some("redraw")
    {
        return false;
    }

    let Some(params) = values.get(2) else {
        return false;
    };
    let Ok(redraw_events) = parse_redraw_events(params) else {
        return false;
    };
    let mut flushed = false;

    for event in redraw_events {
        match event {
            NvimEvent::DefaultColorsSet {
                foreground,
                background,
                ..
            } => {
                theme.default_foreground = foreground;
                theme.default_background = background;
            }
            NvimEvent::HlAttrDefine { attrs, .. } => match attrs.ui_name.as_deref() {
                Some("Normal") => {
                    theme.normal_foreground = attrs.foreground;
                    theme.normal_background = attrs.background;
                }
                Some("NormalFloat") => {
                    theme.normal_float_background = attrs.background;
                }
                _ => {}
            },
            NvimEvent::Flush => flushed = true,
            _ => {}
        }
    }

    flushed
}

fn request(
    writer: &SharedWriter,
    reader: &mut RpcReader,
    id: u64,
    method: &str,
    params: Value,
    events: &Sender<NvimEvent>,
    request_handlers: &RpcRequestHandlers,
) -> Result<Value, String> {
    write_shared_message(
        writer,
        &Value::Array(vec![
            Value::from(0),
            Value::from(id),
            Value::from(method),
            params,
        ]),
    )?;

    loop {
        let message = read_message(reader)?;
        let Some(values) = message.as_array() else {
            return Err("RPC message is not an array".to_owned());
        };

        match values.first().and_then(Value::as_u64) {
            Some(1) => {
                expect_rpc_message_len(values, 4, "RPC response")?;
                let response_id = values.get(1).and_then(Value::as_u64);
                if response_id != Some(id) {
                    return Err(format!("unexpected RPC response id: {response_id:?}"));
                }
                let error = &values[2];
                if !matches!(error, Value::Nil) {
                    return Err(format!("RPC request {method} failed: {error:?}"));
                }
                return Ok(values[3].clone());
            }
            Some(2) => {
                expect_rpc_message_len(values, 3, "RPC notification")?;
                let method = values
                    .get(1)
                    .and_then(string_value)
                    .ok_or_else(|| "RPC notification has no method".to_owned())?;
                let params = &values[2];
                handle_notification(&method, params, events)?;
            }
            Some(0) => {
                expect_rpc_message_len(values, 4, "RPC request")?;
                let request_id = values
                    .get(1)
                    .and_then(Value::as_u64)
                    .ok_or_else(|| "RPC request has no id".to_owned())?;
                let request_method = values
                    .get(2)
                    .and_then(string_value)
                    .ok_or_else(|| "RPC request has no method".to_owned())?;
                let request_params = &values[3];
                handle_request(
                    writer,
                    request_id,
                    &request_method,
                    request_params,
                    request_handlers,
                )?;
            }
            Some(tag) => return Err(format!("unexpected RPC message type: {tag}")),
            None => return Err("RPC message has no type".to_owned()),
        }
    }
}

fn dispatch_message(
    writer: &SharedWriter,
    message: Value,
    events: &Sender<NvimEvent>,
    pending_requests: &PendingRequests,
    request_handlers: &RpcRequestHandlers,
) -> Result<(), String> {
    let Some(values) = message.as_array() else {
        return Err("RPC message is not an array".to_owned());
    };

    match values.first().and_then(Value::as_u64) {
        Some(1) => {
            expect_rpc_message_len(values, 4, "RPC response")?;
            let response_id = values
                .get(1)
                .and_then(Value::as_u64)
                .ok_or_else(|| "RPC response has no id".to_owned())?;
            let error = &values[2];
            let result = if matches!(error, Value::Nil) {
                Ok(values[3].clone())
            } else {
                Err(format!(
                    "Neovim RPC request failed: {}",
                    display_value(error)
                ))
            };
            let response_sender = pending_requests
                .lock()
                .map_err(|_| "RPC request registry is poisoned".to_owned())?
                .remove(&response_id);
            if let Some(response_sender) = response_sender {
                let _ = response_sender.send_blocking(result);
            } else if let Err(error) = result {
                send_event(events, NvimEvent::Error(error))?;
            }
            Ok(())
        }
        Some(2) => {
            expect_rpc_message_len(values, 3, "RPC notification")?;
            let method = values
                .get(1)
                .and_then(string_value)
                .ok_or_else(|| "RPC notification has no method".to_owned())?;
            let params = &values[2];
            handle_notification(&method, params, events)
        }
        Some(0) => {
            expect_rpc_message_len(values, 4, "RPC request")?;
            let id = values
                .get(1)
                .and_then(Value::as_u64)
                .ok_or_else(|| "RPC request has no id".to_owned())?;
            let method = values
                .get(2)
                .and_then(string_value)
                .ok_or_else(|| "RPC request has no method".to_owned())?;
            let params = &values[3];
            handle_request(writer, id, &method, params, request_handlers)
        }
        Some(tag) => Err(format!("unexpected RPC message type: {tag}")),
        None => Err("RPC message has no type".to_owned()),
    }
}

fn handle_request(
    writer: &SharedWriter,
    id: u64,
    method: &str,
    params: &Value,
    request_handlers: &RpcRequestHandlers,
) -> Result<(), String> {
    let response = request_handlers
        .lock()
        .map_err(|_| "RPC request handler registry is poisoned".to_owned())?
        .get(method)
        .cloned()
        .map(|handler| handler(params))
        .unwrap_or_else(|| Err(format!("RPC method is not supported: {method}")));

    let frame = match response {
        Ok(result) => Value::Array(vec![Value::from(1), Value::from(id), Value::Nil, result]),
        Err(error) => Value::Array(vec![
            Value::from(1),
            Value::from(id),
            Value::from(error),
            Value::Nil,
        ]),
    };
    write_shared_message(writer, &frame)
}

pub(crate) fn handle_notification(
    method: &str,
    params: &Value,
    events: &Sender<NvimEvent>,
) -> Result<(), String> {
    if method != "redraw" {
        return Ok(());
    }

    let parsed_events = parse_redraw_events(params)?;
    for event in parsed_events {
        send_event(events, event)?;
    }

    Ok(())
}

fn parse_redraw_events(params: &Value) -> Result<Vec<NvimEvent>, String> {
    let redraw_events = params
        .as_array()
        .ok_or_else(|| "redraw notification params are not an array".to_owned())?;
    let mut parsed_events = Vec::new();

    for (event_index, redraw_event) in redraw_events.iter().enumerate() {
        let values = redraw_event
            .as_array()
            .ok_or_else(|| format!("redraw event {} is not an array", event_index))?;
        let name = values
            .first()
            .and_then(string_value)
            .ok_or_else(|| format!("redraw event {} has no valid name", event_index))?;

        if name == "flush" || name == "mouse_on" || name == "mouse_off" {
            match values.len() {
                1 => {}
                2 => {
                    let args = values[1]
                        .as_array()
                        .ok_or_else(|| format!("redraw event {} payload is not an array", name))?;
                    expect_arg_count(args, 0, 0, &name)?;
                }
                count => {
                    return Err(format!(
                        "redraw event {} expects no arguments, got {}",
                        name,
                        count - 1
                    ));
                }
            }
            parsed_events.push(match name.as_str() {
                "flush" => NvimEvent::Flush,
                "mouse_on" => NvimEvent::MouseEnabled(true),
                "mouse_off" => NvimEvent::MouseEnabled(false),
                _ => unreachable!(),
            });
            continue;
        }

        if !is_known_redraw_event(&name) {
            continue;
        }

        if values.len() < 2 {
            return Err(format!("redraw event {} has no payload", name));
        }
        for (payload_index, payload) in values.iter().skip(1).enumerate() {
            let args = payload.as_array().ok_or_else(|| {
                format!(
                    "redraw event {} payload {} is not an array",
                    name, payload_index
                )
            })?;
            parsed_events.push(parse_redraw_payload(&name, args)?);
        }
    }

    Ok(parsed_events)
}

fn is_known_redraw_event(name: &str) -> bool {
    matches!(
        name,
        "default_colors_set"
            | "hl_attr_define"
            | "option_set"
            | "set_title"
            | "set_icon"
            | "mode_info_set"
            | "mode_change"
            | "ui_send"
            | "grid_resize"
            | "grid_line"
            | "grid_clear"
            | "grid_destroy"
            | "grid_cursor_goto"
            | "grid_scroll"
            | "win_pos"
            | "win_float_pos"
            | "win_viewport"
            | "win_viewport_margins"
            | "msg_set_pos"
            | "win_external_pos"
            | "win_hide"
            | "win_close"
    )
}

fn expect_arg_count(
    args: &[Value],
    minimum: usize,
    maximum: usize,
    event: &str,
) -> Result<(), String> {
    if (minimum..=maximum).contains(&args.len()) {
        return Ok(());
    }
    if minimum == maximum {
        return Err(format!(
            "redraw event {} expects {} arguments, got {}",
            event,
            minimum,
            args.len()
        ));
    }
    Err(format!(
        "redraw event {} expects {} to {} arguments, got {}",
        event,
        minimum,
        maximum,
        args.len()
    ))
}

fn parse_redraw_payload(name: &str, args: &[Value]) -> Result<NvimEvent, String> {
    match name {
        "default_colors_set" => {
            expect_arg_count(args, 3, 5, name)?;
            Ok(NvimEvent::DefaultColorsSet {
                foreground: parse_color(&args[0], "foreground")?,
                background: parse_color(&args[1], "background")?,
                special: parse_color(&args[2], "special")?,
            })
        }
        "hl_attr_define" => {
            expect_arg_count(args, 2, 4, name)?;
            parse_hl_attr_define(args)
        }
        "option_set" => {
            expect_arg_count(args, 2, 2, name)?;
            Ok(NvimEvent::OptionSet {
                name: string_value(&args[0])
                    .ok_or_else(|| "option_set has an invalid option name".to_owned())?,
                value: display_value(&args[1]),
            })
        }
        "set_title" => {
            expect_arg_count(args, 1, 1, name)?;
            Ok(NvimEvent::SetTitle {
                title: string_value(&args[0])
                    .ok_or_else(|| "set_title has an invalid title".to_owned())?,
            })
        }
        "set_icon" => {
            expect_arg_count(args, 1, 1, name)?;
            Ok(NvimEvent::SetIcon {
                icon: string_value(&args[0])
                    .ok_or_else(|| "set_icon has an invalid icon".to_owned())?,
            })
        }
        "mode_info_set" => {
            expect_arg_count(args, 2, 2, name)?;
            parse_mode_info_set(args)
        }
        "mode_change" => {
            expect_arg_count(args, 2, 2, name)?;
            Ok(NvimEvent::ModeChanged {
                mode: string_value(&args[0])
                    .ok_or_else(|| "mode_change has an invalid mode".to_owned())?,
                mode_idx: args[1]
                    .as_u64()
                    .ok_or_else(|| "mode_change has an invalid mode index".to_owned())?,
            })
        }
        "ui_send" => {
            expect_arg_count(args, 1, 1, name)?;
            Ok(NvimEvent::UiSend {
                data: string_value(&args[0])
                    .ok_or_else(|| "ui_send has invalid data".to_owned())?,
            })
        }
        "grid_resize" => {
            expect_arg_count(args, 3, 3, name)?;
            Ok(NvimEvent::GridResized {
                grid: parse_u64_value(&args[0], "grid_resize grid")?,
                width: parse_u32_value(&args[1], "grid_resize width")?,
                height: parse_u32_value(&args[2], "grid_resize height")?,
            })
        }
        "grid_line" => {
            expect_arg_count(args, 4, 5, name)?;
            parse_grid_line(args)
        }
        "grid_clear" => {
            expect_arg_count(args, 1, 1, name)?;
            Ok(NvimEvent::GridClear {
                grid: parse_u64_value(&args[0], "grid_clear grid")?,
            })
        }
        "grid_destroy" => {
            expect_arg_count(args, 1, 1, name)?;
            Ok(NvimEvent::GridDestroy {
                grid: parse_u64_value(&args[0], "grid_destroy grid")?,
            })
        }
        "grid_cursor_goto" => {
            expect_arg_count(args, 3, 3, name)?;
            Ok(NvimEvent::GridCursorGoto {
                grid: parse_u64_value(&args[0], "grid_cursor_goto grid")?,
                row: parse_u64_value(&args[1], "grid_cursor_goto row")?,
                col: parse_u64_value(&args[2], "grid_cursor_goto column")?,
            })
        }
        "grid_scroll" => {
            expect_arg_count(args, 7, 7, name)?;
            Ok(NvimEvent::GridScroll {
                grid: parse_u64_value(&args[0], "grid_scroll grid")?,
                top: parse_u64_value(&args[1], "grid_scroll top")?,
                bot: parse_u64_value(&args[2], "grid_scroll bottom")?,
                left: parse_u64_value(&args[3], "grid_scroll left")?,
                right: parse_u64_value(&args[4], "grid_scroll right")?,
                rows: parse_i64_value(&args[5], "grid_scroll rows")?,
                cols: parse_i64_value(&args[6], "grid_scroll columns")?,
            })
        }
        "win_pos" => {
            expect_arg_count(args, 6, 6, name)?;
            parse_win_pos(args)
        }
        "win_float_pos" => {
            expect_arg_count(args, 11, 11, name)?;
            parse_win_float_pos(args)
        }
        "win_viewport" => {
            expect_arg_count(args, 8, 8, name)?;
            parse_win_viewport(args)
        }
        "win_viewport_margins" => {
            expect_arg_count(args, 6, 6, name)?;
            parse_win_viewport_margins(args)
        }
        "msg_set_pos" => {
            expect_arg_count(args, 6, 6, name)?;
            parse_msg_set_pos(args)
        }
        "win_external_pos" => {
            expect_arg_count(args, 2, 2, name)?;
            Ok(NvimEvent::WinExternalPos {
                grid: parse_u64_value(&args[0], "win_external_pos grid")?,
                win: parse_window_id(&args[1])?,
            })
        }
        "win_hide" => {
            expect_arg_count(args, 1, 1, name)?;
            Ok(NvimEvent::WinHide {
                grid: parse_u64_value(&args[0], "win_hide grid")?,
            })
        }
        "win_close" => {
            expect_arg_count(args, 1, 1, name)?;
            Ok(NvimEvent::WinClose {
                grid: parse_u64_value(&args[0], "win_close grid")?,
            })
        }
        _ => Err(format!("unsupported redraw event {}", name)),
    }
}

fn parse_u32_value(value: &Value, name: &str) -> Result<u32, String> {
    let value = parse_u64_value(value, name)?;
    u32::try_from(value).map_err(|_| format!("{} is out of range", name))
}

fn send_event(events: &Sender<NvimEvent>, event: NvimEvent) -> Result<(), String> {
    events
        .send_blocking(event)
        .map_err(|_| "GPUI stopped receiving Neovim events".to_owned())
}

fn expect_rpc_message_len(
    values: &[Value],
    expected: usize,
    message_type: &str,
) -> Result<(), String> {
    if values.len() == expected {
        Ok(())
    } else {
        Err(format!(
            "{} expects {} fields, got {}",
            message_type,
            expected,
            values.len()
        ))
    }
}
