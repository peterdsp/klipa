# Homebrew Cask for klipa.
#
# This repo IS the tap, so users install with:
#   brew tap peterdsp/klipa https://github.com/peterdsp/klipa
#   brew install --cask klipa
#
# `version` and `sha256` are kept current automatically by the release
# workflow (see scripts/update-package-managers.sh). Do not hand-edit.
cask "klipa" do
  version "0.4.7"
  sha256 "1223a29d2ef059f7b68c85ca30cbf4a039f330e8ddc0bf499efdb5f47e5413c7"

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
