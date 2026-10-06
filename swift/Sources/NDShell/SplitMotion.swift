import AppKit
import QuartzCore

// `<splitview collapsed>`: the sidebar slides out and back in instead of
// snapping. The slide is the split item's own animated collapse: the split
// view draws the sidebar's glass through a layer of its own, which neither a
// transform nor a stand-in can reproduce without a visible change of colour
// at either end. What it would also do is lay the content out again on every
// frame, and for a web page that means Chromium resizing on every frame of
// the slide. So every page in the content is frozen at its size first and
// drawn as a still stretched over the rectangle it is laid out at, frame by
// frame (NDSlidingPage), and resized once when the slide lands. The traffic lights
// ride with the sidebar, read off the page's leading edge, which moves with
// it. A toggle mid-slide turns it round from where it is.

/// A page that can be drawn as a still over a slide instead of laid out on
/// every frame of it (NDCefMotion.swift).
@MainActor protocol NDSlidingPage: NSView {
    /// Freezes the page; `ready` runs once its still is up. False when the
    /// page cannot be frozen and should follow the layout.
    func motionFreeze(ready: @escaping @MainActor () -> Void) -> Bool
    func motionThaw()
}

/// How long a slide waits for the pages' stills before it starts without
/// them.
private let ndMotionStillPatience: TimeInterval = 0.12

@MainActor
final class NDSplitMotion: NSObject {
    private unowned let controller: NDSplitViewController
    private let trace: NDSplitMotionTrace?
    private var link: CADisplayLink?
    private var generation = 0
    private var pages: [NDSlidingPage] = []
    /// Where the sidebar is heading.
    private var collapsed: Bool
    /// Stills still to come before the slide starts.
    private var waiting = 0
    private var started = false
    /// The leading edge of the first page with the sidebar shown, in window
    /// points: the lights sit at their slot's rest when the page is there.
    private var shownX: CGFloat = 0
    private weak var lead: NSView?

    init(controller: NDSplitViewController, collapsed: Bool) {
        self.controller = controller
        self.collapsed = collapsed
        trace = NDSplitMotionTrace.start(view: controller.splitView, direction: collapsed ? "hide" : "show")
        super.init()
    }

    /// Freezes the pages and starts the slide once their stills are up: a
    /// still that arrives after the first frames would swap the page for its
    /// picture in plain sight.
    func begin() {
        guard let item = sidebarItem else { return }
        if let content = controller.splitViewItems.first(where: { $0.behavior == .default })?.viewController.view {
            let found = ndSlidingPages(in: content)
            waiting = found.count
            pages = found.filter { page in
                let frozen = page.motionFreeze { [weak self] in self?.stillReady() }
                if !frozen { waiting -= 1 }
                return frozen
            }
        }
        lead = pages.first
        if let page = pages.first, !item.isCollapsed { shownX = page.convert(page.bounds, to: nil).minX }
        if waiting <= 0 { start() } else {
            DispatchQueue.main.asyncAfter(deadline: .now() + ndMotionStillPatience) { [weak self] in
                MainActor.assumeIsolated { self?.start() }
            }
        }
    }

    private var sidebarItem: NSSplitViewItem? {
        controller.splitViewItems.first { $0.behavior == .sidebar }
    }

    private func stillReady() {
        waiting -= 1
        if waiting <= 0 { start() }
    }

    private func start() {
        guard !started, controller.motion === self else { return }
        started = true
        guard let item = sidebarItem, item.isCollapsed != collapsed else {
            // Toggled back before it moved.
            finish(generation)
            return
        }
        let l = controller.splitView.displayLink(target: self, selector: #selector(tick(_:)))
        l.add(to: .main, forMode: .common)
        link = l
        slide()
    }

    /// The `collapsed` prop again, mid-slide or before it started: heads
    /// for it from wherever the slide is.
    func run(collapsed: Bool) {
        guard collapsed != self.collapsed else { return }
        self.collapsed = collapsed
        guard started else { return }
        trace?.reversals += 1
        slide()
    }

    private func slide() {
        guard let item = sidebarItem, let window = controller.splitView.window else { return }
        generation += 1
        let mine = generation
        let collapsed = self.collapsed
        NSAnimationContext.runAnimationGroup({ _ in
            item.animator().isCollapsed = collapsed
        }, completionHandler: { [weak self] in
            MainActor.assumeIsolated { self?.finish(mine) }
        })
        controller.splitView.layoutSubtreeIfNeeded()
        guard mine == 1 else { return }
        // Showing, the sidebar's views are still where the collapse left them,
        // a width off to the left: the lights rest that far to the right of
        // where the slot is now.
        let sidebar = item.viewController.view
        let sidebarMaxX = sidebar.convert(sidebar.bounds, to: nil).maxX
        if shownX == 0 { shownX = controller.splitView.convert(NSPoint(x: sidebar.frame.width, y: 0), to: nil).x }
        NDTrafficLights.shared.beginRide(window, shift: collapsed ? 0 : shownX - sidebarMaxX)
        NDTrafficLights.shared.ghostRide(window)
        tick(nil)
    }

    @objc private func tick(_ link: CADisplayLink?) {
        if let link { trace?.frame(link) }
        guard let window = controller.splitView.window, let lead else { return }
        NDTrafficLights.shared.offsetRide(window, by: min(0, lead.convert(lead.bounds, to: nil).minX - shownX))
    }

    private func finish(_ mine: Int) {
        guard mine == generation, controller.motion === self else { return }
        controller.motion = nil
        link?.invalidate()
        link = nil
        trace?.stop()
        controller.splitView.layoutSubtreeIfNeeded()
        for page in pages { page.motionThaw() }
        if let window = controller.splitView.window { NDTrafficLights.shared.endRide(window) }
    }
}

@MainActor private func ndSlidingPages(in view: NSView) -> [NDSlidingPage] {
    if let page = view as? NDSlidingPage {
        // A view a few points across is a probe or an extension's page, not
        // something on show.
        return page.isHiddenOrHasHiddenAncestor || page.bounds.width < 32 || page.bounds.height < 32 ? [] : [page]
    }
    return view.subviews.flatMap { ndSlidingPages(in: $0) }
}

extension NDSplitViewController {
    /// The `collapsed` prop. Lands at once off screen, with no sidebar, or with
    /// Reduce Motion on.
    func setSidebarCollapsed(_ collapsed: Bool) {
        guard let item = splitViewItems.first(where: { $0.behavior == .sidebar }) else { return }
        if let motion {
            motion.run(collapsed: collapsed)
            return
        }
        guard item.isCollapsed != collapsed else { return }
        // A peeking sidebar is already in place: it is pinned where it is.
        if reveal.revealed { reveal.conceal(animated: false) }
        guard let window = splitView.window, window.isVisible,
              !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion else {
            item.isCollapsed = collapsed
            return
        }
        let slide = NDSplitMotion(controller: self, collapsed: collapsed)
        motion = slide
        slide.begin()
    }
}

/// `ND_MOTION_TRACE=1`: one `ND_SPLIT_MOTION` line per slide with the display
/// frames it ran on, the same line the GTK host prints.
@MainActor
final class NDSplitMotionTrace {
    private var stamps: [CFTimeInterval] = []
    private var refresh: CFTimeInterval = 1.0 / 60
    private let direction: String
    var reversals = 0

    private init(direction: String) { self.direction = direction }

    static func start(view: NSView, direction: String) -> NDSplitMotionTrace? {
        guard ProcessInfo.processInfo.environment["ND_MOTION_TRACE"] != nil else { return nil }
        return NDSplitMotionTrace(direction: direction)
    }

    func frame(_ link: CADisplayLink) {
        stamps.append(link.timestamp)
        refresh = link.duration > 0 ? link.duration : refresh
    }

    func stop() {
        guard stamps.count >= 2 else { return }
        var worst: CFTimeInterval = 0
        var late = 0
        for (a, b) in zip(stamps, stamps.dropFirst()) {
            worst = max(worst, b - a)
            if (b - a) * 2 > refresh * 3 { late += 1 }
        }
        let line = String(format: "ND_SPLIT_MOTION dir=%@ frames=%d span_ms=%.1f worst_ms=%.1f late=%d refresh_ms=%.2f reversals=%d\n",
                          direction, stamps.count, (stamps.last! - stamps.first!) * 1000, worst * 1000, late, refresh * 1000, reversals)
        FileHandle.standardError.write(line.data(using: .utf8)!)
    }
}
