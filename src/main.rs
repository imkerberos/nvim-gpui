pub mod app;
mod clipboard;
pub mod grid;
pub(crate) mod gui;
pub mod helper;
pub mod image_store;
pub mod input;
mod logging;
pub mod nvim;
pub mod platform;
pub mod settings;
pub(crate) mod widgets;

use std::{env, ffi::OsString, fs, path::PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CliAction {
    Run(CliOptions),
    Help,
    Version,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum NvimConnection {
    Embed,
    Remote(String),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CliOptions {
    debug_window: bool,
    connection: NvimConnection,
    nvim_command: Option<OsString>,
    working_directory: Option<OsString>,
    nvim_args: Vec<OsString>,
}

fn parse_cli<I>(args: I) -> Result<CliAction, String>
where
    I: IntoIterator<Item = OsString>,
{
    let mut debug_window = false;
    let mut connection = NvimConnection::Embed;
    let mut explicit_embed = false;
    let mut nvim_command = None;
    let mut working_directory = None;
    let mut nvim_args = Vec::new();
    let mut pass_through = false;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if !pass_through {
            match arg.to_str() {
                Some("--help") | Some("-h") => return Ok(CliAction::Help),
                Some("--version") | Some("-V") => return Ok(CliAction::Version),
                Some("--debug-window") => {
                    debug_window = true;
                    continue;
                }
                Some("--no-debug-window") => {
                    debug_window = false;
                    continue;
                }
                Some("--embed") => {
                    explicit_embed = true;
                    continue;
                }
                Some("--connect") => {
                    let address = args
                        .next()
                        .ok_or_else(|| "--connect requires an address".to_owned())?;
                    let address = address
                        .into_string()
                        .map_err(|_| "--connect address must be valid UTF-8".to_owned())?;
                    connection = NvimConnection::Remote(address);
                    continue;
                }
                Some(value) if value.starts_with("--connect=") => {
                    let address = value.trim_start_matches("--connect=");
                    if address.is_empty() {
                        return Err("--connect requires an address".to_owned());
                    }
                    connection = NvimConnection::Remote(address.to_owned());
                    continue;
                }
                Some("--nvim-command") => {
                    nvim_command = Some(
                        args.next()
                            .ok_or_else(|| "--nvim-command requires a path".to_owned())?,
                    );
                    continue;
                }
                Some(value) if value.starts_with("--nvim-command=") => {
                    let command = value.trim_start_matches("--nvim-command=");
                    if command.is_empty() {
                        return Err("--nvim-command requires a path".to_owned());
                    }
                    nvim_command = Some(OsString::from(command));
                    continue;
                }
                Some("--cwd") | Some("--working-directory") => {
                    working_directory = Some(
                        args.next()
                            .ok_or_else(|| "--cwd requires a path".to_owned())?,
                    );
                    continue;
                }
                Some(value) if value.starts_with("--cwd=") => {
                    let path = value.trim_start_matches("--cwd=");
                    if path.is_empty() {
                        return Err("--cwd requires a path".to_owned());
                    }
                    working_directory = Some(OsString::from(path));
                    continue;
                }
                Some(value) if value.starts_with("--working-directory=") => {
                    let path = value.trim_start_matches("--working-directory=");
                    if path.is_empty() {
                        return Err("--cwd requires a path".to_owned());
                    }
                    working_directory = Some(OsString::from(path));
                    continue;
                }
                Some("--") => {
                    pass_through = true;
                    continue;
                }
                _ => {}
            }
        }
        nvim_args.push(arg);
    }

    if explicit_embed && matches!(connection, NvimConnection::Remote(_)) {
        return Err("--embed and --connect cannot be used together".to_owned());
    }
    if matches!(connection, NvimConnection::Remote(_))
        && (nvim_command.is_some() || !nvim_args.is_empty())
    {
        return Err(
            "Neovim arguments and --nvim-command are only valid with embed mode".to_owned(),
        );
    }

    Ok(CliAction::Run(CliOptions {
        debug_window,
        connection,
        nvim_command,
        working_directory,
        nvim_args,
    }))
}

fn print_help() {
    println!(
        "Usage: gpvim [GPUI options] [--] [Neovim options]\n\n\
GPUI options:\n  --debug-window       Show the auxiliary debug window (opt-in)\n  --no-debug-window    Hide the auxiliary debug window\n  --embed              Start a local embedded Neovim (default)\n  --connect ADDRESS    Connect to a Neovim msgpack-rpc socket\n  --nvim-command PATH  Select the local Neovim executable for embed mode\n  --cwd PATH           Set the working directory for Neovim\n  -h, --help           Show this help\n  -V, --version        Show the GPUI version\n\n\
ADDRESS may be HOST:PORT, tcp:HOST:PORT, unix:/path, or a Unix socket path.\nAll other arguments are passed to embedded Neovim. Use -- to pass an argument\nthat would otherwise be interpreted as a GPUI option."
    );
}

#[cfg(target_os = "macos")]
fn app_bundle_working_directory() -> Option<PathBuf> {
    let executable = env::current_exe().ok()?;
    let executable = fs::canonicalize(&executable).unwrap_or(executable);
    executable
        .ancestors()
        .find(|path| path.extension().and_then(|extension| extension.to_str()) == Some("app"))
        .map(|application| application.join("Contents/MacOS"))
}

#[cfg(not(target_os = "macos"))]
fn app_bundle_working_directory() -> Option<PathBuf> {
    None
}

fn main() {
    let options = match parse_cli(env::args_os().skip(1)) {
        Ok(CliAction::Run(options)) => options,
        Ok(CliAction::Help) => {
            print_help();
            return;
        }
        Ok(CliAction::Version) => {
            println!("nvim-gpui {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Err(error) => {
            eprintln!("gpvim: {error}");
            print_help();
            return;
        }
    };

    let app_settings = settings::Settings::load();
    if !app_settings.allow_multiple_instances && platform::activate_existing_instance() {
        return;
    }

    let logger = match logging::init(app_settings.log_level) {
        Ok(logger) => Some(logger),
        Err(error) => {
            eprintln!("[logging] {error}");
            None
        }
    };
    log::info!(
        target: "nvim_gpui::startup",
        "starting nvim-gpui (debug_window={}, connection={:?}, nvim_args={})",
        options.debug_window,
        options.connection,
        options.nvim_args.len()
    );

    if let Err(error) = helper::ensure_installed() {
        log::warn!(target: "nvim_gpui::startup", "installation check failed: {error}");
        eprintln!("[gpvim] {error}");
    }

    if let Some(path) = options.working_directory.as_deref() {
        if let Err(error) = env::set_current_dir(path) {
            log::error!(
                target: "nvim_gpui::startup",
                "failed to set working directory {}: {error}",
                path.to_string_lossy()
            );
            eprintln!("gpvim: failed to set working directory: {error}");
            return;
        }
        log::debug!(
            target: "nvim_gpui::startup",
            "working directory set to {}",
            path.to_string_lossy()
        );
    } else if let Some(path) = app_bundle_working_directory() {
        if let Err(error) = env::set_current_dir(&path) {
            log::error!(
                target: "nvim_gpui::startup",
                "failed to set AppBundle working directory {}: {error}",
                path.display()
            );
            eprintln!(
                "gpvim: failed to set AppBundle working directory {}: {error}",
                path.display()
            );
            return;
        }
        log::debug!(
            target: "nvim_gpui::startup",
            "AppBundle working directory set to {}",
            path.display()
        );
    }

    app::run(options, app_settings, logger);
}
