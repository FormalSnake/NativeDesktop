import AppKit

/// Popover: the node's tracked handle is a host-only NSView (the
/// NDMenuNodeView idiom; it never enters the hierarchy); the real NSPopover
/// is lazy, `.transient`, and anchors on the node's TREE PARENT via the
/// cross-cutting structural guard (`ndPopoverStructuralAttach`), exactly
/// mirroring the GTK side's gtk_widget_set_parent guard. `open` is a
/// controlled bool: click-outside/Esc dismissal syncs back through
/// `popoverDidClose` -> `closed`; programmatic closes are flagged so they
/// don't echo (an NSPopover close is animated/async, which outlives a
/// withEchoSuppressed scope).
final class NDPopoverHandleView: NSView, NSPopoverDelegate {
    var nodeID: UInt32 = 0
    var position = "top"
    let contentContainer = FlippedView()
    private(set) weak var anchor: NSView?
    private var pendingOpen = false
    private var programmaticClose = false

    private lazy var popover: NSPopover = {
        let controller = NSViewController()
        controller.view = contentContainer
        let p = NSPopover()
        p.behavior = .transient
        p.contentViewController = controller
        p.delegate = self
        return p
    }()

    /// Cross-cutting append guard: the tree parent is the anchor.
    func attachToParent(_ parent: NSView) {
        anchor = parent
        if pendingOpen {
            pendingOpen = false
            applyOpen(true)
        }
    }

    func detachFromParent(_ parent: NSView) {
        if popover.isShown {
            programmaticClose = true
            popover.close()
        }
        if anchor === parent { anchor = nil }
    }

    func applyOpen(_ open: Bool) {
        if open {
            guard !popover.isShown else { return }
            guard let anchor, anchor.window != nil else {
                // Not anchored/realized yet (create-time open, or attach ran
                // before the window materialized): retry next turn once the
                // hierarchy exists, else keep the flag for attach to honor.
                pendingOpen = true
                if anchor != nil {
                    DispatchQueue.main.async { [weak self] in
                        guard let self, self.pendingOpen else { return }
                        self.pendingOpen = false
                        self.applyOpen(true)
                    }
                }
                return
            }
            let size = contentContainer.fittingSize
            popover.contentSize = NSSize(width: max(size.width, 60), height: max(size.height, 28))
            popover.show(relativeTo: anchor.bounds, of: anchor, preferredEdge: preferredEdge(for: anchor))
        } else {
            pendingOpen = false
            guard popover.isShown else { return }
            programmaticClose = true
            popover.performClose(nil)
        }
    }

    /// `position` names the side of the anchor the popover appears on;
    /// NSRectEdge is in the anchor's own coordinate space, so the vertical
    /// edges swap with the anchor's flippedness.
    private func preferredEdge(for anchor: NSView) -> NSRectEdge {
        switch position {
        case "bottom": return anchor.isFlipped ? .maxY : .minY
        case "left": return .minX
        case "right": return .maxX
        default: return anchor.isFlipped ? .minY : .maxY // "top"
        }
    }

    func popoverDidClose(_ notification: Notification) {
        if programmaticClose {
            programmaticClose = false
            return
        }
        // Genuine dismissal (click-outside / Esc): the app flips its
        // controlled `open` back to false on this event.
        ndEmitEvent(nodeID, "closed", "{}")
    }

    /// Single-child slot (generated structural Popover arms).
    func setChild(_ child: NSView) {
        contentContainer.subviews.forEach { $0.removeFromSuperview() }
        child.translatesAutoresizingMaskIntoConstraints = false
        contentContainer.addSubview(child)
        NSLayoutConstraint.activate([
            child.leadingAnchor.constraint(equalTo: contentContainer.leadingAnchor, constant: 12),
            child.trailingAnchor.constraint(equalTo: contentContainer.trailingAnchor, constant: -12),
            child.topAnchor.constraint(equalTo: contentContainer.topAnchor, constant: 12),
            child.bottomAnchor.constraint(equalTo: contentContainer.bottomAnchor, constant: -12),
        ])
    }

    func clearChild(_ child: NSView) {
        if child.superview === contentContainer { child.removeFromSuperview() }
    }
}

/// `ndCreate`'s Popover arm (generated) calls this.
func makePopover(_ props: [String: Any]) -> NSView {
    let handle = NDPopoverHandleView()
    handle.position = propStr(props, "position") ?? "top"
    if propBool(props, "open") ?? false {
        handle.applyOpen(true) // no anchor yet: recorded as pendingOpen
    }
    return handle
}

/// Generated ndApplyProps Popover.open arm.
func ndPopoverApplyOpen(_ view: NSView, _ open: Bool) {
    (view as? NDPopoverHandleView)?.applyOpen(open)
}

/// Generated ndApplyProps Popover.position arm.
func ndPopoverApplyPosition(_ view: NSView, _ position: String) {
    (view as? NDPopoverHandleView)?.position = position
}

/// Generated ndConnectEvents Popover arm.
func ndPopoverConnect(_ view: NSView, nodeID: UInt32) {
    (view as? NDPopoverHandleView)?.nodeID = nodeID
}

// MARK: - cross-cutting structural guards (generated ndAppendChild/
// ndInsertBefore/ndRemoveChild call these FIRST, whatever the parent kind —
// the AppKit peer of the GTK dispatcher's isA(child, gtk.Popover) guard).

func ndPopoverStructuralAttach(_ child: NSView, _ parent: NSView) -> Bool {
    guard let pop = child as? NDPopoverHandleView else { return false }
    pop.attachToParent(parent)
    return true
}

func ndPopoverStructuralDetach(_ child: NSView, _ parent: NSView) -> Bool {
    guard let pop = child as? NDPopoverHandleView else { return false }
    pop.detachFromParent(parent)
    return true
}

/// Generated structural Popover arms (the popover's OWN content child).
func ndPopoverSetChild(_ parent: NSView, _ child: NSView) {
    (parent as? NDPopoverHandleView)?.setChild(child)
}

func ndPopoverClearChild(_ parent: NSView, _ child: NSView) {
    (parent as? NDPopoverHandleView)?.clearChild(child)
}
