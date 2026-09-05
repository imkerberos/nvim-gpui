# Development Guide

This document contains repository and contributor notes. For installation,
Neovim configuration, image configuration, and user-facing limitations, see
the root [README](../README.md).

## Development environment

The repository uses a Nix flake and `direnv`. The flake follows the
`nixos-26.05` nixpkgs channel and provides Rust, GPUI's native build
dependencies, Neovim, `lazy.nvim`, `snacks.nvim`, the Markdown Tree-sitter
parser, ImageMagick, CMake, `just`, and `gnumake`.

```sh
direnv allow
nix develop
```

The shell keeps generated data in the checkout:

- `.cache/cargo-target` is `CARGO_TARGET_DIR`.
- `.cache/cargo-home` is `CARGO_HOME`.
- `.cache/nvim-*` contains the repository Neovim data, state, and cache.
- `tmp/` is `TMPDIR` for compiler and build-script temporary files.

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

## just tasks

Run tasks from the Nix development shell:

```sh
just fmt          # format Rust sources
just fmt-check    # check formatting without changing files
just check        # formatting and cargo check
just clippy       # Clippy with warnings denied
just test         # all Rust tests
just ci           # fmt-check, clippy, and test
just run          # launch the development GUI
just bundle       # build and verify .cache/macos/nvim-gpui.app on macOS
just dmg          # build the arch-named compressed macOS DMG on macOS
just rime-runtime-macos # build and validate the pinned macOS librime runtime
just rime-runtime-windows # build and validate the pinned Windows runtime
just bundle-windows # build the Windows directory bundle
just rime-runtime-check # validate a staged application-private Rime runtime
just release-prepare 0.2.0  # synchronize release version metadata
just release-check v0.2.0    # validate metadata and changelog before tagging
just release-notes v0.2.0    # preview the GitHub Release notes
```

`Cargo.toml` is the canonical version source. Before creating a release, run
`just release-prepare VERSION`, add the matching section to `CHANGELOG.md`,
then run `just release-check vVERSION`. The script synchronizes `Cargo.lock`,
the macOS AppBundle metadata, and the Homebrew Cask. The release workflow
repeats the check and uses the matching changelog section as the GitHub Release
body.

`Makefile` forwards the common tasks to `just` for environments where a Make
entry point is more convenient.

The development Neovim profile is at
`config/nvim-gpui/init.lua`. It loads the Nix-provided plugins without cloning
or downloading them. Its current test profile enables `snacks.image`, the
Markdown parser, and the Kitty capability fallback used by the GUI.

The reusable native backend is in `src/rime.rs`; it does not depend on GPUI,
Neovim, or the input router. It loads `rime_get_api` dynamically and keeps
Rime's shared data and user data separate from `~/Library/Rime`; its internal
prebuilt data and staging data are derived as `prebuilt/` and `build/` below
the user data directory.

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
nvim-gpui application-support directory, unless `NVIM_GPUI_RIME_USER_DIR` is
set; it never uses `~/Library/Rime`.
Deployment is automatic when the internal `build/` directory is empty and can
be forced with `NVIM_GPUI_RIME_DEPLOY=1`.

## Architecture

- `src/main.rs` parses process-level startup arguments and starts GPUI.
- `src/app.rs` owns the application state, windows, layout, settings, and
  Neovim event dispatch.
- `src/clipboard.rs` owns GPUI system clipboard access, `nvim_paste` text
  insertion, and the remote clipboard provider bridge.
- `src/nvim.rs` owns embedded/remote MessagePack-RPC, redraw decoding,
  environment selection, and child-process lifecycle.
- `src/grid.rs` contains the logical cell model and the single custom
  `GridElement`. It retains one logical cell per terminal position, coalesces
  ordinary neighboring text into shaped lines, and paints Unicode/wide cells
  without creating one GPUI element per cell.
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
  Neovim's exact `compindex` order and configured `zindex`;
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

## Packaging

### Built-in Rime runtime — in progress

The native Rime backend is integrated, but packaging librime with the
application is not complete yet. This work must not be described as a shipped
feature until the runtime artifacts and clean-environment smoke tests pass on
the target platforms.

The target packaging policy is:

- macOS and Windows ship a private librime runtime with nvim-gpui;
- Linux initially uses a system librime, with a bundled runtime reserved for a
  future self-contained package;
- the runtime includes librime's dependent libraries and dynamically loaded
  modules, not only the main library file;
- a small read-only starter `rime-data` set is shipped with the application;
  user dictionaries and user schemas remain in nvim-gpui's application data
  directory;
- `prebuilt/` and `build/` remain librime's internal directories below the
  application-owned Rime user-data directory and are not user settings.

The runtime resolver now uses an explicit Settings path first, then the
`NVIM_GPUI_RIME_LIBRARY` development override, then the application bundle on
macOS/Windows, and finally platform system paths where supported. The bundled
runtime layout is described by `packaging/rime/runtime.toml`, and
`scripts/rime_runtime.py` can stage and validate a platform artifact. The
macOS source builder and AppBundle integration are now implemented. The
Windows source builder is implemented as a PowerShell wrapper around
librime's official `install-boost.bat` and `build.bat` flow, but it still needs
real compilation and clean-environment verification on a Windows host. Until
then, development and the ignored backend smoke test continue to use
`NVIM_GPUI_RIME_LIBRARY` and `NVIM_GPUI_RIME_SHARED_DIR`.

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
├── modules/   # optional dynamically loaded librime modules
└── data/      # read-only curated starter Rime data
```

Use `just rime-runtime SOURCE` to copy an already-built artifact into
`.cache/rime-runtime`, or `just rime-runtime-check` to validate an existing
staging directory. On macOS, `just rime-runtime-macos` builds the pinned
librime source and stages it, provided
`NVIM_GPUI_RIME_STARTER_DATA=/path/to/curated-data` is set. The macOS builder
uses merged plugins and static third-party dependencies, defaults to a
universal arm64/x86_64 dylib, and rejects Nix/Homebrew runtime paths. The
starter data is a build input, not the user's Rime directory; user dictionaries
remain in the application-private user-data directory. These tasks validate
the runtime layout; the macOS `bundle` task copies the validated runtime into
the AppBundle, and the Windows `bundle-windows` task copies it into a
directory bundle.

On Windows, run `just rime-runtime-windows` from a PowerShell-capable
development environment with CMake, Git, Python 3.11+, and the Visual
Studio/LLVM toolchain required by librime. The builder pins the same librime
revision as macOS, invokes librime's official dependency and library build
targets, uses static third-party dependencies, and stages `rime.dll` with the
starter data. Set `NVIM_GPUI_RIME_WINDOWS_ARCH` when the default `x64` target
is not appropriate. If Boost is not already cached, librime's official Boost
installer may also require `aria2c` and `7z`.

After staging the runtime, `just bundle-windows` creates a Windows directory
bundle at `.cache/windows/nvim-gpui`:

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
Windows executable, so this layout is also the first clean-environment bundle
contract. It is a directory bundle rather than an installer and still needs
Windows-host validation, archive/installer integration, and code signing.

On macOS, run `just rime-runtime-macos` first. `just bundle` validates the
staged runtime and copies it into the AppBundle; it does not copy user data or
silently fall back to a system librime. It creates:

```text
.cache/macos/nvim-gpui.app/
├── Contents/MacOS/nvim-gpui
├── Contents/Resources/gpvim
├── Contents/Resources/rime/lib/librime.dylib
├── Contents/Resources/rime/data/...
├── Contents/Resources/neovim-gpui_1024x1024_1024x1024.icns
└── Contents/Info.plist
```

The checked-in rounded ICNS file is declared by `Info.plist`; no generated
icon step is required. The bundle step strips unused Nix dylib load commands
on macOS and fails if an executable still references `/nix/store`. `just dmg`
places the AppBundle and an
`/Applications` shortcut into a compressed UDZO image at
`.cache/macos/nvim-gpui-aarch64.dmg` on Apple Silicon or
`.cache/macos/nvim-gpui-x86_64.dmg` on Intel.

The AppBundle declares source and text document types with
`LSHandlerRank=Alternate`, so it can appear in Finder's Open With menu
without taking ownership of existing source-file icons or defaults.

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

`.github/workflows/ci.yml` runs `just ci` and builds/verifies the macOS
AppBundle for pushes and pull requests. Linux is not tested or supported yet.
`.github/workflows/release.yml` runs on `v*` tags, builds both macOS targets on
Apple Silicon runners (the Intel target uses the `x86_64-darwin` Nix shell
under Rosetta), verifies that each bundle has no Nix store runtime dependency,
uploads arch-specific workflow artifacts, builds the Windows directory bundle
on `windows-latest`, and attaches both macOS packages and the Windows ZIP to a
GitHub Release. Release signing and notarization are intentionally not
configured because they require project-specific platform credentials.

Keep `Cargo.lock` and `flake.lock` in pull requests. Before submitting a
change, run `nix develop -c just ci`; on macOS packaging changes should also
be checked with `nix develop -c just bundle` or `nix develop -c just dmg`.
