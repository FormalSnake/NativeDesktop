import AppKit

// MARK: - Synthetic input — real NSEvents posted into the app's own
// event queue (NSApp.postEvent), never CGEvent/window-server injection:
// TCC-free, in-process, and consumed by AppKit's dispatch exactly like user
// input. Multi-event gestures (drag, double-click) are posted as ONE batch
// because controls run nested mouse-tracking loops inside mouseDown dispatch
// (NSSlider thumbs, NSButton press-and-hold, split dividers) that pull the
// rest of the gesture straight from the queue while blocking the main
// thread — marshaling one phase at a time would deadlock against them.

/// Monotone serial for synthetic mouse events.
nonisolated(unsafe) private var ndSyntheticEventNumber = 1000

/// The button a low-level `pointer down` left held, so a bare `move` phase
/// becomes a dragged-event of that button instead of a hover move.
nonisolated(unsafe) private var ndPointerHeldButton: String? = nil

@MainActor private func ndMouseTypes(_ button: String) -> (down: NSEvent.EventType, drag: NSEvent.EventType, up: NSEvent.EventType) {
    return button == "right"
        ? (.rightMouseDown, .rightMouseDragged, .rightMouseUp)
        : (.leftMouseDown, .leftMouseDragged, .leftMouseUp)
}

@MainActor private func ndPostMouse(_ win: NSWindow, _ type: NSEvent.EventType, _ windowPoint: NSPoint,
                                    clickCount: Int, timestamp: TimeInterval) {
    ndSyntheticEventNumber += 1
    let pressure: Float = (type == .leftMouseUp || type == .rightMouseUp || type == .mouseMoved) ? 0 : 1
    guard let e = NSEvent.mouseEvent(with: type, location: windowPoint, modifierFlags: [],
                                     timestamp: timestamp, windowNumber: win.windowNumber, context: nil,
                                     eventNumber: ndSyntheticEventNumber, clickCount: clickCount,
                                     pressure: pressure) else { return }
    NSApp.postEvent(e, atStart: false)
}

/// Synthetic input needs the target window key and the app active: AppKit
/// swallows the first click into a non-key window as window activation
/// (acceptsFirstMouse), and key events only reach a key window's responder
/// chain. A real user's click would focus the window the same way.
@MainActor private func ndEnsureActive(_ win: NSWindow) {
    NSApp.activate(ignoringOtherApps: true)
    if !win.isKeyWindow { win.makeKeyAndOrderFront(nil) }
}

/// Window + live content view for a Window node's handle (the handle itself
/// may be orphaned once a SplitView took over — `ndWindow(for:)` resolves
/// through the create-time registry).
@MainActor private func ndInputTarget(_ handle: NSView) -> (win: NSWindow, content: NSView)? {
    guard let win = ndWindow(for: handle), let content = ndLiveContentView(ofWindow: win) else { return nil }
    ndEnsureActive(win)
    return (win, content)
}

/// getTree's logical-window-topleft space → NSWindow base coordinates. The
/// inverse of `ndNodeBounds`' convert-into-content, so tree geometry and
/// synthetic input agree on every point by construction.
@MainActor private func ndWindowPoint(_ content: NSView, _ x: Double, _ y: Double) -> NSPoint {
    return content.convert(NSPoint(x: x, y: y), to: nil)
}

/// A widget's center in its window's base coordinates. Window-root handles
/// (orphaned content views) resolve to the live content's center.
@MainActor private func ndCenterInWindow(_ view: NSView) -> (win: NSWindow, point: NSPoint)? {
    if let win = view.window {
        let r = view.convert(view.bounds, to: nil)
        return (win, NSPoint(x: r.midX, y: r.midY))
    }
    guard let win = ndWindow(for: view), let content = ndLiveContentView(ofWindow: win) else { return nil }
    let b = content.bounds
    return (win, content.convert(NSPoint(x: b.midX, y: b.midY), to: nil))
}

@MainActor func ndPostPointerPhase(_ handle: NSView, phase: String, x: Double, y: Double,
                                   button: String, clickCount: Int) -> Bool {
    guard let t = ndInputTarget(handle) else { return false }
    let p = ndWindowPoint(t.content, x, y)
    let types = ndMouseTypes(button)
    let now = ProcessInfo.processInfo.systemUptime
    switch phase {
    case "down":
        ndPointerHeldButton = button
        ndPostMouse(t.win, types.down, p, clickCount: clickCount, timestamp: now)
    case "move":
        if let held = ndPointerHeldButton {
            ndPostMouse(t.win, ndMouseTypes(held).drag, p, clickCount: 0, timestamp: now)
        } else {
            t.win.acceptsMouseMovedEvents = true
            ndPostMouse(t.win, .mouseMoved, p, clickCount: 0, timestamp: now)
        }
    case "up":
        ndPointerHeldButton = nil
        ndPostMouse(t.win, types.up, p, clickCount: clickCount, timestamp: now)
    default:
        return false
    }
    return true
}

@MainActor func ndPostDrag(_ handle: NSView, fromX: Double, fromY: Double, toX: Double, toY: Double,
                           steps: Int, durationMs: Int, button: String) -> Bool {
    guard let t = ndInputTarget(handle) else { return false }
    let types = ndMouseTypes(button)
    let n = max(steps, 1)
    let base = ProcessInfo.processInfo.systemUptime
    let dt = (Double(max(durationMs, 0)) / 1000.0) / Double(n + 1)
    ndPostMouse(t.win, types.down, ndWindowPoint(t.content, fromX, fromY), clickCount: 1, timestamp: base)
    for i in 1...n {
        let f = Double(i) / Double(n)
        let x = fromX + (toX - fromX) * f
        let y = fromY + (toY - fromY) * f
        ndPostMouse(t.win, types.drag, ndWindowPoint(t.content, x, y), clickCount: 1, timestamp: base + dt * Double(i))
    }
    ndPostMouse(t.win, types.up, ndWindowPoint(t.content, toX, toY), clickCount: 1, timestamp: base + dt * Double(n + 1))
    return true
}

/// Full click sequence at a widget's center (doubleClick posts two pairs
/// with clickCount 1 then 2, like real AppKit double-clicks). `dismissAfter`
/// (rightClick) appends an escape so a context menu the click opened cannot
/// wedge the main thread's menu-tracking loop against later marshaled calls
/// — GCD's main queue is not serviced in NSEventTrackingRunLoopMode.
@MainActor func ndPostClicksAtCenter(_ view: NSView, button: String, clicks: Int, dismissAfter: Bool) -> Bool {
    guard let target = ndCenterInWindow(view) else { return false }
    ndEnsureActive(target.win)
    let types = ndMouseTypes(button)
    var now = ProcessInfo.processInfo.systemUptime
    for c in 1...max(clicks, 1) {
        ndPostMouse(target.win, types.down, target.point, clickCount: c, timestamp: now)
        now += 0.01
        ndPostMouse(target.win, types.up, target.point, clickCount: c, timestamp: now)
        now += 0.01
    }
    if dismissAfter {
        ndPostKeyChord(target.win, chars: "\u{1b}", ignoring: "\u{1b}", code: 53, flags: [])
    }
    return true
}

@MainActor func ndPostHoverAtCenter(_ view: NSView) -> Bool {
    guard let target = ndCenterInWindow(view) else { return false }
    target.win.acceptsMouseMovedEvents = true
    ndPostMouse(target.win, .mouseMoved, target.point, clickCount: 0,
                timestamp: ProcessInfo.processInfo.systemUptime)
    return true
}

// MARK: - Keyboard synthesis

/// ANSI/US keycodes for the printable keys the automation layer types.
/// keyCode matters for menu key equivalents and arrow/function handling;
/// plain text insertion goes by `characters`, so unmapped characters still
/// type correctly with keyCode 0.
private let ndKeyCodes: [String: UInt16] = [
    "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9,
    "b": 11, "q": 12, "w": 13, "e": 14, "r": 15, "y": 16, "t": 17,
    "1": 18, "2": 19, "3": 20, "4": 21, "6": 22, "5": 23, "=": 24, "9": 25, "7": 26,
    "-": 27, "8": 28, "0": 29, "]": 30, "o": 31, "u": 32, "[": 33, "i": 34, "p": 35,
    "l": 37, "j": 38, "'": 39, "k": 40, ";": 41, "\\": 42, ",": 43, "/": 44, "n": 45,
    "m": 46, ".": 47, "`": 50,
]

/// Named non-printable keys; arrow characters are AppKit's function-key
/// unicode points (NSUpArrowFunctionKey et al).
private let ndNamedKeys: [String: (chars: String, code: UInt16)] = [
    "return": ("\r", 36), "enter": ("\r", 36), "tab": ("\t", 48), "space": (" ", 49),
    "escape": ("\u{1b}", 53), "esc": ("\u{1b}", 53),
    "delete": ("\u{7f}", 51), "backspace": ("\u{7f}", 51),
    "left": ("\u{F702}", 123), "right": ("\u{F703}", 124),
    "down": ("\u{F701}", 125), "up": ("\u{F700}", 126),
]

@MainActor private func ndPostKeyChord(_ win: NSWindow, chars: String, ignoring: String,
                                       code: UInt16, flags: NSEvent.ModifierFlags) {
    let now = ProcessInfo.processInfo.systemUptime
    if let down = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: flags, timestamp: now,
                                   windowNumber: win.windowNumber, context: nil, characters: chars,
                                   charactersIgnoringModifiers: ignoring, isARepeat: false, keyCode: code) {
        NSApp.postEvent(down, atStart: false)
    }
    if let up = NSEvent.keyEvent(with: .keyUp, location: .zero, modifierFlags: flags, timestamp: now + 0.01,
                                 windowNumber: win.windowNumber, context: nil, characters: chars,
                                 charactersIgnoringModifiers: ignoring, isARepeat: false, keyCode: code) {
        NSApp.postEvent(up, atStart: false)
    }
}

@MainActor private func ndPostCharacter(_ win: NSWindow, _ ch: Character, flags: NSEvent.ModifierFlags = []) {
    var f = flags
    let s = String(ch)
    let lower = s.lowercased()
    if ch.isUppercase { f.insert(.shift) }
    ndPostKeyChord(win, chars: s, ignoring: lower, code: ndKeyCodes[lower] ?? 0, flags: f)
}

@MainActor private func ndPostKeyToken(_ win: NSWindow, _ token: String, _ flags: NSEvent.ModifierFlags) -> Bool {
    if let named = ndNamedKeys[token] {
        ndPostKeyChord(win, chars: named.chars, ignoring: named.chars, code: named.code, flags: flags)
        return true
    }
    guard token.count == 1, let ch = token.first else { return false }
    ndPostCharacter(win, ch, flags: flags)
    return true
}

/// `keys` spec: "cmd+shift+n" presses one chord (driving menu key
/// equivalents through the normal sendEvent path); a bare named key
/// ("escape", "tab") presses that key; any other string types its
/// characters into the focused widget one keystroke at a time.
@MainActor func ndPostKeys(_ handle: NSView, spec: String) -> Bool {
    guard let t = ndInputTarget(handle) else { return false }
    if spec.contains("+") {
        var flags: NSEvent.ModifierFlags = []
        var keyToken: String? = nil
        for part in spec.split(separator: "+").map({ String($0).lowercased() }) {
            switch part {
            case "cmd", "command", "meta": flags.insert(.command)
            case "shift": flags.insert(.shift)
            case "alt", "option", "opt": flags.insert(.option)
            case "ctrl", "control": flags.insert(.control)
            default: keyToken = part
            }
        }
        guard let token = keyToken else { return false }
        return ndPostKeyToken(t.win, token, flags)
    }
    if spec.count > 1, ndNamedKeys[spec.lowercased()] != nil {
        return ndPostKeyToken(t.win, spec.lowercased(), [])
    }
    for ch in spec {
        ndPostCharacter(t.win, ch)
    }
    return !spec.isEmpty
}
