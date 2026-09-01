# nvim-gpui

A GPUI-based graphical frontend for Neovim, written in Rust.

This repository starts with a deliberately small, runnable scaffold. The
window starts an embedded Neovim process by default, consumes its initial UI
redraw, and renders the resulting screen through a custom `GridElement` backed
by a small screen-grid model. It can also attach to an already running Neovim
through its MessagePack-RPC TCP or Unix socket endpoint.

The rendering slice keeps Neovim's logical grid separate from visual spans:

- `GridCell` represents one logical cell and can be a wide-character lead,
  continuation, blank, or ordinary text cell.
- `VisualSpanBuilder` merges adjacent cells with the same highlight, maps a
  wide-character pair to one shaped span, and optionally merges a Nerd Font
  symbol followed by a same-highlight space.
- `GridElement` uses GPUI's text system for shaping and paints each span at its
  logical grid position.

The initial transport, redraw synchronization, and `guifont` selection are
now in place. An explicitly configured `guifont` is used as-is; when it is
empty or not set, the runtime font list is searched for a verified monospace
family. `hl_attr_define` RGB attributes are stored by highlight id and applied
during GPUI text shaping, including foreground,
background, reverse video, bold, italic, underline, undercurl, and
strikethrough. `default_colors_set` supplies the defaults used by incomplete
highlight definitions. `grid_cursor_goto` paints a rounded block cursor with a
short eased, direction-aware jelly transition; a cursor on a wide-character
pair covers both logical cells. The animation is drawn as one additional quad
inside `GridElement`, not as one element per cell.

The current Neovim window is intentionally only the grid surface: the Explorer
and bottom statusbar are removed. Debug information is shown in a separate
top-level floating window, so it cannot change the grid's width. `GridElement`
requests its exact logical size from the active font's monospace advance and
the current RPC grid. Resizing the Neovim window converts its content size back
into grid columns and rows through `nvim_ui_try_resize`.

The debug window is hidden by default; pass `--debug-window` when diagnosing
RPC, font, IME, or grid state.

On macOS and Windows the native titlebar is made transparent and a themed
top-level bar uses the same background as the grid. Windows also gets custom
titlebar hit areas for dragging, minimizing, maximizing, and closing. Neovim's
`set_title` redraw event updates both the native window title and this themed
titlebar; the repository-local debug config enables `'title'` so this can be
seen while testing.

## Development

All development tools are provided by the Nix flake:

```sh
direnv allow
nix develop
just check
just test
just run
just bundle       # macOS AppBundle
gpvim file         # launch the AppBundle on macOS
# The Makefile forwards to the same justfile tasks.
make check
```

`.envrc` calls `use flake`, so after the one-time `direnv allow`, entering the
repository automatically loads the locked Nix development environment. The
explicit `nix develop` command remains useful for CI and non-interactive
shells. The shell also adds the Cargo debug target directory to `PATH`, making
the Rust `gpvim` helper available after the first build.

The development shell exports `NVIM_APPNAME=nvim-gpui`, records the absolute
Nix `nvim` wrapper in `NVIM_GPUI_NVIM`, and keeps the repository paths in the
custom `NVIM_GPUI_CONFIG_DIR` and `NVIM_GPUI_CACHE_DIR` variables. It does not
export `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`, or
`XDG_CACHE_HOME`, so entering this shell does not change how Git and other
tools find the user's files. When the application starts an embedded Neovim,
it injects those XDG variables only into the child process; Neovim then loads
`config/nvim-gpui/` without touching the user's normal Neovim profile.
Neovim's data, state, and cache directories are placed under `.cache/`;
Cargo's target and registry cache are also under `.cache/`, and compiler
temporary files are under `tmp/`. Generated directories are ignored by Git
and can be removed as a project-local cleanup operation.

`just run` starts the wrapper selected by `NVIM_GPUI_NVIM` with `--embed`,
performs the MessagePack-RPC handshake, identifies the client as a
UI, and attaches an initial line grid. `grid_resize`, `grid_clear`,
`grid_destroy`, `grid_line`, `grid_cursor_goto`, `grid_scroll`, and `flush` are
mapped into the Rust cell model. `mode_info_set` and `mode_change` select
block, horizontal, or vertical cursor shapes and their blink timings; the
jelly transition remains one additional quad inside `GridElement`. `option_set`
retains all UI options and applies the font, wide-font, `linespace`, and
shaping-affecting options immediately. Highlight attributes cover the
foreground/background, reverse, dim/blend, text styles, conceal, blink,
overline, and underline variants; `altfont` and `url` are retained as metadata
for the future font and mouse layers. `set_icon` is decoded and retained by the
client model. GPUI 0.2.2 has no cross-platform runtime window-icon setter, so
it does not yet replace the macOS Dock/Finder icon.

In Insert and Command-line modes, the focused grid registers GPUI's system
input handler; marked text stays in the local IME state and committed text is
forwarded to Neovim. The first `guifont` family and `:hN` size are applied to
the grid; when no font is configured, a verified system monospace family is
selected instead. The font's monospace advance is used as the logical cell
width and the row height scales with the requested size. Rime remains an
optional future backend.

Useful tasks are listed with `just --list`. `just ci` runs formatting,
Clippy, and tests.

The application reserves `--help`/`-h`, `--version`/`-V`,
`--debug-window`, `--no-debug-window`, `--embed`, `--connect`, and
`--nvim-command` for GPUI. Every other command-line argument is passed to the
embedded Neovim process unchanged; `--` forces arguments with a GPUI-looking
name to be passed through. For example:

```sh
cargo run -- --clean README.md
cargo run -- --no-debug-window -- ~/notes/today.md
gpvim --nvim-command /nix/store/.../bin/nvim --clean README.md
gpvim --connect unix:/tmp/nvim.sock
gpvim --connect tcp:127.0.0.1:6666
```

`--connect` attaches to an existing Neovim and therefore does not accept local
Neovim arguments. Embed mode uses `--nvim-command PATH` first, then
`NVIM_GPUI_NVIM`, and finally `nvim` from `PATH`. `gpvim` also passes its
current directory as the internal `--cwd` option because LaunchServices may
start an AppBundle with `/` as its process working directory.

On macOS, `just bundle` builds both Rust binaries and creates
`.cache/macos/nvim-gpui.app` with a valid `Info.plist`. The `gpvim` helper is
stored at `Contents/Resources/gpvim`; it opens that AppBundle with
LaunchServices (`open -n`) instead of directly creating a window, so it starts
a separate application instance. Before opening it, `gpvim` resolves the
current shell's `nvim` to an absolute path; this is important for Nix-wrapped
Neovim. Set `NVIM_GPUI_APP` when the bundle has been moved to another location.

`gpvim` is a Rust binary rather than a shell script, so the same helper can be
shipped inside the AppBundle without depending on Bash or the caller's shell.
When the running app finds no executable `gpvim` in `PATH`, it looks for the
bundled helper and attempts to create `/usr/local/bin/gpvim` as a symlink. An
existing path is never overwritten; if that directory is not writable, the
app continues to work and reports the installation error.

A GUI application launched by Finder/LaunchServices does not inherit the
interactive shell's complete environment. The AppBundle detects this launch
context and imports the user's macOS login-shell environment before spawning
Neovim, while preserving repository-specific `NVIM_APPNAME` and
`NVIM_GPUI_*` values. The repository-local `XDG_*` paths are then injected
only into the embedded Neovim child, rather than into the GUI process or the
user's shell. This is also why a Nix-wrapped `nvim` can report missing
runpaths or runtime files when launched by a GUI: the wrapper is not merely a
binary; it supplies environment and runtime paths that are normally prepared
by the shell. `NVIM_GPUI_NVIM` or `--nvim-command` makes the wrapper selection
explicit.

The root grid captures all key events that reach the application and routes
Neovim-owned control, navigation, function, Alt, Ctrl, and platform-modifier
keys through `nvim_input`. Printable text in Insert mode remains with GPUI's
system IME so composition is not committed twice. OS-global shortcuts such as
macOS Cmd-Tab/Cmd-Space or Windows Win-L/Ctrl-Alt-Del are owned by the window
system and cannot be blocked by an ordinary application; they never reach
Neovim-GPUI's event dispatcher.

The GPUI dependency is pinned in `Cargo.toml` to the current published
version used by this scaffold. Development uses GPUI's `runtime_shaders`
feature so macOS builds do not require the optional Xcode-only `metal`
command-line tool; `Cargo.lock` is checked in so dependency resolution
remains reproducible.

The input and image boundaries are reserved in `src/input.rs` and
`src/image_store.rs`. `InputRouter` selects between Neovim, the system IME,
and the future Rime backend based on editor context. The current system-input
slice registers GPUI's `EntityInputHandler`, keeps marked text locally, and
forwards committed text and control keys to Neovim. `ImageStore` currently
stores protocol-neutral assets and grid-anchored placements; Kitty parsing and
GPUI image decoding are not enabled yet.

## Planned slices

1. Coalesce resize requests during continuous window dragging.
2. Expand input, commands, events, and multi-window behavior.
3. Add Kitty graphics parsing and image placement behind `ImageStore`.

## Current client boundary

The project is already a usable minimum Neovim GUI client for an early
editing loop: it can embed or attach to Neovim, attach the UI, render the
single grid with Unicode/wide-cell handling, apply basic highlights, show the
cursor, resize the grid, route system-IME text, and terminate with the Neovim
session. It is not yet a daily-driver replacement for Neovide or a terminal
UI. The largest missing slices are mouse and clipboard support, complete
command-line/message/popup rendering, multi-grid and split-window support,
full redraw event coverage, robust key/IME composition, Kitty graphics, and
connection/error/reconnect handling.
