import AppKit

// `<windowcontrols>`: where the window's own close/minimize/zoom buttons sit.
// The buttons stay the window's (they keep their hover group, their menu and
// their accessibility); this view is the slot they are moved onto, the way
// Electron's `trafficLightPosition` moves them. A window with no such view in
// its tree keeps the buttons where AppKit puts them.
//
// AppKit relays the title bar out on a resize, a key change and a full screen
// transition, and puts the buttons back each time, so the placement is
// re-asserted from those notifications and from the buttons' own frame
// changes rather than done once.

/// The three buttons, leading to trailing.
private let ndTrafficLightTypes: [NSWindow.ButtonType] = [.closeButton, .miniaturizeButton, .zoomButton]

/// What AppKit laid out before anything was moved, so a window whose slot
/// leaves the tree gets its stock title bar back.
private struct NDTrafficLightDefaults {
    var container: NSRect
    var buttons: [NSRect]
}

@MainActor
final class NDTrafficLights {
    static let shared = NDTrafficLights()

    private var slots: [ObjectIdentifier: [WeakSlot]] = [:]
    private var defaults: [ObjectIdentifier: NDTrafficLightDefaults] = [:]
    private var observed: Set<ObjectIdentifier> = []
    private var layoutObservers: [ObjectIdentifier: NSKeyValueObservation] = [:]
    private var applying = false
    private var scheduled: Set<ObjectIdentifier> = []
    /// Windows whose buttons are riding with a slot that is mid-animation (a
    /// sidebar sliding in or out). The ride owns the buttons until it ends:
    /// a title bar pass in between would read the slot's landing place, or
    /// find it clipped and hide them.
    private var riding: [ObjectIdentifier: NSRect] = [:]
    /// Stand-ins for the buttons while a split view slides its sidebar
    /// (`ghostRide`).
    private var ghosts: [ObjectIdentifier: NSView] = [:]

    /// Starts a ride: the buttons go where the slot is now, which has to be
    /// where it rests (the title bar will not keep a button outside its
    /// bounds, so the travel itself is drawn by `slide`), and stay there
    /// until the ride ends, wherever the slot's container is meanwhile.
    /// `shift` moves the resting place off the slot's current one, for a slot
    /// whose container has not reached its rest yet.
    func beginRide(_ window: NSWindow, shift: CGFloat = 0) {
        guard let slot = (slots[ObjectIdentifier(window)] ?? []).lazy.compactMap(\.view).first(where: { $0.window === window && $0.bounds.width > 0 })
        else { return }
        riding[ObjectIdentifier(window)] = slot.convert(slot.bounds, to: nil).offsetBy(dx: shift, dy: 0)
        apply(window)
    }

    /// Draws the buttons `from` points off their place, travelling to `to`,
    /// on the timing the slot's container moves with. An additive animation
    /// on the layers: the buttons' frames stay where the title bar allows,
    /// and added in the same transaction as the container's, it starts on
    /// the same frame.
    func slide(_ window: NSWindow, from: CGFloat, to: CGFloat, duration: TimeInterval, timing: CAMediaTimingFunction) {
        for b in buttons(window) {
            b.wantsLayer = true
            let a = CABasicAnimation(keyPath: "position.x")
            a.isAdditive = true
            a.fromValue = from
            a.toValue = to
            a.duration = duration
            a.timingFunction = timing
            a.fillMode = .both
            a.isRemovedOnCompletion = false
            b.layer?.add(a, forKey: "ndRide")
        }
    }

    /// Swaps the buttons, placed where the ride rests, for pictures of
    /// themselves for the length of a ride. A split view sliding its sidebar
    /// lays the title bar out on every frame, and puts the zoom button back at
    /// its stock place in the render tree on each one, under any placement
    /// made on the view; the pictures are views the title bar never lays out.
    func ghostRide(_ window: NSWindow) {
        let bs = buttons(window)
        guard ghosts[ObjectIdentifier(window)] == nil, bs.count == 3, let bar = bs[0].superview,
              bs.allSatisfy({ $0.superview === bar && !$0.isHidden }) else { return }
        let frame = bs.map(\.frame).reduce(bs[0].frame) { $0.union($1) }
        let ghost = NSView(frame: frame)
        ghost.wantsLayer = true
        for b in bs {
            guard let rep = b.bitmapImageRepForCachingDisplay(in: b.bounds) else { continue }
            b.cacheDisplay(in: b.bounds, to: rep)
            let image = NSImage(size: b.bounds.size)
            image.addRepresentation(rep)
            let picture = NSImageView(frame: b.frame.offsetBy(dx: -frame.minX, dy: -frame.minY))
            picture.image = image
            picture.imageScaling = .scaleNone
            ghost.addSubview(picture)
        }
        bar.addSubview(ghost, positioned: .above, relativeTo: nil)
        ghosts[ObjectIdentifier(window)] = ghost
        for b in bs { b.alphaValue = 0 }
    }

    /// Draws the buttons (or their stand-ins) `dx` points off their place for
    /// one frame of a slide the main thread follows (SplitMotion.swift).
    /// Additive on the layer, like `slide`: the title bar does not keep a
    /// button frame outside it.
    func offsetRide(_ window: NSWindow, by dx: CGFloat) {
        let layers = ghosts[ObjectIdentifier(window)].map { [$0] } ?? buttons(window)
        for v in layers {
            v.wantsLayer = true
            let a = CABasicAnimation(keyPath: "position.x")
            a.isAdditive = true
            a.fromValue = dx
            a.toValue = dx
            a.duration = 3600
            a.fillMode = .both
            a.isRemovedOnCompletion = false
            v.layer?.add(a, forKey: "ndRide")
        }
    }

    /// Reduced motion: the buttons fade with the panel instead of travelling.
    func fade(_ window: NSWindow, from: CGFloat, to: CGFloat) {
        for b in buttons(window) {
            b.alphaValue = from
            b.animator().alphaValue = to
        }
    }

    /// Ends a ride and hands the buttons back to the slot's normal placement.
    func endRide(_ window: NSWindow) {
        riding[ObjectIdentifier(window)] = nil
        ghosts.removeValue(forKey: ObjectIdentifier(window))?.removeFromSuperview()
        for b in buttons(window) {
            b.layer?.removeAnimation(forKey: "ndRide")
            b.alphaValue = 1
        }
        apply(window)
    }

    private struct WeakSlot { weak var view: NDWindowControlsView? }

    /// The cluster's size as AppKit draws it on this OS: leading edge of close
    /// to trailing edge of zoom, and one button's height. Measured off a live
    /// window's stock layout when there is one.
    func clusterSize(for window: NSWindow?) -> NSSize {
        if let window, let d = defaultsFor(window), let first = d.buttons.first, let last = d.buttons.last {
            return NSSize(width: last.maxX - first.minX, height: first.height)
        }
        let probe = NSWindow.standardWindowButton(.closeButton, for: [.titled, .closable, .miniaturizable, .resizable])
        let h = probe?.frame.height ?? 16
        // Stock spacing: 6 pt between 14 pt buttons plus the button's own
        // width, which is what defaultsFor measures on a real window.
        let w = (probe?.frame.width ?? 14) * 3 + 12
        return NSSize(width: w, height: h)
    }

    func register(_ slot: NDWindowControlsView, in window: NSWindow) {
        let key = ObjectIdentifier(window)
        var list = (slots[key] ?? []).filter { $0.view != nil && $0.view !== slot }
        list.append(WeakSlot(view: slot))
        slots[key] = list
        observe(window)
        _ = defaultsFor(window)
        schedule(window)
    }

    func unregister(_ slot: NDWindowControlsView, from window: NSWindow) {
        let key = ObjectIdentifier(window)
        slots[key] = (slots[key] ?? []).filter { $0.view != nil && $0.view !== slot }
        schedule(window)
    }

    /// Coalesced to the next main-queue turn: a slot moving inside one layout
    /// pass reports several frames, and only the last one is where it landed.
    func schedule(_ window: NSWindow) {
        let key = ObjectIdentifier(window)
        if scheduled.contains(key) { return }
        scheduled.insert(key)
        DispatchQueue.main.async { [weak window] in
            MainActor.assumeIsolated {
                guard let window else { return }
                NDTrafficLights.shared.scheduled.remove(ObjectIdentifier(window))
                NDTrafficLights.shared.apply(window)
            }
        }
    }

    private func buttons(_ window: NSWindow) -> [NSButton] {
        ndTrafficLightTypes.compactMap { window.standardWindowButton($0) }
    }

    private func defaultsFor(_ window: NSWindow) -> NDTrafficLightDefaults? {
        let key = ObjectIdentifier(window)
        if let d = defaults[key] { return d }
        let bs = buttons(window)
        guard bs.count == 3, let container = bs[0].superview?.superview else { return nil }
        // Read while AppKit is halfway through putting the buttons back (one
        // moved, the next not yet) the three are closer than their own width,
        // and every placement after would copy that overlap. Not kept then;
        // the next pass reads again.
        let steps = zip(bs.dropFirst(), bs).map { $0.frame.minX - $1.frame.minX }
        guard steps.allSatisfy({ (16...32).contains($0) }) else { return nil }
        let d = NDTrafficLightDefaults(container: container.frame, buttons: bs.map(\.frame))
        defaults[key] = d
        return d
    }

    /// The slot the buttons follow: the first registered one that is actually
    /// on screen in this window. A slot inside a collapsed sidebar has an empty
    /// visible rect, and then the window shows no buttons at all, which is how
    /// a hidden sidebar reads in Arc and Safari's hidden-toolbar mode.
    private func activeSlot(_ window: NSWindow) -> (NDWindowControlsView, NSRect)? {
        for entry in slots[ObjectIdentifier(window)] ?? [] {
            guard let v = entry.view, v.window === window, !v.isHiddenOrHasHiddenAncestor else { continue }
            let visible = v.visibleRect
            guard visible.width >= v.bounds.width - 0.5, visible.height >= v.bounds.height - 0.5, v.bounds.width > 0 else { continue }
            return (v, v.convert(v.bounds, to: nil))
        }
        return nil
    }

    private func hasSlots(_ window: NSWindow) -> Bool {
        (slots[ObjectIdentifier(window)] ?? []).contains { $0.view?.window === window }
    }

    func apply(_ window: NSWindow) {
        guard !applying, let d = defaultsFor(window) else { return }
        let bs = buttons(window)
        guard bs.count == 3, let container = bs[0].superview?.superview else { return }
        applying = true
        defer { applying = false }
        ndSetWindowOwnsTitlebar(window, hasSlots(window))

        // Full screen draws the title bar in a separate reveal window, which
        // AppKit owns; a window without a slot keeps its stock layout.
        if window.styleMask.contains(.fullScreen) || !hasSlots(window) {
            restore(window, d, bs, container)
            return
        }
        let rect: NSRect
        if let resting = riding[ObjectIdentifier(window)] {
            // The title bar puts the buttons back on its own passes (one
            // runs when they are unhidden), so a ride keeps asserting them.
            rect = resting
        } else {
            guard let (_, slot) = activeSlot(window) else {
                for b in bs { b.isHidden = true }
                return
            }
            rect = slot
            recheck(window, placedAt: rect)
        }
        // Window coordinates are bottom-up; the slot's centre as a distance
        // from the top is what the title bar band has to be twice of, so the
        // buttons sit vertically centred on the slot.
        let centreFromTop = window.frame.height - rect.midY
        place(window, d, bs, container, centreFromTop: centreFromTop, leading: rect.minX,
              bandHeight: max(d.container.height, (centreFromTop * 2).rounded()))
    }

    /// The close button's leading edge at `leading`, the row centred
    /// `centreFromTop` down from the window's top, AppKit's own spacing kept.
    private func place(_ window: NSWindow, _ d: NDTrafficLightDefaults, _ bs: [NSButton], _ container: NSView,
                       centreFromTop: CGFloat, leading: CGFloat, bandHeight: CGFloat) {
        var frame = container.frame
        frame.size.height = bandHeight
        frame.origin.y = window.frame.height - bandHeight
        if frame != container.frame { container.frame = frame }
        let first = d.buttons[0].minX
        for (i, b) in bs.enumerated() {
            let stock = d.buttons[i]
            let x = (leading + (stock.minX - first)).rounded()
            // The title bar view fills the band; its y axis runs either way
            // depending on the OS release, so it is asked rather than assumed.
            let flipped = b.superview?.isFlipped ?? false
            let top = (centreFromTop - stock.height / 2).rounded()
            let y = flipped ? top : bandHeight - top - stock.height
            let origin = NSPoint(x: x, y: y)
            if b.frame.origin != origin { b.setFrameOrigin(origin) }
            b.isHidden = false
        }
    }

    /// A split's safe area settling after a toolbar band goes (a layout switch)
    /// moves the slot with its whole column, so the slot's own frame never
    /// changes and nothing above reports it. The slot is read again a moment
    /// after each placement until it holds still.
    private var rechecks: [ObjectIdentifier: Int] = [:]

    private func recheck(_ window: NSWindow, placedAt rect: NSRect) {
        let key = ObjectIdentifier(window)
        let n = rechecks[key] ?? 0
        guard n < 20 else { return }
        rechecks[key] = n + 1
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { [weak window] in
            MainActor.assumeIsolated {
                guard let window else { return }
                let lights = NDTrafficLights.shared
                if let (_, now) = lights.activeSlot(window), now != rect {
                    lights.schedule(window)
                } else {
                    lights.rechecks[key] = 0
                }
            }
        }
    }

    private func restore(_ window: NSWindow, _ d: NDTrafficLightDefaults, _ bs: [NSButton], _ container: NSView) {
        // A toolbar band taller than the stock title bar (a header bar row):
        // the close button goes as far in from the left edge as from the top,
        // centred on the band, the way a unified toolbar sets them. The stock
        // frames were read before any toolbar existed and would pin the
        // buttons to the band's top-left corner.
        let band = (window.frame.height - window.contentLayoutRect.maxY).rounded()
        if !window.styleMask.contains(.fullScreen), window.toolbar?.isVisible == true, band > d.container.height + 1 {
            place(window, d, bs, container, centreFromTop: band / 2, leading: band / 2 - d.buttons[0].width / 2, bandHeight: band)
            return
        }
        var frame = container.frame
        frame.size.height = d.container.height
        frame.origin.y = window.frame.height - d.container.height
        if !window.styleMask.contains(.fullScreen), frame != container.frame { container.frame = frame }
        for (i, b) in bs.enumerated() {
            if !window.styleMask.contains(.fullScreen), b.frame.origin != d.buttons[i].origin {
                b.setFrameOrigin(d.buttons[i].origin)
            }
            b.isHidden = false
        }
    }

    private func observe(_ window: NSWindow) {
        let key = ObjectIdentifier(window)
        guard !observed.contains(key) else { return }
        observed.insert(key)
        let names: [Notification.Name] = [
            NSWindow.didResizeNotification, NSWindow.didEndLiveResizeNotification,
            NSWindow.didBecomeKeyNotification, NSWindow.didResignKeyNotification,
            NSWindow.didEnterFullScreenNotification, NSWindow.didExitFullScreenNotification,
            NSWindow.didChangeScreenNotification,
        ]
        for name in names {
            NotificationCenter.default.addObserver(forName: name, object: window, queue: .main) { [weak window] _ in
                MainActor.assumeIsolated {
                    guard let window else { return }
                    NDTrafficLights.shared.apply(window)
                }
            }
        }
        // AppKit moves the buttons back on its own title bar layouts, and a
        // button's frame change is the signal every such pass sends. During a
        // ride the placement is put back at once, inside that same pass: a
        // turn later the frame between has already been drawn.
        for button in buttons(window) {
            button.postsFrameChangedNotifications = true
            NotificationCenter.default.addObserver(forName: NSView.frameDidChangeNotification, object: button, queue: nil) { [weak window] _ in
                MainActor.assumeIsolated {
                    guard let window else { return }
                    let lights = NDTrafficLights.shared
                    if lights.riding[ObjectIdentifier(window)] != nil { lights.apply(window) } else { lights.schedule(window) }
                }
            }
        }
        // A toolbar band coming or going moves the slot on the window without
        // moving it in its superview, so nothing above reports it.
        layoutObservers[key] = window.observe(\.contentLayoutRect, options: [.new]) { window, _ in
            MainActor.assumeIsolated { NDTrafficLights.shared.schedule(window) }
        }
        NotificationCenter.default.addObserver(forName: NSWindow.willCloseNotification, object: window, queue: .main) { _ in
            MainActor.assumeIsolated {
                NDTrafficLights.shared.slots[key] = nil
                NDTrafficLights.shared.defaults[key] = nil
                NDTrafficLights.shared.layoutObservers[key] = nil
            }
        }
    }
}

final class NDWindowControlsView: NSView {
    let side: String
    private weak var registeredWindow: NSWindow?

    init(side: String) {
        self.side = side
        super.init(frame: .zero)
        setAccessibilityElement(false)
        postsFrameChangedNotifications = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("NDWindowControlsView is not NSCoding-decodable") }

    override nonisolated var isFlipped: Bool { true }

    /// The start slot is the traffic lights' size; the end slot is empty here,
    /// since macOS puts every window button on the leading side.
    override var intrinsicContentSize: NSSize {
        side == "start" ? NDTrafficLights.shared.clusterSize(for: window) : .zero
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if let old = registeredWindow, old !== window { NDTrafficLights.shared.unregister(self, from: old) }
        registeredWindow = window
        guard side == "start", let window else { return }
        invalidateIntrinsicContentSize()
        ndInvalidateBoxChain(from: self)
        NDTrafficLights.shared.register(self, in: window)
    }

    override func setFrameOrigin(_ newOrigin: NSPoint) {
        super.setFrameOrigin(newOrigin)
        if let window, side == "start" { NDTrafficLights.shared.schedule(window) }
    }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        if let window, side == "start" { NDTrafficLights.shared.schedule(window) }
    }

    override func viewDidHide() {
        super.viewDidHide()
        if let window { NDTrafficLights.shared.schedule(window) }
    }

    override func viewDidUnhide() {
        super.viewDidUnhide()
        if let window { NDTrafficLights.shared.schedule(window) }
    }

    /// An ancestor moving (a sidebar sliding, a pane collapsing) does not
    /// touch this view's own frame, so the placement is re-checked on every
    /// layout the window runs.
    override func layout() {
        super.layout()
        if let window, side == "start" { NDTrafficLights.shared.schedule(window) }
    }
}

/// A title bar double click, done the way the user set it in System Settings
/// (Desktop & Dock, "Double-click a window's title bar to").
@MainActor func ndPerformTitlebarDoubleClick(_ window: NSWindow) {
    let action = UserDefaults.standard.string(forKey: "AppleActionOnDoubleClick") ?? "Maximize"
    switch action {
    case "Minimize": window.performMiniaturize(nil)
    case "None": break
    default: window.performZoom(nil)
    }
}

/// Generated WindowControls connect arm. macOS puts all three buttons on the
/// leading side, so the start slot always holds them and the end slot never
/// does; reported once, since the platform has no setting that moves them.
func ndWindowControlsConnect(_ view: NSView, nodeID: UInt32) {
    MainActor.assumeIsolated {
        guard let slot = view as? NDWindowControlsView else { return }
        let empty = slot.side != "start"
        ndEmitEvent(nodeID, "emptyChanged", empty ? "{\"checked\":true}" : "{\"checked\":false}")
    }
}
