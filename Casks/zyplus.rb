cask "zyplus" do
  version "0.1.7"
  sha256 "c5cfe86021494d2ce0a82e532ecb53d82725f6912027584c0072d188c9defd77"

  url "https://github.com/Fankrits/zyplus-editor/releases/download/v#{version}/Zyplus_#{version}_universal.dmg"
  name "Zyplus"
  desc "Markdown editor"
  homepage "https://github.com/Fankrits/zyplus-editor"

  auto_updates true
  depends_on macos: :big_sur

  app "Zyplus.app"
  binary "#{appdir}/Zyplus.app/Contents/MacOS/zyplus-editor", target: "zyplus"

  zap trash: [
    "~/Library/Application Support/com.fankrits.zyplus-editor",
    "~/Library/Caches/com.fankrits.zyplus-editor",
    "~/Library/Preferences/com.fankrits.zyplus-editor.plist",
    "~/Library/Saved Application State/com.fankrits.zyplus-editor.savedState",
    "~/Library/WebKit/com.fankrits.zyplus-editor",
  ]
end
