#!/usr/bin/env bash
set -euo pipefail

# Build the application-private macOS librime runtime.
#
# The resulting runtime is deliberately assembled through the repository's
# staging/validation script. Build dependencies may come from source, but the
# staged artifact must not retain Nix or Homebrew runtime paths.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
manifest="$repo_root/packaging/rime/runtime.toml"

usage() {
  cat <<'EOF'
usage: build-macos.sh [options]

Build the pinned librime source revision and stage an application-private
runtime. Run this command inside the repository's Nix development shell.

options:
  --data-source DIR  starter rime-data directory to embed in runtime/data
  --output DIR       staged runtime output (default: .cache/rime-runtime)
  --work-dir DIR     source and build cache (default: .cache/rime-build/macos)
  --help             show this help

environment:
  NVIM_GPUI_RIME_STARTER_DATA  default value for --data-source
  NVIM_GPUI_RIME_RUNTIME_OUTPUT default value for --output
  NVIM_GPUI_RIME_BUILD_DIR      default value for --work-dir
  NVIM_GPUI_RIME_BUILD_UNIVERSAL=0 to build only the host architecture
  MACOSX_DEPLOYMENT_TARGET      default: 12.0
EOF
}

fail() {
  printf 'rime macOS build error: %s\n' "$1" >&2
  exit 1
}

resolve_repo_path() {
  local value="$1"
  if [[ "$value" = /* ]]; then
    printf '%s\n' "$value"
  else
    printf '%s/%s\n' "$repo_root" "$value"
  fi
}

data_source="${NVIM_GPUI_RIME_STARTER_DATA:-}"
output="${NVIM_GPUI_RIME_RUNTIME_OUTPUT:-.cache/rime-runtime}"
work_dir="${NVIM_GPUI_RIME_BUILD_DIR:-.cache/rime-build/macos}"

while (($# > 0)); do
  case "$1" in
    --data-source)
      (($# >= 2)) || fail "--data-source requires a directory"
      data_source="$2"
      shift 2
      ;;
    --output)
      (($# >= 2)) || fail "--output requires a directory"
      output="$2"
      shift 2
      ;;
    --work-dir)
      (($# >= 2)) || fail "--work-dir requires a directory"
      work_dir="$2"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
done

[[ "$(uname -s)" == "Darwin" ]] || fail "this builder only runs on macOS"

for command in cmake curl git make otool python3 shasum sysctl xcrun; do
  command -v "$command" >/dev/null 2>&1 || fail "required command not found: $command"
done

source_repository="$(python3 - "$manifest" <<'PY'
import sys
import tomllib

with open(sys.argv[1], "rb") as stream:
    print(tomllib.load(stream)["source"]["repository"])
PY
)"
source_revision="$(python3 - "$manifest" <<'PY'
import sys
import tomllib

with open(sys.argv[1], "rb") as stream:
    print(tomllib.load(stream)["source"]["revision"])
PY
)"

[[ -n "$data_source" ]] || fail "starter data is required; pass --data-source DIR"
data_source="$(resolve_repo_path "$data_source")"
[[ -d "$data_source" ]] || fail "starter data directory does not exist: $data_source"
data_source="$(cd "$data_source" && pwd -P)"

output="$(resolve_repo_path "$output")"
work_dir="$(resolve_repo_path "$work_dir")"
source_dir="$work_dir/librime"

build_universal="${NVIM_GPUI_RIME_BUILD_UNIVERSAL:-1}"
if [[ "$build_universal" != "0" ]]; then
  build_mode=universal
else
  build_mode=host
fi
build_dir="$work_dir/cmake-build-$build_mode"
dist_dir="$work_dir/dist-$build_mode"
artifact_dir="$work_dir/artifact-$build_mode"

[[ "$output" != "$source_dir" && "$output" != "$source_dir"/* ]] || \
  fail "output must not be inside the librime source checkout"
[[ "$output" != "$work_dir" && "$output" != "$work_dir"/* ]] || \
  fail "output must not be inside the build cache"

mkdir -p "$work_dir"

if [[ ! -e "$source_dir" ]]; then
  git clone --recursive "$source_repository" "$source_dir"
elif [[ ! -d "$source_dir/.git" ]]; then
  fail "source path exists but is not a git checkout: $source_dir"
fi

git -C "$source_dir" fetch --tags origin
git -C "$source_dir" checkout --detach "$source_revision"
git -C "$source_dir" submodule sync --recursive
git -C "$source_dir" submodule update --init --recursive

boost_version=1.89.0
boost_root="$source_dir/deps/boost-$boost_version"
(
  export BOOST_ROOT="$boost_root"
  export boost_version
  bash "$source_dir/install-boost.sh" --download
)

sdk_root="${SDKROOT:-$(xcrun --sdk macosx --show-sdk-path)}"
deployment_target="${MACOSX_DEPLOYMENT_TARGET:-12.0}"
cmake_architectures=()
if [[ "$build_universal" != "0" ]]; then
  cmake_architectures=(-DCMAKE_OSX_ARCHITECTURES=arm64\;x86_64)
  export CMAKE_OSX_ARCHITECTURES='arm64;x86_64'
  dependency_build_dir=build-universal
else
  unset CMAKE_OSX_ARCHITECTURES
  dependency_build_dir=build-host
fi

export SDKROOT="$sdk_root"
export MACOSX_DEPLOYMENT_TARGET="$deployment_target"

# librime's own deps.mk builds the libraries that BUILD_STATIC consumes. Use a
# separate dependency build cache for each architecture mode so an old host
# build can never be reused while producing a universal runtime. The upstream
# Makefile's Darwin CPU-count probe can emit an empty -j in some shells, so set
# a validated value explicitly.
jobs="$(sysctl -n hw.ncpu 2>/dev/null || printf '1')"
[[ "$jobs" =~ ^[1-9][0-9]*$ ]] || jobs=1
MAKEFLAGS="-j$jobs" NOPARALLEL=1 make -C "$source_dir" deps build="$dependency_build_dir"

make_tree_writable() {
  local root="$1"
  [[ -e "$root" ]] || return 0
  while IFS= read -r -d '' path; do
    [[ -L "$path" ]] || chmod u+w "$path"
  done < <(find "$root" -depth -print0)
}

make_tree_writable "$dist_dir"
make_tree_writable "$artifact_dir"
rm -rf "$dist_dir" "$artifact_dir"
mkdir -p "$dist_dir" "$artifact_dir/lib" "$artifact_dir/data" "$artifact_dir/modules"

cmake -S "$source_dir" -B "$build_dir" \
  -DCMAKE_INSTALL_PREFIX="$dist_dir" \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=ON \
  -DBUILD_STATIC=ON \
  -DBUILD_MERGED_PLUGINS=ON \
  -DENABLE_EXTERNAL_PLUGINS=OFF \
  -DBUILD_DATA=OFF \
  -DBUILD_TEST=OFF \
  -DBoost_NO_BOOST_CMAKE=TRUE \
  -DBOOST_ROOT="$boost_root" \
  -DCMAKE_INSTALL_NAME_DIR=@rpath \
  "${cmake_architectures[@]}"
cmake --build "$build_dir" --config Release
cmake --install "$build_dir" --config Release

dist_lib="$dist_dir/lib"
[[ -d "$dist_lib" ]] || fail "librime did not install a lib directory: $dist_lib"

while IFS= read -r -d '' library; do
  relative_path="${library#"$dist_lib/"}"
  mkdir -p "$artifact_dir/lib/$(dirname "$relative_path")"
  cp -P "$library" "$artifact_dir/lib/$relative_path"
done < <(find "$dist_lib" -maxdepth 1 \( -type f -o -type l \) -name '*.dylib' -print0)

main_library="$(find "$artifact_dir/lib" -maxdepth 1 \( -type f -o -type l \) -name 'librime*.dylib' -print | sort | head -n 1)"
[[ -n "$main_library" ]] || fail "librime dylib was not installed in: $dist_lib"
if [[ ! -e "$artifact_dir/lib/librime.dylib" ]]; then
  ln -s "$(basename "$main_library")" "$artifact_dir/lib/librime.dylib"
fi

if [[ -d "$dist_lib/rime-plugins" ]]; then
  cp -R "$dist_lib/rime-plugins/." "$artifact_dir/modules/"
fi

# Keep starter data independent from the librime source tree. Accept both an
# actual data directory and a package root such as Nix's share/rime-data
# layout, but always flatten the latter into the runtime contract. User
# dictionaries never belong in this artifact.
starter_data="$data_source"
if [[ -d "$data_source/share/rime-data" ]]; then
  starter_data="$data_source/share/rime-data"
elif [[ -d "$data_source/rime-data" ]]; then
  starter_data="$data_source/rime-data"
fi
cp -R "$starter_data/." "$artifact_dir/data/"

while IFS= read -r -d '' binary; do
  dependencies="$(otool -L "$binary")"
  if grep -Eq '/nix/store/|/opt/homebrew/|/usr/local/opt/' <<<"$dependencies"; then
    printf '%s\n' "$dependencies" >&2
    fail "runtime binary has a package-manager dependency: $binary"
  fi
done < <(find "$artifact_dir" -type f -name '*.dylib' -print0)

python3 "$repo_root/scripts/rime_runtime.py" stage \
  --source "$artifact_dir" \
  --output "$output" \
  --platform macos
python3 "$repo_root/scripts/rime_runtime.py" check \
  --root "$output" \
  --platform macos \
  --require-data

printf 'built macOS Rime runtime: %s\n' "$output"
