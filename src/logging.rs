use flexi_logger::{Cleanup, Criterion, FileSpec, Logger, Naming, WriteMode};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
use std::process::Command;

const LOG_DIRECTORY_ENV: &str = "NVIM_GPUI_LOG_DIR";
const LOG_FILE_SIZE_BYTES: u64 = 10 * 1024 * 1024;
const ROTATED_LOG_FILES: usize = 5;

pub(crate) fn init(level: crate::settings::LogLevel) -> Result<flexi_logger::LoggerHandle, String> {
    let directory =
        log_directory().ok_or_else(|| "could not determine the log directory".to_owned())?;
    fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "could not create log directory {}: {error}",
            directory.display()
        )
    })?;

    let logger = Logger::try_with_env_or_str(level.key()).unwrap_or_else(|error| {
        eprintln!(
            "[logging] invalid RUST_LOG ({error}); using {}",
            level.key()
        );
        Logger::with(level.filter())
    });

    let logger = logger
        .log_to_file(
            FileSpec::default()
                .directory(&directory)
                .basename("nvim-gpui"),
        )
        .format(flexi_logger::detailed_format)
        .rotate(
            Criterion::Size(LOG_FILE_SIZE_BYTES),
            Naming::Numbers,
            Cleanup::KeepLogFiles(ROTATED_LOG_FILES),
        )
        .write_mode(WriteMode::Async)
        .start()
        .map_err(|error| format!("could not start logger: {error}"))?;
    log::info!(
        target: "nvim_gpui::logging",
        "logging initialized: directory={}, max_file_size_mb={}, rotated_files={}",
        directory.display(),
        LOG_FILE_SIZE_BYTES / (1024 * 1024),
        ROTATED_LOG_FILES
    );
    Ok(logger)
}

pub(crate) fn set_level(logger: &flexi_logger::LoggerHandle, level: crate::settings::LogLevel) {
    if let Err(error) = logger.parse_new_spec(level.key()) {
        eprintln!(
            "[logging] could not apply log level {}: {error}",
            level.key()
        );
    }
}

pub(crate) fn log_directory() -> Option<PathBuf> {
    if let Some(path) = env::var_os(LOG_DIRECTORY_ENV) {
        return Some(PathBuf::from(path));
    }

    crate::settings::application_support_directory().map(|directory| directory.join("logs"))
}

pub(crate) fn open_log_directory() -> Result<(), String> {
    let directory =
        log_directory().ok_or_else(|| "could not determine the log directory".to_owned())?;
    open_directory(&directory)
}

pub(crate) fn open_directory(directory: &Path) -> Result<(), String> {
    fs::create_dir_all(directory).map_err(|error| {
        format!(
            "could not create directory {}: {error}",
            directory.display()
        )
    })?;

    #[cfg(target_os = "macos")]
    let status = Command::new("open")
        .arg("-R")
        .arg(directory)
        .status()
        .map_err(|error| format!("could not open Finder: {error}"))?;

    #[cfg(target_os = "windows")]
    let status = Command::new("explorer.exe")
        .arg(directory)
        .status()
        .map_err(|error| format!("could not open File Explorer: {error}"))?;

    #[cfg(target_os = "linux")]
    let status = Command::new("xdg-open")
        .arg(directory)
        .status()
        .map_err(|error| format!("could not open the file manager: {error}"))?;

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    return Err("opening a directory is not supported on this platform".to_owned());

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "file manager exited with {}",
            status
                .code()
                .map_or_else(|| "a signal".to_owned(), |code| code.to_string())
        ))
    }
}
