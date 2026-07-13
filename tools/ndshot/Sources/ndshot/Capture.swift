import CoreGraphics
import Foundation
import ImageIO
import ScreenCaptureKit
import UniformTypeIdentifiers

struct CaptureOptions {
    var outPath: String?
    var pid: pid_t?
    var titleSubstring: String?
    var windowID: CGWindowID?
}

func parseCaptureArgs(_ args: [String]) -> CaptureOptions? {
    var options = CaptureOptions()
    var i = 0
    while i < args.count {
        let arg = args[i]
        func nextValue() -> String? {
            i += 1
            return i < args.count ? args[i] : nil
        }
        switch arg {
        case "--out":
            guard let value = nextValue() else {
                eprint("ndshot: --out requires a path")
                return nil
            }
            options.outPath = value
        case "--pid":
            guard let value = nextValue(), let parsed = pid_t(value) else {
                eprint("ndshot: --pid requires an integer")
                return nil
            }
            options.pid = parsed
        case "--title":
            guard let value = nextValue() else {
                eprint("ndshot: --title requires a value")
                return nil
            }
            options.titleSubstring = value
        case "--window-id":
            guard let value = nextValue(), let parsed = UInt32(value) else {
                eprint("ndshot: --window-id requires an integer")
                return nil
            }
            options.windowID = parsed
        default:
            eprint("ndshot: unknown option '\(arg)'")
            return nil
        }
        i += 1
    }
    return options
}

/// --window-id wins outright; otherwise --pid and --title compose (both
/// given must both match) and the first match in enumeration order wins.
func selectWindow(_ windows: [CapturableWindow], options: CaptureOptions) -> CapturableWindow? {
    if let windowID = options.windowID {
        return windows.first { $0.windowID == windowID }
    }
    return windows.first { window in
        if let pid = options.pid, window.pid != pid { return false }
        if let titleSubstring = options.titleSubstring,
            !window.title.lowercased().contains(titleSubstring.lowercased())
        {
            return false
        }
        return true
    }
}

func writePNG(_ image: CGImage, to path: String) -> Bool {
    let url = URL(fileURLWithPath: path)
    guard
        let destination = CGImageDestinationCreateWithURL(
            url as CFURL, UTType.png.identifier as CFString, 1, nil)
    else {
        return false
    }
    CGImageDestinationAddImage(destination, image, nil)
    return CGImageDestinationFinalize(destination)
}

func cmdCapture(_ args: [String]) async -> Int32 {
    guard let options = parseCaptureArgs(args) else {
        eprint(usageText)
        return 64
    }
    guard let outPath = options.outPath else {
        eprint("ndshot: --out <path.png> is required")
        eprint(usageText)
        return 64
    }
    guard options.pid != nil || options.titleSubstring != nil || options.windowID != nil else {
        eprint("ndshot: capture requires at least one of --pid, --title, --window-id")
        return 64
    }

    guard await ensurePermission() else { return 2 }

    let windows: [CapturableWindow]
    do {
        windows = try await fetchCapturableWindows()
    } catch {
        eprint("ndshot: failed to enumerate windows: \(error.localizedDescription)")
        return 4
    }

    guard var target = selectWindow(windows, options: options) else {
        eprint("ndshot: no window matched the given filters. Candidates:")
        for window in windows {
            eprint(window.jsonLine)
        }
        return 3
    }

    // A window mid-open-animation reports its animating (shrunken) frame —
    // observed ~102x104 for an 1100x700 window captured right after app
    // launch — and ScreenCaptureKit sizes the output from that snapshot.
    // Re-sample until the frame is stable across two consecutive reads
    // (250ms apart, ~3s cap), re-locating the same windowID each time.
    var settled = target.frame
    for _ in 0..<12 {
        try? await Task.sleep(nanoseconds: 250_000_000)
        guard let freshWindows = try? await fetchCapturableWindows(),
            let fresh = freshWindows.first(where: { $0.windowID == target.windowID })
        else { break }
        let stable = fresh.frame == settled
        settled = fresh.frame
        target = fresh
        if stable { break }
    }

    let filter = SCContentFilter(desktopIndependentWindow: target.window)
    let info = SCShareableContent.info(for: filter)
    let scale = min(max(CGFloat(info.pointPixelScale == 0 ? 1 : info.pointPixelScale), 1), 4)
    // info.contentRect can come back degenerate (observed ~102x107 for a
    // 1100x700 window mid-presentation) — the SCWindow frame is the reliable
    // size for a desktop-independent window capture; take the larger of the
    // two so a shrunken info rect can never squeeze the output.
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

    let image: CGImage
    do {
        image = try await SCScreenshotManager.captureImage(
            contentFilter: filter, configuration: config)
    } catch {
        eprint("ndshot: capture failed: \(error.localizedDescription)")
        return 4
    }

    guard writePNG(image, to: outPath) else {
        eprint("ndshot: failed to write PNG to \(outPath)")
        return 4
    }

    print("\(outPath) \(image.width)x\(image.height)")
    return 0
}
