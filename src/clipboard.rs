//! Local system clipboard access and the remote Neovim clipboard bridge.
//!
//! Pasting is a GUI operation in both embedded and remote sessions: GPUI reads
//! the local clipboard and the client calls `nvim_paste`. Register clipboard
//! access is different for remote sessions, where Neovim must ask this GUI to
//! read or write the local `+`/`*` clipboard through RPC.

use async_channel::{Receiver, Sender};
use gpui::{App, ClipboardItem};
use rmpv::Value;
use std::sync::mpsc::{sync_channel, SyncSender};

pub(crate) const CLIPBOARD_GET_METHOD: &str = "nvim_gpui_clipboard_get";
pub(crate) const CLIPBOARD_SET_METHOD: &str = "nvim_gpui_clipboard_set";

pub(crate) enum ClipboardRequest {
    Get {
        primary: bool,
        response: SyncSender<Result<String, String>>,
    },
    Set {
        primary: bool,
        text: String,
        response: SyncSender<Result<(), String>>,
    },
}

pub(crate) fn channel() -> (Sender<ClipboardRequest>, Receiver<ClipboardRequest>) {
    async_channel::unbounded()
}

pub(crate) fn get_request_handler(
    requests: Sender<ClipboardRequest>,
) -> impl Fn(&Value) -> Result<Value, String> + Send + Sync + 'static {
    move |params| {
        let primary = register_from_params(params)?;
        let (response_tx, response_rx) = sync_channel(1);
        requests
            .send_blocking(ClipboardRequest::Get {
                primary,
                response: response_tx,
            })
            .map_err(|_| "clipboard UI service is unavailable".to_owned())?;
        let text = response_rx
            .recv()
            .map_err(|_| "clipboard UI service stopped while reading".to_owned())??;
        Ok(register_value(&text))
    }
}

pub(crate) fn set_request_handler(
    requests: Sender<ClipboardRequest>,
) -> impl Fn(&Value) -> Result<Value, String> + Send + Sync + 'static {
    move |params| {
        let values = params
            .as_array()
            .ok_or_else(|| "clipboard_set arguments are not an array".to_owned())?;
        let primary = register_name(values.first())? == "*";
        let lines = values
            .get(1)
            .and_then(Value::as_array)
            .ok_or_else(|| "clipboard_set lines are not an array".to_owned())?;
        let mut text = String::new();
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                text.push('\n');
            }
            text.push_str(
                line.as_str()
                    .ok_or_else(|| "clipboard_set line is not a string".to_owned())?,
            );
        }

        let (response_tx, response_rx) = sync_channel(1);
        requests
            .send_blocking(ClipboardRequest::Set {
                primary,
                text,
                response: response_tx,
            })
            .map_err(|_| "clipboard UI service is unavailable".to_owned())?;
        response_rx
            .recv()
            .map_err(|_| "clipboard UI service stopped while writing".to_owned())??;
        Ok(Value::Nil)
    }
}

pub(crate) fn handle_on_ui_thread(app: &App, request: ClipboardRequest) {
    match request {
        ClipboardRequest::Get { primary, response } => {
            let result = read_text(app, primary);
            let _ = response.send(result);
        }
        ClipboardRequest::Set {
            primary,
            text,
            response,
        } => {
            write_text(app, primary, text);
            let _ = response.send(Ok(()));
        }
    }
}

pub(crate) fn paste_text(app: &App) -> Result<String, String> {
    read_text(app, false)
}

pub(crate) fn remote_provider_lua(channel_id: u64) -> String {
    r#"
local channel = __NVIM_GPUI_CHANNEL_ID__

local function clipboard_copy(register, lines, regtype)
  local ok, err = pcall(vim.rpcrequest, channel, "nvim_gpui_clipboard_set", register, lines, regtype)
  if not ok then
    error(err)
  end
end

local function clipboard_paste(register)
  local ok, value = pcall(vim.rpcrequest, channel, "nvim_gpui_clipboard_get", register)
  if not ok then
    error(value)
  end
  return value
end

vim.g.clipboard = {
  name = "nvim-gpui",
  copy = {
    ["+"] = function(lines, regtype) clipboard_copy("+", lines, regtype) end,
    ["*"] = function(lines, regtype) clipboard_copy("*", lines, regtype) end,
  },
  paste = {
    ["+"] = function() return clipboard_paste("+") end,
    ["*"] = function() return clipboard_paste("*") end,
  },
  cache_enabled = 0,
}

if vim.g.loaded_clipboard_provider then
  vim.g.loaded_clipboard_provider = nil
end
vim.cmd("runtime autoload/provider/clipboard.vim")
"#
    .replace("__NVIM_GPUI_CHANNEL_ID__", &channel_id.to_string())
}

fn register_from_params(params: &Value) -> Result<bool, String> {
    let values = params
        .as_array()
        .ok_or_else(|| "clipboard_get arguments are not an array".to_owned())?;
    Ok(register_name(values.first())? == "*")
}

fn register_name(value: Option<&Value>) -> Result<&str, String> {
    let register = value
        .and_then(Value::as_str)
        .ok_or_else(|| "clipboard register is not a string".to_owned())?;
    match register {
        "+" | "*" => Ok(register),
        _ => Err(format!("unsupported clipboard register: {register}")),
    }
}

fn register_value(text: &str) -> Value {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines = text.split('\n').map(Value::from).collect::<Vec<_>>();
    Value::Array(vec![Value::Array(lines), Value::from("v")])
}

fn read_text(app: &App, primary: bool) -> Result<String, String> {
    let item = if primary {
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        {
            app.read_from_primary()
        }
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        {
            app.read_from_clipboard()
        }
    } else {
        app.read_from_clipboard()
    };
    item.and_then(|item| item.text())
        .ok_or_else(|| "system clipboard does not contain text".to_owned())
}

fn write_text(app: &App, primary: bool, text: String) {
    let item = ClipboardItem::new_string(text);
    if primary {
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        {
            app.write_to_primary(item);
        }
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        {
            app.write_to_clipboard(item);
        }
    } else {
        app.write_to_clipboard(item);
    }
}

#[cfg(test)]
mod tests {
    use super::{register_value, remote_provider_lua};
    use rmpv::Value;

    #[test]
    fn register_value_preserves_multiline_text() {
        assert_eq!(
            register_value("one\ntwo"),
            Value::Array(vec![
                Value::Array(vec![Value::from("one"), Value::from("two")]),
                Value::from("v"),
            ])
        );
    }

    #[test]
    fn remote_provider_uses_the_gui_rpc_methods() {
        let lua = remote_provider_lua(1);
        assert!(lua.contains("nvim_gpui_clipboard_get"));
        assert!(lua.contains("nvim_gpui_clipboard_set"));
        assert!(lua.contains("runtime autoload/provider/clipboard.vim"));
    }
}
