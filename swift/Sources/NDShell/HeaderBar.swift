import AppKit
import CNd

// macOS uses the real native idiom (Notes.app/Mail): a SINGLE
// unified NSToolbar spanning the top, with the sidebar reaching the very top
// of the window (traffic lights floating over it) via the Window create arm's
// .fullSizeContentView + titlebarAppearsTransparent. The per-pane <headerbar>s
// (two, or three for a three-pane SplitView) do NOT each create their
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
    /// GTK's AdwWindowTitle subtitle maps to NSWindow.subtitle here — the
    /// small line under the window title. Propagated by the toolbar manager
    /// once the owning window resolves (see `applySubtitles`).
    var ndSubtitle: String = ""
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
    /// The header's `title` rendered as a small toolbar label (System Settings'
    /// page title), positioned right of `navControl` (see
    /// `NDToolbarManager.defaultItemIdentifiers`). Lazily built from `ndTitle`,
    /// which is set once at create; an empty title is never shown. To change a
    /// create-only title an app remounts the header (`key=`), so a fresh header
    /// carries the new title.
    lazy var titleField: NSTextField = {
        let tf = NSTextField(labelWithString: ndTitle)
        tf.font = .systemFont(ofSize: NSFont.systemFontSize, weight: .semibold)
        tf.textColor = .labelColor
        tf.lineBreakMode = .byTruncatingTail
        return tf
    }()

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
    // slot="top"/"bottom" auxiliary bars (GTK: extra AdwToolbarView bars).
    // When any exist, `contentView` becomes an NDPaneAssemblyView stacking
    // topViews / mainContent / bottomViews; otherwise it stays mainContent
    // directly, byte-identical to the pre-slot behavior.
    var topViews: [NSView] = []
    var bottomViews: [NSView] = []
    var mainContent: NSView?
    private var assembly: NDPaneAssemblyView?

    func refreshAssembly() {
        if topViews.isEmpty && bottomViews.isEmpty {
            contentView = mainContent
            return
        }
        let host = assembly ?? {
            let a = NDPaneAssemblyView()
            assembly = a
            return a
        }()
        host.setParts(top: topViews, content: mainContent, bottom: bottomViews)
        contentView = host
    }
}

/// Vertical top-bars / content / bottom-bars scaffold for a `<toolbarview>`
/// pane with auxiliary bars. Plain constraints (not NSStackView) so the
/// content region fills both axes while the bars hug their fitting height.
final class NDPaneAssemblyView: NSView {
    override var isFlipped: Bool { true }

    func setParts(top: [NSView], content: NSView?, bottom: [NSView]) {
        subviews.forEach { $0.removeFromSuperview() }
        var previousBottom: NSLayoutYAxisAnchor = topAnchor
        for bar in top {
            addSubview(bar)
            bar.translatesAutoresizingMaskIntoConstraints = false
            NSLayoutConstraint.activate([
                bar.topAnchor.constraint(equalTo: previousBottom),
                bar.leadingAnchor.constraint(equalTo: leadingAnchor),
                bar.trailingAnchor.constraint(equalTo: trailingAnchor),
            ])
            previousBottom = bar.bottomAnchor
        }
        var previousTop: NSLayoutYAxisAnchor = bottomAnchor
        for bar in bottom.reversed() {
            addSubview(bar)
            bar.translatesAutoresizingMaskIntoConstraints = false
            NSLayoutConstraint.activate([
                bar.bottomAnchor.constraint(equalTo: previousTop),
                bar.leadingAnchor.constraint(equalTo: leadingAnchor),
                bar.trailingAnchor.constraint(equalTo: trailingAnchor),
            ])
            previousTop = bar.topAnchor
        }
        if let content {
            addSubview(content)
            content.translatesAutoresizingMaskIntoConstraints = false
            NSLayoutConstraint.activate([
                content.topAnchor.constraint(equalTo: previousBottom),
                content.bottomAnchor.constraint(equalTo: previousTop),
                content.leadingAnchor.constraint(equalTo: leadingAnchor),
                content.trailingAnchor.constraint(equalTo: trailingAnchor),
            ])
        }
    }
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

/// NSSearchField with a settable preferred intrinsic width. The generated
/// SearchInput create arm builds this subclass so a header-slotted field can
/// be stretched across the toolbar's free run by `updateSearchFieldWidths()`
/// — intrinsicContentSize is the only sizing channel the toolbar's private
/// item layout honors for custom views. Outside a toolbar (a search field in
/// pane content) `ndPreferredWidth` stays nil and this behaves exactly like
/// NSSearchField.
final class NDSearchField: NSSearchField {
    var ndPreferredWidth: CGFloat? {
        didSet { if ndPreferredWidth != oldValue { invalidateIntrinsicContentSize() } }
    }
    override var intrinsicContentSize: NSSize {
        var s = super.intrinsicContentSize
        if let w = ndPreferredWidth { s.width = w }
        return s
    }
}

/// Per-window `Window.toolbarStyle` (create-only), recorded by the generated
/// Window create arm and consumed by `attachForSidebar` when the unified
/// toolbar attaches. `unified` keeps today's transparent-titlebar treatment;
/// `unifiedCompact`/`expanded`/`preference` map straight onto
/// `NSWindow.ToolbarStyle` (a settings window finally gets labelled items).
nonisolated(unsafe) var ndWindowToolbarStyles: [ObjectIdentifier: String] = [:]

/// Buttons whose `prominent` prop is set: promoted toolbar items render
/// `.prominent` (accent-tinted glass); outside a toolbar the button keeps an
/// accent bezel (applied in `ndButtonApplyProminent`, respected by
/// Backend.swift's cssClasses baseline reset).
nonisolated(unsafe) var ndToolbarProminent: Set<ObjectIdentifier> = []

/// `Button.badge` values, keyed by button identity — consumed by the item
/// builder as a native `NSItemBadge` when the button is promoted into the
/// toolbar. Outside a toolbar the badge has no AppKit rendering (documented).
nonisolated(unsafe) var ndToolbarBadges: [ObjectIdentifier: String] = [:]

/// NDButtons currently promoted into the live toolbar as system-drawn items
/// (view discarded). Rebuilt from scratch on every toolbar rebuild.
/// Automation resolves visibility/bounds/clicks for these through the item
/// rather than the detached view (Automation.swift).
nonisolated(unsafe) var ndToolbarPromotedItems: [ObjectIdentifier: NSToolbarItem] = [:]

/// Per-promoted-button click target, keyed by BUTTON identity and kept for
/// the button's lifetime. Distinct object identity per button so (a) the
/// internal NSToolbarButton AppKit builds for a system-drawn item copies
/// this target and can be located by identity for automation bounds
/// (measured: the internal control's target/action mirror the item's), and
/// (b) clicks route back through the tracked NDButton as sender —
/// EventDispatcher resolves wiring by sender identity.
nonisolated(unsafe) var ndToolbarItemTargets: [ObjectIdentifier: NDToolbarItemTarget] = [:]

final class NDToolbarItemTarget: NSObject {
    weak var button: NDButton?
    init(button: NDButton) { self.button = button }
    @objc func fire(_ sender: Any?) {
        guard let b = button, let action = b.action else { return }
        // sendAction with the BUTTON as sender (not performClick: the
        // detached view has no window to draw its highlight in).
        _ = NSApp.sendAction(action, to: b.target, from: b)
    }
}

/// `Button.prominent` (generated Button create + applyProps arms).
func ndButtonApplyProminent(_ b: NSButton, _ prominent: Bool) {
    if prominent {
        ndToolbarProminent.insert(ObjectIdentifier(b))
    } else {
        ndToolbarProminent.remove(ObjectIdentifier(b))
    }
    b.bezelColor = prominent ? .controlAccentColor : nil
    if ndToolbarPromotedItems[ObjectIdentifier(b)] != nil { ndWindowToolbarManager?.scheduleRebuild() }
}

/// `Button.badge` (generated Button create + applyProps arms). Empty string
/// clears the badge.
func ndButtonApplyBadge(_ b: NSButton, _ badge: String) {
    ndToolbarBadges[ObjectIdentifier(b)] = badge.isEmpty ? nil : badge
    if ndToolbarPromotedItems[ObjectIdentifier(b)] != nil { ndWindowToolbarManager?.scheduleRebuild() }
}

/// `Button.size` -> `NSControl.controlSize` (generated Button arms).
func ndButtonApplySize(_ b: NSButton, _ size: String) {
    switch size {
    case "small": b.controlSize = .small
    case "large": b.controlSize = .large
    default: b.controlSize = .regular
    }
}

/// The window's single toolbar manager (set by the generated Window create arm
/// via `ndWindowToolbarManager`). Owns the one live `NSToolbar`, both pane
/// groups, and the tracking separator bound to the split's divider.
final class NDToolbarManager: NSObject, NSToolbarDelegate {
    private(set) var toolbar: NSToolbar
    private var idsByView: [ObjectIdentifier: NSToolbarItem.Identifier] = [:]
    /// Group identifier per run of consecutive promoted buttons, keyed by the
    /// run's LEAD view (stable across `defaultItemIdentifiers()` recomputes,
    /// which the delegate calls several times per rebuild).
    private var groupIDByLead: [ObjectIdentifier: NSToolbarItem.Identifier] = [:]
    /// Members of each `NSToolbarItemGroup` run, keyed by group identifier —
    /// rebuilt by `defaultItemIdentifiers()`, consumed by the item builder.
    private var groupMembers: [NSToolbarItem.Identifier: [NDButton]] = [:]

    // Pane header handles, once their panes have landed in the split. `list`
    // is the middle "folders / list / content" pane of a three-pane
    // SplitView — nil for a two-pane tree, which keeps the two-bucket
    // toolbar output unchanged.
    private weak var sidebarHeader: NDHeaderBarView?
    private weak var listHeader: NDHeaderBarView?
    private weak var contentHeader: NDHeaderBarView?
    // The inspector pane's header (#9): its items land at the very trailing
    // end, past the content header's end views. Deliberately NO tracking
    // separator for the inspector divider — HIG keeps the inspector's glass
    // continuous with the content region's toolbar.
    private weak var inspectorHeader: NDHeaderBarView?
    // The split the tracking separators align to. Set when the FIRST pane is
    // attached — all panes share one split.
    private weak var split: NSSplitView?
    // Divider 0 sits between sidebar and list-or-content; divider 1 (only
    // emitted when a `list` pane exists) sits between list and content.
    private let trackingSeparatorID0 = NSToolbarItem.Identifier("nd-toolbar-tracking-separator")
    private let trackingSeparatorID1 = NSToolbarItem.Identifier("nd-toolbar-tracking-separator-1")
    private var rebuildScheduled = false
    private var resizeObserver: NSObjectProtocol?
    /// Last computed search-field run (updateSearchFieldWidths) — the seed
    /// for freshly-inserted search items so the convergence rebuild lands
    /// them at the right width.
    private var lastSearchWidth: CGFloat?

    /// `autosaveName` is the Window's `frameAutosaveName`: when present it
    /// keys a STABLE toolbar identifier (the counter-based one changes every
    /// launch, which would orphan any saved configuration) and opts into
    /// user customization + configuration autosave (#1/#8).
    init(autosaveName: String? = nil) {
        if let name = autosaveName, !name.isEmpty {
            self.toolbar = NSToolbar(identifier: "nd-toolbar-\(name)")
            super.init()
            toolbar.allowsUserCustomization = true
            toolbar.autosavesConfiguration = true
        } else {
            self.toolbar = NSToolbar(identifier: "nd-toolbar-\(ndHeaderBarFreshID())")
            super.init()
            toolbar.allowsUserCustomization = false
        }
        toolbar.delegate = self
        // .default (not .iconOnly): unified styles show icons regardless, and
        // the expanded/preference styles (#8) are the ones that NEED labels
        // under their items.
        toolbar.displayMode = .default
        // Hidden until a pane registers items (rebuild() flips it): a VISIBLE
        // empty unified toolbar reserves full toolbar height in the window's
        // safe area, pushing plain (no-headerbar) apps' content down by a
        // blank ~52px strip instead of the standard titlebar.
        toolbar.isVisible = false
    }

    /// The window that OWNS this manager's headerbars — the one its toolbar
    /// attaches to (multi-window: NOT blindly the last-created `gWindow`, which
    /// may have moved on to a later window). Resolved from the split or a pane's
    /// content view once they've landed in a window; falls back to `gWindow`
    /// before attachment (correct for the first/only window).
    private func resolveOwnerWindow() -> NSWindow? {
        return split?.window
            ?? sidebarHeader?.pane?.contentView?.window
            ?? listHeader?.pane?.contentView?.window
            ?? contentHeader?.pane?.contentView?.window
            ?? inspectorHeader?.pane?.contentView?.window
            ?? gWindow
    }

    /// Records `header` under `slot` and captures the split for the tracking
    /// separator. Called once per pane, when the pane is appended to the split
    /// (its slot known). Panes can arrive in either order and either before or
    /// after their headers finish packing children, so a single deferred
    /// rebuild coalesces whatever state exists once the run loop settles.
    func register(header: NDHeaderBarView, slot: String, split: NSSplitView?) {
        // nil split = a <toolbarview> attached directly under <window> (no
        // panes to track) — tracking separators are already guarded on
        // `self.split` being present.
        if let split { self.split = split }
        // Lazy toolbar attachment: only headerbar apps get the unified
        // toolbar strip; plain apps keep the standard titlebar height in
        // their safe area. Same treatment a sidebar-only split gets from the
        // generated Window append arm via `attachForSidebar` directly.
        if let win = resolveOwnerWindow() {
            attachForSidebar(win: win, split: nil) // self.split already set above
        }
        if resizeObserver == nil, let win = resolveOwnerWindow() {
            // Same capture-nothing idiom as scheduleRebuild: resolve the
            // manager through the global at fire time.
            resizeObserver = NotificationCenter.default.addObserver(
                forName: NSWindow.didResizeNotification, object: win, queue: .main
            ) { _ in
                MainActor.assumeIsolated { ndWindowToolbarManager?.updateSearchFieldWidths() }
            }
        }
        if slot == "sidebar" {
            sidebarHeader = header
        } else if slot == "list" {
            listHeader = header
        } else if slot == "inspector" {
            inspectorHeader = header
        } else {
            contentHeader = header
        }
        scheduleRebuild()
    }

    /// The native Mac sidebar treatment (Notes/Mail idiom): a transparent,
    /// title-hidden titlebar plus this window's (possibly item-less) unified
    /// toolbar, so the sidebar's vibrancy reaches the very top under the
    /// traffic lights. `register()` reaches this same path once a
    /// `<headerbar>` pane lands in the split; the generated Window append
    /// arm calls it directly for a sidebar splitview with NO headerbar at
    /// all, so plain sidebar apps get the treatment too. A no-op past the
    /// first call for a given window (`win.toolbar !== toolbar` guards it) —
    /// toolbar visibility stays tied to item count (see `init`/`rebuild`), so
    /// an item-less attach here reserves no titlebar strip.
    func attachForSidebar(win: NSWindow, split: NSSplitView?) {
        if let split { self.split = split }
        guard win.toolbar !== toolbar else { return }
        win.toolbar = toolbar
        // Window.toolbarStyle (#8): the create-only prop recorded by the
        // generated Window create arm decides the chrome shape; `unified`
        // keeps today's treatment.
        let styleName = ndWindowToolbarStyles[ObjectIdentifier(win)] ?? "unified"
        switch styleName {
        case "unifiedCompact": win.toolbarStyle = .unifiedCompact
        case "expanded": win.toolbarStyle = .expanded
        case "preference": win.toolbarStyle = .preference
        default: win.toolbarStyle = .unified
        }
        // Hide the redundant window-title text — but only for the unified
        // styles: with a sidebar + unified toolbar the title would otherwise
        // sit at the content pane's LEADING edge and push toolbar items (the
        // back/forward nav) to the trailing edge. Native sidebar apps
        // (System Settings, Notes, Mail) hide it for exactly this reason.
        // expanded/preference windows are title-bearing document/settings
        // chrome and keep it. Purely visual; `win.title` is retained for
        // Mission Control / a11y either way.
        if styleName == "unified" || styleName == "unifiedCompact" {
            win.titleVisibility = .hidden
        }
    }

    /// Drops a pane's header when its pane leaves the split.
    func unregister(header: NDHeaderBarView) {
        if sidebarHeader === header { sidebarHeader = nil }
        if listHeader === header { listHeader = nil }
        if contentHeader === header { contentHeader = nil }
        if inspectorHeader === header { inspectorHeader = nil }
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
    /// re-pack, and cheap for a handful of items.
    func rebuild() {
        idsByView.removeAll()
        groupIDByLead.removeAll()
        groupMembers.removeAll()
        // Rebuilt from scratch below: only buttons the fresh item list still
        // promotes belong here (automation reads this for visibility/bounds).
        ndToolbarPromotedItems.removeAll()
        for idx in stride(from: toolbar.items.count - 1, through: 0, by: -1) {
            toolbar.removeItem(at: idx)
        }
        var pos = 0
        for id in defaultItemIdentifiers() {
            toolbar.insertItem(withItemIdentifier: id, at: pos)
            pos += 1
        }
        // Visible iff it has items (see init) — plain apps keep the standard
        // titlebar; headerbar apps get the unified toolbar strip.
        toolbar.isVisible = pos > 0
        applySubtitles()
        updateSearchFieldWidths()
    }

    /// HeaderBar.subtitle -> NSWindow.subtitle. The content header wins
    /// (GNOME homes the meaningful title there); side panes fall back in
    /// list/sidebar order. A non-empty subtitle re-shows the title area,
    /// which `attachForSidebar` hides by default — the app asked for text in
    /// the titlebar, so the sidebar-app default yields.
    func applySubtitles() {
        guard let win = resolveOwnerWindow() else { return }
        let subtitle = [contentHeader, listHeader, sidebarHeader]
            .compactMap { $0?.ndSubtitle }
            .first { !$0.isEmpty } ?? ""
        win.subtitle = subtitle
        if !subtitle.isEmpty { win.titleVisibility = .visible }
    }

    /// Stretches every header search field across the toolbar's free run
    /// (window width minus the other items' fitting widths and the
    /// traffic-light / margin region). Called after each rebuild and from the
    /// window-resize observer `register` installs — see the search branch of
    /// `toolbar(_:itemForItemIdentifier:...)` for why the width is owned here
    /// instead of negotiated with the toolbar's private layout.
    /// Width estimate for one toolbar item during the search-field free-run
    /// computation. Promoted (system-drawn) items have NO view to measure —
    /// the `?? 40` fallback of the old code would have become the common
    /// path — so image items use a fixed 38pt (the Tahoe item capsule) and
    /// text-only items measure their title.
    private func estimatedItemWidth(_ item: NSToolbarItem) -> CGFloat {
        if let group = item as? NSToolbarItemGroup {
            return group.subitems.reduce(0) { $0 + estimatedItemWidth($1) }
        }
        if let view = item.view { return view.fittingSize.width }
        if item.image == nil, !item.title.isEmpty {
            let w = (item.title as NSString).size(withAttributes: [.font: NSFont.systemFont(ofSize: NSFont.systemFontSize)]).width
            return w + 24
        }
        return 38
    }

    func updateSearchFieldWidths() {
        guard let win = resolveOwnerWindow() else { return }
        var searches: [NDSearchField] = []
        var others: CGFloat = 0
        for item in toolbar.items {
            if let s = item.view as? NDSearchField { searches.append(s); continue }
            if item.itemIdentifier == .flexibleSpace { continue }
            others += estimatedItemWidth(item) + 12 // + inter-item gap
        }
        guard !searches.isEmpty else { return }
        let lightsAndMargins: CGFloat = 116
        let free = max(160, (win.frame.width - lightsAndMargins - others) / CGFloat(searches.count))
        let changed = lastSearchWidth.map { abs($0 - free) > 1 } ?? true
        lastSearchWidth = free
        for s in searches { s.ndPreferredWidth = free }
        // NSSearchToolbarItem captures its width at INSERTION (later preferred/
        // intrinsic writes don't relayout a live item — measured) — when the
        // computed run differs from what the items were seeded with, one
        // coalesced re-insert lands them at the right width. Converges in a
        // single extra pass: same items ⇒ same computation ⇒ changed=false.
        if changed { scheduleRebuild() }
    }

    /// Whether a header child is promoted to a system-drawn toolbar item
    /// (#1). NDButton only — a ToggleButton (also NDButton-classed, marked
    /// `ndIsToggle`) keeps its view so its on/off state stays visible.
    private func ndPromotable(_ view: NSView) -> Bool {
        guard let b = view as? NDButton else { return false }
        return !b.ndIsToggle
    }

    /// Identifiers for one slot array, collapsing each run of 2+ consecutive
    /// promotable buttons into a single `NSToolbarItemGroup` (#1: related
    /// adjacent actions share one glass grouping). Group identifiers are
    /// cached by the run's lead view so repeated recomputes (the delegate
    /// calls this several times per rebuild) stay stable.
    private func identifiers(for views: [NSView]) -> [NSToolbarItem.Identifier] {
        var out: [NSToolbarItem.Identifier] = []
        var run: [NDButton] = []
        func flushRun() {
            guard !run.isEmpty else { return }
            if run.count == 1 {
                out.append(identifier(for: run[0]))
            } else {
                let lead = ObjectIdentifier(run[0])
                let id: NSToolbarItem.Identifier
                if let cached = groupIDByLead[lead] {
                    id = cached
                } else {
                    id = NSToolbarItem.Identifier(rawValue: "nd-hbg-\(ndHeaderBarFreshID())")
                    groupIDByLead[lead] = id
                }
                groupMembers[id] = run
                out.append(id)
            }
            run = []
        }
        for view in views {
            if let b = view as? NDButton, ndPromotable(b) {
                run.append(b)
            } else {
                flushRun()
                out.append(identifier(for: view))
            }
        }
        flushRun()
        return out
    }

    private func defaultItemIdentifiers() -> [NSToolbarItem.Identifier] {
        var ids: [NSToolbarItem.Identifier] = []
        if let h = sidebarHeader {
            if let nav = h.navControl { ids.append(identifier(for: nav)) }
            ids += identifiers(for: h.startViews)
            ids += identifiers(for: h.endViews)
        }
        // Only insert a tracking separator when the split is present — it is
        // required to construct the item (it binds to the split's divider).
        // Divider 0 always appears (same output as a two-pane split);
        // divider 1 only when a `list` pane exists (three-pane).
        if split != nil {
            ids.append(trackingSeparatorID0)
        }
        if let h = listHeader {
            if let nav = h.navControl { ids.append(identifier(for: nav)) }
            ids += identifiers(for: h.startViews)
            ids += identifiers(for: h.endViews)
            if split != nil {
                ids.append(trackingSeparatorID1)
            }
        }
        if let h = contentHeader {
            if let nav = h.navControl { ids.append(identifier(for: nav)) }
            // The content pane's page title sits right of the back/forward nav
            // (System Settings' small leading title), before any app start items.
            if !h.ndTitle.isEmpty { ids.append(identifier(for: h.titleField)) }
            ids += identifiers(for: h.startViews)
            ids.append(.flexibleSpace)
            ids += identifiers(for: h.endViews)
        }
        // Inspector items trail everything (#9) and get NO tracking separator:
        // the inspector's glass stays continuous with the content region.
        if let h = inspectorHeader {
            if let nav = h.navControl { ids.append(identifier(for: nav)) }
            ids += identifiers(for: h.startViews)
            ids += identifiers(for: h.endViews)
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
        // A group identifier resolves through groupMembers, not idsByView —
        // its member buttons never got per-view identifiers.
        if let members = groupMembers[itemIdentifier] {
            let group = NSToolbarItemGroup(itemIdentifier: itemIdentifier)
            group.subitems = members.enumerated().map { idx, b in
                promotedItem(for: b, identifier: NSToolbarItem.Identifier(rawValue: "\(itemIdentifier.rawValue)-s\(idx)"))
            }
            return group
        }
        // Built as separate statements, not one chained `+`/`??` expression —
        // three optional-map terms combined that way pushed the type checker
        // over its time budget ("unable to type-check in reasonable time").
        var allViews: [NSView] = []
        if let h = sidebarHeader { allViews += h.startViews + h.endViews; if let n = h.navControl { allViews.append(n) } }
        if let h = listHeader { allViews += h.startViews + h.endViews; if let n = h.navControl { allViews.append(n) } }
        if let h = contentHeader { allViews += h.startViews + h.endViews; if let n = h.navControl { allViews.append(n) }; if !h.ndTitle.isEmpty { allViews.append(h.titleField) } }
        if let h = inspectorHeader { allViews += h.startViews + h.endViews; if let n = h.navControl { allViews.append(n) } }
        guard let view = allViews.first(where: { idsByView[ObjectIdentifier($0)] == itemIdentifier }) else {
            return nil
        }
        if let search = view as? NDSearchField {
            // A search/address field absorbs all free toolbar width (browser
            // address bar, Min-style). The toolbar's item layout is private
            // and frame-based for custom views — NSSearchToolbarItem caps its
            // unfocused width, the deprecated min/maxSize path grew the
            // WINDOW, and Auto Layout on the item's root view (TAMIC=false)
            // removes it from layout entirely — so the field rides a plain
            // frame-sized wrapper whose width updateSearchFieldWidths() owns
            // (recomputed after each rebuild and on window resize).
            // Width: the toolbar sizes custom-view items from
            // intrinsicContentSize AT INSERTION (NSSearchField's own intrinsic
            // width is noIntrinsicMetric; NSSearchToolbarItem hard-caps at
            // ~320 regardless of preferredWidthForSearchField — both
            // measured), so seed NDSearchField's overridden intrinsic width
            // BEFORE the item lands; updateSearchFieldWidths() converges it to
            // the free run. Glass: isBordered opts the view item into the
            // system toolbar treatment (the Tahoe capsule) a bare view item
            // lacks.
            if search.ndPreferredWidth == nil { search.ndPreferredWidth = lastSearchWidth ?? 320 }
            let item = NSToolbarItem(itemIdentifier: itemIdentifier)
            item.view = search
            item.isBordered = true
            return item
        }
        // #1: an NDButton child becomes a SYSTEM-DRAWN item — image/action,
        // no custom view — so the Tahoe item glass is the only bezel (the
        // old NSButton-in-item shape drew its .rounded bezel INSIDE the
        // automatic glass), the item has a real label/paletteLabel for
        // overflow + VoiceOver, and `.prominent`/badge render natively.
        if let b = view as? NDButton, ndPromotable(b) {
            return promotedItem(for: b, identifier: itemIdentifier)
        }
        let item = NSToolbarItem(itemIdentifier: itemIdentifier)
        item.view = view
        // Non-interactive view items (the page-title label, a spinner, an
        // image) opt out of the system item treatment explicitly.
        if view is NSTextField || view is NSProgressIndicator || view is NSImageView {
            item.isBordered = false
        }
        return item
    }

    /// Builds the system-drawn toolbar item for a promoted header button.
    /// The NDButton stays the tracked model (events, automation, props);
    /// only its rendering moves to the item.
    private func promotedItem(for b: NDButton, identifier: NSToolbarItem.Identifier) -> NSToolbarItem {
        let item = NSToolbarItem(itemIdentifier: identifier)
        let label = !b.title.isEmpty ? b.title : (b.toolTip ?? b.image?.accessibilityDescription ?? "")
        if let image = b.image {
            item.image = image
        } else {
            // Text-only header action ("Save"): a bordered title item.
            item.title = b.title
        }
        item.label = label
        item.paletteLabel = label
        item.toolTip = b.toolTip
        item.isBordered = true
        let adapter: NDToolbarItemTarget
        if let existing = ndToolbarItemTargets[ObjectIdentifier(b)] {
            adapter = existing
        } else {
            adapter = NDToolbarItemTarget(button: b)
            ndToolbarItemTargets[ObjectIdentifier(b)] = adapter
        }
        item.target = adapter
        item.action = #selector(NDToolbarItemTarget.fire(_:))
        if ndToolbarProminent.contains(ObjectIdentifier(b)) {
            item.style = .prominent
            item.backgroundTintColor = .controlAccentColor
        }
        if let badge = ndToolbarBadges[ObjectIdentifier(b)] {
            item.badge = Int(badge).map { NSItemBadge.count($0) } ?? .text(badge)
        }
        ndToolbarPromotedItems[ObjectIdentifier(b)] = item
        return item
    }

    /// The window this manager's toolbar belongs to — Automation.swift needs
    /// it to convert a promoted item's frame into content-view space.
    func ownerWindow() -> NSWindow? { resolveOwnerWindow() }
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
    // A diffed `update` op only carries the props that CHANGED — absence here
    // means "untouched", never "torn down" (teardown-on-absent destroyed the
    // control on any unrelated HeaderBar update, e.g. a title change while a
    // page navigates). The control materializes on the first call carrying
    // either flag (normally the create op) and persists; each flag only
    // updates its own segment so a one-sided diff can't clobber the other.
    guard canGoBack != nil || canGoForward != nil else { return }
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
    if let b = canGoBack { seg.setEnabled(b, forSegment: 0) }
    if let f = canGoForward { seg.setEnabled(f, forSegment: 1) }
}

/// Records the header's node id so `ndNavSegmentClicked` can emit `back`/
/// `forward` (generated `ndConnectEvents` HeaderBar arm). The segmented
/// control's own target/action is wired in `ndHeaderBarApplyNav`; this only
/// supplies the id the action needs.
func ndHeaderBarConnectNav(_ view: NSView, nodeID: UInt32) {
    guard let bar = view as? NDHeaderBarView else { return }
    bar.ndNodeID = nodeID
}

/// Generated HeaderBar applyProps subtitle arm: record + propagate to the
/// owning window (through the manager when this header is registered; via
/// the global manager otherwise, which covers the pre-registration window).
func ndHeaderBarApplySubtitle(_ bar: NDHeaderBarView, _ subtitle: String) {
    bar.ndSubtitle = subtitle
    (bar.pane?.manager ?? ndWindowToolbarManager)?.applySubtitles()
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
func ndToolbarPanePack(_ pane: NDToolbarPaneView, _ child: NSView, slot: String) {
    if let header = child as? NDHeaderBarView {
        pane.header = header
        header.pane = pane
        // If the pane already landed in the split (manager set), a header added
        // AFTER that must register now — otherwise a `key`-driven remount (used
        // to change a create-only prop like `title`) would leave the new header
        // invisible, since ndToolbarPaneAttachedToSplit only fires for the
        // header present when the PANE first attaches. `split: nil` keeps the
        // manager's existing split reference.
        if let manager = pane.manager {
            manager.register(header: header, slot: pane.slot, split: nil)
        }
    } else if slot == "top" {
        pane.topViews.append(child)
        pane.refreshAssembly()
    } else if slot == "bottom" {
        pane.bottomViews.append(child)
        pane.refreshAssembly()
    } else {
        // Logical only — the pane VIEW never enters the hierarchy. The SplitView
        // arm adds THIS content box (an NSStackView) directly to the split slot,
        // where it fills natively; parenting it inside the plain-NSView pane
        // instead would collapse it to its intrinsic size.
        pane.mainContent?.removeFromSuperview()
        pane.mainContent = child
        pane.refreshAssembly()
    }
}

/// Reverses `ndToolbarPanePack` (generated ToolbarView remove arm).
func ndToolbarPaneUnpack(_ pane: NDToolbarPaneView, _ child: NSView) {
    if let header = child as? NDHeaderBarView, pane.header === header {
        pane.header = nil
        header.pane = nil
        pane.manager?.unregister(header: header)
    } else if pane.mainContent === child {
        child.removeFromSuperview()
        pane.mainContent = nil
        pane.refreshAssembly()
    } else if pane.topViews.contains(where: { $0 === child }) || pane.bottomViews.contains(where: { $0 === child }) {
        pane.topViews.removeAll { $0 === child }
        pane.bottomViews.removeAll { $0 === child }
        child.removeFromSuperview()
        pane.refreshAssembly()
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

/// Window-level peer of `ndToolbarPaneAttachedToSplit` (generated Window
/// append arm): a `<toolbarview>` directly under `<window>` feeds the SAME
/// unified toolbar, so its header items land in the titlebar with the
/// traffic lights inline — matching AdwToolbarView as the window content on
/// GTK. No split ⇒ no tracking separators; the header registers under the
/// content slot.
func ndToolbarPaneAttachedToWindow(_ pane: NDToolbarPaneView) {
    guard let manager = ndWindowToolbarManager else { return }
    pane.slot = "content"
    pane.manager = manager
    guard let header = pane.header else { return }
    manager.register(header: header, slot: "content", split: nil)
}
