// Posts a real mouse click through the window server, for the cases the
// automation `pointer` RPC cannot reach. That RPC posts NSEvents into the
// app's own queue, which AppKit dispatches faithfully but Chromium's context
// menu never sees: the menu is raised from the window server's own right-click
// handling, so the event has to travel that way.
//
// `swift scripts/mac/mac-click.swift <globalX> <globalY> [left|right]`, with
// the point in CoreGraphics global coordinates (top-left origin). The cursor is
// warped onto the point and put back, because the window server routes a click
// by the cursor rather than by the event's location.
import CoreGraphics
import Foundation

let args = CommandLine.arguments
guard args.count >= 3, let x = Double(args[1]), let y = Double(args[2]) else {
    FileHandle.standardError.write("usage: mac-click.swift <x> <y> [left|right]\n".data(using: .utf8)!)
    exit(2)
}
let mode = args.count > 3 ? args[3] : "left"
let right = mode == "right"
let point = CGPoint(x: x, y: y)

let restore = CGEvent(source: nil)?.location
CGWarpMouseCursorPosition(point)
CGAssociateMouseAndMouseCursorPosition(1)
usleep(80_000)

let button: CGMouseButton = right ? .right : .left
let down: CGEventType = right ? .rightMouseDown : .leftMouseDown
let up: CGEventType = right ? .rightMouseUp : .leftMouseUp
if let move = CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: point, mouseButton: button) {
    move.post(tap: .cghidEventTap)
}
usleep(40_000)
for type in (mode == "move" ? [] : [down, up]) {
    guard let event = CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: point, mouseButton: button)
    else { continue }
    event.setIntegerValueField(.mouseEventClickState, value: 1)
    event.post(tap: .cghidEventTap)
    usleep(40_000)
}

// The menu is up now and tracking the cursor; putting the pointer back before
// it is dismissed would close it, so the restore is the caller's to ask for.
if args.contains("--restore"), let restore {
    CGWarpMouseCursorPosition(restore)
}
