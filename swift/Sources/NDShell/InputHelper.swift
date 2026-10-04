import AppKit
import ApplicationServices
import Carbon
import Foundation

// `NDShell --nd-input`: drives the real system cursor for @nativedesktop/test's
// `app.cursor`. The `pointer`/`drag` RPCs post NSEvents into the app's own
// queue, which AppKit's tracking loops, hover tracking areas and context menus
// only partly believe; events posted at the HID tap here are indistinguishable
// from a physical mouse. Reads one JSON command per line on stdin and answers
// one JSON line each on stdout. Coordinates are global logical points with a
// top-left origin, the space WindowInfo.geometry reports.
//
// Posting at the HID tap needs Accessibility, checked against the responsible
// process, so the helper re-spawns itself disclaimed like the capture helper
// and one grant to this binary covers every launcher (`--nd-grant`).

private let ndInputDisclaimMarker = "ND_INPUT_DISCLAIMED"

private typealias NDInputDisclaimFn = @convention(c) (UnsafeMutablePointer<posix_spawnattr_t?>, Int32) -> Int32

/// Re-runs this invocation as its own responsible process, stdio inherited,
/// and exits with its status. Returns only in the child, or when the private
/// symbol is missing (the helper then runs under the launcher's grant).
private func ndInputRespawnDisclaimed() {
    if ProcessInfo.processInfo.environment[ndInputDisclaimMarker] == "1" { return }
    guard let sym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "responsibility_spawnattrs_setdisclaim"),
          let exe = Bundle.main.executablePath
    else { return }
    var attr: posix_spawnattr_t?
    guard posix_spawnattr_init(&attr) == 0 else { return }
    defer { posix_spawnattr_destroy(&attr) }
    guard unsafeBitCast(sym, to: NDInputDisclaimFn.self)(&attr, 1) == 0 else { return }
    var argv: [UnsafeMutablePointer<CChar>?] = CommandLine.arguments.map { strdup($0) } + [nil]
    var env = ProcessInfo.processInfo.environment
    env[ndInputDisclaimMarker] = "1"
    var envp: [UnsafeMutablePointer<CChar>?] = env.map { strdup("\($0.key)=\($0.value)") } + [nil]
    var pid: pid_t = 0
    guard posix_spawn(&pid, exe, nil, &attr, &argv, &envp) == 0 else { return }
    var status: Int32 = 0
    while waitpid(pid, &status, 0) == -1 && errno == EINTR {}
    exit((status & 0x7f) == 0 ? (status >> 8) & 0xff : 1)
}

private struct NDInputState {
    var position: CGPoint = CGEvent(source: nil)?.location ?? .zero
    var held: CGMouseButton?
    var clickCount: Int64 = 1
}

private func ndMouseTypes(_ button: CGMouseButton) -> (down: CGEventType, up: CGEventType, drag: CGEventType) {
    switch button {
    case .right: return (.rightMouseDown, .rightMouseUp, .rightMouseDragged)
    case .center: return (.otherMouseDown, .otherMouseUp, .otherMouseDragged)
    default: return (.leftMouseDown, .leftMouseUp, .leftMouseDragged)
    }
}

private func ndButton(_ name: Any?) -> CGMouseButton {
    switch name as? String {
    case "right": return .right
    case "middle": return .center
    default: return .left
    }
}

/// A click's modifier keys ride on the event itself: the keyboard's own state
/// is not consulted for a synthesized mouse event, so a cmd-click posted while
/// nothing holds cmd down still has to say so.
private func ndFlags(_ names: Any?) -> CGEventFlags {
    var flags: CGEventFlags = []
    for name in names as? [String] ?? [] {
        switch name {
        case "command": flags.insert(.maskCommand)
        case "shift": flags.insert(.maskShift)
        case "option": flags.insert(.maskAlternate)
        case "control": flags.insert(.maskControl)
        default: break
        }
    }
    return flags
}

private func ndPost(
    _ type: CGEventType, _ at: CGPoint, _ button: CGMouseButton, clickCount: Int64 = 1, flags: CGEventFlags = []
) {
    guard let event = CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: at, mouseButton: button)
    else { return }
    event.setIntegerValueField(.mouseEventClickState, value: clickCount)
    if !flags.isEmpty { event.flags = flags }
    event.post(tap: .cghidEventTap)
}

/// Brings the app that owns `pid` forward so the events land in its window
/// rather than whatever sits on top.
private func ndInputFocus(_ pid: pid_t) -> Bool {
    let app = AXUIElementCreateApplication(pid)
    let front = AXUIElementSetAttributeValue(app, kAXFrontmostAttribute as CFString, kCFBooleanTrue)
    var value: CFTypeRef?
    if AXUIElementCopyAttributeValue(app, kAXMainWindowAttribute as CFString, &value) == .success,
       let value, CFGetTypeID(value) == AXUIElementGetTypeID() {
        AXUIElementPerformAction(value as! AXUIElement, kAXRaiseAction as CFString)
    }
    return front == .success
}

private func ndInputRun(_ cmd: [String: Any], _ state: inout NDInputState) -> [String: Any] {
    let op = cmd["op"] as? String ?? ""
    let x = (cmd["x"] as? NSNumber)?.doubleValue
    let y = (cmd["y"] as? NSNumber)?.doubleValue
    // A little time between events, or AppKit coalesces a drag into its end
    // point and a click into nothing.
    let gap: useconds_t = 8_000
    switch op {
    case "trusted":
        return ["ok": true, "trusted": AXIsProcessTrusted()]
    case "position":
        let at = CGEvent(source: nil)?.location ?? state.position
        return ["ok": true, "x": at.x, "y": at.y]
    case "focus":
        guard let pid = (cmd["pid"] as? NSNumber)?.int32Value else { return ["ok": false, "error": "focus needs pid"] }
        return ["ok": ndInputFocus(pid)]
    case "move":
        guard let x, let y else { return ["ok": false, "error": "move needs x and y"] }
        let to = CGPoint(x: x, y: y)
        let steps = max(1, (cmd["steps"] as? NSNumber)?.intValue ?? 1)
        let interval = (cmd["intervalMs"] as? NSNumber).map { useconds_t(max(1, $0.doubleValue * 1000)) } ?? gap
        let from = state.position
        for i in 1...steps {
            let t = Double(i) / Double(steps)
            let at = CGPoint(x: from.x + (to.x - from.x) * t, y: from.y + (to.y - from.y) * t)
            if let held = state.held {
                ndPost(ndMouseTypes(held).drag, at, held)
            } else {
                ndPost(.mouseMoved, at, .left)
            }
            usleep(interval)
        }
        state.position = to
        return ["ok": true]
    case "down":
        let button = ndButton(cmd["button"])
        state.clickCount = (cmd["clickCount"] as? NSNumber)?.int64Value ?? 1
        ndPost(ndMouseTypes(button).down, state.position, button, clickCount: state.clickCount,
               flags: ndFlags(cmd["modifiers"]))
        state.held = button
        usleep(gap)
        return ["ok": true]
    case "up":
        let button = ndButton(cmd["button"])
        ndPost(ndMouseTypes(button).up, state.position, button, clickCount: state.clickCount,
               flags: ndFlags(cmd["modifiers"]))
        state.held = nil
        usleep(gap)
        return ["ok": true]
    case "scroll":
        let dx = Int32((cmd["dx"] as? NSNumber)?.intValue ?? 0)
        let dy = Int32((cmd["dy"] as? NSNumber)?.intValue ?? 0)
        guard let event = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 2,
                                  wheel1: dy, wheel2: dx, wheel3: 0)
        else { return ["ok": false, "error": "could not create scroll event"] }
        event.location = state.position
        // A trackpad's two-finger scroll: continuous, with the gesture's phase
        // (CGScrollPhase) and then the momentum's (CGMomentumScrollPhase).
        if let phase = cmd["phase"] as? String {
            let value: Int64 = ["began": 1, "changed": 2, "ended": 4][phase] ?? 2
            event.setIntegerValueField(.scrollWheelEventIsContinuous, value: 1)
            event.setIntegerValueField(.scrollWheelEventScrollPhase, value: value)
        }
        if let momentum = cmd["momentumPhase"] as? String {
            let value: Int64 = ["began": 1, "changed": 2, "ended": 3][momentum] ?? 2
            event.setIntegerValueField(.scrollWheelEventIsContinuous, value: 1)
            event.setIntegerValueField(.scrollWheelEventMomentumPhase, value: value)
        }
        event.post(tap: .cghidEventTap)
        usleep(gap)
        return ["ok": true]
    case "magnify":
        // A trackpad pinch. CoreGraphics has no public constructor for gesture
        // events; this is the window server's own layout for one (event type
        // 29, gesture subtype 8 = zoom, the magnification as a double, the
        // phase as 1 began, 2 changed, 4 ended), which AppKit turns into an
        // NSEvent of type .magnify for the view under the pointer.
        let amount = (cmd["magnification"] as? NSNumber)?.doubleValue ?? 0
        let phase: Int64 = ["began": 1, "changed": 2, "ended": 4][cmd["phase"] as? String ?? ""] ?? 2
        guard let event = CGEvent(source: nil) else { return ["ok": false, "error": "could not create gesture event"] }
        event.type = CGEventType(rawValue: 29)!
        event.location = state.position
        event.setIntegerValueField(CGEventField(rawValue: 110)!, value: 8)
        event.setDoubleValueField(CGEventField(rawValue: 113)!, value: amount)
        event.setIntegerValueField(CGEventField(rawValue: 132)!, value: phase)
        event.post(tap: .cghidEventTap)
        usleep(gap)
        return ["ok": true]
    case "key":
        // A keypress at the HID level, so it reaches whatever has the keyboard
        // the way a physical key does: a page's own shortcut handling, the
        // menu bar's key equivalents, Chromium's accelerator table.
        var flags: CGEventFlags = []
        var code = (cmd["keyCode"] as? NSNumber)?.uint16Value
        if let char = cmd["char"] as? String, let typed = ndKeyTyping(char) {
            code = typed.code
            if typed.shift { flags.insert(.maskShift) }
        }
        guard let code else { return ["ok": false, "error": "key needs keyCode or a char the keyboard layout types"] }
        for name in cmd["modifiers"] as? [String] ?? [] {
            switch name {
            case "command": flags.insert(.maskCommand)
            case "shift": flags.insert(.maskShift)
            case "option": flags.insert(.maskAlternate)
            case "control": flags.insert(.maskControl)
            default: break
            }
        }
        for down in [true, false] {
            guard let event = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: down)
            else { return ["ok": false, "error": "could not create key event"] }
            event.flags = flags
            event.post(tap: .cghidEventTap)
            usleep(gap)
        }
        return ["ok": true]
    default:
        return ["ok": false, "error": "unknown op '\(op)'"]
    }
}

/// The key, and whether it needs shift, that types `char` on the current
/// keyboard layout. A key code names a physical key, and "=" on a US layout is
/// "¡" on a Spanish one, so a chord spelled by its character is resolved here.
private func ndKeyTyping(_ char: String) -> (code: UInt16, shift: Bool)? {
    guard let source = TISCopyCurrentKeyboardLayoutInputSource()?.takeRetainedValue(),
          let raw = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData)
    else { return nil }
    let data = Unmanaged<CFData>.fromOpaque(raw).takeUnretainedValue() as Data
    return data.withUnsafeBytes { bytes -> (UInt16, Bool)? in
        guard let layout = bytes.baseAddress?.assumingMemoryBound(to: UCKeyboardLayout.self) else { return nil }
        for shift in [false, true] {
            for code in UInt16(0)..<128 {
                var dead: UInt32 = 0
                var length = 0
                var chars = [UniChar](repeating: 0, count: 4)
                let mods: UInt32 = shift ? UInt32(shiftKey >> 8) & 0xFF : 0
                let status = UCKeyTranslate(layout, code, UInt16(kUCKeyActionDown), mods, UInt32(LMGetKbdType()),
                                            OptionBits(kUCKeyTranslateNoDeadKeysBit), &dead, chars.count, &length, &chars)
                if status == noErr, length > 0, String(utf16CodeUnits: chars, count: length) == char { return (code, shift) }
            }
        }
        return nil
    }
}

/// Entry point for `NDShell --nd-input`, dispatched from main.swift before any
/// app setup. Exits 0 at end of input.
func ndInputHelperMain() -> Int32 {
    ndInputRespawnDisclaimed()
    _ = CGMainDisplayID()
    var state = NDInputState()
    while let line = readLine() {
        let reply: [String: Any]
        if let data = line.data(using: .utf8),
           let cmd = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] {
            if !AXIsProcessTrusted(), cmd["op"] as? String != "trusted" {
                reply = ["ok": false, "error": "not trusted for Accessibility: grant \(Bundle.main.executablePath ?? "this binary") (--nd-grant with SIP off, or System Settings)"]
            } else {
                reply = ndInputRun(cmd, &state)
            }
        } else {
            reply = ["ok": false, "error": "not a JSON object"]
        }
        let out = (try? JSONSerialization.data(withJSONObject: reply)) ?? Data("{\"ok\":false}".utf8)
        FileHandle.standardOutput.write(out + Data("\n".utf8))
    }
    return 0
}
