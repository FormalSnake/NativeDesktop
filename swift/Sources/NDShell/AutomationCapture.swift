import AppKit
import ImageIO
import ScreenCaptureKit
import UniformTypeIdentifiers

// Rung 0 of the screenshot ladder (Automation.swift), behind
// ND_AUTOMATION_CAPTURE=screencapturekit or =region: capture the real
// composited window via ScreenCaptureKit, Liquid Glass sidebar included,
// instead of the TCC-free offscreen approximation. Opt-in because it needs
// Screen Recording and takes focus; the default ladder keeps the stock-runner
// no-prompt contract. Lifted from tools/ndshot's Capture.swift.

private func ndWriteCapturedPNG(_ image: CGImage, to path: String) -> Bool {
    let url = URL(fileURLWithPath: path)
    guard let destination = CGImageDestinationCreateWithURL(
        url as CFURL, UTType.png.identifier as CFString, 1, nil)
    else { return false }
    CGImageDestinationAddImage(destination, image, nil)
    return CGImageDestinationFinalize(destination)
}

private func ndCaptureFail(_ why: String) -> Bool {
    FileHandle.standardError.write("ND_SNAPSHOT_SCK \(why)\n".data(using: .utf8)!)
    return false
}

/// One SCK window capture. Fails (false) on missing TCC grant, an
/// unenumerable window, or any capture error — callers fall back to the
/// offscreen ladder.
func ndCaptureWindowSCK(windowID: CGWindowID, to path: String) async -> Bool {
    guard let content = try? await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false),
          let target = content.windows.first(where: { $0.windowID == windowID })
    else { return ndCaptureFail("window \(windowID) not in shareable content") }
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

    do {
        let image = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
        return ndWriteCapturedPNG(image, to: path) || ndCaptureFail("could not write \(path)")
    } catch {
        return ndCaptureFail("capture error: \(error.localizedDescription)")
    }
}

// ND_AUTOMATION_CAPTURE=region: the screen area under the window with
// everything the app stacks on top of it. Context menus, alert sheets and
// popovers are windows of their own, and NSOpenPanel/NSSavePanel draw in a
// separate Apple process, so a single-window capture never contains them.
// Mirrors tools/ndshot's Region.swift.

private let ndPanelServicePrefix = "com.apple.appkit.xpc.openAndSavePanelService"

/// This process's windows plus the open/save panel service's, grown
/// transitively from the target frame, so a cascading submenu that leaves the
/// window still counts and a status item across the screen does not.
private func ndRegionWindows(target: SCWindow, in windows: [SCWindow]) -> ([SCWindow], CGRect) {
    let pid = target.owningApplication?.processID
    var pool = windows.filter { window in
        guard window.windowID != target.windowID, window.isOnScreen,
              let app = window.owningApplication,
              !window.frame.isEmpty, window.frame.origin.x.isFinite
        else { return false }
        return app.processID == pid || app.bundleIdentifier.hasPrefix(ndPanelServicePrefix)
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

func ndCaptureRegionSCK(windowID: CGWindowID, to path: String) async -> Bool {
    guard let content = try? await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true),
          let target = content.windows.first(where: { $0.windowID == windowID })
    else { return ndCaptureFail("window \(windowID) not on screen") }
    let (windows, region) = ndRegionWindows(target: target, in: content.windows)
    FileHandle.standardError.write(
        "ND_SNAPSHOT_REGION windows=\(windows.count) rect=\(Int(region.width))x\(Int(region.height)) ids=\(windows.map { "\($0.windowID):\($0.owningApplication?.processID ?? 0):\(Int($0.frame.width))x\(Int($0.frame.height))@\(Int($0.frame.minX)),\(Int($0.frame.minY))" })\n".data(using: .utf8)!)
    let center = CGPoint(x: target.frame.midX, y: target.frame.midY)
    guard let display = content.displays.first(where: { $0.frame.contains(center) }) ?? content.displays.first
    else { return ndCaptureFail("no display") }
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

    do {
        let image = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
        return ndWriteCapturedPNG(image, to: path) || ndCaptureFail("could not write \(path)")
    } catch {
        return ndCaptureFail("capture error: \(error.localizedDescription)")
    }
}

// The capture itself runs in a helper: this binary re-spawned with
// `--nd-capture` and responsibility disclaimed, the same trick tools/ndshot
// uses. TCC otherwise charges the Screen Recording check to whatever launched
// the host (a terminal, an agent, `bun test`), so the grant would have to follow
// every launcher. Disclaimed, the helper is its own responsible process and one
// grant to this binary (`NDShell --nd-grant`, or System Settings) covers every
// launch.

private typealias NDSetDisclaimFn = @convention(c) (UnsafeMutablePointer<posix_spawnattr_t?>, Int32) -> Int32

// replayd keys its clients by executable path: a second helper of the same
// binary connecting (two hosts screenshotting at once) cancels the first one's
// connection, and the request in flight on it never completes. ReplayKit
// reconnects and evicts the other helper in turn. So helpers of one binary take
// turns, holding the lock until exit; the kernel drops it when the host kills a
// helper at its deadline.
private func ndTakeCaptureLock(_ exe: String, waitingUpTo seconds: TimeInterval) -> Bool {
    var hash: UInt64 = 0xcbf2_9ce4_8422_2325
    for byte in exe.utf8 { hash = (hash ^ UInt64(byte)) &* 0x100_0000_01b3 }
    let path = FileManager.default.temporaryDirectory
        .appendingPathComponent("nd-capture-\(String(hash, radix: 16)).lock").path
    let fd = open(path, O_RDWR | O_CREAT | O_CLOEXEC, 0o600)
    guard fd >= 0 else { return true }
    let until = Date().addingTimeInterval(seconds)
    while flock(fd, LOCK_EX | LOCK_NB) != 0 {
        if Date() > until { return false }
        usleep(20_000)
    }
    return true
}

/// Entry point for `NDShell --nd-capture <windowID> <out.png> <WxH> [--region]`,
/// dispatched from main.swift before any app setup. Exit 0 on a written PNG,
/// 2 when Screen Recording is not granted or the window is gone, 4 otherwise.
func ndCaptureHelperMain(_ args: [String]) -> Int32 {
    guard args.count >= 3, let windowID = CGWindowID(args[0]) else { return 64 }
    let path = args[1]
    let expected = args[2].split(separator: "x").compactMap { Double($0) }
    let region = args.contains("--region")
    // SCK asserts in CGS machinery that was never initialised in a process
    // spawned outside a GUI app context.
    _ = CGMainDisplayID()
    // Exit 4 lets the host retry; its 5s kill bounds the whole helper.
    if let exe = Bundle.main.executablePath, !ndTakeCaptureLock(exe, waitingUpTo: 3) {
        FileHandle.standardError.write("ND_SNAPSHOT_SCK another capture helper of this binary is running\n".data(using: .utf8)!)
        return 4
    }
    final class Box: @unchecked Sendable { var code: Int32? }
    let box = Box()
    Task.detached {
        // A window mid-open-animation is listed at its animating, shrunken
        // frame, and SCK sizes the capture from it. Wait (up to 2s) for the
        // frame the host already knows it has.
        // Stage Manager shows a background app's window as a thumbnail in its
        // strip for as long as the app stays inactive: exit 3 so the host can
        // bring the window forward instead of writing the thumbnail.
        let deadline = Date().addingTimeInterval(2)
        while true {
            guard let content = try? await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false) else {
                box.code = 2
                return
            }
            let frame = content.windows.first(where: { $0.windowID == windowID })?.frame ?? .zero
            if expected.count != 2
                || (abs(frame.width - expected[0]) <= 1 && abs(frame.height - expected[1]) <= 1) { break }
            if Date() > deadline {
                FileHandle.standardError.write(
                    "ND_SNAPSHOT_SCK window server shows \(Int(frame.width))x\(Int(frame.height)), AppKit has \(Int(expected[0]))x\(Int(expected[1]))\n"
                        .data(using: .utf8)!)
                box.code = 3
                return
            }
            try? await Task.sleep(nanoseconds: 50_000_000)
        }
        let ok = region
            ? await ndCaptureRegionSCK(windowID: windowID, to: path)
            : await ndCaptureWindowSCK(windowID: windowID, to: path)
        box.code = ok ? 0 : 4
    }
    while box.code == nil {
        RunLoop.main.run(mode: .default, before: Date().addingTimeInterval(0.02))
    }
    return box.code!
}

private typealias NDSetFrontProcessFn = @convention(c) (
    UnsafeMutablePointer<ProcessSerialNumber>, UInt32, UInt32) -> Int32

/// Activation is cooperative since macOS 14, so `NSApp.activate` from a host
/// an agent or a test runner launched in the background is simply declined.
/// SkyLight's front-process call with the user-generated flag (what yabai
/// focuses windows with) is not; `activate` stays as the fallback.
private typealias NDPostEventRecordFn = @convention(c) (
    UnsafeMutablePointer<ProcessSerialNumber>, UnsafeMutablePointer<UInt8>) -> Int32

@MainActor func ndBringToFront(_ window: NSWindow) {
    NSApp.activate(ignoringOtherApps: true)
    guard let handle = dlopen("/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight", RTLD_LAZY),
          let setFrontSym = dlsym(handle, "_SLPSSetFrontProcessWithOptions"),
          let postSym = dlsym(handle, "SLPSPostEventRecordTo")
    else { return }
    let setFront = unsafeBitCast(setFrontSym, to: NDSetFrontProcessFn.self)
    let post = unsafeBitCast(postSym, to: NDPostEventRecordFn.self)
    var psn = ProcessSerialNumber(highLongOfPSN: 0, lowLongOfPSN: UInt32(kCurrentProcess))
    var windowID = UInt32(window.windowNumber)
    _ = setFront(&psn, windowID, 0x200) // kCPSUserGenerated
    // The window server's make-key handshake (a press/release pair of event
    // records naming the window), as yabai's window_manager_make_key_window
    // sends it; front process alone leaves key status where it was.
    for phase: UInt8 in [0x01, 0x02] {
        var bytes = [UInt8](repeating: 0, count: 0xf8)
        bytes[0x04] = 0xf8
        bytes[0x08] = phase
        bytes[0x3a] = 0x10
        withUnsafeBytes(of: &windowID) { id in
            for i in 0..<4 { bytes[0x3c + i] = id[i] }
        }
        for i in 0x20..<0x30 { bytes[i] = 0xff }
        _ = post(&psn, &bytes)
    }
}

/// Synchronous main-thread bridge for `ndSnapshot`: spawns the capture helper
/// and spins the main runloop (so an open context menu or modal sheet keeps
/// tracking) until it exits or the deadline passes.
@MainActor func ndSnapshotViaSCK(_ target: NSWindow, _ path: String, region: Bool) -> Bool {
    // With a sheet up the live content view resolves to the sheet; capture
    // the window it hangs off, which the region then grows to cover.
    var window = target
    while let parent = window.sheetParent { window = parent }
    guard window.windowNumber > 0, let exe = Bundle.main.executablePath else { return false }
    // SCK captures what the window server shows, so an inactive app captures
    // dimmed traffic lights and grey selections, and under Stage Manager a
    // thumbnail in the strip. Bring the window forward first. A window with a
    // sheet up keeps the sheet key.
    let focused = { NSApp.isActive && (window.isKeyWindow || window.attachedSheet != nil) }
    if !focused() {
        // The first request right after launch is sometimes dropped: ask
        // again every 250ms for up to a second.
        let until = Date().addingTimeInterval(1)
        var nextAsk = Date.distantPast
        while !focused() && Date() < until {
            if Date() >= nextAsk {
                ndBringToFront(window)
                if window.attachedSheet == nil { window.makeKeyAndOrderFront(nil) }
                nextAsk = Date().addingTimeInterval(0.25)
            }
            // Activation and key changes arrive as AppKit-defined events, which
            // only reach NSApp through nextEvent/sendEvent, never a bare runloop
            // spin. Input events stay queued for the app's own loop.
            if let event = NSApp.nextEvent(
                matching: [.appKitDefined, .systemDefined], until: Date().addingTimeInterval(0.02),
                inMode: .default, dequeue: true) {
                NSApp.sendEvent(event)
            }
        }
        if !focused() {
            let front = NSWorkspace.shared.frontmostApplication?.localizedName ?? "?"
            FileHandle.standardError.write(
                "ND_SNAPSHOT_FOCUS unfocused: active=\(NSApp.isActive) key=\(window.isKeyWindow) canKey=\(window.canBecomeKey) front=\(front)\n"
                    .data(using: .utf8)!)
        }
    }
    // SCK samples the window server's composite, not the live view tree: a
    // committed-but-undisplayed subtree would capture stale. Flush first.
    ndFlushWindowServerSurfaces()
    // replayd now and then refuses a stream ("audio/video capture failure")
    // and takes the same request a moment later.
    for _ in 0..<3 {
        switch ndRunCaptureHelper(exe, window.windowNumber, window.frame.size, path, region: region) {
        case 0: return true
        // 3: still animating out of the Stage Manager strip.
        case 3, 4: continue
        default: return false
        }
    }
    return false
}

@MainActor private func ndRunCaptureHelper(
    _ exe: String, _ windowNumber: Int, _ size: NSSize, _ path: String, region: Bool
) -> Int32 {

    var attr: posix_spawnattr_t?
    guard posix_spawnattr_init(&attr) == 0 else { return -1 }
    defer { posix_spawnattr_destroy(&attr) }
    if let sym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "responsibility_spawnattrs_setdisclaim") {
        _ = unsafeBitCast(sym, to: NDSetDisclaimFn.self)(&attr, 1)
    }
    // Written beside the target and promoted only on success, so a helper
    // killed at the deadline never leaves a half-written file where the
    // fallback ladder is about to write.
    let tmpPath = path + ".sck-tmp"
    var args = [exe, "--nd-capture", String(windowNumber), tmpPath, "\(Int(size.width))x\(Int(size.height))"]
    if region { args.append("--region") }
    var argv: [UnsafeMutablePointer<CChar>?] = args.map { strdup($0) } + [nil]
    defer { argv.forEach { free($0) } }
    var pid: pid_t = 0
    guard posix_spawn(&pid, exe, nil, &attr, argv, environ) == 0 else { return -1 }

    var status: Int32 = 0
    var exited = false
    let deadline = Date().addingTimeInterval(5)
    while Date() < deadline {
        if waitpid(pid, &status, WNOHANG) == pid { exited = true; break }
        RunLoop.main.run(mode: .default, before: Date().addingTimeInterval(0.02))
    }
    if !exited {
        kill(pid, SIGKILL)
        waitpid(pid, &status, 0)
        try? FileManager.default.removeItem(atPath: tmpPath)
        FileHandle.standardError.write("ND_SNAPSHOT_SCK timeout\n".data(using: .utf8)!)
        return -1
    }
    let code = (status & 0x7f) == 0 ? (status >> 8) & 0xff : -1
    guard code == 0 else {
        try? FileManager.default.removeItem(atPath: tmpPath)
        let why = code == 2 ? "no Screen Recording grant for \(exe) (run it with --nd-grant, SIP off)" : "exit \(code)"
        FileHandle.standardError.write("ND_SNAPSHOT_SCK failed: \(why)\n".data(using: .utf8)!)
        return code
    }
    try? FileManager.default.removeItem(atPath: path)
    return (try? FileManager.default.moveItem(atPath: tmpPath, toPath: path)) != nil ? 0 : -1
}
