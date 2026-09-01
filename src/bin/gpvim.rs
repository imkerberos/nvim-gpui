use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

fn main() -> ExitCode {
    match run() {
        Ok(status) => ExitCode::from(status),
        Err(error) => {
            eprintln!("gpvim: {error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<u8, String> {
    let arguments: Vec<OsString> = env::args_os().skip(1).collect();

    #[cfg(target_os = "macos")]
    let (application, executable) = {
        let application = locate_app_bundle()?;
        let executable = application.join("Contents/MacOS/nvim-gpui");
        (Some(application), executable)
    };

    #[cfg(not(target_os = "macos"))]
    let executable = locate_direct_executable()?;

    if is_information_request(&arguments) {
        return run_command(&executable, &arguments);
    }

    let forwarded = forwarded_arguments(&arguments);

    #[cfg(target_os = "macos")]
    {
        let application = application.expect("macOS helper must have an AppBundle");
        let status = Command::new("/usr/bin/open")
            .args([OsString::from("-n"), OsString::from("-a")])
            .arg(application)
            .arg("--args")
            .args(forwarded)
            .status()
            .map_err(|error| format!("failed to launch AppBundle: {error}"))?;
        Ok(status.code().unwrap_or(1) as u8)
    }

    #[cfg(not(target_os = "macos"))]
    {
        run_command(&executable, &forwarded)
    }
}

fn forwarded_arguments(arguments: &[OsString]) -> Vec<OsString> {
    let mut forwarded = Vec::with_capacity(arguments.len() + 4);
    if !has_working_directory(arguments) {
        if let Ok(directory) = env::current_dir() {
            forwarded.push(OsString::from("--cwd"));
            forwarded.push(directory.into_os_string());
        }
    }

    if !has_remote_connection(arguments) && !has_nvim_command(arguments) {
        if let Some(nvim) = find_nvim_command() {
            forwarded.push(OsString::from("--nvim-command"));
            forwarded.push(nvim.into_os_string());
        }
    }
    forwarded.extend_from_slice(arguments);
    forwarded
}

fn run_command(executable: &Path, arguments: &[OsString]) -> Result<u8, String> {
    let status = Command::new(executable)
        .args(arguments)
        .status()
        .map_err(|error| format!("failed to launch {}: {error}", executable.display()))?;
    Ok(status.code().unwrap_or(1) as u8)
}

#[cfg(target_os = "macos")]
fn locate_app_bundle() -> Result<PathBuf, String> {
    if let Some(application) = env::var_os("NVIM_GPUI_APP") {
        let application = PathBuf::from(application);
        if application.is_dir() {
            return Ok(application);
        }
        return Err(format!(
            "NVIM_GPUI_APP does not point to an AppBundle: {}",
            application.display()
        ));
    }

    if let Ok(executable) = env::current_exe() {
        let executable = fs::canonicalize(&executable).unwrap_or(executable);
        if let Some(application) = executable
            .ancestors()
            .find(|path| path.extension().and_then(|extension| extension.to_str()) == Some("app"))
        {
            return Ok(application.to_path_buf());
        }
    }

    let development_application =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".cache/macos/nvim-gpui.app");
    if development_application.is_dir() {
        return Ok(development_application);
    }

    let installed_application = PathBuf::from("/Applications/nvim-gpui.app");
    if installed_application.is_dir() {
        return Ok(installed_application);
    }

    Err("AppBundle not found; run `just bundle` or set NVIM_GPUI_APP".to_owned())
}

#[cfg(not(target_os = "macos"))]
fn locate_direct_executable() -> Result<PathBuf, String> {
    if let Some(executable) = env::var_os("NVIM_GPUI_BINARY") {
        let executable = PathBuf::from(executable);
        if is_executable(&executable) {
            return Ok(executable);
        }
    }

    let current = env::current_exe().map_err(|error| format!("cannot locate gpvim: {error}"))?;
    let sibling = current.parent().map(|parent| {
        parent.join(if cfg!(windows) {
            "nvim-gpui.exe"
        } else {
            "nvim-gpui"
        })
    });
    if let Some(sibling) = sibling.filter(|path| is_executable(path)) {
        return Ok(sibling);
    }

    Err("nvim-gpui executable not found".to_owned())
}

fn is_information_request(arguments: &[OsString]) -> bool {
    arguments.len() == 1
        && matches!(
            arguments[0].to_str(),
            Some("--help") | Some("-h") | Some("--version") | Some("-V")
        )
}

fn has_remote_connection(arguments: &[OsString]) -> bool {
    option_present(arguments, &["--connect", "--connect="])
}

fn has_nvim_command(arguments: &[OsString]) -> bool {
    option_present(arguments, &["--nvim-command", "--nvim-command="])
}

fn has_working_directory(arguments: &[OsString]) -> bool {
    option_present(
        arguments,
        &[
            "--cwd",
            "--cwd=",
            "--working-directory",
            "--working-directory=",
        ],
    )
}

fn option_present(arguments: &[OsString], names: &[&str]) -> bool {
    let mut pass_through = false;
    arguments.iter().any(|argument| {
        let Some(argument) = argument.to_str() else {
            return false;
        };
        if pass_through {
            return false;
        }
        if argument == "--" {
            pass_through = true;
            return false;
        }
        names
            .iter()
            .any(|name| argument == *name || (name.ends_with('=') && argument.starts_with(name)))
    })
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    let candidates = if cfg!(windows) {
        vec![OsString::from(name), OsString::from(format!("{name}.exe"))]
    } else {
        vec![OsString::from(name)]
    };

    env::split_paths(&path)
        .flat_map(|directory| candidates.iter().map(move |name| directory.join(name)))
        .find(|candidate| is_executable(candidate))
}

fn find_nvim_command() -> Option<PathBuf> {
    project_environment_is_active()
        .then(|| env::var_os("NVIM_GPUI_NVIM"))
        .flatten()
        .map(PathBuf::from)
        .filter(|path| is_executable(path))
        .or_else(|| find_executable("nvim"))
}

fn project_environment_is_active() -> bool {
    let Some(config_dir) = env::var_os("NVIM_GPUI_CONFIG_DIR") else {
        return false;
    };
    let Some(project_root) = Path::new(&config_dir).parent() else {
        return false;
    };
    let Ok(current_directory) = env::current_dir() else {
        return false;
    };
    current_directory.starts_with(project_root)
}

fn is_executable(path: &Path) -> bool {
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

#[cfg(test)]
mod tests {
    use super::{
        forwarded_arguments, has_nvim_command, has_remote_connection, is_information_request,
    };
    use std::{env, ffi::OsString};

    #[test]
    fn helper_recognizes_fixed_application_options() {
        let arguments = vec![OsString::from("--connect=127.0.0.1:6666")];
        assert!(has_remote_connection(&arguments));
        assert!(!has_nvim_command(&arguments));
    }

    #[test]
    fn helper_keeps_information_commands_attached_to_stdout() {
        assert!(is_information_request(&[OsString::from("--version")]));
        assert!(!is_information_request(&[
            OsString::from("--version"),
            OsString::from("x")
        ]));
    }

    #[test]
    fn helper_does_not_interpret_neovim_options_after_separator() {
        let arguments = vec![
            OsString::from("--"),
            OsString::from("--connect=127.0.0.1:6666"),
        ];
        assert!(!has_remote_connection(&arguments));
        assert!(!has_nvim_command(&arguments));
    }

    #[test]
    fn helper_forwards_the_callers_current_directory_without_explicit_cwd() {
        let forwarded = forwarded_arguments(&[OsString::from("README.md")]);
        let current_directory = env::current_dir().expect("test should have a current directory");

        assert_eq!(
            &forwarded[..2],
            [OsString::from("--cwd"), current_directory.into_os_string()]
        );
    }
}
