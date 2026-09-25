// Samples one process's on-screen windows every `interval` ms for `duration`
// ms and prints each window the first time it is seen, as window-census.swift
// does, so a surface that is on screen for a single frame is still caught.
// Usage: swift scripts/mac/window-watch.swift <pid> <duration ms> <interval ms>.
// Prints "ready" once the first sample is taken; windows present then are the
// baseline and are not reported.
import CoreGraphics
import Foundation

let args = CommandLine.arguments
let pid = args.count > 1 ? Int(args[1]) ?? 0 : 0
let duration = args.count > 2 ? Double(args[2]) ?? 3000 : 3000
let interval = args.count > 3 ? Double(args[3]) ?? 16 : 16

func sample() -> [[String: Any]] {
    guard let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
        as? [[String: Any]] else { return [] }
    return windows.filter { ($0[kCGWindowOwnerPID as String] as? Int) == pid }
}

var seen = Set(sample().compactMap { $0[kCGWindowNumber as String] as? Int })
print("ready")
fflush(stdout)
let end = Date().addingTimeInterval(duration / 1000)
while Date() < end {
    for window in sample() {
        guard let number = window[kCGWindowNumber as String] as? Int, !seen.contains(number) else { continue }
        seen.insert(number)
        let bounds = window[kCGWindowBounds as String] as? [String: Any] ?? [:]
        let entry: [String: Any] = [
            "number": number,
            "layer": window[kCGWindowLayer as String] as? Int ?? 0,
            "alpha": window[kCGWindowAlpha as String] as? Double ?? 0,
            "title": window[kCGWindowName as String] as? String ?? "",
            "x": bounds["X"] as? Double ?? 0,
            "y": bounds["Y"] as? Double ?? 0,
            "width": bounds["Width"] as? Double ?? 0,
            "height": bounds["Height"] as? Double ?? 0,
        ]
        if let data = try? JSONSerialization.data(withJSONObject: entry) {
            print(String(decoding: data, as: UTF8.self))
            fflush(stdout)
        }
    }
    Thread.sleep(forTimeInterval: interval / 1000)
}
