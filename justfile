set shell := ["bash", "-euo", "pipefail", "-c"]

default: check

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

# Launch the GPUI application.
run *args:
    cargo run --bin nvim-gpui -- {{args}}

# Launch the gpvim helper.
gpvim *args:
    cargo run --bin gpvim -- {{args}}

# Build Linux release binaries locally through Docker.
docker-build architecture:
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{architecture}}" in
      x86_64) docker_platform="linux/amd64" ;;
      aarch64) docker_platform="linux/arm64" ;;
      *) echo "unsupported Linux architecture: {{architecture}}" >&2; exit 2 ;;
    esac
    artifact_dir="$PWD/.cache/artifacts/linux-{{architecture}}"
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
      --mount "type=bind,src=$artifact_dir,dst=/workspace/.cache/cargo-target" \
      --mount "type=volume,src=$nix_volume,dst=/nix" \
      --mount "type=volume,src=$cargo_volume,dst=/workspace/.cache/cargo-home" \
      --workdir /workspace \
      --env "NIX_CONFIG=$nix_config" \
      "$nix_image" \
      nix \
        --option sandbox false \
        --option filter-syscalls false \
        develop --accept-flake-config -c just build-release

# Build Linux aarch64 locally through Docker.
docker-build-linux-aarch64:
    just docker-build aarch64

# Copy and validate a platform-specific Rime runtime.
rime-runtime source output=".cache/rime-runtime":
    python3 scripts/rime_runtime.py stage --source "{{source}}" --output "{{output}}"

# Validate an already staged Rime runtime without changing it.
rime-runtime-check root=".cache/rime-runtime":
    python3 scripts/rime_runtime.py check --root "{{root}}" --require-data

# Build and validate the pinned macOS librime runtime.
rime-runtime-macos:
    bash packaging/rime/build-macos.sh

# Build and validate the pinned Windows librime runtime.
rime-runtime-windows:
    powershell -NoProfile -ExecutionPolicy Bypass -File packaging/rime/build-windows.ps1

# Build a macOS AppBundle at .cache/macos/nvim-gpui.app.
bundle: build-release
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "$(uname -s)" != "Darwin" ]; then echo "bundle is only supported on macOS" >&2; exit 1; fi
    runtime="$PWD/.cache/rime-runtime"
    if [ ! -d "$runtime" ]; then echo "missing $runtime; run NVIM_GPUI_RIME_STARTER_DATA=/path/to/curated-data just rime-runtime-macos first" >&2; exit 1; fi
    python3 scripts/rime_runtime.py check --root "$runtime" --platform macos --require-data
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

# Smoke-test both executables inside the macOS AppBundle.
macos-smoke: bundle
    #!/usr/bin/env bash
    set -euo pipefail
    app="$PWD/.cache/macos/nvim-gpui.app"
    "$app/Contents/MacOS/nvim-gpui" --version
    "$app/Contents/Resources/gpvim" --version

# Build a compressed macOS installer disk image.
dmg: macos-smoke
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "$(uname -s)" != "Darwin" ]; then echo "dmg is only supported on macOS" >&2; exit 1; fi
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
package-macos: ci rime-runtime-macos dmg

# Build a Windows directory bundle containing nvim-gpui, gpvim, and Rime.
bundle-windows:
    powershell -NoProfile -ExecutionPolicy Bypass -File packaging/windows/bundle.ps1

# Run checks and build the complete Windows package.
package-windows: ci rime-runtime-windows bundle-windows

# Synchronize Cargo, AppBundle, and Homebrew release versions.
release-prepare version:
    python3 scripts/release.py prepare {{version}}

# Validate release metadata and the matching changelog section.
release-check tag="":
    python3 scripts/release.py check {{tag}}

# Print the changelog section used as GitHub Release notes.
release-notes tag:
    python3 scripts/release.py notes {{tag}}

# Legacy alias for build-release; use build-release in new scripts.
release: build-release
