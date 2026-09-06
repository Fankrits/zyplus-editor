cask "zyplus" do
  version "0.1.4"
  sha256 "0bc653e678448491b4c7c212ec8c796d981b440b01f8a637caab53d5b8ba6eb2"

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
