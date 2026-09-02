use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;

pub(crate) const NVIM_GPUI_ENV: &str = "NVIM_GPUI";
pub(crate) const NVIM_GPUI_ENV_VALUE: &str = "1";

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

pub(crate) fn apply_nvim_environment(command: &mut Command) {
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
    mark_embedded_gui_environment(&mut environment);
    command.envs(environment);
}

pub(crate) fn mark_embedded_gui_environment(environment: &mut HashMap<OsString, OsString>) {
    // This is intentionally set before nvim starts loading init.lua. An RPC
    // global would be too late for startup-time theme selection.
    environment.insert(
        OsString::from(NVIM_GPUI_ENV),
        OsString::from(NVIM_GPUI_ENV_VALUE),
    );
}

pub(super) fn project_nvim_environment_is_active(
    environment: &HashMap<OsString, OsString>,
) -> bool {
    let Ok(current_directory) = std::env::current_dir() else {
        return false;
    };
    project_nvim_environment_is_active_at(environment, &current_directory)
}

pub(crate) fn project_nvim_environment_is_active_at(
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

pub(crate) fn remove_project_nvim_environment(environment: &mut HashMap<OsString, OsString>) {
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

pub(crate) fn apply_project_nvim_environment(environment: &mut HashMap<OsString, OsString>) {
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

pub(crate) fn parse_environment(bytes: &[u8]) -> HashMap<OsString, OsString> {
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
