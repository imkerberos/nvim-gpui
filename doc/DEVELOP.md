# Development Guide

This document contains repository and contributor notes. For installation,
Neovim configuration, image configuration, and user-facing limitations, see
the root [README](../README.md).

## Development environment

The repository uses a Nix flake and `direnv`. The flake follows the
`nixos-26.05` nixpkgs channel and provides Rust, GPUI's native build
dependencies, Neovim, `lazy.nvim`, `snacks.nvim`, the Markdown Tree-sitter
parser, ImageMagick, CMake, `just`, `gh`, and `gnumake`.

```sh
direnv allow
nix develop
```

The platform development tasks provide a single entry point for setting up or
entering the development environment:

```sh
just dev                 # select the current platform automatically
just dev-macos           # enter the macOS Nix development shell
just dev-linux           # enter the Linux Nix development shell
just dev-windows         # install Windows prerequisites with winget
```

`dev-macos` and `dev-linux` enter this repository's Nix flake. `dev-windows`
must be run from an elevated PowerShell or Command Prompt with `winget`.
It installs the x64 Visual Studio 2022 Build Tools with the C++ workload,
CMake components, and Windows 10 SDK 20348, plus Rustup, Git, CMake, Python
3.11, Neovim, `just`, `gh`, Inno Setup, 7-Zip, and aria2. It then installs the
stable Rust toolchain and the `stable-x86_64-pc-windows-msvc` x64 host
toolchain. Windows build tasks use the x64 host toolchain and the
`x86_64-pc-windows-msvc` target.
On an ARM64 Windows guest, rustup installs this non-host x64 toolchain with
its explicit emulation override; Windows 11 on Arm runs the x64 Rust tools
through its compatibility layer.
The task also finds the MSVC `Hostx64/x64/link.exe` and persists it in
`CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER`, so another `link.exe` cannot be
selected accidentally. Restart the terminal after installation, then use an
x64 Native Tools Command Prompt for VS 2022, or start PowerShell from that
environment, so `cl.exe` is on `PATH`.

On a fresh Windows installation, `git` and `just` are not available yet, so
bootstrap the environment directly from PowerShell or Command Prompt. The
repository includes a command wrapper that only requires the Windows-built-in
PowerShell and `winget`:

```powershell
.\dev-windows.cmd
```

The equivalent direct PowerShell command is:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\dev-windows.ps1
```

After the script installs Git and `just`, restart the terminal. Later runs can
use `just dev-windows` or the platform dispatcher `just dev`.

Cargo keeps its normal default locations:

- `target/` is Cargo's default build output directory.
- `~/.cargo/` is Cargo's default home for the registry, Git sources, and tools.
- The operating system's default temporary directory is used for compiler and
  build-script temporary files.
- `.cache/nvim-*` contains the repository Neovim data, state, and cache.

The shell does not export `XDG_CONFIG_HOME`, `XDG_DATA_HOME`,
`XDG_STATE_HOME`, or `XDG_CACHE_HOME` globally. This keeps Git and other
programs using their normal home-directory locations. The application passes
repository-scoped XDG paths only to its embedded Neovim child.

`NVIM_APPNAME=nvim-gpui` and `NVIM_GPUI_*` variables are intended for the
repository's development profile. The application removes stale repository
values when it is launched from another working directory, so a normal
Neovim configuration can be tested outside this checkout.

For an embedded session, the application also sets NVIM_GPUI=1 in the child
environment and sets g:nvim_gpui before Neovim loads init.lua. These are the
frontend markers for GUI-specific configuration, for example:

~~~lua
local is_nvim_gpui = vim.g.nvim_gpui == true
~~~

The markers are injected before startup rather than set through RPC, so they
are available during theme selection. They are not injected when connecting
to an already-running remote Neovim process.

## Windows development with VMware Fusion

On an Apple silicon Mac, use VMware Fusion with a Windows 11 ARM64 guest.
VMware Fusion cannot run an x86/x86_64 Windows guest directly on Apple
silicon, but Windows 11 on Arm can run x64 user-mode applications through its
built-in emulation layer. This allows an x64 nvim-gpui build to be compiled
and smoke-tested in the VM; release validation should still include a native
x64 Windows host or CI runner because emulated CPU and virtual GPU behavior
is not identical.

See the [VMware Apple silicon guest limitations](https://knowledge.broadcom.com/external/article/315602/)
and [Microsoft's Windows on Arm emulation documentation](https://learn.microsoft.com/en-us/windows/arm/apps-on-arm-x86-emulation)
before creating the VM. Use a Windows 11 ARM64 image, not an x64 image.

Inside the VM, run `just dev-windows` from an elevated PowerShell or Command Prompt
terminal. It installs the required tools automatically through `winget`:

- Visual Studio 2022 Build Tools with **Desktop development with C++**, the
  MSVC v143 x64/x86 build tools, a Windows SDK, and CMake tools;
- Rust through `rustup`;
- Git for Windows (Git Bash is optional);
- CMake, Python 3.11+, `just`, Inno Setup, 7-Zip, and aria2;
- Neovim 0.10 or newer.

The [Rustup MSVC prerequisites](https://rust-lang.github.io/rustup/installation/windows-msvc.html)
and [Visual Studio Build Tools workload reference](https://learn.microsoft.com/en-us/visualstudio/install/workload-component-id-vs-build-tools?view=visualstudio)
describe the compiler and SDK components. The full Visual Studio IDE is not
required. The `Justfile` uses PowerShell on Windows, so Git Bash is optional.

Start an **x64 Native Tools Command Prompt for VS 2022**, then use PowerShell
from that environment so `cl.exe` is available. The setup task installs the
x64 host toolchain and the task runner. For an ARM64 Windows guest, the
Justfile automatically selects the x64 toolchain and target for Cargo tasks;
do not set `CARGO_BUILD_TARGET` to the ARM64 host target.

If invoking Cargo manually, use the same explicit toolchain and target:

```powershell
cargo +stable-x86_64-pc-windows-msvc build --release --bins --target x86_64-pc-windows-msvc
```

Only the application's development data is kept in the repository:

```powershell
$env:NVIM_GPUI_CACHE_DIR = "$PWD\.cache"
$env:NVIM_GPUI_CONFIG_DIR = "$PWD\config"
$env:SNACKS_KITTY = "1"
New-Item -ItemType Directory -Force $env:NVIM_GPUI_CACHE_DIR | Out-Null
```

If Neovim is not on `PATH`, set `NVIM_GPUI_NVIM` to an absolute Windows path,
for example `C:/Program Files/Neovim/bin/nvim.exe`. The Nix shell's plugin
paths are not available on Windows; the repository Neovim profile still works
without those optional paths, while image and Tree-sitter development tests
require installing the corresponding plugins separately.

Do not fix the linker conflict by renaming or deleting unrelated `link.exe`
files. Cargo uses the explicit MSVC linker configured by `dev-windows`; check it from
PowerShell with:

```powershell
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER
& $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER /?
```

Run the checks and x64 build with:

```powershell
just ci
just build-release
just run
```

For the Windows Rime runtime and directory bundle, switch to PowerShell in
the same Visual Studio developer environment:

```powershell
just rime-runtime-windows
just bundle-windows
```

When no `NVIM_GPUI_RIME_STARTER_DATA` is supplied, the Rime task downloads the
pinned official starter data. It also locates the standard 7-Zip installation
directory automatically, so 7-Zip does not need to be added to PATH manually.

The resulting Windows bundle is described in the [packaging](#packaging)
section. VMware Fusion is a development and smoke-test environment here; it
does not replace native x64 Windows validation for release artifacts.

## Neovim version requirement

The minimum supported Neovim version is **0.10.0**. This applies to both
embedded Neovim processes and remote sessions opened with `--connect`. The
Neovim UI protocol has versioned multigrid payloads, so the client creates a
protocol adapter after `nvim_get_api_info()` and normalizes those payloads
before they reach application state. Neovim 0.10 and 0.11 use the legacy
floating-window and message-grid payloads; Neovim 0.12 and newer use the
extended payloads with screen positions and compositor indices.

Check the version used by the development shell or an external configuration
with:

~~~sh
nvim --version | head -1
~~~

## just tasks

Run tasks from the Nix development shell on macOS/Linux. On Windows, the
Justfile uses PowerShell, so Git Bash is optional; run the tasks from the
Visual Studio developer environment. See [Windows development with VMware
Fusion](#windows-development-with-vmware-fusion).

The tasks are grouped by responsibility. Low-level tasks do one operation;
the `ci` and `pack-*` tasks compose them:

```text
validation:  fmt, fmt-check, check, clippy, test, ci
build:       build, build-release, release, run, gpvim
dev:         current OS -> dev-macos, dev-linux, or dev-windows
ubuntu test: setup-ubuntu
linux:       docker-build, docker-build-linux-aarch64
packages:    pack-linux-x86_64, pack-linux-aarch64
rime:        rime-runtime, rime-runtime-check, rime-runtime-macos,
             rime-runtime-windows
bundle:      current OS -> bundle-macos or bundle-windows
installer:   installer-windows (Inno Setup)
smoke:       current OS -> smoke-macos or smoke-windows
macOS:       bundle-macos, smoke-macos, dmg, pack-macos
Windows:     bundle-windows, installer-windows, smoke-windows, pack-windows
release:     release-prepare, release-check, release-notes
```

Common commands:

```sh
just ci
just build-release
just release              # compatibility alias for build-release
just dev                  # select the current platform development setup
just run
just bundle               # select bundle-macos or bundle-windows automatically
just installer-windows    # Windows only: build the Inno Setup installer
just smoke                # select smoke-macos or smoke-windows automatically

just pack-macos          # macOS only: checks, runtime, AppBundle, smoke test, DMG
just pack-windows        # Windows only: checks, runtime, bundle, installer, smoke test
just setup-ubuntu        # Ubuntu VM only: runtime libraries and IME support

just docker-build x86_64
just docker-build-linux-aarch64
just pack-linux-x86_64  # local Ubuntu amd64 .deb through Docker
just pack-linux-aarch64  # local Ubuntu arm64 .deb through Docker

just release-prepare 0.2.0
just release-check v0.2.0
just release-notes v0.2.0
```

The dependency flow is intentional: `ci` runs `check`, Clippy, and tests;
`dmg` runs after `smoke-macos`, which runs after `bundle-macos` and
`build-release`; `pack-macos` additionally builds the macOS Rime runtime;
`smoke-windows` runs after `bundle-windows`; `installer-windows` builds the
Inno Setup installer from that bundle; and `pack-windows` additionally builds
the Windows Rime runtime, installer, and smoke-tests both bundled executables.
The old `release` task remains a compatibility alias for `build-release`.

`Cargo.toml` is the canonical version source. Before creating a release, run
`just release-prepare VERSION`, add the matching section to `CHANGELOG.md`,
then run `just release-check vVERSION`. The script synchronizes `Cargo.lock`,
the macOS AppBundle metadata, and the Homebrew Cask. The release workflow
repeats the check and uses the matching changelog section as the GitHub Release
body.

`Makefile` forwards the common tasks to `just` for environments where a Make
entry point is more convenient.

### Ubuntu test environment

`setup-ubuntu` prepares an Ubuntu desktop VM for testing an already-built
nvim-gpui binary. It installs the GPUI runtime libraries, Neovim 0.12.5 when
the existing Neovim is missing or older than 0.10, Ubuntu's default IBus with
the ordinary `ibus-libpinyin` engine, and the system librime/Rime data needed
by nvim-gpui's built-in Rime backend:

```sh
just setup-ubuntu
```

The two input paths are intentionally separate. `System IME` uses IBus and
libpinyin; `Rime` loads librime directly inside nvim-gpui. The setup task does
not install `ibus-rime`, because that would register Rime as an IBus engine and
would blur this distinction. After setup, log out and back in if Ubuntu has
not refreshed the desktop input-method session. Set
`NVIM_GPUI_CONFIGURE_IBUS=0` when the VM already has its own input-method
selection.

If an older Neovim is already installed, the pinned test binary is placed at
`/usr/local/bin/nvim-gpui-nvim` without replacing the existing `nvim` command.
Use it explicitly when testing:

```sh
export NVIM_GPUI_NVIM=/usr/local/bin/nvim-gpui-nvim
```

### Linux builds through Docker

On macOS, Docker can build Linux release-mode binaries for either supported
architecture. The task selects the Docker platform and keeps the copied result
in a separate architecture-specific output directory:

```sh
just docker-build x86_64
just docker-build-linux-aarch64
```

The resulting binaries are stored at:

```text
.cache/artifacts/linux-x86_64/release/nvim-gpui
.cache/artifacts/linux-x86_64/release/gpvim
.cache/artifacts/linux-aarch64/release/nvim-gpui
.cache/artifacts/linux-aarch64/release/gpvim
```

The task creates two persistent Docker data volumes. Each architecture has a
separate Nix store (`nvim-gpui-nix-linux-x86_64` or
`nvim-gpui-nix-linux-aarch64`), while Cargo's default home inside the container
(`/root/.cargo`) uses the shared `nvim-gpui-cargo-home` volume. Subsequent
builds therefore reuse Nix packages and Cargo dependencies instead of
downloading them again. Cargo itself writes to the default `/workspace/target`
directory; the two release binaries are copied into the architecture-specific
artifact directory after the build.

The task uses the pinned `nixos/nix:2.32.3` image by default. Override it only
when intentionally testing another Nix image:

```sh
NVIM_GPUI_NIX_IMAGE=nixos/nix:2.32.3 just docker-build x86_64
```

The Docker task passes `sandbox = false` and `filter-syscalls = false` to the
inner Nix command. The second option is required when `linux/amd64` runs
through QEMU on an Apple Silicon host: Nix's seccomp BPF syscall filter cannot
be loaded through that emulation layer. Docker already provides the outer
container isolation, so the task does not require `--privileged`.

Docker can produce Linux binaries only. Those binaries are useful for local
compile and smoke testing, but they are not release artifacts because a Nix
development shell can leave dynamic references to Nix-provided libraries.
The release workflow therefore builds Linux on native GitHub runners and uses
that job as release validation. macOS and Windows packages still use their
respective native build environments.

### Ubuntu Linux packages

These tasks run Ubuntu 24.04 containers, install the Ubuntu build dependencies,
compile the Rust application against Ubuntu's system libraries, and create
Debian packages. They are suitable for local testing and are also used by the
Linux release jobs on native GitHub ARM64 and x86_64 runners:

```sh
just pack-linux-x86_64
just pack-linux-aarch64
```

The outputs are written to:

```text
dist/ubuntu-x86_64/nvim-gpui_VERSION_amd64.deb
dist/ubuntu-aarch64/nvim-gpui_VERSION_arm64.deb
```

On an Apple Silicon Mac, the arm64 task runs natively and the amd64 task runs
through Docker's `linux/amd64` emulation.

The package contains `nvim-gpui`, `gpvim`, `gpvimdiff`, the desktop entry, and
the application icon set in standard hicolor sizes. The desktop entry uses the
`nvim-gpui` icon name, so desktop environments can resolve it without a
hard-coded path. It declares the GUI libraries and system librime/Rime data as
Debian dependencies. Neovim is listed as a suggestion because Ubuntu
versions may provide an older Neovim; run `just setup-ubuntu` on the test VM
to install a compatible Neovim and the separate IBus/libpinyin test path.
The desktop entry's `MimeType` field makes nvim-gpui available in common Linux
file managers' Open With menus for source and text files; its `%F` argument
forwards the selected paths to the embedded Neovim session.

The task uses persistent Docker volumes for Cargo downloads, the Rust toolchain,
and Ubuntu's APT archive. Override the builder image only when intentionally
testing another Ubuntu image:

```sh
NVIM_GPUI_UBUNTU_IMAGE=ubuntu:24.04 just pack-linux-x86_64
NVIM_GPUI_UBUNTU_IMAGE=ubuntu:24.04 just pack-linux-aarch64
```

The packages are linked against Ubuntu libraries rather than the Nix store,
unlike the binaries produced by `just docker-build x86_64`. The release
workflow runs these tasks on native Linux runners, so Docker is used only for
the reproducible Ubuntu packaging environment and not for cross-architecture
emulation.

### CI/CD workflow

The repository uses native runners for platform validation and keeps Docker as
an optional local helper:

| Event | Jobs | Result |
| --- | --- | --- |
| Pull request or push to `develop`, `master`, or `main` | macOS arm64, Linux x86_64, Linux arm64, Windows x86_64 | Formatting, Clippy, and tests; macOS also builds and smoke-tests its AppBundle and DMG. |
| Push of a `v*` tag | macOS arm64/x86_64, Linux x86_64/arm64, Windows x86_64 | Release metadata validation, native build/test validation, macOS DMG/App ZIP packages, Ubuntu `.deb` packages, and Windows ZIP/installer packages. |
| Successful completion of every release job | Publish job | Creates or updates the GitHub Release, attaches packages, and uploads `SHA256SUMS`. |

The release workflow is gated: a package is not published when any platform
validation or packaging job fails. Re-running a failed workflow is safe; the
publish step updates an existing release and replaces assets with the newly
verified files.

The only intentional release-time manual steps are preparing the version and
changelog, then pushing the tag:

```sh
just release-prepare 0.6.0
# add or update ## [0.6.0] in CHANGELOG.md
just release-check v0.6.0
git add Cargo.toml Cargo.lock Casks/nvim-gpui.rb packaging/macos/Info.plist CHANGELOG.md
git commit -m "release: prepare v0.6.0"
git tag -a v0.6.0 -m "nvim-gpui v0.6.0"
git push origin develop v0.6.0
```

The release assets use the following target names:

```text
nvim-gpui-vVERSION-darwin-aarch64.dmg
nvim-gpui-vVERSION-darwin-x86_64.dmg
nvim-gpui-vVERSION-linux-aarch64.deb
nvim-gpui-vVERSION-linux-x86_64.deb
nvim-gpui-vVERSION-windows-x86_64-setup.exe
```

The release also attaches App ZIP archives for macOS and a portable ZIP for
Windows. Linux release packages are Ubuntu/Debian `.deb` files and use system
librime, Rime data, and GUI libraries; Flatpak and a self-contained Linux Rime
runtime remain future packaging work. Windows ARM64 is not a separate release
target yet; the Windows x86_64 package can run on Windows 11 on Arm through
x64 emulation.

The development Neovim profile is at
`config/nvim-gpui/init.lua`. It loads the Nix-provided plugins without cloning
or downloading them. Its current test profile enables `snacks.image`, the
Markdown parser, and the Kitty capability fallback used by the GUI.

## Repository layout

The repository is split into protocol/state code, reusable rendering modules,
GUI windows, and platform packaging:

| Path | Responsibility |
| --- | --- |
| `src/main.rs` | Process entry point, CLI parsing, and GPUI startup. |
| `src/app.rs`, `src/app/` | Main application entity, redraw state, lifecycle, compositor, editor rendering, and titlebar/windows. |
| `src/grid.rs`, `src/grid/` | Terminal cell model, shaping/cache, cursor, highlight resolution, and the custom grid element. |
| `src/nvim.rs`, `src/nvim/` | Embedded/remote Neovim transport, MessagePack-RPC protocol, environment, session, and version handling. |
| `src/gui.rs`, `src/gui/` | Standalone Settings and About windows. |
| `src/input.rs` | System IME, Rime, and Neovim input routing. |
| `src/rime.rs` | GPUI-independent native librime loading, session handling, context, and runtime discovery. |
| `src/clipboard.rs`, `src/image_store.rs`, `src/logging.rs` | Clipboard bridge, Kitty image storage, and asynchronous application logging. |
| `src/update_check.rs` | GitHub stable-release lookup and HTTP client adapter used by Settings. |
| `src/settings.rs`, `src/platform.rs`, `src/helper.rs`, `src/widgets.rs` | Persistent settings, platform integration, CLI helper installation, and shared GUI widgets. |
| `config/nvim-gpui/` | Isolated Neovim configuration used by the development shell. |
| `packaging/rime/` | librime source-build manifests/builders and curated starter-data selection. |
| `packaging/macos/`, `packaging/windows/` | AppBundle validation and platform bundle scripts. |
| `scripts/` | Release metadata, runtime staging/validation, and starter-data tooling. |
| `.github/workflows/` | Native cross-platform CI and gated release automation. |
| `assets/` | Icons, screenshots, and bundled Nerd Fonts. |
| `.cache/`, `tmp/` | Ignored build outputs, runtime artifacts, Neovim state, and temporary files. |

The Rust module roots such as `app.rs`, `grid.rs`, `nvim.rs`, and `gui.rs`
remain public entry points for their respective module trees; implementation
details live in the adjacent directories after the refactor.

The reusable native backend is in `src/rime.rs`; it does not depend on GPUI,
Neovim, or the input router. It loads `rime_get_api` dynamically and keeps
Rime's shared data and user data separate from `~/Library/Rime`; its internal
staging data is kept in `build/` below the user data directory. librime's
optional prebuilt-data fallback is left unset so librime can use its default
`${shared_data_dir}/build` location; nvim-gpui does not create or expose a
separate prebuilt directory.

The backend integration test is ignored by default because it requires a
native librime and data installation. Run it explicitly inside the development
shell:

~~~sh
NVIM_GPUI_RIME_LIBRARY=/path/to/librime.dylib \
NVIM_GPUI_RIME_SHARED_DIR=/path/to/rime-data \
cargo test --lib -- --ignored --nocapture
~~~

The smoke test includes both an intentionally unbound key (F35) and an
unbound modified key (Control+F35); both must return `false` from
`process_key`, which is the signal used to forward the original event to
Neovim once. It also sends the default `Shift_L` press/release pair and checks
`RimeStatus.is_ascii_mode`, because Rime's ASCII mode switch can change state
while still returning `false` from `process_key`. GPUI reports a modifier-only
press through `ModifiersChangedEvent`, so the input router reconstructs these
Rime press/release events instead of waiting for a `KeyDownEvent` that GPUI
does not emit for a standalone Shift.

When `NVIM_GPUI_RIME_SHARED_DIR` is set, the application initializes the
backend, but Rime remains disabled until it is selected or activated
explicitly. The library is taken from `NVIM_GPUI_RIME_LIBRARY` or the platform
bundle search path. The application stores its Rime user data below the
nvim-gpui application-support directory; it never uses `~/Library/Rime` or
the `NVIM_GPUI_RIME_USER_DIR` override.
Deployment is automatic when the internal `build/` directory is empty and can
be forced with `NVIM_GPUI_RIME_DEPLOY=1`.

## Architecture

- `src/main.rs` parses process-level startup arguments and starts GPUI.
- `src/app.rs` and `src/app/` own the application entity, lifecycle, windows,
  layout, compositor state, settings integration, and Neovim event dispatch.
- `src/clipboard.rs` owns GPUI system clipboard access, `nvim_paste` text
  insertion, and the remote clipboard provider bridge.
- `src/nvim.rs` and `src/nvim/` own embedded/remote MessagePack-RPC, versioned
  redraw decoding through `compat.rs`, environment selection, transport, and
  child-process lifecycle.
- `src/grid.rs` and `src/grid/` contain the logical cell model and the single
  custom `GridElement`. The model retains one logical cell per terminal
  position, coalesces ordinary neighboring text into shaped lines, and paints
  Unicode/wide cells without creating one GPUI element per cell.
- `src/gui.rs` and `src/gui/` contain the standalone Settings and About
  windows; shared controls such as the path editor live in `src/widgets.rs`.
- `src/input.rs` is the `InputRouter` boundary for Neovim, system IME, and the
  native Rime backend in `src/rime.rs`.
- `src/image_store.rs` owns Kitty Graphics Protocol transfers, placements,
  placeholders, and bounded image-cache eviction.
- `src/platform.rs` contains macOS font registration, Dock icon setup, and
  platform-specific window behavior.
- `src/settings.rs` persists user settings independently from Neovim.
- `src/logging.rs` configures the `log` facade and the bounded asynchronous
  `flexi_logger` file logger.
- `src/helper.rs` and `src/bin/gpvim.rs` implement the Rust `gpvim` launcher
  used by the AppBundle and CLI installation flow.

## Neovim protocol coverage

The current client attaches linegrid and multigrid UI support and maps the
following redraw areas into the application model:

- grid creation, resize, clear, destroy, line updates, scrolling, and cursor
  movement;
- normal split positions and floating-grid positions/visibility, including
  Neovim's exact `compindex` order on newer versions and the legacy `zindex`
  ordering used by Neovim 0.10/0.11;
- native message/cmdline grid positioning through `msg_set_pos`;
- floating-window `blend` attributes, including the `winblend` value that
  Neovim folds into the final highlight attributes;
- `win_viewport` and `win_viewport_margins` state for each window grid;
- `mode_info_set`, `mode_change`, and cursor blink/shape information;
- `hl_attr_define`, `default_colors_set`, and the main text attributes;
- the initial `Normal`/`NormalFloat` theme snapshot and later theme changes,
  applied to the main window background and custom titlebar at `flush`;
- `option_set`, including `guifont`, `guifontwide`, and `linespace`;
- `set_title`, `set_icon`, and `ui_send` for image data.

The client is intentionally still an early implementation. Mouse input,
complete command-line/message rendering, richer Kitty composition, reconnect
behavior, and broader redraw coverage remain future slices.

## Graceful window close

The main window registers `Window::on_window_should_close`. For embedded
Neovim, it returns `false` while it asynchronously asks Neovim for modified
buffers through `nvim_exec_lua`. The confirmation prompt is a GPUI overlay
rendered above the editor; it is not an `ext_window` surface and does not
participate in Neovim's grid layout.

For Unix-socket and TCP connections, closing the client returns immediately
without querying or changing any remote buffers. nvim-gpui then exits and
drops only its RPC connection; the remote Neovim server remains running.

The prompt offers three paths:

- `Cancel` hides the prompt and keeps the session alive;
- `Save All & Quit` runs `:wall`, then closes the frontend and the embedded
  Neovim process; and
- `Discard & Quit` runs `:qa!` for embedded Neovim.

When the modified-buffer query fails, the prompt remains open and displays the
error instead of silently discarding data. The `quit_on_window_close` setting
continues to bypass this flow when disabled.

## Clipboard

The main window handles the configured paste shortcut (Cmd-V by default) by
reading text from the local GPUI system clipboard and calling Neovim's
`nvim_paste` API. This path is shared by embedded and remote sessions and keeps
multiline paste inside Neovim's mode-aware paste handling.

Remote sessions also register `nvim_gpui_clipboard_get` and
`nvim_gpui_clipboard_set` request handlers. After the handlers are advertised,
the client installs a remote `g:clipboard` provider in Neovim. Consequently,
remote `+` and `*` register operations read and write the local GUI clipboard;
embedded sessions leave Neovim's normal local provider unchanged.

The paste shortcut is persisted in the application settings file as
`paste_shortcut=cmd-v`, `paste_shortcut=ctrl-v`, or `paste_shortcut=disabled`.
It can also be changed from the Settings window.

## System IME

System text input is exposed through GPUI's `EntityInputHandler`. The
`InputRouter` selects Rime for Insert, command-line, prompt, and terminal
contexts when Rime is enabled. When Rime is disabled, those text-input
contexts use the system IME instead. Normal mode remains owned by Neovim.
When Rime is active, committed text is sent back through `nvim_paste`.

Do not send every key event to both the system IME and Neovim. When the target
is `InputTarget::SystemIme`, printable keys, space, and keys reported as being
in IME composition are left to the platform input handler. Otherwise the
character can be committed once by `KeyDownEvent` and a second time by the
IME callback. Control, navigation, editing, and mode-switch keys such as
Escape, Enter, Backspace, arrows, and modified keys are still forwarded to
Neovim when the active backend does not consume them. `InputTarget::Rime` is
backed by the native backend. Its preedit is converted to the same
`grid::ImeComposition` used by the system IME, while its candidates are drawn
as an app-owned overlay above the Neovim compositor. Navigation keys are sent
to librime while it consumes the composition; unconsumed keys continue to
Neovim.

The platform may present an IME using no-inline composition or inline
composition. Both cases enter through the GPUI input-handler callbacks, but
the client-side state is handled as follows:

- `replace_and_mark_text_in_range` updates a transient `SystemImeState` for
  the preedit text. It does not modify Neovim or `GridModel`.
- The inline preedit is merged during `GridElement`'s cell paint pass. It is
  not a fake Neovim cell and does not use the underlying cell's virtual-text
  highlight; it uses normal text attributes and its own marked-text style.
- The caret position is measured from the shaped preedit prefix, so it moves
  with the IME selection rather than remaining at the original cell.
- `replace_text_in_range` forwards only the committed text to Neovim once,
  then clears the transient state. Neovim remains the authority for the
  actual grid contents, so preedit text must never be inserted into the
  Neovim buffer manually.
- `unmark_text` cancels the transient composition without sending text.

The Rime path does not use `EntityInputHandler` for key input: `KeyDownEvent`
and modifier-only `ModifiersChangedEvent` events are translated to librime
keysyms, and the returned context drives the shared inline composition
renderer. The candidate popup is intentionally separate
from Neovim's `compindex`/`zindex` layers because it is owned by the GUI.

GPUI exposes UTF-16 ranges to the platform. `SystemImeState` stores UTF-8
byte ranges internally and performs the conversion at the input boundary.
This distinction must be preserved when changing IME callbacks or rendering
the marked range.

Multigrid coordinate handling is intentional. `cursor_grid` is the last
cursor grid committed at `flush`; `pending_cursor_grid` belongs to the current
redraw batch. `ime_input_grid` identifies the painted grid element that owns
the active system IME handler, and is therefore separate from both pending
state and the general cursor lookup. `ime_cursor_position()` must read the
cursor from `ime_input_grid` and return coordinates local to that grid. The
handler then combines those local coordinates with its element bounds, which
already contain the grid's screen placement. Grid movement, scrolling,
resizing, font metric changes, and mode changes mark the IME coordinates
dirty; the next painted handler calls
`Window::invalidate_character_coordinates()`.

Relevant regression tests include:

- `input::tests::system_ime_state_round_trips_utf16_ranges`;
- `input::tests::system_ime_owns_printable_keys_but_not_control_keys`;
- `app::tests::ime_cursor_position_uses_the_registered_grid`; and
- `app::tests::cursor_grid_is_committed_only_at_flush`.

The current IME path is implemented and tested on macOS. Other platform
backends are not yet supported by the project.

## Logging and diagnostics

The application uses the `log` facade with `flexi_logger`. Logging is
initialized in `src/main.rs` before the installation check, working-directory
setup, Neovim startup, and GPUI application launch. If the logger cannot be
started, the application reports the problem on stderr and continues without
file logging.

By default, macOS logs are written to:

```text
~/Library/Application Support/nvim-gpui/logs/
```

Set `NVIM_GPUI_LOG_DIR` to override the directory. The current file is
rotated at 10 MiB and five rotated files are retained. `WriteMode::Async` is
used so logging does not block the UI or RPC path. The logger handle remains
alive for the lifetime of `main`, allowing the asynchronous writer to flush
when the application exits.

The default level is `off`. The Settings → `Application behavior` panel can change the
level at runtime and persists `Off`, `Error`, `Warn`, `Info`, `Debug`, or
`Trace`. `RUST_LOG` still overrides the initial level for development and uses
the normal `log` filter syntax. Useful diagnostics include:

```sh
RUST_LOG=nvim_gpui=debug gpvim --debug-window
RUST_LOG=nvim_gpui::ime=trace,nvim_gpui::state=debug gpvim
```

The main targets are `nvim_gpui::startup`, `nvim_gpui::nvim`,
`nvim_gpui::state`, `nvim_gpui::ime`, and `nvim_gpui::input`. IME logging
records lifecycle events, byte lengths, UTF-16 ranges, grid IDs, and cursor
coordinates, but not the raw input text. Avoid adding per-cell or per-key
`info` logs; use `debug` or `trace` for high-frequency diagnostics and keep
payloads bounded.

## Testing an external Neovim configuration

The development shell includes a repository profile, but the GUI can also use
the normal configuration. From outside the repository, use the built helper
or binary and select the system Neovim explicitly when needed:

```sh
NVIM_GPUI_NVIM="$(command -v nvim)" \
  /path/to/nvim-gpui --embed --clean
```

For a Nix-wrapped Neovim, pass the absolute wrapper path with
`--nvim-command` or set `NVIM_GPUI_NVIM`. This preserves the wrapper's runtime
environment instead of attempting to execute the underlying store binary in
isolation.

The AppBundle imports the macOS login-shell environment before starting
Neovim. This is needed because Finder and LaunchServices do not normally
inherit the interactive shell's complete `PATH` and Neovim-related variables.

To test an already-running remote Neovim instance, use `--connect` and set
`--connect-timeout` when the endpoint may be unavailable. The timeout is a
positive number of seconds and applies to TCP connections; it defaults to
three seconds:

```sh
gpvim --connect 127.0.0.1:16662 --connect-timeout 3
```

## Packaging

### Built-in Rime runtime

The native Rime backend and the application-private macOS runtime are
implemented and verified. The macOS AppBundle carries librime and a small
starter-data set, so the packaged application does not need a system or Nix
librime installation at runtime. Windows has the same private-runtime layout,
and its x86_64 builder and directory bundle are built and smoke-tested by the
release workflow. Linux initially uses a system librime; a bundled Linux
runtime is reserved for a future self-contained package.

The packaging policy is:

- macOS ships a private librime runtime and curated starter data;
- Windows has the same private-runtime layout; the current release contract
  validates and publishes x86_64 bundles from a native Windows runner;
- Linux uses a system librime for now;
- the runtime includes librime's dependent libraries and any dynamically
  loaded modules, not only the main library file;
- the starter `rime-data` set is read-only and intentionally small; user
  dictionaries and user schemas remain in nvim-gpui's application data
  directory; and
- `build/` remains librime's internal staging directory below the
  application-owned Rime user-data directory and is not a user setting;
  nvim-gpui does not create a separate `prebuilt/` directory.

When no explicit path is supplied, the runtime resolver checks the
development environment override, the application bundle, and supported
system locations; an explicit path is used on its own. On macOS and Windows,
application startup asks it for the bundled runtime; on Linux, the Settings
paths and automatic system discovery
remain available. The bundled runtime layout is described by
`packaging/rime/runtime.toml`, and `scripts/rime_runtime.py` stages and
validates platform artifacts. The macOS source builder, runtime staging, and
AppBundle integration are complete. The Windows source builder is a
PowerShell wrapper around librime's official `install-boost.bat` and
`build.bat` flow; the release workflow runs it on a native Windows runner.

Settings follow the packaging boundary: macOS and Windows display the bundled
librime and shared-data paths as read-only values, while Linux keeps those two
paths configurable for system installations. On every platform, Rime user
data is fixed at the nvim-gpui application-support directory's `rime/`
subdirectory and can be opened from Settings. The old user-data setting and
`NVIM_GPUI_RIME_USER_DIR` environment override are ignored.

The Nix development shell exposes nixpkgs' `rime-data` only as the default
starter-data build input through `NVIM_GPUI_RIME_STARTER_DATA`. The builders
run `scripts/rime_starter_data.py`, which selects the luna-pinyin schema and
its required dictionaries/configuration from that package instead of copying
all available schemas. The selected data is copied into the staged artifact;
the application never uses the Nix store path at runtime.

A staged runtime has this contract:

```text
rime-runtime/
├── lib/       # librime and its runtime libraries
├── modules/   # optional external dynamically loaded librime modules
└── data/      # read-only curated starter Rime data
```

The macOS build enables merged plugins, so the librime plugins shipped by the
source tree are linked into `lib/librime.1.16.1.dylib`. Consequently,
`modules/` is normally empty for the current macOS runtime; it is retained in
the contract for future external plugins. The versioned library names keep
their symlink relationships:

```text
lib/librime.1.16.1.dylib       # regular file
lib/librime.1.dylib -> librime.1.16.1.dylib
lib/librime.dylib -> librime.1.dylib
```

Use `just rime-runtime SOURCE` to copy an already-built artifact into
`.cache/rime-runtime`, or `just rime-runtime-check` to validate an existing
staging directory. On macOS, run:

```sh
just rime-runtime-macos
just bundle-macos
```

The macOS builder uses merged plugins and static third-party dependencies,
defaults to a universal arm64/x86_64 dylib, and rejects Nix/Homebrew runtime
paths. Set `NVIM_GPUI_RIME_STARTER_DATA=/path/to/curated-data` only when a
different curated data source is needed. The starter data is a build input,
not the user's Rime directory; user dictionaries remain in the
application-private user-data directory. `just bundle-macos` copies the validated
runtime into the AppBundle while preserving library symlinks. `just dmg`
then creates the compressed macOS package.

On Windows, run `just rime-runtime-windows` from a PowerShell-capable
development environment with CMake, Git, Python 3.11+, and the Visual
Studio/LLVM toolchain required by librime. The builder pins the same librime
revision as macOS, invokes librime's official dependency and library build
targets, uses static third-party dependencies, and stages `rime.dll` with the
starter data. If `NVIM_GPUI_RIME_STARTER_DATA` is not set, it downloads the
four pinned official Rime data archives listed in
`packaging/rime/starter-data.toml`, verifies their SHA-256 digests, and caches
them below the librime build directory. Set `NVIM_GPUI_RIME_STARTER_DATA` to a
local data directory for an offline or custom build. Set
`NVIM_GPUI_RIME_WINDOWS_ARCH` when the default `x64` target is not appropriate.
If Boost is not already cached, librime's official Boost installer may also
require `aria2c` and `7z`.

After staging the runtime, `just bundle-windows` creates a Windows directory
bundle at `.cache/windows/nvim-gpui` locally, or at the workflow's isolated
artifact directory in CI:

```text
.cache/windows/nvim-gpui/
├── nvim-gpui.exe
├── gpvim.exe
└── rime/
    ├── lib/rime.dll
    ├── modules/       # optional dynamic modules
    └── data/          # read-only starter Rime data
```

The `rime/` location is intentional: the runtime resolver searches beside the
Windows executable, so this layout is also the clean-environment bundle
contract. `just installer-windows` packages this directory bundle with Inno
Setup and writes the installer to
`dist/windows/nvim-gpui-<version>-setup.exe`. The installer targets the
`x86_64-pc-windows-msvc` application and uses `x64compatible`, so it can run on
x64 Windows and Windows 11 on Arm through x64 emulation. Code signing is not
included yet.

`just smoke-windows` runs both bundled executables with `--version` and checks
that they start successfully. `just smoke` selects `smoke-macos` or
`smoke-windows` for the current operating system. `pack-windows` includes this
smoke test and creates both the directory bundle and Inno Setup installer.

On macOS, run `just rime-runtime-macos` first. `just bundle-macos` validates the
staged runtime and copies it into the AppBundle; it does not copy user data or
silently fall back to a system librime. It creates:

```text
.cache/macos/nvim-gpui.app/
├── Contents/MacOS/nvim-gpui
├── Contents/Resources/gpvim
├── Contents/Resources/rime/lib/librime.1.16.1.dylib
├── Contents/Resources/rime/lib/librime.1.dylib -> librime.1.16.1.dylib
├── Contents/Resources/rime/lib/librime.dylib -> librime.1.dylib
├── Contents/Resources/rime/modules/        # usually empty with merged plugins
├── Contents/Resources/rime/data/...
├── Contents/Resources/neovim-gpui_1024x1024_1024x1024.icns
└── Contents/Info.plist
```

The checked-in rounded ICNS file is declared by `Info.plist`; no generated
icon step is required. The bundle step strips unused Nix dylib load commands
on macOS and fails if a Mach-O image or bundled file still references
`/nix/store`. `just dmg`
places the AppBundle and an
`/Applications` shortcut into a compressed UDZO image at
`.cache/macos/nvim-gpui-aarch64.dmg` on Apple Silicon or
`.cache/macos/nvim-gpui-x86_64.dmg` on Intel.

The AppBundle declares source and text document types with
`LSHandlerRank=Alternate`, so it can appear in Finder's Open With menu
without taking ownership of existing source-file icons or defaults. GPUI's
platform open-URL callback converts those file URLs into Neovim `:edit`
requests, including files opened while the application is already running.

GPUI also normalizes native file drops from macOS, Windows, X11, and Wayland
into `ExternalPaths`. nvim-gpui handles those drops at the workspace boundary:
embedded sessions pass files and directories to Neovim's structured `:edit`
request, while remote sessions show a local-path warning and do not forward
the paths over RPC. The editor suppresses the synthetic mouse events GPUI uses
while dispatching an external drop, so a drop cannot become an accidental
Neovim mouse press, release, or movement. On Windows, the native drag feedback
may show `+copy`; that is the platform's default drop effect and does not mean
that nvim-gpui copies file contents.

The Windows installer registers the `nvim-gpui` ProgID under the wildcard
`OpenWithProgids` key. This adds nvim-gpui to Explorer's Open With menu for any
file without replacing an existing default association; the portable ZIP does
not install that registry entry. The installer also offers to add its install
directory to the current user's PATH, which makes both `nvim-gpui` and `gpvim`
available from new terminals. Because the installer is per-user, it does not
modify the machine-wide PATH.

## Debugging

The debug window is hidden by default. Add `--debug-window` when diagnosing
RPC, font, input, image, or grid state:

```sh
just run -- --debug-window
```

For image issues, check the Neovim side with `:checkhealth snacks` and verify
that `SNACKS_KITTY=1` is set before Snacks initializes. For RPC lifecycle
issues, verify that Neovim exits through the normal `nvim_exit` path rather
than leaving the GUI process alive.

## GitHub Actions

`.github/workflows/ci.yml` runs on pushes and pull requests. It checks macOS
arm64, Linux x86_64/arm64, and Windows x86_64. The macOS job also builds and
smoke-tests the private Rime runtime, AppBundle, and DMG. The Linux jobs use
native runners rather than the local Docker/QEMU path.

`.github/workflows/release.yml` runs on `v*` tags. It validates release
metadata and changelog entries, builds both macOS targets on Apple Silicon
runners (the Intel target uses the `x86_64-darwin` Nix shell under Rosetta),
validates and packages native Linux x86_64/arm64 `.deb` files through the
Ubuntu Docker task, and builds/tests the Windows x86_64 bundle and installer.
The publish job runs only after every platform job has succeeded, attaches all
five platform packages plus the optional portable archives, and uploads a
`SHA256SUMS` file. Re-running the workflow is idempotent for an existing
GitHub Release.

Release signing and notarization are intentionally not configured because
they require project-specific platform credentials. The release contract is
currently macOS arm64/x86_64, Linux arm64/x86_64, and Windows x86_64.

Keep `Cargo.lock` and `flake.lock` in pull requests. Before submitting a
change, run `nix develop -c just ci`; on macOS packaging changes should also
be checked with `nix develop -c just pack-macos`.
