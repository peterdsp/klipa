# Homebrew Cask for klipa.
#
# This repo IS the tap, so users install with:
#   brew tap peterdsp/klipa https://github.com/peterdsp/klipa
#   brew install --cask klipa
#
# `version` and `sha256` are kept current automatically by the release
# workflow (see scripts/update-package-managers.sh). Do not hand-edit.
cask "klipa" do
  version "0.2.0"
  sha256 "7f34309b73af70d4c1d1868746f800c7012cfa9bf06120c7428843fc4db27d66"

  url "https://github.com/peterdsp/klipa/releases/download/v#{version}/klipa-#{version}-macos.pkg"
  name "klipa"
  desc "Small, fast, menubar clipboard manager with keep-awake"
  homepage "https://klipa.peterdsp.dev"

  depends_on macos: ">= :big_sur"

  pkg "klipa-#{version}-macos.pkg"

  uninstall quit:    "dev.peterdsp.klipa",
            pkgutil: "dev.peterdsp.klipa"

  zap trash: [
    "~/Library/Application Support/dev.peterdsp.klipa",
  ]
end
