//! Minimal Neovim MessagePack-RPC bootstrap.
//!
//! It starts `nvim --embed` with the caller's remaining command-line
//! arguments, identifies the client, attaches a small line-grid UI, and
//! forwards line-grid redraw events and queued input to GPUI/Neovim.
//! Richer UI capabilities will be layered on top of these events later.

use async_channel::{Receiver, Sender};
use rmpv::Value;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::Duration;

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
    client_info_params, mouse_event_notification_frame, resize_request_frame,
    term_event_notification_frame,
};
use session::run_session;
use transport::{connect_remote, write_shared_message, RemoteConnection, SharedWriter};
use types::NvimCommand;
pub use types::{DisconnectReason, NvimEvent, NvimTheme};
use version::parse_protocol_info;
pub use version::{NvimCapabilities, NvimProtocolInfo, NvimVersion};

const CLIENT_NAME: &str = "nvim-gpui";
const NVIM_GPUI_STARTUP_COMMAND: &str = "let g:nvim_gpui = v:true";
const NVIM_EXITED: &str = "nvim process exited";
const STARTUP_THEME_TIMEOUT: Duration = Duration::from_secs(1);

type PendingRequests = Arc<Mutex<HashMap<u64, Sender<Result<Value, String>>>>>;
pub type RpcRequestHandler = Arc<dyn Fn(&Value) -> Result<Value, String> + Send + Sync + 'static>;
type RpcRequestHandlers = Arc<Mutex<HashMap<String, RpcRequestHandler>>>;

#[derive(Clone)]
pub(crate) enum ConnectionSpec {
    Embedded {
        command: OsString,
        args: Vec<OsString>,
    },
    Remote {
        address: String,
    },
}

pub struct NvimProcess {
    child: Option<Arc<Mutex<Child>>>,
    remote: Option<Arc<RemoteConnection>>,
    shutdown_requested: Arc<AtomicBool>,
    commands: Sender<NvimCommand>,
    events: Receiver<NvimEvent>,
    startup_theme: Option<NvimTheme>,
    protocol: Option<NvimProtocolInfo>,
    connection: ConnectionSpec,
    request_handlers: RpcRequestHandlers,
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
        log::info!(
            target: "nvim_gpui::nvim",
            "connecting to remote Neovim address={address}"
        );
        let (reader, writer, remote) = connect_remote(address)?;
        let writer: SharedWriter = Arc::new(Mutex::new(writer));
        Self::start_workers(
            width,
            height,
            writer,
            reader,
            None,
            Some(Arc::new(remote)),
            ConnectionSpec::Remote {
                address: address.to_owned(),
            },
        )
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

    pub(crate) fn connect_from_spec(
        connection: &ConnectionSpec,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let process = match connection {
            ConnectionSpec::Embedded { command, args } => {
                Self::spawn_with_command(width, height, command, args.clone())
            }
            ConnectionSpec::Remote { address } => Self::connect(width, height, address),
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
        let pending_requests: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let command_pending_requests = Arc::clone(&pending_requests);
        let worker_pending_requests = Arc::clone(&pending_requests);
        let request_handlers: RpcRequestHandlers = Arc::new(Mutex::new(HashMap::new()));
        let command_request_handlers = Arc::clone(&request_handlers);
        let worker_request_handlers = Arc::clone(&request_handlers);
        let (event_tx, events) = async_channel::unbounded();
        let worker_tx = event_tx.clone();
        let (command_tx, command_rx) = async_channel::unbounded();
        let (startup_theme_tx, startup_theme_rx) = std::sync::mpsc::sync_channel::<NvimTheme>(1);
        let (protocol_tx, protocol_rx) = std::sync::mpsc::sync_channel::<NvimProtocolInfo>(1);
        let rpc_shutdown_commands = command_tx.clone();

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
                    &worker_request_handlers,
                );
                let shutdown_requested = worker_shutdown_requested.load(Ordering::Acquire);
                let clean_exit = if !shutdown_requested
                    && result
                        .as_ref()
                        .err()
                        .is_some_and(|error| error == NVIM_EXITED)
                {
                    wait_for_child_exit(&worker_child).map(|status| status.success())
                } else {
                    child_exit_status(&worker_child).map(|status| status.success())
                };
                if let Err(error) = result.as_ref() {
                    log::error!(target: "nvim_gpui::nvim", "Neovim RPC worker failed: {error}");
                    if !shutdown_requested && error != NVIM_EXITED {
                        eprintln!("[nvim-rpc] {error}");
                        let _ = worker_tx.send_blocking(NvimEvent::Error(error.clone()));
                    }
                    stop_backend(&worker_child, &worker_remote);
                }

                rpc_alive.store(false, Ordering::Release);
                fail_pending_requests(&worker_pending_requests, "RPC connection closed");
                let _ = rpc_shutdown_commands.send_blocking(NvimCommand::Shutdown);

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

        let startup_theme = startup_theme_rx.recv_timeout(STARTUP_THEME_TIMEOUT).ok();
        let protocol = protocol_rx.recv_timeout(STARTUP_THEME_TIMEOUT).ok();
        log::debug!(
            target: "nvim_gpui::nvim",
            "Neovim startup handshake received: theme={}, protocol={}",
            startup_theme.is_some(),
            protocol.is_some()
        );

        Ok(Self {
            child,
            remote,
            shutdown_requested,
            commands: command_tx,
            events,
            startup_theme,
            protocol,
            connection,
            request_handlers: command_request_handlers,
        })
    }

    pub fn events(&self) -> Receiver<NvimEvent> {
        self.events.clone()
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
        log::debug!(target: "nvim_gpui::nvim", "queueing RPC request: {method}");
        let (response_tx, response_rx) = async_channel::bounded(1);
        self.commands
            .try_send(NvimCommand::Request {
                method,
                params,
                response: response_tx,
            })
            .map_err(|error| format!("failed to queue Neovim RPC request: {error}"))?;
        Ok(response_rx)
    }

    pub fn send_input(&self, input: impl Into<String>) -> Result<(), String> {
        self.commands
            .try_send(NvimCommand::Input(input.into()))
            .map_err(|error| format!("failed to queue Neovim input: {error}"))
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
        self.commands
            .try_send(NvimCommand::Mouse {
                button: button.into(),
                action: action.into(),
                modifier: modifier.into(),
                grid,
                row,
                col,
            })
            .map_err(|error| format!("failed to queue Neovim mouse input: {error}"))
    }

    pub fn send_resize(&self, width: u32, height: u32) -> Result<(), String> {
        log::debug!(
            target: "nvim_gpui::nvim",
            "queueing Neovim resize: width={}, height={}",
            width,
            height
        );
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
        log::debug!(target: "nvim_gpui::nvim", "shutting down Neovim connection");
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

fn child_exit_status(child: &Option<Arc<Mutex<Child>>>) -> Option<ExitStatus> {
    child
        .as_ref()
        .and_then(|child| child.lock().ok())
        .and_then(|mut child| child.try_wait().ok().flatten())
}

fn wait_for_child_exit(child: &Option<Arc<Mutex<Child>>>) -> Option<ExitStatus> {
    for _ in 0..50 {
        if let Some(status) = child_exit_status(child) {
            return Some(status);
        }
        thread::sleep(Duration::from_millis(10));
    }
    child_exit_status(child)
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
        Ok(mut pending) => pending
            .drain()
            .map(|(_, sender)| sender)
            .collect::<Vec<_>>(),
        Err(_) => return,
    };
    for sender in requests {
        let _ = sender.send_blocking(Err(error.to_owned()));
    }
}

fn run_command_writer(
    writer: SharedWriter,
    commands: Receiver<NvimCommand>,
    events: Sender<NvimEvent>,
    shutdown_requested: Arc<AtomicBool>,
    rpc_ready: Receiver<()>,
    rpc_alive: Arc<AtomicBool>,
    pending_requests: PendingRequests,
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
            NvimCommand::Resize { width, height } => {
                let message = resize_request_frame(request_id, width, height);
                request_id += 1;
                (message, None)
            }
            NvimCommand::Request {
                method,
                params,
                response,
            } => {
                let id = request_id;
                request_id += 1;
                let message = Value::Array(vec![
                    Value::from(0),
                    Value::from(id),
                    Value::from(method),
                    params,
                ]);
                (message, Some((id, response)))
            }
            NvimCommand::TermEvent { event, value } => {
                (term_event_notification_frame(event, value), None)
            }
            NvimCommand::Shutdown => return,
        };
        if let Some((id, response)) = pending_response.as_ref() {
            match pending_requests.lock() {
                Ok(mut pending) => {
                    pending.insert(*id, response.clone());
                }
                Err(_) => {
                    let _ =
                        response.send_blocking(Err("RPC request registry is poisoned".to_owned()));
                    let _ = events.send_blocking(NvimEvent::Error(
                        "RPC request registry is poisoned".to_owned(),
                    ));
                    return;
                }
            }
        }
        if let Err(error) = write_shared_message(&writer, &message) {
            log::error!(target: "nvim_gpui::nvim", "failed to write Neovim RPC message: {error}");
            if let Some((id, response)) = pending_response {
                if let Ok(mut pending) = pending_requests.lock() {
                    pending.remove(&id);
                }
                let _ = response.send_blocking(Err(error.clone()));
            }
            if !shutdown_requested.load(Ordering::Acquire) {
                let _ = events.send_blocking(NvimEvent::Error(error));
            }
            return;
        }
    }
}
