{
  description = "Development environment for nvim-gpui";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
  };

  outputs = { nixpkgs, ... }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];

      forAllSystems = nixpkgs.lib.genAttrs systems;
    in {
      devShells = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          linuxLibraries = with pkgs; [
            fontconfig
            freetype
            libGL
            libxkbcommon
            wayland
            xorg.libX11
            xorg.libXcursor
            xorg.libXi
            xorg.libXrandr
          ];
          nativeLibraries = with pkgs; [
            openssl
            pkg-config
          ];
          treesitterMarkdown = pkgs.vimPlugins.nvim-treesitter.withPlugins (plugins: [
            plugins.tree-sitter-markdown
          ]);
        in {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              clippy
              gnumake
              just
              neovim
              imagemagick
              python3
              rust-analyzer
              rustc
              rustfmt
            ];

            buildInputs = nativeLibraries
              ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux linuxLibraries;

            RUST_BACKTRACE = "1";
            NVIM_APPNAME = "nvim-gpui";
            # Nix's Darwin linker environment can add libiconv even when the
            # final binary has no iconv symbol references. Remove unused
            # dylib load commands so distributable AppBundles do not depend
            # on the Nix store.
            RUSTFLAGS = pkgs.lib.optionalString
              pkgs.stdenv.hostPlatform.isDarwin
              "-C link-arg=-Wl,-dead_strip_dylibs";

            shellHook = ''
              export NVIM_GPUI_CACHE_DIR="$PWD/.cache"
              export NVIM_GPUI_CONFIG_DIR="$PWD/config"
              export NVIM_GPUI_LAZY="${pkgs.vimPlugins.lazy-nvim}"
              export NVIM_GPUI_SNACKS="${pkgs.vimPlugins.snacks-nvim}"
              export NVIM_GPUI_TREESITTER="${treesitterMarkdown}"
              export NVIM_GPUI_IMAGEMAGICK="${pkgs.imagemagick}"
              export PATH="$PWD/.cache/cargo-target/debug:$PWD/bin:$PATH"
              export NVIM_GPUI_NVIM="''${NVIM_GPUI_NVIM:-$(command -v nvim)}"
              export SNACKS_KITTY="''${SNACKS_KITTY:-1}"
              export CARGO_TARGET_DIR="$NVIM_GPUI_CACHE_DIR/cargo-target"
              export CARGO_HOME="$NVIM_GPUI_CACHE_DIR/cargo-home"
              export TMPDIR="$PWD/tmp"
              mkdir -p "$CARGO_TARGET_DIR" "$CARGO_HOME" "$TMPDIR" \
                "$NVIM_GPUI_CONFIG_DIR/$NVIM_APPNAME" \
                "$NVIM_GPUI_CACHE_DIR/nvim-data" \
                "$NVIM_GPUI_CACHE_DIR/nvim-state" \
                "$NVIM_GPUI_CACHE_DIR/nvim-cache"
              export CARGO_TERM_COLOR=always
              echo "nvim-gpui development shell"
              echo "  cargo target $CARGO_TARGET_DIR"
              echo "  cargo home   $CARGO_HOME"
              echo "  temp         $TMPDIR"
              echo "  nvim config  $NVIM_GPUI_CONFIG_DIR/$NVIM_APPNAME"
              echo "  image tools  $NVIM_GPUI_IMAGEMAGICK"
              echo "  gpvim        $CARGO_TARGET_DIR/debug/gpvim (after cargo build)"
              echo "  just check   type-check and verify formatting"
              echo "  just run     launch the GPUI scaffold"
            '';
          };
        });
    };
}
