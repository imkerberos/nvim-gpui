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

# Generate the multi-resolution macOS application icon from the checked-in
# 1024px source image.
icon:
    if [ "$(uname -s)" != "Darwin" ]; then echo "icon is only supported on macOS" >&2; exit 1; fi; \
    iconset="$PWD/.cache/macos/nvim-gpui.iconset"; \
    rm -rf "$iconset"; \
    mkdir -p "$iconset"; \
    for size in 16 32 128 256 512; do \
        sips -s format png -z "$size" "$size" assets/neovim-gpui-app-icon.png --out "$iconset/icon_${size}x${size}.png" >/dev/null; \
        double=$((size * 2)); \
        sips -s format png -z "$double" "$double" assets/neovim-gpui-app-icon.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null; \
    done; \
    iconutil --convert icns "$iconset" --output assets/neovim-gpui.icns; \
    echo "created assets/neovim-gpui.icns"

# Build a macOS AppBundle at .cache/macos/nvim-gpui.app.
bundle: icon
    if [ "$(uname -s)" != "Darwin" ]; then echo "bundle is only supported on macOS" >&2; exit 1; fi
    cargo build --release --bins
    rm -rf "$PWD/.cache/macos/nvim-gpui.app"
    mkdir -p "$PWD/.cache/macos/nvim-gpui.app/Contents/MacOS" "$PWD/.cache/macos/nvim-gpui.app/Contents/Resources"
    install -m 755 "${CARGO_TARGET_DIR:-target}/release/nvim-gpui" "$PWD/.cache/macos/nvim-gpui.app/Contents/MacOS/nvim-gpui"
    install -m 755 "${CARGO_TARGET_DIR:-target}/release/gpvim" "$PWD/.cache/macos/nvim-gpui.app/Contents/Resources/gpvim"
    install -m 644 packaging/macos/Info.plist "$PWD/.cache/macos/nvim-gpui.app/Contents/Info.plist"
    install -m 644 assets/nvim-gpui.svg "$PWD/.cache/macos/nvim-gpui.app/Contents/Resources/nvim-gpui.svg"
    install -m 644 assets/neovim-gpui.icns "$PWD/.cache/macos/nvim-gpui.app/Contents/Resources/neovim-gpui.icns"
    echo "created $PWD/.cache/macos/nvim-gpui.app"

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
