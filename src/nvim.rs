//! Minimal Neovim MessagePack-RPC bootstrap.
//!
//! It starts `nvim --embed` with the caller's remaining command-line
//! arguments, identifies the client, attaches a small line-grid UI, and
//! forwards line-grid redraw events and queued input to GPUI/Neovim.
//! Richer UI capabilities will be layered on top of these events later.

use async_channel::{Receiver, Sender, TrySendError};
use rmpv::Value;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io::Read;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::AtomicU64;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};

mod compat;
mod environment;
mod protocol;
mod session;
mod transport;
mod types;
mod version;

#[cfg(test)]
mod tests;

use environment::apply_nvim_environment;
pub use environment::configured_nvim_command;
use protocol::{
    client_info_params, edit_file_command_params, mouse_event_notification_frame,
    resize_request_frame, term_event_notification_frame,
};
use session::{run_request_dispatcher, run_session};
use transport::{write_shared_message, RemoteConnection, SharedWriter};
pub use types::{DisconnectReason, NvimEvent, NvimFloatAnchor, NvimFloatPosition, NvimTheme};
use types::{NvimCommand, RequestState};
use version::parse_protocol_info;
pub use version::{NvimCapabilities, NvimProtocolInfo, NvimVersion};

const CLIENT_NAME: &str = "nvim-gpui";
const NVIM_GPUI_STARTUP_COMMAND: &str = "let g:nvim_gpui = v:true";
const NVIM_EXITED: &str = "nvim process exited";
const STARTUP_THEME_TIMEOUT: Duration = Duration::from_secs(1);
pub(crate) const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const COMMAND_QUEUE_CAPACITY: usize = 256;
const EVENT_QUEUE_CAPACITY: usize = 4096;
const INCOMING_REQUEST_QUEUE_CAPACITY: usize = 32;
const MAX_PENDING_REQUESTS: usize = 128;
const RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const RPC_TIMEOUT_POLL_INTERVAL: Duration = Duration::from_millis(50);

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(u64);

impl SessionId {
    fn next() -> Self {
        Self(NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed))
    }
}

struct RequestRegistry {
    queued: HashMap<u64, Arc<RequestState>>,
    pending: HashMap<u64, Arc<RequestState>>,
}

type PendingRequests = Arc<Mutex<RequestRegistry>>;
pub type RpcRequestHandler = Arc<dyn Fn(&Value) -> Result<Value, String> + Send + Sync + 'static>;
type RpcRequestHandlers = Arc<Mutex<HashMap<String, RpcRequestHandler>>>;

#[derive(Clone)]
struct MouseInput {
    button: String,
    action: String,
    modifier: String,
    grid: u64,
    row: u64,
    col: u64,
}

#[derive(Default)]
struct CoalescedCommands {
    resize: Option<(u32, u32)>,
    mouse_move: Option<MouseInput>,
    wake_enqueued: bool,
}

#[derive(Clone)]
pub(crate) enum ConnectionSpec {
    Embedded {
        command: OsString,
        args: Vec<OsString>,
    },
    Remote {
        address: String,
        connect_timeout: Duration,
    },
}

pub struct NvimProcess {
    session_id: SessionId,
    child: Option<Arc<Mutex<Child>>>,
    remote: Option<Arc<RemoteConnection>>,
    shutdown_requested: Arc<AtomicBool>,
    commands: Sender<NvimCommand>,
    events: Receiver<NvimEvent>,
    startup_theme: Option<NvimTheme>,
    protocol: Option<NvimProtocolInfo>,
    connection: ConnectionSpec,
    request_handlers: RpcRequestHandlers,
    coalesced: Arc<Mutex<CoalescedCommands>>,
    request_registry: PendingRequests,
    next_request_token: AtomicU64,
    request_timeout_stop: Arc<AtomicBool>,
    request_timeout_thread: Option<thread::JoinHandle<()>>,
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
        let nvim_command = nvim_command.as_ref().to_owned();
        let nvim_args: Vec<OsString> = nvim_args.into_iter().collect();
        log::info!(
            target: "nvim_gpui::nvim",
            "starting embedded Neovim command={} args={}",
            nvim_command.to_string_lossy(),
            nvim_args.len()
        );
        let mut command = Command::new(&nvim_command);
        #[cfg(target_os = "windows")]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        apply_nvim_environment(&mut command);
        command
            .args(["--embed", "--cmd", NVIM_GPUI_STARTUP_COMMAND])
            .args(&nvim_args);
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
        Self::start_workers(
            width,
            height,
            writer,
            reader,
            Some(child),
            None,
            ConnectionSpec::Embedded {
                command: nvim_command,
                args: nvim_args,
            },
        )
    }

    pub fn connect(width: u32, height: u32, address: &str) -> Result<Self, String> {
        Self::connect_with_timeout(width, height, address, DEFAULT_CONNECT_TIMEOUT)
    }

    pub fn connect_with_timeout(
        width: u32,
        height: u32,
        address: &str,
        connect_timeout: Duration,
    ) -> Result<Self, String> {
        log::info!(
            target: "nvim_gpui::nvim",
            "connecting to remote Neovim address={address}, timeout={connect_timeout:?}"
        );
        let (reader, writer, remote) =
            transport::connect_remote_with_timeout(address, connect_timeout)?;
        let writer: SharedWriter = Arc::new(Mutex::new(writer));
        let process = Self::start_workers(
            width,
            height,
            writer,
            reader,
            None,
            Some(Arc::new(remote)),
            ConnectionSpec::Remote {
                address: address.to_owned(),
                connect_timeout,
            },
        )?;
        if process.protocol().is_none() {
            return Err("Neovim RPC handshake did not complete".to_owned());
        }
        Ok(process)
    }

    pub fn reconnect(&self, width: u32, height: u32) -> Result<Self, String> {
        if self.shutdown_requested.load(Ordering::Acquire) {
            return Err("Neovim connection is shutting down".to_owned());
        }
        Self::connect_from_spec(&self.connection, width, height)
    }

    pub(crate) fn connection_spec(&self) -> ConnectionSpec {
        self.connection.clone()
    }

    pub(crate) fn is_remote(&self) -> bool {
        matches!(self.connection, ConnectionSpec::Remote { .. })
    }

    pub(crate) fn connect_from_spec(
        connection: &ConnectionSpec,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let process = match connection {
            ConnectionSpec::Embedded { command, args } => {
                Self::spawn_with_command(width, height, command, args.clone())
            }
            ConnectionSpec::Remote {
                address,
                connect_timeout,
            } => Self::connect_with_timeout(width, height, address, *connect_timeout),
        }?;
        if process.protocol().is_none() {
            return Err("Neovim RPC handshake did not complete".to_owned());
        }
        Ok(process)
    }

    fn start_workers(
        width: u32,
        height: u32,
        writer: SharedWriter,
        reader: Box<dyn Read + Send>,
        child: Option<Arc<Mutex<Child>>>,
        remote: Option<Arc<RemoteConnection>>,
        connection: ConnectionSpec,
    ) -> Result<Self, String> {
        let session_id = SessionId::next();
        let worker_child = child.clone();
        let worker_remote = remote.clone();
        let shutdown_requested = Arc::new(AtomicBool::new(false));
        let worker_shutdown_requested = Arc::clone(&shutdown_requested);
        let command_shutdown_requested = Arc::clone(&shutdown_requested);
        let (rpc_ready_tx, rpc_ready_rx) = async_channel::bounded::<()>(1);
        let command_rpc_ready = rpc_ready_rx;
        let worker_rpc_ready = rpc_ready_tx;
        let rpc_alive = Arc::new(AtomicBool::new(true));
        let command_rpc_alive = Arc::clone(&rpc_alive);
        let worker_rpc_alive = Arc::clone(&rpc_alive);
        let pending_requests: PendingRequests = Arc::new(Mutex::new(RequestRegistry {
            queued: HashMap::new(),
            pending: HashMap::new(),
        }));
        let command_pending_requests = Arc::clone(&pending_requests);
        let worker_pending_requests = Arc::clone(&pending_requests);
        let request_handlers: RpcRequestHandlers = Arc::new(Mutex::new(HashMap::new()));
        let command_request_handlers = Arc::clone(&request_handlers);
        let dispatcher_request_handlers = Arc::clone(&request_handlers);
        let worker_request_handlers = Arc::clone(&request_handlers);
        let (event_tx, events) = async_channel::bounded(EVENT_QUEUE_CAPACITY);
        let worker_tx = event_tx.clone();
        let (command_tx, command_rx) = async_channel::bounded(COMMAND_QUEUE_CAPACITY);
        let coalesced = Arc::new(Mutex::new(CoalescedCommands::default()));
        let command_coalesced = Arc::clone(&coalesced);
        let (incoming_request_tx, incoming_requests) =
            async_channel::bounded(INCOMING_REQUEST_QUEUE_CAPACITY);
        let dispatcher_writer = Arc::clone(&writer);
        let request_timeout_stop = Arc::new(AtomicBool::new(false));
        let watchdog_stop = Arc::clone(&request_timeout_stop);
        let (startup_theme_tx, startup_theme_rx) = std::sync::mpsc::sync_channel::<NvimTheme>(1);
        let (protocol_tx, protocol_rx) = std::sync::mpsc::sync_channel::<NvimProtocolInfo>(1);
        let rpc_shutdown_commands = command_tx.clone();

        thread::Builder::new()
            .name("nvim-rpc-requests".to_owned())
            .spawn(move || {
                run_request_dispatcher(
                    dispatcher_writer,
                    incoming_requests,
                    dispatcher_request_handlers,
                );
            })
            .map_err(|error| {
                stop_backend(&child, &remote);
                format!("failed to start Neovim request worker: {error}")
            })?;

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
                    command_pending_requests,
                    command_coalesced,
                );
            })
            .map_err(|error| {
                stop_backend(&child, &remote);
                format!("failed to start Neovim input worker: {error}")
            })?;

        thread::Builder::new()
            .name("nvim-rpc".to_owned())
            .spawn(move || {
                let result = run_session(
                    writer,
                    reader,
                    width,
                    height,
                    &worker_tx,
                    &worker_rpc_ready,
                    &startup_theme_tx,
                    &protocol_tx,
                    &worker_pending_requests,
                    &incoming_request_tx,
                    &worker_request_handlers,
                );
                let shutdown_requested = worker_shutdown_requested.load(Ordering::Acquire);
                let embedded_clean_exit = !shutdown_requested
                    && result
                        .as_ref()
                        .err()
                        .is_some_and(|error| error == NVIM_EXITED);
                let clean_exit = if embedded_clean_exit {
                    Some(true)
                } else {
                    child_exit_status(&worker_child).map(|status| status.success())
                };
                if let Err(error) = result.as_ref() {
                    log::error!(target: "nvim_gpui::nvim", "Neovim RPC worker failed: {error}");
                    if !shutdown_requested && error != NVIM_EXITED {
                        eprintln!("[nvim-rpc] {error}");
                        let _ = worker_tx.try_send(NvimEvent::Error(error.clone()));
                    }
                    stop_backend(&worker_child, &worker_remote);
                }

                worker_rpc_alive.store(false, Ordering::Release);
                fail_pending_requests(&worker_pending_requests, "RPC connection closed");
                let _ = rpc_shutdown_commands.try_send(NvimCommand::Shutdown);

                if let Some(child) = worker_child.as_ref() {
                    if let Ok(mut child) = child.lock() {
                        let _ = child.wait();
                    }
                }
                let reason = disconnect_reason(
                    &result,
                    shutdown_requested,
                    worker_remote.is_some(),
                    clean_exit,
                );
                log::info!(
                    target: "nvim_gpui::nvim",
                    "Neovim RPC worker stopped: reason={reason:?}"
                );
                let _ = worker_tx.send_blocking(NvimEvent::Disconnected { reason });
            })
            .map_err(|error| {
                shutdown_requested.store(true, Ordering::Release);
                stop_backend(&child, &remote);
                format!("failed to start nvim RPC worker: {error}")
            })?;

        let watchdog_pending_requests = Arc::clone(&pending_requests);
        let watchdog_rpc_alive = Arc::clone(&rpc_alive);
        let request_timeout_thread = thread::Builder::new()
            .name("nvim-rpc-timeouts".to_owned())
            .spawn(move || {
                run_pending_request_watchdog(
                    watchdog_pending_requests,
                    watchdog_rpc_alive,
                    watchdog_stop,
                );
            })
            .map_err(|error| {
                shutdown_requested.store(true, Ordering::Release);
                stop_backend(&child, &remote);
                format!("failed to start Neovim request watchdog: {error}")
            })?;

        let startup_theme = startup_theme_rx.recv_timeout(STARTUP_THEME_TIMEOUT).ok();
        let protocol = protocol_rx.recv_timeout(STARTUP_THEME_TIMEOUT).ok();
        log::debug!(
            target: "nvim_gpui::nvim",
            "Neovim startup handshake received: theme={}, protocol={}",
            startup_theme.is_some(),
            protocol.is_some()
        );

        Ok(Self {
            session_id,
            child,
            remote,
            shutdown_requested,
            commands: command_tx,
            events,
            startup_theme,
            protocol,
            connection,
            request_handlers: command_request_handlers,
            coalesced,
            request_registry: pending_requests,
            next_request_token: AtomicU64::new(1),
            request_timeout_stop,
            request_timeout_thread: Some(request_timeout_thread),
        })
    }

    pub fn events(&self) -> Receiver<NvimEvent> {
        self.events.clone()
    }

    pub(crate) fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub fn startup_theme(&self) -> Option<NvimTheme> {
        self.startup_theme
    }

    pub fn protocol(&self) -> Option<&NvimProtocolInfo> {
        self.protocol.as_ref()
    }

    pub fn version(&self) -> Option<NvimVersion> {
        self.protocol().map(|protocol| protocol.version)
    }

    pub fn register_request_handler<F>(
        &self,
        method: impl Into<String>,
        handler: F,
    ) -> Result<(), String>
    where
        F: Fn(&Value) -> Result<Value, String> + Send + Sync + 'static,
    {
        let method = method.into();
        log::debug!(target: "nvim_gpui::nvim", "registering RPC request handler: {method}");
        let methods = {
            let mut handlers = self
                .request_handlers
                .lock()
                .map_err(|_| "RPC request handler registry is poisoned".to_owned())?;
            handlers.insert(method, Arc::new(handler));
            handlers.keys().cloned().collect::<Vec<_>>()
        };
        let response = self.request("nvim_set_client_info", client_info_params(methods))?;
        drop(response);
        Ok(())
    }

    pub fn request(
        &self,
        method: impl Into<String>,
        params: Value,
    ) -> Result<Receiver<Result<Value, String>>, String> {
        let method = method.into();
        let deadline = Instant::now() + RPC_REQUEST_TIMEOUT;
        log::debug!(target: "nvim_gpui::nvim", "queueing RPC request: {method}");
        let (response_tx, response_rx) = async_channel::bounded(1);
        let token = self.next_request_token.fetch_add(1, Ordering::Relaxed);
        let request = Arc::new(RequestState::new(method, deadline, response_tx));
        {
            let mut registry = self
                .request_registry
                .lock()
                .map_err(|_| "RPC request registry is poisoned".to_owned())?;
            if registry.queued.len() + registry.pending.len() >= MAX_PENDING_REQUESTS {
                return Err(format!(
                    "Neovim RPC request limit reached ({MAX_PENDING_REQUESTS})"
                ));
            }
            registry.queued.insert(token, Arc::clone(&request));
        }
        if let Err(error) = self.commands.try_send(NvimCommand::Request {
            params,
            token,
            request,
        }) {
            if let Ok(mut registry) = self.request_registry.lock() {
                registry.queued.remove(&token);
            }
            return Err(format!("failed to queue Neovim RPC request: {error}"));
        }
        Ok(response_rx)
    }

    pub fn edit_file(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<Receiver<Result<Value, String>>, String> {
        self.request("nvim_cmd", edit_file_command_params(path.as_ref()))
    }

    pub fn send_input(&self, input: impl Into<String>) -> Result<(), String> {
        self.commands
            .try_send(NvimCommand::Input(input.into()))
            .map_err(|error| format!("failed to queue Neovim input: {error}"))
    }

    pub fn send_paste(
        &self,
        data: impl Into<String>,
    ) -> Result<Receiver<Result<Value, String>>, String> {
        let data = data.into();
        log::debug!(
            target: "nvim_gpui::input",
            "queueing Neovim paste: bytes={}",
            data.len()
        );
        self.request(
            "nvim_paste",
            Value::Array(vec![
                Value::from(data),
                Value::Boolean(false),
                Value::from(-1_i64),
            ]),
        )
    }

    pub fn send_mouse(
        &self,
        button: impl Into<String>,
        action: impl Into<String>,
        modifier: impl Into<String>,
        grid: u64,
        row: u64,
        col: u64,
    ) -> Result<(), String> {
        let button = button.into();
        let action = action.into();
        let modifier = modifier.into();
        if action == "move" {
            let mut coalesced = self
                .coalesced
                .lock()
                .map_err(|_| "Neovim coalesced command queue is poisoned".to_owned())?;
            coalesced.mouse_move = Some(MouseInput {
                button,
                action,
                modifier,
                grid,
                row,
                col,
            });
            let should_wake = !coalesced.wake_enqueued;
            coalesced.wake_enqueued = true;
            drop(coalesced);
            if should_wake {
                self.wake_coalesced_commands()
            } else {
                Ok(())
            }
        } else {
            self.commands
                .try_send(NvimCommand::Mouse {
                    button,
                    action,
                    modifier,
                    grid,
                    row,
                    col,
                })
                .map_err(|error| format!("failed to queue Neovim mouse input: {error}"))
        }
    }

    pub fn send_resize(&self, width: u32, height: u32) -> Result<(), String> {
        log::debug!(
            target: "nvim_gpui::nvim",
            "queueing Neovim resize: width={}, height={}",
            width,
            height
        );
        let mut coalesced = self
            .coalesced
            .lock()
            .map_err(|_| "Neovim coalesced command queue is poisoned".to_owned())?;
        coalesced.resize = Some((width, height));
        let should_wake = !coalesced.wake_enqueued;
        coalesced.wake_enqueued = true;
        drop(coalesced);
        if should_wake {
            self.wake_coalesced_commands()
        } else {
            Ok(())
        }
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

    fn wake_coalesced_commands(&self) -> Result<(), String> {
        match self.commands.try_send(NvimCommand::DrainCoalesced) {
            Ok(()) | Err(TrySendError::Full(_)) => Ok(()),
            Err(TrySendError::Closed(_)) => {
                if let Ok(mut coalesced) = self.coalesced.lock() {
                    coalesced.wake_enqueued = false;
                }
                Err("failed to queue Neovim coalesced input: command worker stopped".to_owned())
            }
        }
    }
}

impl Drop for NvimProcess {
    fn drop(&mut self) {
        log::debug!(target: "nvim_gpui::nvim", "shutting down Neovim connection");
        self.shutdown_requested.store(true, Ordering::Release);
        self.request_timeout_stop.store(true, Ordering::Release);
        fail_pending_requests(&self.request_registry, "RPC connection closed");
        if let Some(thread) = self.request_timeout_thread.take() {
            let _ = thread.join();
        }
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

fn child_exit_status(child: &Option<Arc<Mutex<Child>>>) -> Option<ExitStatus> {
    child
        .as_ref()
        .and_then(|child| child.lock().ok())
        .and_then(|mut child| child.try_wait().ok().flatten())
}

fn disconnect_reason(
    result: &Result<(), String>,
    shutdown_requested: bool,
    is_remote: bool,
    clean_exit: Option<bool>,
) -> DisconnectReason {
    if shutdown_requested {
        return DisconnectReason::Requested;
    }
    if let Err(error) = result {
        if error != NVIM_EXITED {
            return DisconnectReason::ProtocolError(error.clone());
        }
        if is_remote {
            // A remote Neovim server closes the RPC channel with an orderly
            // EOF when it exits (for example after `:q!`). Treat that EOF as
            // the server's clean exit so the frontend closes instead of
            // endlessly reconnecting to a server that no longer exists.
            return DisconnectReason::CleanExit;
        }
    }
    if is_remote {
        DisconnectReason::TransportClosed
    } else if clean_exit == Some(true) {
        DisconnectReason::CleanExit
    } else {
        DisconnectReason::UnexpectedExit
    }
}

fn fail_pending_requests(pending: &PendingRequests, error: &str) {
    let requests = match pending.lock() {
        Ok(mut pending) => {
            let mut requests = pending
                .queued
                .drain()
                .map(|(_, request)| request)
                .collect::<Vec<_>>();
            requests.extend(pending.pending.drain().map(|(_, request)| request));
            requests
        }
        Err(_) => return,
    };
    for request in requests {
        request.complete(Err(error.to_owned()));
    }
}

fn run_pending_request_watchdog(
    pending_requests: PendingRequests,
    rpc_alive: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Acquire) && rpc_alive.load(Ordering::Acquire) {
        thread::sleep(RPC_TIMEOUT_POLL_INTERVAL);
        let now = Instant::now();
        let expired = match pending_requests.lock() {
            Ok(mut pending) => {
                let mut expired = Vec::new();
                pending.queued.retain(|_, request| {
                    if request.deadline <= now {
                        expired.push(Arc::clone(request));
                        false
                    } else {
                        true
                    }
                });
                pending.pending.retain(|_, request| {
                    if request.deadline <= now {
                        expired.push(Arc::clone(request));
                        false
                    } else {
                        true
                    }
                });
                expired
            }
            Err(_) => return,
        };
        for request in expired {
            log::warn!(
                target: "nvim_gpui::nvim",
                "Neovim RPC request timed out: method={}",
                request.method
            );
            request.complete(Err(format!(
                "Neovim RPC request timed out after {RPC_REQUEST_TIMEOUT:?}"
            )));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_command_writer(
    writer: SharedWriter,
    commands: Receiver<NvimCommand>,
    events: Sender<NvimEvent>,
    shutdown_requested: Arc<AtomicBool>,
    rpc_ready: Receiver<()>,
    rpc_alive: Arc<AtomicBool>,
    pending_requests: PendingRequests,
    coalesced: Arc<Mutex<CoalescedCommands>>,
) {
    let mut request_id = 1_000_000;

    if rpc_ready.recv_blocking().is_err() {
        return;
    }

    loop {
        if shutdown_requested.load(Ordering::Acquire) || !rpc_alive.load(Ordering::Acquire) {
            return;
        }

        let command = match commands.recv_blocking() {
            Ok(command) => command,
            Err(_) => return,
        };

        if matches!(&command, NvimCommand::DrainCoalesced) {
            if let Err(error) = flush_coalesced_commands(&writer, &coalesced, &mut request_id) {
                log::error!(target: "nvim_gpui::nvim", "failed to write coalesced Neovim command: {error}");
                rpc_alive.store(false, Ordering::Release);
                let _ = events.try_send(NvimEvent::Error(error));
                return;
            }
            continue;
        }

        let (message, pending_response) = match command {
            NvimCommand::Input(input) => (
                Value::Array(vec![
                    Value::from(2),
                    Value::from("nvim_input"),
                    Value::Array(vec![Value::from(input)]),
                ]),
                None,
            ),
            NvimCommand::Mouse {
                button,
                action,
                modifier,
                grid,
                row,
                col,
            } => (
                mouse_event_notification_frame(button, action, modifier, grid, row, col),
                None,
            ),
            NvimCommand::Request {
                params,
                token,
                request,
            } => {
                let queued_request = match pending_requests.lock() {
                    Ok(mut pending) => pending.queued.remove(&token),
                    Err(_) => {
                        request.complete(Err("RPC request registry is poisoned".to_owned()));
                        return;
                    }
                };
                let Some(request) = queued_request else {
                    continue;
                };
                if request.deadline <= Instant::now() {
                    request.complete(Err(format!(
                        "Neovim RPC request timed out before send: {}",
                        request.method
                    )));
                    continue;
                }
                let pending_count = match pending_requests.lock() {
                    Ok(pending) => pending.pending.len(),
                    Err(_) => {
                        request.complete(Err("RPC request registry is poisoned".to_owned()));
                        return;
                    }
                };
                if pending_count >= MAX_PENDING_REQUESTS {
                    request.complete(Err(format!(
                        "Neovim RPC request limit reached ({MAX_PENDING_REQUESTS})"
                    )));
                    continue;
                }
                let id = request_id;
                request_id += 1;
                let message = Value::Array(vec![
                    Value::from(0),
                    Value::from(id),
                    Value::from(request.method.clone()),
                    params,
                ]);
                (message, Some((id, request)))
            }
            NvimCommand::TermEvent { event, value } => {
                (term_event_notification_frame(event, value), None)
            }
            NvimCommand::DrainCoalesced => continue,
            NvimCommand::Shutdown => return,
        };
        if let Some((id, response)) = pending_response.as_ref() {
            match pending_requests.lock() {
                Ok(mut pending) => {
                    pending.pending.insert(*id, Arc::clone(response));
                }
                Err(_) => {
                    response.complete(Err("RPC request registry is poisoned".to_owned()));
                    let _ = events.try_send(NvimEvent::Error(
                        "RPC request registry is poisoned".to_owned(),
                    ));
                    rpc_alive.store(false, Ordering::Release);
                    fail_pending_requests(&pending_requests, "RPC writer stopped");
                    return;
                }
            }
        }
        if let Err(error) = write_shared_message(&writer, &message) {
            log::error!(target: "nvim_gpui::nvim", "failed to write Neovim RPC message: {error}");
            if let Some((id, response)) = pending_response {
                if let Ok(mut pending) = pending_requests.lock() {
                    pending.pending.remove(&id);
                }
                response.complete(Err(error.clone()));
            }
            if !shutdown_requested.load(Ordering::Acquire) {
                let _ = events.try_send(NvimEvent::Error(error));
            }
            rpc_alive.store(false, Ordering::Release);
            fail_pending_requests(&pending_requests, "RPC writer stopped");
            return;
        }

        if commands.is_empty() {
            if let Err(error) = flush_coalesced_commands(&writer, &coalesced, &mut request_id) {
                log::error!(target: "nvim_gpui::nvim", "failed to write coalesced Neovim command: {error}");
                rpc_alive.store(false, Ordering::Release);
                let _ = events.try_send(NvimEvent::Error(error));
                fail_pending_requests(&pending_requests, "RPC writer stopped");
                return;
            }
        }
    }
}

fn flush_coalesced_commands(
    writer: &SharedWriter,
    coalesced: &Arc<Mutex<CoalescedCommands>>,
    request_id: &mut u64,
) -> Result<(), String> {
    let (resize, mouse_move) = {
        let mut coalesced = coalesced
            .lock()
            .map_err(|_| "Neovim coalesced command queue is poisoned".to_owned())?;
        let commands = (coalesced.resize.take(), coalesced.mouse_move.take());
        coalesced.wake_enqueued = false;
        commands
    };

    if let Some((width, height)) = resize {
        let message = resize_request_frame(*request_id, width, height);
        *request_id += 1;
        write_shared_message(writer, &message)
            .map_err(|error| format!("failed to write Neovim resize: {error}"))?;
    }
    if let Some(mouse) = mouse_move {
        let message = mouse_event_notification_frame(
            mouse.button,
            mouse.action,
            mouse.modifier,
            mouse.grid,
            mouse.row,
            mouse.col,
        );
        write_shared_message(writer, &message)
            .map_err(|error| format!("failed to write Neovim mouse move: {error}"))?;
    }
    Ok(())
}
