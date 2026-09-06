set shell := ["bash", "-euo", "pipefail", "-c"]
set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

python_command := if os_family() == "windows" { "python.exe" } else { "python3" }

# Show available tasks when no task is specified.
default:
    just --list

# Format all Rust sources.
fmt:
    cargo fmt --all

# Check formatting without changing files.
fmt-check:
    cargo fmt --all -- --check

# Type-check the workspace.
check: fmt-check
    cargo check --all-targets --locked

# Run Clippy with warnings treated as errors.
clippy:
    cargo clippy --locked --all-targets --all-features -- -D warnings

# Run unit and integration tests.
test:
    cargo test --locked --all-targets

# Run the complete local CI validation suite.
ci: check clippy test

# Build debug binaries.
build:
    cargo build --locked

# Build optimized binaries without packaging them.
build-release:
    cargo build --locked --release --bins

# Compatibility alias for build-release; use build-release in new scripts.
release: build-release

# Launch the GPUI application.
run *args:
    cargo run --bin nvim-gpui -- {{args}}

# Launch the gpvim helper.
gpvim *args:
    cargo run --bin gpvim -- {{args}}

# Enter the macOS Nix development environment.
dev-macos:
    {{if os() == "macos" { "nix develop --accept-flake-config" } else { error("dev-macos is only supported on macOS") }}}

# Enter the Linux Nix development environment.
dev-linux:
    {{if os() == "linux" { "nix develop --accept-flake-config" } else { error("dev-linux is only supported on Linux") }}}

# Install Windows prerequisites; use dev-windows.cmd first when Just is not installed.
dev-windows:
    {{if os() == "windows" { "powershell -NoProfile -ExecutionPolicy Bypass -File scripts/dev-windows.ps1" } else { error("dev-windows is only supported on Windows") }}}

# Select the development environment task for the current operating system.
dev:
    just {{if os() == "macos" { "dev-macos" } else if os() == "linux" { "dev-linux" } else if os() == "windows" { "dev-windows" } else { error("unsupported operating system for dev") }}}

# Prepare an Ubuntu VM for runtime and input-method testing.
[linux]
setup-ubuntu:
    bash scripts/setup-ubuntu.sh

# Build Linux release binaries locally through Docker.
[unix]
docker-build architecture:
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{architecture}}" in
      x86_64) docker_platform="linux/amd64" ;;
      aarch64) docker_platform="linux/arm64" ;;
      *) echo "unsupported Linux architecture: {{architecture}}" >&2; exit 2 ;;
    esac
    artifact_dir="$PWD/.cache/artifacts/linux-{{architecture}}"
    artifact_release_dir="$artifact_dir/release"
    nix_volume="nvim-gpui-nix-linux-{{architecture}}"
    cargo_volume="nvim-gpui-cargo-home"
    nix_image="${NVIM_GPUI_NIX_IMAGE:-nixos/nix:2.32.3}"
    nix_config=$'experimental-features = nix-command flakes\nsandbox = false\nfilter-syscalls = false'
    command -v docker >/dev/null 2>&1 || { echo "docker is required; start Docker Desktop first" >&2; exit 1; }
    docker info >/dev/null || { echo "Docker daemon is unavailable; start Docker Desktop first" >&2; exit 1; }
    mkdir -p "$artifact_dir"
    docker volume create "$nix_volume" >/dev/null
    docker volume create "$cargo_volume" >/dev/null
    docker run --rm -i --pull=missing \
      --platform "$docker_platform" \
      --mount "type=bind,src=$PWD,dst=/workspace" \
      --mount "type=volume,src=$nix_volume,dst=/nix" \
      --mount "type=volume,src=$cargo_volume,dst=/root/.cargo" \
      --workdir /workspace \
      --env "NIX_CONFIG=$nix_config" \
      "$nix_image" \
      nix \
        --option sandbox false \
        --option filter-syscalls false \
        develop --accept-flake-config -c just build-release
    mkdir -p "$artifact_release_dir"
    for binary in nvim-gpui gpvim; do
      source="$PWD/target/release/$binary"
      [ -x "$source" ] || { echo "missing Linux release binary: $source" >&2; exit 1; }
      install -m 755 "$source" "$artifact_release_dir/$binary"
    done
    echo "created Linux artifacts in $artifact_release_dir"

# Build Linux aarch64 locally through Docker.
[unix]
docker-build-linux-aarch64:
    just docker-build aarch64

# Copy and validate a platform-specific Rime runtime.
rime-runtime source output=".cache/rime-runtime":
    {{python_command}} scripts/rime_runtime.py stage --source "{{source}}" --output "{{output}}"

# Validate an already staged Rime runtime without changing it.
rime-runtime-check root=".cache/rime-runtime":
    {{python_command}} scripts/rime_runtime.py check --root "{{root}}" --require-data

# Build and validate the pinned macOS librime runtime.
[macos]
rime-runtime-macos:
    bash packaging/rime/build-macos.sh

# Build and validate the pinned Windows librime runtime.
[windows]
rime-runtime-windows:
    {{if os() == "windows" { "powershell -NoProfile -ExecutionPolicy Bypass -File packaging/rime/build-windows.ps1" } else { error("rime-runtime-windows is only supported on Windows") }}}

# Build a macOS AppBundle at .cache/macos/nvim-gpui.app.
[macos]
bundle-macos: build-release
    #!/usr/bin/env bash
    set -euo pipefail
    runtime="$PWD/.cache/rime-runtime"
    if [ ! -d "$runtime" ]; then echo "missing $runtime; run NVIM_GPUI_RIME_STARTER_DATA=/path/to/curated-data just rime-runtime-macos first" >&2; exit 1; fi
    {{python_command}} scripts/rime_runtime.py check --root "$runtime" --platform macos --require-data
    if [ -e "$PWD/.cache/macos/nvim-gpui.app" ]; then
        while IFS= read -r -d '' path; do
            [ -L "$path" ] || chmod u+w "$path"
        done < <(find "$PWD/.cache/macos/nvim-gpui.app" -depth -print0)
    fi
    rm -rf "$PWD/.cache/macos/nvim-gpui.app"
    mkdir -p "$PWD/.cache/macos/nvim-gpui.app/Contents/MacOS" "$PWD/.cache/macos/nvim-gpui.app/Contents/Resources"
    install -m 755 "${CARGO_TARGET_DIR:-target}/release/nvim-gpui" "$PWD/.cache/macos/nvim-gpui.app/Contents/MacOS/nvim-gpui"
    install -m 755 "${CARGO_TARGET_DIR:-target}/release/gpvim" "$PWD/.cache/macos/nvim-gpui.app/Contents/Resources/gpvim"
    install -m 644 packaging/macos/Info.plist "$PWD/.cache/macos/nvim-gpui.app/Contents/Info.plist"
    install -m 644 assets/icons/neovim-gpui.png "$PWD/.cache/macos/nvim-gpui.app/Contents/Resources/neovim-gpui.png"
    install -m 644 assets/icons/neovim-gpui_1024x1024_1024x1024.icns "$PWD/.cache/macos/nvim-gpui.app/Contents/Resources/neovim-gpui_1024x1024_1024x1024.icns"
    cp -RP "$runtime" "$PWD/.cache/macos/nvim-gpui.app/Contents/Resources/rime"
    bash packaging/macos/verify-no-nix-deps.sh "$PWD/.cache/macos/nvim-gpui.app"
    echo "created $PWD/.cache/macos/nvim-gpui.app"

# Select the platform bundle task.
bundle:
    just {{if os() == "macos" { "bundle-macos" } else if os() == "windows" { "bundle-windows" } else { error("bundle is only supported on macOS and Windows") }}}

# Smoke-test both executables inside the macOS AppBundle.
[macos]
smoke-macos: bundle-macos
    #!/usr/bin/env bash
    set -euo pipefail
    app="$PWD/.cache/macos/nvim-gpui.app"
    "$app/Contents/MacOS/nvim-gpui" --version
    "$app/Contents/Resources/gpvim" --version

# Smoke-test both executables inside the Windows directory bundle.
[windows]
smoke-windows: bundle-windows
    {{if os() == "windows" { "powershell -NoProfile -ExecutionPolicy Bypass -File scripts/smoke-windows.ps1" } else { error("smoke-windows is only supported on Windows") }}}

# Select the platform smoke-test task.
smoke:
    just {{if os() == "macos" { "smoke-macos" } else if os() == "windows" { "smoke-windows" } else { error("smoke is only supported on macOS and Windows") }}}

# Build a compressed macOS installer disk image.
[macos]
dmg: smoke-macos
    #!/usr/bin/env bash
    set -euo pipefail
    case "${NVIM_GPUI_PACKAGE_ARCH:-$(uname -m)}" in
      arm64) arch="aarch64" ;;
      aarch64) arch="aarch64" ;;
      x86_64) arch="x86_64" ;;
      *) echo "unsupported macOS architecture: $(uname -m)" >&2; exit 1 ;;
    esac
    staging="$PWD/.cache/macos/nvim-gpui-dmg-staging"
    output="$PWD/.cache/macos/nvim-gpui-${arch}.dmg"
    rm -rf "$staging" "$output"
    mkdir -p "$staging"
    cp -R "$PWD/.cache/macos/nvim-gpui.app" "$staging/nvim-gpui.app"
    ln -s /Applications "$staging/Applications"
    hdiutil create -volname "nvim-gpui" -srcfolder "$staging" -ov -format UDZO "$output" >/dev/null
    rm -rf "$staging"
    test -s "$output"
    hdiutil imageinfo "$output" >/dev/null
    echo "created $output"

# Run checks and build the complete macOS package.
[macos]
pack-macos: ci rime-runtime-macos dmg

# Build a Windows directory bundle containing nvim-gpui, gpvim, and Rime.
[windows]
bundle-windows:
    powershell -NoProfile -ExecutionPolicy Bypass -File packaging/windows/bundle.ps1

# Run checks, build, and smoke-test the complete Windows package.
[windows]
pack-windows: ci rime-runtime-windows smoke-windows

# Synchronize Cargo, AppBundle, and Homebrew release versions.
release-prepare version:
    {{python_command}} scripts/release.py prepare {{version}}

# Validate release metadata and the matching changelog section.
release-check tag="":
    {{python_command}} scripts/release.py check {{tag}}

# Print the changelog section used as GitHub Release notes.
release-notes tag:
    {{python_command}} scripts/release.py notes {{tag}}
