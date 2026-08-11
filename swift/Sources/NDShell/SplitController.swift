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

/// A pane whose content root scrolls its own content: an NSScrollView, or a
/// Box whose sole arranged subview is one. Such a pane may extend under the
/// floating glass chrome — the scroll view insets its content via the safe
/// area itself and AppKit draws the scroll edge effect under the glass.
private func ndIsScrollShaped(_ view: NSView) -> Bool {
    if view is NSScrollView { return true }
    if let stack = view as? NSStackView, stack.arrangedSubviews.count == 1,
       stack.arrangedSubviews[0] is NSScrollView {
        return true
    }
    return false
}

/// Wraps a pane's content in a flipped plain-NSView host (generated
/// SplitView append/insertBefore arms). The NSSplitViewItem supplies the
/// sidebar material/glass BEHIND this host; a plain NSView host does not
/// block the glass the way the old NSVisualEffectView wrapper did.
///
/// Two layouts (HIG Tahoe edge-to-edge layering):
///  - Scroll-shaped content pins to the HOST EDGES inside an
///    NSBackgroundExtensionView, so content scrolls under the floating
///    glass toolbar (scroll edge effect) and its background mirrors into
///    the unsafe regions. The extension view is nested INSIDE the flipped
///    host — automation's y-order contract rides the host class, never
///    swap it out (see NDPaneHostView).
///  - Anything else keeps the safe-area pin: nothing insets a control
///    stack below the titlebar of a fullSizeContentView window on its own,
///    and edge-pinned controls rendered underneath the sidebar
///    (title field poking out past the glass, owner-reported).
func ndMakePaneViewController(_ content: NSView) -> NSViewController {
    let host = NDPaneHostView()
    host.translatesAutoresizingMaskIntoConstraints = false
    content.translatesAutoresizingMaskIntoConstraints = false
    if ndIsScrollShaped(content) {
        let extended = NSBackgroundExtensionView()
        extended.translatesAutoresizingMaskIntoConstraints = false
        // Manual placement: automatic placement would park the scroll view
        // inside the safe area, exactly the inset-below-the-toolbar layout
        // this branch exists to remove.
        extended.automaticallyPlacesContentView = false
        extended.contentView = content
        host.addSubview(extended)
        NSLayoutConstraint.activate([
            extended.leadingAnchor.constraint(equalTo: host.leadingAnchor),
            extended.trailingAnchor.constraint(equalTo: host.trailingAnchor),
            extended.topAnchor.constraint(equalTo: host.topAnchor),
            extended.bottomAnchor.constraint(equalTo: host.bottomAnchor),
            content.leadingAnchor.constraint(equalTo: extended.leadingAnchor),
            content.trailingAnchor.constraint(equalTo: extended.trailingAnchor),
            content.topAnchor.constraint(equalTo: extended.topAnchor),
            content.bottomAnchor.constraint(equalTo: extended.bottomAnchor),
        ])
    } else {
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
    let vc = NSViewController()
    vc.view = host
    return vc
}
