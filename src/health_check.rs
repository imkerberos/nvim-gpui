use std::{
    cell::RefCell,
    env,
    io::{self, Write},
    panic::{self, AssertUnwindSafe},
    process::{Command, Stdio},
    rc::Rc,
};

#[cfg(target_os = "linux")]
use std::fs;

use gpui::{AppContext, Application, EmptyView, GpuSpecs, WindowOptions};

#[derive(Debug)]
struct CommandProbe {
    available: bool,
    output: String,
}

pub(crate) fn run() {
    println!("nvim-gpui health check");
    println!("version: {}", env!("CARGO_PKG_VERSION"));
    println!("os: {}", env::consts::OS);
    println!("architecture: {}", env::consts::ARCH);
    println!("os-version: {}", operating_system_version());
    println!("gpui-backend: {}", gpui_backend());
    println!("display: {}", display_context());

    print_environment("DISPLAY");
    print_environment("WAYLAND_DISPLAY");
    print_environment("XDG_SESSION_TYPE");

    println!();
    println!("graphics:");
    print_vulkan_probe();
    print_opengl_probe();
    print_platform_gpu_probe();

    println!();
    if !can_probe_window() {
        println!("gpui-window: skipped (no graphical display was detected)");
        println!("status: limited (OS and external graphics probes only)");
        return;
    }

    let _ = probe_gpui_window();
}

fn operating_system_version() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(os_release) = fs::read_to_string("/etc/os-release") {
            if let Some(pretty_name) = os_release.lines().find_map(|line| {
                line.strip_prefix("PRETTY_NAME=")
                    .map(|value| value.trim_matches('"').to_owned())
            }) {
                return pretty_name;
            }
        }
        return command_text("uname", &["-sr"])
            .unwrap_or_else(|| "unknown Linux distribution".to_owned());
    }

    #[cfg(target_os = "macos")]
    {
        return command_text("sw_vers", &["-productVersion"])
            .unwrap_or_else(|| "unknown macOS version".to_owned());
    }

    #[cfg(target_os = "windows")]
    {
        return command_text("cmd", &["/C", "ver"])
            .unwrap_or_else(|| "unknown Windows version".to_owned());
    }

    #[allow(unreachable_code)]
    "unknown operating system version".to_owned()
}

fn gpui_backend() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        return "DirectX 11";
    }

    #[cfg(target_os = "macos")]
    {
        return "Metal";
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        return "Vulkan";
    }

    #[allow(unreachable_code)]
    "unknown"
}

fn display_context() -> String {
    #[cfg(target_os = "linux")]
    {
        if env::var_os("ZED_HEADLESS").is_some() {
            return "headless (ZED_HEADLESS is set)".to_owned();
        }
        if env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty()) {
            return "Wayland".to_owned();
        }
        if env::var_os("DISPLAY").is_some_and(|value| !value.is_empty()) {
            return "X11".to_owned();
        }
        return "headless (no WAYLAND_DISPLAY or DISPLAY)".to_owned();
    }

    #[cfg(target_os = "macos")]
    {
        return "AppKit".to_owned();
    }

    #[cfg(target_os = "windows")]
    {
        return "Windows desktop".to_owned();
    }

    #[allow(unreachable_code)]
    "unknown".to_owned()
}

fn print_environment(name: &str) {
    let value = match env::var(name) {
        Ok(value) if value.is_empty() => "<empty>".to_owned(),
        Ok(value) => value,
        Err(env::VarError::NotPresent) => "<unset>".to_owned(),
        Err(env::VarError::NotUnicode(_)) => "<non-UTF-8>".to_owned(),
    };
    println!("{name}: {value}");
}

fn print_vulkan_probe() {
    println!("vulkan:");
    match command_probe("vulkaninfo", &["--summary"]) {
        Some(probe) if probe.available => print_probe_output(&probe.output),
        Some(probe) => {
            println!("  unavailable");
            print_probe_output(&probe.output);
        }
        None => println!("  command not found: vulkaninfo --summary"),
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn print_opengl_probe() {
    println!("opengl:");
    match command_probe("glxinfo", &["-B"]) {
        Some(probe) if probe.available => print_probe_output(&probe.output),
        Some(probe) => {
            println!("  unavailable");
            print_probe_output(&probe.output);
        }
        None => println!("  command not found: glxinfo -B"),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
fn print_opengl_probe() {
    println!("opengl:");
    println!("  not used by GPUI on this platform");
}

#[cfg(target_os = "macos")]
fn print_platform_gpu_probe() {
    println!("platform-gpu:");
    match command_probe("system_profiler", &["SPDisplaysDataType"]) {
        Some(probe) if probe.available => print_probe_output(&probe.output),
        Some(probe) => {
            println!("  unavailable");
            print_probe_output(&probe.output);
        }
        None => println!("  command not found: system_profiler SPDisplaysDataType"),
    }
}

#[cfg(target_os = "windows")]
fn print_platform_gpu_probe() {
    println!("platform-gpu:");
    println!("  DXGI device and driver details are reported by GPUI below");
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn print_platform_gpu_probe() {}

fn command_probe(program: &str, arguments: &[&str]) -> Option<CommandProbe> {
    let output = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&output.stderr).into_owned();
    }
    Some(CommandProbe {
        available: output.status.success(),
        output: text,
    })
}

fn command_text(program: &str, arguments: &[&str]) -> Option<String> {
    let probe = command_probe(program, arguments)?;
    probe.available.then(|| probe.output.trim().to_owned())
}

fn print_probe_output(output: &str) {
    let mut lines = output.lines();
    for line in lines.by_ref().take(80) {
        println!("  {line}");
    }
    if lines.next().is_some() {
        println!("  ... output truncated after 80 lines");
    }
}

fn can_probe_window() -> bool {
    #[cfg(target_os = "linux")]
    {
        return env::var_os("ZED_HEADLESS").is_none()
            && (env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty())
                || env::var_os("DISPLAY").is_some_and(|value| !value.is_empty()));
    }

    true
}

fn probe_gpui_window() -> Result<Option<GpuSpecs>, String> {
    let application =
        panic::catch_unwind(AssertUnwindSafe(Application::new)).map_err(|payload| {
            crate::startup_diagnostics::report_panic(payload.as_ref());
            let error = "GPUI application initialization panicked";
            print_gpui_probe_failure(error);
            error.to_owned()
        })?;

    let result = Rc::new(RefCell::new(None));
    let result_for_callback = result.clone();
    let run_result = panic::catch_unwind(AssertUnwindSafe(|| {
        application.run(move |cx| {
            let window_result = cx
                .open_window(
                    WindowOptions {
                        show: false,
                        focus: false,
                        ..Default::default()
                    },
                    |_, cx| cx.new(|_| EmptyView),
                )
                .map_err(|error| error.to_string())
                .and_then(|window| {
                    window
                        .update(cx, |_, window, _| window.gpu_specs())
                        .map_err(|error| error.to_string())
                });
            print_gpui_probe_result(&window_result);
            let _ = io::stdout().flush();
            *result_for_callback.borrow_mut() = Some(window_result);
            cx.quit();
        });
    }));

    if let Err(payload) = run_result {
        crate::startup_diagnostics::report_panic(payload.as_ref());
        let error = "GPUI event loop panicked";
        print_gpui_probe_failure(error);
        return Err(error.to_owned());
    }

    let Some(probe_result) = result.borrow_mut().take() else {
        let error = "GPUI event loop returned without completing the probe";
        print_gpui_probe_failure(error);
        return Err(error.to_owned());
    };
    probe_result
}

fn print_gpu_specs(specs: &GpuSpecs) {
    println!("gpu.device: {}", non_empty_or_unknown(&specs.device_name));
    println!("gpu.driver: {}", non_empty_or_unknown(&specs.driver_name));
    println!(
        "gpu.driver-info: {}",
        non_empty_or_unknown(&specs.driver_info)
    );
    println!("gpu.software-emulated: {}", specs.is_software_emulated);
}

fn print_gpui_probe_result(result: &Result<Option<GpuSpecs>, String>) {
    match result {
        Ok(Some(specs)) => {
            println!("gpui-window: available");
            print_gpu_specs(specs);
            println!("status: ok");
        }
        Ok(None) => {
            println!("gpui-window: available");
            println!("gpu: GPUI did not expose device specifications");
            println!("status: ok");
        }
        Err(error) => {
            print_gpui_probe_failure(error);
        }
    }
}

fn print_gpui_probe_failure(error: &str) {
    println!("gpui-window: unavailable");
    println!("gpui-error: {error}");
    println!("status: failed");
}

fn non_empty_or_unknown(value: &str) -> &str {
    if value.is_empty() {
        "unknown"
    } else {
        value
    }
}
