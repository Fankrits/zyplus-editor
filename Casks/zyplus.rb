cask "zyplus" do
  version "0.1.8"
  sha256 "80f6966ead9c27bd9d223210b53a1176a70c166fd616e607e54a2f2c6f85c559"

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
