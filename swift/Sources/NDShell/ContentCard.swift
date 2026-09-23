import AppKit

// `<splitview contentStyle="card">`: the content pane is a rounded card
// beside the split view's own glass sidebar (Arc's layout). The pane behind
// the card draws the sidebar material, so the margin around the card reads as
// the sidebar's surface; the content root is inset by the margin and clipped
// to the card's corner, which is what keeps a web page's square corners
// inside the curve.

/// The margin between the card and the window edges.
let ndContentCardMargin: CGFloat = 8

/// Content roots currently drawn as a card, so a style apply that carries no
/// border leaves their corner alone (Backend.swift's `ndApplyStyle`).
nonisolated(unsafe) var ndContentCardRoots: Set<ObjectIdentifier> = []

/// The pane background behind the card: the same material the sidebar item
/// draws, so there is no seam where the sidebar ends.
final class NDContentCardBackdrop: NSVisualEffectView {
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

/// The card's own hairline tracks light and dark, which a CGColor does not do
/// on its own.
final class NDContentCardRootObserver: NSView {
    weak var root: NSView?
    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        if let root { ndStyleContentCardRoot(root, on: true) }
    }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

@MainActor private func ndStyleContentCardRoot(_ root: NSView, on: Bool) {
    let key = ObjectIdentifier(root)
    if on {
        ndContentCardRoots.insert(key)
        root.wantsLayer = true
        guard let layer = root.layer else { return }
        layer.cornerRadius = NDRadius.card
        layer.cornerCurve = .continuous
        layer.masksToBounds = true
        layer.borderWidth = 1
        var color = NSColor.separatorColor.cgColor
        root.effectiveAppearance.performAsCurrentDrawingAppearance { color = NSColor.separatorColor.cgColor }
        layer.borderColor = color
    } else if ndContentCardRoots.contains(key) {
        ndContentCardRoots.remove(key)
        root.layer?.cornerRadius = 0
        root.layer?.masksToBounds = false
        root.layer?.borderWidth = 0
        root.layer?.borderColor = nil
    }
}

/// Re-derived on every split layout: the leading margin depends on whether
/// the sidebar is showing (its own padding is the gap then), and a pane that
/// was reinstalled (a reveal putting content back, a swapped root) comes back
/// flush until this runs.
@MainActor func ndApplyContentCard(_ controller: NDSplitViewController) {
    guard let item = controller.splitViewItems.first(where: { $0.behavior == .default }) else { return }
    let host = item.viewController.view
    let content = ndPaneContent(in: host)
    let on = controller.contentCard

    // Only behind the content pane: the sidebar stays the split view's own
    // glass, which reflects the page beside it the way a native sidebar does.
    let sidebarHost = controller.splitViewItems.first { $0.behavior == .sidebar }?.viewController.view
    sidebarHost?.subviews.first { $0 is NDContentCardBackdrop }?.removeFromSuperview()
    for pane in [host] {
        let backdrop = pane.subviews.first { $0 is NDContentCardBackdrop }
        if on, backdrop == nil {
            let bg = NDContentCardBackdrop()
            bg.material = .sidebar
            bg.blendingMode = .behindWindow
            bg.state = .followsWindowActiveState
            bg.frame = pane.bounds
            bg.autoresizingMask = [.width, .height]
            pane.addSubview(bg, positioned: .below, relativeTo: nil)
        } else if !on, let backdrop {
            backdrop.removeFromSuperview()
        }
    }

    let sidebarShown = controller.splitViewItems.contains { $0.behavior == .sidebar && !$0.isCollapsed }
    let m = ndContentCardMargin
    ndSetPaneInsets(host, on ? NSEdgeInsets(top: m, left: sidebarShown ? 0 : m, bottom: m, right: m) : NSEdgeInsets())

    // Previous roots that are no longer this pane's content lose the look.
    for (key, observer) in ndContentCardObservers where observer.superview === host && content.map(ObjectIdentifier.init) != key {
        if let old = observer.root { ndStyleContentCardRoot(old, on: false) }
        observer.removeFromSuperview()
        ndContentCardObservers[key] = nil
    }
    guard let content else { return }
    ndStyleContentCardRoot(content, on: on)
    let key = ObjectIdentifier(content)
    if on, ndContentCardObservers[key] == nil {
        let observer = NDContentCardRootObserver(frame: .zero)
        observer.root = content
        host.addSubview(observer)
        ndContentCardObservers[key] = observer
    } else if !on, let observer = ndContentCardObservers[key] {
        observer.removeFromSuperview()
        ndContentCardObservers[key] = nil
    }
}

nonisolated(unsafe) private var ndContentCardObservers: [ObjectIdentifier: NDContentCardRootObserver] = [:]
