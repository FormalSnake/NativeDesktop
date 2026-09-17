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
    private var devToolsView: UnsafeMutablePointer<cef_browser_view_t>?
    /// The inspector's view between `close_dev_tools` and its browser actually
    /// going away.
    private var devToolsClosing: UnsafeMutablePointer<cef_browser_view_t>?
    private var devToolsCloseTimer: Timer?
    /// Inspector views whose browser has gone. They stay children of the window
    /// for the rest of its life: `remove_child_view` on one of these poisons the
    /// next `show_dev_tools` or `close_dev_tools`, which then hangs the UI
    /// thread outright. Asking the box layout for a width of zero instead leaves
    /// them costing a pointer each and nothing on screen.
    private var closedDevToolsViews: [UnsafeMutablePointer<cef_browser_view_t>] = []
    private weak var anchor: NSWindow?
    /// The Chromium subtree now living in `view`, and the superview it came
    /// from. Chromium re-attaches its own native view host on some layout and
    /// navigation paths, which puts the subtree back; `relift` is what notices.
    private weak var lifted: NSView?
    private var reliftCount = 0
    private var reliftTimer: Timer?
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

    /// Any Views window still open. The quit waits on this after the browsers
    /// have gone: `cef_window_t::close` is asynchronous, and a window still
    /// unwinding when `cef_shutdown` runs takes the process out through its own
    /// activation path.
    static var anyWindowOpen: Bool {
        live.allObjects.contains { $0.cefWindow != nil }
    }

    static func closeDockedDevTools() {
        for window in live.allObjects { window.closeDevTools() }
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

        guard let view else { return }
        guard let handle = window.pointee.get_window_handle?(window) else {
            ndCefWarn("chrome style: the Views window reported no native handle")
            return
        }
        let content = Unmanaged<NSView>.fromOpaque(handle).takeUnretainedValue()
        guard let anchorWindow = content.window else {
            ndCefWarn("chrome style: the Views window has no NSWindow")
            return
        }
        anchor = anchorWindow
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
        // Chromium's views hierarchy only produces frames for a widget it
        // believes is showing, so the window is shown for real and made
        // imperceptible instead of being left hidden.
        let wasKey = view.window?.isKeyWindow ?? false
        window.pointee.show?(window)
        if wasKey { view.window?.makeKey() }
        syncAnchor()
        observeGeometry()
        view.ndTrace("chrome anchored \(anchorWindow.frame) target=\(targetScreenFrame().map(\.debugDescription) ?? "none")")
        if ProcessInfo.processInfo.environment["ND_CEF_DUMP_VIEWS"] == "1" { dumpViews() }
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

    /// Docks the DevTools BrowserView beside the page inside the same Views
    /// window, which is what puts its web contents under the NSView already
    /// lifted into the host. Chrome's own dock is a WebContents split inside
    /// the browser window; this is the same shape, drawn by CEF's box layout.
    func dockDevTools(_ popup: UnsafeMutablePointer<cef_browser_view_t>) -> Bool {
        guard !closed, let cefWindow, let browserView else { return false }
        if let devToolsView {
            nd_cef_ref_release(devToolsView)
            self.devToolsView = nil
        }
        let panel = UnsafeMutableRawPointer(cefWindow).assumingMemoryBound(to: cef_panel_t.self)
        var settings = cef_box_layout_settings_t()
        settings.size = MemoryLayout<cef_box_layout_settings_t>.size
        settings.horizontal = 1
        // The split ratio comes from the two delegates' get_preferred_size,
        // which always add up to the window width, rather than from per-view
        // flex: cef_box_layout_t::set_flex_for_view segfaults on a BrowserView
        // passed through its cef_view_t base.
        settings.default_flex = 0
        guard let layout = panel.pointee.set_to_box_layout?(panel, &settings) else { return false }
        defer { nd_cef_ref_release(UnsafeMutableRawPointer(layout)) }
        // Two references: one for `add_child_view`, which takes the one it is
        // passed, and one for the field below. The reference the callback
        // arrived with is balanced by its own release, so without this the
        // field owns nothing and releasing it later lands on freed memory.
        nd_cef_ref_add(popup)
        nd_cef_ref_add(popup)
        let tools = UnsafeMutableRawPointer(popup).assumingMemoryBound(to: cef_view_t.self)
        panel.pointee.add_child_view?(panel, tools)
        devToolsView = popup
        view?.ndTrace("chrome devtools docked")
        return true
    }

    /// Undoes `dockDevTools`. Nothing in the window's layout is touched here:
    /// `devToolsView` going nil is already enough for the delegate to ask the
    /// box layout for a width of zero, and the page fills the window again as
    /// soon as `devToolsBrowserClosed` relays it out.
    func closeDevTools() {
        guard let docked = devToolsView else { return }
        devToolsView = nil
        devToolsClosing = docked
        if let browserHost = view?.browserHost() {
            browserHost.pointee.close_dev_tools?(browserHost)
            nd_cef_ref_release(browserHost)
        }
        view?.ndTrace("chrome devtools closing")
        // CEF creates the inspector's browser with a client of its own, so this
        // client's `on_before_close` never fires for it. The BrowserView losing
        // its browser is the same signal seen from the only side that has one.
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
                self.devToolsCloseTimer?.invalidate()
                self.devToolsCloseTimer = nil
                self.devToolsBrowserClosed()
            }
        }
    }

    /// The inspector's browser has gone: the window can be laid out again and
    /// the page takes the whole width back.
    func devToolsBrowserClosed() {
        guard let closing = devToolsClosing else { return }
        closedDevToolsViews.append(closing)
        devToolsClosing = nil
        if let cefWindow {
            let panel = UnsafeMutableRawPointer(cefWindow).assumingMemoryBound(to: cef_panel_t.self)
            panel.pointee.layout?(panel)
        }
        view?.ndTrace("chrome devtools closed")
    }

    /// Chrome docks DevTools to the right at about a third of the contents
    /// width; the page takes what is left, so the two always add up to the
    /// window and the box layout leaves no gap.
    func dockedSize(devTools: Bool, view asked: UnsafeMutableRawPointer?) -> cef_size_t {
        let bounds = view?.bounds ?? .zero
        let width = Int32(bounds.width.rounded())
        let height = Int32(bounds.height.rounded())
        // A view left behind by a closed inspector takes no width at all.
        if let asked, closedDevToolsViews.contains(where: { UnsafeMutableRawPointer($0) == asked }) {
            return cef_size_t(width: 0, height: max(1, height))
        }
        guard devToolsView != nil else {
            return cef_size_t(width: devTools ? 0 : max(1, width), height: max(1, height))
        }
        let tools = max(1, width / 3)
        return cef_size_t(width: devTools ? tools : max(1, width - tools), height: max(1, height))
    }

    // MARK: - Lifting the web contents

    /// Moves Chromium's whole widget surface into the host view, or puts it
    /// back after Chromium re-attached it to the anchor.
    ///
    /// The lift is the anchor's content view, not the page's own
    /// WebContentsViewCocoa. Chromium's views are not NSViews: the only NSViews
    /// under a Views window are the compositor superview and whatever a
    /// views::NativeViewHost attaches, so the content view is the one node that
    /// holds all of them. Docked DevTools is a second WebContents in the same
    /// container, and lifting the container is what brings it along instead of
    /// leaving it drawing into a window nobody can see.
    private func liftWebContents() {
        guard !closed, let anchor, let view else { return }
        if let lifted {
            guard lifted.superview !== view else { return }
            reliftCount += 1
            view.ndTrace("chrome relift #\(reliftCount)")
            lifted.removeFromSuperview()
            lifted.frame = view.bounds
            view.addSubview(lifted)
            return
        }
        if ProcessInfo.processInfo.environment["ND_CEF_DUMP_VIEWS"] == "1" { dumpViews() }
        guard let content = anchor.contentView else { return }
        view.ndTrace("chrome lift \(NSStringFromClass(type(of: content)))")
        lifted = content
        // AppKit keeps a content view either way, so the anchor is handed an
        // empty one rather than left pointing at a view in another window.
        anchor.contentView = NSView(frame: content.frame)
        content.frame = view.bounds
        content.autoresizingMask = [.width, .height]
        view.addSubview(content)
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
    }

    /// The webview's own rectangle in AppKit screen coordinates, or nil when
    /// the view is not on screen.
    private func targetScreenFrame() -> NSRect? {
        guard let view, let host = view.window, !view.isHiddenOrHasHiddenAncestor else { return nil }
        let rect = host.convertToScreen(view.convert(view.bounds, to: nil))
        return (rect.width >= 1 && rect.height >= 1) ? rect : nil
    }

    private func syncAnchor() {
        guard !closed, let anchor, let cefWindow else { return }
        guard let host = view?.window, let rect = targetScreenFrame() else {
            if anchor.isVisible { cefWindow.pointee.hide?(cefWindow) }
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
        if !anchor.isVisible { cefWindow.pointee.show?(cefWindow) }
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
        if let anchorWindow = anchor {
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
        reliftTimer?.invalidate()
        reliftTimer = nil
        devToolsCloseTimer?.invalidate()
        devToolsCloseTimer = nil
        // The views closed inspectors left behind come out of the window here,
        // with nothing else in flight: leaving one in the layout means Chromium
        // walks a BrowserView with no browser while it destroys the window, and
        // the quit dies in its own activation path.
        if let cefWindow {
            let panel = UnsafeMutableRawPointer(cefWindow).assumingMemoryBound(to: cef_panel_t.self)
            for stale in closedDevToolsViews {
                let tools = UnsafeMutableRawPointer(stale).assumingMemoryBound(to: cef_view_t.self)
                panel.pointee.remove_child_view?(panel, tools)
            }
        }
        for stale in closedDevToolsViews { nd_cef_ref_release(stale) }
        closedDevToolsViews = []
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
        command = ndCefAlloc(cef_command_handler_t.self, self)
        wireWindowDelegate()
        wireBrowserViewDelegate()
        wireDevToolsViewDelegate()
        wireCommand()
    }

    /// The docked DevTools is the second BrowserView in the page's window, and
    /// it has to be a Chrome style one: CEF refuses an Alloy DevTools popup
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
        devToolsViewDelegate.pointee.base.get_preferred_size = { selfPointer, cefView in
            let asked = UnsafeMutableRawPointer(cefView)
            nd_cef_ref_release(cefView)
            return ndCefPreferredSize(selfPointer, devTools: true, view: asked)
        }
    }

    private func wireWindowDelegate() {
        guard let windowDelegate else { return }
        windowDelegate.pointee.get_window_runtime_style = { _ in CEF_RUNTIME_STYLE_CHROME }
        windowDelegate.pointee.is_frameless = { _, window in
            nd_cef_ref_release(window)
            return 1
        }
        windowDelegate.pointee.with_standard_window_buttons = { _, window in
            nd_cef_ref_release(window)
            return 0
        }
        windowDelegate.pointee.can_resize = { _, window in
            nd_cef_ref_release(window)
            return 0
        }
        windowDelegate.pointee.can_maximize = { _, window in
            nd_cef_ref_release(window)
            return 0
        }
        windowDelegate.pointee.can_minimize = { _, window in
            nd_cef_ref_release(window)
            return 0
        }
        // Placed where the webview already is, so the anchor never appears at
        // the screen origin before the first sync.
        windowDelegate.pointee.get_initial_bounds = { selfPointer, window in
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
        windowDelegate.pointee.on_window_created = { selfPointer, window in
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

    private func wireBrowserViewDelegate() {
        guard let browserViewDelegate else { return }
        browserViewDelegate.pointee.get_browser_runtime_style = { _ in CEF_RUNTIME_STYLE_CHROME }
        browserViewDelegate.pointee.base.get_preferred_size = { selfPointer, cefView in
            nd_cef_ref_release(cefView)
            return ndCefPreferredSize(selfPointer, devTools: false, view: nil)
        }
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
        // it is taken into the page's own window rather than given one.
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

/// Resolved once: `cef_id_for_command_id_name` answers -1 for a name this
/// build does not know, which is a name that cannot be triggered either.
let ndCefBlockedChromeCommands: Set<Int32> = {
    let names = [
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
        "IDC_WINDOW_MENU_NEW_INCOGNITO_WINDOW",
    ]
    var blocked: Set<Int32> = []
    for name in names {
        let id = name.withCString { nd_cef_command_id($0) }
        if id >= 0 { blocked.insert(id) }
    }
    return blocked
}()

/// The size one of the window's two BrowserViews asks the box layout for.
private func ndCefPreferredSize(
    _ handler: UnsafeMutableRawPointer?, devTools: Bool, view asked: UnsafeMutableRawPointer?
) -> cef_size_t {
    var size = cef_size_t(width: 0, height: 0)
    // The pointer crosses the isolation boundary as a bit pattern, the same way
    // every other CEF pointer does here: Swift's raw pointers are not Sendable.
    let token = UInt(bitPattern: asked)
    ndCefDeliver(handler) { view in
        guard let chrome = view?.chrome else { return }
        size = chrome.dockedSize(devTools: devTools, view: UnsafeMutableRawPointer(bitPattern: token))
    }
    return size
}

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
