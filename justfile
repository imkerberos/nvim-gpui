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
    cargo check --all-targets

# Run Clippy with warnings treated as errors.
clippy:
    cargo clippy --all-targets --all-features -- -D warnings

# Build the debug binary.
build:
    cargo build

# Build the optimized binary.
release:
    cargo build --release

# Synchronize Cargo, AppBundle, and Homebrew release versions.
release-prepare version:
    python3 scripts/release.py prepare {{version}}

# Validate release metadata and the matching changelog section.
release-check tag="":
    python3 scripts/release.py check {{tag}}

# Print the changelog section used as GitHub Release notes.
release-notes tag:
    python3 scripts/release.py notes {{tag}}

# Build a macOS AppBundle at .cache/macos/nvim-gpui.app.
bundle:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "$(uname -s)" != "Darwin" ]; then echo "bundle is only supported on macOS" >&2; exit 1; fi
    runtime="$PWD/.cache/rime-runtime"
    if [ ! -d "$runtime" ]; then echo "missing $runtime; run NVIM_GPUI_RIME_STARTER_DATA=/path/to/curated-data just rime-runtime-macos first" >&2; exit 1; fi
    python3 scripts/rime_runtime.py check --root "$runtime" --platform macos --require-data
    cargo build --release --bins
    rm -rf "$PWD/.cache/macos/nvim-gpui.app"
    mkdir -p "$PWD/.cache/macos/nvim-gpui.app/Contents/MacOS" "$PWD/.cache/macos/nvim-gpui.app/Contents/Resources"
    install -m 755 "${CARGO_TARGET_DIR:-target}/release/nvim-gpui" "$PWD/.cache/macos/nvim-gpui.app/Contents/MacOS/nvim-gpui"
    install -m 755 "${CARGO_TARGET_DIR:-target}/release/gpvim" "$PWD/.cache/macos/nvim-gpui.app/Contents/Resources/gpvim"
    install -m 644 packaging/macos/Info.plist "$PWD/.cache/macos/nvim-gpui.app/Contents/Info.plist"
    install -m 644 assets/icons/neovim-gpui.png "$PWD/.cache/macos/nvim-gpui.app/Contents/Resources/neovim-gpui.png"
    install -m 644 assets/icons/neovim-gpui_1024x1024_1024x1024.icns "$PWD/.cache/macos/nvim-gpui.app/Contents/Resources/neovim-gpui_1024x1024_1024x1024.icns"
    cp -R "$runtime" "$PWD/.cache/macos/nvim-gpui.app/Contents/Resources/rime"
    bash packaging/macos/verify-no-nix-deps.sh "$PWD/.cache/macos/nvim-gpui.app"
    echo "created $PWD/.cache/macos/nvim-gpui.app"

# Copy and validate a platform-specific Rime runtime into a staging directory.
# The source must already be a self-contained artifact from the platform
# builder; this task never reads librime from the Nix store implicitly.
rime-runtime source output=".cache/rime-runtime":
    python3 scripts/rime_runtime.py stage --source "{{source}}" --output "{{output}}"

# Build and validate the pinned macOS librime runtime. The starter data must
# be supplied separately with NVIM_GPUI_RIME_STARTER_DATA or --data-source.
rime-runtime-macos:
    bash packaging/rime/build-macos.sh

# Validate an already staged Rime runtime without changing it.
rime-runtime-check root=".cache/rime-runtime":
    python3 scripts/rime_runtime.py check --root "{{root}}" --require-data

# Build a compressed macOS installer disk image containing the AppBundle.
# The output is .cache/macos/nvim-gpui-aarch64.dmg or
# .cache/macos/nvim-gpui-x86_64.dmg, depending on the host architecture.
dmg: bundle
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "$(uname -s)" != "Darwin" ]; then echo "dmg is only supported on macOS" >&2; exit 1; fi
    case "$(uname -m)" in
      arm64) arch="aarch64" ;;
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

# Launch the macOS AppBundle through the gpvim helper.
gpvim *args:
    cargo run --bin gpvim -- {{args}}

# Run unit and integration tests.
test:
    cargo test --all-targets

# Launch the GPUI application.
run *args:
    cargo run --bin nvim-gpui -- {{args}}

# Run the local CI checks.
ci: fmt-check clippy test
