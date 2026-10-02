<p align="center">
  <img src="assets/icons/neovim-gpui.png" alt="nvim-gpui icon" width="128">
</p>

<h1 align="center">nvim-gpui</h1>

Another GPU-rendered Neovim client for a native, cross-platform desktop
experience.

`nvim-gpui` brings Neovim's editing engine, configuration, and plugin ecosystem
to a focused desktop application. Built with GPUI, it renders Neovim's linegrid
and multigrid UI directly instead of embedding a terminal, while integrating
with the host platform for input methods, clipboard, file opening, drag and
drop, and images.

The core editing workflow is ready for everyday use. Neovim remains the
editor engine; nvim-gpui focuses on rendering, native input, and desktop
integration. Advanced protocol and plugin-specific integrations continue to
evolve as the project grows.

macOS is currently the most mature platform. Release packages are also
available for Linux and Windows, with platform-specific validation and
integration continuing across all three targets.

<p align="center">
  <img src="assets/screenshots/editor-cjk.png" alt="CJK text editing in nvim-gpui" width="32%">
  <img src="assets/screenshots/nerd-fonts.png" alt="Nerd Font rendering in nvim-gpui" width="32%">
  <img src="assets/screenshots/snacks-picker.png" alt="Snacks picker image preview in nvim-gpui" width="32%">
</p>

See [CHANGELOG.md](CHANGELOG.md) for release history.

## Features

- Native linegrid and multigrid rendering, including floating windows.
- Embedded Neovim sessions and connections to an already-running Neovim.
- Compatibility with the versioned UI payloads used by Neovim 0.10, 0.11, and
  0.12+.
- Unicode, CJK, wide-character, and grapheme-aware text rendering.
- Bundled Nerd Font support and configurable `guifont`/`guifontwide` handling.
- Kitty image support for plugins such as `snacks.nvim`.
- System IME support and a built-in Rime input method with a private runtime on
  macOS and Windows, plus system librime support on Linux.
- Local and remote clipboard integration through Neovim's paste and provider
  APIs.
- Native file opening, Open With integration, and drag and drop across desktop
  platforms.

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

To launch the installed application directly from a terminal:

```sh
open -a nvim-gpui
```

### Debian/Ubuntu

Download the `.deb` package matching your CPU architecture, install Neovim,
then install nvim-gpui:

- [ARM64 `.deb`](https://github.com/imkerberos/nvim-gpui/releases/latest/download/nvim-gpui-latest-debian-aarch64.deb)
- [X86_64 `.deb`](https://github.com/imkerberos/nvim-gpui/releases/latest/download/nvim-gpui-latest-debian-x86_64.deb)

```sh
sudo apt update
sudo apt install neovim
nvim --version                 # must be 0.10.0 or newer
sudo apt install ./nvim-gpui-latest-debian-x86_64.deb
gpvim
```

Use the ARM64 filename for ARM64 systems. If your distribution provides an
older Neovim, install a newer version from the [official Neovim
releases](https://github.com/neovim/neovim/releases) before launching
nvim-gpui. The package installs the required Ubuntu/Debian GUI dependencies and
recommends the system Rime dependencies for the built-in Rime backend. It also
registers nvim-gpui as an Open With option for common source and text files;
the desktop entry passes selected files to the embedded Neovim session.

### Fedora

Download the RPM matching your CPU architecture, install Neovim, then install
nvim-gpui:

- [ARM64 RPM](https://github.com/imkerberos/nvim-gpui/releases/latest/download/nvim-gpui-latest-fedora-aarch64.rpm)
- [X86_64 RPM](https://github.com/imkerberos/nvim-gpui/releases/latest/download/nvim-gpui-latest-fedora-x86_64.rpm)

```sh
sudo dnf install neovim
nvim --version                 # must be 0.10.0 or newer
sudo dnf install ./nvim-gpui-latest-fedora-x86_64.rpm
gpvim
```

Use the ARM64 filename on ARM64 systems. The RPM declares the Fedora GUI,
font, graphics, and Rime runtime dependencies.

### Arch Linux

Download the x86_64 package from the [latest
release](https://github.com/imkerberos/nvim-gpui/releases/latest), install
Neovim, then install nvim-gpui:

```sh
sudo pacman -S neovim
sudo pacman -U ./nvim-gpui-latest-arch-x86_64.pkg.tar.zst
gpvim
```

The current release provides an x86_64 package. ARM64 Arch packages are not
currently provided.

### Windows

Install Neovim 0.10.0 or newer from the [official Neovim
releases](https://github.com/neovim/neovim/releases) first, and make sure
`nvim.exe` is available on `PATH`. The nvim-gpui package does not include
Neovim itself.

Download and run the [latest Windows
installer](https://github.com/imkerberos/nvim-gpui/releases/latest/download/nvim-gpui-latest-windows-x86_64-setup.exe). The
installer creates Start Menu and optional desktop shortcuts, and adds
nvim-gpui to Explorer's Open With menu for files without changing existing
default associations. It also offers to add the installed `nvim-gpui` and
`gpvim` commands to the current user's PATH. You can also download the [latest
portable ZIP](https://github.com/imkerberos/nvim-gpui/releases/latest/download/nvim-gpui-latest-windows-x86_64.zip),
extract it, and run `nvim-gpui.exe` directly.

The Windows package targets x86_64 Windows and also works on Windows 11 on
Arm through its x64 application compatibility layer.

After installation, start nvim-gpui from the application launcher or use the
command-line helper where it is available:

```sh
gpvim
```

Open a file or pass arguments to Neovim from a terminal:

```sh
gpvim README.md
gpvim --clean README.md
gpvimdiff file1 file2
```

`gpvimdiff` opens Neovim in diff mode.

## Drag and drop

In embedded mode, drag files or directories from Finder, a Linux file manager,
or Windows Explorer into the nvim-gpui window to open them in Neovim. Remote
sessions started with `--connect` accept the drop only to explain that opening
local files and directories is not supported; local paths are never sent to
the remote Neovim process.

## Release packages

Download the package matching your operating system and CPU architecture from
the [latest release](https://github.com/imkerberos/nvim-gpui/releases/latest):

| Target | ARM64 | X86_64 |
| --- | --- | --- |
| <img src="https://cdn.simpleicons.org/apple" alt="macOS" width="18" height="18"> macOS | [DMG](https://github.com/imkerberos/nvim-gpui/releases/latest/download/nvim-gpui-latest-macos-aarch64.dmg)<br>[App ZIP](https://github.com/imkerberos/nvim-gpui/releases/latest/download/nvim-gpui-latest-macos-aarch64.app.zip) | [DMG](https://github.com/imkerberos/nvim-gpui/releases/latest/download/nvim-gpui-latest-macos-x86_64.dmg)<br>[App ZIP](https://github.com/imkerberos/nvim-gpui/releases/latest/download/nvim-gpui-latest-macos-x86_64.app.zip) |
| <img src="https://cdn.simpleicons.org/ubuntu/E95420" alt="Ubuntu / Debian" width="18" height="18"> Ubuntu / Debian | [DEB](https://github.com/imkerberos/nvim-gpui/releases/latest/download/nvim-gpui-latest-debian-aarch64.deb) | [DEB](https://github.com/imkerberos/nvim-gpui/releases/latest/download/nvim-gpui-latest-debian-x86_64.deb) |
| <img src="https://cdn.simpleicons.org/fedora/51A2DA" alt="Fedora" width="18" height="18"> Fedora | [RPM](https://github.com/imkerberos/nvim-gpui/releases/latest/download/nvim-gpui-latest-fedora-aarch64.rpm) | [RPM](https://github.com/imkerberos/nvim-gpui/releases/latest/download/nvim-gpui-latest-fedora-x86_64.rpm) |
| <img src="https://cdn.simpleicons.org/archlinux/1793D1" alt="Arch Linux" width="18" height="18"> Arch Linux | — | [Package](https://github.com/imkerberos/nvim-gpui/releases/latest/download/nvim-gpui-latest-arch-x86_64.pkg.tar.zst) |
| <img src="https://cdn.simpleicons.org/windows/0078D4" alt="Windows" width="18" height="18"> Windows | — | [Installer](https://github.com/imkerberos/nvim-gpui/releases/latest/download/nvim-gpui-latest-windows-x86_64-setup.exe)<br>[Portable ZIP](https://github.com/imkerberos/nvim-gpui/releases/latest/download/nvim-gpui-latest-windows-x86_64.zip) |

All links point to the latest GitHub Release. Ubuntu/Debian, Fedora, and Arch
Linux packages use the system GUI and input-method libraries; install a
compatible Neovim version separately. The Windows release is an x86_64 build
and relies on Windows 11 on Arm's x64 application compatibility layer when
used on ARM64 hardware.

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
runtime is selected automatically when the application is launched from the
macOS AppBundle or Windows bundle. Development builds also search the repository's
`.cache/rime-runtime` staging directory; a different development runtime can
be selected with `NVIM_GPUI_RIME_RUNTIME=/path/to/rime-runtime`.

## Font configuration

Font selection is available in Settings → `Font and image`. nvim-gpui uses the
selected font chains for its own grid renderer. Each font name is a token: the
arrow keys move between tokens, and Backspace/Delete remove one whole token.
Click the empty part of the editor to open the installed-font candidate list;
fonts already present in the chain remain visible and are marked as selected.
Normal, italic, bold, and bold-italic faces are derived automatically from the
same chain.

Neovim's `guifont` and `guifontwide` options are parsed as ordered comma-
separated chains and appended as compatibility fallbacks, so the same Neovim
configuration can continue to work in both a terminal and nvim-gpui.

The Settings controls are:

- `Font size`: the shared grid size. The available values are 10, 11, 12, 13,
  14, 15, 16, 18, 20, 22, 24, 28, and 32 px. It applies to the primary
  regular font, the primary wide-character font, and bundled Nerd Font glyphs.
- `guifont`: an ordered chain of installed system monospace fonts used for the
  regular grid. An empty chain uses the platform preference.
- `guifontwide`: an ordered chain of installed Unicode-capable fonts used for
  wide characters. An empty chain uses the platform/system preference.
  The list tests actual glyph coverage; it does not decide whether a font is
  suitable from `CJK` or another substring in its family name. Fonts such as
  LXGW WenKai can therefore appear in the list.
- `Nerd font`: choose between the bundled Symbols Nerd Font and Symbols Nerd
  Font Mono. No Nerd Font installation is required.
- `Fallback mode`: choose whether bundled Nerd Font glyphs are disabled,
  selected automatically when the primary font lacks a glyph, or always used
  for Nerd Font cells.

For reliable text and CJK alignment, you can also set both `guifont` and
`guifontwide` in your Neovim configuration. Replace the font names with fonts
installed on your system:

```lua
vim.opt.guifont = "Iosevka Term Slab,JetBrainsMono Nerd Font"
vim.opt.guifontwide = "LXGW WenKai,Noto Sans CJK SC"
```

If these options are not set, nvim-gpui still selects an installed platform
font at runtime. The default preference order is:

| Platform | Primary regular font | Primary wide-character font |
| --- | --- | --- |
| macOS | Menlo, SF Mono, Monaco | PingFang SC, Hiragino Sans GB |
| Windows | Cascadia Mono, Consolas, Courier New | Microsoft YaHei UI, Microsoft YaHei, SimSun |
| Linux | Ubuntu Mono, Noto Sans Mono, DejaVu Sans Mono, Liberation Mono, Source Code Pro | Noto Sans Mono CJK SC, Noto Sans CJK SC, WenQuanYi Zen Hei |

The first installed font that passes the relevant font check is selected. If
none of the preferred fonts is available, nvim-gpui searches the installed
font collection for a suitable regular or Unicode-capable family. Each entry
in an explicit Neovim `guifont` or `guifontwide` chain is parsed in order and
used as a fallback for the corresponding configured chain; if `guifontwide` is
absent, Neovim's full `guifont` chain is also considered for wide-character
fallback. Font names containing commas or colons may use Neovim's backslash
escaping rules.

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

## Image terminal-size fallback

Both `snacks.image` and [image.nvim](https://github.com/3rd/image.nvim) may
receive zero or missing pixel dimensions when Neovim is embedded in
nvim-gpui. The following function can be used after the image plugin has been
configured; it patches only the terminal-size query and updates the fallback
when the GPUI window is resized:

```lua
local function setup_image_terminal_fallback()
  if vim.g.nvim_gpui ~= true then return end

  local cell_width = tonumber(vim.env.NVIM_GPUI_CELL_WIDTH) or 9
  local cell_height = tonumber(vim.env.NVIM_GPUI_CELL_HEIGHT) or 18
  local image_size
  local snacks_size
  local needs_resize_hook = false

  local function is_positive_finite(value)
    return type(value) == "number" and value > 0 and value < math.huge
  end

  local function has_cell_metrics(size)
    return size and is_positive_finite(size.cell_width)
      and is_positive_finite(size.cell_height)
  end

  local function update_size()
    local columns = vim.o.columns
    local rows = vim.o.lines
    image_size = {
      screen_x = columns * cell_width,
      screen_y = rows * cell_height,
      screen_cols = columns,
      screen_rows = rows,
      cell_width = cell_width,
      cell_height = cell_height,
    }
    snacks_size = {
      width = columns * cell_width,
      height = rows * cell_height,
      columns = columns,
      rows = rows,
      cell_width = cell_width,
      cell_height = cell_height,
      scale = math.max(1, cell_width / 8),
    }
  end

  local image_ok, image_terminal = pcall(require, "image/utils/term")
  if image_ok and type(image_terminal.get_size) == "function" then
    local size_ok, native_size = pcall(image_terminal.get_size)
    if not size_ok or not has_cell_metrics(native_size) then
      needs_resize_hook = true
      image_terminal.get_size = function() return image_size end
    end
  end

  local snacks_ok, snacks_terminal = pcall(require, "snacks.image.terminal")
  if snacks_ok and type(snacks_terminal.size) == "function" then
    local size_ok, native_size = pcall(snacks_terminal.size)
    if not size_ok or not has_cell_metrics(native_size) then
      needs_resize_hook = true
      snacks_terminal.size = function() return snacks_size end
    end
  end

  if needs_resize_hook then
    update_size()
    vim.api.nvim_create_autocmd("VimResized", { callback = update_size })
  end
end

setup_image_terminal_fallback()
```

The fallback is compatible with either plugin: absent modules are ignored, and
only the provider whose terminal-size result is invalid is replaced.

### image.nvim status

`image.nvim` is currently not supported in embedded nvim-gpui. Its Kitty
backend writes graphics data directly to stdout, while embedded Neovim uses
stdout for the MsgPack-RPC connection. Changing `kitty_method` to
`unicode-placeholders` changes placement semantics but does not change this
output channel. The development configuration therefore currently uses
`snacks.image`; no image.nvim patch is included in the flake.

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
--health-check       Report OS and graphics capabilities, then exit
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

## Update checks

Open Settings → `Application behavior` → `Updates` to check for a newer stable
release or open its download page. Automatic checks are enabled by default and
run at most once per day after startup. They do not block the editor, download
files, or send application data.

## Current limitations

The core editing and desktop integration paths are usable today. Some advanced
Neovim UI protocol details and third-party plugin features may not yet behave
exactly like they do in a terminal or in other Neovim GUI clients; these areas
continue to be developed and validated across platforms.

## License

nvim-gpui is distributed under the [MIT License](LICENSE).
