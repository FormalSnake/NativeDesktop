import AppKit

// Widget-level drag and drop for the AppKit backend: the `draggable` /
// `dragPayload` / `dropTarget` props and the dragStarted / dragEnded /
// dragOver / dropped events. AppKit peer of src/gtk/dnd.zig.
//
// These are universal props, so this module is driven from ONE arm above the
// generated kind dispatch (tools/codegen.ts, UNIVERSAL_EVENTS) rather than
// from 69 per-widget templates. AppKit puts both halves on the view itself,
// which a universal arm cannot use directly: `NSDraggingSource` and
// `NSDraggingDestination` are protocols the view's own CLASS has to adopt,
// and the core hands us instances of NSButton, NSStackView, NSScrollView and
// friends. So each half gets a companion object instead:
//
//   drag side  — a pan recognizer on the host view, whose action starts a real
//                `beginDraggingSession(with:event:source:)` with a proxy as
//                the NSDraggingSource.
//   drop side  — a transparent NDDropZone subview pinned to the host's bounds,
//                which IS an NSDraggingDestination.
//
// The zone is invisible to the mouse (`hitTest` -> nil) except while a
// NativeDesktop drag is in flight, so a drop target never costs the host view
// a click. AppKit finds a dragging destination BY hit-testing, which is
// exactly why the flag has to flip before the session starts.
//
// Consequence of that gate: drags originating in another application do not
// reach these events. External file drags stay on `app.onFileDrop`, which is
// where they were already.

// ============================================================================
// Wire types
// ============================================================================

/// App-specific pasteboard type. `.string` rides along so a payload can also
/// be dropped on any text destination on the system.
private let ndDragPayloadType = NSPasteboard.PasteboardType("dev.nativedesktop.drag-payload")

/// Whether a drag started by this process is in flight. Read by every drop
/// zone's `hitTest`.
nonisolated(unsafe) private var ndDragInFlight = false

/// Point in the HOST view's own coordinate space, measured from its TOP-left
/// so both backends agree: GTK's drop coordinates are y-down, and an app
/// hit-testing a dock zone should not need to know which one it is on.
private func ndDragPoint(in host: NSView, windowPoint: NSPoint) -> NSPoint {
    let local = host.convert(windowPoint, from: nil)
    return host.isFlipped ? local : NSPoint(x: local.x, y: host.bounds.height - local.y)
}

private func ndEmitDragPoint(_ nodeID: UInt32, _ name: String, _ payload: String, _ point: NSPoint) {
    guard nodeID != 0 else { return }
    let json = "{\"text\":\"\(ndJsonEscape(payload))\",\"data\":{\"x\":\(point.x),\"y\":\(point.y)}}"
    ndEmitEvent(nodeID, name, json)
}

// ============================================================================
// Per-view state
// ============================================================================

private final class NDDragDropEntry {
    var nodeID: UInt32 = 0
    var payload = ""
    var source: NDDragSourceProxy?
    var recognizer: NSPanGestureRecognizer?
    var zone: NDDropZone?
}

nonisolated(unsafe) private var ndDragDropEntries: [ObjectIdentifier: NDDragDropEntry] = [:]

private func entry(for view: NSView) -> NDDragDropEntry {
    let key = ObjectIdentifier(view)
    if let existing = ndDragDropEntries[key] { return existing }
    let fresh = NDDragDropEntry()
    ndDragDropEntries[key] = fresh
    return fresh
}

/// release_node purge seam (Backend.swift's `ndPurgeNodeRegistries`). The
/// recognizer and the zone are subobjects of a view the core is releasing, so
/// they go with it — a recycled address must not inherit the dead node's
/// payload or keep reporting against its id.
func ndDragDropPurge(_ view: NSView) {
    guard let e = ndDragDropEntries[ObjectIdentifier(view)] else { return }
    if let r = e.recognizer { view.removeGestureRecognizer(r) }
    e.zone?.removeFromSuperview()
    ndDragDropEntries[ObjectIdentifier(view)] = nil
}

// ============================================================================
// Drag side
// ============================================================================

/// One per draggable view: the pan recognizer's target AND the session's
/// `NSDraggingSource`. Holds the host weakly — the entry table holds this.
/// `@unchecked Sendable` for the same reason the registries above are
/// `nonisolated(unsafe)`: every path in and out of this object is an AppKit
/// callback on the main thread, and the generated dispatcher that builds it
/// is nonisolated, so a real `@MainActor` annotation would only move the
/// isolation complaint to the call site.
private final class NDDragSourceProxy: NSObject, NSDraggingSource, @unchecked Sendable {
    weak var host: NSView?

    init(host: NSView) {
        self.host = host
        super.init()
    }

    private var current: NDDragDropEntry? {
        guard let host else { return nil }
        return ndDragDropEntries[ObjectIdentifier(host)]
    }

    /// A pan recognizer, not a `mouseDown` override: the host view is an
    /// arbitrary AppKit control we do not own. It only recognizes after the
    /// pointer actually moves, so a plain click still reaches the control.
    @MainActor @objc func handlePan(_ sender: NSPanGestureRecognizer) {
        guard sender.state == .began, let host, let e = current else { return }
        // The event driving the recognizer is the one the session needs; there
        // is no other way to reach it from a gesture action.
        guard let event = NSApp.currentEvent else { return }

        let item = NSPasteboardItem()
        item.setString(e.payload, forType: ndDragPayloadType)
        item.setString(e.payload, forType: .string)
        let draggingItem = NSDraggingItem(pasteboardWriter: item)
        draggingItem.setDraggingFrame(host.bounds, contents: ndDragImage(of: host))

        ndDragInFlight = true
        _ = host.beginDraggingSession(with: [draggingItem], event: event, source: self)
    }

    func draggingSession(
        _ session: NSDraggingSession,
        sourceOperationMaskFor context: NSDraggingContext
    ) -> NSDragOperation {
        [.copy, .move]
    }

    func draggingSession(_ session: NSDraggingSession, willBeginAt screenPoint: NSPoint) {
        guard let e = current, e.nodeID != 0 else { return }
        ndEmitEvent(e.nodeID, "dragStarted", "{\"text\":\"\(ndJsonEscape(e.payload))\"}")
    }

    func draggingSession(
        _ session: NSDraggingSession,
        endedAt screenPoint: NSPoint,
        operation: NSDragOperation
    ) {
        ndDragInFlight = false
        guard let e = current, e.nodeID != 0 else { return }
        ndEmitEvent(e.nodeID, "dragEnded", "{}")
    }
}

/// Snapshot of the view as the drag image. Without one the session drags
/// nothing visible, which reads as a broken gesture rather than a drag.
/// Via PDF rather than a cached bitmap rep: `bitmapImageRepForCachingDisplay`
/// hands back a rep that can still hold the last frame it cached, which is
/// the ghosting AutomationCapture.swift already had to work around.
@MainActor private func ndDragImage(of view: NSView) -> NSImage? {
    let bounds = view.bounds
    guard bounds.width >= 1, bounds.height >= 1 else { return nil }
    return NSImage(data: view.dataWithPDF(inside: bounds))
}

// ============================================================================
// Drop side
// ============================================================================

private final class NDDropZone: NSView {
    weak var host: NSView?

    private var entry: NDDragDropEntry? {
        guard let host else { return nil }
        return ndDragDropEntries[ObjectIdentifier(host)]
    }

    /// Transparent to the mouse unless a NativeDesktop drag is in flight.
    /// AppKit locates a dragging destination by hit-testing, so the zone has
    /// to be hittable exactly then and at no other time.
    override func hitTest(_ point: NSPoint) -> NSView? {
        ndDragInFlight ? super.hitTest(point) : nil
    }

    private func payload(from sender: any NSDraggingInfo) -> String {
        let board = sender.draggingPasteboard
        return board.string(forType: ndDragPayloadType) ?? board.string(forType: .string) ?? ""
    }

    private func report(_ name: String, _ sender: any NSDraggingInfo) {
        guard let host, let e = entry, e.nodeID != 0 else { return }
        ndEmitDragPoint(e.nodeID, name, payload(from: sender),
                        ndDragPoint(in: host, windowPoint: sender.draggingLocation))
    }

    override func draggingEntered(_ sender: any NSDraggingInfo) -> NSDragOperation {
        report("dragOver", sender)
        return .copy
    }

    override func draggingUpdated(_ sender: any NSDraggingInfo) -> NSDragOperation {
        report("dragOver", sender)
        return .copy
    }

    override func performDragOperation(_ sender: any NSDraggingInfo) -> Bool {
        report("dropped", sender)
        return true
    }
}

// ============================================================================
// Generated-dispatcher seam
// ============================================================================

/// Universal props arm, called from both `ndCreate` and `ndApplyProps`.
func ndDragDropApply(_ view: NSView, _ props: [String: Any]) {
    // Nothing is allocated for the overwhelming majority of nodes, which never
    // mention any of the three props.
    guard props["draggable"] != nil || props["dragPayload"] != nil || props["dropTarget"] != nil else { return }
    let e = entry(for: view)
    if let payload = propStr(props, "dragPayload") { e.payload = payload }

    if let on = propBool(props, "draggable") {
        if on, e.recognizer == nil {
            let proxy = NDDragSourceProxy(host: view)
            let recognizer = NSPanGestureRecognizer(target: proxy, action: #selector(NDDragSourceProxy.handlePan(_:)))
            view.addGestureRecognizer(recognizer)
            e.source = proxy
            e.recognizer = recognizer
        } else if !on, let recognizer = e.recognizer {
            view.removeGestureRecognizer(recognizer)
            e.recognizer = nil
            e.source = nil
        }
    }

    if let on = propBool(props, "dropTarget") {
        if on, e.zone == nil {
            let zone = NDDropZone(frame: view.bounds)
            zone.host = view
            zone.autoresizingMask = [.width, .height]
            zone.registerForDraggedTypes([ndDragPayloadType, .string])
            view.addSubview(zone, positioned: .above, relativeTo: nil)
            e.zone = zone
        } else if !on, let zone = e.zone {
            zone.removeFromSuperview()
            e.zone = nil
        }
    }
}

/// Universal connect arm. The props arm runs at create time, before the core
/// knows to call this, so the companions learn their node id here.
func ndDragDropConnect(_ view: NSView, nodeID: UInt32) {
    guard let e = ndDragDropEntries[ObjectIdentifier(view)] else { return }
    e.nodeID = nodeID
}
