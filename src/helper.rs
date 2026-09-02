//! Installation and discovery of the optional `gpvim` command-line helper.

use std::{
    env, fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::symlink;

pub(crate) fn is_available_in_path() -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    let command_name = if cfg!(windows) { "gpvim.exe" } else { "gpvim" };
    env::split_paths(&path).any(|directory| is_executable_path(&directory.join(command_name)))
}

pub(crate) fn install() -> Result<(), String> {
    if is_available_in_path() {
        return Ok(());
    }

    let Some(helper) = bundled_path() else {
        return Err("bundled gpvim helper could not be found".to_owned());
    };

    #[cfg(unix)]
    {
        let preferred = PathBuf::from("/usr/local/bin/gpvim");
        match install_link(&helper, &preferred) {
            Ok(()) => Ok(()),
            Err((_, error)) if error.kind() == ErrorKind::PermissionDenied => {
                let Some(directory) = user_path_directory() else {
                    return Err(format_link_error(
                        &helper,
                        &(preferred, error),
                        "add ~/.local/bin or ~/bin to PATH and retry",
                    ));
                };
                let fallback = directory.join(command_name());
                install_link(&helper, &fallback).map_err(|fallback_error| {
                    format!(
                        "{}; fallback failed: {}",
                        format_link_error(
                            &helper,
                            &(PathBuf::from("/usr/local/bin/gpvim"), error),
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

    install()
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
fn command_name() -> &'static str {
    "gpvim"
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
