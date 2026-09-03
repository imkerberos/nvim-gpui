//! Minimal Neovim MessagePack-RPC bootstrap.
//!
//! It starts `nvim --embed` with the caller's remaining command-line
//! arguments, identifies the client, attaches a small line-grid UI, and
//! forwards line-grid redraw events and queued input to GPUI/Neovim.
//! Richer UI capabilities will be layered on top of these events later.

use async_channel::{Receiver, Sender};
use rmpv::Value;
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::process::{Child, Command, Stdio};
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
use protocol::{resize_request_frame, term_event_notification_frame};
use session::run_session;
use transport::{connect_remote, write_shared_message, RemoteConnection, SharedWriter};
use types::NvimCommand;
pub use types::{NvimEvent, NvimTheme};
use version::parse_protocol_info;
pub use version::{NvimCapabilities, NvimProtocolInfo, NvimVersion};

const CLIENT_NAME: &str = "nvim-gpui";
const NVIM_GPUI_STARTUP_COMMAND: &str = "let g:nvim_gpui = v:true";
const NVIM_EXITED: &str = "nvim process exited";
const STARTUP_THEME_TIMEOUT: Duration = Duration::from_secs(1);

pub struct NvimProcess {
    child: Option<Arc<Mutex<Child>>>,
    remote: Option<Arc<RemoteConnection>>,
    shutdown_requested: Arc<AtomicBool>,
    commands: Sender<NvimCommand>,
    events: Receiver<NvimEvent>,
    startup_theme: Option<NvimTheme>,
    protocol: Option<NvimProtocolInfo>,
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
        command
            .args(["--embed", "--cmd", NVIM_GPUI_STARTUP_COMMAND])
            .args(nvim_args);
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
        let (rpc_ready_tx, rpc_ready_rx) = async_channel::bounded::<()>(1);
        let command_rpc_ready = rpc_ready_rx;
        let worker_rpc_ready = rpc_ready_tx;
        let rpc_alive = Arc::new(AtomicBool::new(true));
        let command_rpc_alive = Arc::clone(&rpc_alive);
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
                );
                if let Err(error) = result {
                    if !worker_shutdown_requested.load(Ordering::Acquire) && error != NVIM_EXITED {
                        eprintln!("[nvim-rpc] {error}");
                        let _ = worker_tx.send_blocking(NvimEvent::Error(error));
                    }
                    stop_backend(&worker_child, &worker_remote);
                }

                rpc_alive.store(false, Ordering::Release);
                let _ = rpc_shutdown_commands.send_blocking(NvimCommand::Shutdown);

                if let Some(child) = worker_child.as_ref() {
                    if let Ok(mut child) = child.lock() {
                        let _ = child.wait();
                    }
                }
                let _ = worker_tx.send_blocking(NvimEvent::Disconnected);
            })
            .map_err(|error| {
                shutdown_requested.store(true, Ordering::Release);
                stop_backend(&child, &remote);
                format!("failed to start nvim RPC worker: {error}")
            })?;

        let startup_theme = startup_theme_rx.recv_timeout(STARTUP_THEME_TIMEOUT).ok();
        let protocol = protocol_rx.recv_timeout(STARTUP_THEME_TIMEOUT).ok();

        Ok(Self {
            child,
            remote,
            shutdown_requested,
            commands: command_tx,
            events,
            startup_theme,
            protocol,
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
    rpc_ready: Receiver<()>,
    rpc_alive: Arc<AtomicBool>,
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
            NvimCommand::Shutdown => return,
        };
        if let Err(error) = write_shared_message(&writer, &message) {
            if !shutdown_requested.load(Ordering::Acquire) {
                let _ = events.send_blocking(NvimEvent::Error(error));
            }
            return;
        }
    }
}
