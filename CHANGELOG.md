# Changelog

All notable changes to nvim-gpui are documented here.

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
