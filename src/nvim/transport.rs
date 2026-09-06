use rmpv::Value;
use std::io::{BufReader, ErrorKind, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::net::UnixStream;

use super::NVIM_EXITED;

pub(super) type RpcReader = BufReader<Box<dyn Read + Send>>;
pub(super) type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

pub(super) enum RemoteConnection {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
}

impl RemoteConnection {
    pub(super) fn shutdown(&self) {
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

pub(super) type RpcStreams = (
    Box<dyn Read + Send>,
    Box<dyn Write + Send>,
    RemoteConnection,
);

pub(super) fn connect_remote_with_timeout(
    address: &str,
    connect_timeout: Duration,
) -> Result<RpcStreams, String> {
    if let Some(address) = address
        .strip_prefix("tcp://")
        .or_else(|| address.strip_prefix("tcp:"))
    {
        return connect_tcp(address, connect_timeout);
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

    connect_tcp(address, connect_timeout)
}

fn connect_tcp(address: &str, connect_timeout: Duration) -> Result<RpcStreams, String> {
    let mut last_error = None;
    let addresses = address
        .to_socket_addrs()
        .map_err(|error| format!("failed to resolve Neovim address {address}: {error}"))?;

    for socket_address in addresses {
        match TcpStream::connect_timeout(&socket_address, connect_timeout) {
            Ok(stream) => return tcp_streams(stream),
            Err(error) => last_error = Some(error),
        }
    }

    let error = last_error
        .map(|error| error.to_string())
        .unwrap_or_else(|| "address resolved to no endpoints".to_owned());
    Err(format!(
        "failed to connect to Neovim at {address} within {connect_timeout:?}: {error}"
    ))
}

fn tcp_streams(stream: TcpStream) -> Result<RpcStreams, String> {
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

pub(crate) fn read_message(reader: &mut impl Read) -> Result<Value, String> {
    rmpv::decode::read_value(reader).map_err(|error| {
        if error.kind() == ErrorKind::UnexpectedEof {
            NVIM_EXITED.to_owned()
        } else {
            format!("failed to decode RPC message: {error}")
        }
    })
}

pub(super) fn write_shared_message(writer: &SharedWriter, message: &Value) -> Result<(), String> {
    let mut writer = writer
        .lock()
        .map_err(|_| "failed to lock Neovim RPC writer".to_owned())?;
    write_message(&mut *writer, message)
}

pub(crate) fn write_message(writer: &mut impl Write, message: &Value) -> Result<(), String> {
    rmpv::encode::write_value(writer, message)
        .map_err(|error| format!("failed to encode RPC message: {error}"))?;
    writer
        .flush()
        .map_err(|error| format!("failed to flush RPC message: {error}"))
}
