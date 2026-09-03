use async_channel::Sender;
use rmpv::Value;
use std::io::{BufReader, Read};
use std::sync::mpsc::SyncSender;

use super::protocol::*;
use super::transport::{read_message, write_shared_message, RpcReader, SharedWriter};
use super::{parse_protocol_info, NvimEvent, NvimProtocolInfo, NvimTheme};

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

    request(
        &writer,
        &mut reader,
        request_id,
        "nvim_set_client_info",
        client_info_params(),
        events,
    )?;
    request_id += 1;

    request(
        &writer,
        &mut reader,
        request_id,
        "nvim_ui_attach",
        ui_attach_params_for(width, height, &protocol.capabilities),
        events,
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
        dispatch_message(&writer, message, events)?;
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

    let Some(redraw_events) = values.get(2).and_then(Value::as_array) else {
        return false;
    };
    let mut flushed = false;

    for redraw_event in redraw_events {
        let Some(values) = redraw_event.as_array() else {
            continue;
        };
        let Some(name) = values.first().and_then(string_value) else {
            continue;
        };

        if name == "flush" {
            flushed = true;
            continue;
        }

        for payload in values.iter().skip(1) {
            let Some(args) = payload.as_array() else {
                continue;
            };
            match name.as_str() {
                "default_colors_set" if args.len() >= 3 => {
                    theme.default_foreground = parse_color(&args[0], "foreground").ok().flatten();
                    theme.default_background = parse_color(&args[1], "background").ok().flatten();
                }
                "hl_attr_define" if args.len() >= 2 => {
                    let Ok(NvimEvent::HlAttrDefine { attrs, .. }) = parse_hl_attr_define(args)
                    else {
                        continue;
                    };
                    match attrs.ui_name.as_deref() {
                        Some("Normal") => {
                            theme.normal_foreground = attrs.foreground;
                            theme.normal_background = attrs.background;
                        }
                        Some("NormalFloat") => {
                            theme.normal_float_background = attrs.background;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
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
                let response_id = values.get(1).and_then(Value::as_u64);
                if response_id != Some(id) {
                    return Err(format!("unexpected RPC response id: {response_id:?}"));
                }
                let error = values.get(2).unwrap_or(&Value::Nil);
                if !matches!(error, Value::Nil) {
                    return Err(format!("RPC request {method} failed: {error:?}"));
                }
                return values
                    .get(3)
                    .cloned()
                    .ok_or_else(|| "RPC response has no result".to_owned());
            }
            Some(2) => {
                let method = values
                    .get(1)
                    .and_then(string_value)
                    .ok_or_else(|| "RPC notification has no method".to_owned())?;
                let params = values.get(2).unwrap_or(&Value::Nil);
                handle_notification(&method, params, events)?;
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
) -> Result<(), String> {
    let Some(values) = message.as_array() else {
        return Err("RPC message is not an array".to_owned());
    };

    match values.first().and_then(Value::as_u64) {
        Some(1) => {
            let error = values.get(2).unwrap_or(&Value::Nil);
            if !matches!(error, Value::Nil) {
                send_event(
                    events,
                    NvimEvent::Error(format!("Neovim RPC request failed: {error:?}")),
                )?;
            }
            Ok(())
        }
        Some(2) => {
            let method = values
                .get(1)
                .and_then(string_value)
                .ok_or_else(|| "RPC notification has no method".to_owned())?;
            let params = values.get(2).unwrap_or(&Value::Nil);
            handle_notification(&method, params, events)
        }
        Some(0) => {
            let id = values
                .get(1)
                .and_then(Value::as_u64)
                .ok_or_else(|| "RPC request has no id".to_owned())?;
            write_shared_message(
                writer,
                &Value::Array(vec![
                    Value::from(1),
                    Value::from(id),
                    Value::from("nvim-gpui does not accept RPC requests yet"),
                    Value::Nil,
                ]),
            )
        }
        Some(tag) => Err(format!("unexpected RPC message type: {tag}")),
        None => Err("RPC message has no type".to_owned()),
    }
}

pub(crate) fn handle_notification(
    method: &str,
    params: &Value,
    events: &Sender<NvimEvent>,
) -> Result<(), String> {
    if method != "redraw" {
        return Ok(());
    }

    let redraw_events = params
        .as_array()
        .ok_or_else(|| "redraw notification params are not an array".to_owned())?;

    for redraw_event in redraw_events {
        let Some(values) = redraw_event.as_array() else {
            continue;
        };
        let Some(name) = values.first().and_then(string_value) else {
            continue;
        };

        for payload in values.iter().skip(1) {
            let Some(args) = payload.as_array() else {
                continue;
            };
            match name.as_str() {
                "default_colors_set" if args.len() >= 3 => {
                    send_event(
                        events,
                        NvimEvent::DefaultColorsSet {
                            foreground: parse_color(&args[0], "foreground")?,
                            background: parse_color(&args[1], "background")?,
                            special: parse_color(&args[2], "special")?,
                        },
                    )?;
                }
                "hl_attr_define" if args.len() >= 2 => {
                    send_event(events, parse_hl_attr_define(args)?)?;
                }
                "option_set" if args.len() >= 2 => {
                    send_event(
                        events,
                        NvimEvent::OptionSet {
                            name: string_value(&args[0]).unwrap_or_default(),
                            value: display_value(&args[1]),
                        },
                    )?;
                }
                "set_title" if !args.is_empty() => {
                    send_event(
                        events,
                        NvimEvent::SetTitle {
                            title: string_value(&args[0]).unwrap_or_default(),
                        },
                    )?;
                }
                "set_icon" if !args.is_empty() => {
                    send_event(
                        events,
                        NvimEvent::SetIcon {
                            icon: string_value(&args[0]).unwrap_or_default(),
                        },
                    )?;
                }
                "mode_info_set" if args.len() >= 2 => {
                    send_event(events, parse_mode_info_set(args)?)?;
                }
                "mode_change" if args.len() >= 2 => {
                    send_event(
                        events,
                        NvimEvent::ModeChanged {
                            mode: string_value(&args[0]).unwrap_or_default(),
                            mode_idx: args[1].as_u64().unwrap_or_default(),
                        },
                    )?;
                }
                "ui_send" if !args.is_empty() => {
                    if let Some(data) = string_value(&args[0]) {
                        send_event(events, NvimEvent::UiSend { data })?;
                    }
                }
                "grid_resize" if args.len() >= 3 => {
                    send_event(
                        events,
                        NvimEvent::GridResized {
                            grid: args[0].as_u64().unwrap_or_default(),
                            width: args[1].as_u64().unwrap_or_default() as u32,
                            height: args[2].as_u64().unwrap_or_default() as u32,
                        },
                    )?;
                }
                "grid_line" if args.len() >= 4 => {
                    send_event(events, parse_grid_line(args)?)?;
                }
                "grid_clear" if !args.is_empty() => {
                    send_event(
                        events,
                        NvimEvent::GridClear {
                            grid: args[0].as_u64().unwrap_or_default(),
                        },
                    )?;
                }
                "grid_destroy" if !args.is_empty() => {
                    send_event(
                        events,
                        NvimEvent::GridDestroy {
                            grid: args[0].as_u64().unwrap_or_default(),
                        },
                    )?;
                }
                "grid_cursor_goto" if args.len() >= 3 => {
                    send_event(
                        events,
                        NvimEvent::GridCursorGoto {
                            grid: args[0].as_u64().unwrap_or_default(),
                            row: args[1].as_u64().unwrap_or_default(),
                            col: args[2].as_u64().unwrap_or_default(),
                        },
                    )?;
                }
                "grid_scroll" if args.len() >= 7 => {
                    send_event(
                        events,
                        NvimEvent::GridScroll {
                            grid: args[0].as_u64().unwrap_or_default(),
                            top: args[1].as_u64().unwrap_or_default(),
                            bot: args[2].as_u64().unwrap_or_default(),
                            left: args[3].as_u64().unwrap_or_default(),
                            right: args[4].as_u64().unwrap_or_default(),
                            rows: args[5].as_i64().unwrap_or_default(),
                            cols: args[6].as_i64().unwrap_or_default(),
                        },
                    )?;
                }
                "win_pos" if args.len() >= 6 => {
                    send_event(events, parse_win_pos(args)?)?;
                }
                "win_float_pos" if args.len() >= 11 => {
                    send_event(events, parse_win_float_pos(args)?)?;
                }
                "win_viewport" if args.len() >= 8 => {
                    send_event(events, parse_win_viewport(args)?)?;
                }
                "win_viewport_margins" if args.len() >= 6 => {
                    send_event(events, parse_win_viewport_margins(args)?)?;
                }
                "msg_set_pos" if args.len() >= 6 => {
                    send_event(events, parse_msg_set_pos(args)?)?;
                }
                "win_external_pos" if args.len() >= 2 => {
                    send_event(
                        events,
                        NvimEvent::WinExternalPos {
                            grid: args[0].as_u64().ok_or_else(|| {
                                "win_external_pos has an invalid grid id".to_owned()
                            })?,
                            win: parse_window_id(&args[1])?,
                        },
                    )?;
                }
                "win_hide" if !args.is_empty() => {
                    send_event(
                        events,
                        NvimEvent::WinHide {
                            grid: args[0]
                                .as_u64()
                                .ok_or_else(|| "win_hide has an invalid grid id".to_owned())?,
                        },
                    )?;
                }
                "win_close" if !args.is_empty() => {
                    send_event(
                        events,
                        NvimEvent::WinClose {
                            grid: args[0]
                                .as_u64()
                                .ok_or_else(|| "win_close has an invalid grid id".to_owned())?,
                        },
                    )?;
                }
                _ => {}
            }
        }

        if name == "flush" {
            send_event(events, NvimEvent::Flush)?;
        }
    }

    Ok(())
}

fn send_event(events: &Sender<NvimEvent>, event: NvimEvent) -> Result<(), String> {
    events
        .send_blocking(event)
        .map_err(|_| "GPUI stopped receiving Neovim events".to_owned())
}
