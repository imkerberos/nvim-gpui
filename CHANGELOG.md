# Changelog

All notable changes to nvim-gpui are documented here.

## [0.5.0] - 2026-09-05

### Added

- Built-in native Rime input support with preedit text, candidate popup,
  candidate navigation, modifier-key handling, and commit routing across
  Insert, command-line, prompt, and terminal contexts.
- Rime status indicator and context menu in the custom titlebar, including
  redeploy and opening the application-private user data directory.
- Rime settings for backend selection, candidate layout, activation shortcut,
  runtime detection, configuration testing, and platform-specific runtime
  paths.
- GPUI-owned unsaved-changes confirmation when the main window is closed.

### Improved

- Rime user data is isolated under nvim-gpui's application-support directory;
  the application does not use `~/Library/Rime` or expose librime's internal
  build directories as settings.
- macOS bundles now carry a private librime runtime and curated read-only
  starter data, while Linux keeps system librime paths available.
- Settings path editors support caret movement, mouse selection, paste, and
  clearer runtime test and restart feedback.
- Runtime validation checks bundled files, symlinks, Mach-O dependencies, and
  accidental Nix store references.

### Packaging

- Added reproducible librime runtime staging and macOS AppBundle/DMG flows,
  with a Windows runtime and directory-bundle builder available for local
  validation.
- Windows packaging remains excluded from CI/CD until it can be validated on a
  Windows host.

## [0.4.0] - 2026-09-04

### Improved

- Refactored multigrid rendering around an explicit compositor frame with
  shared layer geometry, clipping, paint order, and semantic layer context.
- Routed mouse hit testing through compositor layers, including topmost float
  selection and drag/release capture for the original grid.
- Centralized highlight resolution for default colors, floating surfaces,
  reverse, blend, dim, cursor, decorations, and inline composition text.
- Separated Neovim's logical cell spans from glyph metrics; wide cells now
  follow protocol continuation markers and Nerd Font symbols no longer consume
  adjacent padding cells.
- Calculated IME cursor offsets from Unicode grapheme and display-width rules,
  including the `ambiwidth` and `emoji` UI options.

## [0.3.0] - 2026-09-04

### Added

- Inline system IME composition with marked text and caret tracking across
  multigrid layouts.
- Local system clipboard integration through Neovim's `nvim_paste` API.
- Remote clipboard bridging for Neovim sessions connected over RPC.
- Configurable paste shortcut, including Cmd-V, Ctrl-V, and disabled states.
- Bounded asynchronous file logging with `flexi_logger`.
- Settings UI for fonts, fallback behavior, startup state, image cache size,
  paste shortcut, and the command-line helper.

### Improved

- Redraw state is applied atomically at `flush`, with stronger protocol and
  session lifecycle coverage.
- IME coordinates follow the active cursor grid instead of assuming the main
  grid.
- GUI, widget, Neovim window, grid, and application state modules are more
  clearly separated.
- Development documentation now describes the IME, clipboard, and logging
  behavior and the current macOS-only platform scope.

## [0.2.0] - 2026-09-03

### Added

- RPC request/reply support for communication with Neovim.
- Automatic reconnection after an unexpected Neovim exit or lost connection.
- Mouse input support.
- Viewport rendering for scrolling and partially visible windows.
- Global jelly cursor animation, including movement between split windows.

### Improved

- Floating-window rendering with better layering, transparency, and clipping.
- Image previews in floating windows.
- Startup sizing and redraw behavior to reduce visible flicker.
- Theme synchronization for the editor surface and custom titlebar.
- Internal application, grid, and Neovim module organization.
- Input processing no longer relies on continuous polling.

## [0.1.0] - 2026-09-02

Initial public release.
