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
nonisolated(unsafe) var ndSplitControllers: [ObjectIdentifier: NSSplitViewController] = [:]

/// Looks up the controller owning `split` (generated SplitView append/
/// insertBefore/remove/apply arms resolve this before touching split view
/// items; precedent: `ndToolbarPaneAttachedToSplit`).
func ndSplitViewController(for split: NSSplitView) -> NSSplitViewController? {
    ndSplitControllers[ObjectIdentifier(split)]
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
    if let stack = view as? NSStackView, stack.arrangedSubviews.count == 1,
       stack.arrangedSubviews[0] is NSScrollView {
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
    guard let stack = view as? NSStackView, stack.orientation == .vertical else { return false }
    // The vexpanding child (Layout.swift's ndLayoutFlags) is the one that
    // absorbs the leftover height, so it is the one that reaches the edges: a
    // list pane with a search box above it still reads as scrolling even
    // though the stack around it does not.
    let expanding = stack.arrangedSubviews.filter { ndLayoutFlags[ObjectIdentifier($0)]?.vexpand == true }
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
    content.translatesAutoresizingMaskIntoConstraints = false
    switch ndPaneContentShape(content) {
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
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: host.safeAreaLayoutGuide.leadingAnchor),
            content.trailingAnchor.constraint(equalTo: host.safeAreaLayoutGuide.trailingAnchor),
            content.topAnchor.constraint(equalTo: host.safeAreaLayoutGuide.topAnchor),
            content.bottomAnchor.constraint(equalTo: host.safeAreaLayoutGuide.bottomAnchor),
        ])
    case .plain:
        host.addSubview(content)
        // ALL leading/trailing/top pins go through the safe-area guide, not
        // the host edges: with the content item's
        // automaticallyAdjustsSafeAreaInsets, the pane's FRAME extends under
        // the floating glass sidebar (Tahoe layering — the editor background
        // is what the glass blurs), but its LAYOUT must inset past it. For
        // the sidebar pane those insets are zero, so one form serves both.
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: host.safeAreaLayoutGuide.leadingAnchor),
            content.trailingAnchor.constraint(equalTo: host.safeAreaLayoutGuide.trailingAnchor),
            content.bottomAnchor.constraint(equalTo: host.bottomAnchor),
            content.topAnchor.constraint(equalTo: host.safeAreaLayoutGuide.topAnchor),
        ])
    }
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
