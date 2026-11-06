# Homebrew cask draft for Castor. There is no tap yet: create the public repository
# github.com/basedpolymer/homebrew-tap, put this file at Casks/castor.rb, then users run
#   brew install --cask basedpolymer/tap/castor
# On each release: set `version` and both `sha256` values (shasum -a 256 of the two DMGs).
# The app updates itself (Velopack), hence `auto_updates true`.
cask "castor" do
  arch arm: "arm64", intel: "x64"

  version "0.1.0"
  sha256 arm:   "REPLACE_WITH_SHA256_OF_Castor-#{version}-macos-arm64.dmg",
         intel: "REPLACE_WITH_SHA256_OF_Castor-#{version}-macos-x64.dmg"

  url "https://github.com/basedpolymer/castor/releases/download/v#{version}/Castor-#{version}-macos-#{arch}.dmg"
  name "Castor"
  desc "Offline toolbox of 110 file tools (PDF, image, video, audio, create, files)"
  homepage "https://basedpolymer.eu/castor"

  livecheck do
    url :url
    strategy :github_latest
  end

  auto_updates true
  depends_on macos: ">= :big_sur"

  app "Castor.app"
  # `castor-cli` is a small launcher that runs the app binary as `castor cli …`. MCP clients
  # call the app binary itself: /Applications/Castor.app/Contents/MacOS/castor mcp
  # `binary` installs a symlink: the launcher must resolve it to find `castor` (checked by
  # packaging/macos/build.sh; not the case in 0.1.0 builds of 2026-11-06).
  binary "#{appdir}/Castor.app/Contents/MacOS/castor-cli"

  zap trash: [
    "~/Library/Application Support/eu.basedpolymer.Castor",
    "~/Library/Caches/eu.basedpolymer.Castor",
    "~/Library/Preferences/eu.basedpolymer.castor.plist",
  ]
end
