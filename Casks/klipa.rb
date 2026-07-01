# Homebrew Cask for klipa.
#
# This repo IS the tap, so users install with:
#   brew tap peterdsp/klipa https://github.com/peterdsp/klipa
#   brew install --cask klipa
#
# `version` and `sha256` are kept current automatically by the release
# workflow (see scripts/update-package-managers.sh). Do not hand-edit.
cask "klipa" do
  version "0.4.0"
  sha256 "2c73146ea97cf28cf1bb5b2c467d1bd06ed13efc8ac7bc4e6cbedd40b21631b6"

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
