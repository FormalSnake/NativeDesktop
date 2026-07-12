import Foundation
import ScreenCaptureKit

/// Thin wrapper around SCWindow that knows how to render itself as the
/// one-line JSON object `list`/`capture`'s error paths print.
struct CapturableWindow {
    let window: SCWindow

    var pid: pid_t { window.owningApplication?.processID ?? 0 }
    var windowID: CGWindowID { window.windowID }
    var appName: String { window.owningApplication?.applicationName ?? "" }
    var title: String { window.title ?? "" }
    var frame: CGRect { window.frame }
    var onScreen: Bool { window.isOnScreen }

    var jsonLine: String {
        let object: [String: Any] = [
            "pid": pid,
            "windowID": windowID,
            "app": appName,
            "title": title,
            "x": frame.origin.x,
            "y": frame.origin.y,
            "width": frame.size.width,
            "height": frame.size.height,
            "onScreen": onScreen,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: object) else {
            return "{}"
        }
        return String(data: data, encoding: .utf8) ?? "{}"
    }
}

func fetchCapturableWindows() async throws -> [CapturableWindow] {
    let content = try await SCShareableContent.excludingDesktopWindows(
        false, onScreenWindowsOnly: false)
    return content.windows.map(CapturableWindow.init)
}

func cmdList() async -> Int32 {
    guard ensurePermission() else { return 2 }

    do {
        let windows = try await fetchCapturableWindows()
        for window in windows {
            print(window.jsonLine)
        }
        return 0
    } catch {
        eprint("ndshot: failed to enumerate windows: \(error.localizedDescription)")
        return 1
    }
}
