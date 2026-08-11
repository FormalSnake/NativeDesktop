import AppKit

/// C4: `onHoverChanged` (Button/Box) — the AppKit peer of GTK's
/// `EventControllerMotion` enter/leave. `NSTrackingArea`'s owner just needs to
/// implement `mouseEntered`/`mouseExited`; this small object is that owner so
/// `ndHoverConnect` works on any `NSView` (an `NDButton` or a plain
/// `NSStackView`) without a dedicated subclass per widget.
private final class NDHoverTracker: NSObject {
    let nodeID: UInt32
    init(nodeID: UInt32) { self.nodeID = nodeID }

    func mouseEntered(with event: NSEvent) {
        ndEmitEvent(nodeID, "hoverChanged", "{\"checked\":true}")
    }
    func mouseExited(with event: NSEvent) {
        ndEmitEvent(nodeID, "hoverChanged", "{\"checked\":false}")
    }
}

/// Keeps each tracker alive (`NSTrackingArea` retains its owner, but the area
/// itself is only reachable through the view) — same accepted leak profile as
/// `ndNodeTypography`/`ndLayoutFlags` in Backend.swift, bounded by live widget count.
nonisolated(unsafe) private var ndHoverTrackers: [ObjectIdentifier: NDHoverTracker] = [:]

/// Generated `ndConnectEvents` hover arm (Button/Box `onHoverChanged`, C4).
/// `.inVisibleRect` keeps the tracked rect correct across resizes/reparenting
/// with no `updateTrackingAreas` override needed.
/// release_node purge seam (Backend.swift's `ndPurgeNodeRegistries`).
func ndHoverPurge(_ view: NSView) {
    ndHoverTrackers[ObjectIdentifier(view)] = nil
}

func ndHoverConnect(_ view: NSView, nodeID: UInt32) {
    let tracker = NDHoverTracker(nodeID: nodeID)
    ndHoverTrackers[ObjectIdentifier(view)] = tracker
    let area = NSTrackingArea(
        rect: .zero,
        options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
        owner: tracker,
        userInfo: nil
    )
    view.addTrackingArea(area)
}
