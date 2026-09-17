// On-screen window census for one process, used by the CEF gates to prove that
// no engine-created window ever becomes visible. Run it with `swift
// scripts/mac/window-census.swift <pid>`; it prints one JSON object per line.
//
// The window server, not the app, is the source of truth here: a Chromium
// window the app never hears about still shows up in this list. Titles need a
// Screen Recording grant and are absent without one, which is why the
// assertions ride on alpha, layer and bounds instead.
import CoreGraphics
import Foundation

let pid = CommandLine.arguments.count > 1 ? Int(CommandLine.arguments[1]) ?? 0 : 0
guard
    let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
        as? [[String: Any]]
else {
    FileHandle.standardError.write("window census unavailable\n".data(using: .utf8)!)
    exit(1)
}

for window in windows {
    guard let owner = window[kCGWindowOwnerPID as String] as? Int, owner == pid else { continue }
    let bounds = window[kCGWindowBounds as String] as? [String: Any] ?? [:]
    let entry: [String: Any] = [
        "number": window[kCGWindowNumber as String] as? Int ?? 0,
        "layer": window[kCGWindowLayer as String] as? Int ?? 0,
        "alpha": window[kCGWindowAlpha as String] as? Double ?? 0,
        "title": window[kCGWindowName as String] as? String ?? "",
        "x": bounds["X"] as? Double ?? 0,
        "y": bounds["Y"] as? Double ?? 0,
        "width": bounds["Width"] as? Double ?? 0,
        "height": bounds["Height"] as? Double ?? 0,
    ]
    guard let data = try? JSONSerialization.data(withJSONObject: entry) else { continue }
    print(String(decoding: data, as: UTF8.self))
}
