// Prints the window number of the largest on-screen window owned by a process.
// Build: swiftc -O tools/winid.swift -o winid     Use: ZYPLUS_TEST_WINID=/path/to/winid
import CoreGraphics
import Foundation

guard CommandLine.arguments.count > 1, let pid = Int(CommandLine.arguments[1]) else { exit(64) }
let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
guard let list = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] else { exit(1) }
var best: (id: Int, area: Double)? = nil
for window in list {
    guard let owner = window[kCGWindowOwnerPID as String] as? Int, owner == pid,
          let id = window[kCGWindowNumber as String] as? Int,
          let bounds = window[kCGWindowBounds as String] as? [String: Any] else { continue }
    let width = (bounds["Width"] as? NSNumber)?.doubleValue ?? 0
    let height = (bounds["Height"] as? NSNumber)?.doubleValue ?? 0
    if best == nil || width * height > best!.area { best = (id, width * height) }
}
guard let found = best else { exit(2) }
print(found.id)
