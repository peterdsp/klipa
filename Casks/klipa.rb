# Homebrew Cask for klipa.
#
# This repo IS the tap, so users install with:
#   brew tap peterdsp/klipa https://github.com/peterdsp/klipa
#   brew install --cask klipa
#
# `version` and `sha256` are kept current automatically by the release
# workflow (see scripts/update-package-managers.sh). Do not hand-edit.
cask "klipa" do
  version "0.4.3"
  sha256 "1f7b961b6280c560d2e8c12e82f6fded8367b76eb48b61b8bb6750821cac6433"

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
