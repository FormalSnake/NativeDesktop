import AppKit
import ImageIO
import ScreenCaptureKit
import UniformTypeIdentifiers

// Rung 0 of the screenshot ladder (Automation.swift), behind
// ND_AUTOMATION_CAPTURE=screencapturekit: capture the real composited window
// via ScreenCaptureKit — Liquid Glass sidebar included — instead of the
// TCC-free offscreen approximation. Opt-in because it prompts for
// screen-recording TCC on an ungranted machine; the default ladder keeps the
// stock-runner no-prompt contract. Lifted from tools/ndshot's Capture.swift.

private func ndWriteCapturedPNG(_ image: CGImage, to path: String) -> Bool {
    let url = URL(fileURLWithPath: path)
    guard let destination = CGImageDestinationCreateWithURL(
        url as CFURL, UTType.png.identifier as CFString, 1, nil)
    else { return false }
    CGImageDestinationAddImage(destination, image, nil)
    return CGImageDestinationFinalize(destination)
}

/// One SCK window capture. Fails (false) on missing TCC grant, an
/// unenumerable window, or any capture error — callers fall back to the
/// offscreen ladder.
func ndCaptureWindowSCK(windowID: CGWindowID, to path: String) async -> Bool {
    guard let content = try? await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false),
          let target = content.windows.first(where: { $0.windowID == windowID })
    else { return false }
    let filter = SCContentFilter(desktopIndependentWindow: target)
    let info = SCShareableContent.info(for: filter)
    let scale = min(max(CGFloat(info.pointPixelScale == 0 ? 1 : info.pointPixelScale), 1), 4)
    // info.contentRect can come back degenerate for a window
    // mid-presentation — the SCWindow frame is the reliable size for a
    // desktop-independent window capture; take the larger of the two so a
    // shrunken info rect can never squeeze the output.
    let infoSize = info.contentRect.size
    let contentSize = CGSize(
        width: max(infoSize.width.isFinite ? infoSize.width : 0, target.frame.size.width),
        height: max(infoSize.height.isFinite ? infoSize.height : 0, target.frame.size.height))

    let config = SCStreamConfiguration()
    config.width = max(1, Int((contentSize.width * scale).rounded()))
    config.height = max(1, Int((contentSize.height * scale).rounded()))
    config.showsCursor = false
    config.captureResolution = .best
    config.scalesToFit = false

    guard let image = try? await SCScreenshotManager.captureImage(
        contentFilter: filter, configuration: config)
    else { return false }
    return ndWriteCapturedPNG(image, to: path)
}

/// Synchronous main-thread bridge for `ndSnapshot`: the automation RPC
/// arrives as one marshaled main-thread call, but SCK is async-only. Spins
/// the main runloop (never a blocking semaphore — SCK internals may hop to
/// the main queue) until the detached capture lands or the deadline passes.
/// Automation serves one RPC at a time, so no second snapshot can interleave.
@MainActor func ndSnapshotViaSCK(_ window: NSWindow, _ path: String) -> Bool {
    let windowID = CGWindowID(window.windowNumber)
    guard window.windowNumber > 0 else { return false }
    // SCK samples the window server's composite, not the live view tree: a
    // committed-but-undisplayed subtree would capture stale (the ladder's
    // later displayIfNeeded runs AFTER this rung). Flush first.
    ndFlushWindowServerSurfaces()
    final class Box: @unchecked Sendable {
        var done = false
        var ok = false
        var timedOut = false
    }
    let box = Box()
    // Capture to a sibling temp path and promote on the main thread only
    // while the deadline hasn't passed: a capture that outlives the timeout
    // (the TCC prompt blocking SCShareableContent on an ungranted machine)
    // must not overwrite `path` AFTER the fallback ladder already wrote it
    // and the RPC answered.
    let tmpPath = path + ".sck-tmp"
    Task.detached {
        let ok = await ndCaptureWindowSCK(windowID: windowID, to: tmpPath)
        await MainActor.run {
            if box.timedOut {
                try? FileManager.default.removeItem(atPath: tmpPath)
                return
            }
            if ok {
                try? FileManager.default.removeItem(atPath: path)
                box.ok = (try? FileManager.default.moveItem(atPath: tmpPath, toPath: path)) != nil
            }
            box.done = true
        }
    }
    let deadline = Date().addingTimeInterval(5)
    while !box.done && Date() < deadline {
        RunLoop.main.run(mode: .default, before: Date().addingTimeInterval(0.05))
    }
    // Both sides of this flag run on the main thread, so the late capture
    // sees it before it can touch `path`.
    if !box.done { box.timedOut = true }
    return box.done && box.ok
}
