# Development Guide

This document contains repository and contributor notes. For installation,
Neovim configuration, image configuration, and user-facing limitations, see
the root [README](../README.md).

## Development environment

The repository uses a Nix flake and `direnv`. The flake follows the
`nixos-26.05` nixpkgs channel and provides Rust, GPUI's native build
dependencies, Neovim, `lazy.nvim`, `snacks.nvim`, the Markdown Tree-sitter
parser, ImageMagick, `just`, and `gnumake`.

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
```

`Makefile` forwards the common tasks to `just` for environments where a Make
entry point is more convenient.

The development Neovim profile is at
`config/nvim-gpui/init.lua`. It loads the Nix-provided plugins without cloning
or downloading them. Its current test profile enables `snacks.image`, the
Markdown parser, and the Kitty capability fallback used by the GUI.

## Architecture

- `src/main.rs` parses process-level startup arguments and starts GPUI.
- `src/app.rs` owns the application state, windows, layout, settings, and
  Neovim event dispatch.
- `src/nvim.rs` owns embedded/remote MessagePack-RPC, redraw decoding,
  environment selection, and child-process lifecycle.
- `src/grid.rs` contains the logical cell model and the single custom
  `GridElement`. It retains one logical cell per terminal position, coalesces
  ordinary neighboring text into shaped lines, and paints Unicode/wide cells
  without creating one GPUI element per cell.
- `src/input.rs` is the `InputRouter` boundary for Neovim, system IME, and the
  future Rime backend.
- `src/image_store.rs` owns Kitty Graphics Protocol transfers, placements,
  placeholders, and bounded image-cache eviction.
- `src/platform.rs` contains macOS font registration, Dock icon setup, and
  platform-specific window behavior.
- `src/settings.rs` persists user settings independently from Neovim.
- `src/helper.rs` and `src/bin/gpvim.rs` implement the Rust `gpvim` launcher
  used by the AppBundle and CLI installation flow.

## Neovim protocol coverage

The current client attaches linegrid and multigrid UI support and maps the
following redraw areas into the application model:

- grid creation, resize, clear, destroy, line updates, scrolling, and cursor
  movement;
- normal split positions and floating-grid positions/visibility;
- `mode_info_set`, `mode_change`, and cursor blink/shape information;
- `hl_attr_define`, `default_colors_set`, and the main text attributes;
- `option_set`, including `guifont`, `guifontwide`, and `linespace`;
- `set_title`, `set_icon`, and `ui_send` for image data.

The client is intentionally still an early implementation. Mouse input,
clipboard integration, complete command-line/message rendering, richer Kitty
composition, reconnect behavior, and broader redraw coverage remain future
slices.

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

On macOS, `just bundle` creates:

```text
.cache/macos/nvim-gpui.app/
├── Contents/MacOS/nvim-gpui
├── Contents/Resources/gpvim
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
`.github/workflows/release.yml` runs on `v*` tags, builds the macOS AppBundle
and DMG on both Apple Silicon and Intel runners, verifies that the bundle has
no Nix store runtime dependency, uploads arch-specific workflow artifacts, and
attaches both DMGs and AppBundle archives to a GitHub Release. Release signing
and notarization are intentionally not configured because they require
project-specific Apple credentials.

Keep `Cargo.lock` and `flake.lock` in pull requests. Before submitting a
change, run `nix develop -c just ci`; on macOS packaging changes should also
be checked with `nix develop -c just bundle` or `nix develop -c just dmg`.
