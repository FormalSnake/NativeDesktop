import AppKit

// A popover panel for an anchor that sits so near its window's edge that an
// NSPopover centred on it would hang out of the window. NSPopover keeps its
// arrow on the anchor and slides its body only to stay on the screen, and it
// has no public way to slide the body along the window instead; this panel
// does, with the arrow still on the anchor's centre. It is used only when the
// centred NSPopover would not fit (Popovers.swift).

private let ndPanelArrowWidth: CGFloat = 22
private let ndPanelArrowHeight: CGFloat = 10
private let ndPanelRadius: CGFloat = 16

/// Borderless, so it has to say it can be key for the fields in it to type.
final class NDPopoverPanel: NSPanel {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }
}

/// The body with its arrow, as one outline in the panel's own coordinates.
private func ndPanelShape(body: NSRect, arrowX: CGFloat, edge: NSRectEdge) -> CGPath {
    let path = CGMutablePath()
    path.addRoundedRect(in: body, cornerWidth: ndPanelRadius, cornerHeight: ndPanelRadius)
    let half = ndPanelArrowWidth / 2
    let tri = CGMutablePath()
    switch edge {
    case .maxY: // panel below the anchor: arrow on the body's top edge
        tri.move(to: CGPoint(x: arrowX - half, y: body.maxY))
        tri.addLine(to: CGPoint(x: arrowX, y: body.maxY + ndPanelArrowHeight))
        tri.addLine(to: CGPoint(x: arrowX + half, y: body.maxY))
    default: // panel above the anchor: arrow on the body's bottom edge
        tri.move(to: CGPoint(x: arrowX - half, y: body.minY))
        tri.addLine(to: CGPoint(x: arrowX, y: body.minY - ndPanelArrowHeight))
        tri.addLine(to: CGPoint(x: arrowX + half, y: body.minY))
    }
    tri.closeSubpath()
    path.addPath(tri)
    return path
}

@MainActor
final class NDAnchoredPanel {
    private var panel: NDPopoverPanel?
    private var monitors: [Any] = []
    private var observers: [NSObjectProtocol] = []
    /// A dismissal the user made (a click outside, Escape, the app going to
    /// the background); a close asked for by the app does not call it.
    var onDismiss: (() -> Void)?

    var isShown: Bool { panel?.isVisible ?? false }
    var window: NSWindow? { panel }

    /// `anchor` is in `parent`'s screen space; `above` puts the panel over it.
    /// The body is centred on the anchor, then moved along the window's edge
    /// by as much as it takes to stay inside, and the arrow keeps pointing at
    /// the anchor's centre.
    func show(content: NSView, size: NSSize, anchor: NSRect, above: Bool, in parent: NSWindow) {
        close()
        let margin: CGFloat = 8
        let bounds = parent.convertToScreen(parent.contentLayoutRect)
        var bodyX = anchor.midX - size.width / 2
        bodyX = min(max(bodyX, bounds.minX + margin), bounds.maxX - margin - size.width)
        let bodyY = above ? anchor.maxY + ndPanelArrowHeight : anchor.minY - ndPanelArrowHeight - size.height
        let frame = NSRect(x: bodyX, y: above ? bodyY - ndPanelArrowHeight : bodyY,
                           width: size.width, height: size.height + ndPanelArrowHeight)
        let body = NSRect(x: 0, y: above ? ndPanelArrowHeight : 0, width: size.width, height: size.height)
        // Keep the arrow on the body's straight run, clear of its corners.
        let arrowX = min(max(anchor.midX - frame.minX, ndPanelRadius + ndPanelArrowWidth / 2),
                         size.width - ndPanelRadius - ndPanelArrowWidth / 2)

        let p = NDPopoverPanel(contentRect: frame, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        p.isOpaque = false
        p.backgroundColor = .clear
        p.hasShadow = true
        p.level = parent.level
        // A borderless panel takes the system's appearance, not the window's;
        // in a dark window under a light system the material and its labels
        // come out light grey on grey.
        p.appearance = parent.effectiveAppearance
        p.isReleasedWhenClosed = false
        let root = NSView(frame: NSRect(origin: .zero, size: frame.size))
        root.wantsLayer = true
        let outline = ndPanelShape(body: body, arrowX: arrowX, edge: above ? .minY : .maxY)
        let glass = NSGlassEffectView(frame: root.bounds)
        // Untinted glass lets a bright page straight through and the rows
        // lose their contrast; NSPopover's glass is dimmed about this much.
        glass.tintColor = NSColor.windowBackgroundColor.withAlphaComponent(0.7)
        let shape = CAShapeLayer()
        shape.path = outline
        glass.wantsLayer = true
        glass.layer?.mask = shape
        let chrome: NSView = glass
        chrome.autoresizingMask = [.width, .height]
        root.addSubview(chrome)
        content.removeFromSuperview()
        content.frame = body
        content.autoresizingMask = []
        content.translatesAutoresizingMaskIntoConstraints = true
        root.addSubview(content)
        p.contentView = root
        parent.addChildWindow(p, ordered: .above)
        p.makeKeyAndOrderFront(nil)
        // The shadow follows the drawn outline, which exists only once drawn.
        p.display()
        p.invalidateShadow()
        panel = p
        watch(p, parent)
    }

    private func dismissByUser() {
        guard isShown else { return }
        close()
        onDismiss?()
    }

    private func watch(_ p: NSWindow, _ parent: NSWindow) {
        monitors.append(NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .otherMouseDown]) { [weak self, weak p] event in
            if event.window !== p { MainActor.assumeIsolated { self?.dismissByUser() } }
            return event
        } as Any)
        monitors.append(NSEvent.addLocalMonitorForEvents(matching: [.keyDown]) { [weak self, weak p] event in
            guard event.keyCode == 53, event.window === p else { return event }
            MainActor.assumeIsolated { self?.dismissByUser() }
            return nil
        } as Any)
        let center = NotificationCenter.default
        observers.append(center.addObserver(forName: NSApplication.didResignActiveNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.dismissByUser() }
        })
        for name in [NSWindow.didMoveNotification, NSWindow.didResizeNotification, NSWindow.willCloseNotification] {
            observers.append(center.addObserver(forName: name, object: parent, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.dismissByUser() }
            })
        }
    }

    /// Takes the panel down; the content view goes with it and is re-homed by
    /// whoever shows it next.
    func close() {
        for m in monitors { NSEvent.removeMonitor(m) }
        monitors.removeAll()
        for o in observers { NotificationCenter.default.removeObserver(o) }
        observers.removeAll()
        guard let p = panel else { return }
        panel = nil
        p.parent?.removeChildWindow(p)
        p.orderOut(nil)
    }
}
