import AppKit
import CNd

// M11 Phase B: macOS uses the REAL native idiom (Notes.app/Mail) — a SINGLE
// unified NSToolbar spanning the top, with the sidebar reaching the very top
// of the window (traffic lights floating over it) via the Window create arm's
// .fullSizeContentView + titlebarAppearsTransparent. The per-pane <headerbar>s
// (two, or three for an M13 three-pane SplitView) do NOT each create their
// own toolbar: their items MERGE into the one window toolbar, separated by an
// NSTrackingSeparatorToolbarItem per divider (aligned to the split's dividers)
// — sidebar items sit left of divider 0, list items (if any) sit between
// divider 0 and 1, content items sit right of the last divider. The GTK peer
// stacks a real AdwHeaderBar inside each AdwToolbarView; on the Mac that
// split-per-pane header maps onto the one unified toolbar instead.

/// Host-rendered handle for a mounted `<headerbar>`. It never joins any view
/// hierarchy — it holds the header's start/end child views until its owning
/// `<toolbarview>` pane registers them into the window toolbar. `pane` is set
/// once this header is packed into its pane so a late child add can trigger a
/// coalesced toolbar rebuild.
final class NDHeaderBarView: NSView {
    var ndTitle: String = ""
    var startViews: [NSView] = []
    var endViews: [NSView] = []
    weak var pane: NDToolbarPaneView?
    /// The floating back/forward control (System Settings' leading `< >`),
    /// materialized by `ndHeaderBarApplyNav` when the app sets `canGoBack`/
    /// `canGoForward`. It's a synthesized toolbar item — not a declared child —
    /// so it's rendered as a LEADING item ahead of `startViews` (see
    /// `NDToolbarManager.defaultItemIdentifiers`).
    var navControl: NSSegmentedControl?
    /// This header's node id, recorded by `ndHeaderBarConnectNav` so the
    /// segmented control's action can emit `back`/`forward` back to the runtime.
    var ndNodeID: UInt32 = 0

    @objc func ndNavSegmentClicked(_ sender: NSSegmentedControl) {
        let seg = sender.selectedSegment
        guard seg == 0 || seg == 1, sender.isEnabled(forSegment: seg) else { return }
        let name = seg == 0 ? "back" : "forward"
        name.withCString { cName in
            "{}".withCString { cJson in
                nd_emit_event(gCtx, ndNodeID, cName, cJson)
            }
        }
    }
}

/// Host-rendered handle for a mounted `<toolbarview>` pane — a LOGICAL holder
/// that never enters the view hierarchy. Its non-header child is recorded in
/// `contentView`; the SplitView arm adds THAT box directly to its slot (and
/// vibrancy-wraps it for the sidebar), so it fills the slot natively. Its
/// `<headerbar>` child is recorded in `header` — those items feed the window
/// toolbar under this pane's `slot`, resolved once the pane hits the split.
final class NDToolbarPaneView: NSView {
    override var isFlipped: Bool { true }
    var header: NDHeaderBarView?
    var contentView: NSView?
    var slot: String = "content"
    weak var manager: NDToolbarManager?
}

// Monotonic source for per-child item identifiers ("nd-hb-<n>") and toolbar
// identifiers — single global counter, module scope, same `nonisolated(unsafe)`
// idiom as Backend.swift's `gridCells` (this process's UI code runs on one
// thread in practice).
nonisolated(unsafe) private var ndHeaderBarNextID = 0
private func ndHeaderBarFreshID() -> Int {
    ndHeaderBarNextID += 1
    return ndHeaderBarNextID
}

/// The window's single toolbar manager (set by the generated Window create arm
/// via `ndWindowToolbarManager`). Owns the one live `NSToolbar`, both pane
/// groups, and the tracking separator bound to the split's divider.
final class NDToolbarManager: NSObject, NSToolbarDelegate {
    private(set) var toolbar: NSToolbar
    private var idsByView: [ObjectIdentifier: NSToolbarItem.Identifier] = [:]

    // Pane header handles, once their panes have landed in the split. `list`
    // is the middle "folders / list / content" pane (M13 three-pane
    // SplitView) — nil for a two-pane tree, in which case the toolbar output
    // is byte-identical to the pre-M13 two-bucket behavior.
    private weak var sidebarHeader: NDHeaderBarView?
    private weak var listHeader: NDHeaderBarView?
    private weak var contentHeader: NDHeaderBarView?
    // The split the tracking separators align to. Set when the FIRST pane is
    // attached — all panes share one split.
    private weak var split: NSSplitView?
    // Divider 0 sits between sidebar and list-or-content; divider 1 (only
    // emitted when a `list` pane exists) sits between list and content.
    private let trackingSeparatorID0 = NSToolbarItem.Identifier("nd-toolbar-tracking-separator")
    private let trackingSeparatorID1 = NSToolbarItem.Identifier("nd-toolbar-tracking-separator-1")
    private var rebuildScheduled = false

    override init() {
        self.toolbar = NSToolbar(identifier: "nd-toolbar-\(ndHeaderBarFreshID())")
        super.init()
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = false
    }

    /// Records `header` under `slot` and captures the split for the tracking
    /// separator. Called once per pane, when the pane is appended to the split
    /// (its slot known). Panes can arrive in either order and either before or
    /// after their headers finish packing children, so a single deferred
    /// rebuild coalesces whatever state exists once the run loop settles.
    func register(header: NDHeaderBarView, slot: String, split: NSSplitView) {
        self.split = split
        if slot == "sidebar" {
            sidebarHeader = header
        } else if slot == "list" {
            listHeader = header
        } else {
            contentHeader = header
        }
        scheduleRebuild()
    }

    /// Drops a pane's header when its pane leaves the split.
    func unregister(header: NDHeaderBarView) {
        if sidebarHeader === header { sidebarHeader = nil }
        if listHeader === header { listHeader = nil }
        if contentHeader === header { contentHeader = nil }
        scheduleRebuild()
    }

    /// Coalesces bursts of register/pack calls within one commit into a single
    /// toolbar rebuild on the next main-queue turn — the panes, their headers,
    /// and the header children all land in the same commit but in an order we
    /// can't assume, so we rebuild once after they've all settled rather than
    /// racing a rebuild per structural op. Runs on the main thread already (the
    /// vtable contract), so `DispatchQueue.main.async` just defers, no hop.
    func scheduleRebuild() {
        if rebuildScheduled { return }
        rebuildScheduled = true
        // Same dispatch idiom as Backend.swift's marshal_async: capture nothing
        // non-Sendable — the sole manager is reachable via the nonisolated(unsafe)
        // `ndWindowToolbarManager` global, so no `self` crosses the isolation
        // boundary (which the Swift 6 concurrency checker rejects). By the next
        // main-queue turn both panes and their headers have settled.
        DispatchQueue.main.async {
            guard let mgr = ndWindowToolbarManager else { return }
            mgr.rebuildScheduled = false
            mgr.rebuild()
        }
    }

    private func identifier(for view: NSView) -> NSToolbarItem.Identifier {
        let key = ObjectIdentifier(view)
        if let existing = idsByView[key] { return existing }
        let id = NSToolbarItem.Identifier(rawValue: "nd-hb-\(ndHeaderBarFreshID())")
        idsByView[key] = id
        return id
    }

    /// Rebuilds the toolbar item list from scratch (fresh identifiers) rather
    /// than mutating a live list in place — simple and reliable for a full
    /// re-pack, and cheap for a handful of items (mirrors the M11 Phase A
    /// full-recreate rebuild pattern this replaces).
    func rebuild() {
        idsByView.removeAll()
        for idx in stride(from: toolbar.items.count - 1, through: 0, by: -1) {
            toolbar.removeItem(at: idx)
        }
        var pos = 0
        for id in defaultItemIdentifiers() {
            toolbar.insertItem(withItemIdentifier: id, at: pos)
            pos += 1
        }
    }

    private func defaultItemIdentifiers() -> [NSToolbarItem.Identifier] {
        var ids: [NSToolbarItem.Identifier] = []
        if let h = sidebarHeader {
            if let nav = h.navControl { ids.append(identifier(for: nav)) }
            ids += h.startViews.map { identifier(for: $0) }
            ids += h.endViews.map { identifier(for: $0) }
        }
        // Only insert a tracking separator when the split is present — it is
        // required to construct the item (it binds to the split's divider).
        // Divider 0 always appears (byte-identical to the pre-M13 two-pane
        // output); divider 1 only when a `list` pane exists (three-pane).
        if split != nil {
            ids.append(trackingSeparatorID0)
        }
        if let h = listHeader {
            if let nav = h.navControl { ids.append(identifier(for: nav)) }
            ids += h.startViews.map { identifier(for: $0) }
            ids += h.endViews.map { identifier(for: $0) }
            if split != nil {
                ids.append(trackingSeparatorID1)
            }
        }
        if let h = contentHeader {
            if let nav = h.navControl { ids.append(identifier(for: nav)) }
            ids += h.startViews.map { identifier(for: $0) }
            ids.append(.flexibleSpace)
            ids += h.endViews.map { identifier(for: $0) }
        }
        return ids
    }

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        defaultItemIdentifiers()
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        defaultItemIdentifiers()
    }

    func toolbar(_ toolbar: NSToolbar, itemForItemIdentifier itemIdentifier: NSToolbarItem.Identifier,
                 willBeInsertedIntoToolbar flag: Bool) -> NSToolbarItem? {
        if itemIdentifier == trackingSeparatorID0 {
            guard let split = split else { return nil }
            // dividerIndex 0: sidebar (arranged subview 0) vs. list-or-content.
            return NSTrackingSeparatorToolbarItem(identifier: itemIdentifier, splitView: split, dividerIndex: 0)
        }
        if itemIdentifier == trackingSeparatorID1 {
            guard let split = split else { return nil }
            // dividerIndex 1: list (arranged subview 1) vs. content — only
            // constructed when a `list` pane is registered.
            return NSTrackingSeparatorToolbarItem(identifier: itemIdentifier, splitView: split, dividerIndex: 1)
        }
        // Built as separate statements, not one chained `+`/`??` expression —
        // three optional-map terms combined that way pushed the type checker
        // over its time budget ("unable to type-check in reasonable time").
        var allViews: [NSView] = []
        if let h = sidebarHeader { allViews += h.startViews + h.endViews; if let n = h.navControl { allViews.append(n) } }
        if let h = listHeader { allViews += h.startViews + h.endViews; if let n = h.navControl { allViews.append(n) } }
        if let h = contentHeader { allViews += h.startViews + h.endViews; if let n = h.navControl { allViews.append(n) } }
        guard let view = allViews.first(where: { idsByView[ObjectIdentifier($0)] == itemIdentifier }) else {
            return nil
        }
        let item = NSToolbarItem(itemIdentifier: itemIdentifier)
        item.view = view
        return item
    }
}

/// Appends `child` to `bar`'s start/end slot arrays (generated HeaderBar
/// append/insertBefore arm). If the bar's pane is already registered with the
/// window toolbar, a coalesced rebuild picks the new item up.
func ndHeaderBarPack(_ bar: NDHeaderBarView, _ child: NSView, slot: String) {
    if slot == "end" {
        bar.endViews.append(child)
    } else {
        bar.startViews.append(child)
    }
    bar.pane?.manager?.scheduleRebuild()
}

/// Materializes / updates the header's floating back/forward control from the
/// `canGoBack`/`canGoForward` props (generated HeaderBar create + applyProps
/// arms). The control appears when either prop is PRESENT (the app opts in),
/// with each segment enabled per its flag — matching System Settings, where the
/// `< >` always show and grey out when navigation isn't available. When neither
/// prop is present it's torn down. A new control triggers a toolbar rebuild;
/// enabled-state-only changes update in place (the item is already installed).
func ndHeaderBarApplyNav(_ bar: NDHeaderBarView, canGoBack: Bool?, canGoForward: Bool?) {
    guard canGoBack != nil || canGoForward != nil else {
        if bar.navControl != nil {
            bar.navControl = nil
            bar.pane?.manager?.scheduleRebuild()
        }
        return
    }
    let seg: NSSegmentedControl
    if let existing = bar.navControl {
        seg = existing
    } else {
        seg = NSSegmentedControl()
        seg.segmentCount = 2
        seg.trackingMode = .momentary
        seg.segmentStyle = .separated
        seg.setImage(NSImage(systemSymbolName: "chevron.backward", accessibilityDescription: "Back"), forSegment: 0)
        seg.setImage(NSImage(systemSymbolName: "chevron.forward", accessibilityDescription: "Forward"), forSegment: 1)
        seg.target = bar
        seg.action = #selector(NDHeaderBarView.ndNavSegmentClicked(_:))
        bar.navControl = seg
        bar.pane?.manager?.scheduleRebuild()
    }
    seg.setEnabled(canGoBack ?? false, forSegment: 0)
    seg.setEnabled(canGoForward ?? false, forSegment: 1)
}

/// Records the header's node id so `ndNavSegmentClicked` can emit `back`/
/// `forward` (generated `ndConnectEvents` HeaderBar arm). The segmented
/// control's own target/action is wired in `ndHeaderBarApplyNav`; this only
/// supplies the id the action needs.
func ndHeaderBarConnectNav(_ view: NSView, nodeID: UInt32) {
    guard let bar = view as? NDHeaderBarView else { return }
    bar.ndNodeID = nodeID
}

/// Removes `child` from whichever slot array holds it (generated HeaderBar
/// remove arm) and requests a rebuild if the pane is registered.
func ndHeaderBarUnpack(_ bar: NDHeaderBarView, _ child: NSView) {
    bar.startViews.removeAll { $0 === child }
    bar.endViews.removeAll { $0 === child }
    bar.pane?.manager?.scheduleRebuild()
}

/// Records a `<toolbarview>` pane's child (generated ToolbarView append arm):
/// a `<headerbar>` becomes the pane's `header`; anything else becomes the
/// pane's `contentView`, pinned inside the pane so the split slot (and the
/// sidebar vibrancy wrapper) shows it.
func ndToolbarPanePack(_ pane: NDToolbarPaneView, _ child: NSView) {
    if let header = child as? NDHeaderBarView {
        pane.header = header
        header.pane = pane
    } else {
        // Logical only — the pane VIEW never enters the hierarchy. The SplitView
        // arm adds THIS content box (an NSStackView) directly to the split slot,
        // where it fills natively; parenting it inside the plain-NSView pane
        // instead would collapse it to its intrinsic size (the M11 Phase B bug).
        pane.contentView?.removeFromSuperview()
        pane.contentView = child
    }
}

/// Reverses `ndToolbarPanePack` (generated ToolbarView remove arm).
func ndToolbarPaneUnpack(_ pane: NDToolbarPaneView, _ child: NSView) {
    if let header = child as? NDHeaderBarView, pane.header === header {
        pane.header = nil
        header.pane = nil
        pane.manager?.unregister(header: header)
    } else if pane.contentView === child {
        child.removeFromSuperview()
        pane.contentView = nil
    }
}

/// Called when a pane lands in the split (generated SplitView append/
/// insertBefore arm): the pane's slot is now known, so its header's items can
/// be registered into the window toolbar on the correct side of the tracking
/// separator, and the split is handed to the manager for that separator.
func ndToolbarPaneAttachedToSplit(_ pane: NDToolbarPaneView, split: NSSplitView, slot: String) {
    guard let manager = ndWindowToolbarManager else { return }
    pane.slot = slot
    pane.manager = manager
    guard let header = pane.header else { return }
    manager.register(header: header, slot: slot, split: split)
}

/// Reverses `ndToolbarPaneAttachedToSplit` (generated SplitView remove arm).
func ndToolbarPaneDetachedFromSplit(_ pane: NDToolbarPaneView) {
    if let header = pane.header { pane.manager?.unregister(header: header) }
    pane.manager = nil
}
