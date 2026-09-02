//! Installation and discovery of the optional `gpvim` command-line helper.

use std::{
    env, fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::symlink;
#[cfg(target_os = "macos")]
use std::process::Command;

#[cfg(windows)]
const HELPER_COMMAND_NAMES: &[&str] = &["gpvim.exe", "gpvimdiff.exe"];
#[cfg(not(windows))]
const HELPER_COMMAND_NAMES: &[&str] = &["gpvim", "gpvimdiff"];

pub(crate) fn is_available_in_path() -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    HELPER_COMMAND_NAMES.iter().all(|command_name| {
        env::split_paths(&path).any(|directory| is_executable_path(&directory.join(command_name)))
    })
}

pub(crate) fn install() -> Result<(), String> {
    install_internal(true)
}

fn install_internal(request_admin_authorization: bool) -> Result<(), String> {
    let _ = request_admin_authorization;

    if is_available_in_path() {
        return Ok(());
    }

    let Some(helper) = bundled_path() else {
        return Err("bundled gpvim helper could not be found".to_owned());
    };

    #[cfg(unix)]
    {
        let preferred_directory = PathBuf::from("/usr/local/bin");
        let preferred_links = command_paths(&preferred_directory);
        match install_links(&helper, &preferred_links) {
            Ok(()) => Ok(()),
            Err((failed_path, error)) if error.kind() == ErrorKind::PermissionDenied => {
                #[cfg(target_os = "macos")]
                if request_admin_authorization {
                    return install_links_with_macos_authorization(&helper, &preferred_links)
                        .map_err(|authorization_error| {
                            format!(
                                "could not install gpvim and gpvimdiff in {}: administrator authorization failed: {}",
                                preferred_directory.display(),
                                authorization_error,
                            )
                        });
                }

                let Some(directory) = user_path_directory() else {
                    return Err(format_link_error(
                        &helper,
                        &(failed_path, error),
                        "add ~/.local/bin or ~/bin to PATH and retry",
                    ));
                };
                let fallback_links = command_paths(&directory);
                install_links(&helper, &fallback_links).map_err(|fallback_error| {
                    format!(
                        "{}; fallback failed: {}",
                        format_link_error(
                            &helper,
                            &(failed_path, error),
                            "add ~/.local/bin or ~/bin to PATH and retry",
                        ),
                        format_link_error(&helper, &fallback_error, "choose another PATH entry")
                    )
                })
            }
            Err(error) => Err(format_link_error(
                &helper,
                &error,
                "remove the existing entry or choose another PATH entry",
            )),
        }
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

pub(crate) fn ensure_installed() -> Result<(), String> {
    if is_available_in_path() {
        return Ok(());
    }

    // Keep startup compatible with the original behavior: a development or
    // incomplete installation without a bundled helper is not fatal.
    if bundled_path().is_none() {
        return Ok(());
    }

    // Startup should not unexpectedly show an administrator prompt. It may
    // still use an already-configured user-writable PATH directory.
    install_internal(false)
}

fn bundled_path() -> Option<PathBuf> {
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

#[cfg(unix)]
fn install_link(helper: &Path, link: &Path) -> Result<(), (PathBuf, io::Error)> {
    if fs::symlink_metadata(link).is_ok() {
        return Err((
            link.to_owned(),
            io::Error::new(ErrorKind::AlreadyExists, "destination already exists"),
        ));
    }
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent).map_err(|error| (parent.to_owned(), error))?;
    }
    symlink(helper, link).map_err(|error| (link.to_owned(), error))
}

#[cfg(unix)]
fn install_links(helper: &Path, links: &[PathBuf]) -> Result<(), (PathBuf, io::Error)> {
    for link in links {
        match install_link(helper, link) {
            Ok(()) => {}
            Err((path, error))
                if error.kind() == ErrorKind::AlreadyExists
                    && fs::read_link(&path).is_ok_and(|target| target == helper) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn command_paths(directory: &Path) -> Vec<PathBuf> {
    HELPER_COMMAND_NAMES
        .iter()
        .map(|command_name| directory.join(command_name))
        .collect()
}

#[cfg(target_os = "macos")]
fn install_links_with_macos_authorization(helper: &Path, links: &[PathBuf]) -> Result<(), String> {
    let first_link = links
        .first()
        .ok_or_else(|| "no gpvim helper links were requested".to_owned())?;
    let parent = first_link.parent().ok_or_else(|| {
        format!(
            "could not determine parent directory for {}",
            first_link.display()
        )
    })?;
    let parent = parent
        .to_str()
        .ok_or_else(|| "gpvim install path is not valid UTF-8".to_owned())?;
    let helper = helper
        .to_str()
        .ok_or_else(|| "bundled gpvim path is not valid UTF-8".to_owned())?;

    let mut shell_commands = vec![format!("/bin/mkdir -p {}", shell_quote(parent))];
    for link in links {
        let link = link
            .to_str()
            .ok_or_else(|| "gpvim install path is not valid UTF-8".to_owned())?;
        let quoted_link = shell_quote(link);
        shell_commands.push(format!(
            "if [ -L {quoted_link} ]; then :; else /bin/ln -s {} {quoted_link}; fi",
            shell_quote(helper),
        ));
    }
    let shell_command = shell_commands.join(" && ");
    let apple_script = format!(
        "do shell script \"{}\" with administrator privileges",
        escape_applescript_string(&shell_command),
    );
    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(apple_script)
        .output()
        .map_err(|error| format!("could not start macOS authorization dialog: {error}"))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        Err(format!(
            "authorization process exited with {}",
            output
                .status
                .code()
                .map_or_else(|| "a signal".to_owned(), |code| code.to_string())
        ))
    } else {
        Err(stderr)
    }
}

#[cfg(target_os = "macos")]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn escape_applescript_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(unix)]
fn format_link_error(helper: &Path, error: &(PathBuf, io::Error), suggestion: &str) -> String {
    format!(
        "could not install gpvim symlink {} -> {}: {}; {}",
        error.0.display(),
        helper.display(),
        error.1,
        suggestion
    )
}

#[cfg(unix)]
fn user_path_directory() -> Option<PathBuf> {
    let home = PathBuf::from(env::var_os("HOME")?);
    let path = env::var_os("PATH")?;
    [home.join(".local/bin"), home.join("bin")]
        .into_iter()
        .find(|candidate| env::split_paths(&path).any(|directory| directory == *candidate))
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
