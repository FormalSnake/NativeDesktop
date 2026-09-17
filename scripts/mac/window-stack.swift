// The on-screen window stack at one point, front to back, for the gates that
// drive a REAL window-server click. Such a click goes to whatever window is
// topmost under the cursor, so a leg that never sees its click has to be able
// to say which window took it instead of reporting the feature broken.
//
// `swift scripts/mac/window-stack.swift <globalX> <globalY>`, the point in
// CoreGraphics global coordinates; one JSON object per line, front first.
import CoreGraphics
import Foundation

guard CommandLine.arguments.count >= 3,
      let x = Double(CommandLine.arguments[1]), let y = Double(CommandLine.arguments[2]) else {
    FileHandle.standardError.write("usage: window-stack.swift <x> <y>\n".data(using: .utf8)!)
    exit(2)
}
guard let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] else {
    FileHandle.standardError.write("window stack unavailable\n".data(using: .utf8)!)
    exit(1)
}

for window in windows {
    let bounds = window[kCGWindowBounds as String] as? [String: Any] ?? [:]
    let bx = bounds["X"] as? Double ?? 0
    let by = bounds["Y"] as? Double ?? 0
    let width = bounds["Width"] as? Double ?? 0
    let height = bounds["Height"] as? Double ?? 0
    guard x >= bx, x <= bx + width, y >= by, y <= by + height else { continue }
    let entry: [String: Any] = [
        "owner": window[kCGWindowOwnerName as String] as? String ?? "",
        "pid": window[kCGWindowOwnerPID as String] as? Int ?? 0,
        "layer": window[kCGWindowLayer as String] as? Int ?? 0,
        "alpha": window[kCGWindowAlpha as String] as? Double ?? 0,
        "x": bx, "y": by, "width": width, "height": height,
    ]
    guard let data = try? JSONSerialization.data(withJSONObject: entry) else { continue }
    print(String(decoding: data, as: UTF8.self))
}
