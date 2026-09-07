#!/usr/bin/env bash
set -euo pipefail

# Build an Ubuntu-linked binary and package it as a .deb.
# This script is run as root inside the Ubuntu Docker container started by
# `just pack-linux-x86_64` or `just pack-linux-aarch64`; it deliberately does
# not use the Nix toolchain.

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
deb_arch="${NVIM_GPUI_DEB_ARCH:-amd64}"
rust_target="${NVIM_GPUI_RUST_TARGET:-x86_64-unknown-linux-gnu}"
platform="${NVIM_GPUI_PLATFORM:-x86_64}"
output_dir="${NVIM_GPUI_DEB_OUTPUT:-$repo_root/dist/ubuntu-$platform}"
cargo_target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"

fail() {
  printf 'Ubuntu package error: %s\n' "$1" >&2
  exit 1
}

((EUID == 0)) || fail 'the Docker packaging script must run as root'
[[ -r /etc/os-release ]] || fail '/etc/os-release is missing'
# shellcheck disable=SC1091
. /etc/os-release
[[ "${ID:-}" == 'ubuntu' ]] || fail "this package task requires Ubuntu (detected: ${ID:-unknown})"

[[ -f "$repo_root/Cargo.toml" ]] || fail "Cargo.toml not found below $repo_root"
[[ -f "$repo_root/packaging/linux/nvim-gpui.desktop" ]] \
  || fail 'Linux desktop entry is missing'
icon_source_dir="$repo_root/packaging/linux/icons/hicolor"
[[ -d "$icon_source_dir" ]] || fail 'Linux application icon set is missing'
case "$deb_arch" in
  amd64|arm64) ;;
  *) fail "unsupported Debian architecture: $deb_arch" ;;
esac

export DEBIAN_FRONTEND=noninteractive
# The official Ubuntu image removes downloaded archives after every install.
# Keep the shared Docker APT volume useful across packaging runs.
rm -f /etc/apt/apt.conf.d/docker-clean
apt-get update
apt-get install --no-install-recommends -y \
  build-essential \
  ca-certificates \
  cmake \
  curl \
  desktop-file-utils \
  dpkg-dev \
  file \
  git \
  libegl1-mesa-dev \
  libfontconfig1-dev \
  libfreetype6-dev \
  libgl1-mesa-dev \
  libssl-dev \
  libwayland-dev \
  libx11-dev \
  libxcb1-dev \
  libxcursor-dev \
  libxi-dev \
  libxkbcommon-dev \
  libxkbcommon-x11-dev \
  libxrandr-dev \
  pkg-config \
  python3

export CARGO_HOME="${CARGO_HOME:-/root/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-/root/.rustup}"
export PATH="$CARGO_HOME/bin:$PATH"

if ! command -v rustup >/dev/null 2>&1; then
  printf 'Installing the stable Rust toolchain\n'
  curl --fail --location --retry 3 --proto '=https' --tlsv1.2 \
    https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path
fi
rustup toolchain install stable --profile minimal --no-self-update
rustup default stable

rust_host="$(rustc -vV | sed -n 's/^host: //p')"
[[ "$rust_host" == "$rust_target" ]] \
  || fail "the Docker compiler is not the requested Ubuntu target ($rust_target): $rust_host"

printf 'Building nvim-gpui with the Ubuntu %s toolchain\n' "$deb_arch"
export CARGO_TARGET_DIR="$cargo_target_dir"
cargo build --locked --release --bins

release_dir="$cargo_target_dir/release"
for executable in nvim-gpui gpvim; do
  [[ -x "$release_dir/$executable" ]] \
    || fail "release executable is missing: $release_dir/$executable"
done

version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo_root/Cargo.toml" | head -n 1)"
[[ "$version" =~ ^[0-9]+(\.[0-9]+){2}([+~-][0-9A-Za-z.-]+)?$ ]] \
  || fail "could not parse a Debian-compatible version from Cargo.toml: $version"

temporary_dir="$(mktemp -d)"
trap 'rm -rf "$temporary_dir"' EXIT
package_root="$temporary_dir/nvim-gpui"
mkdir -p \
  "$package_root/DEBIAN" \
  "$package_root/usr/bin" \
  "$package_root/usr/share/applications" \
  "$package_root/usr/share/doc/nvim-gpui"

install -m 755 "$release_dir/nvim-gpui" "$package_root/usr/bin/nvim-gpui"
install -m 755 "$release_dir/gpvim" "$package_root/usr/bin/gpvim"
ln -s gpvim "$package_root/usr/bin/gpvimdiff"
install -m 644 "$repo_root/packaging/linux/nvim-gpui.desktop" \
  "$package_root/usr/share/applications/nvim-gpui.desktop"
mapfile -t icon_files < <(find "$icon_source_dir" -type f -name 'nvim-gpui.png' -print | sort)
((${#icon_files[@]} > 0)) || fail 'Linux application icon set is empty'
for icon_file in "${icon_files[@]}"; do
  relative_path="${icon_file#"$icon_source_dir"/}"
  install -D -m 644 "$icon_file" \
    "$package_root/usr/share/icons/hicolor/$relative_path"
done
install -m 644 "$repo_root/README.md" "$package_root/usr/share/doc/nvim-gpui/README.md"
install -m 644 "$repo_root/CHANGELOG.md" "$package_root/usr/share/doc/nvim-gpui/CHANGELOG.md"

desktop-file-validate "$package_root/usr/share/applications/nvim-gpui.desktop"

# librime is loaded with libloading, so dpkg-shlibdeps cannot discover it from
# the ELF files. Keep it, its data, and the ordinary Rime schema explicit.
mkdir -p "$temporary_dir/debian"
cat > "$temporary_dir/debian/control" <<EOF
Source: nvim-gpui
Section: editors
Priority: optional
Maintainer: nvim-gpui contributors

Package: nvim-gpui
Architecture: any
Description: GPUI-based graphical frontend for Neovim
 nvim-gpui is a native graphical frontend for Neovim.
EOF
shlib_depends="$(
  cd "$temporary_dir"
  dpkg-shlibdeps -O --ignore-missing-info \
    -e "$package_root/usr/bin/nvim-gpui" \
    -e "$package_root/usr/bin/gpvim" \
    | sed -n 's/^shlibs:Depends=//p'
)"
[[ -n "$shlib_depends" ]] || fail 'dpkg-shlibdeps did not produce runtime dependencies'

cat > "$package_root/DEBIAN/control" <<EOF
Package: nvim-gpui
Version: $version
Section: editors
Priority: optional
Architecture: $deb_arch
Maintainer: nvim-gpui contributors
Depends: $shlib_depends, librime1t64 | librime1, librime-data, rime-data-luna-pinyin
Suggests: neovim (>= 0.10.0), ibus, ibus-libpinyin
Description: GPUI-based graphical frontend for Neovim
 nvim-gpui is a native graphical frontend for Neovim.
 It supports local embedded sessions and connections to remote Neovim
 instances through the Neovim msgpack-RPC UI protocol.
EOF

package_file="$output_dir/nvim-gpui_${version}_${deb_arch}.deb"
mkdir -p "$output_dir"
rm -f "$package_file"
dpkg-deb --build --root-owner-group "$package_root" "$package_file" >/dev/null

# Check the package and start both command-line entry points without opening a
# display. This catches an accidental Nix-linked binary before the .deb leaves
# the container.
dpkg-deb --info "$package_file" >/dev/null
dpkg-deb --contents "$package_file" >/dev/null
"$package_root/usr/bin/nvim-gpui" --version >/dev/null
"$package_root/usr/bin/gpvim" --version >/dev/null

if [[ "${NVIM_GPUI_OUTPUT_UID:-}" =~ ^[0-9]+$ ]] \
  && [[ "${NVIM_GPUI_OUTPUT_GID:-}" =~ ^[0-9]+$ ]]; then
  chown "$NVIM_GPUI_OUTPUT_UID:$NVIM_GPUI_OUTPUT_GID" "$package_file"
fi

printf 'created Ubuntu %s package: %s\n' "$deb_arch" "$package_file"
