#!/usr/bin/env bash
set -euo pipefail

# Build a Fedora RPM from the current worktree through Docker.

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
fedora_arch="${1:-${NVIM_GPUI_FEDORA_ARCH:-x86_64}}"
docker_image="${NVIM_GPUI_FEDORA_IMAGE:-fedora:44}"

case "$fedora_arch" in
  x86_64)
    default_docker_platform='linux/amd64'
    ;;
  aarch64)
    default_docker_platform='linux/arm64'
    ;;
  *)
    echo "unsupported Fedora architecture: $fedora_arch (expected x86_64 or aarch64)" >&2
    exit 1
    ;;
esac

output_dir="${NVIM_GPUI_FEDORA_OUTPUT:-$repo_root/dist/fedora-$fedora_arch}"
docker_platform="${NVIM_GPUI_FEDORA_DOCKER_PLATFORM:-$default_docker_platform}"

command -v docker >/dev/null 2>&1 \
  || { echo 'docker is required; start Docker Desktop first' >&2; exit 1; }
docker info >/dev/null \
  || { echo 'Docker daemon is unavailable; start Docker Desktop first' >&2; exit 1; }
mkdir -p "$output_dir"
docker volume create "nvim-gpui-fedora-$fedora_arch-cargo" >/dev/null

docker run --rm -i --pull=missing \
  --platform "$docker_platform" \
  --mount "type=bind,src=$repo_root,dst=/workspace" \
  --mount "type=volume,src=nvim-gpui-fedora-$fedora_arch-cargo,dst=/root/.cargo" \
  --workdir /workspace \
  --env "NVIM_GPUI_FEDORA_ARCH=$fedora_arch" \
  --env "NVIM_GPUI_FEDORA_OUTPUT=/workspace/dist/fedora-$fedora_arch" \
  --env "NVIM_GPUI_OUTPUT_UID=$(id -u)" \
  --env "NVIM_GPUI_OUTPUT_GID=$(id -g)" \
  "$docker_image" \
  bash /workspace/packaging/fedora/build-rpm-in-docker.sh
