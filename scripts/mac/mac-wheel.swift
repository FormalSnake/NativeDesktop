// Posts a real trackpad scroll to one process. The automation `scroll` RPC
// drives a ScrollView's adjustments, which a web page does not have, so a
// wheel leg needs the event itself.
//
// `swift scripts/mac/mac-wheel.swift <pid> <globalX> <globalY> <dy> [steps] [momentum]`
// with the point in CoreGraphics global coordinates (top-left origin). The pid
// is only there to keep the call sites uniform; the window server routes the
// gesture by the cursor, which this warps onto the point and puts back after.
// A gesture is a began/changed/ended phase run, because Chromium latches
// scrolling on the phase fields rather than on the wheel delta; `momentum`
// appends the deceleration phases macOS sends after a fling.
import CoreGraphics
import Foundation

let args = CommandLine.arguments
guard args.count >= 5, let x = Double(args[2]), let y = Double(args[3]), let dy = Double(args[4])
else {
    FileHandle.standardError.write("usage: mac-wheel.swift <pid> <x> <y> <dy> [steps] [momentum]\n".data(using: .utf8)!)
    exit(2)
}
let steps = args.count > 5 ? (Int(args[5]) ?? 6) : 6
let momentum = args.count > 6 && args[6] == "momentum"

// CGEventTypes.h: the two phase fields have no Swift enum cases.
let scrollPhase = CGEventField(rawValue: 99)!
let momentumPhase = CGEventField(rawValue: 123)!

func post(delta: Double, phase: Int64, momentum: Int64) {
    guard let event = CGEvent(
        scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1,
        wheel1: Int32(delta.rounded()), wheel2: 0, wheel3: 0)
    else { return }
    event.location = CGPoint(x: x, y: y)
    event.setIntegerValueField(scrollPhase, value: phase)
    event.setIntegerValueField(momentumPhase, value: momentum)
    // The window server routes a scroll by the CURSOR, not by the event's own
    // location, and a scroll delivered straight to the pid never reaches
    // Chromium's compositor at all, so the pointer is warped onto the target
    // first and the event goes through the session tap.
    event.post(tap: .cghidEventTap)
}

// Its own process is never the pid's; a warped cursor is restored below.
let restore = CGEvent(source: nil)?.location
CGWarpMouseCursorPosition(CGPoint(x: x, y: y))
CGAssociateMouseAndMouseCursorPosition(1)
usleep(60_000)
defer {
    if let restore { CGWarpMouseCursorPosition(restore) }
}

// kCGScrollPhaseBegan 1, Changed 2, Ended 4; momentum Begin 1, Continue 2, End 3.
post(delta: 0, phase: 1, momentum: 0)
for _ in 0..<steps {
    post(delta: dy / Double(steps), phase: 2, momentum: 0)
    usleep(16_000)
}
post(delta: 0, phase: 4, momentum: 0)
if momentum {
    post(delta: 0, phase: 0, momentum: 1)
    for step in 0..<8 {
        post(delta: dy / Double(steps) / Double(step + 1), phase: 0, momentum: 2)
        usleep(16_000)
    }
    post(delta: 0, phase: 0, momentum: 3)
}
