<p align="center">
  <img src="assets/icons/neovim-gpui.png" alt="nvim-gpui icon" width="128">
</p>

# nvim-gpui

A Rust and GPUI graphical frontend for Neovim.

<code>nvim-gpui</code> is an early-stage Neovim GUI client. It connects to
Neovim over MessagePack-RPC, renders its grid with GPUI, supports Unicode and
wide CJK cells, and implements the Kitty Graphics Protocol for image-capable
Neovim plugins such as <code>snacks.nvim</code>.

> This project is experimental. It is useful for trying a native GPUI
> frontend and for developing the rendering and protocol layers, but it is not
> yet a drop-in replacement for Neovide or a terminal UI.

## Highlights

- Rust application built with [GPUI](https://gpui.rs/).
- Embedded Neovim by default, with TCP and Unix-socket connections available.
- Unicode-aware grid rendering with wide-character and grapheme support.
- <code>guifont</code> and <code>guifontwide</code> aware cell metrics and
  bundled Nerd Font fallbacks.
- One custom <code>GridElement</code> instead of one UI element per cell.
- Multigrid, split windows, floating windows, highlights, cursor styles, and
  an elastic jelly cursor.
- Kitty Graphics Protocol image transfers and placements for image plugins.
- Native macOS AppBundle, Dock icon, <code>gpvim</code> helper, and DMG
  packaging.

## Quick start

Install Neovim and nvim-gpui with Homebrew:

~~~sh
brew install neovim
brew tap imkerberos/nvim-gpui https://github.com/imkerberos/nvim-gpui.git
brew install --cask imkerberos/nvim-gpui/nvim-gpui
gpvim
~~~

The tap contains architecture-aware downloads for Apple Silicon and Intel.
The Cask also installs the <code>gpvim</code> helper, which launches the
AppBundle and preserves the caller's working directory. To pass Neovim
arguments:

~~~sh
gpvim --clean README.md
gpvim --debug-window
gpvimdiff file1 file2
~~~

<code>gpvimdiff</code> is a symlink to the same helper as
<code>gpvim</code>; its name enables Neovim's diff mode.

To open the installed application directly:

~~~sh
open -a nvim-gpui
~~~

## Temporary trust for unsigned builds

The current macOS builds are unsigned because the project does not yet have an
Apple Developer ID certificate and notarization setup. When an unsigned app is
downloaded from GitHub, macOS may report that it is damaged or cannot be
verified.

Only do this for a build downloaded from a source you trust. After moving the
app to `/Applications`, remove its download quarantine attribute and launch it:

~~~sh
xattr -dr com.apple.quarantine /Applications/nvim-gpui.app
open /Applications/nvim-gpui.app
~~~

Replace the path if you installed the app elsewhere. This is a temporary
Gatekeeper override, not a code signature; it may need to be repeated after
installing a newer build. Proper public distribution will require Developer
ID signing and Apple notarization.

## Required Neovim font configuration

For predictable cell width and CJK alignment, configure both
<code>guifont</code> and <code>guifontwide</code> in your Neovim
configuration. Do not configure only <code>guifont</code>: the GUI needs an
explicit wide-glyph face for CJK, full-width punctuation, and other two-cell
characters.

Add this to <code>init.lua</code>, replacing the families with fonts installed
on your system:

~~~lua
vim.opt.guifont = "Iosevka Term Slab:h16"
vim.opt.guifontwide = "LXGW WenKai:h16"
~~~

<code>guifont</code> supplies the normal monospace cell metrics.
<code>guifontwide</code> supplies wide glyphs while preserving their two-cell
logical footprint. The <code>:h16</code> suffix is the Neovim GUI font-size
syntax; use the same size for both faces unless you have a deliberate reason
not to.

If <code>guifont</code> is empty, nvim-gpui can select a verified system
monospace font, but explicit configuration is recommended for reproducible
layout. If <code>guifontwide</code> is omitted, the normal face is used as a
fallback and CJK glyph metrics may vary with the platform's font fallback.

## Selecting a GUI-specific theme

Neovim's UI protocol can report that a UI is attached, but that includes a
terminal UI as well as a graphical UI. nvim-gpui therefore exposes both
<code>vim.g.nvim_gpui = true</code> and <code>NVIM_GPUI=1</code> to its
embedded Neovim process before <code>init.lua</code> is loaded. Use the global
for startup-time theme selection:

~~~lua
local is_nvim_gpui = vim.g.nvim_gpui == true

if is_nvim_gpui then
  vim.cmd.colorscheme("your-gui-theme")
else
  vim.cmd.colorscheme("your-terminal-theme")
end
~~~

The environment form is also available if you prefer not to rely on a global:

~~~lua
local is_nvim_gpui = vim.env.NVIM_GPUI == "1"
~~~

This marker is guaranteed for embedded sessions started by nvim-gpui. When
using <code>--connect</code> with an already-running Neovim, startup has
already happened, so choose the theme in that Neovim process or use a
<code>UIEnter</code> hook for attach-time behavior.

## Required snacks.image terminal (`TERM`) fallback

nvim-gpui embeds Neovim instead of giving it a real terminal file descriptor.
Therefore Snacks may not identify the Kitty-capable GUI through its normal
terminal response query. Set the explicit Snacks terminal fallback **before
Snacks initializes**:

~~~lua
-- Required for an embedded Neovim session in nvim-gpui.
vim.env.SNACKS_KITTY = "1"

require("lazy").setup({
  {
    "folke/snacks.nvim",
    priority = 1000,
    opts = {
      image = {
        enabled = true,
        -- Keep this false: nvim-gpui advertises Kitty support explicitly.
        force = false,
        doc = {
          enabled = true,
          inline = true,
          float = true,
        },
      },
    },
  },
})
~~~

The equivalent shell fallback is:

~~~sh
SNACKS_KITTY=1 gpvim path/to/file.md
~~~

<code>SNACKS_KITTY=1</code> is Snacks' explicit terminal/`TERM` detection
fallback; setting `TERM` alone is not sufficient for this embedded session.
It is not a request to render images in an unsupported terminal. With a Nix
development shell, the repository already exports this value for its test
profile. Image formats other than PNG may also require ImageMagick on the
Neovim side.

See [Snacks' image documentation](https://github.com/folke/snacks.nvim/blob/main/docs/image.md)
for the plugin's current options and supported document types.

## Using your normal Neovim configuration

The repository development shell has an isolated Neovim profile under
<code>config/nvim-gpui</code>. To test your normal configuration, run the GUI
from outside this repository, or select your Neovim wrapper explicitly:

~~~sh
NVIM_GPUI_NVIM="$(command -v nvim)" \
  /path/to/nvim-gpui --embed
~~~

For a Nix-wrapped Neovim, use the absolute wrapper path with
<code>--nvim-command</code> or <code>NVIM_GPUI_NVIM</code>. This preserves the
wrapper's runtime environment and avoids launching the underlying store binary
without its runpath and environment setup.

## Command-line options

~~~text
--debug-window       Show the auxiliary debug window
--no-debug-window    Hide the auxiliary debug window
--embed              Start a local embedded Neovim (default)
--connect ADDRESS    Connect to a Neovim RPC socket
--nvim-command PATH  Select the local Neovim executable for embed mode
--cwd PATH           Set Neovim's working directory
--                  Pass all following arguments to Neovim
~~~

<code>ADDRESS</code> may be <code>HOST:PORT</code>,
<code>tcp:HOST:PORT</code>, <code>unix:/path</code>, or a Unix socket path.
Unknown arguments are passed through to embedded Neovim. <code>gpvim</code>
starts the macOS AppBundle through LaunchServices and forwards the caller's
working directory.

## Current limitations

The project is still being developed. The largest incomplete areas are mouse
input, clipboard integration, complete command-line and message rendering,
richer Kitty composition and animation, reconnect behavior, and broader
redraw coverage.

## License

MIT. See [Cargo.toml](Cargo.toml) for the package metadata.
