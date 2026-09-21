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
            guard window.isVisible, window.alphaValue > 0 else { continue }
            if described.insert(key).inserted {
                owners.first?.traceSurface(
                    "candidate class=\(NSStringFromClass(type(of: window))) level=\(window.level.rawValue) "
                        + "parent=\(window.parent.map { NSStringFromClass(type(of: $0)) } ?? "none") \(window.frame)")
            }
            guard window.level == .normal else { continue }
            guard NSStringFromClass(type(of: window)).contains(viewsWindowClass) else { continue }
            guard let owner = self.owner(of: window, among: owners) else { continue }
            adopt(window, owner: owner)
        }

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
#endif
