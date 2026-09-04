use super::environment::{
    apply_project_nvim_environment, mark_embedded_gui_environment, parse_environment,
    project_nvim_environment_is_active_at, remove_project_nvim_environment, NVIM_GPUI_ENV,
    NVIM_GPUI_ENV_VALUE,
};
use super::protocol::{
    mouse_event_notification_frame, resize_request_frame, term_event_notification_frame,
    ui_attach_params, ui_attach_params_for,
};
use super::session::{handle_notification, observe_startup_theme};
use super::transport::{read_message, write_message};
use super::version::parse_protocol_info;
use super::{
    disconnect_reason, DisconnectReason, NvimCapabilities, NvimEvent, NvimProcess, NvimTheme,
    NVIM_EXITED,
};
use crate::clipboard::{CLIPBOARD_GET_METHOD, CLIPBOARD_SET_METHOD};
use async_channel::unbounded;
use rmpv::Value;
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    io::Cursor,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::channel,
        Arc,
    },
};

#[test]
fn request_frame_uses_msgpack_rpc_shape() {
    let mut bytes = Vec::new();
    write_message(
        &mut bytes,
        &Value::Array(vec![
            Value::from(0),
            Value::from(7),
            Value::from("nvim_get_api_info"),
            Value::Array(Vec::new()),
        ]),
    )
    .expect("request should encode");

    let decoded = rmpv::decode::read_value(&mut Cursor::new(bytes)).expect("request decodes");
    assert_eq!(decoded[0].as_u64(), Some(0));
    assert_eq!(decoded[1].as_u64(), Some(7));
    assert_eq!(decoded[2].as_str(), Some("nvim_get_api_info"));
}

#[test]
fn api_metadata_builds_version_and_ui_capabilities() {
    let api_info = Value::Array(vec![
        Value::from(1),
        Value::Map(vec![
            (
                Value::from("version"),
                Value::Map(vec![
                    (Value::from("major"), Value::from(0)),
                    (Value::from("minor"), Value::from(10)),
                    (Value::from("patch"), Value::from(4)),
                    (Value::from("api_level"), Value::from(12)),
                    (Value::from("api_compatible"), Value::from(0)),
                    (Value::from("api_prerelease"), Value::Boolean(false)),
                ]),
            ),
            (
                Value::from("ui_options"),
                Value::Array(vec![Value::from("rgb"), Value::from("ext_linegrid")]),
            ),
            (
                Value::from("ui_events"),
                Value::Array(vec![
                    Value::Map(vec![(Value::from("name"), Value::from("grid_line"))]),
                    Value::from("flush"),
                ]),
            ),
        ]),
    ]);

    let protocol = parse_protocol_info(&api_info).expect("API metadata should decode");

    assert_eq!(protocol.channel_id, 1);
    assert_eq!(protocol.version.major, 0);
    assert_eq!(protocol.version.minor, 10);
    assert_eq!(protocol.version.patch, 4);
    assert_eq!(protocol.version.api_level, 12);
    assert_eq!(protocol.version.api_compatible, 0);
    assert!(protocol.version.supports_api(12));
    assert!(protocol.capabilities.supports_ui_option("rgb"));
    assert!(!protocol.capabilities.supports_ui_option("ext_hlstate"));
    assert!(protocol.capabilities.supports_ui_event("grid_line"));
    assert!(protocol.capabilities.supports_ui_event("flush"));
}

#[test]
fn embedded_nvim_reports_protocol_metadata_before_ui_events() {
    let process = NvimProcess::spawn(80, 24, std::iter::empty::<OsString>())
        .expect("embedded Neovim should start");
    let protocol = process
        .protocol()
        .expect("protocol metadata should be available after startup");

    assert!(protocol.version.api_level > 0);
    assert!(!protocol.capabilities.ui_options.is_empty());
    assert!(!protocol.capabilities.ui_events.is_empty());
    assert!(protocol.capabilities.ui_options.contains("ext_linegrid"));
    assert!(protocol.capabilities.ui_events.contains("flush"));

    let events = process.events();
    assert!(matches!(
        events.try_recv().expect("ApiReady should be queued"),
        NvimEvent::ApiReady { .. }
    ));
}

#[test]
fn embedded_nvim_reports_and_accepts_mouse_input() {
    let process = NvimProcess::spawn(80, 24, std::iter::empty::<OsString>())
        .expect("embedded Neovim should start");
    let events = process.events();

    let mut saw_mouse_option = false;
    for _ in 0..200 {
        while let Ok(event) = events.try_recv() {
            if matches!(
                event,
                NvimEvent::OptionSet { ref name, ref value }
                    if name == "mouse" && value == "nvi"
            ) {
                saw_mouse_option = true;
            }
        }
        if saw_mouse_option {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(saw_mouse_option, "startup mouse option should be reported");
    while events.try_recv().is_ok() {}

    process
        .send_input(":call setline(1, 'abcdef')\n")
        .expect("Neovim command should queue");
    process
        .send_mouse("left", "press", "", 0, 0, 4)
        .expect("Neovim mouse press should queue");
    process
        .send_mouse("left", "release", "", 0, 0, 4)
        .expect("Neovim mouse release should queue");

    let mut saw_cursor_move = false;
    for _ in 0..200 {
        while let Ok(event) = events.try_recv() {
            if matches!(event, NvimEvent::GridCursorGoto { row: 0, col: 4, .. }) {
                saw_cursor_move = true;
            }
        }
        if saw_cursor_move {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    assert!(
        saw_cursor_move,
        "nvim_input_mouse should move the buffer cursor"
    );
}

#[test]
fn embedded_nvim_rpc_request_round_trips_a_response() {
    let process = NvimProcess::spawn(80, 24, std::iter::empty::<OsString>())
        .expect("embedded Neovim should start");
    let response = process
        .request("nvim_get_mode", Value::Array(Vec::new()))
        .expect("RPC request should queue");

    let mut result = None;
    for _ in 0..200 {
        if let Ok(value) = response.try_recv() {
            result = Some(value);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let value = result
        .expect("RPC response should arrive")
        .expect("nvim_get_mode should succeed");
    assert!(
        value.as_map().is_some(),
        "nvim_get_mode should return a map"
    );
}

#[test]
fn embedded_nvim_replies_to_a_nvim_rpc_request() {
    let process = NvimProcess::spawn(80, 24, std::iter::empty::<OsString>())
        .expect("embedded Neovim should start");
    let called = Arc::new(AtomicBool::new(false));
    let called_by_handler = Arc::clone(&called);
    process
        .register_request_handler("nvim_gpui_test", move |params| {
            called_by_handler.store(true, Ordering::Release);
            Ok(params.clone())
        })
        .expect("request handler should register");

    process
        .send_input(
            ":lua for _, chan in ipairs(vim.api.nvim_list_chans()) do if chan.mode == 'rpc' then vim.rpcrequest(chan.id, 'nvim_gpui_test', 7); break end end\n",
        )
        .expect("Neovim input should queue");

    for _ in 0..200 {
        if called.load(Ordering::Acquire) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        called.load(Ordering::Acquire),
        "Neovim request should reach the registered GUI handler"
    );

    let response = process
        .request("nvim_get_mode", Value::Array(Vec::new()))
        .expect("RPC request should queue after the incoming request");
    assert!(
        response
            .recv_blocking()
            .expect("response channel should stay open")
            .is_ok(),
        "the session should remain usable after replying to a Neovim request"
    );
}

#[test]
fn gui_clipboard_provider_forwards_remote_yanks_to_the_client() {
    let process = NvimProcess::spawn(80, 24, std::iter::empty::<OsString>())
        .expect("embedded Neovim should start");
    let (set_tx, set_rx) = channel();
    process
        .register_request_handler(CLIPBOARD_GET_METHOD, |_| {
            Ok(Value::Array(vec![
                Value::Array(vec![Value::from("")]),
                Value::from("v"),
            ]))
        })
        .expect("clipboard get handler should register");
    process
        .register_request_handler(CLIPBOARD_SET_METHOD, move |params| {
            set_tx
                .send(params.clone())
                .map_err(|_| "clipboard test receiver was dropped".to_owned())?;
            Ok(Value::Nil)
        })
        .expect("clipboard set handler should register");

    let setup = process
        .request(
            "nvim_exec_lua",
            Value::Array(vec![
                Value::from(crate::clipboard::remote_provider_lua(
                    process
                        .protocol()
                        .expect("protocol should be available")
                        .channel_id,
                )),
                Value::Array(Vec::new()),
            ]),
        )
        .expect("clipboard provider setup should queue");
    let setup_result = setup
        .recv_blocking()
        .expect("clipboard provider setup response should arrive");
    assert!(
        setup_result.is_ok(),
        "clipboard setup failed: {setup_result:?}"
    );

    process
        .send_input(":call setline(1, ['one', 'two']) | call deletebufline(bufnr(), 3) | normal! gg\"+2yy\n")
        .expect("yank command should queue");
    let params = set_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("remote yank should reach the GUI clipboard handler");
    let params = params
        .as_array()
        .expect("clipboard set params should be an array");
    assert_eq!(params[0].as_str(), Some("+"));
    assert_eq!(
        params[1],
        Value::Array(vec![
            Value::from("one"),
            Value::from("two"),
            Value::from(""),
        ])
    );
    assert_eq!(params[2].as_str(), Some("V"));
}

#[test]
fn embedded_nvim_accepts_multiline_paste() {
    let process = NvimProcess::spawn(80, 24, std::iter::empty::<OsString>())
        .expect("embedded Neovim should start");
    process
        .send_input("i")
        .expect("insert mode input should queue");

    let paste = process
        .send_paste("one\ntwo")
        .expect("nvim_paste request should queue")
        .recv_blocking()
        .expect("nvim_paste response should arrive")
        .expect("nvim_paste should succeed");
    assert_eq!(paste, Value::Boolean(true));

    let lines = process
        .request(
            "nvim_buf_get_lines",
            Value::Array(vec![
                Value::from(0),
                Value::from(0),
                Value::from(-1_i64),
                Value::Boolean(false),
            ]),
        )
        .expect("buffer read request should queue")
        .recv_blocking()
        .expect("buffer read response should arrive")
        .expect("buffer read should succeed");
    assert_eq!(
        lines,
        Value::Array(vec![Value::from("one"), Value::from("two")])
    );
}

#[test]
fn embedded_nvim_can_be_reconnected_after_a_clean_exit() {
    let process = NvimProcess::spawn(80, 24, std::iter::empty::<OsString>())
        .expect("embedded Neovim should start");
    let events = process.events();
    process
        .send_input(":qa!\n")
        .expect("Neovim quit command should queue");

    let mut disconnected = None;
    for _ in 0..400 {
        while let Ok(event) = events.try_recv() {
            if let NvimEvent::Disconnected { reason } = event {
                disconnected = Some(reason);
            }
        }
        if disconnected.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(disconnected, Some(DisconnectReason::CleanExit));

    let replacement = process
        .reconnect(80, 24)
        .expect("the embedded command should be restartable");
    assert!(
        replacement.version().is_some(),
        "the replacement should complete the RPC handshake"
    );
}

#[test]
fn embedded_nvim_forwards_nvim_ui_send_event() {
    let process = NvimProcess::spawn(80, 24, std::iter::empty::<OsString>())
        .expect("embedded Neovim should start");
    let events = process.events();
    while events.try_recv().is_ok() {}

    process
        .send_input(
            ":lua vim.api.nvim_ui_send(string.char(27)..'_Ga=T,f=100,t=d,i=42,m=0;aGVsbG8='..string.char(27)..'\\\\')\n",
        )
        .expect("Neovim input should queue");

    let mut found = false;
    for _ in 0..200 {
        while let Ok(event) = events.try_recv() {
            if matches!(event, NvimEvent::UiSend { .. }) {
                found = true;
            }
        }
        if found {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    assert!(found, "nvim_ui_send should produce a UiSend redraw event");
}

#[test]
fn ui_attach_marks_ui_send_clients_as_tty_interactive() {
    let capabilities = NvimCapabilities {
        ui_options: ["rgb", "ext_linegrid", "ext_multigrid"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        ui_events: ["ui_send"].into_iter().map(str::to_owned).collect(),
    };
    let attach_params = ui_attach_params_for(80, 24, &capabilities);
    let options = attach_params[2]
        .as_map()
        .expect("ui options should be a map");

    for option in ["stdin_tty", "stdout_tty"] {
        assert_eq!(
            options
                .iter()
                .find(|(key, _)| key.as_str() == Some(option))
                .and_then(|(_, value)| value.as_bool()),
            Some(true),
            "{option} should be enabled for a ui_send client"
        );
    }
}

#[test]
fn resize_request_frame_uses_the_nvim_ui_resize_method() {
    let frame = resize_request_frame(42, 120, 40);

    assert_eq!(frame[0].as_u64(), Some(0));
    assert_eq!(frame[1].as_u64(), Some(42));
    assert_eq!(frame[2].as_str(), Some("nvim_ui_try_resize"));
    assert_eq!(frame[3][0].as_u64(), Some(120));
    assert_eq!(frame[3][1].as_u64(), Some(40));
}

#[test]
fn mouse_event_notification_uses_nvim_input_mouse() {
    let frame = mouse_event_notification_frame(
        "left".to_owned(),
        "press".to_owned(),
        "CS".to_owned(),
        0,
        12,
        34,
    );

    assert_eq!(frame[0].as_u64(), Some(2));
    assert_eq!(frame[1].as_str(), Some("nvim_input_mouse"));
    assert_eq!(frame[2][0].as_str(), Some("left"));
    assert_eq!(frame[2][1].as_str(), Some("press"));
    assert_eq!(frame[2][2].as_str(), Some("CS"));
    assert_eq!(frame[2][3].as_u64(), Some(0));
    assert_eq!(frame[2][4].as_u64(), Some(12));
    assert_eq!(frame[2][5].as_u64(), Some(34));
}

#[test]
fn an_eof_is_classified_as_a_normal_nvim_exit() {
    let mut reader = Cursor::new(Vec::<u8>::new());

    assert_eq!(read_message(&mut reader), Err(NVIM_EXITED.to_owned()));
}

#[test]
fn disconnect_reason_distinguishes_shutdown_transport_and_protocol_failures() {
    assert_eq!(
        disconnect_reason(&Ok(()), true, false, Some(true)),
        DisconnectReason::Requested
    );
    assert_eq!(
        disconnect_reason(&Err(NVIM_EXITED.to_owned()), false, false, Some(true)),
        DisconnectReason::CleanExit
    );
    assert_eq!(
        disconnect_reason(&Err(NVIM_EXITED.to_owned()), false, true, None),
        DisconnectReason::TransportClosed
    );
    assert_eq!(
        disconnect_reason(&Err(NVIM_EXITED.to_owned()), false, false, Some(false)),
        DisconnectReason::UnexpectedExit
    );
    assert_eq!(
        disconnect_reason(&Err("malformed redraw".to_owned()), false, false, None),
        DisconnectReason::ProtocolError("malformed redraw".to_owned())
    );
}

#[test]
fn startup_environment_parser_keeps_nul_delimited_values() {
    let environment = parse_environment(b"PATH=/nix/bin\0NVIM_APPNAME=nvim-gpui\0");

    assert_eq!(
        environment.get(std::ffi::OsStr::new("PATH")),
        Some(&std::ffi::OsString::from("/nix/bin"))
    );
    assert_eq!(
        environment.get(std::ffi::OsStr::new("NVIM_APPNAME")),
        Some(&std::ffi::OsString::from("nvim-gpui"))
    );
}

#[test]
fn project_nvim_paths_are_applied_only_to_the_child_environment() {
    let mut environment = HashMap::from([
        (
            OsString::from("XDG_CONFIG_HOME"),
            OsString::from("/Users/me/.config"),
        ),
        (
            OsString::from("XDG_DATA_HOME"),
            OsString::from("/Users/me/.local/share"),
        ),
        (
            OsString::from("NVIM_GPUI_CONFIG_DIR"),
            OsString::from("/repo/config"),
        ),
        (
            OsString::from("NVIM_GPUI_CACHE_DIR"),
            OsString::from("/repo/.cache"),
        ),
    ]);

    apply_project_nvim_environment(&mut environment);

    assert_eq!(
        environment.get(OsStr::new("XDG_CONFIG_HOME")),
        Some(&OsString::from("/repo/config"))
    );
    assert_eq!(
        environment.get(OsStr::new("XDG_DATA_HOME")),
        Some(&OsString::from("/repo/.cache/nvim-data"))
    );
    assert_eq!(
        environment.get(OsStr::new("XDG_STATE_HOME")),
        Some(&OsString::from("/repo/.cache/nvim-state"))
    );
    assert_eq!(
        environment.get(OsStr::new("XDG_CACHE_HOME")),
        Some(&OsString::from("/repo/.cache/nvim-cache"))
    );
}

#[test]
fn embedded_gui_marker_is_available_to_startup_configuration() {
    let mut environment = HashMap::new();

    mark_embedded_gui_environment(&mut environment);

    assert_eq!(
        environment.get(OsStr::new(NVIM_GPUI_ENV)),
        Some(&OsString::from(NVIM_GPUI_ENV_VALUE))
    );
}

#[test]
fn project_nvim_environment_is_scoped_to_its_repository() {
    let environment = HashMap::from([(
        OsString::from("NVIM_GPUI_CONFIG_DIR"),
        OsString::from("/repo/config"),
    )]);

    assert!(project_nvim_environment_is_active_at(
        &environment,
        Path::new("/repo")
    ));
    assert!(project_nvim_environment_is_active_at(
        &environment,
        Path::new("/repo/src")
    ));
    assert!(!project_nvim_environment_is_active_at(
        &environment,
        Path::new("/tmp")
    ));
}

#[test]
fn stale_project_nvim_variables_are_removed_outside_the_repository() {
    let mut environment = HashMap::from([
        (OsString::from("NVIM_APPNAME"), OsString::from("nvim-gpui")),
        (
            OsString::from("NVIM_GPUI_CONFIG_DIR"),
            OsString::from("/repo/config"),
        ),
        (
            OsString::from("NVIM_GPUI_NVIM"),
            OsString::from("/repo/nvim"),
        ),
        (OsString::from("DIRENV_IN_ENVRC"), OsString::from("1")),
    ]);

    remove_project_nvim_environment(&mut environment);

    assert!(environment.is_empty());
}

#[test]
fn redraw_option_set_becomes_a_typed_event() {
    let (sender, receiver) = unbounded();
    let params = Value::Array(vec![Value::Array(vec![
        Value::from("option_set"),
        Value::Array(vec![Value::from("guifont"), Value::from("Monaco:h12")]),
    ])]);

    handle_notification("redraw", &params, &sender).expect("redraw should decode");

    assert_eq!(
        receiver.try_recv().expect("event should be available"),
        NvimEvent::OptionSet {
            name: "guifont".to_owned(),
            value: "Monaco:h12".to_owned(),
        }
    );
}

#[test]
fn malformed_redraw_is_rejected_before_any_event_is_emitted() {
    let (sender, receiver) = unbounded();
    let params = Value::Array(vec![
        Value::Array(vec![Value::from("mouse_on")]),
        Value::Array(vec![
            Value::from("grid_resize"),
            Value::Array(vec![
                Value::from(1),
                Value::from(80),
                Value::from("invalid height"),
            ]),
        ]),
    ]);

    let error = handle_notification("redraw", &params, &sender)
        .expect_err("malformed redraw should be rejected");

    assert!(error.contains("grid_resize"));
    assert!(receiver.try_recv().is_err());
}

#[test]
fn malformed_known_redraw_payload_is_not_silently_ignored() {
    let (sender, receiver) = unbounded();
    let params = Value::Array(vec![Value::Array(vec![Value::from("grid_clear")])]);

    let error = handle_notification("redraw", &params, &sender)
        .expect_err("missing redraw payload should be rejected");

    assert!(error.contains("grid_clear"));
    assert!(receiver.try_recv().is_err());
}

#[test]
fn unknown_redraw_events_are_skipped_without_blocking_flush() {
    let (sender, receiver) = unbounded();
    let params = Value::Array(vec![
        Value::Array(vec![
            Value::from("future_redraw_event"),
            Value::Boolean(true),
        ]),
        Value::Array(vec![Value::from("flush")]),
    ]);

    handle_notification("redraw", &params, &sender).expect("unknown redraw should be skipped");

    assert_eq!(
        receiver.try_recv().expect("flush should be available"),
        NvimEvent::Flush
    );
    assert!(receiver.try_recv().is_err());
}

#[test]
fn malformed_optional_grid_line_flag_is_rejected() {
    let (sender, receiver) = unbounded();
    let params = Value::Array(vec![Value::Array(vec![
        Value::from("grid_line"),
        Value::Array(vec![
            Value::from(1),
            Value::from(0),
            Value::from(0),
            Value::Array(Vec::new()),
            Value::from("invalid wrap flag"),
        ]),
    ])]);

    let error = handle_notification("redraw", &params, &sender)
        .expect_err("invalid grid_line flag should be rejected");

    assert!(error.contains("wraps_to_next"));
    assert!(receiver.try_recv().is_err());
}

#[test]
fn redraw_mouse_state_becomes_a_typed_event() {
    let (sender, receiver) = unbounded();
    let params = Value::Array(vec![
        Value::Array(vec![Value::from("mouse_on")]),
        Value::Array(vec![Value::from("mouse_off")]),
    ]);

    handle_notification("redraw", &params, &sender).expect("redraw should decode");

    assert_eq!(
        receiver.try_recv().expect("mouse_on should be available"),
        NvimEvent::MouseEnabled(true)
    );
    assert_eq!(
        receiver.try_recv().expect("mouse_off should be available"),
        NvimEvent::MouseEnabled(false)
    );
}

#[test]
fn redraw_ui_send_becomes_a_typed_event() {
    let (sender, receiver) = unbounded();
    let params = Value::Array(vec![Value::Array(vec![
        Value::from("ui_send"),
        Value::Array(vec![Value::from("\x1b[>q")]),
    ])]);

    handle_notification("redraw", &params, &sender).expect("redraw should decode");

    assert_eq!(
        receiver.try_recv().expect("event should be available"),
        NvimEvent::UiSend {
            data: "\x1b[>q".to_owned(),
        }
    );
}

#[test]
fn redraw_set_title_becomes_a_typed_event() {
    let (sender, receiver) = unbounded();
    let params = Value::Array(vec![Value::Array(vec![
        Value::from("set_title"),
        Value::Array(vec![Value::from("nvim-gpui — README.md")]),
    ])]);

    handle_notification("redraw", &params, &sender).expect("redraw should decode");

    assert_eq!(
        receiver.try_recv().expect("event should be available"),
        NvimEvent::SetTitle {
            title: "nvim-gpui — README.md".to_owned(),
        }
    );
}

#[test]
fn redraw_hl_attr_define_decodes_rgb_attributes_and_styles() {
    let (sender, receiver) = unbounded();
    let params = Value::Array(vec![Value::Array(vec![
        Value::from("hl_attr_define"),
        Value::Array(vec![
            Value::from(12),
            Value::Map(vec![
                (Value::from("foreground"), Value::from(0xffcc00u64)),
                (Value::from("background"), Value::from(0x112233u64)),
                (Value::from("special"), Value::from(0x00ff00u64)),
                (Value::from("bold"), Value::Boolean(true)),
                (Value::from("undercurl"), Value::Boolean(true)),
                (Value::from("blend"), Value::from(25u64)),
                (Value::from("altfont"), Value::from(3u64)),
                (Value::from("url"), Value::from("https://neovim.io")),
            ]),
            Value::Map(Vec::new()),
            Value::Array(Vec::new()),
        ]),
    ])]);

    handle_notification("redraw", &params, &sender).expect("redraw should decode");

    assert_eq!(
        receiver.try_recv().expect("event should be available"),
        NvimEvent::HlAttrDefine {
            id: crate::grid::HighlightId(12),
            attrs: crate::grid::HighlightAttrs {
                foreground: Some(0xffcc00),
                background: Some(0x112233),
                special: Some(0x00ff00),
                bold: true,
                undercurl: true,
                blend: Some(25),
                altfont: Some(3),
                url: Some("https://neovim.io".to_owned()),
                ..Default::default()
            },
        }
    );
}

#[test]
fn redraw_hl_attr_define_keeps_semantic_ui_name() {
    let (sender, receiver) = unbounded();
    let params = Value::Array(vec![Value::Array(vec![
        Value::from("hl_attr_define"),
        Value::Array(vec![
            Value::from(12),
            Value::Map(vec![(Value::from("background"), Value::from(0x001419u64))]),
            Value::Map(Vec::new()),
            Value::Array(vec![Value::Map(vec![
                (Value::from("kind"), Value::from("ui")),
                (Value::from("ui_name"), Value::from("NormalFloat")),
                (Value::from("hi_name"), Value::from("NormalFloat")),
            ])]),
        ]),
    ])]);

    handle_notification("redraw", &params, &sender).expect("redraw should decode");

    assert_eq!(
        receiver.try_recv().expect("event should be available"),
        NvimEvent::HlAttrDefine {
            id: crate::grid::HighlightId(12),
            attrs: crate::grid::HighlightAttrs {
                background: Some(0x001419),
                ui_name: Some("NormalFloat".to_owned()),
                ..Default::default()
            },
        }
    );
}

#[test]
fn redraw_set_icon_becomes_a_typed_event() {
    let (sender, receiver) = unbounded();
    let params = Value::Array(vec![Value::Array(vec![
        Value::from("set_icon"),
        Value::Array(vec![Value::from("nvim-gpui")]),
    ])]);

    handle_notification("redraw", &params, &sender).expect("redraw should decode");

    assert_eq!(
        receiver.try_recv().expect("event should be available"),
        NvimEvent::SetIcon {
            icon: "nvim-gpui".to_owned(),
        }
    );
}

#[test]
fn redraw_grid_destroy_becomes_a_typed_event() {
    let (sender, receiver) = unbounded();
    let params = Value::Array(vec![Value::Array(vec![
        Value::from("grid_destroy"),
        Value::Array(vec![Value::from(1)]),
    ])]);

    handle_notification("redraw", &params, &sender).expect("redraw should decode");

    assert_eq!(
        receiver.try_recv().expect("event should be available"),
        NvimEvent::GridDestroy { grid: 1 }
    );
}

#[test]
fn redraw_mode_info_set_decodes_cursor_shapes_blink_and_attributes() {
    let (sender, receiver) = unbounded();
    let mode = |shape: &str, percentage: u64, attr_id: u64| {
        Value::Map(vec![
            (Value::from("cursor_shape"), Value::from(shape)),
            (Value::from("cell_percentage"), Value::from(percentage)),
            (Value::from("blinkwait"), Value::from(700u64)),
            (Value::from("blinkon"), Value::from(400u64)),
            (Value::from("blinkoff"), Value::from(250u64)),
            (Value::from("attr_id"), Value::from(attr_id)),
            (Value::from("attr_id_lm"), Value::from(0u64)),
        ])
    };
    let params = Value::Array(vec![Value::Array(vec![
        Value::from("mode_info_set"),
        Value::Array(vec![
            Value::Boolean(true),
            Value::Array(vec![
                mode("block", 100, 0),
                mode("horizontal", 25, 8),
                mode("vertical", 20, 9),
            ]),
        ]),
    ])]);

    handle_notification("redraw", &params, &sender).expect("redraw should decode");

    assert_eq!(
        receiver.try_recv().expect("event should be available"),
        NvimEvent::ModeInfoSet {
            cursor_style_enabled: true,
            modes: vec![
                crate::grid::CursorModeInfo {
                    shape: crate::grid::CursorShape::Block,
                    cell_percentage: 100,
                    blink_wait: 700,
                    blink_on: 400,
                    blink_off: 250,
                    attr_id: Some(crate::grid::HighlightId(0)),
                    attr_id_lm: Some(crate::grid::HighlightId(0)),
                },
                crate::grid::CursorModeInfo {
                    shape: crate::grid::CursorShape::Horizontal,
                    cell_percentage: 25,
                    blink_wait: 700,
                    blink_on: 400,
                    blink_off: 250,
                    attr_id: Some(crate::grid::HighlightId(8)),
                    attr_id_lm: Some(crate::grid::HighlightId(0)),
                },
                crate::grid::CursorModeInfo {
                    shape: crate::grid::CursorShape::Vertical,
                    cell_percentage: 20,
                    blink_wait: 700,
                    blink_on: 400,
                    blink_off: 250,
                    attr_id: Some(crate::grid::HighlightId(9)),
                    attr_id_lm: Some(crate::grid::HighlightId(0)),
                },
            ],
        }
    );
}

#[test]
fn redraw_default_colors_set_decodes_rgb_defaults() {
    let (sender, receiver) = unbounded();
    let params = Value::Array(vec![Value::Array(vec![
        Value::from("default_colors_set"),
        Value::Array(vec![
            Value::from(0x101010u64),
            Value::from(0xf0f0f0u64),
            Value::from(0xff0000u64),
            Value::from(15u64),
            Value::from(0u64),
        ]),
    ])]);

    handle_notification("redraw", &params, &sender).expect("redraw should decode");

    assert_eq!(
        receiver.try_recv().expect("event should be available"),
        NvimEvent::DefaultColorsSet {
            foreground: Some(0x101010),
            background: Some(0xf0f0f0),
            special: Some(0xff0000),
        }
    );
}

#[test]
fn startup_theme_is_collected_before_the_first_flush_is_forwarded() {
    let message = Value::Array(vec![
        Value::from(2u64),
        Value::from("redraw"),
        Value::Array(vec![
            Value::Array(vec![
                Value::from("default_colors_set"),
                Value::Array(vec![
                    Value::from(0x101010u64),
                    Value::from(0xf0f0f0u64),
                    Value::from(0xff0000u64),
                ]),
            ]),
            Value::Array(vec![
                Value::from("hl_attr_define"),
                Value::Array(vec![
                    Value::from(1u64),
                    Value::Map(vec![
                        (Value::from("foreground"), Value::from(0x202020u64)),
                        (Value::from("background"), Value::from(0xe0e0e0u64)),
                    ]),
                    Value::Map(Vec::new()),
                    Value::Array(vec![Value::Map(vec![
                        (Value::from("kind"), Value::from("ui")),
                        (Value::from("ui_name"), Value::from("Normal")),
                    ])]),
                ]),
            ]),
            Value::Array(vec![Value::from("flush")]),
        ]),
    ]);
    let mut theme = NvimTheme::default();

    assert!(observe_startup_theme(&message, &mut theme));
    assert_eq!(
        theme,
        NvimTheme {
            default_foreground: Some(0x101010),
            default_background: Some(0xf0f0f0),
            normal_foreground: Some(0x202020),
            normal_background: Some(0xe0e0e0),
            normal_float_background: None,
        }
    );
}

#[test]
fn redraw_grid_line_preserves_highlight_repeat_and_wrap() {
    let (sender, receiver) = unbounded();
    let params = Value::Array(vec![Value::Array(vec![
        Value::from("grid_line"),
        Value::Array(vec![
            Value::from(1),
            Value::from(2),
            Value::from(3),
            Value::Array(vec![
                Value::Array(vec![Value::from("界"), Value::from(9)]),
                Value::Array(vec![Value::from(""), Value::from(9)]),
                Value::Array(vec![Value::from("x"), Value::from(10), Value::from(2)]),
            ]),
            Value::Boolean(true),
        ]),
    ])]);

    handle_notification("redraw", &params, &sender).expect("redraw should decode");

    assert_eq!(
        receiver.try_recv().expect("event should be available"),
        NvimEvent::GridLine {
            grid: 1,
            row: 2,
            col_start: 3,
            cells: vec![
                crate::grid::GridLineCell::new("界", crate::grid::HighlightId(9), 1),
                crate::grid::GridLineCell::new("", crate::grid::HighlightId(9), 1),
                crate::grid::GridLineCell::new("x", crate::grid::HighlightId(10), 2),
            ],
            wraps_to_next: true,
        }
    );
}

#[test]
fn redraw_grid_line_preserves_zero_repeat_markers() {
    let (sender, receiver) = unbounded();
    let params = Value::Array(vec![Value::Array(vec![
        Value::from("grid_line"),
        Value::Array(vec![
            Value::from(1),
            Value::from(0),
            Value::from(0),
            Value::Array(vec![Value::Array(vec![
                Value::from(" "),
                Value::from(0),
                Value::from(0),
            ])]),
            Value::Boolean(false),
        ]),
    ])]);

    handle_notification("redraw", &params, &sender).expect("redraw should decode");

    assert_eq!(
        receiver.try_recv().expect("event should be available"),
        NvimEvent::GridLine {
            grid: 1,
            row: 0,
            col_start: 0,
            cells: vec![crate::grid::GridLineCell::new(
                " ",
                crate::grid::DEFAULT_HIGHLIGHT,
                0,
            )],
            wraps_to_next: false,
        }
    );
}

#[test]
fn redraw_multigrid_window_events_are_decoded() {
    let (sender, receiver) = unbounded();
    let params = Value::Array(vec![
        Value::Array(vec![
            Value::from("win_pos"),
            Value::Array(vec![
                Value::from(2),
                Value::Ext(1, vec![205, 3, 232]),
                Value::from(3),
                Value::from(4),
                Value::from(40),
                Value::from(10),
            ]),
        ]),
        Value::Array(vec![
            Value::from("win_float_pos"),
            Value::Array(vec![
                Value::from(3),
                Value::Ext(1, vec![205, 3, 233]),
                Value::from("NW"),
                Value::from(1),
                Value::from(0),
                Value::from(0),
                Value::Boolean(true),
                Value::from(50),
                Value::from(7),
                Value::from(5),
                Value::from(6),
            ]),
        ]),
        Value::Array(vec![
            Value::from("win_hide"),
            Value::Array(vec![Value::from(3)]),
        ]),
    ]);

    handle_notification("redraw", &params, &sender).expect("redraw should decode");

    assert_eq!(
        receiver.try_recv().expect("win_pos should be available"),
        NvimEvent::WinPos {
            grid: 2,
            win: vec![205, 3, 232],
            row: 3,
            col: 4,
            width: 40,
            height: 10,
        }
    );
    assert_eq!(
        receiver
            .try_recv()
            .expect("win_float_pos should be available"),
        NvimEvent::WinFloatPos {
            grid: 3,
            win: vec![205, 3, 233],
            anchor: "NW".to_owned(),
            anchor_grid: 1,
            anchor_row: 0,
            anchor_col: 0,
            mouse_enabled: true,
            zindex: 50,
            compindex: 7,
            screen_row: 5,
            screen_col: 6,
        }
    );
    assert_eq!(
        receiver.try_recv().expect("win_hide should be available"),
        NvimEvent::WinHide { grid: 3 }
    );
}

#[test]
fn redraw_viewport_events_are_decoded() {
    let (sender, receiver) = unbounded();
    let window = Value::Ext(1, vec![205, 3, 232]);
    let params = Value::Array(vec![
        Value::Array(vec![
            Value::from("win_viewport"),
            Value::Array(vec![
                Value::from(2),
                window.clone(),
                Value::from(10),
                Value::from(28),
                Value::from(14),
                Value::from(7),
                Value::from(100),
                Value::from(-2),
            ]),
        ]),
        Value::Array(vec![
            Value::from("win_viewport_margins"),
            Value::Array(vec![
                Value::from(2),
                window,
                Value::from(1),
                Value::from(2),
                Value::from(3),
                Value::from(4),
            ]),
        ]),
    ]);

    handle_notification("redraw", &params, &sender).expect("redraw should decode");

    assert_eq!(
        receiver
            .try_recv()
            .expect("win_viewport should be available"),
        NvimEvent::WinViewport {
            grid: 2,
            win: vec![205, 3, 232],
            topline: 10,
            botline: 28,
            curline: 14,
            curcol: 7,
            line_count: 100,
            scroll_delta: -2,
        }
    );
    assert_eq!(
        receiver
            .try_recv()
            .expect("win_viewport_margins should be available"),
        NvimEvent::WinViewportMargins {
            grid: 2,
            win: vec![205, 3, 232],
            top: 1,
            bottom: 2,
            left: 3,
            right: 4,
        }
    );
}

#[test]
fn redraw_msg_set_pos_is_decoded() {
    let (sender, receiver) = unbounded();
    let params = Value::Array(vec![Value::Array(vec![
        Value::from("msg_set_pos"),
        Value::Array(vec![
            Value::from(3),
            Value::from(20),
            Value::Boolean(true),
            Value::from("─"),
            Value::from(200),
            Value::from(9),
        ]),
    ])]);

    handle_notification("redraw", &params, &sender).expect("redraw should decode");

    assert_eq!(
        receiver
            .try_recv()
            .expect("msg_set_pos should be available"),
        NvimEvent::MsgSetPos {
            grid: 3,
            row: 20,
            scrolled: true,
            sep_char: "─".to_owned(),
            zindex: 200,
            compindex: 9,
        }
    );
}

#[test]
fn ui_attach_enables_multigrid() {
    let params = ui_attach_params(80, 24);
    let options = params[2].as_map().expect("ui options should be a map");

    assert_eq!(
        options
            .iter()
            .find(|(key, _)| key.as_str() == Some("ext_multigrid"))
            .and_then(|(_, value)| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        options
            .iter()
            .find(|(key, _)| key.as_str() == Some("ext_hlstate"))
            .and_then(|(_, value)| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        options
            .iter()
            .find(|(key, _)| key.as_str() == Some("stdout_tty"))
            .and_then(|(_, value)| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        options
            .iter()
            .find(|(key, _)| key.as_str() == Some("stdin_tty"))
            .and_then(|(_, value)| value.as_bool()),
        Some(true)
    );
}

#[test]
fn term_event_notification_uses_the_nvim_ui_term_event_api() {
    let frame = term_event_notification_frame(
        "termresponse".to_owned(),
        "\x1bP>|kitty 0.40.0\x1b\\".to_owned(),
    );

    assert_eq!(frame[0].as_u64(), Some(2));
    assert_eq!(frame[1].as_str(), Some("nvim_ui_term_event"));
    assert_eq!(frame[2][0].as_str(), Some("termresponse"));
}
