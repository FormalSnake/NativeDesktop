import AppKit
import QuartzCore

// `<splitview edgeReveal>`: while the sidebar is collapsed, the pointer
// touching the window's leading edge slides the sidebar in as a floating
// panel over the content, and it slides away again when the pointer leaves
// it. The content pane is not resized, so a web page under it does not
// relayout (Arc's hidden sidebar, Safari's hidden tab bar).
//
// The collapsed split item stays collapsed. What moves is the pane's content
// root: it is lifted out of the item's host into the panel while shown, and
// installed back into the host when the panel goes, the same install the
// generated structural arms use.
//
// A `<windowcontrols>` slot in the sidebar takes the traffic lights along:
// hidden while the sidebar is, they slide in and out with the panel on the
// same timing, on the slot's place in it.

/// How far the pointer has to be from the leading edge to reveal. Narrow
/// enough that it never sits over anything the content draws.
private let ndRevealEdgeWidth: CGFloat = 6
/// The panel's gap to the window edges, matching the floating sidebar's own.
private let ndRevealInset: CGFloat = 8
/// Grace before a pointer that left the panel hides it: crossing a gap
/// between two controls, or overshooting the edge, should not flicker it.
private let ndRevealLeaveDelay: TimeInterval = 0.25

private final class NDRevealEdgeView: NSView {
    var onEnter: (() -> Void)?

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self, userInfo: nil))
    }

    override func mouseEntered(with event: NSEvent) { onEnter?() }

    // Transparent to clicks: only the pointer's presence matters here.
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

/// Holds the drop shadow (the glass clips to its own radius) and tracks the
/// pointer leaving.
private final class NDRevealPanel: NSView {
    var onExit: (() -> Void)?
    var onEnter: (() -> Void)?
    var radius: CGFloat = 16

    override nonisolated var isFlipped: Bool { true }

    override func layout() {
        super.layout()
        layer?.shadowPath = CGPath(roundedRect: bounds, cornerWidth: radius, cornerHeight: radius, transform: nil)
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self, userInfo: nil))
    }

    override func mouseEntered(with event: NSEvent) { onEnter?() }
    override func mouseExited(with event: NSEvent) { onExit?() }
}

@MainActor
final class NDSplitReveal {
    private unowned let controller: NDSplitViewController
    private var edge: NDRevealEdgeView?
    private var panel: NDRevealPanel?
    private var lifted: NSView?
    private weak var liftedHost: NSView?
    private var hideWork: DispatchWorkItem?
    private(set) var revealed = false
    var nodeID: UInt32 = 0
    /// The sidebar's width the last time it was open, so the panel comes in
    /// at the width the person left it at.
    var lastSidebarWidth: CGFloat = 0

    init(controller: NDSplitViewController) {
        self.controller = controller
    }

    private var sidebarItem: NSSplitViewItem? {
        controller.splitViewItems.first { $0.behavior == .sidebar }
    }

    /// The view the edge strip and the panel float in: the split's own
    /// container, above every pane (and so above a web page's native view).
    private var stage: NSView? {
        controller.splitView.superview ?? controller.view.window?.contentView
    }

    /// Re-derives what should be installed from `edgeReveal` and the item's
    /// collapsed state. Cheap enough to run on every split layout.
    func update() {
        let wanted = controller.edgeReveal && (sidebarItem?.isCollapsed ?? false)
        if !wanted {
            if revealed { conceal(animated: false) }
            edge?.removeFromSuperview()
            edge = nil
            return
        }
        guard let stage else { return }
        if edge == nil {
            let strip = NDRevealEdgeView()
            strip.onEnter = { [weak self] in self?.reveal(animated: true) }
            edge = strip
        }
        guard let strip = edge else { return }
        if strip.superview !== stage || stage.subviews.last !== (panel ?? strip) {
            stage.addSubview(strip, positioned: .above, relativeTo: nil)
            if let panel { stage.addSubview(panel, positioned: .above, relativeTo: nil) }
        }
        strip.frame = NSRect(x: 0, y: 0, width: ndRevealEdgeWidth, height: stage.bounds.height)
        strip.autoresizingMask = [.height]
    }

    private func setRevealed(_ on: Bool) {
        guard revealed != on else { return }
        revealed = on
        if nodeID != 0 { ndEmitEvent(nodeID, "revealChanged", on ? "{\"checked\":true}" : "{\"checked\":false}") }
    }

    private var reduceMotion: Bool { NSWorkspace.shared.accessibilityDisplayShouldReduceMotion }

    func reveal(animated: Bool) {
        hideWork?.cancel()
        hideWork = nil
        guard !revealed, controller.edgeReveal, let item = sidebarItem, item.isCollapsed,
              let stage, let window = stage.window else { return }
        let host = item.viewController.view
        guard let content = ndPaneContent(in: host) else { return }

        let width = lastSidebarWidth > 0 ? lastSidebarWidth : max(item.minimumThickness, 240)
        let height = max(0, stage.bounds.height - ndRevealInset * 2)
        let radius = ndRevealRadius(window)

        let panel = NDRevealPanel()
        panel.radius = radius
        panel.wantsLayer = true
        panel.layer?.shadowColor = NSColor.black.cgColor
        panel.layer?.shadowOpacity = 0.28
        panel.layer?.shadowRadius = 18
        panel.layer?.shadowOffset = .zero
        let chrome: NSView
        let body = NSView()
        body.translatesAutoresizingMaskIntoConstraints = false
        if #available(macOS 26.0, *) {
            let glass = NSGlassEffectView()
            glass.cornerRadius = radius
            glass.contentView = body
            chrome = glass
        } else {
            let effect = NSVisualEffectView()
            effect.material = .sidebar
            effect.blendingMode = .withinWindow
            effect.state = .active
            effect.wantsLayer = true
            effect.layer?.cornerRadius = radius
            effect.layer?.masksToBounds = true
            effect.addSubview(body)
            NSLayoutConstraint.activate([
                body.leadingAnchor.constraint(equalTo: effect.leadingAnchor),
                body.trailingAnchor.constraint(equalTo: effect.trailingAnchor),
                body.topAnchor.constraint(equalTo: effect.topAnchor),
                body.bottomAnchor.constraint(equalTo: effect.bottomAnchor),
            ])
            chrome = effect
        }
        // Sized before the chrome goes in: autoresizing adds the panel's own
        // growth from zero to the chrome, which doubled it.
        panel.frame = NSRect(x: 0, y: 0, width: width, height: height)
        chrome.frame = NSRect(x: 0, y: 0, width: width, height: height)
        chrome.autoresizingMask = [.width, .height]
        panel.addSubview(chrome)

        content.removeFromSuperview()
        content.translatesAutoresizingMaskIntoConstraints = false
        body.addSubview(content)
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: body.leadingAnchor),
            content.trailingAnchor.constraint(equalTo: body.trailingAnchor),
            content.topAnchor.constraint(equalTo: body.topAnchor),
            content.bottomAnchor.constraint(equalTo: body.bottomAnchor),
        ])
        lifted = content
        liftedHost = host

        let shown = ndRevealFrame(stage: stage, x: ndRevealInset, width: width, height: height)
        panel.frame = shown
        panel.autoresizingMask = [.height]
        panel.onExit = { [weak self] in self?.scheduleHide() }
        panel.onEnter = { [weak self] in
            self?.hideWork?.cancel()
            self?.hideWork = nil
        }
        stage.addSubview(panel, positioned: .above, relativeTo: nil)
        self.panel = panel
        setRevealed(true)

        guard animated else {
            NDTrafficLights.shared.schedule(window)
            return
        }
        // The traffic lights ride with the panel: placed on its slot where it
        // lands, then drawn along the same slide on the same timing.
        panel.layoutSubtreeIfNeeded()
        let lights = NDTrafficLights.shared
        lights.beginRide(window)
        let duration = 0.2 * ndAnimationSlowdown
        let timing = CAMediaTimingFunction(controlPoints: 0.23, 1, 0.32, 1)
        let start = ndRevealFrame(stage: stage, x: -width - ndRevealInset, width: width, height: height)
        if reduceMotion {
            panel.alphaValue = 0
        } else {
            panel.frame = start
        }
        let trace = NDRevealTrace.start(panel: panel, direction: "in")
        NSAnimationContext.runAnimationGroup({ ctx in
            ctx.duration = duration
            ctx.timingFunction = timing
            ctx.allowsImplicitAnimation = true
            if self.reduceMotion {
                panel.animator().alphaValue = 1
                lights.fade(window, from: 0, to: 1)
            } else {
                panel.animator().frame = shown
                lights.slide(window, from: start.minX - shown.minX, to: 0, duration: duration, timing: timing)
            }
        }, completionHandler: { [weak self, weak panel] in
            MainActor.assumeIsolated {
                trace?.stop()
                // A conceal that began mid-slide owns the ride now.
                guard let self, let panel, self.panel === panel, self.revealed else { return }
                lights.endRide(window)
            }
        })
    }

    private func scheduleHide() {
        hideWork?.cancel()
        let work = DispatchWorkItem { [weak self] in
            MainActor.assumeIsolated {
                guard let self, let panel = self.panel else { return }
                // Still over the panel (a tracking area can report an exit
                // into one of the panel's own subviews' popovers or menus).
                if let window = panel.window {
                    let p = panel.convert(window.mouseLocationOutsideOfEventStream, from: nil)
                    if panel.bounds.contains(p) { return }
                }
                self.conceal(animated: true)
            }
        }
        hideWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + ndRevealLeaveDelay, execute: work)
    }

    func conceal(animated: Bool) {
        hideWork?.cancel()
        hideWork = nil
        guard revealed, let panel else { return }
        setRevealed(false)
        guard animated, let stage = panel.superview else {
            finishConceal(panel)
            return
        }
        let window = panel.window
        let lights = NDTrafficLights.shared
        let width = panel.frame.width
        let from = panel.layer?.presentation()?.frame.minX ?? panel.frame.minX
        let end = ndRevealFrame(stage: stage, x: -width - ndRevealInset, width: width, height: panel.frame.height)
        let duration = 0.15 * ndAnimationSlowdown
        let timing = CAMediaTimingFunction(controlPoints: 0.23, 1, 0.32, 1)
        // Placed where the panel rests, and drawn from wherever the panel is
        // now, so a slide in still running turns round from where it got to.
        if let window { lights.beginRide(window) }
        let trace = NDRevealTrace.start(panel: panel, direction: "out")
        NSAnimationContext.runAnimationGroup({ ctx in
            ctx.duration = duration
            ctx.timingFunction = timing
            ctx.allowsImplicitAnimation = true
            if self.reduceMotion {
                panel.animator().alphaValue = 0
                if let window { lights.fade(window, from: 1, to: 0) }
            } else {
                panel.animator().frame = end
                if let window {
                    lights.slide(window, from: from - ndRevealInset, to: end.minX - ndRevealInset, duration: duration, timing: timing)
                }
            }
        }, completionHandler: { [weak self, weak panel] in
            MainActor.assumeIsolated {
                trace?.stop()
                guard let self, let panel else { return }
                self.finishConceal(panel)
            }
        })
    }

    /// Puts the lifted content back in its pane and drops the panel. A reveal
    /// that started while this one was sliding out owns a different panel, so
    /// only the one that finished is removed.
    private func finishConceal(_ panel: NDRevealPanel) {
        let window = panel.window
        panel.removeFromSuperview()
        guard self.panel === panel else { return }
        self.panel = nil
        if let content = lifted, let host = liftedHost {
            content.removeFromSuperview()
            ndInstallPaneContent(content, into: host)
        }
        lifted = nil
        liftedHost = nil
        if let window { NDTrafficLights.shared.endRide(window) }
    }
}

/// Stretches the reveal's slide by this factor (`ND_ANIMATION_SLOWDOWN`), so a
/// test can capture the frames in between. 1 unless set.
private let ndAnimationSlowdown: Double = {
    guard let raw = ProcessInfo.processInfo.environment["ND_ANIMATION_SLOWDOWN"], let v = Double(raw), v >= 1 else { return 1 }
    return v
}()

/// `ND_REVEAL_TRACE=1`: while the panel slides, prints on every display frame
/// where the panel and the close button are on screen (their presentation
/// layers), as `ND_REVEAL_FRAME` lines, so a test can hold the two together
/// frame by frame. `panel` and `close` are the panel's and the close
/// button's leading edges in window points.
@MainActor
private final class NDRevealTrace: NSObject {
    private weak var panel: NSView?
    private let direction: String
    private var link: CADisplayLink?
    private let begin = CACurrentMediaTime()

    private init(panel: NSView, direction: String) {
        self.panel = panel
        self.direction = direction
    }

    static func start(panel: NSView, direction: String) -> NDRevealTrace? {
        guard ProcessInfo.processInfo.environment["ND_REVEAL_TRACE"] == "1" else { return nil }
        let trace = NDRevealTrace(panel: panel, direction: direction)
        let link = panel.displayLink(target: trace, selector: #selector(tick))
        link.add(to: .main, forMode: .common)
        trace.link = link
        return trace
    }

    @objc private func tick() {
        guard let panel, let window = panel.window, let layer = panel.layer,
              let close = window.standardWindowButton(.closeButton), let closeLayer = close.layer else { return }
        let dx = (layer.presentation()?.position.x ?? layer.position.x) - layer.position.x
        let panelX = panel.convert(panel.bounds, to: nil).minX + dx
        let closeDx = (closeLayer.presentation()?.position.x ?? closeLayer.position.x) - closeLayer.position.x
        let closeX = close.convert(close.bounds, to: nil).minX + closeDx
        let alpha = closeLayer.presentation()?.opacity ?? closeLayer.opacity
        let line = String(format: "ND_REVEAL_FRAME dir=%@ t=%.4f panel=%.2f close=%.2f alpha=%.2f hidden=%d\n",
                          direction, CACurrentMediaTime() - begin, panelX, closeX, alpha, close.isHidden ? 1 : 0)
        FileHandle.standardError.write(line.data(using: .utf8)!)
    }

    func stop() {
        tick()
        link?.invalidate()
        link = nil
    }
}

/// A frame in `stage` at `x` from the leading edge, inset from the top and
/// bottom, whichever way the stage's y axis runs.
private func ndRevealFrame(stage: NSView, x: CGFloat, width: CGFloat, height: CGFloat) -> NSRect {
    NSRect(x: x, y: ndRevealInset, width: width, height: height)
}

/// Concentric with the window's own corner at the panel's inset, so the
/// floating panel reads as the same shape as the pinned sidebar.
private func ndRevealRadius(_ window: NSWindow) -> CGFloat {
    // Tahoe's window corner is 16 pt on a window with no toolbar.
    ndConcentricRadius(containerRadius: 16 + ndRevealInset, inset: ndRevealInset)
}

/// Generated SplitView connect arm: the controller emits `revealChanged`.
func ndSplitRevealConnect(_ view: NSView, nodeID: UInt32) {
    MainActor.assumeIsolated {
        guard let split = view as? NSSplitView, let controller = ndSplitViewController(for: split) else { return }
        controller.reveal.nodeID = nodeID
    }
}

/// Generated SplitView command arm (`revealSidebar` / `concealSidebar`).
@MainActor func ndSplitRevealCommand(_ view: NSView, _ command: String) {
    guard let split = view as? NSSplitView, let controller = ndSplitViewController(for: split) else { return }
    switch command {
    case "revealSidebar": controller.reveal.reveal(animated: true)
    case "concealSidebar": controller.reveal.conceal(animated: true)
    default: break
    }
}
