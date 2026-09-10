#!/usr/bin/env bash
set -euo pipefail

# Build an Arch Linux x86_64 package from the current worktree through Docker.
# On an arm64 host, Docker uses QEMU when --platform is linux/amd64.

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
output_dir="${NVIM_GPUI_ARCH_OUTPUT:-$repo_root/dist/arch-x86_64}"
docker_image="${NVIM_GPUI_ARCH_IMAGE:-archlinux:base-devel}"
docker_platform="${NVIM_GPUI_ARCH_DOCKER_PLATFORM:-linux/amd64}"
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo_root/Cargo.toml" | head -n 1)"
source_dir="$repo_root/.cache/arch"
source_archive="$source_dir/nvim-gpui-$version.tar.gz"

[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] \
  || { echo "could not parse a package version from Cargo.toml: $version" >&2; exit 1; }
command -v docker >/dev/null 2>&1 \
  || { echo 'docker is required; start Docker Desktop first' >&2; exit 1; }
docker info >/dev/null \
  || { echo 'Docker daemon is unavailable; start Docker Desktop first' >&2; exit 1; }

mkdir -p "$source_dir" "$output_dir"
trap 'rm -f "$source_archive"' EXIT

# Keep the archive's files at its root; PKGBUILD handles both this layout and
# the tagged GitHub archive layout (which has a nvim-gpui-$pkgver directory).
tar \
  --exclude='.git' \
  --exclude='.direnv' \
  --exclude='.cache' \
  --exclude='target' \
  --exclude='dist' \
  --exclude='tmp' \
  -C "$repo_root" -czf "$source_archive" .

docker volume create nvim-gpui-arch-cargo >/dev/null
docker volume create nvim-gpui-arch-pacman >/dev/null
docker volume create nvim-gpui-arch-target >/dev/null
docker run --rm -i --pull=missing \
  --platform "$docker_platform" \
  --mount "type=bind,src=$repo_root,dst=/workspace" \
  --mount 'type=volume,src=nvim-gpui-arch-cargo,dst=/root/.cargo' \
  --mount 'type=volume,src=nvim-gpui-arch-pacman,dst=/var/cache/pacman/pkg' \
  --mount 'type=volume,src=nvim-gpui-arch-target,dst=/workspace/.cache/arch-target' \
  --workdir /workspace \
  --env NVIM_GPUI_ARCH_SOURCE_TARBALL=/workspace/.cache/arch/nvim-gpui-${version}.tar.gz \
  --env NVIM_GPUI_ARCH_OUTPUT=/workspace/dist/arch-x86_64 \
  --env CARGO_TARGET_DIR=/workspace/.cache/arch-target \
  --env "NVIM_GPUI_OUTPUT_UID=$(id -u)" \
  --env "NVIM_GPUI_OUTPUT_GID=$(id -g)" \
  "$docker_image" \
  bash /workspace/packaging/arch/build-package-in-docker.sh
