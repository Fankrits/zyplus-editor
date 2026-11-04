cask "zyplus" do
  version "0.4.0"
  sha256 "2f1b3f33a09716d5f601bd1c6162fab42c99ad47a1cc201bafb455f60089352e"

  url "https://github.com/Fankrits/zyplus-editor/releases/download/v#{version}/Zyplus_#{version}_universal.dmg"
  name "Zyplus"
  desc "Markdown editor"
  homepage "https://github.com/Fankrits/zyplus-editor"

  auto_updates true
  depends_on macos: :big_sur

  app "Zyplus.app"
  binary "#{appdir}/Zyplus.app/Contents/MacOS/zyplus-editor", target: "zyplus"

  # Application Support is left alone: it holds the user's notes (Zyplus/).
  zap trash: [
    "~/Library/Caches/com.fankrits.zyplus-editor",
    "~/Library/Preferences/com.fankrits.zyplus-editor.plist",
    "~/Library/Saved Application State/com.fankrits.zyplus-editor.savedState",
    "~/Library/WebKit/com.fankrits.zyplus-editor",
  ]
end
