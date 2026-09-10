#!/usr/bin/env bash
set -euo pipefail

# Build the Fedora RPM inside the Fedora container started by
# `scripts/build-fedora-rpm.sh`.

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
fedora_arch="${NVIM_GPUI_FEDORA_ARCH:-x86_64}"
output_dir="${NVIM_GPUI_FEDORA_OUTPUT:-$repo_root/dist/fedora-$fedora_arch}"
output_uid="${NVIM_GPUI_OUTPUT_UID:-}"
output_gid="${NVIM_GPUI_OUTPUT_GID:-}"

fail() {
  printf 'Fedora package error: %s\n' "$1" >&2
  exit 1
}

((EUID == 0)) || fail 'the Docker packaging script must run as root'
case "$fedora_arch" in
  x86_64|aarch64) ;;
  *) fail "unsupported Fedora architecture: $fedora_arch" ;;
esac
[[ "$(uname -m)" == "$fedora_arch" ]] \
  || fail "the container architecture is not $fedora_arch: $(uname -m)"
[[ -r /etc/os-release ]] || fail '/etc/os-release is missing'
# shellcheck disable=SC1091
. /etc/os-release
[[ "${ID:-}" == 'fedora' ]] || fail "this package task requires Fedora (detected: ${ID:-unknown})"

[[ -f "$repo_root/Cargo.toml" ]] || fail "Cargo.toml not found below $repo_root"
[[ -f "$repo_root/packaging/fedora/nvim-gpui.spec" ]] \
  || fail 'Fedora RPM spec is missing'
[[ -f "$repo_root/packaging/debian/nvim-gpui.desktop" ]] \
  || fail 'Linux desktop entry is missing'
icon_source_dir="$repo_root/packaging/debian/icons/hicolor"
[[ -d "$icon_source_dir" ]] || fail 'Linux application icon set is missing'

dnf -y install \
  ca-certificates \
  cargo \
  cmake \
  bsdtar \
  desktop-file-utils \
  fontconfig-devel \
  freetype-devel \
  git \
  libX11-devel \
  libxcb-devel \
  libXcursor-devel \
  libXi-devel \
  libXrandr-devel \
  libxkbcommon-devel \
  libxkbcommon-x11-devel \
  mesa-libEGL-devel \
  mesa-libGL-devel \
  neovim \
  openssl-devel \
  pkgconf-pkg-config \
  rpm-build \
  tar \
  wayland-devel

version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo_root/Cargo.toml" | head -n 1)"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] \
  || fail "could not parse a Fedora-compatible version from Cargo.toml: $version"
spec_version="$(sed -n 's/^Version:[[:space:]]*//p' \
  "$repo_root/packaging/fedora/nvim-gpui.spec" | head -n 1)"
[[ "$version" == "$spec_version" ]] \
  || fail "Cargo.toml version ($version) and RPM spec version ($spec_version) differ"

temporary_dir="$(mktemp -d)"
trap 'rm -rf "$temporary_dir"' EXIT
source_stage_parent="$temporary_dir/source"
source_stage="$source_stage_parent/nvim-gpui-$version"
rpm_topdir="$temporary_dir/rpmbuild"
mkdir -p "$source_stage_parent" \
  "$source_stage" \
  "$rpm_topdir/BUILD" \
  "$rpm_topdir/BUILDROOT" \
  "$rpm_topdir/RPMS" \
  "$rpm_topdir/SOURCES" \
  "$rpm_topdir/SPECS" \
  "$rpm_topdir/SRPMS"

# Stage the current worktree, including uncommitted documentation or license
# changes, but not generated build/output directories. Copying each top-level
# entry avoids macOS bind-mount stat/xattr limitations seen with GNU tar.
shopt -s dotglob nullglob
for entry in "$repo_root"/*; do
  entry_name="${entry##*/}"
  case "$entry_name" in
    .git|.direnv|.cache|target|dist|tmp) continue ;;
  esac
  cp -a --no-preserve=xattr "$entry" "$source_stage/"
done
shopt -u dotglob nullglob

mkdir -p "$source_stage/.cargo"
printf 'Vendoring Rust dependencies for the RPM source archive\n'
(
  cd "$source_stage"
  cargo vendor vendor > .cargo/config.toml
)
tar --no-xattrs -C "$source_stage_parent" -czf \
  "$rpm_topdir/SOURCES/nvim-gpui-$version.tar.gz" \
  "nvim-gpui-$version"
cp "$repo_root/packaging/fedora/nvim-gpui.spec" "$rpm_topdir/SPECS/nvim-gpui.spec"

printf 'Building nvim-gpui %s as a Fedora %s RPM\n' "$version" "$fedora_arch"
rpmbuild -ba \
  --define "_topdir $rpm_topdir" \
  "$rpm_topdir/SPECS/nvim-gpui.spec"

mapfile -t rpm_files < <(
  find "$rpm_topdir/RPMS/$fedora_arch" -maxdepth 1 -type f \
    -name "nvim-gpui-${version}-*.$fedora_arch.rpm" \
    ! -name '*-debuginfo-*' -print | sort
)
[[ "${#rpm_files[@]}" -eq 1 ]] \
  || fail "expected one main $fedora_arch RPM, found ${#rpm_files[@]}"

package_file="$output_dir/$(basename "${rpm_files[0]}")"
mkdir -p "$output_dir"
rm -f "$output_dir"/nvim-gpui-"$version"-*.$fedora_arch.rpm
cp "${rpm_files[0]}" "$package_file"
rpm -qip "$package_file" >/dev/null
rpm -qpl "$package_file" >/dev/null

if [[ "$output_uid" =~ ^[0-9]+$ ]] && [[ "$output_gid" =~ ^[0-9]+$ ]]; then
  chown "$output_uid:$output_gid" "$package_file"
fi

printf 'created Fedora %s package: %s\n' "$fedora_arch" "$package_file"
