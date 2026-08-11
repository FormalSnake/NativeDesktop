import AppKit

// cmux-parity-polish-2: `<paned>` is a plain draggable two-child NSSplitView,
// distinct from SplitView's NSSplitViewController-hosted item/sidebar-glass
// split (SplitController.swift). No item wrapping is needed — the split
// view's own two subviews ARE the panes — so PanedController is a bare
// NSObject (not a view controller) that just owns divider-position
// conversion and doubles as the NSSplitViewDelegate. The registry's strong
// reference retains the controller for process life, same intentional
// permanent-retain contract as `ndSplitControllers`.
nonisolated(unsafe) var ndPanedControllers: [ObjectIdentifier: PanedController] = [:]

/// Looks up the controller owning `split` (generated Paned apply/connect
/// arms resolve this before touching the divider or the node id).
func ndPanedController(for split: NSSplitView) -> PanedController? {
    ndPanedControllers[ObjectIdentifier(split)]
}

/// Generated ndConnectEvents Paned arm: records the node id the controller
/// emits onPositionChanged against (webview-idiom custom connect —
/// PanedController IS the delegate, so there is nothing else to wire).
func ndPanedConnect(_ view: NSView, nodeID: UInt32) {
    guard let split = view as? NSSplitView, let controller = ndPanedController(for: split) else { return }
    controller.nodeID = nodeID
}

/// Owns a `<paned>`'s divider: converts the wire's 0..1 `position` fraction
/// against the split's current size (create-time prop AND later React
/// updates), and is the `NSSplitViewDelegate` that turns a settled user drag
/// into `onPositionChanged`. GTK peer: cbPanedPositionChanged /
/// ndPanedEmitPosition / ndPanedApplyPosition in tools/codegen.ts's
/// ZIG_EXTRA block.
final class PanedController: NSObject, NSSplitViewDelegate {
    var nodeID: UInt32 = 0
    let split: NSSplitView
    private var suppressed = false
    private var pendingFraction: Double?
    // Last known fraction, programmatic or user-dragged; reapplyFraction
    // restores it after subview mutations discard the divider position.
    private var lastFraction: Double?
    private var debounce: DispatchWorkItem?
    private let minPaneExtent: CGFloat = 120

    init(split: NSSplitView) {
        self.split = split
        super.init()
        split.delegate = self
        // NSSplitView has no "first laid out" signal of its own (GTK peer:
        // the "map" signal, connected in the generated create arm) — frame
        // change is the closest AppKit equivalent, and posting isn't
        // guaranteed on for a plain, non-Auto-Layout-driven NSView.
        split.postsFrameChangedNotifications = true
        NotificationCenter.default.addObserver(self, selector: #selector(frameChanged), name: NSView.frameDidChangeNotification, object: split)
    }

    private var extent: CGFloat { split.isVertical ? split.bounds.width : split.bounds.height }

    /// React-driven `position` write (the create-time prop AND every later
    /// update). A brand-new split reports zero size before its first layout
    /// pass, so an early write is stashed and replayed by `frameChanged`
    /// once real geometry exists.
    func setPositionFraction(_ frac: Double) {
        lastFraction = frac
        guard extent > 0 else { pendingFraction = frac; return }
        suppressed = true
        split.setPosition(frac * extent, ofDividerAt: 0)
        suppressed = false
    }

    /// The generated structural arms call this after any pane add/remove:
    /// NSSplitView's adjustSubviews redistributes on subview mutation,
    /// silently discarding the divider position while the JS-side `position`
    /// value is unchanged (so no prop update arrives to restore it).
    /// Re-applies the last known fraction once both panes exist.
    func reapplyFraction() {
        guard lastFraction != nil, split.subviews.count == 2 else { return }
        // Deferred one runloop turn (capture-nothing idiom, see frameChanged):
        // the structural arms call this mid-commit, when a freshly added pane
        // (a nested split in particular) is still zero-sized — a synchronous
        // setPosition against that half-built state re-enters NSSplitView
        // layout and collapses BOTH panes to zero (measured). One turn later
        // the split has tiled its subviews and the divider write is clean.
        let key = ObjectIdentifier(split)
        DispatchQueue.main.async {
            MainActor.assumeIsolated { ndPanedControllers[key]?.reapplyFractionNow() }
        }
    }

    private func reapplyFractionNow() {
        guard let frac = lastFraction, split.subviews.count == 2 else { return }
        guard extent > 0 else { pendingFraction = frac; return }
        suppressed = true
        split.setPosition(frac * extent, ofDividerAt: 0)
        suppressed = false
    }

    /// Applies (and clears) the stashed pending fraction, if geometry exists.
    func applyPendingFraction() {
        guard let frac = pendingFraction, extent > 0 else { return }
        pendingFraction = nil
        setPositionFraction(frac)
    }

    @objc private func frameChanged() {
        guard pendingFraction != nil, extent > 0 else { return }
        // Deferred one runloop turn: a NESTED paned's first real frame
        // arrives from inside its ancestor split's adjustSubviews pass, and
        // a synchronous setPosition here re-enters NSSplitView layout —
        // measured: both ancestor panes collapse to width 0. By the next
        // turn the ancestor's pass has settled and the divider write is
        // safe. Same capture-nothing dispatch idiom as scheduleRebuild:
        // resolve the controller through the registry at fire time.
        let key = ObjectIdentifier(split)
        DispatchQueue.main.async {
            MainActor.assumeIsolated { ndPanedControllers[key]?.applyPendingFraction() }
        }
    }

    /// Fires continuously while dragging (and once per programmatic write,
    /// hence `suppressed`) — debounce 150ms so onPositionChanged reports the
    /// settled value, not one event per pixel of drag (GTK parity).
    func splitViewDidResizeSubviews(_ notification: Notification) {
        guard !suppressed, nodeID != 0 else { return }
        debounce?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.emitPosition() }
        debounce = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15, execute: work)
    }

    /// NSSplitView has no direct "position" readback (unlike GtkPaned) — the
    /// settled fraction is derived from the first pane's share of the total.
    private func emitPosition() {
        guard extent > 0, let first = split.subviews.first else { return }
        let frac = (split.isVertical ? first.frame.width : first.frame.height) / extent
        lastFraction = Double(frac)
        ndEmitEvent(nodeID, "positionChanged", "{\"position\":\(frac)}")
    }

    /// Keeps either pane from being dragged below minPaneExtent.
    func splitView(_ splitView: NSSplitView, constrainMinCoordinate proposedMinimumPosition: CGFloat, ofSubviewAt dividerIndex: Int) -> CGFloat {
        max(proposedMinimumPosition, minPaneExtent)
    }

    func splitView(_ splitView: NSSplitView, constrainMaxCoordinate proposedMaximumPosition: CGFloat, ofSubviewAt dividerIndex: Int) -> CGFloat {
        min(proposedMaximumPosition, extent - minPaneExtent)
    }

    /// Undoes `init`: drops the frame-change observer and any in-flight
    /// debounce so the split view isn't kept alive (nor `emitPosition` fired)
    /// after teardown. Called once, from `ndPanedTeardown`, never from
    /// `deinit` — the registry's strong reference means this instance never
    /// deinits on its own (see `ndPanedControllers`).
    func teardown() {
        NotificationCenter.default.removeObserver(self, name: NSView.frameDidChangeNotification, object: split)
        debounce?.cancel()
        debounce = nil
    }
}

/// Cross-cutting (mirrors `ndPopoverStructuralDetach`/`ndTrayItemStructuralDetach`):
/// called from `ndRemoveChild` and `vt.unparent` wherever a widget is
/// permanently detached, regardless of parent kind — a no-op unless `view` is
/// itself a `<paned>`'s outer split view (its two panes detaching is the
/// `parentKind == "Paned"` arm in `ndRemoveChild`, not this). Unlike the
/// one-per-window `ndSplitControllers`, `<paned>` is an unbounded per-split-
/// node primitive, so leaving `ndPanedControllers` unpruned leaks an
/// NSSplitView + PanedController + NotificationCenter observer on every
/// split/close.
func ndPanedTeardown(_ view: NSView) {
    guard let split = view as? NSSplitView,
          let controller = ndPanedControllers.removeValue(forKey: ObjectIdentifier(split)) else { return }
    controller.teardown()
}
