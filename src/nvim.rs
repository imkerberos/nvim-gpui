//! Minimal Neovim MessagePack-RPC bootstrap.
//!
//! It starts `nvim --embed` with the caller's remaining command-line
//! arguments, identifies the client, attaches a small line-grid UI, and
//! forwards line-grid redraw events and queued input to GPUI/Neovim.
//! Richer UI capabilities will be layered on top of these events later.

use crate::grid::{CursorModeInfo, CursorShape, GridLineCell, HighlightAttrs, HighlightId};
use async_channel::{Receiver, Sender, TryRecvError};
use rmpv::Value;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io::{BufReader, ErrorKind, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;

#[cfg(unix)]
use std::os::unix::net::UnixStream;

const CLIENT_NAME: &str = "nvim-gpui";
const NVIM_EXITED: &str = "nvim process exited";

type RpcReader = BufReader<Box<dyn Read + Send>>;
type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

enum RemoteConnection {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
}

impl RemoteConnection {
    fn shutdown(&self) {
        match self {
            Self::Tcp(stream) => {
                let _ = stream.shutdown(Shutdown::Both);
            }
            #[cfg(unix)]
            Self::Unix(stream) => {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
    }
}

type RpcStreams = (
    Box<dyn Read + Send>,
    Box<dyn Write + Send>,
    RemoteConnection,
);

enum NvimCommand {
    Input(String),
    Resize { width: u32, height: u32 },
    TermEvent { event: String, value: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NvimEvent {
    ApiReady {
        api_level: u64,
    },
    UiAttached {
        width: u32,
        height: u32,
    },
    GridResized {
        grid: u64,
        width: u32,
        height: u32,
    },
    GridLine {
        grid: u64,
        row: u64,
        col_start: u64,
        cells: Vec<GridLineCell>,
        wraps_to_next: bool,
    },
    DefaultColorsSet {
        foreground: Option<u32>,
        background: Option<u32>,
        special: Option<u32>,
    },
    HlAttrDefine {
        id: HighlightId,
        attrs: HighlightAttrs,
    },
    GridClear {
        grid: u64,
    },
    GridDestroy {
        grid: u64,
    },
    GridCursorGoto {
        grid: u64,
        row: u64,
        col: u64,
    },
    GridScroll {
        grid: u64,
        top: u64,
        bot: u64,
        left: u64,
        right: u64,
        rows: i64,
        cols: i64,
    },
    WinPos {
        grid: u64,
        win: Vec<u8>,
        row: u64,
        col: u64,
        width: u64,
        height: u64,
    },
    WinFloatPos {
        grid: u64,
        win: Vec<u8>,
        anchor: String,
        anchor_grid: u64,
        anchor_row: i64,
        anchor_col: i64,
        mouse_enabled: bool,
        zindex: i64,
        compindex: i64,
        screen_row: i64,
        screen_col: i64,
    },
    WinExternalPos {
        grid: u64,
        win: Vec<u8>,
    },
    WinHide {
        grid: u64,
    },
    WinClose {
        grid: u64,
    },
    OptionSet {
        name: String,
        value: String,
    },
    SetTitle {
        title: String,
    },
    SetIcon {
        icon: String,
    },
    ModeInfoSet {
        cursor_style_enabled: bool,
        modes: Vec<CursorModeInfo>,
    },
    ModeChanged {
        mode: String,
        mode_idx: u64,
    },
    UiSend {
        data: String,
    },
    Flush,
    Error(String),
    Disconnected,
}

pub struct NvimProcess {
    child: Option<Arc<Mutex<Child>>>,
    remote: Option<Arc<RemoteConnection>>,
    shutdown_requested: Arc<AtomicBool>,
    commands: Sender<NvimCommand>,
    events: Receiver<NvimEvent>,
}

/// Return the Neovim executable selected by the repository development shell.
///
/// The shell variables can outlive the repository directory when a user runs
/// `nix develop` and then changes directories. Only honor that selection while
/// the current working directory is still inside the repository that owns the
/// configured `NVIM_GPUI_CONFIG_DIR`.
pub fn configured_nvim_command() -> Option<OsString> {
    let environment: HashMap<OsString, OsString> = std::env::vars_os().collect();
    project_nvim_environment_is_active(&environment)
        .then(|| environment.get(OsStr::new("NVIM_GPUI_NVIM")).cloned())
        .flatten()
}

impl NvimProcess {
    pub fn spawn(
        width: u32,
        height: u32,
        nvim_args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, String> {
        let nvim_command = configured_nvim_command().unwrap_or_else(|| OsString::from("nvim"));
        Self::spawn_with_command(width, height, nvim_command, nvim_args)
    }

    pub fn spawn_with_command(
        width: u32,
        height: u32,
        nvim_command: impl AsRef<OsStr>,
        nvim_args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, String> {
        let mut command = Command::new(nvim_command);
        apply_nvim_environment(&mut command);
        command.arg("--embed").args(nvim_args);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| format!("failed to start nvim --embed: {error}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "nvim stdin was not piped".to_owned())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "nvim stdout was not piped".to_owned())?;
        let writer: SharedWriter = Arc::new(Mutex::new(Box::new(stdin)));
        let reader: Box<dyn Read + Send> = Box::new(stdout);
        let child = Arc::new(Mutex::new(child));
        Self::start_workers(width, height, writer, reader, Some(child), None)
    }

    pub fn connect(width: u32, height: u32, address: &str) -> Result<Self, String> {
        let (reader, writer, remote) = connect_remote(address)?;
        let writer: SharedWriter = Arc::new(Mutex::new(writer));
        Self::start_workers(width, height, writer, reader, None, Some(Arc::new(remote)))
    }

    fn start_workers(
        width: u32,
        height: u32,
        writer: SharedWriter,
        reader: Box<dyn Read + Send>,
        child: Option<Arc<Mutex<Child>>>,
        remote: Option<Arc<RemoteConnection>>,
    ) -> Result<Self, String> {
        let worker_child = child.clone();
        let worker_remote = remote.clone();
        let shutdown_requested = Arc::new(AtomicBool::new(false));
        let worker_shutdown_requested = Arc::clone(&shutdown_requested);
        let command_shutdown_requested = Arc::clone(&shutdown_requested);
        let rpc_ready = Arc::new(AtomicBool::new(false));
        let command_rpc_ready = Arc::clone(&rpc_ready);
        let worker_rpc_ready = Arc::clone(&rpc_ready);
        let rpc_alive = Arc::new(AtomicBool::new(true));
        let command_rpc_alive = Arc::clone(&rpc_alive);
        let (event_tx, events) = async_channel::unbounded();
        let worker_tx = event_tx.clone();
        let (command_tx, command_rx) = async_channel::unbounded();

        let command_writer = Arc::clone(&writer);
        let command_events = event_tx.clone();
        thread::Builder::new()
            .name("nvim-input".to_owned())
            .spawn(move || {
                run_command_writer(
                    command_writer,
                    command_rx,
                    command_events,
                    command_shutdown_requested,
                    command_rpc_ready,
                    command_rpc_alive,
                );
            })
            .map_err(|error| {
                stop_backend(&child, &remote);
                format!("failed to start Neovim input worker: {error}")
            })?;

        thread::Builder::new()
            .name("nvim-rpc".to_owned())
            .spawn(move || {
                let result =
                    run_session(writer, reader, width, height, &worker_tx, &worker_rpc_ready);
                if let Err(error) = result {
                    if !worker_shutdown_requested.load(Ordering::Acquire) && error != NVIM_EXITED {
                        eprintln!("[nvim-rpc] {error}");
                        let _ = worker_tx.send_blocking(NvimEvent::Error(error));
                    }
                    stop_backend(&worker_child, &worker_remote);
                }

                if let Some(child) = worker_child.as_ref() {
                    if let Ok(mut child) = child.lock() {
                        let _ = child.wait();
                    }
                }
                rpc_alive.store(false, Ordering::Release);
                let _ = worker_tx.send_blocking(NvimEvent::Disconnected);
            })
            .map_err(|error| {
                shutdown_requested.store(true, Ordering::Release);
                stop_backend(&child, &remote);
                format!("failed to start nvim RPC worker: {error}")
            })?;

        Ok(Self {
            child,
            remote,
            shutdown_requested,
            commands: command_tx,
            events,
        })
    }

    pub fn events(&self) -> Receiver<NvimEvent> {
        self.events.clone()
    }

    pub fn send_input(&self, input: impl Into<String>) -> Result<(), String> {
        self.commands
            .try_send(NvimCommand::Input(input.into()))
            .map_err(|error| format!("failed to queue Neovim input: {error}"))
    }

    pub fn send_resize(&self, width: u32, height: u32) -> Result<(), String> {
        self.commands
            .try_send(NvimCommand::Resize { width, height })
            .map_err(|error| format!("failed to queue Neovim resize: {error}"))
    }

    pub fn send_term_event(
        &self,
        event: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), String> {
        self.commands
            .try_send(NvimCommand::TermEvent {
                event: event.into(),
                value: value.into(),
            })
            .map_err(|error| format!("failed to queue Neovim terminal response: {error}"))
    }
}

impl Drop for NvimProcess {
    fn drop(&mut self) {
        self.shutdown_requested.store(true, Ordering::Release);
        stop_backend(&self.child, &self.remote);
    }
}

fn stop_backend(child: &Option<Arc<Mutex<Child>>>, remote: &Option<Arc<RemoteConnection>>) {
    if let Some(child) = child {
        terminate_child(child);
    }
    if let Some(remote) = remote {
        remote.shutdown();
    }
}

fn terminate_child(child: &Arc<Mutex<Child>>) {
    if let Ok(mut child) = child.lock() {
        let running = child
            .try_wait()
            .map(|status| status.is_none())
            .unwrap_or(true);
        if running {
            let _ = child.kill();
        }
    }
}

fn run_command_writer(
    writer: SharedWriter,
    commands: Receiver<NvimCommand>,
    events: Sender<NvimEvent>,
    shutdown_requested: Arc<AtomicBool>,
    rpc_ready: Arc<AtomicBool>,
    rpc_alive: Arc<AtomicBool>,
) {
    let mut request_id = 1_000_000;

    loop {
        if shutdown_requested.load(Ordering::Acquire) || !rpc_alive.load(Ordering::Acquire) {
            return;
        }

        if !rpc_ready.load(Ordering::Acquire) {
            thread::yield_now();
            continue;
        }

        let command = match commands.try_recv() {
            Ok(command) => command,
            Err(TryRecvError::Empty) => {
                thread::sleep(std::time::Duration::from_millis(1));
                continue;
            }
            Err(TryRecvError::Closed) => return,
        };

        let message = match command {
            NvimCommand::Input(input) => Value::Array(vec![
                Value::from(2),
                Value::from("nvim_input"),
                Value::Array(vec![Value::from(input)]),
            ]),
            NvimCommand::Resize { width, height } => {
                let message = resize_request_frame(request_id, width, height);
                request_id += 1;
                message
            }
            NvimCommand::TermEvent { event, value } => term_event_notification_frame(event, value),
        };
        if let Err(error) = write_shared_message(&writer, &message) {
            if !shutdown_requested.load(Ordering::Acquire) {
                let _ = events.send_blocking(NvimEvent::Error(error));
            }
            return;
        }
    }
}

fn connect_remote(address: &str) -> Result<RpcStreams, String> {
    if let Some(address) = address
        .strip_prefix("tcp://")
        .or_else(|| address.strip_prefix("tcp:"))
    {
        let stream = TcpStream::connect(address)
            .map_err(|error| format!("failed to connect to Neovim at {address}: {error}"))?;
        let _ = stream.set_nodelay(true);
        let reader = stream
            .try_clone()
            .map_err(|error| format!("failed to clone Neovim TCP reader: {error}"))?;
        let writer = stream
            .try_clone()
            .map_err(|error| format!("failed to clone Neovim TCP writer: {error}"))?;
        return Ok((
            Box::new(reader),
            Box::new(writer),
            RemoteConnection::Tcp(stream),
        ));
    }

    if let Some(path) = address
        .strip_prefix("unix://")
        .or_else(|| address.strip_prefix("unix:"))
    {
        return connect_unix(path);
    }

    if address.contains('/') || Path::new(address).exists() {
        return connect_unix(address);
    }

    let stream = TcpStream::connect(address)
        .map_err(|error| format!("failed to connect to Neovim at {address}: {error}"))?;
    let _ = stream.set_nodelay(true);
    let reader = stream
        .try_clone()
        .map_err(|error| format!("failed to clone Neovim TCP reader: {error}"))?;
    let writer = stream
        .try_clone()
        .map_err(|error| format!("failed to clone Neovim TCP writer: {error}"))?;
    Ok((
        Box::new(reader),
        Box::new(writer),
        RemoteConnection::Tcp(stream),
    ))
}

#[cfg(unix)]
fn connect_unix(path: &str) -> Result<RpcStreams, String> {
    let stream = UnixStream::connect(path)
        .map_err(|error| format!("failed to connect to Neovim socket {path}: {error}"))?;
    let reader = stream
        .try_clone()
        .map_err(|error| format!("failed to clone Neovim Unix reader: {error}"))?;
    let writer = stream
        .try_clone()
        .map_err(|error| format!("failed to clone Neovim Unix writer: {error}"))?;
    Ok((
        Box::new(reader),
        Box::new(writer),
        RemoteConnection::Unix(stream),
    ))
}

#[cfg(not(unix))]
fn connect_unix(path: &str) -> Result<RpcStreams, String> {
    let _ = path;
    Err("unix Neovim sockets are not supported on this platform; use tcp:HOST:PORT".to_owned())
}

fn apply_nvim_environment(command: &mut Command) {
    let mut environment: HashMap<OsString, OsString> = std::env::vars_os().collect();
    let project_environment = project_nvim_environment_is_active(&environment);

    if should_import_login_environment() {
        if let Some(startup_environment) = login_shell_environment() {
            let current_environment = environment.clone();
            environment.extend(startup_environment);
            let mut protected = Vec::new();
            if project_environment {
                protected.extend([
                    "NVIM_APPNAME",
                    "NVIM_GPUI_CACHE_DIR",
                    "NVIM_GPUI_CONFIG_DIR",
                    "NVIM_GPUI_ENVIRONMENT",
                    "NVIM_GPUI_NVIM",
                    "DIRENV_DIR",
                    "DIRENV_IN_ENVRC",
                    "PATH",
                    "PWD",
                    "TMPDIR",
                    "CARGO_HOME",
                    "CARGO_TARGET_DIR",
                ]);
            }
            for key in protected {
                if let Some(value) = current_environment.get(OsStr::new(key)) {
                    environment.insert(OsString::from(key), value.clone());
                }
            }
        }
    }

    // Keep the repository-local Neovim profile scoped to the child process.
    // The development shell deliberately does not export XDG_* because those
    // variables are global to tools such as Git. Neovim still gets its
    // isolated config, data, state, and cache directories here.
    if project_environment {
        apply_project_nvim_environment(&mut environment);
    } else {
        remove_project_nvim_environment(&mut environment);
    }
    command.envs(environment);
}

fn project_nvim_environment_is_active(environment: &HashMap<OsString, OsString>) -> bool {
    let Ok(current_directory) = std::env::current_dir() else {
        return false;
    };
    project_nvim_environment_is_active_at(environment, &current_directory)
}

fn project_nvim_environment_is_active_at(
    environment: &HashMap<OsString, OsString>,
    current_directory: &Path,
) -> bool {
    let Some(config_dir) = environment.get(OsStr::new("NVIM_GPUI_CONFIG_DIR")) else {
        return false;
    };
    let Some(project_root) = Path::new(config_dir).parent() else {
        return false;
    };
    current_directory.starts_with(project_root)
}

fn remove_project_nvim_environment(environment: &mut HashMap<OsString, OsString>) {
    for key in [
        "NVIM_GPUI_CACHE_DIR",
        "NVIM_GPUI_CONFIG_DIR",
        "NVIM_GPUI_ENVIRONMENT",
        "NVIM_GPUI_IMAGEMAGICK",
        "NVIM_GPUI_LAZY",
        "NVIM_GPUI_NVIM",
        "NVIM_GPUI_SNACKS",
        "NVIM_GPUI_TREESITTER",
        "NVIM_GPUI_CELL_WIDTH",
        "NVIM_GPUI_CELL_HEIGHT",
        "DIRENV_DIR",
        "DIRENV_IN_ENVRC",
    ] {
        environment.remove(OsStr::new(key));
    }

    if environment.get(OsStr::new("NVIM_APPNAME")) == Some(&OsString::from("nvim-gpui")) {
        environment.remove(OsStr::new("NVIM_APPNAME"));
    }
}

fn apply_project_nvim_environment(environment: &mut HashMap<OsString, OsString>) {
    if let Some(config_dir) = environment.get(OsStr::new("NVIM_GPUI_CONFIG_DIR")).cloned() {
        environment.insert(OsString::from("XDG_CONFIG_HOME"), config_dir);
    }

    let Some(cache_dir) = environment.get(OsStr::new("NVIM_GPUI_CACHE_DIR")).cloned() else {
        return;
    };

    let cache_dir = std::path::PathBuf::from(cache_dir);
    for (name, suffix) in [
        ("XDG_DATA_HOME", "nvim-data"),
        ("XDG_STATE_HOME", "nvim-state"),
        ("XDG_CACHE_HOME", "nvim-cache"),
    ] {
        environment.insert(
            OsString::from(name),
            cache_dir.join(suffix).into_os_string(),
        );
    }
}

fn should_import_login_environment() -> bool {
    std::env::var_os("NVIM_GPUI_IMPORT_SHELL_ENV").as_deref() == Some(OsStr::new("1"))
        || is_app_bundle_process()
}

fn is_app_bundle_process() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.to_str().map(str::to_owned))
        .is_some_and(|path| path.contains(".app/Contents/MacOS/"))
}

fn login_shell_environment() -> Option<HashMap<OsString, OsString>> {
    #[cfg(target_os = "macos")]
    {
        let shell = std::env::var_os("SHELL").unwrap_or_else(|| OsString::from("/bin/zsh"));
        let output = Command::new(shell).args(["-ilc", "env -0"]).output().ok()?;
        if !output.status.success() {
            return None;
        }

        Some(parse_environment(&output.stdout))
    }

    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

fn parse_environment(bytes: &[u8]) -> HashMap<OsString, OsString> {
    bytes
        .split(|byte| *byte == 0)
        .filter_map(|entry| {
            let separator = entry.iter().position(|byte| *byte == b'=')?;
            let key = String::from_utf8_lossy(&entry[..separator]);
            let value = String::from_utf8_lossy(&entry[separator + 1..]);
            Some((OsString::from(key.as_ref()), OsString::from(value.as_ref())))
        })
        .collect()
}

fn run_session(
    writer: SharedWriter,
    reader: Box<dyn Read + Send>,
    width: u32,
    height: u32,
    events: &Sender<NvimEvent>,
    rpc_ready: &AtomicBool,
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

    let api_level = api_info
        .as_array()
        .and_then(|values| values.get(1))
        .and_then(|metadata| map_value(metadata, "version"))
        .and_then(|version| map_value(version, "api_level"))
        .and_then(Value::as_u64)
        .ok_or_else(|| "nvim_get_api_info response has no api level".to_owned())?;
    send_event(events, NvimEvent::ApiReady { api_level })?;

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
        ui_attach_params(width, height),
        events,
    )?;
    rpc_ready.store(true, Ordering::Release);
    send_event(events, NvimEvent::UiAttached { width, height })?;

    loop {
        let message = read_message(&mut reader)?;
        dispatch_message(&writer, message, events)?;
    }
}

fn client_info_params() -> Value {
    Value::Array(vec![
        Value::from(CLIENT_NAME),
        Value::Map(vec![
            (Value::from("major"), Value::from(0)),
            (Value::from("minor"), Value::from(1)),
            (Value::from("patch"), Value::from(0)),
        ]),
        Value::from("ui"),
        Value::Map(Vec::new()),
        Value::Map(Vec::new()),
    ])
}

fn ui_attach_params(width: u32, height: u32) -> Value {
    Value::Array(vec![
        Value::from(width),
        Value::from(height),
        Value::Map(vec![
            (Value::from("rgb"), Value::Boolean(true)),
            (Value::from("ext_linegrid"), Value::Boolean(true)),
            (Value::from("ext_multigrid"), Value::Boolean(true)),
            // GPUI supplies interactive keyboard input through nvim_input.
            // Mark it as a TTY-like input so plugins such as Snacks Dashboard
            // do not mistake this UI for a non-interactive/piped frontend.
            (Value::from("stdin_tty"), Value::Boolean(true)),
            (Value::from("stdout_tty"), Value::Boolean(true)),
        ]),
    ])
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

fn resize_request_frame(id: u64, width: u32, height: u32) -> Value {
    Value::Array(vec![
        Value::from(0),
        Value::from(id),
        Value::from("nvim_ui_try_resize"),
        Value::Array(vec![Value::from(width), Value::from(height)]),
    ])
}

fn term_event_notification_frame(event: String, value: String) -> Value {
    Value::Array(vec![
        Value::from(2),
        Value::from("nvim_ui_term_event"),
        Value::Array(vec![Value::from(event), Value::from(value)]),
    ])
}

fn handle_notification(
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

fn parse_hl_attr_define(args: &[Value]) -> Result<NvimEvent, String> {
    let id = args[0]
        .as_u64()
        .ok_or_else(|| "hl_attr_define has an invalid highlight id".to_owned())?;
    let attrs = parse_highlight_attrs(&args[1])?;

    Ok(NvimEvent::HlAttrDefine {
        id: HighlightId(id),
        attrs,
    })
}

fn parse_mode_info_set(args: &[Value]) -> Result<NvimEvent, String> {
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

fn parse_cursor_mode_info(value: &Value) -> Result<CursorModeInfo, String> {
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

fn parse_percentage(value: &Value, name: &str) -> Result<u8, String> {
    let value = parse_u32(value, name)?;
    Ok(value.min(100) as u8)
}

fn parse_u32(value: &Value, name: &str) -> Result<u32, String> {
    value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| format!("mode_info_set has an invalid {name}"))
}

fn parse_optional_highlight_id(value: &Value, name: &str) -> Result<Option<HighlightId>, String> {
    value
        .as_u64()
        .map(|value| Some(HighlightId(value)))
        .ok_or_else(|| format!("mode_info_set has an invalid {name}"))
}

fn parse_highlight_attrs(value: &Value) -> Result<HighlightAttrs, String> {
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
                attrs.blend = value
                    .as_u64()
                    .and_then(|blend| u8::try_from(blend).ok())
                    .or_else(|| (value.as_i64() == Some(-1)).then_some(0));
                if attrs.blend.is_none() {
                    return Err("hl_attr_define has an invalid blend level".to_owned());
                }
            }
            _ => {}
        }
    }

    Ok(attrs)
}

fn parse_color(value: &Value, name: &str) -> Result<Option<u32>, String> {
    if value.as_i64() == Some(-1) {
        return Ok(None);
    }

    value
        .as_u64()
        .and_then(|color| u32::try_from(color).ok())
        .map(Some)
        .ok_or_else(|| format!("hl_attr_define has an invalid {name} color"))
}

fn parse_bool(value: &Value, name: &str) -> Result<bool, String> {
    bool_value(value).ok_or_else(|| format!("hl_attr_define has an invalid {name} flag"))
}

fn parse_grid_line(args: &[Value]) -> Result<NvimEvent, String> {
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
        wraps_to_next: args.get(4).and_then(bool_value).unwrap_or(false),
    })
}

fn parse_win_pos(args: &[Value]) -> Result<NvimEvent, String> {
    Ok(NvimEvent::WinPos {
        grid: parse_u64_value(&args[0], "win_pos grid")?,
        win: parse_window_id(&args[1])?,
        row: parse_u64_value(&args[2], "win_pos row")?,
        col: parse_u64_value(&args[3], "win_pos column")?,
        width: parse_u64_value(&args[4], "win_pos width")?,
        height: parse_u64_value(&args[5], "win_pos height")?,
    })
}

fn parse_win_float_pos(args: &[Value]) -> Result<NvimEvent, String> {
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

fn parse_u64_value(value: &Value, name: &str) -> Result<u64, String> {
    value.as_u64().ok_or_else(|| format!("{name} is invalid"))
}

fn parse_window_id(value: &Value) -> Result<Vec<u8>, String> {
    match value {
        Value::Ext(_, bytes) => Ok(bytes.clone()),
        _ => Err(format!(
            "window id is not a MessagePack extension: {value:?}"
        )),
    }
}

fn parse_i64_value(value: &Value, name: &str) -> Result<i64, String> {
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

fn send_event(events: &Sender<NvimEvent>, event: NvimEvent) -> Result<(), String> {
    events
        .send_blocking(event)
        .map_err(|_| "GPUI stopped receiving Neovim events".to_owned())
}

fn read_message(reader: &mut impl Read) -> Result<Value, String> {
    rmpv::decode::read_value(reader).map_err(|error| {
        if error.kind() == ErrorKind::UnexpectedEof {
            NVIM_EXITED.to_owned()
        } else {
            format!("failed to decode RPC message: {error}")
        }
    })
}

fn write_shared_message(writer: &SharedWriter, message: &Value) -> Result<(), String> {
    let mut writer = writer
        .lock()
        .map_err(|_| "failed to lock Neovim RPC writer".to_owned())?;
    write_message(&mut *writer, message)
}

fn write_message(writer: &mut impl Write, message: &Value) -> Result<(), String> {
    rmpv::encode::write_value(writer, message)
        .map_err(|error| format!("failed to encode RPC message: {error}"))?;
    writer
        .flush()
        .map_err(|error| format!("failed to flush RPC message: {error}"))
}

fn map_value<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let Value::Map(entries) = value else {
        return None;
    };

    entries.iter().find_map(|(entry_key, entry_value)| {
        (string_value(entry_key).as_deref() == Some(key)).then_some(entry_value)
    })
}

fn string_value(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => value.as_str().map(str::to_owned),
        _ => None,
    }
}

fn display_value(value: &Value) -> String {
    string_value(value).unwrap_or_else(|| format!("{value:?}"))
}

fn bool_value(value: &Value) -> Option<bool> {
    match value {
        Value::Boolean(value) => Some(*value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_project_nvim_environment, handle_notification, parse_environment,
        project_nvim_environment_is_active_at, read_message, remove_project_nvim_environment,
        resize_request_frame, term_event_notification_frame, ui_attach_params, write_message,
        NvimEvent, NVIM_EXITED,
    };
    use async_channel::unbounded;
    use rmpv::Value;
    use std::{
        collections::HashMap,
        ffi::{OsStr, OsString},
        io::Cursor,
        path::Path,
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
    fn resize_request_frame_uses_the_nvim_ui_resize_method() {
        let frame = resize_request_frame(42, 120, 40);

        assert_eq!(frame[0].as_u64(), Some(0));
        assert_eq!(frame[1].as_u64(), Some(42));
        assert_eq!(frame[2].as_str(), Some("nvim_ui_try_resize"));
        assert_eq!(frame[3][0].as_u64(), Some(120));
        assert_eq!(frame[3][1].as_u64(), Some(40));
    }

    #[test]
    fn an_eof_is_classified_as_a_normal_nvim_exit() {
        let mut reader = Cursor::new(Vec::<u8>::new());

        assert_eq!(read_message(&mut reader), Err(NVIM_EXITED.to_owned()));
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
}
