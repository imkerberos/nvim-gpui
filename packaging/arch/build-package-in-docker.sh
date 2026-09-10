#!/usr/bin/env bash
set -euo pipefail

# Build the Arch Linux package inside the Arch container started by
# `scripts/build-arch-package.sh`.

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
output_dir="${NVIM_GPUI_ARCH_OUTPUT:-$repo_root/dist/arch-x86_64}"
source_archive="${NVIM_GPUI_ARCH_SOURCE_TARBALL:-}"
output_uid="${NVIM_GPUI_OUTPUT_UID:-}"
output_gid="${NVIM_GPUI_OUTPUT_GID:-}"

fail() {
  printf 'Arch package error: %s\n' "$1" >&2
  exit 1
}

((EUID == 0)) || fail 'the Docker packaging script must run as root'
[[ "$(uname -m)" == 'x86_64' ]] \
  || fail "the container architecture is not x86_64: $(uname -m)"
[[ -f "$repo_root/packaging/arch/PKGBUILD" ]] || fail 'Arch PKGBUILD is missing'
[[ -n "$source_archive" && -f "$source_archive" ]] \
  || fail 'the local source archive is missing'

# Docker Desktop's amd64 emulation can reject pacman's downloader seccomp
# sandbox. The container is already isolated by Docker, so disable only this
# nested sandbox for portability across Docker runtimes.
pacman -Syu --disable-sandbox --disable-download-timeout --noconfirm --needed \
  base-devel \
  cmake \
  desktop-file-utils \
  fontconfig \
  freetype2 \
  git \
  libglvnd \
  libx11 \
  libxcb \
  libxcursor \
  libxi \
  libxkbcommon \
  libxkbcommon-x11 \
  libxrandr \
  librime \
  librime-data \
  neovim \
  pkgconf \
  rust \
  wayland

useradd --create-home --shell /bin/bash builder
if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
  mkdir -p "$CARGO_TARGET_DIR"
  chown -R builder:builder "$CARGO_TARGET_DIR"
fi
build_root="$(mktemp -d /tmp/nvim-gpui-arch.XXXXXX)"
trap 'rm -rf "$build_root"' EXIT
package_output="$build_root/pkgdest"
mkdir -p "$package_output"
cp "$repo_root/packaging/arch/PKGBUILD" "$build_root/PKGBUILD"
cp "$source_archive" "$build_root/nvim-gpui-$(sed -n 's/^pkgver=//p' \
  "$repo_root/packaging/arch/PKGBUILD" | head -n 1).tar.gz"
chown -R builder:builder "$build_root"

printf 'Building nvim-gpui as an Arch Linux x86_64 package\n'
runuser -u builder -- env \
  HOME=/home/builder \
  PKGDEST="$package_output" \
  NVIM_GPUI_ARCH_SOURCE_TARBALL=local \
  makepkg --dir "$build_root" --clean --cleanbuild --noconfirm

mapfile -t package_files < <(
  find "$package_output" -maxdepth 1 -type f \
    -name 'nvim-gpui-*.pkg.tar.*' -print | sort
)
[[ "${#package_files[@]}" -eq 1 ]] \
  || fail "expected one Arch package, found ${#package_files[@]}"

package_file="$output_dir/$(basename "${package_files[0]}")"
mkdir -p "$output_dir"
rm -f "$output_dir"/nvim-gpui-*.pkg.tar.*
cp "${package_files[0]}" "$package_file"
pacman -Qip "$package_file" >/dev/null
tar -tf "$package_file" >/dev/null

if [[ "$output_uid" =~ ^[0-9]+$ ]] && [[ "$output_gid" =~ ^[0-9]+$ ]]; then
  chown "$output_uid:$output_gid" "$package_file"
fi

printf 'created Arch Linux x86_64 package: %s\n' "$package_file"
