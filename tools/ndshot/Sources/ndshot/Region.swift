import CoreGraphics
import Foundation
import ScreenCaptureKit

// `capture --region`: the screen area under a window, with everything the app
// stacks on top of it. Context menus, alert sheets and popovers are windows of
// their own, and NSOpenPanel/NSSavePanel draw in a separate Apple process, so a
// single-window capture never contains them. Mirrored in the host's
// AutomationCapture.swift.

private let panelServicePrefix = "com.apple.appkit.xpc.openAndSavePanelService"

/// Windows that belong in a region capture of `target`: its own process's
/// windows plus the open/save panel service's, grown transitively from the
/// target frame, so a cascading submenu that leaves the window still counts and
/// a status item across the screen does not.
func regionWindows(target: SCWindow, in windows: [SCWindow]) -> (windows: [SCWindow], region: CGRect) {
    let pid = target.owningApplication?.processID
    var pool = windows.filter { window in
        guard window.windowID != target.windowID, window.isOnScreen,
              let app = window.owningApplication,
              !window.frame.isEmpty, window.frame.origin.x.isFinite
        else { return false }
        return app.processID == pid || app.bundleIdentifier.hasPrefix(panelServicePrefix)
    }
    var picked = [target]
    var region = target.frame
    var grew = true
    while grew {
        grew = false
        pool.removeAll { window in
            guard window.frame.intersects(region) else { return false }
            picked.append(window)
            region = region.union(window.frame)
            grew = true
            return true
        }
    }
    return (picked, region)
}

func captureRegion(target: SCWindow, content: SCShareableContent) async throws -> CGImage {
    let (windows, region) = regionWindows(target: target, in: content.windows)
    let center = CGPoint(x: target.frame.midX, y: target.frame.midY)
    guard let display = content.displays.first(where: { $0.frame.contains(center) }) ?? content.displays.first
    else { throw CocoaError(.featureUnsupported) }
    let clipped = region.intersection(display.frame)

    let filter = SCContentFilter(display: display, including: windows)
    let info = SCShareableContent.info(for: filter)
    let scale = min(max(CGFloat(info.pointPixelScale == 0 ? 1 : info.pointPixelScale), 1), 4)

    let config = SCStreamConfiguration()
    config.sourceRect = clipped.offsetBy(dx: -display.frame.minX, dy: -display.frame.minY)
    config.width = max(1, Int((clipped.width * scale).rounded()))
    config.height = max(1, Int((clipped.height * scale).rounded()))
    config.showsCursor = false
    config.captureResolution = .best
    config.scalesToFit = false
    return try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
}
