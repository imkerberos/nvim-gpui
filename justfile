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

# Build a macOS AppBundle at .cache/macos/nvim-gpui.app.
bundle:
    if [ "$(uname -s)" != "Darwin" ]; then echo "bundle is only supported on macOS" >&2; exit 1; fi
    cargo build --release --bins
    rm -rf "$PWD/.cache/macos/nvim-gpui.app"
    mkdir -p "$PWD/.cache/macos/nvim-gpui.app/Contents/MacOS" "$PWD/.cache/macos/nvim-gpui.app/Contents/Resources"
    install -m 755 "${CARGO_TARGET_DIR:-target}/release/nvim-gpui" "$PWD/.cache/macos/nvim-gpui.app/Contents/MacOS/nvim-gpui"
    install -m 755 "${CARGO_TARGET_DIR:-target}/release/gpvim" "$PWD/.cache/macos/nvim-gpui.app/Contents/Resources/gpvim"
    install -m 644 packaging/macos/Info.plist "$PWD/.cache/macos/nvim-gpui.app/Contents/Info.plist"
    install -m 644 assets/nvim-gpui.svg "$PWD/.cache/macos/nvim-gpui.app/Contents/Resources/nvim-gpui.svg"
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
