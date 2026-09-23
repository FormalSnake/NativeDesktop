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
    private var applying = false
    private var scheduled: Set<ObjectIdentifier> = []
    /// Windows whose buttons are held hidden while their slot is mid-animation
    /// (a sidebar sliding in), so they appear where the slot lands rather than
    /// jumping there ahead of it.
    private var suspended: Set<ObjectIdentifier> = []

    func setSuspended(_ window: NSWindow, _ on: Bool) {
        let key = ObjectIdentifier(window)
        if on { suspended.insert(key) } else { suspended.remove(key) }
        if on {
            for b in buttons(window) { b.isHidden = true }
        } else {
            schedule(window)
        }
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
        if suspended.contains(ObjectIdentifier(window)) {
            for b in bs { b.isHidden = true }
            return
        }
        guard let (_, rect) = activeSlot(window) else {
            for b in bs { b.isHidden = true }
            return
        }
        // Window coordinates are bottom-up; the slot's centre as a distance
        // from the top is what the title bar band has to be twice of, so the
        // buttons sit vertically centred on the slot.
        let windowHeight = window.frame.height
        let centreFromTop = windowHeight - rect.midY
        let bandHeight = max(d.container.height, (centreFromTop * 2).rounded())
        var frame = container.frame
        frame.size.height = bandHeight
        frame.origin.y = windowHeight - bandHeight
        if frame != container.frame { container.frame = frame }
        let first = d.buttons[0].minX
        for (i, b) in bs.enumerated() {
            let stock = d.buttons[i]
            let x = (rect.minX + (stock.minX - first)).rounded()
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

    private func restore(_ window: NSWindow, _ d: NDTrafficLightDefaults, _ bs: [NSButton], _ container: NSView) {
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
        // AppKit moves the buttons back on its own title bar layouts; the
        // close button's frame change is the one signal every such pass sends.
        if let close = window.standardWindowButton(.closeButton) {
            close.postsFrameChangedNotifications = true
            NotificationCenter.default.addObserver(forName: NSView.frameDidChangeNotification, object: close, queue: .main) { [weak window] _ in
                MainActor.assumeIsolated {
                    guard let window else { return }
                    NDTrafficLights.shared.schedule(window)
                }
            }
        }
        NotificationCenter.default.addObserver(forName: NSWindow.willCloseNotification, object: window, queue: .main) { _ in
            MainActor.assumeIsolated {
                NDTrafficLights.shared.slots[key] = nil
                NDTrafficLights.shared.defaults[key] = nil
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
