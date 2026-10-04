#if canImport(CCef)
import AppKit
import CCef

/// Chromium's own sheets and bubbles, adopted into the app's window.
///
/// Chrome style gives the browser no toolbar, so every Views surface Chromium
/// anchors (the WebAuthn sheet, HTTP auth, the autofill and save-password
/// bubbles) is placed against the bare browser widget. On GTK that widget is an
/// X child window and the surface is painted inside it; on AppKit the widget is
/// a content view lifted out of an invisible anchor window, and each surface
/// becomes an `NSWindow` of its own that follows the anchor rather than the app.
/// No CEF callback is consulted about any of them.
///
/// They are windows of this process, though, so the host can take them: this
/// sweeps the process's window list, recognises a Chromium-drawn surface, makes
/// it a child of the window hosting the browser it belongs to, and puts it where
/// Chrome puts a tab-modal sheet, centred horizontally at the top of the web
/// contents. The surface is still Chromium's and still drawn by Views; what
/// changes is that it moves, hides and dies with the app window.
@MainActor enum NDCefSurfaceWindows {
    /// Chromium's Views windows on macOS are all `NativeWidgetMacNSWindow` or a
    /// subclass of it. The name is the discriminator because there is no header
    /// for the class and nothing else tells a Views window from an AppKit one:
    /// the app's own windows are NSWindow/NSPanel, the anchors are known by
    /// identity, and a menu or a tooltip sits above the normal window level.
    private static let viewsWindowClass = "NativeWidgetMacNSWindow"

    private struct Adopted {
        weak var window: NSWindow?
        weak var owner: NDCefChromeWindow?
        /// The size Chromium gave the surface when it was adopted. Views resizes
        /// a sheet as its content changes, so the placement is recomputed from
        /// the live frame rather than from this; it is kept for the trace.
        let size: NSSize
    }

    private static var adopted: [ObjectIdentifier: Adopted] = [:]
    private static var timer: Timer?
    /// Windows the sweep has already described, so a rejected candidate is
    /// reported once rather than five times a second.
    private static var described: Set<ObjectIdentifier> = []
    private static var sweeps = 0

    /// Mirrors the GTK side's 200ms top-level watch. A notification would be
    /// tighter, but a Views window is ordered in without ever becoming key or
    /// main, so `didBecomeKey`/`didBecomeMain` never fire for one and the only
    /// reliable signal is the process's window list.
    private static let interval: TimeInterval = 0.2

    static func start() {
        NDCefDownloadAnimation.install()
        NDCefPictureInPicture.install()
        guard timer == nil else { return }
        timer = Timer.scheduledTimer(withTimeInterval: interval, repeats: true) { _ in
            MainActor.assumeIsolated { sweep() }
        }
    }

    /// Hands every surface back before the engine tears down. A Chromium window
    /// left as a child of a window AppKit is closing is one more object in the
    /// quit path, and that path already has a crash of its own.
    static func releaseAll() {
        timer?.invalidate()
        timer = nil
        NDCefPictureInPicture.releaseAll()
        for entry in adopted.values {
            guard let window = entry.window else { continue }
            window.parent?.removeChildWindow(window)
        }
        adopted.removeAll()
    }

    private static func sweep() {
        let owners = NDCefChromeWindow.liveWindows
        guard !owners.isEmpty else { return }
        let anchors = Set(owners.compactMap { $0.anchorWindow.map(ObjectIdentifier.init) })
        let hosts = Set(owners.compactMap { $0.hostWindow.map(ObjectIdentifier.init) })

        sweeps += 1
        if sweeps == 1 { owners.first?.traceSurface("watch started over \(owners.count) browser(s)") }
        for window in NSApp.windows {
            let key = ObjectIdentifier(window)
            if adopted[key] != nil { continue }
            if anchors.contains(key) || hosts.contains(key) { continue }
            // A browser window Chrome made for itself is never a surface to
            // adopt: it is kept off screen, however often Views shows it.
            if NDCefChromeCreated.owns(window) {
                if window.isVisible || window.alphaValue > 0 { NDCefChromeCreated.hide(window) }
                continue
            }
            NDCefPictureInPicture.sweep(window)
            guard window.isVisible, window.alphaValue > 0 else { continue }
            if described.insert(key).inserted {
                owners.first?.traceSurface(
                    "candidate class=\(NSStringFromClass(type(of: window))) level=\(window.level.rawValue) "
                        + "parent=\(window.parent.map { NSStringFromClass(type(of: $0)) } ?? "none") \(window.frame)")
            }
            guard window.level == .normal else { continue }
            guard NSStringFromClass(type(of: window)).contains(viewsWindowClass) else { continue }
            // Backstop for NDCefDownloadAnimation, which stops the window
            // before it is ordered in.
            if NDCefDownloadAnimation.matches(window) {
                window.orderOut(nil)
                owners.first?.traceSurface("download animation hidden \(window.frame.size)")
                continue
            }
            guard let owner = self.owner(of: window, among: owners) else { continue }
            adopt(window, owner: owner)
        }

        NDCefPictureInPicture.sweepGone()
        var gone: [ObjectIdentifier] = []
        for (key, entry) in adopted {
            guard let window = entry.window, window.isVisible else {
                gone.append(key)
                entry.window?.parent?.removeChildWindow(entry.window!)
                continue
            }
            guard let owner = entry.owner else {
                gone.append(key)
                window.parent?.removeChildWindow(window)
                continue
            }
            place(window, owner: owner)
        }
        for key in gone { adopted.removeValue(forKey: key) }
    }

    /// The browser a surface belongs to. Chromium parents a sheet to the widget
    /// that raised it, so the anchor is the answer whenever AppKit reports one;
    /// a surface with no parent is matched by overlap instead, which is what a
    /// bubble ordered straight onto the screen gives.
    private static func owner(of window: NSWindow, among owners: [NDCefChromeWindow]) -> NDCefChromeWindow? {
        if let parent = window.parent {
            if let hit = owners.first(where: { $0.anchorWindow === parent }) { return hit }
        }
        var best: (owner: NDCefChromeWindow, area: CGFloat)?
        for owner in owners {
            guard let rect = owner.surfaceTargetFrame else { continue }
            let overlap = rect.intersection(window.frame)
            guard !overlap.isEmpty else { continue }
            let area = overlap.width * overlap.height
            if best == nil || area > best!.area { best = (owner, area) }
        }
        return best?.owner
    }

    private static func adopt(_ window: NSWindow, owner: NDCefChromeWindow) {
        adopted[ObjectIdentifier(window)] = Adopted(window: window, owner: owner, size: window.frame.size)
        // Animating a window Chromium is driving would fight its own layout.
        window.animationBehavior = .none
        place(window, owner: owner)
        owner.traceSurface(
            "adopted class=\(NSStringFromClass(type(of: window))) \(window.frame.size) "
                + "over \(owner.surfaceTargetFrame.map(\.debugDescription) ?? "no webview rect")")
    }

    /// Where Chrome puts a tab-modal sheet: centred on the web contents and
    /// pinned to its top edge, clamped so a surface taller or wider than the
    /// view still starts inside it.
    private static func place(_ window: NSWindow, owner: NDCefChromeWindow) {
        guard let host = owner.hostWindow, let target = owner.surfaceTargetFrame else {
            // The tab is hidden or the view is off screen. The surface goes with
            // it rather than floating over whatever is behind.
            if window.isVisible {
                window.parent?.removeChildWindow(window)
                window.orderOut(nil)
            }
            return
        }
        if window.parent !== host {
            window.parent?.removeChildWindow(window)
            host.addChildWindow(window, ordered: .above)
        }
        var frame = window.frame
        frame.origin.x = max(target.minX, target.midX - frame.width / 2)
        frame.origin.y = min(target.maxY - frame.height, target.maxY)
        if frame != window.frame { window.setFrame(frame, display: false) }
    }
}
/// Chrome's download-started animation: an arrow drawn in a parentless,
/// click-through Views window of its own as a download begins, anchored to a
/// toolbar this embedding does not have. No feature, switch or pref in CEF 151
/// turns it off (the partial-view pref covers the bubble only;
/// chrome.downloads.setUiOptions does, but needs an extension), and the app's
/// panel is the report, so the window is never ordered in: the Views window
/// class's ordering methods skip it. The sweep would leave it on screen for up
/// to one tick.
@MainActor enum NDCefDownloadAnimation {
    private static var installed = false

    static func matches(_ window: NSWindow) -> Bool {
        let hit = window.parent == nil && window.ignoresMouseEvents && window.frame.width <= 96 && window.frame.height <= 96
            && NDCefDownloads.startedRecently
        if hit, ProcessInfo.processInfo.environment["ND_WEBVIEW_TRACE"] == "1" {
            FileHandle.standardError.write("ND_WV cef download animation kept off screen \(window.frame.size)\n".data(using: .utf8)!)
        }
        return hit
    }

    static func install() {
        guard !installed, let cls = NSClassFromString("NativeWidgetMacNSWindow") else { return }
        installed = true
        typealias Order = @convention(c) (NSWindow, Selector, Int, Int) -> Void
        typealias Plain = @convention(c) (NSWindow, Selector, AnyObject?) -> Void
        wrap(cls, #selector(NSWindow.order(_:relativeTo:)), as: Order.self) { original in
            let block: @convention(block) (NSWindow, Int, Int) -> Void = { window, place, other in
                if place != NSWindow.OrderingMode.out.rawValue {
                    if MainActor.assumeIsolated({ matches(window) }) { return }
                    MainActor.assumeIsolated { _ = NDCefInstallPrompt.claim(window) }
                }
                original(window, #selector(NSWindow.order(_:relativeTo:)), place, other)
            }
            return imp_implementationWithBlock(block)
        }
        for selector in [#selector(NSWindow.orderFront(_:)), #selector(NSWindow.makeKeyAndOrderFront(_:))] {
            wrap(cls, selector, as: Plain.self) { original in
                let block: @convention(block) (NSWindow, AnyObject?) -> Void = { window, sender in
                    if MainActor.assumeIsolated({ matches(window) }) { return }
                    // A prompt answered unseen must not take key status from
                    // the app's window, so it is only ordered in.
                    if MainActor.assumeIsolated({ NDCefInstallPrompt.claim(window) }), selector == #selector(NSWindow.makeKeyAndOrderFront(_:)) {
                        window.orderFront(sender)
                        return
                    }
                    original(window, selector, sender)
                }
                return imp_implementationWithBlock(block)
            }
        }
        wrap(cls, #selector(NSWindow.makeKey), as: (@convention(c) (NSWindow, Selector) -> Void).self) { original in
            let block: @convention(block) (NSWindow) -> Void = { window in
                if MainActor.assumeIsolated({ NDCefInstallPrompt.holds(window) }) { return }
                original(window, #selector(NSWindow.makeKey))
            }
            return imp_implementationWithBlock(block)
        }
    }

    /// Replaces `selector` on `cls` only. When the class inherits the method,
    /// it gets one of its own that calls the inherited one, so NSWindow itself
    /// is never touched.
    private static func wrap<F>(_ cls: AnyClass, _ selector: Selector, as _: F.Type, _ make: (F) -> IMP) {
        guard let method = class_getInstanceMethod(cls, selector) else { return }
        let original = unsafeBitCast(method_getImplementation(method), to: F.self)
        let imp = make(original)
        if !class_addMethod(cls, selector, imp, method_getTypeEncoding(method)) {
            method_setImplementation(method, imp)
        }
    }
}

/// Chrome's "Add <name>?" prompt for a Web Store install the app has already
/// confirmed in a dialog of its own (`acceptExtensionInstall`).
///
/// The prompt is a Views window that no CEF callback is asked about, and
/// Chrome offers no switch or policy that skips it for a store install. So the
/// first parentless Views window ordered in while an install is armed is taken
/// as the prompt: it is ordered in transparent and click-through, which Views
/// still counts as shown (its accept button only enables once the dialog has
/// been visible for a moment), and its "Add extension" button is pressed
/// through the accessibility tree. A prompt still up after a few seconds is
/// put back on screen for the user to answer.
@MainActor enum NDCefInstallPrompt {
    private static let armedFor: TimeInterval = 20
    private static let firstPress: TimeInterval = 0.8
    private static let pressInterval: TimeInterval = 0.4
    private static let presses = 10

    private static var armedUntil: Date?
    private static var held: Set<ObjectIdentifier> = []
    /// The window that was key when the prompt came up. Chrome activates its
    /// dialog and, once it closes, nothing of the app's is key any more.
    private static weak var keyBefore: NSWindow?

    static func arm() {
        armedUntil = Date().addingTimeInterval(armedFor)
        trace("armed")
    }

    static func holds(_ window: NSWindow) -> Bool {
        held.contains(ObjectIdentifier(window))
    }

    /// Takes `window` as the prompt when an install is armed. True for a
    /// window already taken.
    static func claim(_ window: NSWindow) -> Bool {
        if holds(window) { return true }
        guard let until = armedUntil, Date() < until else { return false }
        guard window.parent == nil, window.level == .normal,
              window.frame.width >= 200, window.frame.height >= 80,
              !NDCefChromeCreated.owns(window) else { return false }
        armedUntil = nil
        keyBefore = NSApp.keyWindow
        held.insert(ObjectIdentifier(window))
        window.alphaValue = 0
        window.ignoresMouseEvents = true
        trace("claimed \(window.frame.size)")
        DispatchQueue.main.asyncAfter(deadline: .now() + firstPress) { [weak window] in
            MainActor.assumeIsolated { press(window, left: presses) }
        }
        return true
    }

    private static func press(_ window: NSWindow?, left: Int) {
        guard let window, window.isVisible else {
            if let window { held.remove(ObjectIdentifier(window)) }
            trace("answered")
            keyBefore?.makeKeyAndOrderFront(nil)
            keyBefore = nil
            return
        }
        guard left > 0 else {
            held.remove(ObjectIdentifier(window))
            window.alphaValue = 1
            window.ignoresMouseEvents = false
            trace("unanswered, shown")
            return
        }
        pressAccept(window, attempt: presses - left)
        DispatchQueue.main.asyncAfter(deadline: .now() + pressInterval) { [weak window] in
            MainActor.assumeIsolated { press(window, left: left - 1) }
        }
    }

    /// Views drops a key event sent to a window that is not key, but its
    /// accessibility tree takes a press: the buttons
    /// are found there, and the accept button is the trailing one in the
    /// bottom row, where macOS puts a dialog's default action.
    private static func pressAccept(_ window: NSWindow, attempt: Int) {
        var buttons: [NSObject] = []
        var queue: [Any] = [window.contentView as Any]
        var seen = 0
        while !queue.isEmpty, seen < 400 {
            let node = queue.removeFirst()
            seen += 1
            guard let element = node as? NSObject, element.responds(to: #selector(NSAccessibilityProtocol.accessibilityChildren)) else { continue }
            let accessible = element as? NSAccessibilityProtocol
            if accessible?.accessibilityRole() == .button, accessible?.isAccessibilityEnabled() == true {
                buttons.append(element)
            }
            queue.append(contentsOf: accessible?.accessibilityChildren() ?? [])
        }
        let frames = buttons.map { (($0 as? NSAccessibilityProtocol)?.accessibilityFrame() ?? .zero) }
        guard let bottom = frames.map(\.minY).min() else {
            if attempt == 0 { trace("no enabled button yet") }
            return
        }
        let row = zip(buttons, frames).filter { abs($0.1.minY - bottom) < 4 }
        guard let accept = row.max(by: { $0.1.maxX < $1.1.maxX }) else { return }
        let title = (accept.0 as? NSAccessibilityProtocol)?.accessibilityTitle() ?? (accept.0 as? NSAccessibilityProtocol)?.accessibilityLabel() ?? ""
        let pressed = (accept.0 as? NSAccessibilityProtocol)?.accessibilityPerformPress() ?? false
        trace("press \"\(title)\" of \(buttons.count) button(s) pressed=\(pressed)")
    }

    private static func trace(_ message: String) {
        guard ProcessInfo.processInfo.environment["ND_WEBVIEW_TRACE"] == "1" else { return }
        FileHandle.standardError.write("ND_WV cef installPrompt \(message)\n".data(using: .utf8)!)
    }
}
#endif
