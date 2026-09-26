#if canImport(CCef)
import AppKit
import CCef
import Foundation

/// Chrome style embedding for one `<webview>`, opted into with
/// `ND_CEF_STYLE=chrome`.
///
/// Chrome style is what carries Chromium's own extension runtime,
/// `--load-extension` and `chrome://extensions`. It is unreachable through the
/// Alloy path this engine otherwise uses: on macOS a non-NULL `parent_view`
/// forces Alloy ("Alloy style will always be used if |windowless_rendering_
/// enabled| is true or if |parent_view| is provided",
/// include/internal/cef_types_mac.h), so the browser has to be born in a CEF
/// Views window CEF owns.
///
/// The window is never the thing the user sees. It is created frameless, with
/// no Chrome toolbar, so the BrowserView fills it at the origin, and then the
/// web contents NSView is lifted out of it into the NDCefWebView. Pixels and
/// input then live in the host's own window and there is no frame tracking to
/// lag: the web contents is an ordinary autoresizing subview.
///
/// The window itself stays alive as an anchor, transparent and click-through,
/// glued over the webview in screen coordinates. Chromium positions everything
/// it draws outside the web contents (select popups, autofill, permission and
/// extension bubbles, Chromium's own context menu) against the widget's screen
/// bounds, and those come from this window.
@MainActor final class NDCefChromeWindow {
    /// Weak, not unowned: the view's own deinit is one of the paths into
    /// `teardown`, and an unowned read of an object already being destroyed is
    /// a fatal error rather than a nil.
    private weak var view: NDCefWebView?
    private var cefWindow: UnsafeMutablePointer<cef_window_t>?
    private var browserView: UnsafeMutablePointer<cef_browser_view_t>?
    /// Where the frontend last said the page belongs, in the webview's own
    /// coordinates with the origin at the top left, the frontend's own. Nil
    /// while no inspector is docked, which is the page filling the webview.
    private var pageRect: cef_rect_t?
    private lazy var frontend = NDCefDockFrontend(window: self)
    private var devToolsView: UnsafeMutablePointer<cef_browser_view_t>?
    /// The inspector's view between `close_dev_tools` and its browser actually
    /// going away.
    private var devToolsClosing: UnsafeMutablePointer<cef_browser_view_t>?
    private var devToolsCloseTimer: Timer?
    private var dockFrontendTimer: Timer?
    /// The docked inspector's own Views window. A Chrome style window hosts at
    /// most one Chrome style BrowserView (cef_types_runtime.h), and DevTools can
    /// only be Chrome style. With the inspector added to the page's window, its
    /// BrowserView took over the widget's `kBrowserViewKey`, so every later tab
    /// update of the page looked itself up in the inspector's tab strip and hit
    /// `NOTREACHED() << "Tab view not found for handle"` in
    /// HorizontalTabStripRegionView::GetTabData: any navigation, reload or stop
    /// of an inspected page killed the process. The inspector gets a window and
    /// an anchor of its own, lifted into the host under the page. Chrome's own
    /// dock has the same shape: the inspector fills the webview and draws its
    /// panels around the hole it keeps for the page, and the page is drawn in
    /// that hole (`pageRect`).
    private var toolsWindow: UnsafeMutablePointer<cef_window_t>?
    private weak var toolsAnchor: NSWindow?
    private weak var toolsLifted: NSView?
    private weak var anchor: NSWindow?
    /// The Chromium subtree now living in `view`, and the superview it came
    /// from. Chromium re-attaches its own native view host on some layout and
    /// navigation paths, which puts the subtree back; `relift` is what notices.
    private weak var lifted: NSView?
    private var reliftCount = 0
    private var reliftTimer: Timer?
    /// Frame-rate poll from a dock until the inspector's subtree is in the
    /// host view. Until then the inspector draws nowhere a user can see or
    /// click, and the page-wide relift poll would leave it so for up to its
    /// whole period.
    private var toolsLiftTimer: Timer?
    private var observers: [NSObjectProtocol] = []
    /// The host window the observers are registered on. A `<webview>` moved
    /// between windows (`moveNode`) keeps this object, so the notifications
    /// have to follow it or the anchor stops tracking the new window's own
    /// resize and move.
    private weak var observedWindow: NSWindow?
    private var closed = false

    /// Every live instance. Chromium's shutdown walks a docked DevTools
    /// BrowserView whose web contents now lives in the host's window and dies
    /// on a bad access, so every quit path closes the dock first.
    nonisolated(unsafe) private static let live = NSHashTable<NDCefChromeWindow>.weakObjects()

    /// Any Views window this host has not handed back to CEF. The anchor
    /// NSWindow outlives it either way: `cef_window_t::close` is asynchronous
    /// and, measured on 151.3.23, the window is never deallocated before the
    /// process goes, on the quit that succeeds as much as on the one that
    /// faults, so waiting on the NSWindow itself buys nothing.
    static var anyWindowOpen: Bool {
        live.allObjects.contains { $0.cefWindow != nil }
    }

    static func closeDockedDevTools() {
        for window in live.allObjects { window.closeDevTools() }
    }

    /// Every instance that still has a Views window, for the surface adopter.
    static var liveWindows: [NDCefChromeWindow] {
        live.allObjects.filter { $0.cefWindow != nil && !$0.closed }
    }

    /// The invisible window CEF drew the browser into. Chromium parents its own
    /// sheets to it, which is how a surface is matched to its browser.
    var anchorWindow: NSWindow? { anchor }

    /// The app window showing this browser.
    var hostWindow: NSWindow? { view?.window }

    /// The webview's rectangle in screen coordinates, or nil when its tab is
    /// hidden or it is off screen.
    var surfaceTargetFrame: NSRect? { targetScreenFrame() }

    func traceSurface(_ message: String) {
        view?.ndTrace("chrome surface \(message)")
    }

    init(view: NDCefWebView) {
        self.view = view
        NDCefChromeWindow.live.add(self)
    }

    /// Separate from `init` because `cef_window_create_top_level` calls
    /// `on_window_created` before it returns, and that callback resolves back
    /// through the view, which cannot be holding this object yet.
    func start(url: String, profile: String) {
        guard let view else { return }
        let box = view.box
        guard let client = box.client,
              let browserViewDelegate = box.browserViewDelegate,
              let windowDelegate = box.windowDelegate else { return }

        var settings = cef_browser_settings_t()
        settings.size = MemoryLayout<cef_browser_settings_t>.size
        var target = cef_string_t()
        ndCefSetString(url, &target)
        defer { nd_cef_string_clear(&target) }

        // Handed over, same contract as the Alloy create: the library owns
        // what it is passed and this view keeps its own reference.
        let context = NDCefProfiles.context(for: profile)
        nd_cef_ref_add(client)
        nd_cef_ref_add(context)
        nd_cef_ref_add(browserViewDelegate)
        browserView = nd_cef_browser_view_create(
            client, &target, &settings, nil, context, browserViewDelegate)
        guard browserView != nil else {
            ndCefWarn("cef_browser_view_create failed")
            return
        }
        nd_cef_ref_add(windowDelegate)
        if nd_cef_window_create_top_level(windowDelegate) == nil {
            ndCefWarn("cef_window_create_top_level failed")
        }
    }

    // MARK: - Window

    /// `on_window_created`. The reference on |window| is this object's.
    func windowCreated(_ window: UnsafeMutablePointer<cef_window_t>) {
        cefWindow = window
        let panel = UnsafeMutableRawPointer(window).assumingMemoryBound(to: cef_panel_t.self)
        // Fill layout plus a frameless window with no Chrome toolbar is what
        // puts the web contents at the window origin at the window's size, so
        // the lifted NSView needs no offset of its own.
        if let layout = panel.pointee.set_to_fill_layout?(panel) {
            nd_cef_ref_release(UnsafeMutableRawPointer(layout))
        }
        if let browserView {
            // add_child_view TAKES the reference it is passed, the same
            // hand-over contract as the create calls, so the one this object
            // keeps has to be added first. Without it the BrowserView is freed
            // the moment this object drops its own, while the window still
            // holds it, and every later window operation faults.
            nd_cef_ref_add(browserView)
            let child = UnsafeMutableRawPointer(browserView).assumingMemoryBound(to: cef_view_t.self)
            panel.pointee.add_child_view?(panel, child)
        }

        guard let view, let anchorWindow = Self.anchorWindow(of: window) else { return }
        anchor = anchorWindow
        Self.makeImperceptible(anchorWindow)
        // Chromium's views hierarchy only produces frames for a widget it
        // believes is showing, so the window is shown for real and made
        // imperceptible instead of being left hidden.
        let wasKey = view.window?.isKeyWindow ?? false
        window.pointee.show?(window)
        if wasKey { view.window?.makeKey() }
        syncAnchor()
        observeGeometry()
        NDCefSurfaceWindows.start()
        view.ndTrace("chrome anchored \(anchorWindow.frame) target=\(targetScreenFrame().map(\.debugDescription) ?? "none")")
        if ProcessInfo.processInfo.environment["ND_CEF_DUMP_VIEWS"] == "1" { dumpViews() }
    }

    private static func anchorWindow(of window: UnsafeMutablePointer<cef_window_t>) -> NSWindow? {
        guard let handle = window.pointee.get_window_handle?(window) else {
            ndCefWarn("chrome style: the Views window reported no native handle")
            return nil
        }
        let content = Unmanaged<NSView>.fromOpaque(handle).takeUnretainedValue()
        guard let anchorWindow = content.window else {
            ndCefWarn("chrome style: the Views window has no NSWindow")
            return nil
        }
        return anchorWindow
    }

    private static func makeImperceptible(_ anchorWindow: NSWindow) {
        anchorWindow.alphaValue = 0
        anchorWindow.hasShadow = false
        anchorWindow.ignoresMouseEvents = true
        anchorWindow.isExcludedFromWindowsMenu = true
        // Out of the process's accessibility tree: the anchor is a real window
        // as far as AppKit is concerned, and a frameless child window in front
        // of the app's own is what an assistive client (and `System Events`)
        // reads as "window 1" — a window with no title-bar buttons and none of
        // the app's content. The web contents' own AX tree hangs off the lifted
        // view inside the host window and is unaffected.
        anchorWindow.setAccessibilityElement(false)
        anchorWindow.animationBehavior = .none
        anchorWindow.collectionBehavior = [.transient, .ignoresCycle, .fullScreenAuxiliary]
    }

    /// `on_after_created`, once the browser behind the BrowserView exists.
    func browserCreated(host browserHost: UnsafeMutablePointer<cef_browser_host_t>) {
        liftWebContents()
        if view?.isHiddenOrHasHiddenAncestor ?? true || view?.window == nil {
            browserHost.pointee.was_hidden?(browserHost, 1)
        }
        reliftTimer?.invalidate()
        // Chromium re-attaches its native view host on paths that have no
        // client callback (a cross-process navigation swaps the render widget
        // view, a renderer crash rebuilds it). The poll is what puts the new
        // subtree back in the host view; `ND_WEBVIEW_TRACE=1` reports how often
        // it fires.
        reliftTimer = Timer.scheduledTimer(withTimeInterval: 0.25, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.liftWebContents() }
        }
    }

    // MARK: - DevTools

    /// Docks the DevTools BrowserView in a Views window of its own (see
    /// `toolsWindow`).
    func dockDevTools(_ popup: UnsafeMutablePointer<cef_browser_view_t>) -> Bool {
        guard !closed, cefWindow != nil, browserView != nil,
              let delegate = view?.box.devToolsWindowDelegate else { return false }
        if let devToolsView {
            nd_cef_ref_release(devToolsView)
            self.devToolsView = nil
        }
        // A window still held by an inspector on its way out goes now: this
        // one takes the column.
        closeToolsWindow()
        // The field's own reference. The one the callback arrived with is
        // balanced by its own release.
        nd_cef_ref_add(popup)
        devToolsView = popup
        // Where the page goes until the frontend says otherwise: Chrome's own
        // right-dock default, so the page is in its usual column for the frame
        // or two before the first announcement rather than covering the
        // inspector.
        let bounds = view?.bounds ?? .zero
        let width = Int32(bounds.width.rounded())
        let height = Int32(bounds.height.rounded())
        pageRect = cef_rect_t(x: 0, y: 0, width: max(1, width - Self.dockWidth(width)), height: max(1, height))
        nd_cef_ref_add(delegate)
        if nd_cef_window_create_top_level(delegate) == nil {
            ndCefWarn("cef_window_create_top_level failed for the inspector")
        }
        syncAnchor()
        layoutLifted()
        pointFrontendAtDock()
        view?.ndTrace("chrome devtools docked")
        return true
    }

    /// `on_window_created` for the inspector's window. The reference on
    /// |window| is this object's.
    func devToolsWindowCreated(_ window: UnsafeMutablePointer<cef_window_t>) {
        guard !closed, let devToolsView else {
            window.pointee.close?(window)
            nd_cef_ref_release(window)
            return
        }
        toolsWindow = window
        let panel = UnsafeMutableRawPointer(window).assumingMemoryBound(to: cef_panel_t.self)
        if let layout = panel.pointee.set_to_fill_layout?(panel) {
            nd_cef_ref_release(UnsafeMutableRawPointer(layout))
        }
        // add_child_view takes the reference it is passed.
        nd_cef_ref_add(devToolsView)
        let child = UnsafeMutableRawPointer(devToolsView).assumingMemoryBound(to: cef_view_t.self)
        panel.pointee.add_child_view?(panel, child)
        guard let anchorWindow = Self.anchorWindow(of: window) else { return }
        toolsAnchor = anchorWindow
        Self.makeImperceptible(anchorWindow)
        let wasKey = view?.window?.isKeyWindow ?? false
        window.pointee.show?(window)
        if wasKey { view?.window?.makeKey() }
        observeGeometry()
        syncAnchor()
        toolsLiftTimer?.invalidate()
        let deadline = Date().addingTimeInterval(5)
        toolsLiftTimer = Timer.scheduledTimer(withTimeInterval: 1.0 / 60, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                guard !self.closed, self.toolsWindow != nil, self.toolsLifted == nil, Date() < deadline else {
                    self.toolsLiftTimer?.invalidate()
                    self.toolsLiftTimer = nil
                    return
                }
                self.liftWebContents()
            }
        }
    }

    /// The inspector's window, closed and handed back to CEF.
    private func closeToolsWindow() {
        toolsLiftTimer?.invalidate()
        toolsLiftTimer = nil
        if let toolsLifted, let toolsAnchor {
            toolsLifted.removeFromSuperview()
            toolsAnchor.contentView = toolsLifted
        }
        toolsLifted = nil
        if let toolsAnchor {
            toolsAnchor.parent?.removeChildWindow(toolsAnchor)
            toolsAnchor.orderOut(nil)
        }
        toolsAnchor = nil
        if let toolsWindow {
            self.toolsWindow = nil
            toolsWindow.pointee.close?(toolsWindow)
            nd_cef_ref_release(toolsWindow)
        }
    }

    /// Puts the page where the frontend asked for it. This is the hole in the
    /// inspector's layout, and it is also what device mode reports, so the
    /// phone is drawn in the page area instead of inside the inspector.
    func pageBoundsAnnounced(_ rect: cef_rect_t) {
        guard !closed, devToolsView != nil, rect.width > 0, rect.height > 0 else { return }
        if let current = pageRect, current.x == rect.x, current.y == rect.y,
           current.width == rect.width, current.height == rect.height { return }
        pageRect = rect
        syncAnchor()
        layoutLifted()
        view?.ndTrace("chrome devtools page rect \(rect.width)x\(rect.height)@\(rect.x),\(rect.y)")
    }

    /// Tells the frontend it is docked, so it draws its own close button. The
    /// URL is only readable once CEF has put it on the main frame, and the
    /// inspector's browser is not on this client, so this polls for it rather
    /// than waiting on a load callback that never comes.
    private func pointFrontendAtDock() {
        dockFrontendTimer?.invalidate()
        var tries = 0
        dockFrontendTimer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                tries += 1
                guard self.dockFrontend() || tries >= 100 else { return }
                self.dockFrontendTimer?.invalidate()
                self.dockFrontendTimer = nil
            }
        }
    }

    private func dockFrontend() -> Bool {
        guard let devToolsView, let browser = devToolsView.pointee.get_browser?(devToolsView) else { return false }
        defer { nd_cef_ref_release(browser) }
        // The same poll is where the frontend's own browser first answers, so
        // the protocol session that reads its page rectangle starts here.
        if !frontend.isRunning, let browserHost = browser.pointee.get_host?(browser) {
            defer { nd_cef_ref_release(browserHost) }
            frontend.start(host: browserHost, observer: view?.box.dockObserver)
        }
        guard let frame = browser.pointee.get_main_frame?(browser) else { return false }
        defer { nd_cef_ref_release(frame) }
        return ndCefDockFrontend(frame)
    }

    /// Both arrive from inside CEF's walk over its message observers
    /// (CefDevToolsController::DispatchProtocolMessage), and answering a result
    /// sends the next method, whose notifications CEF dispatches through the
    /// same list while it is still being walked: "Check failed:
    /// !check_reentrancy" (base/observer_list.h) on every dock. Each is taken
    /// on a turn of its own, in the order it arrived.
    func frontendResult(id: Int32, ok: Bool) {
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated { self?.frontend.handleResult(id: id, ok: ok) }
        }
    }

    func frontendEvent(method: String, json: String) {
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated { self?.frontend.handleEvent(method: method, json: json) }
        }
    }

    /// Undoes `dockDevTools`. The page takes the whole width back at once; the
    /// inspector's window goes once its browser has (`retire`).
    func closeDevTools() {
        guard let docked = devToolsView else { return }
        devToolsView = nil
        devToolsClosing = docked
        if let browserHost = view?.browserHost() {
            browserHost.pointee.close_dev_tools?(browserHost)
            nd_cef_ref_release(browserHost)
        }
        syncAnchor()
        layoutLifted()
        view?.ndTrace("chrome devtools closing")
        awaitDevToolsBrowser()
    }

    /// The inspector's browser is going: the window can be laid out again, and
    /// the page takes the whole width back, once it has actually gone.
    ///
    /// The frontend's own close button takes the browser away without passing
    /// through `closeDevTools`, so a still-docked view is given up here too;
    /// leaving it set keeps `hasDockedDevTools` true and the app's toggle then
    /// tries to close an inspector that is already gone.
    ///
    /// `on_browser_destroyed` runs while the browser still answers, so the dock
    /// is given up only once that browser has really gone, and only while it is
    /// still the one on screen. Both are load-bearing: acting on the report
    /// alone takes down an inspector docked since, which then lays out at zero
    /// width and leaves the page beside a blank half.
    func devToolsBrowserClosed() {
        if devToolsClosing != nil {
            awaitDevToolsBrowser()
            return
        }
        guard let docked = devToolsView else { return }
        // As a bit pattern, the same way every other CEF pointer crosses an
        // isolation boundary here: Swift's raw pointers are not Sendable.
        let token = UInt(bitPattern: docked)
        devToolsCloseTimer?.invalidate()
        let deadline = Date().addingTimeInterval(5)
        devToolsCloseTimer = Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                let stop = {
                    self.devToolsCloseTimer?.invalidate()
                    self.devToolsCloseTimer = nil
                }
                // Superseded by a dock since, or the report was spurious and
                // the browser is still there: either way this is not ours.
                guard let mine = self.devToolsView, UInt(bitPattern: mine) == token else { return stop() }
                guard self.browserIsGone(mine) else {
                    if Date() >= deadline { stop() }
                    return
                }
                stop()
                self.devToolsView = nil
                self.dockFrontendTimer?.invalidate()
                self.dockFrontendTimer = nil
                self.retire(mine)
            }
        }
    }

    private func browserIsGone(_ browserView: UnsafeMutablePointer<cef_browser_view_t>) -> Bool {
        let browser = browserView.pointee.get_browser?(browserView)
        defer { if let browser { nd_cef_ref_release(browser) } }
        return browser == nil || browser!.pointee.is_valid?(browser) == 0
    }

    /// CEF creates the inspector's browser with a client of its own, so this
    /// client's `on_before_close` never fires for it, and the delegate's
    /// `on_browser_destroyed` runs while the browser is still answering. The
    /// BrowserView losing its browser is the signal, from the only side that
    /// has one.
    private func awaitDevToolsBrowser() {
        devToolsCloseTimer?.invalidate()
        let deadline = Date().addingTimeInterval(5)
        devToolsCloseTimer = Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                guard let closing = self.devToolsClosing else {
                    self.devToolsCloseTimer?.invalidate()
                    self.devToolsCloseTimer = nil
                    return
                }
                let browser = closing.pointee.get_browser?(closing)
                let gone = browser == nil || browser!.pointee.is_valid?(browser) == 0
                if let browser { nd_cef_ref_release(browser) }
                guard gone || Date() >= deadline else { return }
                self.retire(closing)
            }
        }
    }

    private func retire(_ closing: UnsafeMutablePointer<cef_browser_view_t>) {
        devToolsClosing = nil
        devToolsCloseTimer?.invalidate()
        devToolsCloseTimer = nil
        // Only when nothing took its place: a dock since owns the window, the
        // page rectangle and the session behind it.
        if devToolsView == nil {
            closeToolsWindow()
            frontend.stop()
            pageRect = nil
        }
        nd_cef_ref_release(closing)
        syncAnchor()
        layoutLifted()
        view?.ndTrace("chrome devtools closed")
    }

    /// Chrome's own right-dock default, floored at the width the frontend's
    /// toolbar needs: below that the toolbar overflows to the right and takes
    /// the close button off screen with it (397px on 151.3.23). The upper
    /// clamp leaves the page a column of its own.
    private static func dockWidth(_ width: Int32) -> Int32 {
        let wanted = width * 45 / 100
        return min(max(wanted, 400), width > 240 ? width - 240 : width / 2)
    }

    /// The page's and the inspector's rectangles in the webview's own
    /// coordinates. The inspector fills the webview; the page is the hole the
    /// frontend keeps for it, clamped to the webview.
    private func columns(_ bounds: NSRect) -> (page: NSRect, tools: NSRect) {
        guard devToolsView != nil, let rect = pageRect else { return (bounds, bounds) }
        let x = min(max(0, CGFloat(rect.x)), bounds.width - 1)
        let top = min(max(0, CGFloat(rect.y)), bounds.height - 1)
        let width = max(1, min(CGFloat(rect.width), bounds.width - x))
        let height = max(1, min(CGFloat(rect.height), bounds.height - top))
        let y = view?.isFlipped == true ? top : bounds.height - top - height
        return (NSRect(x: bounds.minX + x, y: bounds.minY + y, width: width, height: height), bounds)
    }

    /// Both lifted subtrees at their rectangle. The page's Views window is the
    /// page rectangle's size too (`syncAnchor`), so the page lays out at it.
    private func layoutLifted() {
        guard let view else { return }
        let (page, tools) = columns(view.bounds)
        if let lifted, lifted.superview === view, lifted.frame != page { lifted.frame = page }
        if let toolsLifted, toolsLifted.superview === view, toolsLifted.frame != tools { toolsLifted.frame = tools }
    }

    // MARK: - Lifting the web contents

    /// Moves Chromium's whole widget surface into the host view, or puts it
    /// back after Chromium re-attached it to the anchor.
    ///
    /// The lift is the anchor's content view, not the page's own
    /// WebContentsViewCocoa. Chromium's views are not NSViews: the only NSViews
    /// under a Views window are the compositor superview and whatever a
    /// views::NativeViewHost attaches, so the content view is the one node that
    /// holds all of them. A docked inspector's window is lifted the same way,
    /// under the page.
    private func liftWebContents() {
        guard !closed, let anchor, let view else { return }
        let (page, tools) = columns(view.bounds)
        if let lifted {
            if lifted.superview !== view {
                reliftCount += 1
                view.ndTrace("chrome relift #\(reliftCount)")
                lifted.removeFromSuperview()
                lifted.frame = page
                view.addSubview(lifted)
            }
        } else {
            if ProcessInfo.processInfo.environment["ND_CEF_DUMP_VIEWS"] == "1" { dumpViews() }
            guard let content = anchor.contentView else { return }
            view.ndTrace("chrome lift \(NSStringFromClass(type(of: content)))")
            lifted = Self.lift(content, from: anchor, into: view, at: page)
        }
        liftDevTools(into: view, at: tools)
    }

    /// The inspector's subtree, once Chromium has attached its web contents
    /// view. One attached after the lift is parented to the empty content view
    /// the anchor is left with, in a window that is alpha 0 and click-through:
    /// it draws, since its pixels come through the compositor, and takes no
    /// press at all.
    private func liftDevTools(into view: NSView, at frame: NSRect) {
        guard let toolsAnchor, let devToolsView else { return }
        if let toolsLifted {
            guard toolsLifted.superview !== view else { return }
            toolsLifted.removeFromSuperview()
            toolsLifted.frame = frame
            view.addSubview(toolsLifted, positioned: .below, relativeTo: lifted?.superview === view ? lifted : nil)
            return
        }
        guard let content = toolsAnchor.contentView,
              content.subviews.contains(where: { NSStringFromClass(type(of: $0)).contains("WebContentsView") })
        else { return }
        toolsLifted = Self.lift(content, from: toolsAnchor, into: view, at: frame, below: lifted)
        self.view?.ndTrace("chrome devtools lifted \(frame)")
    }

    private static func lift(
        _ content: NSView, from anchor: NSWindow, into view: NSView, at frame: NSRect, below: NSView? = nil
    ) -> NSView {
        // AppKit keeps a content view either way, so the anchor is handed an
        // empty one rather than left pointing at a view in another window.
        anchor.contentView = NSView(frame: content.frame)
        content.frame = frame
        content.autoresizingMask = []
        // The inspector goes under the page: the page is drawn in its hole.
        view.addSubview(content, positioned: .below, relativeTo: below?.superview === view ? below : nil)
        return content
    }

    /// The inspector's own render widget under a point of the webview, for
    /// `hitTest`. Its BridgedContentView asks its Views widget what is under the
    /// point, and the inspector's Chrome style window answers with its root view
    /// everywhere, so every press on the inspector went to a widget nobody sees.
    /// The page's column is left to AppKit.
    func devToolsHit(at point: NSPoint) -> NSView? {
        guard let view, let toolsLifted, toolsLifted.superview === view,
              toolsLifted.frame.contains(point) else { return nil }
        if let lifted, lifted.superview === view, lifted.frame.contains(point) { return nil }
        let local = toolsLifted.convert(point, from: view)
        for child in toolsLifted.subviews.reversed()
        where NSStringFromClass(type(of: child)).contains("WebContentsView") {
            if let hit = child.hitTest(local) { return hit }
        }
        return nil
    }

    /// Chromium's keyboard target. It is an ordinary subview of the host window
    /// now, so focus is an AppKit first-responder change. Routing focus through
    /// cef_browser_host_t::set_focus instead activates the anchor window, and
    /// the host takes key straight back, which leaves the page unfocused.
    var focusTarget: NSView? { lifted }

    // MARK: - Anchor geometry

    /// Called from the host view whenever its screen rectangle or visibility
    /// can have changed. The anchor carries no pixels, so this only has to be
    /// correct, not immediate.
    func hostGeometryChanged() {
        if view?.window !== observedWindow { observeGeometry() }
        syncAnchor()
        liftWebContents()
        layoutLifted()
    }

    /// The webview's own rectangle in AppKit screen coordinates, or nil when
    /// the view is not on screen.
    private func targetScreenFrame() -> NSRect? {
        guard let view, let host = view.window, !view.isHiddenOrHasHiddenAncestor else { return nil }
        let rect = host.convertToScreen(view.convert(view.bounds, to: nil))
        return (rect.width >= 1 && rect.height >= 1) ? rect : nil
    }

    /// One column of the webview in screen coordinates.
    private func screenFrame(of column: NSRect) -> NSRect? {
        guard let view, let host = view.window, column.width >= 1, column.height >= 1 else { return nil }
        return host.convertToScreen(view.convert(column, to: nil))
    }

    private func syncAnchor() {
        guard !closed, let view else { return }
        let visible = targetScreenFrame() != nil
        let (page, tools) = columns(view.bounds)
        if let anchor, let cefWindow {
            place(anchor, window: cefWindow, at: visible ? screenFrame(of: page) : nil)
        }
        if let toolsAnchor, let toolsWindow {
            place(toolsAnchor, window: toolsWindow, at: visible ? screenFrame(of: tools) : nil)
        }
    }

    private func place(_ anchor: NSWindow, window: UnsafeMutablePointer<cef_window_t>, at rect: NSRect?) {
        guard let host = view?.window, let rect else {
            if anchor.isVisible { window.pointee.hide?(window) }
            return
        }
        if anchor.parent !== host {
            anchor.parent?.removeChildWindow(anchor)
            host.addChildWindow(anchor, ordered: .above)
        }
        if anchor.frame != rect {
            anchor.setFrame(rect, display: false)
            if anchor.frame != rect { view?.ndTrace("chrome anchor clamped want=\(rect) got=\(anchor.frame)") }
        }
        if !anchor.isVisible { window.pointee.show?(window) }
    }

    /// `ND_CEF_DUMP_VIEWS=1`. Chromium's views are not NSViews, so the only
    /// NSViews under the anchor are the ones a views::NativeViewHost attaches;
    /// which of them to lift is a question about this tree.
    private func dumpViews() {
        func walk(_ node: NSView, _ depth: Int) {
            let pad = String(repeating: "  ", count: depth)
            view?.ndTrace("views \(pad)\(NSStringFromClass(type(of: node))) \(node.frame)")
            for child in node.subviews { walk(child, depth + 1) }
        }
        guard let content = anchor?.contentView else { return }
        walk(content, 0)
    }

    private func observeGeometry() {
        let center = NotificationCenter.default
        for observer in observers { center.removeObserver(observer) }
        observers = []
        observedWindow = view?.window
        let sync: @Sendable (Notification) -> Void = { [weak self] _ in
            MainActor.assumeIsolated { self?.syncAnchor() }
        }
        guard let host = view?.window else { return }
        // The anchor is alpha 0 and click-through, so the only thing that can
        // make it key is Chromium activating its own widget. Key would take the
        // host window's title bar out of its active look for a window nobody
        // can see, and the web contents is in the host's responder chain now,
        // so it needs no key window of its own. Refusing key through the
        // NSWindow subclass is not an option: Chromium's activation path
        // segfaults on a window whose canBecomeKeyWindow is NO.
        for anchorWindow in [anchor, toolsAnchor].compactMap({ $0 }) {
            observers.append(
                center.addObserver(
                    forName: NSWindow.didBecomeKeyNotification, object: anchorWindow, queue: nil
                ) { [weak self] _ in
                    MainActor.assumeIsolated {
                        guard let self, let host = self.view?.window else { return }
                        host.makeKey()
                    }
                })
        }
        for name in [
            NSWindow.didResizeNotification,
            NSWindow.didMoveNotification,
            NSWindow.didEndLiveResizeNotification,
            NSWindow.didEnterFullScreenNotification,
            NSWindow.didExitFullScreenNotification,
            NSWindow.didChangeScreenNotification,
            NSWindow.didDeminiaturizeNotification,
        ] {
            observers.append(center.addObserver(forName: name, object: host, queue: nil, using: sync))
        }
    }

    // MARK: - Teardown

    /// Ordering matters: the lifted subtree belongs to Chromium's view
    /// hierarchy, so it goes home before the browser is closed. Closing with
    /// the subtree still in a foreign window leaves Chromium removing a view
    /// from a superview it does not expect.
    func teardown() {
        guard !closed else { return }
        closed = true
        frontend.stop()
        reliftTimer?.invalidate()
        reliftTimer = nil
        toolsLiftTimer?.invalidate()
        toolsLiftTimer = nil
        devToolsCloseTimer?.invalidate()
        devToolsCloseTimer = nil
        dockFrontendTimer?.invalidate()
        dockFrontendTimer = nil
        for observer in observers { NotificationCenter.default.removeObserver(observer) }
        observers = []
        // The lifted subtree goes back first: it is the Views window's content
        // view, and closing the window while it is parented into another one
        // takes Chromium's teardown through a hierarchy that no longer exists.
        if let lifted, let anchor {
            lifted.removeFromSuperview()
            anchor.contentView = lifted
        }
        lifted = nil
        if let toolsLifted, let toolsAnchor {
            toolsLifted.removeFromSuperview()
            toolsAnchor.contentView = toolsLifted
        }
        toolsLifted = nil
        if let toolsAnchor {
            toolsAnchor.parent?.removeChildWindow(toolsAnchor)
            toolsAnchor.orderOut(nil)
        }
        closeDevTools()
        if let anchor {
            anchor.parent?.removeChildWindow(anchor)
            // Off screen before anything else unwinds. An anchor still in the
            // window list is a window AppKit can make key while it closes the
            // app's own windows, and Chromium's observer for that notification
            // is gone by then: the quit dies in `-[NSWindow becomeKeyWindow]`.
            anchor.orderOut(nil)
        }
        anchor = nil
        // The Views window outlives this call on purpose: it belongs to the
        // browser, and closing it while the browser is still alive walks a
        // child view list Chromium is about to rebuild. `closeWindow` runs it
        // once the browser has reported `on_before_close`; a window left open
        // instead is one AppKit closes during app teardown, which reaches a
        // Chromium observer on state that is already gone.
        if let browserView {
            self.browserView = nil
            nd_cef_ref_release(browserView)
        }
    }

    /// Closes the Views window, once the browser it belonged to has gone.
    func closeWindow() {
        closeToolsWindow()
        guard let cefWindow else { return }
        self.cefWindow = nil
        cefWindow.pointee.close?(cefWindow)
        nd_cef_ref_release(cefWindow)
    }

    /// Whether this view's inspector is still open or on its way out, for the
    /// toggle and for the ordered quit.
    var hasDockedDevTools: Bool { devToolsView != nil || devToolsClosing != nil }

}

// MARK: - Delegates

extension NDCefHandlerBox {
    /// The three capi structs Chrome style adds: a window delegate that pins
    /// the window to Chrome style and strips its frame, a browser view delegate
    /// that pins the browser to Chrome style with no toolbar, and the Chrome
    /// command handler, which is the only hook for the commands Chromium's own
    /// menus and accelerators would otherwise run.
    func buildChrome() {
        windowDelegate = ndCefAlloc(cef_window_delegate_t.self, self)
        browserViewDelegate = ndCefAlloc(cef_browser_view_delegate_t.self, self)
        devToolsViewDelegate = ndCefAlloc(cef_browser_view_delegate_t.self, self)
        devToolsWindowDelegate = ndCefAlloc(cef_window_delegate_t.self, self)
        dockObserver = ndCefAlloc(cef_dev_tools_message_observer_t.self, self)
        command = ndCefAlloc(cef_command_handler_t.self, self)
        wireWindowDelegate()
        wireBrowserViewDelegate()
        wireDevToolsViewDelegate()
        wireDockObserver()
        wireCommand()
    }

    /// The inspector's browser is CEF's own, so its protocol traffic arrives on
    /// an observer of its own rather than the one attached to the page.
    private func wireDockObserver() {
        guard let dockObserver else { return }
        dockObserver.pointee.on_dev_tools_method_result = { selfPointer, browser, messageID, success, _, _ in
            nd_cef_ref_release(browser)
            let ok = success != 0
            ndCefDeliver(selfPointer) { $0?.chrome?.frontendResult(id: messageID, ok: ok) }
        }
        dockObserver.pointee.on_dev_tools_event = { selfPointer, browser, method, params, paramsSize in
            let name = ndCefString(method)
            nd_cef_ref_release(browser)
            let raw = ndCefJSONText(params, paramsSize)
            ndCefDeliver(selfPointer) { $0?.chrome?.frontendEvent(method: name, json: raw) }
        }
    }

    /// The docked DevTools has to be a Chrome style BrowserView: CEF refuses an Alloy DevTools popup
    /// ("only Chrome style is supported for DevTools popups",
    /// libcef/browser/views/browser_view_impl.cc) and the BrowserView it hands
    /// back is then half-built, which is what made the inspector impossible to
    /// close and the quit after it fault.
    private func wireDevToolsViewDelegate() {
        guard let devToolsViewDelegate else { return }
        devToolsViewDelegate.pointee.get_browser_runtime_style = { _ in CEF_RUNTIME_STYLE_CHROME }
        devToolsViewDelegate.pointee.get_chrome_toolbar_type = { _, browserView in
            nd_cef_ref_release(browserView)
            return CEF_CTT_NONE
        }
        // The inspector's browser is CEF's own, so this client's
        // `on_before_close` never reports it. This is the one callback that
        // does, and the frontend's own close button is a path that reaches the
        // dock through nothing else.
        devToolsViewDelegate.pointee.on_browser_destroyed = { selfPointer, browserView, browser in
            nd_cef_ref_release(browserView)
            nd_cef_ref_release(browser)
            ndCefDeliver(selfPointer) { $0?.chrome?.devToolsBrowserClosed() }
        }
    }

    private func wireWindowDelegate() {
        guard let windowDelegate else { return }
        Self.wireAnchorWindow(windowDelegate, devTools: false)
        if let devToolsWindowDelegate { Self.wireAnchorWindow(devToolsWindowDelegate, devTools: true) }
    }

    /// A frameless, fixed Chrome style window placed where its column of the
    /// webview already is, so the anchor never appears at the screen origin
    /// before the first sync.
    private static func wireAnchorWindow(_ delegate: UnsafeMutablePointer<cef_window_delegate_t>, devTools: Bool) {
        delegate.pointee.get_window_runtime_style = { _ in CEF_RUNTIME_STYLE_CHROME }
        delegate.pointee.is_frameless = { _, window in
            nd_cef_ref_release(window)
            return 1
        }
        delegate.pointee.with_standard_window_buttons = { _, window in
            nd_cef_ref_release(window)
            return 0
        }
        delegate.pointee.can_resize = { _, window in
            nd_cef_ref_release(window)
            return 0
        }
        delegate.pointee.can_maximize = { _, window in
            nd_cef_ref_release(window)
            return 0
        }
        delegate.pointee.can_minimize = { _, window in
            nd_cef_ref_release(window)
            return 0
        }
        delegate.pointee.get_initial_bounds = { selfPointer, window in
            nd_cef_ref_release(window)
            var bounds = cef_rect_t(x: 0, y: 0, width: 800, height: 600)
            ndCefDeliver(selfPointer) { view in
                guard let view, let host = view.window else { return }
                let rect = host.convertToScreen(view.convert(view.bounds, to: nil))
                guard rect.width >= 1, rect.height >= 1 else { return }
                bounds = ndCefDipRect(rect)
            }
            return bounds
        }
        if devTools {
            delegate.pointee.on_window_created = { selfPointer, window in
                guard let window else { return }
                nd_cef_ref_add(window)
                let kept = UInt(bitPattern: window)
                ndCefDeliver(selfPointer) { view in
                    guard let created = UnsafeMutableRawPointer(bitPattern: kept)?
                        .assumingMemoryBound(to: cef_window_t.self) else { return }
                    if let chrome = view?.chrome {
                        chrome.devToolsWindowCreated(created)
                    } else {
                        created.pointee.close?(created)
                        nd_cef_ref_release(created)
                    }
                }
                nd_cef_ref_release(window)
            }
        } else {
            delegate.pointee.on_window_created = { selfPointer, window in
                guard let window else { return }
                nd_cef_ref_add(window)
                let kept = UInt(bitPattern: window)
                ndCefDeliver(selfPointer) { view in
                    guard let created = UnsafeMutableRawPointer(bitPattern: kept)?
                        .assumingMemoryBound(to: cef_window_t.self) else { return }
                    if let chrome = view?.chrome {
                        chrome.windowCreated(created)
                    } else {
                        nd_cef_ref_release(created)
                    }
                }
                nd_cef_ref_release(window)
            }
        }
    }

    private func wireBrowserViewDelegate() {
        guard let browserViewDelegate else { return }
        browserViewDelegate.pointee.get_browser_runtime_style = { _ in CEF_RUNTIME_STYLE_CHROME }
        browserViewDelegate.pointee.get_chrome_toolbar_type = { _, browserView in
            nd_cef_ref_release(browserView)
            return CEF_CTT_NONE
        }
        // Document picture-in-picture is a Chromium-owned top-level window with
        // no other way to refuse it.
        browserViewDelegate.pointee.use_frameless_window_for_picture_in_picture = { _, browserView in
            nd_cef_ref_release(browserView)
            return 0
        }
        browserViewDelegate.pointee.allow_move_for_picture_in_picture = { _, browserView in
            nd_cef_ref_release(browserView)
            return 0
        }
        browserViewDelegate.pointee.allow_picture_in_picture_without_user_activation = {
            _, browserView in
            nd_cef_ref_release(browserView)
            return 0
        }
        // DevTools is the one popup BrowserView that is allowed to exist, and
        // it goes into a frameless window of this host's, not a default one.
        browserViewDelegate.pointee.get_delegate_for_popup_browser_view = {
            selfPointer, browserView, _, client, isDevTools in
            nd_cef_ref_release(browserView)
            nd_cef_ref_release(client)
            guard isDevTools != 0 else { return nil }
            return ndCefHandOut(ndCefBox(selfPointer)?.devToolsViewDelegate)
        }
        browserViewDelegate.pointee.on_popup_browser_view_created = {
            selfPointer, browserView, popup, isDevTools in
            nd_cef_ref_release(browserView)
            guard isDevTools != 0, let popup else {
                nd_cef_ref_release(popup)
                return 0
            }
            nd_cef_ref_add(popup)
            let kept = UInt(bitPattern: popup)
            var docked: Int32 = 0
            ndCefDeliver(selfPointer) { view in
                guard let created = UnsafeMutableRawPointer(bitPattern: kept)?
                    .assumingMemoryBound(to: cef_browser_view_t.self) else { return }
                if view?.chrome?.dockDevTools(created) == true {
                    docked = 1
                } else {
                    nd_cef_ref_release(created)
                }
            }
            nd_cef_ref_release(popup)
            // Returning 0 would hand the BrowserView a cef_window_t of its own.
            return docked
        }
    }

    /// Every Chrome command that would open a window of its own, refused. The
    /// numbers move between Chromium versions, so the set is resolved from
    /// cef_command_ids.h names at load.
    private func wireCommand() {
        guard let command else { return }
        command.pointee.on_chrome_command = { _, browser, commandID, _ in
            nd_cef_ref_release(browser)
            return ndCefBlockedChromeCommands.contains(commandID) ? 1 : 0
        }
    }
}

/// Every Chrome command that would open a window of its own, or answer with a
/// bubble anchored to browser chrome this engine does not have. The context menu
/// drops every item carrying one of these (`ndCefMenuItemDenied`), except the
/// open-link ones, which are rerouted to the app's `newWindow`.
let ndCefBlockedChromeCommands: Set<Int32> = ndCefCommandIDs([
    "IDC_NEW_WINDOW", "IDC_NEW_INCOGNITO_WINDOW", "IDC_NEW_TAB", "IDC_NEW_TAB_TO_RIGHT",
    "IDC_RESTORE_TAB", "IDC_MOVE_TAB_TO_NEW_WINDOW", "IDC_WINDOW_CLOSE",
    "IDC_OPEN_IN_CHROME", "IDC_TASK_MANAGER", "IDC_VIEW_SOURCE",
    "IDC_DEV_TOOLS", "IDC_DEV_TOOLS_CONSOLE", "IDC_DEV_TOOLS_DEVICES",
    "IDC_DEV_TOOLS_INSPECT", "IDC_DEV_TOOLS_TOGGLE",
    "IDC_PRINT", "IDC_BASIC_PRINT", "IDC_SHOW_DOWNLOADS", "IDC_SHOW_HISTORY",
    "IDC_SHOW_BOOKMARK_MANAGER", "IDC_BOOKMARK_THIS_TAB", "IDC_OPTIONS", "IDC_ABOUT",
    "IDC_MANAGE_EXTENSIONS", "IDC_CLEAR_BROWSING_DATA", "IDC_FEEDBACK",
    "IDC_HELP_PAGE_VIA_MENU", "IDC_SHOW_SIGNIN", "IDC_UPGRADE_DIALOG",
    "IDC_SHOW_APP_MENU", "IDC_WINDOW_MENU_NEW_TAB", "IDC_WINDOW_MENU_NEW_WINDOW",
    "IDC_WINDOW_MENU_NEW_INCOGNITO_WINDOW", "IDC_OPEN_FILE",
    "IDC_CONTENT_CONTEXT_OPENLINKNEWTAB", "IDC_CONTENT_CONTEXT_OPENLINKNEWWINDOW",
    "IDC_CONTENT_CONTEXT_OPENLINKOFFTHERECORD", "IDC_CONTENT_CONTEXT_OPENLINKINPROFILE",
    "IDC_CONTENT_CONTEXT_OPENLINKBOOKMARKAPP", "IDC_CONTENT_CONTEXT_OPENIMAGENEWTAB",
    "IDC_CONTENT_CONTEXT_OPENAVNEWTAB", "IDC_CONTENT_CONTEXT_PICTUREINPICTURE",
    "IDC_CONTENT_CONTEXT_VIEWFRAMESOURCE", "IDC_CONTENT_CONTEXT_VIEWPAGESOURCE",
    "IDC_CONTENT_CONTEXT_PRINT", "IDC_ROUTE_MEDIA", "IDC_CONTENT_CONTEXT_GENERATE_QR_CODE",
    "IDC_CONTENT_CONTEXT_SEARCHLENSFORIMAGE", "IDC_CONTENT_CONTEXT_TRANSLATE",
])

/// AppKit screen coordinates to the DIP screen rectangle CEF's Views layer
/// uses: same unit on this platform, but Chromium's origin is the top-left of
/// the primary display and AppKit's is its bottom-left.
func ndCefDipRect(_ rect: NSRect) -> cef_rect_t {
    let primaryHeight = NSScreen.screens.first?.frame.height ?? rect.maxY
    return cef_rect_t(
        x: Int32(rect.minX.rounded()),
        y: Int32((primaryHeight - rect.maxY).rounded()),
        width: Int32(max(1, rect.width.rounded())),
        height: Int32(max(1, rect.height.rounded()))
    )
}
#endif
