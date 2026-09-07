<p align="center">
  <img src="assets/icons/neovim-gpui.png" alt="nvim-gpui icon" width="128">
</p>

<h1 align="center">nvim-gpui</h1>

A native macOS graphical frontend for Neovim.

`nvim-gpui` is experimental software. It is suitable for trying a native
Neovim editing experience, but it is not yet a complete replacement for
Neovide or a terminal UI.

macOS is currently the primary supported platform. Release packages are also
built for Linux and Windows, but support on those platforms remains
experimental.

<p align="center">
  <img src="assets/screenshots/editor-cjk.png" alt="CJK text editing in nvim-gpui" width="32%">
  <img src="assets/screenshots/nerd-fonts.png" alt="Nerd Font rendering in nvim-gpui" width="32%">
  <img src="assets/screenshots/snacks-picker.png" alt="Snacks picker image preview in nvim-gpui" width="32%">
</p>

See [CHANGELOG.md](CHANGELOG.md) for release history.

## Features

- Unicode and CJK text support.
- Bundled Nerd Font support.
- Image support for plugins such as `snacks.nvim`.
- Built-in Rime input method with a private runtime on macOS and Windows, and
  system librime support on Linux.

## Neovim requirement

nvim-gpui requires Neovim **0.10.0 or newer**. This requirement applies to
both embedded sessions and `--connect` targets. The client adapts the
versioned multigrid UI payloads used by Neovim 0.10, 0.11, and 0.12+.

## Quick start

### macOS

The easiest way to install both Neovim and nvim-gpui is Homebrew:

```sh
brew install neovim
brew tap imkerberos/nvim-gpui https://github.com/imkerberos/nvim-gpui.git
brew install --cask imkerberos/nvim-gpui/nvim-gpui
gpvim
```

You can also download the DMG for your Mac from the [latest
release](https://github.com/imkerberos/nvim-gpui/releases/latest), open it,
and drag `nvim-gpui.app` to `/Applications`. Neovim must be installed
separately and must be version 0.10.0 or newer.

Current macOS builds are unsigned. If macOS reports that the downloaded
application is damaged or cannot be verified, only do the following for an
application downloaded from a source you trust:

```sh
xattr -dr com.apple.quarantine /Applications/nvim-gpui.app
open /Applications/nvim-gpui.app
```

Replace the path if you installed the application elsewhere.

The AppBundle is available in Finder's Open With menu for source and text
files. Opening a file there sends it to the embedded Neovim session, including
when nvim-gpui is already running.

### Debian/Ubuntu

Download the `.deb` package matching your CPU architecture from the [latest
release](https://github.com/imkerberos/nvim-gpui/releases/latest), then install
Neovim and nvim-gpui:

```sh
sudo apt update
sudo apt install neovim
nvim --version                 # must be 0.10.0 or newer
sudo apt install ./nvim-gpui-v<VERSION>-linux-x86_64.deb
gpvim
```

Use `nvim-gpui-v<VERSION>-linux-aarch64.deb` on ARM64. If your distribution
provides an older Neovim, install a newer version from the [official Neovim
releases](https://github.com/neovim/neovim/releases) before launching
nvim-gpui. The package installs the required Ubuntu/Debian GUI and system Rime
dependencies automatically. It also registers nvim-gpui as an Open With option
for common source and text files; the desktop entry passes selected files to
the embedded Neovim session.

### Windows

Install Neovim 0.10.0 or newer from the [official Neovim
releases](https://github.com/neovim/neovim/releases) first, and make sure
`nvim.exe` is available on `PATH`. The nvim-gpui package does not include
Neovim itself.

Download and run
`nvim-gpui-v<VERSION>-windows-x86_64-setup.exe` from the [latest
release](https://github.com/imkerberos/nvim-gpui/releases/latest). The
installer creates Start Menu and optional desktop shortcuts, and adds
nvim-gpui to Explorer's Open With menu for files without changing existing
default associations. It also offers to add the installed `nvim-gpui` and
`gpvim` commands to the current user's PATH. You can also download the
portable ZIP, extract it, and run `nvim-gpui.exe` directly.

The Windows package targets x86_64 Windows and also works on Windows 11 on
Arm through its x64 application compatibility layer.

After installation, start nvim-gpui from the application launcher or use the
command-line helper where it is available:

```sh
gpvim
```

Open a file or pass arguments to Neovim on macOS and Debian/Ubuntu:

```sh
gpvim README.md
gpvim --clean README.md
gpvimdiff file1 file2
```

`gpvimdiff` opens Neovim in diff mode. To open the installed application
directly:

```sh
open -a nvim-gpui
```

## Drag and drop

In embedded mode, drag files or directories from Finder, a Linux file manager,
or Windows Explorer into the nvim-gpui window to open them in Neovim. Remote
sessions started with `--connect` accept the drop only to explain that opening
local files and directories is not supported; local paths are never sent to
the remote Neovim process.

## Release packages

Download the package matching your operating system and CPU architecture from
the [latest release](https://github.com/imkerberos/nvim-gpui/releases/latest):

| Target | Package | Installation |
| --- | --- | --- |
| `darwin-aarch64` | `nvim-gpui-v<VERSION>-darwin-aarch64.dmg` | Open the disk image and copy the app to `/Applications`. |
| `darwin-x86_64` | `nvim-gpui-v<VERSION>-darwin-x86_64.dmg` | Open the disk image and copy the app to `/Applications`. |
| `linux-aarch64` | `nvim-gpui-v<VERSION>-linux-aarch64.deb` | Install with `sudo apt install ./nvim-gpui-*.deb`. |
| `linux-x86_64` | `nvim-gpui-v<VERSION>-linux-x86_64.deb` | Install with `sudo apt install ./nvim-gpui-*.deb`. |
| `windows-x86_64` | `nvim-gpui-v<VERSION>-windows-x86_64-setup.exe` | Run the installer on x64 Windows or Windows 11 on Arm. |

The release also includes App ZIP archives for macOS and a portable ZIP for
Windows. The Linux packages target Ubuntu/Debian and use the system GUI and
librime packages; install a compatible Neovim version separately. The Windows
release is an x86_64 build and relies on Windows 11 on Arm's x64 application
compatibility layer when used on ARM64 hardware.

## Built-in Rime input

The macOS and Windows packages include a private `librime` runtime and a
small, read-only starter data set. User dictionaries and custom schemas are
stored outside the bundle in nvim-gpui's application-support directory; the
bundled Rime data is not `~/Library/Rime`. Linux packages use the system
librime and Rime data installed through the distribution.

To enable it, open Settings → IME:

1. Select `Rime` as the input method.
2. On macOS and Windows, review the read-only librime and Rime data paths
   supplied by the application bundle.
3. Review the read-only user data directory and use `Open` when needed.
4. Click `Test`.

On Linux, the librime and Rime data fields remain configurable because the
system installation supplies those paths. The user data directory always
uses nvim-gpui's application-support directory.

Rime starts disabled even when it is selected as the backend. Press the
platform default activation shortcut (`Cmd-\` on macOS, `Ctrl-\` on Linux and
Windows) to toggle it, then enter Insert mode and type with Rime. The bundled
runtime is selected automatically when
the application is launched from the macOS AppBundle or Windows bundle; do not set
`NVIM_GPUI_RIME_LIBRARY` or `NVIM_GPUI_RIME_SHARED_DIR` when testing that
path.

## Font configuration

For reliable text and CJK alignment, set both `guifont` and `guifontwide` in
your Neovim configuration. Replace the font names with fonts installed on
your system:

```lua
vim.opt.guifont = "Iosevka Term Slab:h16"
vim.opt.guifontwide = "LXGW WenKai:h16"
```

If these options are not set, nvim-gpui uses a system monospace font as a
fallback.

## GUI-specific theme

If you use the same Neovim configuration in a terminal and in nvim-gpui, you
can select a separate theme for the GUI:

```lua
if vim.g.nvim_gpui == true then
  vim.cmd.colorscheme("your-gui-theme")
else
  vim.cmd.colorscheme("your-terminal-theme")
end
```

The equivalent environment check is:

```lua
if vim.env.NVIM_GPUI == "1" then
  vim.cmd.colorscheme("your-gui-theme")
end
```

These checks work for embedded sessions started by nvim-gpui. When using
`--connect`, Neovim has already started, so set the theme in that Neovim
session or use a `UIEnter` autocmd.

## Image previews with snacks.nvim

For image previews in an embedded Neovim session, set `SNACKS_KITTY` before
Snacks loads:

```lua
vim.env.SNACKS_KITTY = "1"

require("lazy").setup({
  {
    "folke/snacks.nvim",
    priority = 1000,
    opts = {
      image = {
        enabled = true,
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
```

The equivalent shell command is:

```sh
SNACKS_KITTY=1 gpvim path/to/file.md
```

See [Snacks' image documentation](https://github.com/folke/snacks.nvim/blob/main/docs/image.md)
for the plugin's current options and supported document types.

## Use your existing Neovim configuration

The repository development shell uses an isolated configuration under
`config/nvim-gpui`. To launch nvim-gpui with your normal Neovim executable:

```sh
NVIM_GPUI_NVIM="$(command -v nvim)" \
  /path/to/nvim-gpui --embed
```

For a Nix-wrapped Neovim, pass the wrapper's absolute path with
`--nvim-command` or `NVIM_GPUI_NVIM`.

## Command-line options

```text
--debug-window       Show the auxiliary debug window
--no-debug-window    Hide the auxiliary debug window
--embed              Start a local embedded Neovim (default)
--connect ADDRESS    Connect to a running Neovim session
--connect-timeout SECONDS  Set the remote TCP connection timeout (default: 3)
--nvim-command PATH  Select the Neovim executable for embed mode
--cwd PATH           Set Neovim's working directory
--                   Pass all following arguments to Neovim
```

`ADDRESS` may be a TCP address such as `HOST:PORT`, or a Unix socket path.
`--connect-timeout` accepts a positive number of seconds, including decimals,
and applies only to remote TCP connections. Unknown arguments are passed
through to embedded Neovim.

## Clipboard

Cmd-V reads the local system clipboard and sends the text through Neovim's
`nvim_paste` API, so multiline and mode-aware paste work in both embedded and
remote sessions. The shortcut can be changed in Settings to Cmd-V, Ctrl-V, or
Disabled.

When using `--connect`, `+` and `*` register operations are bridged to the
local GUI clipboard. On Linux, `*` uses the primary selection; on other
platforms it falls back to the normal system clipboard.

## Logs

Runtime logs are written to `~/Library/Application Support/nvim-gpui/logs` on
macOS. Set `NVIM_GPUI_LOG_DIR` to use another directory. The default log
level is `off`. Choose `Off`, `Error`, `Warn`, `Info`, `Debug`, or `Trace` in
Settings → `Application behavior`. `RUST_LOG` can still override the initial level for
development, for example:

```sh
RUST_LOG=nvim_gpui=debug gpvim
```

Log files rotate at 10 MiB, with five rotated files retained.

## Current limitations

The project is still experimental. Some advanced Neovim UI and third-party
plugin features may not yet behave exactly like they do in a terminal or in
other Neovim GUI clients.

## License

MIT.
