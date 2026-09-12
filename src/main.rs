#![cfg_attr(all(target_os = "windows", not(test)), windows_subsystem = "windows")]

pub mod app;
mod editor;
pub mod grid;
pub(crate) mod gui;
mod health_check;
pub mod helper;
pub mod input;
mod logging;
pub mod nvim;
pub mod platform;
pub mod settings;
mod startup_diagnostics;
mod update_check;
pub(crate) mod widgets;

use std::{env, ffi::OsString, path::PathBuf, process::ExitCode, time::Duration};

#[cfg(target_os = "macos")]
use std::fs;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CliAction {
    Run(CliOptions),
    Help,
    Version,
    HealthCheck,
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
    connect_timeout: Duration,
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
    let mut connect_timeout = None;
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
                Some("--health-check") => return Ok(CliAction::HealthCheck),
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
                Some("--connect-timeout") => {
                    let timeout = args
                        .next()
                        .ok_or_else(|| "--connect-timeout requires seconds".to_owned())?;
                    let timeout = timeout
                        .into_string()
                        .map_err(|_| "--connect-timeout must be valid UTF-8".to_owned())?;
                    connect_timeout = Some(parse_connect_timeout(&timeout)?);
                    continue;
                }
                Some(value) if value.starts_with("--connect-timeout=") => {
                    let timeout = value.trim_start_matches("--connect-timeout=");
                    if timeout.is_empty() {
                        return Err("--connect-timeout requires seconds".to_owned());
                    }
                    connect_timeout = Some(parse_connect_timeout(timeout)?);
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
                Some(value) if value.starts_with('-') => {
                    return Err(format!(
                        "unknown nvim-gpui option: {value}; pass Neovim options after `--`"
                    ));
                }
                _ => {}
            }
        }
        nvim_args.push(arg);
    }

    if explicit_embed && matches!(connection, NvimConnection::Remote(_)) {
        return Err("--embed and --connect cannot be used together".to_owned());
    }
    if connect_timeout.is_some() && matches!(connection, NvimConnection::Embed) {
        return Err("--connect-timeout requires --connect".to_owned());
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
        connect_timeout: connect_timeout.unwrap_or(nvim::DEFAULT_CONNECT_TIMEOUT),
        nvim_command,
        working_directory,
        nvim_args,
    }))
}

fn parse_connect_timeout(value: &str) -> Result<Duration, String> {
    let seconds = value
        .parse::<f64>()
        .map_err(|_| format!("--connect-timeout must be a positive number of seconds: {value}"))?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(format!(
            "--connect-timeout must be a positive number of seconds: {value}"
        ));
    }
    Duration::try_from_secs_f64(seconds)
        .map_err(|_| format!("--connect-timeout is outside the supported range: {value}"))
}

fn print_help() {
    println!(
        "Usage: gpvim [GPUI options] [--] [Neovim options]\n\n\
GPUI options:\n  --debug-window             Show the auxiliary debug window (opt-in)\n  --no-debug-window          Hide the auxiliary debug window\n  --embed                    Start a local embedded Neovim (default)\n  --connect ADDRESS          Connect to a Neovim msgpack-rpc socket\n  --connect-timeout SECONDS  Set the remote TCP connection timeout (default: 3)\n  --nvim-command PATH        Select the local Neovim executable for embed mode\n  --cwd PATH                 Set the working directory for Neovim\n  --health-check             Report OS and graphics capabilities, then exit\n  -h, --help                 Show this help\n  -V, --version              Show the GPUI version\n\n\
ADDRESS may be HOST:PORT, tcp:HOST:PORT, unix:/path, or a Unix socket path.\nSECONDS must be a positive number and may include decimals. The timeout applies\nto remote TCP connections; Unix socket connections are not affected. Non-option\narguments are passed to embedded Neovim. Neovim options must be placed after\n--, for example: `gpvim -- --clean`."
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

fn main() -> ExitCode {
    let options = match parse_cli(env::args_os().skip(1)) {
        Ok(CliAction::Run(options)) => options,
        Ok(CliAction::Help) => {
            print_help();
            return ExitCode::SUCCESS;
        }
        Ok(CliAction::Version) => {
            println!("nvim-gpui {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Ok(CliAction::HealthCheck) => {
            health_check::run();
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("gpvim: {error}");
            print_help();
            return ExitCode::from(2);
        }
    };

    let app_settings = settings::Settings::load();
    if !app_settings.allow_multiple_instances && platform::activate_existing_instance() {
        return ExitCode::SUCCESS;
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
            return ExitCode::from(1);
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
            return ExitCode::from(1);
        }
        log::debug!(
            target: "nvim_gpui::startup",
            "AppBundle working directory set to {}",
            path.display()
        );
    }

    if app::run(options, app_settings, logger) {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn rejects_unknown_options_before_neovim_separator() {
        let error = parse_cli(args(&["--helth-check"])).expect_err("typo must be rejected");

        assert!(error.contains("unknown nvim-gpui option: --helth-check"));
        assert!(error.contains("pass Neovim options after `--`"));
    }

    #[test]
    fn passes_neovim_options_after_separator() {
        let CliAction::Run(options) = parse_cli(args(&["--", "--clean", "file.txt"]))
            .expect("Neovim options after -- should be accepted")
        else {
            panic!("expected a run action");
        };

        assert_eq!(
            options.nvim_args,
            vec![OsString::from("--clean"), OsString::from("file.txt")]
        );
    }

    #[test]
    fn passes_file_paths_without_separator() {
        let CliAction::Run(options) =
            parse_cli(args(&["file.txt"])).expect("file paths should be accepted without --")
        else {
            panic!("expected a run action");
        };

        assert_eq!(options.nvim_args, vec![OsString::from("file.txt")]);
    }
}
