import ApplicationServices
import Foundation

// Brings the target window forward and makes it key before a capture, so the
// picture shows the focused state (coloured traffic lights, accent selection)
// and not a Stage Manager thumbnail. Activation is cooperative since macOS 14,
// so this goes through SkyLight the way yabai focuses windows: front process
// with the user-generated flag, then the make-key event-record pair. Mirrors
// the host's ndBringToFront (AutomationCapture.swift). From outside the app
// SkyLight alone makes the process frontmost without the app taking key, so
// the Accessibility route runs first whenever it is granted.

private typealias SetFrontProcessFn = @convention(c) (
    UnsafeMutablePointer<ProcessSerialNumber>, UInt32, UInt32) -> Int32
private typealias PostEventRecordFn = @convention(c) (
    UnsafeMutablePointer<ProcessSerialNumber>, UnsafeMutablePointer<UInt8>) -> Int32
private typealias GetProcessForPIDFn = @convention(c) (
    pid_t, UnsafeMutablePointer<ProcessSerialNumber>) -> OSStatus

/// Raises the window and makes its app frontmost through Accessibility, which
/// the target app honours even when it was launched in the background. Needs
/// the Accessibility grant (`doctor --grant`, or System Settings); returns false
/// without it.
private func focusViaAccessibility(pid: pid_t, frame: CGRect) -> Bool {
    guard AXIsProcessTrusted() else { return false }
    let app = AXUIElementCreateApplication(pid)
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &value) == .success,
          let windows = value as? [AXUIElement]
    else { return false }
    // AX has no public window id, so match the window by its frame.
    let match = windows.first { window in
        var pos: CFTypeRef?, size: CFTypeRef?
        guard AXUIElementCopyAttributeValue(window, kAXPositionAttribute as CFString, &pos) == .success,
              AXUIElementCopyAttributeValue(window, kAXSizeAttribute as CFString, &size) == .success
        else { return false }
        var origin = CGPoint.zero, extent = CGSize.zero
        AXValueGetValue(pos as! AXValue, .cgPoint, &origin)
        AXValueGetValue(size as! AXValue, .cgSize, &extent)
        return abs(origin.x - frame.minX) <= 1 && abs(origin.y - frame.minY) <= 1
            && abs(extent.width - frame.width) <= 1 && abs(extent.height - frame.height) <= 1
    } ?? windows.first
    guard let window = match else { return false }
    AXUIElementSetAttributeValue(app, kAXFrontmostAttribute as CFString, kCFBooleanTrue)
    AXUIElementSetAttributeValue(window, kAXMainAttribute as CFString, kCFBooleanTrue)
    return AXUIElementPerformAction(window, kAXRaiseAction as CFString) == .success
}

/// Accessibility first, SkyLight as the fallback. Returns false when neither
/// was available.
func focusWindow(pid: pid_t, windowID: CGWindowID, frame: CGRect) -> Bool {
    if focusViaAccessibility(pid: pid, frame: frame) { return true }
    return focusViaSkyLight(pid: pid, windowID: windowID)
}

private func focusViaSkyLight(pid: pid_t, windowID: CGWindowID) -> Bool {
    guard let skyLight = dlopen("/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight", RTLD_LAZY),
          let setFrontSym = dlsym(skyLight, "_SLPSSetFrontProcessWithOptions"),
          let postSym = dlsym(skyLight, "SLPSPostEventRecordTo"),
          let lookupSym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "GetProcessForPID")
    else { return false }
    var psn = ProcessSerialNumber()
    guard unsafeBitCast(lookupSym, to: GetProcessForPIDFn.self)(pid, &psn) == noErr else { return false }
    let post = unsafeBitCast(postSym, to: PostEventRecordFn.self)
    var id = UInt32(windowID)
    _ = unsafeBitCast(setFrontSym, to: SetFrontProcessFn.self)(&psn, id, 0x200) // kCPSUserGenerated
    for phase: UInt8 in [0x01, 0x02] {
        var bytes = [UInt8](repeating: 0, count: 0xf8)
        bytes[0x04] = 0xf8
        bytes[0x08] = phase
        bytes[0x3a] = 0x10
        withUnsafeBytes(of: &id) { raw in
            for i in 0..<4 { bytes[0x3c + i] = raw[i] }
        }
        for i in 0x20..<0x30 { bytes[i] = 0xff }
        _ = post(&psn, &bytes)
    }
    return true
}
