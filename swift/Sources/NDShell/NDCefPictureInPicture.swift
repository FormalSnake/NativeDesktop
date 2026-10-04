#if canImport(CCef)
import AppKit
import CCef
import QuartzCore

/// Chromium's picture-in-picture windows, taken over by the host so they read
/// as the system's own: a floating video (`requestPictureInPicture`) and a
/// document picture-in-picture window (`documentPictureInPicture.requestWindow`).
///
/// Both are Views windows of this process with no CEF callback for their look
/// or their motion. The host recognises one as Chromium orders it in, takes
/// away Chromium's frame and controls, rounds and shadows it, and drives it
/// itself: dragged from anywhere at display rate, thrown into the screen corner
/// the release velocity points at, tucked half off an edge, resized from its
/// edges or by pinching with the aspect kept, with controls that show on hover.
/// The page inside is still Chromium's.
@MainActor enum NDCefPictureInPicture {
    enum Kind: String { case video, document }

    private static let viewsWindowClass = "NativeWidgetMacNSWindow"
    private static var controllers: [ObjectIdentifier: NDPipController] = [:]
    private static var installed = false

    /// The view that asked for a document window most recently. Chromium does
    /// not say which browser a picture-in-picture window belongs to, but the
    /// popup callback does, and the window follows it within a few frames.
    private static weak var pendingDocumentOpener: NDCefWebView?

    static var trace: Bool { ProcessInfo.processInfo.environment["ND_PIP_TRACE"] == "1" }

    static func log(_ message: @autoclosure () -> String) {
        guard trace else { return }
        FileHandle.standardError.write("ND_PIP \(message())\n".data(using: .utf8)!)
    }

    /// Windows in motion; while any is, NSCursor set does nothing (see
    /// NDPipController.quietCursor). Swapped in on the first motion only.
    private nonisolated(unsafe) static var cursorHolds = 0
    private static var cursorHoldInstalled = false

    static func holdCursor(_ on: Bool) {
        if on, !cursorHoldInstalled {
            cursorHoldInstalled = true
            if let method = class_getInstanceMethod(NSCursor.self, #selector(NSCursor.set)) {
                typealias Set = @convention(c) (NSCursor, Selector) -> Void
                let original = unsafeBitCast(method_getImplementation(method), to: Set.self)
                let block: @convention(block) (NSCursor) -> Void = { cursor in
                    if cursorHolds == 0 { original(cursor, #selector(NSCursor.set)) }
                }
                method_setImplementation(method, imp_implementationWithBlock(block))
            }
        }
        cursorHolds = max(0, cursorHolds + (on ? 1 : -1))
    }

    static func noteDocumentOpener(_ view: NDCefWebView) {
        pendingDocumentOpener = view
    }

    /// A Chromium picture-in-picture window: a Views window on the floating
    /// level with no parent. Menus, tooltips and bubbles sit higher, or hang off
    /// a parent window, or never take the mouse.
    static func isCandidate(_ window: NSWindow) -> Bool {
        guard window.level == .floating, window.parent == nil, !window.ignoresMouseEvents else { return false }
        return isViewsWindow(window)
    }

    /// NativeWidgetMacNSWindow and its subclasses (the frameless one is what
    /// both picture-in-picture windows are).
    private static func isViewsWindow(_ window: NSWindow) -> Bool {
        let name = NSStringFromClass(type(of: window))
        return name.hasPrefix("NativeWidgetMac") && name.hasSuffix("NSWindow")
    }

    static func controller(for window: NSWindow) -> NDPipController? {
        controllers[ObjectIdentifier(window)]
    }

    /// Wraps the Views window class's ordering so a picture-in-picture window
    /// is restyled before its first frame reaches the screen. The sweep below
    /// is the backstop for one that gets its level after it is ordered in.
    static func install() {
        guard !installed, let cls = NSClassFromString(viewsWindowClass) else { return }
        installed = true
        typealias Order = @convention(c) (NSWindow, Selector, Int, Int) -> Void
        typealias Plain = @convention(c) (NSWindow, Selector, AnyObject?) -> Void
        wrap(cls, #selector(NSWindow.order(_:relativeTo:)), as: Order.self) { original in
            let block: @convention(block) (NSWindow, Int, Int) -> Void = { window, place, other in
                if place != NSWindow.OrderingMode.out.rawValue {
                    MainActor.assumeIsolated { willShow(window) }
                }
                original(window, #selector(NSWindow.order(_:relativeTo:)), place, other)
            }
            return imp_implementationWithBlock(block)
        }
        for selector in [#selector(NSWindow.orderFront(_:)), #selector(NSWindow.makeKeyAndOrderFront(_:))] {
            wrap(cls, selector, as: Plain.self) { original in
                let block: @convention(block) (NSWindow, AnyObject?) -> Void = { window, sender in
                    MainActor.assumeIsolated { willShow(window) }
                    original(window, selector, sender)
                }
                return imp_implementationWithBlock(block)
            }
        }
    }

    private static func wrap<F>(_ cls: AnyClass, _ selector: Selector, as _: F.Type, _ make: (F) -> IMP) {
        guard let method = class_getInstanceMethod(cls, selector) else { return }
        let original = unsafeBitCast(method_getImplementation(method), to: F.self)
        let imp = make(original)
        if !class_addMethod(cls, selector, imp, method_getTypeEncoding(method)) {
            method_setImplementation(method, imp)
        }
    }

    private static func willShow(_ window: NSWindow) {
        guard controllers[ObjectIdentifier(window)] == nil, isCandidate(window) else { return }
        adopt(window, animated: true)
    }

    /// Called from the surface sweep for every window of the process.
    private static var seen: Set<ObjectIdentifier> = []
    static func sweep(_ window: NSWindow) {
        if trace, window.isVisible, window.level != .normal, seen.insert(ObjectIdentifier(window)).inserted {
            log("seen class=\(NSStringFromClass(type(of: window))) level=\(window.level.rawValue) parent=\(window.parent != nil) ignoresMouse=\(window.ignoresMouseEvents) cb=\(window.collectionBehavior.rawValue) title=\(window.title) style=\(window.styleMask.rawValue) \(window.frame)")
        }
        guard window.isVisible, controllers[ObjectIdentifier(window)] == nil, isCandidate(window) else { return }
        adopt(window, animated: false)
    }

    /// Drops the controllers whose window Chromium closed (the page left
    /// picture-in-picture, the opener tab closed, or the window's own close).
    static func sweepGone() {
        for (key, controller) in controllers where !controller.isAlive {
            controller.finish()
            controllers.removeValue(forKey: key)
        }
    }

    static func releaseAll() {
        for controller in controllers.values { controller.finish() }
        controllers.removeAll()
    }

    private static func adopt(_ window: NSWindow, animated: Bool) {
        let kind: Kind = containsWebContents(window.contentView) ? .document : .video
        let opener: NDCefWebView? = kind == .document ? pendingDocumentOpener : nil
        if kind == .document { pendingDocumentOpener = nil }
        let controller = NDPipController(window: window, kind: kind, opener: opener)
        controllers[ObjectIdentifier(window)] = controller
        controller.adopt(animated: animated)
    }

    /// A document window hosts a full web contents view; the floating video is
    /// drawn by Views alone.
    private static func containsWebContents(_ view: NSView?) -> Bool {
        guard let view else { return false }
        if NSStringFromClass(type(of: view)).contains("WebContentsViewCocoa") { return true }
        return view.subviews.contains { containsWebContents($0) }
    }
}

// MARK: - Motion

/// One axis of a critically damped (or softer) spring, in Apple's terms:
/// `response` is the period of the undamped oscillation in seconds, `damping`
/// the ratio (1 settles without overshoot). Stepped at a fixed small dt so a
/// long frame cannot make it unstable.
struct NDPipSpring {
    var value: Double
    var velocity: Double
    var target: Double
    var response: Double
    var damping: Double

    mutating func advance(_ dt: Double) {
        let stiffness = pow(2 * .pi / response, 2)
        let friction = 4 * .pi * damping / response
        var left = dt
        while left > 0 {
            let step = min(left, 1.0 / 480)
            let force = -stiffness * (value - target) - friction * velocity
            velocity += force * step
            value += velocity * step
            left -= step
        }
    }

    var settled: Bool { abs(value - target) < 0.25 && abs(velocity) < 4 }
}

/// Frame pacing of one drag or one flight, reported in the trace so a drive can
/// hold it to the display's rate.
struct NDPipFrameStats {
    private(set) var intervals: [Double] = []
    private var last: CFTimeInterval = 0

    private(set) var workMax: Double = 0

    mutating func work(_ seconds: Double) { workMax = max(workMax, seconds) }

    mutating func tick(_ at: CFTimeInterval) {
        if last > 0 { intervals.append(at - last) }
        last = at
    }

    func summary(_ phase: String, refresh: Double) -> String {
        guard !intervals.isEmpty else { return "phase=\(phase) frames=0" }
        let ms = intervals.map { $0 * 1000 }.sorted()
        let mean = ms.reduce(0, +) / Double(ms.count)
        let p95 = ms[min(ms.count - 1, Int(Double(ms.count) * 0.95))]
        let budget = 1000 / refresh
        let late = ms.filter { $0 > budget * 1.5 }.count
        return String(
            format: "phase=%@ frames=%d refreshHz=%.0f meanMs=%.2f p95Ms=%.2f maxMs=%.2f late=%d moveMaxMs=%.2f",
            phase, ms.count + 1, refresh, mean, p95, ms.last ?? 0, late, workMax * 1000)
    }
}

/// Apple's momentum projection (Designing Fluid Interfaces): where a throw at
/// `velocity` points per second comes to rest under scroll-like deceleration.
func ndPipProject(_ velocity: Double, deceleration: Double = 0.998) -> Double {
    (velocity / 1000) * deceleration / (1 - deceleration)
}

// MARK: - Controller

@MainActor final class NDPipController: NSObject {
    let kind: NDCefPictureInPicture.Kind
    private weak var window: NSWindow?
    private weak var opener: NDCefWebView?
    private var overlay: NDPipOverlay?
    private var link: CADisplayLink?
    private var monitors: [Any] = []

    /// Where the window rests: a corner of a screen's visible frame, or tucked
    /// past its left or right edge with only the tab showing.
    enum Rest: Equatable {
        case corner(top: Bool, right: Bool)
        case stashed(right: Bool)
    }
    private(set) var rest: Rest = .corner(top: false, right: true)

    private enum Mode {
        case idle
        case dragging(grab: NSPoint, samples: [(t: CFTimeInterval, p: NSPoint)])
        case flying
        case resizing(anchor: NSPoint, aspect: Double, grabbedRight: Bool, grabbedTop: Bool)
        case pinching(anchorTop: Bool, anchorRight: Bool)
    }
    private var mode: Mode = .idle
    private var springX: NDPipSpring?
    private var springY: NDPipSpring?
    private var springW: NDPipSpring?
    private var stats = NDPipFrameStats()
    private var statsPhase = ""
    private var aspect: Double = 16.0 / 9.0
    private var fadeIn: (start: CFTimeInterval, duration: Double)?
    private var closingAfterFlight = false

    static let margin: CGFloat = 16
    static let tab: CGFloat = 28
    static let cornerRadius: CGFloat = 12

    init(window: NSWindow, kind: NDCefPictureInPicture.Kind, opener: NDCefWebView?) {
        self.window = window
        self.kind = kind
        self.opener = opener
        super.init()
    }

    var isAlive: Bool { window?.isVisible == true }

    /// ND_PIP_REDUCE_MOTION=1 stands in for the system setting in a drive,
    /// which must not change the owner's accessibility preferences.
    private var reduceMotion: Bool {
        NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
            || ProcessInfo.processInfo.environment["ND_PIP_REDUCE_MOTION"] == "1"
    }

    func adopt(animated: Bool) {
        guard let window, let frameView = window.contentView?.superview ?? window.contentView else { return }
        window.animationBehavior = .none
        window.isOpaque = false
        window.backgroundColor = .clear
        window.hasShadow = true
        window.collectionBehavior.insert([.canJoinAllSpaces, .fullScreenAuxiliary])
        frameView.wantsLayer = true
        frameView.layer?.cornerRadius = Self.cornerRadius
        frameView.layer?.cornerCurve = .continuous
        frameView.layer?.masksToBounds = true
        if window.frame.height > 0 { aspect = window.frame.width / window.frame.height }

        if kind == .video { silenceChromiumControls(in: window) }
        // Views puts a behind-window NSVisualEffectView under a document
        // window's contents. The page covers it, so it shows nothing, but the
        // window server still blurs what is behind the window on every frame
        // the window moves, and that drops frames.
        for case let effect as NSVisualEffectView in frameView.subviews {
            effect.isHidden = true
        }

        let overlay = NDPipOverlay(controller: self, kind: kind)
        overlay.frame = frameView.bounds
        overlay.autoresizingMask = [.width, .height]
        frameView.addSubview(overlay, positioned: .above, relativeTo: nil)
        self.overlay = overlay
        installMonitors()
        rest = nearestCorner(to: window.frame)

        NDCefPictureInPicture.log("adopted kind=\(kind.rawValue) id=\(window.windowNumber) frame=\(window.frame) opener=\(opener != nil)")
        if kind == .video {
            findVideoOpener { [weak self] source in
                guard let self else { return }
                if self.opener != nil { self.openerReady() }
                self.animateOpen(from: animated ? source : nil)
            }
            if animated { window.alphaValue = 0 }
        } else {
            openerReady()
            animateOpen(from: animated ? openerSourceRect() : nil)
        }
        window.invalidateShadow()
    }

    /// Chromium shows its own controls over the video when the pointer enters
    /// the window or the window takes the keyboard (VideoOverlayWindowViews
    /// OnMouseEvent and OnNativeFocus). The pointer reaches Views through the
    /// tracking area BridgedContentView adds once, as it joins the window, and
    /// through mouse-moved events sent to the first responder; both are taken
    /// away. Focus is kept off by a subclass of this one window's class that
    /// refuses key status, the way KVO swaps an observed object's class.
    private func silenceChromiumControls(in window: NSWindow) {
        func strip(_ view: NSView) {
            if NSStringFromClass(type(of: view)).contains("BridgedContentView") {
                for area in view.trackingAreas { view.removeTrackingArea(area) }
            }
            view.subviews.forEach(strip)
        }
        if let root = window.contentView?.superview ?? window.contentView { strip(root) }
        window.acceptsMouseMovedEvents = false
        Self.refuseKey(window)
    }

    private static var noKeyClasses: [String: AnyClass] = [:]

    private static func refuseKey(_ window: NSWindow) {
        let base: AnyClass = type(of: window)
        let baseName = NSStringFromClass(base)
        guard !baseName.hasPrefix("NDPipNoKey_") else { return }
        let cls: AnyClass
        if let made = noKeyClasses[baseName] {
            cls = made
        } else {
            guard let made = objc_allocateClassPair(base, "NDPipNoKey_\(baseName)", 0) else { return }
            let no: @convention(block) (NSWindow) -> Bool = { _ in false }
            let imp = imp_implementationWithBlock(no)
            for selector in [#selector(getter: NSWindow.canBecomeKey), #selector(getter: NSWindow.canBecomeMain)] {
                let types = method_getTypeEncoding(class_getInstanceMethod(base, selector)!)
                class_addMethod(made, selector, imp, types)
            }
            objc_registerClassPair(made)
            noKeyClasses[baseName] = made
            cls = made
        }
        object_setClass(window, cls)
    }

    func finish() {
        link?.invalidate()
        link = nil
        quietCursor(false)
        for monitor in monitors { NSEvent.removeMonitor(monitor) }
        monitors.removeAll()
        opener?.emitData("pictureInPicture", ["state": "closed", "kind": kind.rawValue])
        NDCefPictureInPicture.log("closed kind=\(kind.rawValue)")
    }

    private var announced = false

    private func openerReady() {
        guard !announced, let opener else { return }
        announced = true
        opener.emitData("pictureInPicture", ["state": "opened", "kind": kind.rawValue])
    }

    // MARK: Opener and source rect

    /// The floating video has no popup callback: the tab that owns it is the
    /// one whose document reports a picture-in-picture element. The answer also
    /// carries where the video sits, which is where the window grows out of.
    /// Chromium shows the window before the page's promise settles, so the
    /// element can be missing for the first few frames: asked again until
    /// `deadline`, and `done` gets nil once the window cannot wait any longer.
    private func findVideoOpener(attempt: Int = 0, _ done: @escaping (NSRect?) -> Void) {
        let views = NDCefWebView.liveViews.allObjects
        var pending = views.count
        let script = """
            (() => { const v = document.pictureInPictureElement; if (!v) return ''; \
            const r = v.getBoundingClientRect(); return JSON.stringify([r.x, r.y, r.width, r.height]); })()
            """
        let retry: @MainActor () -> Void = { [weak self] in
            guard let self, self.isAlive || attempt == 0 else { return }
            // Three tries fit inside the open; after that the window opens
            // without a source and the lookup goes on for the controls.
            if attempt == 2 { done(nil) }
            guard attempt < 20 else { return }
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                MainActor.assumeIsolated {
                    self.findVideoOpener(attempt: attempt + 1, attempt >= 2 ? { _ in self.openerReady() } : done)
                }
            }
        }
        if views.isEmpty { return retry() }
        var found = false
        for view in views {
            view.devTools.evaluate(script, world: "") { [weak self, weak view] value, _ in
                MainActor.assumeIsolated {
                    guard let self, !found else { return }
                    pending -= 1
                    if let value, !value.isEmpty, let view {
                        found = true
                        self.opener = view
                        done(Self.screenRect(cssRect: value, in: view))
                    } else if pending == 0 {
                        retry()
                    }
                }
            }
        }
    }

    private static func screenRect(cssRect json: String, in view: NDCefWebView) -> NSRect? {
        guard let data = json.data(using: .utf8),
            let parts = try? JSONSerialization.jsonObject(with: data) as? [Double], parts.count == 4,
            let hostWindow = view.window
        else { return nil }
        // CSS pixels are view points at 100% zoom; the zoom factor is what the
        // page's own layout already folded in, close enough for where a window
        // grows from.
        let x = parts[0], y = parts[1], w = parts[2], h = parts[3]
        let local = NSRect(x: x, y: view.isFlipped ? y : view.bounds.height - y - h, width: w, height: h)
        return hostWindow.convertToScreen(view.convert(local, to: nil))
    }

    /// A document window has no element to come out of; it grows out of the
    /// middle of the page that opened it.
    private func openerSourceRect() -> NSRect? {
        guard let view = opener, let hostWindow = view.window, let window else { return nil }
        let page = hostWindow.convertToScreen(view.convert(view.bounds, to: nil))
        let w = min(page.width * 0.5, window.frame.width)
        let h = w / max(aspect, 0.1)
        return NSRect(x: page.midX - w / 2, y: page.midY - h / 2, width: w, height: h)
    }

    // MARK: Open and close

    /// The window opens into the corner nearest where Chromium put it, at the
    /// host's margins, and grows out of `source` on the way.
    private func animateOpen(from source: NSRect?) {
        guard let window, let screen = screen(for: NSPoint(x: window.frame.midX, y: window.frame.midY)) else { return }
        rest = nearestCorner(to: window.frame)
        let final = NSRect(origin: origin(for: rest, size: window.frame.size, on: screen), size: window.frame.size)
        if reduceMotion || source == nil {
            window.setFrame(final, display: false)
            window.alphaValue = 0
            fadeIn = (CACurrentMediaTime(), 0.2)
            mode = .flying
            springX = NDPipSpring(value: final.minX, velocity: 0, target: final.minX, response: 0.4, damping: 1)
            springY = NDPipSpring(value: final.minY, velocity: 0, target: final.minY, response: 0.4, damping: 1)
            beginStats("open")
            startLink()
            return
        }
        let start = source!
        window.setFrame(start, display: false)
        window.alphaValue = 0
        fadeIn = (CACurrentMediaTime(), 0.12)
        springW = NDPipSpring(value: start.width, velocity: 0, target: final.width, response: 0.42, damping: 1)
        springX = NDPipSpring(value: start.minX, velocity: 0, target: final.minX, response: 0.42, damping: 1)
        springY = NDPipSpring(value: start.minY, velocity: 0, target: final.minY, response: 0.42, damping: 1)
        mode = .flying
        beginStats("open")
        startLink()
    }

    /// The close button and back-to-tab. The window shrinks back into where it
    /// came from before Chromium closes it.
    func close(returningToTab: Bool) {
        guard let window else { return }
        if returningToTab {
            opener?.emitData("pictureInPicture", ["state": "returnToTab", "kind": kind.rawValue])
            if let host = opener?.window {
                NSApp.activate()
                host.makeKeyAndOrderFront(nil)
            }
        }
        // Through the page's own API on the opener, which is what Chromium's
        // own buttons amount to: a closed document window fires pagehide, and
        // the video's close pauses it as Chrome's does.
        let script: String
        switch (kind, returningToTab) {
        case (.document, _):
            script = "(() => { documentPictureInPicture.window?.close(); return 'ok'; })()"
        case (.video, true):
            script = "document.exitPictureInPicture().then(() => 'ok')"
        case (.video, false):
            script = "(async () => { const v = document.pictureInPictureElement; await document.exitPictureInPicture(); v?.pause(); return 'ok'; })()"
        }
        let closeNow: @MainActor () -> Void = { [weak self] in
            guard let self else { return }
            guard let opener = self.opener else {
                NDCefPictureInPicture.log("close without an opener")
                return
            }
            opener.devTools.evaluate(script, world: "") { _, error in
                if let error { NDCefPictureInPicture.log("close failed \(error)") }
            }
        }
        if reduceMotion {
            NSAnimationContext.runAnimationGroup({ context in
                context.duration = 0.18
                window.animator().alphaValue = 0
            }, completionHandler: { MainActor.assumeIsolated { closeNow() } })
            return
        }
        let target = (kind == .video ? nil : openerSourceRect()) ?? window.frame.insetBy(dx: window.frame.width * 0.2, dy: window.frame.height * 0.2)
        closingAfterFlight = true
        closeAction = closeNow
        springW = NDPipSpring(value: window.frame.width, velocity: 0, target: target.width, response: 0.32, damping: 1)
        springX = NDPipSpring(value: window.frame.minX, velocity: 0, target: target.minX, response: 0.32, damping: 1)
        springY = NDPipSpring(value: window.frame.minY, velocity: 0, target: target.minY, response: 0.32, damping: 1)
        mode = .flying
        beginStats("close")
        startLink()
    }
    private var closeAction: (@MainActor () -> Void)?

    /// Play and pause go to the page's own element, the one the floating
    /// window shows.
    func togglePlay(_ done: @escaping (Bool?) -> Void) {
        guard let opener else { return done(nil) }
        let script = """
            (() => { const v = document.pictureInPictureElement; if (!v) return ''; \
            if (v.paused) v.play(); else v.pause(); return String(v.paused); })()
            """
        opener.devTools.evaluate(script, world: "") { value, _ in
            MainActor.assumeIsolated { done(value.map { $0 == "true" }) }
        }
    }

    func readPaused(_ done: @escaping (Bool?) -> Void) {
        guard let opener else { return done(nil) }
        opener.devTools.evaluate("String(document.pictureInPictureElement?.paused ?? '')", world: "") { value, _ in
            MainActor.assumeIsolated { done(value.flatMap { $0.isEmpty ? nil : $0 == "true" }) }
        }
    }

    // MARK: Geometry

    private func screen(for point: NSPoint) -> NSScreen? {
        NSScreen.screens.first { $0.frame.contains(point) }
            ?? NSScreen.screens.min { distance(point, to: $0.frame) < distance(point, to: $1.frame) }
    }

    private func distance(_ p: NSPoint, to r: NSRect) -> CGFloat {
        let dx = max(r.minX - p.x, 0, p.x - r.maxX)
        let dy = max(r.minY - p.y, 0, p.y - r.maxY)
        return hypot(dx, dy)
    }

    private func origin(for rest: Rest, size: NSSize, on screen: NSScreen, y: CGFloat? = nil) -> NSPoint {
        let v = screen.visibleFrame
        switch rest {
        case let .corner(top, right):
            return NSPoint(
                x: right ? v.maxX - size.width - Self.margin : v.minX + Self.margin,
                y: top ? v.maxY - size.height - Self.margin : v.minY + Self.margin)
        case let .stashed(right):
            let yy = min(max(y ?? v.minY + Self.margin, v.minY + Self.margin), v.maxY - size.height - Self.margin)
            return NSPoint(x: right ? v.maxX - Self.tab : v.minX - size.width + Self.tab, y: yy)
        }
    }

    private func nearestCorner(to frame: NSRect) -> Rest {
        guard let screen = screen(for: NSPoint(x: frame.midX, y: frame.midY)) else { return .corner(top: false, right: true) }
        let v = screen.visibleFrame
        return .corner(top: frame.midY > v.midY, right: frame.midX > v.midX)
    }

    /// Where a release at `origin` moving at `velocity` comes to rest. The
    /// momentum is projected forward at scroll deceleration and the corner the
    /// projected centre lands nearest wins, so a flick reaches the far corner.
    /// Tucking past an edge takes more: the centre has to cross the edge under
    /// a much shorter projection, a push at the edge rather than any fast throw
    /// toward that side.
    private func restFor(origin: NSPoint, size: NSSize, velocity: CGVector) -> (Rest, NSScreen?) {
        let centre = NSPoint(x: origin.x + size.width / 2, y: origin.y + size.height / 2)
        let projected = NSPoint(x: centre.x + ndPipProject(velocity.dx), y: centre.y + ndPipProject(velocity.dy))
        let pushed = centre.x + ndPipProject(velocity.dx, deceleration: 0.99)
        guard let screen = screen(for: centre) ?? screen(for: projected) else {
            return (.corner(top: false, right: true), nil)
        }
        let v = screen.visibleFrame
        if pushed < v.minX { return (.stashed(right: false), screen) }
        if pushed > v.maxX { return (.stashed(right: true), screen) }
        return (.corner(top: projected.y > v.midY, right: projected.x > v.midX), screen)
    }

    // MARK: Drag

    /// The pointer is read at display rate rather than per mouse event, so the
    /// window moves once per frame with the newest position.
    private var pressedAt: NSPoint = .zero

    func beginDrag(at mouse: NSPoint) {
        guard let window else { return }
        stopFlight()
        let o = window.frame.origin
        pressedAt = mouse
        mode = .dragging(grab: NSPoint(x: mouse.x - o.x, y: mouse.y - o.y), samples: [(CACurrentMediaTime(), o)])
        beginStats("drag")
        overlay?.setDragging(true)
        startLink()
    }

    func endDrag() {
        guard let window, case let .dragging(_, samples) = mode else { return }
        overlay?.setDragging(false)
        let travelled = hypot(NSEvent.mouseLocation.x - pressedAt.x, NSEvent.mouseLocation.y - pressedAt.y)
        if travelled < 3, case .stashed = rest {
            mode = .idle
            reportStats()
            reveal()
            return
        }
        // Release velocity from the last ~80 ms of display-rate positions.
        let now = CACurrentMediaTime()
        let recent = samples.filter { now - $0.t < 0.08 }
        var velocity = CGVector.zero
        if let first = recent.first, let last = recent.last, last.t - first.t > 0.008 {
            velocity = CGVector(dx: (last.p.x - first.p.x) / (last.t - first.t), dy: (last.p.y - first.p.y) / (last.t - first.t))
        }
        reportStats()
        let (rest, screen) = restFor(origin: window.frame.origin, size: window.frame.size, velocity: velocity)
        settle(to: rest, on: screen, velocity: velocity, phase: "throw")
    }

    private func settle(to rest: Rest, on screen: NSScreen?, velocity: CGVector, phase: String) {
        guard let window, let screen = screen ?? self.screen(for: NSPoint(x: window.frame.midX, y: window.frame.midY)) else { return }
        self.rest = rest
        overlay?.setStashed(rest)
        let target = origin(for: rest, size: window.frame.size, on: screen, y: window.frame.minY)
        NDCefPictureInPicture.log(String(format: "settle %@ v=%.0f,%.0f to=%@", phase, velocity.dx, velocity.dy, "\(rest)"))
        startFlight(to: target, width: window.frame.width, velocity: velocity, phase: phase)
    }

    /// Reveals a stashed window into the nearest corner on its side.
    func reveal() {
        guard let window, case let .stashed(right) = rest,
            let screen = screen(for: NSPoint(x: window.frame.midX, y: window.frame.midY))
        else { return }
        let top = window.frame.midY > screen.visibleFrame.midY
        settle(to: .corner(top: top, right: right), on: screen, velocity: .zero, phase: "reveal")
    }

    func stash(right: Bool) {
        guard let window, let screen = screen(for: NSPoint(x: window.frame.midX, y: window.frame.midY)) else { return }
        settle(to: .stashed(right: right), on: screen, velocity: .zero, phase: "stash")
    }

    private func startFlight(to target: NSPoint, width: CGFloat, velocity: CGVector, phase: String) {
        guard let window else { return }
        let f = window.frame
        // A throw keeps its own speed into the spring; damping 1 is Apple's
        // value for repositioning picture in picture.
        springX = NDPipSpring(value: f.minX, velocity: velocity.dx, target: target.x, response: 0.4, damping: 1)
        springY = NDPipSpring(value: f.minY, velocity: velocity.dy, target: target.y, response: 0.4, damping: 1)
        springW = NDPipSpring(value: f.width, velocity: 0, target: width, response: 0.4, damping: 1)
        if reduceMotion, phase != "open" {
            window.setFrame(NSRect(x: target.x, y: target.y, width: width, height: width / aspect), display: true)
            springX = nil
            springY = nil
            springW = nil
            mode = .idle
            return
        }
        mode = .flying
        beginStats(phase)
        startLink()
    }

    private func stopFlight() {
        springX = nil
        springY = nil
        springW = nil
        if case .flying = mode { reportStats() }
        mode = .idle
    }

    // MARK: Resize and pinch

    func beginResize(at mouse: NSPoint) {
        guard let window else { return }
        stopFlight()
        let f = window.frame
        let right = mouse.x > f.midX
        let top = mouse.y > f.midY
        let anchor = NSPoint(x: right ? f.minX : f.maxX, y: top ? f.minY : f.maxY)
        mode = .resizing(anchor: anchor, aspect: f.width / max(f.height, 1), grabbedRight: right, grabbedTop: top)
    }

    func resize(to mouse: NSPoint) {
        guard let window, case let .resizing(anchor, aspect, right, top) = mode else { return }
        let width = clampWidth(max(abs(mouse.x - anchor.x), abs(mouse.y - anchor.y) * aspect), rubber: true)
        let height = width / aspect
        window.setFrame(
            NSRect(x: right ? anchor.x : anchor.x - width, y: top ? anchor.y : anchor.y - height, width: width, height: height),
            display: true)
    }

    func endResize() {
        guard case .resizing = mode, let window else { return }
        mode = .idle
        let width = clampWidth(window.frame.width, rubber: false)
        if width != window.frame.width {
            settleSize(width)
        } else {
            settle(to: nearestCorner(to: window.frame), on: nil, velocity: .zero, phase: "resize")
        }
    }

    func beginPinch() {
        guard let window else { return }
        stopFlight()
        let anchor: (top: Bool, right: Bool)
        switch rest {
        case let .corner(top, right): anchor = (top, right)
        case let .stashed(right): anchor = (false, right)
        }
        _ = window
        mode = .pinching(anchorTop: anchor.top, anchorRight: anchor.right)
    }

    /// Grows away from the corner the window is docked in, so the corner stays
    /// put under the fingers' side of the screen.
    func pinch(by magnification: CGFloat) {
        guard let window, case let .pinching(top, right) = mode else { return }
        let f = window.frame
        let width = clampWidth(f.width * (1 + magnification), rubber: true)
        let height = width / aspect
        let x = right ? f.maxX - width : f.minX
        let y = top ? f.maxY - height : f.minY
        window.setFrame(NSRect(x: x, y: y, width: width, height: height), display: true)
    }

    func endPinch() {
        guard case .pinching = mode, let window else { return }
        mode = .idle
        settleSize(snappedWidth(window.frame.width))
    }

    /// The sizes a pinch settles on, as on iPad: three steps of the screen's
    /// width, the smallest never under 240 points.
    func snapWidths() -> [CGFloat] {
        guard let window, let screen = screen(for: NSPoint(x: window.frame.midX, y: window.frame.midY)) else { return [320, 480, 640] }
        let w = screen.visibleFrame.width
        return [max(240, w * 0.18), max(320, w * 0.26), max(400, w * 0.38)]
    }

    private func snappedWidth(_ width: CGFloat) -> CGFloat {
        snapWidths().min { abs(log($0 / width)) < abs(log($1 / width)) } ?? width
    }

    private func clampWidth(_ width: CGFloat, rubber: Bool) -> CGFloat {
        let sizes = snapWidths()
        let lo = sizes.first! * 0.75
        let hi = sizes.last! * 1.25
        guard rubber else { return min(max(width, lo), hi) }
        // Past either bound the window keeps following, ever more slowly.
        func band(_ over: CGFloat, _ dimension: CGFloat) -> CGFloat {
            (over * dimension * 0.55) / (dimension + 0.55 * abs(over))
        }
        if width < lo { return lo - band(lo - width, lo) }
        if width > hi { return hi + band(width - hi, hi) }
        return width
    }

    private func settleSize(_ width: CGFloat) {
        guard let window, let screen = screen(for: NSPoint(x: window.frame.midX, y: window.frame.midY)) else { return }
        let size = NSSize(width: width, height: width / aspect)
        let corner = nearestCorner(to: window.frame)
        rest = corner
        overlay?.setStashed(corner)
        NDCefPictureInPicture.log(String(format: "settle size w=%.0f", width))
        startFlight(to: origin(for: corner, size: size, on: screen), width: width, velocity: .zero, phase: "size")
    }

    // MARK: Display link

    /// While a window moves under the pointer AppKit re-resolves the cursor on
    /// every frame and every mouse event, and each of those ends in an
    /// NSCursor set, even with nothing to change. With an accessibility pointer
    /// size or colour, a set renders a new cursor image on the main thread,
    /// measured at 20 to 60 ms per frame on a document window (whose page
    /// keeps a cursor area). For as long as a picture-in-picture window is in
    /// motion the cursor is held as it is (NDCefPictureInPicture.holdCursor);
    /// nothing under a moving window should change it anyway.
    private var cursorQuiet = false

    private func quietCursor(_ on: Bool) {
        guard on != cursorQuiet else { return }
        cursorQuiet = on
        NDCefPictureInPicture.holdCursor(on)
    }

    private func startLink() {
        quietCursor(true)
        guard link == nil, let overlay else { return }
        let link = overlay.displayLink(target: self, selector: #selector(tick(_:)))
        link.preferredFrameRateRange = CAFrameRateRange(minimum: 80, maximum: 120, preferred: 120)
        link.add(to: .main, forMode: .common)
        self.link = link
    }

    private var lastTick: CFTimeInterval = 0

    @objc private func tick(_ link: CADisplayLink) {
        guard let window else {
            link.invalidate()
            self.link = nil
            return
        }
        let now = link.timestamp
        let dt = lastTick > 0 ? min(now - lastTick, 1.0 / 30) : link.duration
        lastTick = now
        stats.tick(now)
        if let fade = fadeIn {
            let p = min(1, (CACurrentMediaTime() - fade.start) / fade.duration)
            window.alphaValue = p
            if p >= 1 { fadeIn = nil }
        }
        switch mode {
        case let .dragging(grab, samples):
            let mouse = NSEvent.mouseLocation
            let o = NSPoint(x: mouse.x - grab.x, y: mouse.y - grab.y)
            let started = CACurrentMediaTime()
            if o != window.frame.origin { window.setFrameOrigin(o) }
            stats.work(CACurrentMediaTime() - started)
            var kept = samples.filter { now - $0.t < 0.15 }
            kept.append((now, o))
            mode = .dragging(grab: grab, samples: kept)
        case .flying:
            guard var x = springX, var y = springY else { break }
            x.advance(dt)
            y.advance(dt)
            springX = x
            springY = y
            var frame = window.frame
            if var w = springW {
                w.advance(dt)
                springW = w
                frame.size = NSSize(width: w.value, height: w.value / aspect)
            }
            frame.origin = NSPoint(x: x.value, y: y.value)
            let started = CACurrentMediaTime()
            if frame.size == window.frame.size {
                window.setFrameOrigin(frame.origin)
            } else {
                window.setFrame(frame, display: false)
            }
            stats.work(CACurrentMediaTime() - started)
            if x.settled, y.settled, springW?.settled ?? true, fadeIn == nil {
                springX = nil
                springY = nil
                springW = nil
                mode = .idle
                reportStats()
                window.invalidateShadow()
                if closingAfterFlight {
                    closingAfterFlight = false
                    window.alphaValue = 0
                    closeAction?()
                    closeAction = nil
                }
            }
        default:
            break
        }
        if case .idle = mode, fadeIn == nil {
            link.invalidate()
            self.link = nil
            lastTick = 0
            quietCursor(false)
        }
    }

    private func beginStats(_ phase: String) {
        if !statsPhase.isEmpty { reportStats() }
        stats = NDPipFrameStats()
        statsPhase = phase
    }

    private func reportStats() {
        guard !statsPhase.isEmpty else { return }
        let refresh = Double(window?.screen?.maximumFramesPerSecond ?? 60)
        NDCefPictureInPicture.log("frames \(stats.summary(statsPhase, refresh: refresh))")
        statsPhase = ""
    }

    // MARK: Events Chromium would otherwise get

    /// A pinch on a document window would zoom the page, and a drag of the
    /// page's own draggable region moves the window without a landing: both
    /// are taken here, for this window only.
    private func installMonitors() {
        let magnify = NSEvent.addLocalMonitorForEvents(matching: [.magnify]) { [weak self] event in
            guard let self, let window = MainActor.assumeIsolated({ self.window }), event.window === window else { return event }
            MainActor.assumeIsolated { self.handleMagnify(event) }
            return nil
        }
        let up = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .leftMouseUp]) { [weak self] event in
            guard let self, let window = MainActor.assumeIsolated({ self.window }), event.window === window else { return event }
            let down = event.type == .leftMouseDown
            MainActor.assumeIsolated { self.pressed(down: down) }
            return event
        }
        monitors = [magnify, up].compactMap { $0 }
    }
    private var pressOrigin: NSPoint?

    private func pressed(down: Bool) {
        guard let window else { return }
        if down {
            pressOrigin = window.frame.origin
            return
        }
        guard case .idle = mode, let from = pressOrigin, from != window.frame.origin else { return }
        // Chromium moved it (the page's own draggable region): land it in a
        // corner as if it had been dropped there.
        pressOrigin = nil
        DispatchQueue.main.async {
            MainActor.assumeIsolated {
                guard let window = self.window else { return }
                let (rest, screen) = self.restFor(origin: window.frame.origin, size: window.frame.size, velocity: .zero)
                self.settle(to: rest, on: screen, velocity: .zero, phase: "landing")
            }
        }
    }

    func handleMagnify(_ event: NSEvent) {
        switch event.phase {
        case .began: beginPinch()
        case .changed: pinch(by: event.magnification)
        case .ended, .cancelled: endPinch()
        default:
            // A magnify without phases (a mouse with a zoom gesture): one step.
            beginPinch()
            pinch(by: event.magnification)
            endPinch()
        }
    }

    // Two-finger horizontal swipes stash and reveal, with the swipe's momentum.
    private var swipeDX: CGFloat = 0
    private var swipeVelocity: CGFloat = 0
    private var swipeLast: CFTimeInterval = 0

    func handleScroll(_ event: NSEvent) {
        guard event.hasPreciseScrollingDeltas else { return }
        let now = event.timestamp
        if event.phase == .began {
            swipeDX = 0
            swipeVelocity = 0
        }
        if event.phase == .changed || event.phase == .began {
            // Which way the fingers went: a natural-scrolling trackpad reports
            // the content's direction, which is the fingers'.
            let dx = event.isDirectionInvertedFromDevice ? event.scrollingDeltaX : -event.scrollingDeltaX
            swipeDX += dx
            if swipeLast > 0, now > swipeLast { swipeVelocity = dx / (now - swipeLast) }
        }
        swipeLast = now
        guard event.phase == .ended || event.momentumPhase == .began else { return }
        let projected = swipeDX + CGFloat(ndPipProject(Double(swipeVelocity)))
        NDCefPictureInPicture.log(String(format: "swipe dx=%.0f v=%.0f projected=%.0f", swipeDX, swipeVelocity, projected))
        guard abs(projected) > 80 else { return }
        swipeDX = 0
        let right = projected > 0
        switch rest {
        case let .stashed(stashedRight) where stashedRight != right: reveal()
        case .corner(_, let cornerRight) where cornerRight == right: stash(right: right)
        default: break
        }
    }
}

// MARK: - Overlay

/// The host's layer over Chromium's window: it takes the pointer for the video
/// (so Chromium's own controls never show), carries the hover controls, the
/// stash tab and, on a document window, the grabber.
@MainActor final class NDPipOverlay: NSView {
    private weak var controller: NDPipController?
    private let kind: NDCefPictureInPicture.Kind
    private let controls = NSView()
    private let closeButton: NDPipButton
    private let returnButton: NDPipButton
    private let playButton: NDPipButton?
    private let grabber = NSView()
    /// Keeps the white controls legible over a light video or page.
    private let scrim = CAGradientLayer()
    private let centreScrim = CAGradientLayer()
    /// A plain dim rather than a material: a material cannot blur Chromium's
    /// content, which is a remote layer.
    private let stashCover = NSView()
    private let stashChevron = NSImageView()
    private var stashed: NDPipController.Rest = .corner(top: false, right: true)
    private var hovering = false
    private var dragging = false
    private var pressedButton: NDPipButton?
    private var hideWork: DispatchWorkItem?

    static let edge: CGFloat = 6
    private var edgeCursor: NSCursor.FrameResizePosition?

    init(controller: NDPipController, kind: NDCefPictureInPicture.Kind) {
        self.controller = controller
        self.kind = kind
        closeButton = NDPipButton(symbol: "xmark", label: "Close", size: 28)
        returnButton = NDPipButton(symbol: "pip.exit", label: "Back to Tab", size: 28)
        playButton = kind == .video ? NDPipButton(symbol: "pause.fill", label: "Pause", size: 44) : nil
        super.init(frame: .zero)
        wantsLayer = true
        setAccessibilityIdentifier("nd-pip-overlay")

        stashCover.wantsLayer = true
        stashCover.layer?.backgroundColor = NSColor.black.withAlphaComponent(0.45).cgColor
        stashCover.alphaValue = 0
        stashCover.autoresizingMask = [.width, .height]
        addSubview(stashCover)
        stashChevron.symbolConfiguration = .init(pointSize: 15, weight: .semibold)
        stashChevron.contentTintColor = .white
        stashCover.addSubview(stashChevron)

        controls.wantsLayer = true
        controls.alphaValue = 0
        controls.autoresizingMask = [.width, .height]
        addSubview(controls)
        for button in [closeButton, returnButton] + (playButton.map { [$0] } ?? []) {
            controls.addSubview(button)
        }
        scrim.colors = [NSColor.black.withAlphaComponent(0.38).cgColor, NSColor.black.withAlphaComponent(0).cgColor]
        scrim.startPoint = CGPoint(x: 0.5, y: 1)
        scrim.endPoint = CGPoint(x: 0.5, y: 0)
        controls.layer?.addSublayer(scrim)
        if kind == .video {
            // The play button sits in the middle; a soft pool behind it.
            centreScrim.type = .radial
            centreScrim.colors = [NSColor.black.withAlphaComponent(0.25).cgColor, NSColor.black.withAlphaComponent(0).cgColor]
            centreScrim.startPoint = CGPoint(x: 0.5, y: 0.5)
            centreScrim.endPoint = CGPoint(x: 1, y: 1)
            controls.layer?.addSublayer(centreScrim)
        }
        if kind == .document {
            grabber.wantsLayer = true
            grabber.layer?.backgroundColor = NSColor.white.withAlphaComponent(0.72).cgColor
            grabber.layer?.cornerRadius = 2.5
            grabber.layer?.cornerCurve = .continuous
            grabber.shadow = {
                let s = NSShadow()
                s.shadowColor = NSColor.black.withAlphaComponent(0.35)
                s.shadowBlurRadius = 3
                return s
            }()
            controls.addSubview(grabber)
        }
        closeButton.setAccessibilityIdentifier("nd-pip-close")
        returnButton.setAccessibilityIdentifier("nd-pip-return")
        playButton?.setAccessibilityIdentifier("nd-pip-play")
    }

    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { false }

    override func layout() {
        super.layout()
        let b = bounds
        let inset: CGFloat = 8
        closeButton.frame.origin = NSPoint(x: inset, y: b.maxY - inset - closeButton.frame.height)
        returnButton.frame.origin = NSPoint(x: closeButton.frame.maxX + 6, y: closeButton.frame.minY)
        if let playButton {
            playButton.frame.origin = NSPoint(x: b.midX - playButton.frame.width / 2, y: b.midY - playButton.frame.height / 2)
        }
        grabber.frame = NSRect(x: b.midX - 18, y: b.maxY - 10, width: 36, height: 5)
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        scrim.frame = CGRect(x: 0, y: b.maxY - 64, width: b.width, height: 64)
        centreScrim.frame = CGRect(x: b.midX - 70, y: b.midY - 70, width: 140, height: 140)
        CATransaction.commit()
        layoutStash()
    }

    private func layoutStash() {
        let b = bounds
        let right: Bool
        if case let .stashed(r) = stashed { right = r } else { right = false }
        stashChevron.image = NSImage(systemSymbolName: right ? "chevron.compact.left" : "chevron.compact.right", accessibilityDescription: "Show")
        let size = NSSize(width: 14, height: 28)
        let x = right ? NDPipController.tab / 2 - size.width / 2 : b.maxX - NDPipController.tab / 2 - size.width / 2
        stashChevron.frame = NSRect(x: x, y: b.midY - size.height / 2, width: size.width, height: size.height)
    }

    // MARK: Hit testing

    /// The video takes every point; a document window only the controls, the
    /// grabber and its edges, so the page under it stays clickable.
    override func hitTest(_ point: NSPoint) -> NSView? {
        let local = convert(point, from: superview)
        guard bounds.contains(local) else { return nil }
        if case .stashed = stashed { return self }
        if controls.alphaValue > 0.01 {
            for button in [closeButton, returnButton] + (playButton.map { [$0] } ?? []) where button.frame.insetBy(dx: -4, dy: -4).contains(local) {
                return self
            }
        }
        if kind == .video { return self }
        if grabberHitRect.contains(local) { return self }
        return nil
    }

    private var grabberHitRect: NSRect {
        NSRect(x: bounds.midX - 44, y: bounds.maxY - 22, width: 88, height: 22)
    }

    private func button(at local: NSPoint) -> NDPipButton? {
        guard controls.alphaValue > 0.01 else { return nil }
        return ([closeButton, returnButton] + (playButton.map { [$0] } ?? [])).first { $0.frame.insetBy(dx: -4, dy: -4).contains(local) }
    }

    private func onEdge(_ local: NSPoint) -> Bool {
        kind == .video && !bounds.insetBy(dx: Self.edge, dy: Self.edge).contains(local)
    }

    private func resizePosition(_ p: NSPoint) -> NSCursor.FrameResizePosition {
        let left = p.x < Self.edge * 3, right = p.x > bounds.width - Self.edge * 3
        let bottom = p.y < Self.edge * 3, top = p.y > bounds.height - Self.edge * 3
        switch (left, right, bottom, top) {
        case (true, _, _, true): return .topLeft
        case (_, true, _, true): return .topRight
        case (true, _, true, _): return .bottomLeft
        case (_, true, true, _): return .bottomRight
        case (true, _, _, _): return .left
        case (_, true, _, _): return .right
        case (_, _, true, _): return .bottom
        default: return .top
        }
    }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        for area in trackingAreas { removeTrackingArea(area) }
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .mouseMoved, .activeAlways, .inVisibleRect, .cursorUpdate], owner: self))
    }

    /// Answered without a set. AppKit resolves the cursor again on every frame
    /// the window moves under a still pointer; a view that answers keeps it
    /// from setting one itself (see NDPipController.quietCursor).
    override func cursorUpdate(with event: NSEvent) {}

    override func mouseEntered(with event: NSEvent) { setHover(true) }
    override func mouseExited(with event: NSEvent) { setHover(false) }

    override func mouseMoved(with event: NSEvent) {
        setHover(true)
        let local = convert(event.locationInWindow, from: nil)
        let edge: NSCursor.FrameResizePosition? = onEdge(local) ? resizePosition(local) : nil
        // Set only on a change: each set is costly under an accessibility
        // pointer (see NDPipController.quietCursor).
        if edge != edgeCursor {
            edgeCursor = edge
            (edge.map { NSCursor.frameResize(position: $0, directions: .all) } ?? NSCursor.arrow).set()
        }
    }

    // MARK: Pointer

    private enum Press { case none, button, drag, resize }
    private var press: Press = .none

    override func mouseDown(with event: NSEvent) {
        let local = convert(event.locationInWindow, from: nil)
        NDCefPictureInPicture.log("press at \(Int(local.x)),\(Int(local.y)) of \(Int(bounds.width))x\(Int(bounds.height)) \(stateDescription)")
        if let hit = button(at: local), stashedCorner {
            pressedButton = hit
            hit.setPressed(true)
            press = .button
            return
        }
        if onEdge(local), stashedCorner {
            press = .resize
            controller?.beginResize(at: NSEvent.mouseLocation)
            return
        }
        press = .drag
        controller?.beginDrag(at: NSEvent.mouseLocation)
    }

    private var stashedCorner: Bool {
        if case .stashed = stashed { return false }
        return true
    }

    override func mouseDragged(with event: NSEvent) {
        switch press {
        case .button:
            let local = convert(event.locationInWindow, from: nil)
            pressedButton?.setPressed(pressedButton?.frame.insetBy(dx: -10, dy: -10).contains(local) == true)
        case .resize:
            controller?.resize(to: NSEvent.mouseLocation)
        default:
            break
        }
    }

    override func mouseUp(with event: NSEvent) {
        defer {
            press = .none
            pressedButton = nil
        }
        switch press {
        case .button:
            guard let button = pressedButton else { return }
            button.setPressed(false)
            let local = convert(event.locationInWindow, from: nil)
            guard button.frame.insetBy(dx: -10, dy: -10).contains(local) else { return }
            activate(button)
        case .resize:
            controller?.endResize()
        case .drag:
            controller?.endDrag()
        case .none:
            break
        }
    }

    override func scrollWheel(with event: NSEvent) {
        if kind == .video { controller?.handleScroll(event) } else { super.scrollWheel(with: event) }
    }

    override func magnify(with event: NSEvent) {
        controller?.handleMagnify(event)
    }

    private func activate(_ button: NDPipButton) {
        if button === closeButton {
            controller?.close(returningToTab: false)
        } else if button === returnButton {
            controller?.close(returningToTab: true)
        } else if button === playButton {
            controller?.togglePlay { [weak self] paused in
                guard let paused else { return }
                self?.showPaused(paused)
            }
        }
    }

    private func showPaused(_ paused: Bool) {
        playButton?.setSymbol(paused ? "play.fill" : "pause.fill", label: paused ? "Play" : "Pause")
    }

    // MARK: States

    func setDragging(_ on: Bool) {
        dragging = on
        fadeControls(on ? false : hovering)
    }

    func setStashed(_ rest: NDPipController.Rest) {
        stashed = rest
        layoutStash()
        let on: Bool
        if case .stashed = rest { on = true } else { on = false }
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.2
            context.timingFunction = CAMediaTimingFunction(controlPoints: 0.23, 1, 0.32, 1)
            stashCover.animator().alphaValue = on ? 1 : 0
        }
        if on { fadeControls(false) }
    }

    private func setHover(_ on: Bool) {
        hovering = on
        if case .stashed = stashed { return }
        guard !dragging else { return }
        fadeControls(on)
        if on, kind == .video {
            controller?.readPaused { [weak self] paused in
                if let paused { self?.showPaused(paused) }
            }
        }
    }

    private func fadeControls(_ visible: Bool) {
        NDCefPictureInPicture.log("controls \(visible ? "shown" : "hidden")")
        NSAnimationContext.runAnimationGroup { context in
            context.duration = visible ? 0.18 : 0.25
            context.timingFunction = CAMediaTimingFunction(controlPoints: 0.23, 1, 0.32, 1)
            controls.animator().alphaValue = visible ? 1 : 0
        }
    }

    /// For the automation tree: what is on screen, by state.
    var stateDescription: String {
        "controls=\(controls.alphaValue > 0.5) stashed=\(stashedCorner ? "no" : "yes")"
    }
}

/// A round control on a HUD material, the way the system's own floating video
/// draws them. Feedback lands on press.
@MainActor final class NDPipButton: NSVisualEffectView {
    private let icon = NSImageView()

    init(symbol: String, label: String, size: CGFloat) {
        super.init(frame: NSRect(x: 0, y: 0, width: size, height: size))
        material = .hudWindow
        blendingMode = .withinWindow
        state = .active
        wantsLayer = true
        layer?.cornerRadius = size / 2
        layer?.cornerCurve = .continuous
        // The material cannot sample Chromium's content (a remote layer), so
        // over a dark page it is a dark disc on dark: a hairline gives it an
        // edge, as the system's own floating video controls have.
        layer?.borderWidth = 1
        layer?.borderColor = NSColor.white.withAlphaComponent(0.18).cgColor
        icon.symbolConfiguration = .init(pointSize: size * 0.42, weight: .semibold)
        icon.contentTintColor = .white
        icon.frame = bounds
        icon.autoresizingMask = [.width, .height]
        icon.imageScaling = .scaleNone
        addSubview(icon)
        setSymbol(symbol, label: label)
        setAccessibilityRole(.button)
    }

    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    func setSymbol(_ symbol: String, label: String) {
        icon.image = NSImage(systemSymbolName: symbol, accessibilityDescription: label)
        setAccessibilityLabel(label)
    }

    func setPressed(_ on: Bool) {
        guard let layer else { return }
        let scale: CGFloat = on ? 0.9 : 1
        CATransaction.begin()
        CATransaction.setAnimationDuration(0.12)
        CATransaction.setAnimationTimingFunction(CAMediaTimingFunction(controlPoints: 0.23, 1, 0.32, 1))
        layer.anchorPoint = CGPoint(x: 0.5, y: 0.5)
        layer.position = CGPoint(x: frame.midX, y: frame.midY)
        layer.setAffineTransform(CGAffineTransform(scaleX: scale, y: scale))
        CATransaction.commit()
        icon.alphaValue = on ? 0.7 : 1
    }
}
#endif
