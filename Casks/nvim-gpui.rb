cask "nvim-gpui" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.2.0"
  # Replace :no_check with per-architecture SHA-256 values once the release
  # assets have been published and their checksums are recorded.
  sha256 :no_check

  url "https://github.com/imkerberos/nvim-gpui/releases/download/v#{version}/nvim-gpui-#{arch}.dmg"
  name "nvim-gpui"
  desc "GPUI graphical frontend for Neovim"
  homepage "https://github.com/imkerberos/nvim-gpui"

  depends_on macos: :monterey

  app "nvim-gpui.app"
  binary "#{appdir}/nvim-gpui.app/Contents/Resources/gpvim"
  binary "#{appdir}/nvim-gpui.app/Contents/Resources/gpvim", target: "gpvimdiff"
end
