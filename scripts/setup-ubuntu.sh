#!/usr/bin/env bash
set -euo pipefail

# Prepare an Ubuntu desktop VM for nvim-gpui runtime and IME testing.
#
# This installs two deliberately separate input paths:
#   - Ubuntu's system IME: IBus with the libpinyin engine;
#   - nvim-gpui's built-in Rime backend: librime and Rime data.
#
# ibus-rime is intentionally not installed. It would register Rime as a
# system IBus engine and make it impossible to test the two paths separately.

minimum_neovim_version="${NVIM_GPUI_MIN_NEOVIM_VERSION:-0.10.0}"
neovim_version="${NVIM_GPUI_NEOVIM_VERSION:-0.12.5}"
neovim_prefix="${NVIM_GPUI_NEOVIM_PREFIX:-/opt/nvim-gpui}"
configure_ibus="${NVIM_GPUI_CONFIGURE_IBUS:-1}"

usage() {
  cat <<'EOF'
usage: setup-ubuntu.sh

Prepare an Ubuntu desktop VM for nvim-gpui runtime and input-method testing.

environment:
  NVIM_GPUI_NEOVIM_VERSION     Neovim version to install when needed (default: 0.12.5)
  NVIM_GPUI_NEOVIM_SHA256      SHA-256 for a custom Neovim version/architecture
  NVIM_GPUI_NEOVIM_PREFIX      install prefix (default: /opt/nvim-gpui)
  NVIM_GPUI_CONFIGURE_IBUS     set im-config to IBus (default: 1; use 0 to skip)
EOF
}

fail() {
  printf 'Ubuntu setup error: %s\n' "$1" >&2
  exit 1
}

warn() {
  printf 'Ubuntu setup warning: %s\n' "$1" >&2
}

if (($# > 0)); then
  case "$1" in
    --help|-h)
      usage
      exit 0
      ;;
    *)
      usage >&2
      fail "unknown argument: $1"
      ;;
  esac
fi

[[ -r /etc/os-release ]] || fail "/etc/os-release is missing"
# shellcheck disable=SC1091
. /etc/os-release
[[ "${ID:-}" == "ubuntu" ]] || fail "this task requires Ubuntu (detected: ${ID:-unknown})"

if (( EUID == 0 )); then
  root_command=()
else
  command -v sudo >/dev/null 2>&1 || fail "sudo is required when this script is not run as root"
  root_command=(sudo)
fi

[[ "$neovim_prefix" = /* ]] || fail "NVIM_GPUI_NEOVIM_PREFIX must be an absolute path"

"${root_command[@]}" apt-get update

# Ubuntu 24.04 renamed the librime runtime package during the t64 transition.
# Keep the fallback so the setup script remains useful on older Ubuntu VMs.
rime_library_package="librime1t64"
if ! apt-cache show "$rime_library_package" 2>/dev/null | grep -q '^Package:'; then
  rime_library_package="librime1"
fi

runtime_packages=(
  ca-certificates
  curl
  file
  im-config
  ibus
  ibus-libpinyin
  ibus-wayland
  libegl1
  libfontconfig1
  libfreetype6
  libgl1
  libwayland-client0
  libx11-6
  libxcb1
  libxcursor1
  libxi6
  libxkbcommon0
  libxkbcommon-x11-0
  libxrandr2
  librime-data
  rime-data-luna-pinyin
  xdg-utils
  "$rime_library_package"
)

"${root_command[@]}" env DEBIAN_FRONTEND=noninteractive \
  apt-get install --no-install-recommends -y "${runtime_packages[@]}"

version_at_least() {
  local current="$1"
  local required="$2"
  [[ "$(printf '%s\n' "$current" "$required" | sort -V | head -n1)" == "$required" ]]
}

neovim_version_from() {
  local executable="$1"
  "$executable" --version 2>/dev/null \
    | sed -n '1s/^NVIM v//; 1s/^NVIM //; 1s/ .*//p'
}

install_pinned_neovim() {
  local machine="$1"
  local archive_arch
  local expected_sha256="${NVIM_GPUI_NEOVIM_SHA256:-}"

  case "$machine" in
    x86_64)
      archive_arch="x86_64"
      ;;
    aarch64|arm64)
      archive_arch="arm64"
      ;;
    *)
      fail "unsupported Ubuntu architecture for Neovim: $machine"
      ;;
  esac

  # These are the digests of the official Neovim 0.12.5 Linux archives.
  # Custom versions must provide NVIM_GPUI_NEOVIM_SHA256 explicitly.
  if [[ -z "$expected_sha256" && "$neovim_version" == "0.12.5" ]]; then
    case "$archive_arch" in
      arm64)
        expected_sha256="1aa5ca085249580ae0f91eb14f27ec0919773ff2d99a163d03f3d6c21ac29725"
        ;;
      x86_64)
        expected_sha256="bce0f56eda1f1b1db6eee8f4133d7a38813ea07933837dd1777411ca384c6875"
        ;;
    esac
  fi
  [[ "$expected_sha256" =~ ^[0-9a-fA-F]{64}$ ]] \
    || fail "a SHA-256 digest is required for Neovim $neovim_version ($archive_arch)"

  local install_dir="$neovim_prefix/nvim-$neovim_version-$archive_arch"
  local link_path="/usr/local/bin/nvim-gpui-nvim"
  if [[ -x "$install_dir/bin/nvim" ]]; then
    printf 'Neovim %s is already installed at %s\n' "$neovim_version" "$install_dir/bin/nvim"
  else
    local temporary_dir
    temporary_dir="$(mktemp -d)"
    trap 'rm -rf "$temporary_dir"' RETURN
    local archive="$temporary_dir/nvim.tar.gz"
    local url="https://github.com/neovim/neovim/releases/download/v${neovim_version}/nvim-linux-${archive_arch}.tar.gz"

    printf 'Downloading Neovim %s for %s\n' "$neovim_version" "$archive_arch"
    curl --fail --location --retry 3 --proto '=https' --tlsv1.2 \
      "$url" --output "$archive"
    printf '%s  %s\n' "$expected_sha256" "$archive" | sha256sum --check --strict -

    tar -xzf "$archive" -C "$temporary_dir"
    local extracted_dir="$temporary_dir/nvim-linux-$archive_arch"
    [[ -x "$extracted_dir/bin/nvim" ]] || fail "Neovim archive has an unexpected layout"

    "${root_command[@]}" rm -rf "$install_dir"
    "${root_command[@]}" install -d -m 0755 "$neovim_prefix"
    "${root_command[@]}" cp -a "$extracted_dir" "$install_dir"
    trap - RETURN
    rm -rf "$temporary_dir"
  fi

  # Recreate the stable entry point even when the versioned installation was
  # already present from an earlier run.
  "${root_command[@]}" ln -sfn "$install_dir/bin/nvim" "$link_path"
  if ! command -v nvim >/dev/null 2>&1; then
    "${root_command[@]}" ln -sfn "$link_path" /usr/local/bin/nvim
  fi
  printf 'Neovim test executable: %s\n' "$link_path"
}

current_nvim="$(command -v nvim || true)"
current_nvim_version=""
if [[ -n "$current_nvim" ]]; then
  current_nvim_version="$(neovim_version_from "$current_nvim" || true)"
fi

if [[ -n "$current_nvim_version" ]] && version_at_least "$current_nvim_version" "$minimum_neovim_version"; then
  printf 'Neovim %s already satisfies the minimum version %s\n' \
    "$current_nvim_version" "$minimum_neovim_version"
else
  if [[ -n "$current_nvim" ]]; then
    warn "existing Neovim ${current_nvim_version:-version unknown} is older than ${minimum_neovim_version}; it will not be replaced"
  fi
  install_pinned_neovim "$(uname -m)"
  if [[ -n "$current_nvim" ]]; then
    printf 'Use the pinned Neovim with: export NVIM_GPUI_NVIM=/usr/local/bin/nvim-gpui-nvim\n'
  fi
fi

if [[ "$configure_ibus" == "1" ]]; then
  if ! im-config -n ibus >/dev/null 2>&1; then
    warn "could not select IBus with im-config; configure IBus in the Ubuntu desktop settings"
  fi
fi

[[ -d /usr/share/rime-data ]] || fail "Rime data was not installed at /usr/share/rime-data"
ldconfig -p 2>/dev/null | grep -Eq 'librime\.so(\.|[[:space:]])' \
  || fail "the system librime shared library was not found"

printf '\nUbuntu test environment is ready.\n'
printf 'System IME: IBus + libpinyin (not ibus-rime).\n'
printf 'Built-in Rime: %s + librime-data + rime-data-luna-pinyin.\n' "$rime_library_package"
printf 'Log out and back in if the desktop session does not show IBus yet.\n'
printf 'Select System IME and Rime separately in nvim-gpui when testing each path.\n'
