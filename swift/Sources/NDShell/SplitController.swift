import AppKit

// Liquid Glass sidebar migration: the generated SplitView create arm
// (tools/codegen.ts) builds an `NSSplitViewController` instead of a bare
// `NSSplitView`. `NSSplitViewItem(sidebarWithViewController:)` is the idiom
// that gets the floating glass sidebar treatment on macOS 26 (and correct
// vibrancy pre-26) from one un-guarded code path (WWDC25 session 310,
// "Build an AppKit app with the new design"). The widget HANDLE that crosses
// the vtable stays `controller.splitView` (NOT `controller.view` — measured
// live, they are distinct objects; see `ndLiveContentView` below), so every
// generated arm's `parent as! NSSplitView`
// cast and the HeaderBar's `NSTrackingSeparatorToolbarItem` (bound to that
// same instance) keep working unchanged. The controller itself has no ABI
// handle of its own; it's retained here, keyed by its splitView's identity.
// The registry's strong reference intentionally retains the controller for
// process life (peer of `ndWindowToolbarManager`'s single-instance retain in
// HeaderBar.swift).
nonisolated(unsafe) var ndSplitControllers: [ObjectIdentifier: NDSplitViewController] = [:]

/// Looks up the controller owning `split` (generated SplitView append/
/// insertBefore/remove/apply arms resolve this before touching split view
/// items; precedent: `ndToolbarPaneAttachedToSplit`).
func ndSplitViewController(for split: NSSplitView) -> NDSplitViewController? {
    ndSplitControllers[ObjectIdentifier(split)]
}

/// The controller behind every `<splitview>`, and the one place
/// `sidebarWidth`/`listWidth` become real widths.
///
/// `NSSplitViewItem.preferredThicknessFraction` governs the
/// divider-double-click reset and the fullscreen-enter size (Apple's own doc
/// on the property), not the first layout: measured live, sidebarWidth 0.24
/// and 0.30 both came up at `minimumThickness`. What decides the landed width
/// is the divider position, so the fraction is taken of the split's width and
/// set as one.
///
/// The fraction stays the authority until a person drags a divider, and it is
/// re-landed on every width the window settles at, because a position is
/// absolute and a fraction is not. There is no single "final" width to wait
/// for: becoming `contentViewController` lays the split out at the
/// controller's fitting size, `ndInstallSplitAsWindowContent` restores the
/// frame after it, and an app that carries its window size in a store resizes
/// again on top of that. Landing once meant taking the fraction of whichever
/// width happened to be current, and the divider then held its POINTS through
/// every later resize (measured in a real app: a 0.24 sidebar came up at the
/// 180pt floor on a 1280pt window, because the fraction was spent while the
/// window was momentarily at its fitting size). Re-landing also makes the
/// prop mean the same thing it means on GTK, where
/// `AdwOverlaySplitView:sidebar-width-fraction` tracks the window for the
/// widget's whole life.
final class NDSplitViewController: NSSplitViewController {
    var pendingSidebarFraction: Double?
    var pendingListFraction: Double?

    /// Per pane, the split width its fraction was last landed against. A
    /// relayout at the same width is not a reason to move a divider; a window
    /// resize is. Tracked per pane so a collapsed one, which is skipped, still
    /// gets its fraction when it opens.
    private var appliedSidebarWidth: CGFloat = 0
    private var appliedListWidth: CGFloat = 0
    /// Set the first time a person drags a divider: their points win after
    /// that, and the declared fraction stops re-landing.
    private var dividerMoved = false
    /// Our own `setPosition` calls post the same notification a drag does, so
    /// they are bracketed rather than mistaken for one.
    private var landingFractions = false
    private var observingDrags = false

    /// `breakpoint` prop (px, 0 means off): below this content width the
    /// sidebar collapses regardless of `collapsed`; at or above it,
    /// `explicitCollapsed` (the app's own last `collapsed` value) is what's
    /// restored. This mirrors the GTK peer's `AdwBreakpoint`, which unapplies
    /// to the value it captured before it took over, not to a hardcoded
    /// `false`.
    var breakpointPx: CGFloat?
    /// The app's own `collapsed` value, kept separately from `isCollapsed` so
    /// the breakpoint has something to restore once the window widens back
    /// out.
    var explicitCollapsed = false
    private var breakpointActive = false

    override func viewDidLayout() {
        super.viewDidLayout()
        observeDividerDrags()
        applyFractions()
        applyBreakpoint()
    }

    /// A hysteresis band around `breakpointPx`, not a single crossing point,
    /// so a divider drag or a live window resize can't toggle the sidebar on
    /// every pixel at the boundary.
    private func applyBreakpoint() {
        guard let threshold = breakpointPx, splitView.window != nil else { return }
        guard let index = splitViewItems.firstIndex(where: { $0.behavior == .sidebar }) else { return }
        let width = splitView.bounds.width
        guard width > 1 else { return }
        let hysteresis: CGFloat = 8
        if width < threshold - hysteresis {
            breakpointActive = true
            splitViewItems[index].isCollapsed = true
        } else if width > threshold + hysteresis, breakpointActive {
            breakpointActive = false
            splitViewItems[index].isCollapsed = explicitCollapsed
        }
    }

    /// Generated SplitView append/insertBefore arms call this once the item
    /// has landed. Inserting an item into a live split does not run the
    /// controller through `viewDidLayout` (measured: no callback at all), so a
    /// sidebar mounted after the window is up would hold its fraction pending
    /// forever and come up at `minimumThickness`. A pane that just mounted has
    /// no width of its own whatever the user did to the last one, so this is
    /// the one path that lands a fraction past `dividerMoved`.
    func splitViewItemsChanged() {
        splitView.layoutSubtreeIfNeeded()
        appliedSidebarWidth = 0
        appliedListWidth = 0
        applyFractions(force: true)
    }

    private func observeDividerDrags() {
        guard !observingDrags else { return }
        observingDrags = true
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(splitViewWillResize(_:)),
            name: NSSplitView.willResizeSubviewsNotification,
            object: splitView,
        )
    }

    /// The moment the app's declared fraction stops being the authority.
    ///
    /// The notification's `NSSplitViewDividerIndex` is not the test on its
    /// own: AppKit sets it for its OWN divider adjustments during layout too
    /// (measured, on a split nobody had touched), and treating that as a drag
    /// froze the fraction before it had ever landed. A drag runs inside
    /// AppKit's mouse-tracking loop, so the deciding fact is whether a mouse
    /// is down in this application right now.
    @objc private func splitViewWillResize(_ note: Notification) {
        guard !landingFractions, note.userInfo?["NSSplitViewDividerIndex"] != nil else { return }
        switch NSApp.currentEvent?.type {
        case .leftMouseDown, .leftMouseDragged: dividerMoved = true
        default: break
        }
    }

    /// Divider positions are cumulative from the leading edge, so the list's
    /// position carries the panes before it; the sidebar is landed first for
    /// that reason. A collapsed pane is skipped rather than spent: setting a
    /// position would open the pane the app asked to keep shut.
    ///
    /// Every width the split settles at gets the fraction, including the ones
    /// the window is only passing through (the controller's fitting size, the
    /// restored frame, a size the app applies from a store). Waiting for the
    /// "real" one instead needs a way to know which that is, and there is
    /// none: a guard that required the split's width to equal its window's
    /// content width simply never matched on a window whose fitting size had
    /// pushed it wider than its own frame, and the sidebar stayed at its
    /// floor. Landing on each of them costs one divider move per resize and
    /// is right at rest, which is what an app and a screenshot both see.
    private func applyFractions(force: Bool = false) {
        guard force || !dividerMoved else { return }
        guard pendingSidebarFraction != nil || pendingListFraction != nil else { return }
        let total = splitView.bounds.width
        guard total > 1, splitView.window != nil else { return }
        landingFractions = true
        defer { landingFractions = false }
        if let fraction = pendingSidebarFraction, abs(total - appliedSidebarWidth) > 0.5,
           let index = splitViewItems.firstIndex(where: { $0.behavior == .sidebar }),
           !splitViewItems[index].isCollapsed {
            appliedSidebarWidth = total
            splitView.setPosition((total * fraction).rounded(), ofDividerAt: index)
        }
        if let fraction = pendingListFraction, abs(total - appliedListWidth) > 0.5,
           let index = splitViewItems.firstIndex(where: { $0.behavior == .contentList }),
           !splitViewItems[index].isCollapsed {
            appliedListWidth = total
            // After the sidebar moved, so the panes before this divider are
            // measured where they actually are.
            splitView.layoutSubtreeIfNeeded()
            let leading = splitViewItems[..<index].reduce(0.0) { $0 + $1.viewController.view.frame.width }
            splitView.setPosition(leading + (total * fraction).rounded(), ofDividerAt: index)
        }
    }
}

/// Resolves the window's LIVE root content view. Once a SplitView becomes
/// `contentViewController` (generated Window append arm), the Window's
/// originally-tracked handle (the create arm's `FlippedView`) is orphaned,
/// so `get_window`/the crash overlay/automation bounds conversion/snapshot
/// must all resolve to whatever's actually installed instead. But
/// `NSSplitViewController.view` is NOT documented to always be the same
/// object as its `splitView` (Apple's own caveat on that property), and only
/// `splitView` is verified flipped (top-left, y-down) here, which is the
/// coordinate contract every content root must uphold (`FlippedView`'s doc
/// comment in NDGen/Widgets.swift; Automation.swift's node_bounds/snapshot
/// rely on it). Measured live: resolving to `.view` instead inverted
/// getTree's y-order (a pinned/topmost row reported the LARGEST y, not the
/// smallest). `.view` is a distinct, non-flipped wrapper for
/// `NSSplitViewController` in practice, so `.splitView` is preferred
/// whenever the installed controller is one.
func ndLiveContentView() -> NSView? {
    if let split = gWindow?.contentViewController as? NSSplitViewController {
        return split.splitView
    }
    return gWindow?.contentViewController?.view ?? gWindow?.contentView
}

/// The NSWindow a Window node's handle belongs to — the per-window peer of the
/// `gWindow` global. A freshly-created Window's handle (its content FlippedView)
/// is still `contentView`, so `.window` resolves directly; once a SplitView
/// takes over as contentViewController that view is orphaned (`.window` == nil),
/// so fall back to the create-time registry (`ndContentToWindow`, keyed by the
/// handle's identity, populated by `gWindow`'s observer in main.swift) and
/// finally the single-window `gWindow`.
func ndWindow(for handle: NSView) -> NSWindow? {
    return handle.window ?? ndContentToWindow[ObjectIdentifier(handle)] ?? gWindow
}

/// The CURRENT live content view of a specific `NSWindow` (the split's
/// `splitView` once one took over, else the content VC's view / contentView) —
/// the window-keyed core of `ndLiveContentView()` and `ndLiveContentView(for:)`.
/// Automation's `node_bounds` resolves each widget's OWN window through this so a
/// widget living in window B reports bounds in window B's space, not the single
/// global window's.
func ndLiveContentView(ofWindow win: NSWindow?) -> NSView? {
    guard let win else { return nil }
    if let split = win.contentViewController as? NSSplitViewController {
        return split.splitView
    }
    return win.contentViewController?.view ?? win.contentView
}

/// Per-window peer of `ndLiveContentView()` for the multi-window reconstruction
/// path (src/tree.zig's `resolveWindow` -> Backend.swift's `resolve_window`):
/// resolves a Window node's handle to the CURRENT live content of the window it
/// belongs to (which differs from the handle once a SplitView took over — same
/// rationale as `ndLiveContentView`).
func ndLiveContentView(for handle: NSView) -> NSView? {
    return ndLiveContentView(ofWindow: ndWindow(for: handle))
}

/// Installs a registered SplitView as `win`'s contentViewController — the one
/// hosting path that earns the automatic Liquid Glass sidebar treatment on
/// macOS 26 (M11 Phase C). Called by the generated Window append arm for a
/// direct `<splitview>` child AND for one wrapped in a `<toastoverlay>`
/// (Toasts.swift's `ndEmbeddedSplit`): parenting the bare splitView as a
/// plain subview instead leaves every pane's content hanging off the
/// controller's never-installed wrapper view — measured live, the whole split
/// subtree reports visible=false and renders nothing.
///
/// Assigning contentViewController resizes the window to the controller
/// view's fitting size (measured 900x600 -> 500x500), clobbering Window
/// defaultWidth/Height. Save the frame first and reassert it right after
/// (display:true) — the split view's fitting size is only a floor, so the
/// reasserted 1100x700 sticks with no lingering size constraint.
///
/// Do NOT steer the initial size via `controller.preferredContentSize`
/// instead: a non-zero preferred size makes AppKit install fixed
/// width/height constraints on the controller's view
/// (`NSViewController.preferredContentSize.{width,height}` @ priority 501)
/// that AppKit re-syncs on every layout pass. Those pin the window to the
/// windowed size through a fullscreen transition — the window reports
/// .fullScreen but its frame never grows to the screen, leaving a small
/// app box in a black fullscreen space. Zeroing preferredContentSize does
/// not drop the constraints, and removing them by hand only holds until
/// the next layout pass re-derives them from the still-non-zero property,
/// so the plain save/reassert-frame path is the one that survives
/// fullscreen.
func ndInstallSplitAsWindowContent(_ split: NSSplitView, _ controller: NSSplitViewController, _ win: NSWindow) {
    let ndSavedFrame = win.frame
    win.contentViewController = controller
    win.setFrame(ndSavedFrame, display: true)
    // A sidebar-behavior split item earns the full-height sidebar
    // treatment even with no <headerbar> anywhere in the tree — the
    // window-toolbar attach that used to fire only from a registering
    // headerbar (NDToolbarManager.register) now fires directly here too,
    // via the same helper, so the sidebar's vibrancy reaches the very top
    // under the traffic lights (Notes/Mail idiom) for plain sidebar apps.
    if controller.splitViewItems.contains(where: { $0.behavior == .sidebar }) {
        ndWindowToolbarManager?.attachForSidebar(win: win, split: split)
    }
}

/// Minimum height for a `<toolbarview slot="top"/"bottom">` bar hosted as a
/// split item accessory. A bar whose content hugs to nothing (an empty
/// `<box>`, or one whose only child has not mounted yet) would otherwise
/// collapse the accessory to zero height and take the fade with it.
private let ndPaneAccessoryMinHeight: CGFloat = 28

/// A `<toolbarview slot="top">` / `slot="bottom"` bar hosted as a real split
/// item accessory. The scroll edge effect is applied automatically underneath
/// toolbar items, titlebar accessories and split item accessories, and is not
/// settable on a scroll view (WWDC25 310), so a bar stacked as a plain
/// subview (`NDPaneAssemblyView`) gets no fade at all, which is why nothing
/// in the tree faded before. `slot="top"` already means what a top-aligned
/// accessory means, so this is a backend upgrade on an unchanged app tree.
final class NDPaneAccessoryViewController: NSSplitViewItemAccessoryViewController {
    let bar: NSView

    init(bar: NSView) {
        self.bar = bar
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) {
        fatalError("NDPaneAccessoryViewController is built in code, never unarchived")
    }

    override func loadView() {
        let host = NDPaneHostView()
        bar.translatesAutoresizingMaskIntoConstraints = false
        host.addSubview(bar)
        NSLayoutConstraint.activate([
            bar.leadingAnchor.constraint(equalTo: host.leadingAnchor),
            bar.trailingAnchor.constraint(equalTo: host.trailingAnchor),
            bar.topAnchor.constraint(equalTo: host.topAnchor),
            bar.bottomAnchor.constraint(equalTo: host.bottomAnchor),
            host.heightAnchor.constraint(greaterThanOrEqualToConstant: ndPaneAccessoryMinHeight),
        ])
        view = host
        automaticallyAppliesContentInsets = true
        // macOS 26.1 only, and deliberately without an else branch: omitting
        // it leaves the system automatic style, which still fades.
        // NSScrollEdgeEffectStyle is NS_SWIFT_UI_ACTOR, hence assumeIsolated
        // (every vtable call already arrives on the UI thread).
        if #available(macOS 26.1, *) {
            MainActor.assumeIsolated { preferredScrollEdgeEffectStyle = .soft }
        }
    }
}

private func ndAccessoriesMatch(_ controllers: [NSSplitViewItemAccessoryViewController], _ bars: [NSView]) -> Bool {
    guard controllers.count == bars.count else { return false }
    for (index, controller) in controllers.enumerated() {
        guard (controller as? NDPaneAccessoryViewController)?.bar === bars[index] else { return false }
    }
    return true
}

/// Reconciles `item`'s accessories against a `<toolbarview>` pane's
/// `slot="top"`/`slot="bottom"` bars. Compared by bar identity first, so a
/// React update that touches neither leaves the live accessories (and their
/// animatable hidden state) alone; a changed edge is replaced through the
/// array property, whose own contract detaches the controllers it drops.
func ndSyncPaneAccessories(_ item: NSSplitViewItem, top: [NSView], bottom: [NSView]) {
    if !ndAccessoriesMatch(item.topAlignedAccessoryViewControllers, top) {
        item.topAlignedAccessoryViewControllers = top.map { NDPaneAccessoryViewController(bar: $0) }
    }
    if !ndAccessoriesMatch(item.bottomAlignedAccessoryViewControllers, bottom) {
        item.bottomAlignedAccessoryViewControllers = bottom.map { NDPaneAccessoryViewController(bar: $0) }
    }
}

/// The view controller behind one SplitView pane. It exists so a
/// `<toolbarview>` pane's auxiliary bars can become split item accessories:
/// the generated append arm builds the `NSSplitViewItem` AFTER this
/// controller, and AppKit has no `didMove(toParent:)` hook, so the first
/// attach rides `viewWillAppear` (by which point the item is in the
/// controller) and every later one rides `NDToolbarPaneView.refreshAssembly`.
final class NDPaneViewController: NSViewController {
    weak var pane: NDToolbarPaneView?

    override func viewWillAppear() {
        super.viewWillAppear()
        syncAccessories()
    }

    /// A no-op until this controller's split item exists, and for a pane
    /// mounted directly under `<window>`: with no item to attach to, that
    /// one keeps the `NDPaneAssemblyView` stack.
    func syncAccessories() {
        guard let pane, pane.usesSplitAccessories,
              let controller = parent as? NSSplitViewController,
              let item = controller.splitViewItems.first(where: { $0.viewController === self }) else { return }
        ndSyncPaneAccessories(item, top: pane.topViews, bottom: pane.bottomViews)
    }
}

/// A view that scrolls its own content: an NSScrollView, or a Box whose sole
/// arranged subview is one. Such a root may run edge to edge under the
/// floating glass chrome — the scroll view insets its content via the safe
/// area itself and AppKit draws the scroll edge effect over it. Also read by
/// the generated Window append arm (`ndWindowRootInset`).
func ndIsScrollShaped(_ view: NSView) -> Bool {
    if view is NSScrollView { return true }
    if let box = view as? NDBoxView, box.ndChildren.count == 1,
       box.ndChildren[0] is NSScrollView {
        return true
    }
    return false
}

/// A view built AROUND something that scrolls: a vertical Box whose single
/// expanding child is a scroll view, with siblings (a search field above, a
/// footer caption below) that do not scroll. The list is the pane's subject,
/// but the stack around it has to stay inside the safe area, since those
/// siblings have no content-inset channel of their own. Read by the generated
/// Window append arm (`ndWindowRootInset`) as well.
func ndIsListShaped(_ view: NSView) -> Bool {
    guard let box = view as? NDBoxView, box.ndOrientation == .vertical else { return false }
    // The vexpanding child (Layout.swift's ndLayoutFlags) is the one that
    // absorbs the leftover height, so it is the one that reaches the edges: a
    // list pane with a search box above it still reads as scrolling even
    // though the box around it does not.
    let expanding = box.ndChildren.filter { ndLayoutFlags[ObjectIdentifier($0)]?.vexpand == true }
    return expanding.count == 1 && expanding[0] is NSScrollView
}

/// How a pane's content root relates to the floating chrome around it.
private enum NDPaneContentShape {
    /// `ndIsScrollShaped`: the root can run edge to edge and scroll under
    /// the chrome.
    case scrolling
    /// `ndIsListShaped`: safe-area pinned on every edge, no edge-to-edge run.
    case scrollingWithSiblings
    case plain
}

private func ndPaneContentShape(_ view: NSView) -> NDPaneContentShape {
    if ndIsScrollShaped(view) { return .scrolling }
    return ndIsListShaped(view) ? .scrollingWithSiblings : .plain
}

/// What `ndInstallPaneContent` landed for a pane content root, so the shape can
/// be re-derived later against the same host.
private struct NDPaneInstall {
    weak var host: NSView?
    var shape: NDPaneContentShape
    var bottom: NSLayoutConstraint?
}

nonisolated(unsafe) private var ndPaneInstalls: [ObjectIdentifier: NDPaneInstall] = [:]

/// release_node purge seam (Backend.swift's `ndPurgeNodeRegistries`).
func ndPaneInstallPurge(_ view: NSView) {
    ndPaneInstalls[ObjectIdentifier(view)] = nil
}

/// Re-derives a pane content root's shape and updates the pane in place.
///
/// `ndIsListShaped` is answered from the box's children and their `vexpand`
/// flags, and both keep moving after the pane is installed: a footer caption
/// mounts, a list gains its expand flag through a later `ndApplyStyle`. Reached
/// from `ndInvalidateBoxChain` (Layout.swift), which is already called for
/// every one of those events, so this runs inside the same synchronous commit
/// that changed the tree.
///
/// Only the safe-area pair is re-derived. `.plain` and `.scrollingWithSiblings`
/// differ by exactly one constraint (whether the bottom edge follows the safe
/// area or the host), so switching between them is a constant swap on the live
/// pane. Crossing into or out of `.scrolling` would have to move the content
/// into or out of an NSBackgroundExtensionView, i.e. reparent a pane that is on
/// screen, which is what evicted live pane content the last time this was
/// tried; that shape stays decided at install.
func ndRefreshPaneShape(_ content: NSView) {
    guard var install = ndPaneInstalls[ObjectIdentifier(content)],
          let host = install.host,
          let bottom = install.bottom else { return }
    let want = ndPaneContentShape(content)
    guard want != install.shape, install.shape != .scrolling, want != .scrolling else { return }
    bottom.isActive = false
    let anchor = want == .scrollingWithSiblings ? host.safeAreaLayoutGuide.bottomAnchor : host.bottomAnchor
    let replacement = content.bottomAnchor.constraint(equalTo: anchor)
    replacement.isActive = true
    install.bottom = replacement
    install.shape = want
    ndPaneInstalls[ObjectIdentifier(content)] = install
}

/// Wraps a pane's content in a flipped plain-NSView host (generated
/// SplitView append/insertBefore arms). The NSSplitViewItem supplies the
/// sidebar material/glass BEHIND this host; a plain NSView host does not
/// block the glass the way the old NSVisualEffectView wrapper did.
///
/// Three layouts (HIG Tahoe edge-to-edge layering):
///  - Scroll-shaped content pins to the HOST EDGES inside an
///    NSBackgroundExtensionView, so content scrolls under the floating
///    glass toolbar (scroll edge effect) and its background mirrors into
///    the unsafe regions. The extension view is nested INSIDE the flipped
///    host — automation's y-order contract rides the host class, never
///    swap it out (see NDPaneHostView).
///  - A stack whose expanding child scrolls but whose siblings don't stays
///    inside the safe area on every edge, so those siblings can't land under
///    the chrome and nothing of theirs is mirrored into it.
///  - Anything else keeps the safe-area pin: nothing insets a control
///    stack below the titlebar of a fullSizeContentView window on its own,
///    and edge-pinned controls rendered underneath the sidebar
///    (title field poking out past the glass, owner-reported).
func ndMakePaneViewController(_ content: NSView) -> NDPaneViewController {
    let host = NDPaneHostView()
    host.translatesAutoresizingMaskIntoConstraints = false
    ndInstallPaneContent(content, into: host)
    let vc = NDPaneViewController()
    vc.view = host
    // The generated arm hands over `pane.contentView`, never the pane (which
    // never joins the hierarchy). Recover it so the pane's auxiliary bars
    // can become this item's accessories.
    if let pane = ndPaneByContentView[ObjectIdentifier(content)] {
        vc.pane = pane
        pane.paneController = vc
    }
    return vc
}

/// Pins a pane's content root inside `host` per its shape, replacing whatever
/// was installed there. React sends a swapped pane child as remove-then-append
/// (deletions commit before placements), so the host is momentarily empty
/// between the two ops and the second half has to be able to install from
/// scratch — a swap that only knew how to retarget the outgoing view's
/// constraints left the pane blank for the rest of the process's life.
func ndInstallPaneContent(_ content: NSView, into host: NSView) {
    host.subviews.forEach { $0.removeFromSuperview() }
    content.translatesAutoresizingMaskIntoConstraints = false
    let shape = ndPaneContentShape(content)
    var bottom: NSLayoutConstraint?
    defer { ndPaneInstalls[ObjectIdentifier(content)] = NDPaneInstall(host: host, shape: shape, bottom: bottom) }
    switch shape {
    case .scrolling:
        let extended = NSBackgroundExtensionView()
        extended.translatesAutoresizingMaskIntoConstraints = false
        // Manual placement: automatic placement would park the scroll view
        // inside the safe area, exactly the inset-below-the-toolbar layout
        // this branch exists to remove.
        extended.automaticallyPlacesContentView = false
        extended.contentView = content
        host.addSubview(extended)
        // Horizontal pins ride the safe-area guide, vertical pins the edges.
        // A floating glass sidebar projects a LEFT safe-area inset into the
        // content pane, and the scroll view's automatic content insets
        // translate it into contentInsets.left; with the document
        // width-pinned to the clip (generated ScrollView create arm) that
        // shifted the whole document right and pushed its tail off the
        // window. Content starting at the safe leading edge sees a zero
        // horizontal inset, and the extension view still mirrors its
        // background beneath the glass. Vertically the content keeps the
        // full-height frame so it scrolls under the toolbar, with the
        // automatic TOP inset holding the resting position clear of the
        // titlebar. The guide is `extended`'s own (same region as the host's,
        // since extended is edge-pinned) so every content constraint lives in
        // `extended` and `ndSwapInstalledPaneContent` can retarget them all.
        NSLayoutConstraint.activate([
            extended.leadingAnchor.constraint(equalTo: host.leadingAnchor),
            extended.trailingAnchor.constraint(equalTo: host.trailingAnchor),
            extended.topAnchor.constraint(equalTo: host.topAnchor),
            extended.bottomAnchor.constraint(equalTo: host.bottomAnchor),
            content.leadingAnchor.constraint(equalTo: extended.safeAreaLayoutGuide.leadingAnchor),
            content.trailingAnchor.constraint(equalTo: extended.safeAreaLayoutGuide.trailingAnchor),
            content.topAnchor.constraint(equalTo: extended.topAnchor),
            content.bottomAnchor.constraint(equalTo: extended.bottomAnchor),
        ])
    case .scrollingWithSiblings:
        host.addSubview(content)
        // Safe area on EVERY edge, and NO NSBackgroundExtensionView. Its
        // non-scrolling siblings have no content-inset channel of their own,
        // so an edge pin would render the search box under the titlebar, the
        // footer under a bottom accessory, or (horizontally) shift the whole
        // stack right by a floating sidebar's left inset: the tail-off-window
        // regression documented in the `.scrolling` branch above. With the
        // content already clear of every unsafe region there is nothing left
        // for an extension view to extend — it mirrors and blurs the nearest
        // CONTENT into those regions, not just a background colour, which put
        // a smeared copy of a focused search field's ring and of a list's
        // footer caption into the titlebar band.
        bottom = content.bottomAnchor.constraint(equalTo: host.safeAreaLayoutGuide.bottomAnchor)
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: host.safeAreaLayoutGuide.leadingAnchor),
            content.trailingAnchor.constraint(equalTo: host.safeAreaLayoutGuide.trailingAnchor),
            content.topAnchor.constraint(equalTo: host.safeAreaLayoutGuide.topAnchor),
            bottom!,
        ])
    case .plain:
        host.addSubview(content)
        // ALL leading/trailing/top pins go through the safe-area guide, not
        // the host edges: with the content item's
        // automaticallyAdjustsSafeAreaInsets, the pane's FRAME extends under
        // the floating glass sidebar (Tahoe layering — the editor background
        // is what the glass blurs), but its LAYOUT must inset past it. For
        // the sidebar pane those insets are zero, so one form serves both.
        bottom = content.bottomAnchor.constraint(equalTo: host.bottomAnchor)
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: host.safeAreaLayoutGuide.leadingAnchor),
            content.trailingAnchor.constraint(equalTo: host.safeAreaLayoutGuide.trailingAnchor),
            bottom!,
            content.topAnchor.constraint(equalTo: host.safeAreaLayoutGuide.topAnchor),
        ])
    }
}
