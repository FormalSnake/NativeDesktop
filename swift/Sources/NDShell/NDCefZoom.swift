#if canImport(CCef)
import AppKit
import CCef

/// Page zoom without Chromium's zoom bubble.
///
/// Every zoom change in Chrome style raises `ZoomBubbleView`, and it anchors to
/// the location bar's zoom icon. This browser has no location bar, so the
/// bubble is anchored to nothing and `NDCefSurfaceWindows` would place it over
/// the top centre of the page. There is no CEF switch for it
/// (`ZoomController::SetShowsNotificationBubble` is C++ only), so each zoom this
/// engine makes closes the bubble window it produces, and the app hears the new
/// factor as `zoomChanged` and draws its own indicator.

/// Chrome's zoom commands, which a key chord and ctrl+wheel both run.
private let ndCefZoomCommands: [Int32: cef_zoom_command_t] = {
    var map: [Int32: cef_zoom_command_t] = [:]
    for (name, command) in [
        ("IDC_ZOOM_PLUS", CEF_ZOOM_COMMAND_IN),
        ("IDC_ZOOM_MINUS", CEF_ZOOM_COMMAND_OUT),
        ("IDC_ZOOM_NORMAL", CEF_ZOOM_COMMAND_RESET),
    ] {
        let id = name.withCString { nd_cef_command_id($0) }
        if id >= 0 { map[id] = command }
    }
    return map
}()

/// Serves a zoom command in place of Chrome, which would show its bubble.
/// `host.zoom` steps through the same preset levels Chrome's command does.
func ndCefServeZoomCommand(_ handler: UnsafeMutableRawPointer?, _ commandID: Int32) -> Bool {
    guard let command = ndCefZoomCommands[commandID] else { return false }
    ndCefDeliver(handler) { view in
        view?.ndChangeZoom(source: "page") { $0.pointee.zoom?($0, command) }
    }
    return true
}

extension NDCefWebView {
    /// Runs one zoom change with the bubble it raises closed, then reports the
    /// resulting factor.
    func ndChangeZoom(source: String, _ change: (UnsafeMutablePointer<cef_browser_host_t>) -> Void) {
        guard let browserHost = browserHost() else { return }
        NDCefZoomBubble.suppress { change(browserHost) }
        nd_cef_ref_release(browserHost)
        ndReportZoom(source: source)
    }

    /// Emits `zoomChanged` when the level differs from the one last reported.
    /// Navigation is a source too: Chromium restores a host's saved level when
    /// a page of that host commits.
    func ndReportZoom(source: String) {
        guard let browserHost = browserHost() else { return }
        defer { nd_cef_ref_release(browserHost) }
        guard let level = browserHost.pointee.get_zoom_level?(browserHost) else { return }
        guard abs(level - reportedZoomLevel) > 1e-6 else { return }
        reportedZoomLevel = level
        let factor = (pow(1.2, level) * 1000).rounded() / 1000
        ndTrace("zoomChanged factor=\(factor) source=\(source)")
        emitData("zoomChanged", ["factor": factor, "source": source])
    }
}

/// Closes the zoom bubble a zoom change raises.
///
/// The bubble is a Views window (`NativeWidgetMacNSWindow`) created while the
/// change runs, or shortly after it. The windows that existed before are
/// remembered, and any small Views window that appears in the next 600ms is the
/// bubble. It is closed rather than hidden: a hidden bubble is still "showing"
/// to Chrome's coordinator, which would refresh it instead of making another.
@MainActor enum NDCefZoomBubble {
    private static let viewsWindowClass = "NativeWidgetMacNSWindow"
    /// The zoom bubble is one row of controls, 48pt tall on macOS 27. A sheet
    /// or a prompt that happens to open in the same instant is taller.
    private static let maxHeight: CGFloat = 80
    private static let window: TimeInterval = 0.6

    private static var known: Set<ObjectIdentifier> = []
    private static var until: Date?

    static func suppress(_ change: () -> Void) {
        if until == nil { known = Set(viewsWindows().map(ObjectIdentifier.init)) }
        change()
        until = Date().addingTimeInterval(window)
        closeNew()
        for delay in [0.016, 0.05, 0.1, 0.2, 0.4, window + 0.05] {
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
                MainActor.assumeIsolated { closeNew() }
            }
        }
    }

    private static func viewsWindows() -> [NSWindow] {
        NSApp.windows.filter { NSStringFromClass(type(of: $0)).contains(viewsWindowClass) }
    }

    private static func closeNew() {
        guard let deadline = until else { return }
        for window in viewsWindows() where !known.contains(ObjectIdentifier(window)) {
            known.insert(ObjectIdentifier(window))
            guard window.frame.height <= maxHeight else { continue }
            if ProcessInfo.processInfo.environment["ND_WEBVIEW_TRACE"] == "1" {
                FileHandle.standardError.write(
                    "ND_WV cef zoom bubble closed \(window.frame.size) visible=\(window.isVisible)\n".data(using: .utf8)!)
            }
            window.parent?.removeChildWindow(window)
            window.orderOut(nil)
            window.close()
        }
        if Date() >= deadline {
            until = nil
            known = []
        }
    }
}
#endif
