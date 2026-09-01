pub mod app;
pub mod grid;
pub mod image_store;
pub mod input;
pub mod nvim;
pub mod platform;

use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::symlink;

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

fn gpvim_is_available_in_path() -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    let command_name = if cfg!(windows) { "gpvim.exe" } else { "gpvim" };
    env::split_paths(&path).any(|directory| is_executable_path(&directory.join(command_name)))
}

fn is_executable_path(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
}

fn bundled_gpvim_path() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(application) = env::var_os("NVIM_GPUI_APP") {
        candidates.push(PathBuf::from(application).join("Contents/Resources/gpvim"));
    }
    if let Ok(executable) = env::current_exe() {
        let executable = fs::canonicalize(&executable).unwrap_or(executable);
        candidates.extend(
            executable
                .ancestors()
                .filter(|path| {
                    path.extension().and_then(|extension| extension.to_str()) == Some("app")
                })
                .map(|application| application.join("Contents/Resources/gpvim")),
        );
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".cache/macos/nvim-gpui.app/Contents/Resources/gpvim"),
    );
    candidates.into_iter().find(|path| is_executable_path(path))
}

fn ensure_gpvim_helper() -> Result<(), String> {
    if gpvim_is_available_in_path() {
        return Ok(());
    }

    let Some(helper) = bundled_gpvim_path() else {
        return Ok(());
    };

    #[cfg(unix)]
    {
        let link = Path::new("/usr/local/bin/gpvim");
        if fs::symlink_metadata(link).is_ok() {
            return Err(format!(
                "gpvim is not executable from PATH and {} already exists",
                link.display()
            ));
        }
        if let Some(parent) = link.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "could not create gpvim helper directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        symlink(&helper, link).map_err(|error| {
            format!(
                "could not install gpvim symlink {} -> {}: {error}",
                link.display(),
                helper.display()
            )
        })?;
        Ok(())
    }

    #[cfg(not(unix))]
    {
        let _ = helper;
        Err(
            "gpvim is not executable from PATH; automatic helper links are only supported on Unix"
                .to_owned(),
        )
    }
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

    if let Err(error) = ensure_gpvim_helper() {
        eprintln!("[gpvim] {error}");
    }

    if let Some(path) = options.working_directory.as_deref() {
        if let Err(error) = env::set_current_dir(path) {
            eprintln!("gpvim: failed to set working directory: {error}");
            return;
        }
    } else if let Some(path) = app_bundle_working_directory() {
        if let Err(error) = env::set_current_dir(&path) {
            eprintln!(
                "gpvim: failed to set AppBundle working directory {}: {error}",
                path.display()
            );
            return;
        }
    }

    app::run(options);
}
