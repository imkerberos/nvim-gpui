use std::{
    any::Any,
    env,
    panic::{self, AssertUnwindSafe},
};

/// Runs GPUI startup while preserving the original panic payload and backtrace.
///
/// GPUI 0.2.2 initializes the Linux platform from `Application::new()` and uses
/// `unwrap` for some platform errors, so the application cannot observe those
/// failures as a regular `Result`. Report useful context before handing the
/// panic back to Rust's normal panic hook.
pub(crate) fn run_with_diagnostics<F, T>(operation: F) -> T
where
    F: FnOnce() -> T,
{
    match panic::catch_unwind(AssertUnwindSafe(operation)) {
        Ok(value) => value,
        Err(payload) => {
            report_panic(payload.as_ref());
            panic::resume_unwind(payload);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum FailureKind {
    NoSupportedDeviceFound,
    NoWaylandLib,
    Other,
}

fn classify_failure(message: &str) -> FailureKind {
    if message.contains("NoSupportedDeviceFound") {
        FailureKind::NoSupportedDeviceFound
    } else if message.contains("NoWaylandLib") {
        FailureKind::NoWaylandLib
    } else {
        FailureKind::Other
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&'static str>().copied())
        .unwrap_or("non-string panic payload")
}

fn report_panic(payload: &(dyn Any + Send)) {
    let message = panic_message(payload);
    let failure_kind = classify_failure(message);

    eprintln!("[nvim-gpui] GPUI startup failed; diagnostic context follows.");
    eprintln!(
        "[nvim-gpui] platform: {} | architecture: {}",
        env::consts::OS,
        env::consts::ARCH
    );
    eprintln!(
        "[nvim-gpui] XDG_SESSION_TYPE={} | WAYLAND_DISPLAY={} | DISPLAY={}",
        environment_value("XDG_SESSION_TYPE"),
        environment_value("WAYLAND_DISPLAY"),
        environment_value("DISPLAY")
    );

    match failure_kind {
        FailureKind::NoSupportedDeviceFound => report_gpu_hints(),
        FailureKind::NoWaylandLib => report_wayland_hints(),
        FailureKind::Other => {
            eprintln!(
                "[nvim-gpui] The original startup error and error chain will be re-raised below unchanged; set RUST_BACKTRACE=full for a full backtrace."
            );
        }
    }
}

fn environment_value(name: &str) -> String {
    match env::var(name) {
        Ok(value) if value.is_empty() => "<empty>".to_owned(),
        Ok(value) => value,
        Err(env::VarError::NotPresent) => "<unset>".to_owned(),
        Err(env::VarError::NotUnicode(_)) => "<non-UTF-8>".to_owned(),
    }
}

fn report_gpu_hints() {
    eprintln!(
        "[nvim-gpui] No supported Vulkan GPU device was found. Run `vulkaninfo --summary` to inspect the Vulkan loader, device, and driver."
    );
    eprintln!(
        "[nvim-gpui] Arch Linux driver candidates: AMD=`vulkan-radeon`, Intel=`vulkan-intel`, NVIDIA=`nvidia-utils` (or the matching `nvidia`/`nvidia-open` driver package)."
    );
    eprintln!(
        "[nvim-gpui] Install only the package matching the actual GPU and kernel, then run `vulkaninfo --summary` again."
    );
    eprintln!(
        "[nvim-gpui] The original startup error and error chain will be re-raised below unchanged; set RUST_BACKTRACE=full for a full backtrace."
    );
}

fn report_wayland_hints() {
    eprintln!(
        "[nvim-gpui] Wayland support could not load its runtime library. Check `libwayland-client.so.0` and related libraries with `ldconfig -p | grep wayland`."
    );
    eprintln!(
        "[nvim-gpui] On Arch Linux, install the Wayland runtime with `sudo pacman -S wayland`."
    );
    eprintln!(
        "[nvim-gpui] To temporarily test X11, run `WAYLAND_DISPLAY=\"\" ./nvim-gpui` with a valid `DISPLAY`."
    );
    eprintln!(
        "[nvim-gpui] The original startup error and error chain will be re-raised below unchanged; set RUST_BACKTRACE=full for a full backtrace."
    );
}

#[cfg(test)]
mod tests {
    use super::{classify_failure, FailureKind};

    #[test]
    fn classifies_missing_vulkan_device() {
        assert_eq!(
            classify_failure("Unable to init GPU context: NoSupportedDeviceFound"),
            FailureKind::NoSupportedDeviceFound
        );
    }

    #[test]
    fn classifies_missing_wayland_library() {
        assert_eq!(
            classify_failure("called `Result::unwrap()` on an `Err` value: NoWaylandLib"),
            FailureKind::NoWaylandLib
        );
    }

    #[test]
    fn leaves_unrelated_startup_errors_generic() {
        assert_eq!(
            classify_failure("failed to open window"),
            FailureKind::Other
        );
    }
}
