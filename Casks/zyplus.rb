cask "zyplus" do
  version "0.2.3"
  sha256 "0eeeb1fe957c122506d54f80975c30594dcd67e2728ff32e59a938f395da1890"

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
