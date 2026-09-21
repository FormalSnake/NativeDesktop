#if canImport(CCef)
import AppKit
import CCef
import Foundation

/// The Chromium (CEF) surface behind `<webview>`, presenting the same internal
/// contract as the WKWebView in NDWebView.swift: `ndSetURL`, an `ndHandleCommand`
/// switch, and the schema's events out through `nd_emit_event`. NDWebView owns
/// one of these and forwards to it when the resolved engine is chromium, so the
/// generated widget code (NDGen/Widgets.swift) never learns there are two
/// engines.
///
/// Embedding is Alloy-style and windowed: CEF creates its own NSView inside
/// this one. The browser is only created once this view is in a window, which
/// is not a nicety. An unparented create silently produces a top-level
/// Chromium window over the app, the one failure the spec bans outright.
///
/// Where the WKWebView surface polls the view's navigation properties, this
/// one is push-only: CEF's display and load handlers report the same state
/// changes directly, so there is no timer here.
///
/// The scriptable half of the contract (evaluate, user scripts, worlds, script
/// messages, the framework's own page-side agent) lives in NDCefScripts.swift
/// on top of the DevTools substrate; profiles, cookies and custom schemes live
/// in NDCefProfiles.swift.
final class NDCefWebView: NSView {
    /// The `<webview>` node this view reports as. Emits go through it because
    /// the node id and the event names belong to the widget, not the engine.
    weak var host: NDWebView?

    /// Retained by every handler struct below and holding this view weakly, so
    /// a callback arriving after the view is gone resolves to nothing instead
    /// of a freed object. CEF outlives the view whenever a browser is still
    /// closing.
    nonisolated(unsafe) let box = NDCefHandlerBox()

    nonisolated(unsafe) private var browser: UnsafeMutablePointer<cef_browser_t>?
    private var createRequested = false
    /// The address the app last asked for, and the one that last COMMITTED.
    /// BOTH are "where this view already is". The `url` prop is an echo: the
    /// engine reports an address, the app stores it, and storing it re-applies
    /// the prop, so an app is always one event behind and re-asks for whichever
    /// of the two it last heard about. A guard that knows only one of them
    /// turns that into an endless load storm. Peer of NDWebView's
    /// url/committedURL pair.
    private var requestedURL: String
    private var committedURL = ""
    /// What the browser was actually created with. A `url` prop applied while
    /// the browser was still being created lands in `requestedURL` alone, so
    /// adoption has to reconcile the two or the view sits on the old address
    /// forever. Mounting a view with `url=""` and arming it on the next commit
    /// is the normal shape for a tab, a background page or a popup.
    private var createdURL = ""
    /// A load asked for before the browser had a main frame to load it into.
    /// CEF hands one over a beat after `on_after_created`, so dropping the
    /// address (which is what used to happen) left a tab armed one commit
    /// after mount sitting on about:blank.
    private var deferredURL = ""
    /// The `profile` prop, resolved to a request context once at create time.
    private let profile: String

    lazy var devTools = NDCefDevTools(view: self)

    /// Non-nil under `ND_CEF_STYLE=chrome`: the Views window the browser was
    /// born in, and the lift of its web contents into this view.
    var chrome: NDCefChromeWindow?

    // Last-emitted navigation state; events fire only on change, matching the
    // WKWebView surface. It doubles as the answer to the automation
    // `webviewInfo` RPC, which has no engine-side property to read here.
    private var lastTitle = ""
    private var lastLoading = false
    private var lastCanGoBack = false
    private var lastCanGoForward = false
    private var lastProgress: Double = -1
    private var lastSecure: Bool?

    // Script state (NDCefScripts.swift).
    var userScripts: [NDCefUserScript] = []
    var scriptIdentifiers: [String: String] = [:]
    /// Bumped whenever a key is removed or replaced, so an install whose
    /// identifier arrives after the fact knows it was superseded.
    var scriptGenerations: [String: Int] = [:]
    var messageChannels: Set<NDCefMessageChannel> = []
    var boundWorlds: Set<String> = []
    var suppressContextMenu: Bool
    var contextMenuItems: [NDContextMenuItem] = []
    var lastMenuHit = NDContextMenuHit()
    var lastHoveredLink = ""
    var pendingFindText = ""
    /// The app's context-menu items keyed by the command id they were given.
    /// CEF reserves MENU_ID_USER_FIRST upward for the client, so nothing here
    /// can collide with Chromium's own commands.
    var contextMenuCommands: [Int32: NDContextMenuItem] = [:]
    var nextContextMenuCommand: Int32 = 26500
    /// The menu this view is showing, and the link the click that raised it
    /// landed on. Chrome's "open link in …" items are answered with the app's
    /// `newWindow`, and this is the only place that URL is still in hand.
    private var menuRun: NDCefContextMenuRun?
    private var menuLinkURL = ""
    /// Newest `Page.captureScreenshot` PNG, kept because the automation
    /// snapshot ladder is synchronous and Chromium's content lives in a remote
    /// layer that AppKit's own render paths cannot draw.
    var cachedFrame: NSImage?

    init(url: String, profile: String, contextMenuMode: String) {
        self.requestedURL = url
        self.profile = profile
        self.suppressContextMenu = contextMenuMode == "suppress"
        super.init(frame: .zero)
        box.view = self
        box.build()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("NDCefWebView is not NSCoding-decodable") }

    deinit {
        // The CEF teardown is NOT done here. Closing a Views window and handing
        // Chromium its own content view back both reach into a view hierarchy
        // that has to still exist, and a `deinit` runs from an autorelease pool
        // pop with this object already destroyed: the same sequence faults
        // inside CEF. `ndCefPurge` runs it at release_node instead, while
        // everything is alive.
        MainActor.assumeIsolated { releaseEngine() }
        box.teardown()
    }

    /// Closes the browser and, under Chrome style, the window it was born in.
    /// Idempotent: the release_node seam calls it, and `deinit` calls it again
    /// for a view that was dropped without one.
    ///
    /// Ordering, the same one the Linux engine takes: the inspector's browser
    /// goes first, then the lifted subtree goes home, then the page browser.
    /// Closing the page browser while the inspector is still attached leaves
    /// CEF unwinding the inspected browser first, which is the
    /// `CefBrowserInfo::RemoveFrame` fault.
    @MainActor func releaseEngine() {
        cancelContextMenu()
        closeDevToolsForShutdown()
        chrome?.teardown()
        guard let browser else {
            chrome?.closeWindow()
            chrome = nil
            return
        }
        self.browser = nil
        if let browserHost = browser.pointee.get_host?(browser) {
            browserHost.pointee.close_browser?(browserHost, 1)
            nd_cef_ref_release(browserHost)
        }
        nd_cef_ref_release(browser)
        ndTrace("engine released")
    }

    /// `close_dev_tools` on this view, whichever style it runs.
    @MainActor func closeDevToolsForShutdown() {
        chrome?.closeDevTools()
        guard let browserHost = browserHost() else { return }
        defer { nd_cef_ref_release(browserHost) }
        if browserHost.pointee.has_dev_tools?(browserHost) != 0 {
            browserHost.pointee.close_dev_tools?(browserHost)
        }
    }

    /// Still inspecting. The quit sequence waits on this rather than on a
    /// fixed delay: an inspector that has not reported its `on_before_close`
    /// is one CEF is still unwinding.
    @MainActor var stillInspecting: Bool {
        if sideBrowsers > 0 { return true }
        guard let browserHost = browserHost() else { return false }
        defer { nd_cef_ref_release(browserHost) }
        return browserHost.pointee.has_dev_tools?(browserHost) != 0
    }

    @MainActor var stillOpen: Bool { browser != nil }

    /// What each phase of the ordered quit is waiting on, for the warning a
    /// phase that ran out of time prints.
    @MainActor var shutdownState: String {
        let host = browserHost()
        defer { if let host { nd_cef_ref_release(host) } }
        let devTools = host.map { $0.pointee.has_dev_tools?($0) != 0 } ?? false
        return "node=\(self.host?.ndNodeID ?? 0) browser=\(browser != nil) side=\(sideBrowsers) devtools=\(devTools)"
            + " docked=\(chrome?.hasDockedDevTools ?? false)"
    }

    /// Whether a callback's browser is this view's page browser rather than the
    /// inspector's, which shares the client.
    @MainActor func ownsBrowser(_ identifier: Int32) -> Bool {
        guard let browser else { return false }
        return browser.pointee.get_identifier?(browser) == identifier
    }

    // MARK: - Browser lifetime

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        createBrowserIfNeeded()
        chrome?.hostGeometryChanged()
    }

    /// The autoresizing pass, deliberately NOT `layout()`. Setting a subview's
    /// frame from `layout()` marks it as needing a constraint update while the
    /// window is inside its own constraint pass, and AppKit answers that by
    /// raising out of `_postWindowNeedsUpdateConstraints`, which NSApplication
    /// turns into a crash. CEF's view carries an autoresizing mask, so this
    /// only reconciles the case where it was created before this view had a
    /// real size.
    override func resizeSubviews(withOldSize oldSize: NSSize) {
        super.resizeSubviews(withOldSize: oldSize)
        for child in subviews where child.frame != bounds { child.frame = bounds }
        chrome?.hostGeometryChanged()
    }

    override func viewDidHide() {
        super.viewDidHide()
        setBrowserHidden(true)
        chrome?.hostGeometryChanged()
    }

    override func viewDidUnhide() {
        super.viewDidUnhide()
        setBrowserHidden(false)
        chrome?.hostGeometryChanged()
    }

    private func createBrowserIfNeeded() {
        guard !createRequested, NDCefRuntime.isActive else { return }
        guard window != nil else { return }
        createRequested = true

        createdURL = requestedURL.isEmpty ? "about:blank" : requestedURL
        if NDCefRuntime.isChromeStyle {
            let window = NDCefChromeWindow(view: self)
            chrome = window
            window.start(url: createdURL, profile: profile)
            return
        }

        var info = cef_window_info_t()
        info.size = MemoryLayout<cef_window_info_t>.size
        info.bounds = cef_rect_t(
            x: 0,
            y: 0,
            width: Int32(max(1, bounds.width)),
            height: Int32(max(1, bounds.height))
        )
        info.parent_view = Unmanaged.passUnretained(self).toOpaque()
        // Explicit rather than inherited: parent_view already forces Alloy,
        // and saying so keeps a future default change from moving this view
        // into a CEF-owned window.
        info.runtime_style = CEF_RUNTIME_STYLE_ALLOY

        var settings = cef_browser_settings_t()
        settings.size = MemoryLayout<cef_browser_settings_t>.size

        var url = cef_string_t()
        ndCefSetString(createdURL, &url)
        defer { nd_cef_string_clear(&url) }

        // Both the client and the request context are handed over: the library
        // owns what it is passed, and this view keeps its own reference.
        let context = NDCefProfiles.context(for: profile)
        nd_cef_ref_add(box.client)
        nd_cef_ref_add(context)
        if nd_cef_create_browser(&info, box.client, &url, &settings, nil, context) == 0 {
            ndCefWarn("cef_browser_host_create_browser failed")
        }
    }

    /// Browsers on this client that are not the page's: the docked inspector is
    /// the only one, and the ordered quit waits for it to report its own
    /// `on_before_close` before the page browser is closed.
    private var sideBrowsers = 0

    fileprivate func adoptBrowser(_ created: UnsafeMutablePointer<cef_browser_t>) {
        guard browser == nil else {
            sideBrowsers += 1
            nd_cef_ref_release(created)
            return
        }
        browser = created
        NDCefWebView.liveViews.add(self)
        guard let browserHost = created.pointee.get_host?(created) else { return }
        defer { nd_cef_ref_release(browserHost) }
        // The observer has to be attached before the first method call, or the
        // reply has nowhere to land.
        nd_cef_ref_add(box.devToolsObserver)
        box.attachDevTools(browserHost.pointee.add_dev_tools_message_observer?(browserHost, box.devToolsObserver))
        devTools.start()
        ndInstallScripts()
        NDCefSchemeRouter.register(self, browser: created)
        if !requestedURL.isEmpty, requestedURL != createdURL { loadInMainFrame(requestedURL) }
        if let chrome {
            chrome.browserCreated(host: browserHost)
            return
        }
        guard let handle = browserHost.pointee.get_window_handle?(browserHost) else { return }
        let view = Unmanaged<NSView>.fromOpaque(handle).takeUnretainedValue()
        view.frame = bounds
        view.autoresizingMask = [.width, .height]
        if isHiddenOrHasHiddenAncestor {
            browserHost.pointee.was_hidden?(browserHost, 1)
        }
    }

    fileprivate func forgetBrowser(identifier: Int32) {
        guard let browser else { return }
        let mine = browser.pointee.get_identifier?(browser) ?? -1
        guard mine == identifier else {
            sideBrowsers = max(0, sideBrowsers - 1)
            chrome?.devToolsBrowserClosed()
            return
        }
        self.browser = nil
        nd_cef_ref_release(browser)
        // The Views window is the browser's; it goes now that the browser has
        // reported itself closed.
        chrome?.closeWindow()
        chrome = nil
    }

    /// Every view with a live browser, for the ordered quit below.
    nonisolated(unsafe) static let liveViews = NSHashTable<NDCefWebView>.weakObjects()

    private func setBrowserHidden(_ hidden: Bool) {
        guard let browserHost = browserHost() else { return }
        defer { nd_cef_ref_release(browserHost) }
        browserHost.pointee.was_hidden?(browserHost, hidden ? 1 : 0)
    }

    /// The browser's host, with a reference the CALLER owns. Every use is
    /// paired with `nd_cef_ref_release`.
    func browserHost() -> UnsafeMutablePointer<cef_browser_host_t>? {
        guard let browser else { return nil }
        return browser.pointee.get_host?(browser)
    }

    var hasBrowser: Bool { browser != nil }

    /// A plain NSView refuses first responder by default, and this one has to
    /// take it: the widget's `focus` command lands on the NDWebView above,
    /// which forwards here, and the a11y probe reports focus for any
    /// descendant. Chromium routes the keystrokes itself once told.
    override var acceptsFirstResponder: Bool { true }

    /// The app's own menu gets first refusal on the key equivalents it
    /// declared. `-[NSWindow sendEvent:]` walks the view hierarchy before it
    /// reaches the main menu, and Chromium answers YES to cmd+W, cmd+T and the
    /// rest of its own accelerators, so without this the app's menu never sees
    /// them: the command handler refuses them on Chromium's side and the
    /// keystroke is lost. Only DECLARED chords are taken; the default Edit
    /// items forward to a responder chain the web contents is not part of, so
    /// cmd+C, cmd+V, cmd+A and cmd+Z stay Chromium's.
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        if ndMenuDeclaredChords.contains(ndEventChord(event)),
           NSApp.mainMenu?.performKeyEquivalent(with: event) == true {
            return true
        }
        return super.performKeyEquivalent(with: event)
    }

    override func becomeFirstResponder() -> Bool {
        if let target = chrome?.focusTarget, target !== self {
            return window?.makeFirstResponder(target) ?? false
        }
        if let browserHost = browserHost() {
            browserHost.pointee.set_focus?(browserHost, 1)
            nd_cef_ref_release(browserHost)
        }
        return true
    }

    override func resignFirstResponder() -> Bool {
        if let browserHost = browserHost() {
            browserHost.pointee.set_focus?(browserHost, 0)
            nd_cef_ref_release(browserHost)
        }
        return true
    }

    // MARK: - Props and commands

    /// createAndUpdate `url` prop. Same echo guard as the WKWebView surface:
    /// the app feeds `onNavigate` back into the prop, so a load only starts
    /// when the address actually differs from where the view already is.
    func ndSetURL(_ value: String) {
        ndTrace("setURL want=\(value) requested=\(requestedURL) committed=\(committedURL) hasBrowser=\(hasBrowser)")
        guard !value.isEmpty, value != requestedURL, value != committedURL else { return }
        requestedURL = value
        // No browser yet, or one that has not reported itself: adoption
        // reconciles this against the address it was created with.
        guard hasBrowser else { return }
        loadInMainFrame(value)
    }

    private func loadInMainFrame(_ value: String) {
        guard let browser, let frame = browser.pointee.get_main_frame?(browser) else {
            deferredURL = value
            ndTrace("load deferred \(value)")
            return
        }
        deferredURL = ""
        ndTrace("load issued \(value)")
        defer { nd_cef_ref_release(frame) }
        var url = cef_string_t()
        ndCefSetString(value, &url)
        defer { nd_cef_string_clear(&url) }
        frame.pointee.load_url?(frame, &url)
    }

    /// Retried from the engine's own load signals, which is where a main frame
    /// first exists. Re-parks if it still does not, so this cannot spin.
    fileprivate func retryDeferredLoad() {
        guard !deferredURL.isEmpty else { return }
        let value = deferredURL
        deferredURL = ""
        loadInMainFrame(value)
    }

    /// createAndUpdate `contextMenuMode` prop. The page-side agent decides
    /// whether to `preventDefault()`, so the live page is told directly and the
    /// script is rebuilt for the next load, exactly as on the WebKit surface.
    func ndSetContextMenuMode(_ mode: String) {
        let suppress = mode == "suppress"
        guard suppress != suppressContextMenu else { return }
        suppressContextMenu = suppress
        installInternalAgent()
        devTools.evaluate(
            "window.__ndSetSuppressMenu && window.__ndSetSuppressMenu(\(suppress))",
            world: NDCefWebView.internalWorldName
        ) { _, _ in }
    }

    func ndHandleCommand(_ command: String, argJson: String) {
        let arg = ndCefParseJSON(argJson)
        let obj = arg as? [String: Any] ?? [:]
        switch command {
        case "goBack":
            if let browser, browser.pointee.can_go_back?(browser) != 0 { browser.pointee.go_back?(browser) }
        case "goForward":
            if let browser, browser.pointee.can_go_forward?(browser) != 0 { browser.pointee.go_forward?(browser) }
        case "reload":
            browser.map { $0.pointee.reload?($0) }
        case "stop":
            browser.map { $0.pointee.stop_load?($0) }
        case "setZoom":
            guard let zoom = arg as? NSNumber else {
                ndCefWarn("malformed setZoom arg")
                return
            }
            setZoom(zoom.doubleValue)
        case "setUserAgent":
            // Per-context at creation on this engine, so a live change is a
            // page-level override of navigator.userAgent rather than a header
            // change. CDP's Network.setUserAgentOverride does both.
            guard let agent = arg as? String else {
                ndCefWarn("malformed setUserAgent arg")
                return
            }
            devTools.call("Network.setUserAgentOverride", ["userAgent": agent])
        case "executeJavaScript": ndExecuteJavaScript(obj)
        case "addUserScript": ndAddUserScript(obj)
        case "removeUserScript":
            guard let id = obj["id"] as? String else {
                ndCefWarn("removeUserScript: missing id")
                return
            }
            ndRemoveUserScript(id)
        case "clearUserScripts":
            ndClearUserScripts(world: obj["world"] as? String)
        case "registerScriptMessage": ndRegisterScriptMessage(obj)
        case "unregisterScriptMessage": ndUnregisterScriptMessage(obj)
        case "respondScheme": NDCefSchemes.respond(obj)
        case "respondPermission": NDCefPermissions.respond(obj)
        case "getCookies": ndGetCookies(obj)
        case "setCookie": ndSetCookie(obj)
        case "deleteCookie": ndDeleteCookie(obj)
        case "findStart", "findNext", "findPrevious": ndFind(command, obj)
        case "findStop": ndFindStop()
        case "saveSession": ndSaveSession(obj)
        case "restoreSession": ndRestoreSession(obj)
        case "setMuted":
            let muted = (arg as? NSNumber)?.boolValue ?? (obj["muted"] as? NSNumber)?.boolValue ?? false
            ndSetMuted(muted)
        case "setContextMenuItems":
            contextMenuItems = NDContextMenuItem.parse(arg)
        case "openDevTools":
            openDevTools(obj)
        default:
            ndCefWarn("unknown WebView command \(command)")
        }
    }

    /// Armed by openDevTools, consumed by on_before_dev_tools_popup: the
    /// devtools window is allowed only on explicit app request, so a page can
    /// never conjure it and the stray-window rule holds for everything else.
    private var devToolsRequested = false

    func takeDevToolsRequest() -> Bool {
        let wanted = devToolsRequested
        devToolsRequested = false
        return wanted
    }

    /// Chromium's devtools in its own top-level window, the same shape the
    /// GTK engine ships. Optional x/y (view-relative CSS pixels, the
    /// contextMenu event's coordinates) starts with that element inspected.
    ///
    /// The command toggles, which is what `openDevTools`, F12 and the app's own
    /// Inspect Element all rely on; asking for an inspector that is already
    /// there otherwise leaves the page stuck at the docked width.
    private func openDevTools(_ obj: [String: Any]) {
        guard let browserHost = browserHost() else { return }
        defer { nd_cef_ref_release(browserHost) }
        // Chrome style's inspector is a BrowserView this object docked, and
        // `has_dev_tools` keeps answering 1 for a while after it is closed, so
        // the dock's own state is what the toggle reads there.
        if let chrome {
            if chrome.hasDockedDevTools {
                chrome.closeDevTools()
                return
            }
        } else if browserHost.pointee.has_dev_tools?(browserHost) != 0 {
            browserHost.pointee.close_dev_tools?(browserHost)
            return
        }
        devToolsRequested = true
        var windowInfo = cef_window_info_t()
        if let x = (obj["x"] as? NSNumber)?.int32Value, let y = (obj["y"] as? NSNumber)?.int32Value {
            var point = cef_point_t(x: x, y: y)
            browserHost.pointee.show_dev_tools?(browserHost, &windowInfo, nil, nil, &point)
        } else {
            browserHost.pointee.show_dev_tools?(browserHost, &windowInfo, nil, nil, nil)
        }
    }

    // MARK: - Context menu

    /// Builds and pops up the NSMenu for Chromium's model. `point` is in the
    /// page's own CSS pixels from the top-left of the web contents, which is
    /// this view's rectangle; AppKit measures from the bottom.
    @MainActor func showContextMenu(model: UInt, callback: UInt, at point: NSPoint, link: String) -> Bool {
        guard let modelPointer = UnsafeMutableRawPointer(bitPattern: model)?
            .assumingMemoryBound(to: cef_menu_model_t.self),
            let callback = UnsafeMutableRawPointer(bitPattern: callback)?
                .assumingMemoryBound(to: cef_run_context_menu_callback_t.self) else { return false }
        cancelContextMenu()
        menuLinkURL = link
        let entries = ndCefCopyMenuModel(modelPointer)
        guard let run = NDCefContextMenuRun(entries: entries, callback: callback, view: self) else { return false }
        menuRun = run
        let spot = NSPoint(x: point.x, y: isFlipped ? point.y : bounds.height - point.y)
        ndTrace("chrome menu items=\(entries.count) at=\(Int(spot.x)),\(Int(spot.y))")
        run.trace(entries)
        run.present(at: spot)
        return true
    }

    /// Dismisses a menu nobody answered: the view is going away, the page
    /// navigated, or the host is quitting.
    @MainActor func cancelContextMenu() {
        guard let run = menuRun else { return }
        menuRun = nil
        run.cancel()
    }

    @MainActor func forgetContextMenu(_ run: NDCefContextMenuRun) {
        if menuRun === run { menuRun = nil }
    }

    /// Chrome's own "open link in …" items run before `cef_command_handler_t`
    /// sees them, and they are answered with a browser window of its own, so the
    /// reroute to the app happens here and the command deny list is only the
    /// backstop.
    @MainActor func ndRerouteOpenLink(_ commandID: Int32) -> Bool {
        guard !menuLinkURL.isEmpty, ndCefOpenLinkCommands.contains(commandID) else { return false }
        emitText("newWindow", menuLinkURL)
        return true
    }

    /// CEF zoom is a log scale around 1.0; the widget's factor is linear.
    private func setZoom(_ factor: Double) {
        guard factor > 0, let browserHost = browserHost() else { return }
        defer { nd_cef_ref_release(browserHost) }
        browserHost.pointee.set_zoom_level?(browserHost, log(factor) / log(1.2))
    }

    // MARK: - Find

    private func ndFind(_ command: String, _ obj: [String: Any]) {
        guard let browserHost = browserHost() else { return }
        defer { nd_cef_ref_release(browserHost) }
        if command == "findStart" {
            guard let text = obj["text"] as? String else {
                ndCefWarn("findStart: missing text")
                return
            }
            pendingFindText = text
        }
        guard !pendingFindText.isEmpty else { return }
        var text = cef_string_t()
        ndCefSetString(pendingFindText, &text)
        defer { nd_cef_string_clear(&text) }
        let caseSensitive = (obj["caseSensitive"] as? NSNumber)?.boolValue ?? false
        browserHost.pointee.find?(
            browserHost,
            &text,
            command == "findPrevious" ? 0 : 1,
            caseSensitive ? 1 : 0,
            command == "findStart" ? 0 : 1
        )
    }

    private func ndFindStop() {
        pendingFindText = ""
        guard let browserHost = browserHost() else { return }
        defer { nd_cef_ref_release(browserHost) }
        browserHost.pointee.stop_finding?(browserHost, 1)
    }

    fileprivate func emitFindResult(count: Int32, final: Bool) {
        host?.emitData("findResult", ["matchFound": count > 0, "matchCount": Int(count), "done": final])
    }

    // MARK: - Audio

    private func ndSetMuted(_ muted: Bool) {
        if let browserHost = browserHost() {
            browserHost.pointee.set_audio_muted?(browserHost, muted ? 1 : 0)
            nd_cef_ref_release(browserHost)
        }
        // The native mute silences the pipeline; the page-side agent is what
        // reports the state and keeps new media elements muted, the same way
        // the WebKit surface does it.
        devTools.evaluate(
            "window.__ndSetMuted && window.__ndSetMuted(\(muted))",
            world: NDCefWebView.internalWorldName
        ) { _, _ in }
    }

    // MARK: - Session

    /// CEF exposes no serialized interaction state, so the session is the
    /// address of the current navigation entry. Restoring puts the view back
    /// on that page; scroll offset and form state do not survive, which is the
    /// engine's limit rather than a shortcut.
    private func ndSaveSession(_ obj: [String: Any]) {
        let id = obj["id"] as? String ?? ""
        let payload: [String: Any] = ["url": committedURL, "title": lastTitle]
        guard let data = try? JSONSerialization.data(withJSONObject: payload) else { return }
        host?.emitData("sessionSaved", ["id": id, "state": data.base64EncodedString()])
    }

    private func ndRestoreSession(_ obj: [String: Any]) {
        guard let encoded = obj["state"] as? String, let data = Data(base64Encoded: encoded),
              let payload = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let url = payload["url"] as? String else {
            ndCefWarn("restoreSession: malformed state")
            return
        }
        ndSetURL(url)
    }

    // MARK: - Events

    func ndTrace(_ message: String) {
        guard ProcessInfo.processInfo.environment["ND_WEBVIEW_TRACE"] == "1" else { return }
        FileHandle.standardError.write("ND_WV cef node=\(host?.ndNodeID ?? 0) \(message)\n".data(using: .utf8)!)
    }

    func emitText(_ name: String, _ value: String) {
        host?.emitEvent(name, json: ndCefTextJson(value))
    }

    func emitData(_ name: String, _ fields: [String: Any]) {
        host?.emitData(name, fields)
    }

    fileprivate func emitAddress(_ value: String) {
        ndTrace("addressChange \(value) requested=\(requestedURL) deferred=\(deferredURL)")
        // The browser is created on about:blank whenever the view has no url
        // yet. That is this file's placeholder, not a page the app asked for,
        // and WebKit reports nothing at all for a view that never loaded
        // anything. Reporting it is what gives the app a second address to
        // echo back, and the echo is a load storm.
        if value == "about:blank", requestedURL != "about:blank" { return }
        // An address the app did not ask for means the page navigated itself,
        // so the app is free to ask for its own again later.
        if value != requestedURL { requestedURL = "" }
        committedURL = value
        // A menu still up belongs to the page that has just been replaced.
        cancelContextMenu()
        emitText("navigate", value)
    }

    fileprivate func emitTitle(_ value: String) {
        lastTitle = value
        emitText("titleChanged", value)
    }

    /// What the automation `webviewInfo` RPC reports for a chromium view.
    /// The address that actually committed, which is what `webviewInfo`
    /// promises ("null before the first commit"). Answering with the requested
    /// one instead would report a page that may never have loaded.
    var ndPageState: NDWebViewPageState {
        NDWebViewPageState(
            url: committedURL.isEmpty ? nil : committedURL,
            title: lastTitle.isEmpty ? nil : lastTitle,
            loading: lastLoading,
            canGoBack: lastCanGoBack,
            canGoForward: lastCanGoForward
        )
    }

    fileprivate func emitProgress(_ value: Double) {
        let rounded = (value * 1000).rounded() / 1000
        guard rounded != lastProgress else { return }
        lastProgress = rounded
        host?.emitEvent("loadProgress", json: "{\"value\":\(rounded)}")
    }

    fileprivate func updateLoadState(loading: Bool, canGoBack: Bool, canGoForward: Bool) {
        if loading != lastLoading {
            lastLoading = loading
            host?.emitEvent("loadingChanged", json: "{\"checked\":\(loading)}")
        }
        if canGoBack != lastCanGoBack {
            lastCanGoBack = canGoBack
            host?.emitEvent("backAvailable", json: "{\"checked\":\(canGoBack)}")
        }
        if canGoForward != lastCanGoForward {
            lastCanGoForward = canGoForward
            host?.emitEvent("forwardAvailable", json: "{\"checked\":\(canGoForward)}")
        }
    }

    /// ERR_ABORTED is a load superseded by a newer one, the peer of the
    /// NSURLErrorCancelled filter on the WKWebView surface.
    fileprivate func emitLoadFailed(url: String, message: String, code: Int32) {
        guard code != -3 else { return }
        host?.emitData("loadFailed", ["url": url, "error": message.isEmpty ? "Load failed (\(code))" : message])
    }

    /// Read off the visible navigation entry at load end, which is where CEF
    /// keeps the TLS state for the page that actually committed.
    fileprivate func emitSecurity() {
        guard let browserHost = browserHost() else { return }
        defer { nd_cef_ref_release(browserHost) }
        guard let entry = browserHost.pointee.get_visible_navigation_entry?(browserHost) else { return }
        defer { nd_cef_ref_release(entry) }
        var secure = false
        var insecureContent = false
        if let ssl = entry.pointee.get_sslstatus?(entry) {
            secure = ssl.pointee.is_secure_connection?(ssl) != 0
            // DISPLAYED_INSECURE_CONTENT | RAN_INSECURE_CONTENT
            insecureContent = secure && (ssl.pointee.get_content_status?(ssl).rawValue ?? 0) != 0
            nd_cef_ref_release(ssl)
        }
        guard lastSecure != secure else { return }
        lastSecure = secure
        emitData("securityChanged", ["secure": secure, "insecureContent": insecureContent])
    }

    fileprivate func emitFavicon(_ urls: [String]) {
        guard let icon = urls.first(where: { !$0.isEmpty }) else { return }
        emitData("faviconChanged", ["pageUrl": committedURL, "iconUrl": icon])
    }

    fileprivate func emitDownload(url: String, suggestedName: String) {
        var fields: [String: Any] = ["url": url]
        if !suggestedName.isEmpty { fields["suggestedFilename"] = suggestedName }
        emitData("downloadRequested", fields)
    }

    fileprivate func didFinishLoad() {
        emitSecurity()
        NDCefCapture.refresh(self)
    }
}

// MARK: - Handlers

/// Owns every capi struct for one view. CEF holds references to them for as
/// long as a browser exists, which can outlast the NSView, so the structs point
/// here and this points back weakly.
final class NDCefHandlerBox {
    weak var view: NDCefWebView?
    fileprivate(set) var client: UnsafeMutablePointer<cef_client_t>?
    fileprivate(set) var display: UnsafeMutablePointer<cef_display_handler_t>?
    fileprivate(set) var load: UnsafeMutablePointer<cef_load_handler_t>?
    fileprivate(set) var lifeSpan: UnsafeMutablePointer<cef_life_span_handler_t>?
    fileprivate(set) var find: UnsafeMutablePointer<cef_find_handler_t>?
    fileprivate(set) var download: UnsafeMutablePointer<cef_download_handler_t>?
    fileprivate(set) var jsDialog: UnsafeMutablePointer<cef_jsdialog_handler_t>?
    fileprivate(set) var permission: UnsafeMutablePointer<cef_permission_handler_t>?
    fileprivate(set) var dialog: UnsafeMutablePointer<cef_dialog_handler_t>?
    fileprivate(set) var contextMenu: UnsafeMutablePointer<cef_context_menu_handler_t>?
    fileprivate(set) var focus: UnsafeMutablePointer<cef_focus_handler_t>?
    fileprivate(set) var keyboard: UnsafeMutablePointer<cef_keyboard_handler_t>?
    fileprivate(set) var request: UnsafeMutablePointer<cef_request_handler_t>?
    fileprivate(set) var resourceRequest: UnsafeMutablePointer<cef_resource_request_handler_t>?
    fileprivate(set) var devToolsObserver: UnsafeMutablePointer<cef_dev_tools_message_observer_t>?
    /// Chrome style only (NDCefChromeWindow.swift). Alloy has no Chrome command
    /// surface and is never born in a Views window, so these stay nil there.
    var command: UnsafeMutablePointer<cef_command_handler_t>?
    var windowDelegate: UnsafeMutablePointer<cef_window_delegate_t>?
    var browserViewDelegate: UnsafeMutablePointer<cef_browser_view_delegate_t>?
    var devToolsViewDelegate: UnsafeMutablePointer<cef_browser_view_delegate_t>?
    /// Kept for as long as the observer should stay attached: destroying the
    /// registration is what detaches it.
    fileprivate(set) var devToolsRegistration: UnsafeMutablePointer<cef_registration_t>?

    func build() {
        display = ndCefAlloc(cef_display_handler_t.self, self)
        load = ndCefAlloc(cef_load_handler_t.self, self)
        lifeSpan = ndCefAlloc(cef_life_span_handler_t.self, self)
        find = ndCefAlloc(cef_find_handler_t.self, self)
        download = ndCefAlloc(cef_download_handler_t.self, self)
        jsDialog = ndCefAlloc(cef_jsdialog_handler_t.self, self)
        permission = ndCefAlloc(cef_permission_handler_t.self, self)
        dialog = ndCefAlloc(cef_dialog_handler_t.self, self)
        contextMenu = ndCefAlloc(cef_context_menu_handler_t.self, self)
        focus = ndCefAlloc(cef_focus_handler_t.self, self)
        keyboard = ndCefAlloc(cef_keyboard_handler_t.self, self)
        request = ndCefAlloc(cef_request_handler_t.self, self)
        resourceRequest = ndCefAlloc(cef_resource_request_handler_t.self, self)
        devToolsObserver = ndCefAlloc(cef_dev_tools_message_observer_t.self, self)
        if NDCefRuntime.isChromeStyle { buildChrome() }
        client = ndCefAlloc(cef_client_t.self, self)
        wireDisplay()
        wireLoad()
        wireLifeSpan()
        wireFind()
        wireDownload()
        wireJSDialog()
        wirePermission()
        wireDialog()
        wireContextMenu()
        wireFocus()
        wireKeyboard()
        wireRequest()
        wireDevTools()
        wireClient()
    }

    /// Drops the view's own reference on each struct. CEF may still hold its
    /// own, in which case the block survives with a nil `view`.
    func teardown() {
        nd_cef_ref_release(devToolsRegistration)
        for object in [
            client.map(UnsafeMutableRawPointer.init),
            display.map(UnsafeMutableRawPointer.init),
            load.map(UnsafeMutableRawPointer.init),
            lifeSpan.map(UnsafeMutableRawPointer.init),
            find.map(UnsafeMutableRawPointer.init),
            download.map(UnsafeMutableRawPointer.init),
            jsDialog.map(UnsafeMutableRawPointer.init),
            dialog.map(UnsafeMutableRawPointer.init),
            contextMenu.map(UnsafeMutableRawPointer.init),
            focus.map(UnsafeMutableRawPointer.init),
            keyboard.map(UnsafeMutableRawPointer.init),
            request.map(UnsafeMutableRawPointer.init),
            resourceRequest.map(UnsafeMutableRawPointer.init),
            devToolsObserver.map(UnsafeMutableRawPointer.init),
            command.map(UnsafeMutableRawPointer.init),
            windowDelegate.map(UnsafeMutableRawPointer.init),
            browserViewDelegate.map(UnsafeMutableRawPointer.init),
            devToolsViewDelegate.map(UnsafeMutableRawPointer.init),
        ] {
            nd_cef_ref_release(object)
        }
        client = nil
        display = nil
        load = nil
        lifeSpan = nil
        find = nil
        download = nil
        jsDialog = nil
        dialog = nil
        contextMenu = nil
        focus = nil
        keyboard = nil
        request = nil
        resourceRequest = nil
        devToolsObserver = nil
        devToolsRegistration = nil
        command = nil
        windowDelegate = nil
        browserViewDelegate = nil
        devToolsViewDelegate = nil
    }

    fileprivate func attachDevTools(_ registration: UnsafeMutablePointer<cef_registration_t>?) {
        nd_cef_ref_release(devToolsRegistration)
        devToolsRegistration = registration
    }

    private func wireClient() {
        guard let client else { return }
        // Every getter hands back a NEW reference: the caller owns what it
        // receives, per the capi contract.
        client.pointee.get_display_handler = { selfPointer in
            ndCefHandOut(ndCefBox(selfPointer)?.display)
        }
        client.pointee.get_load_handler = { selfPointer in
            ndCefHandOut(ndCefBox(selfPointer)?.load)
        }
        client.pointee.get_life_span_handler = { selfPointer in
            ndCefHandOut(ndCefBox(selfPointer)?.lifeSpan)
        }
        client.pointee.get_find_handler = { selfPointer in
            ndCefHandOut(ndCefBox(selfPointer)?.find)
        }
        client.pointee.get_download_handler = { selfPointer in
            ndCefHandOut(ndCefBox(selfPointer)?.download)
        }
        client.pointee.get_permission_handler = { selfPointer in
            ndCefHandOut(ndCefBox(selfPointer)?.permission)
        }
        client.pointee.get_jsdialog_handler = { selfPointer in
            ndCefHandOut(ndCefBox(selfPointer)?.jsDialog)
        }
        client.pointee.get_dialog_handler = { selfPointer in
            ndCefHandOut(ndCefBox(selfPointer)?.dialog)
        }
        client.pointee.get_context_menu_handler = { selfPointer in
            ndCefHandOut(ndCefBox(selfPointer)?.contextMenu)
        }
        client.pointee.get_focus_handler = { selfPointer in
            ndCefHandOut(ndCefBox(selfPointer)?.focus)
        }
        client.pointee.get_keyboard_handler = { selfPointer in
            ndCefHandOut(ndCefBox(selfPointer)?.keyboard)
        }
        client.pointee.get_request_handler = { selfPointer in
            ndCefHandOut(ndCefBox(selfPointer)?.request)
        }
        client.pointee.get_command_handler = { selfPointer in
            ndCefHandOut(ndCefBox(selfPointer)?.command)
        }
    }

    private func wireDisplay() {
        guard let display else { return }
        // Every one of these is keyed on the browser: the inspector is a second
        // browser on this same client, and without the check its address and
        // title are reported as the view's, which puts `devtools://…` in the
        // app's address bar, its session store and its window title.
        display.pointee.on_address_change = { selfPointer, browser, frame, url in
            let value = ndCefString(url)
            let identifier = browser?.pointee.get_identifier?(browser) ?? -1
            nd_cef_ref_release(browser)
            nd_cef_ref_release(frame)
            ndCefDeliver(selfPointer) { view in
                guard view?.ownsBrowser(identifier) == true else { return }
                view?.emitAddress(value)
            }
        }
        display.pointee.on_title_change = { selfPointer, browser, title in
            let value = ndCefString(title)
            let identifier = browser?.pointee.get_identifier?(browser) ?? -1
            nd_cef_ref_release(browser)
            ndCefDeliver(selfPointer) { view in
                guard view?.ownsBrowser(identifier) == true else { return }
                view?.emitTitle(value)
            }
        }
        display.pointee.on_loading_progress_change = { selfPointer, browser, progress in
            let identifier = browser?.pointee.get_identifier?(browser) ?? -1
            nd_cef_ref_release(browser)
            ndCefDeliver(selfPointer) { view in
                guard view?.ownsBrowser(identifier) == true else { return }
                view?.emitProgress(progress)
            }
        }
        display.pointee.on_favicon_urlchange = { selfPointer, browser, iconUrls in
            nd_cef_ref_release(browser)
            var urls: [String] = []
            for index in 0..<nd_cef_string_list_count(iconUrls) {
                var slot = cef_string_t()
                if nd_cef_string_list_at(iconUrls, index, &slot) != 0 {
                    urls.append(ndCefString(&slot))
                }
                nd_cef_string_clear(&slot)
            }
            ndCefDeliver(selfPointer) { $0?.emitFavicon(urls) }
        }
    }

    private func wireLoad() {
        guard let load else { return }
        load.pointee.on_loading_state_change = { selfPointer, browser, isLoading, canGoBack, canGoForward in
            nd_cef_ref_release(browser)
            let loading = isLoading != 0
            let back = canGoBack != 0
            let forward = canGoForward != 0
            ndCefDeliver(selfPointer) { view in
                view?.retryDeferredLoad()
                view?.updateLoadState(loading: loading, canGoBack: back, canGoForward: forward)
                if !loading { view?.didFinishLoad() }
            }
        }
        // The initial blank document starting to load is the earliest point a
        // main frame is guaranteed to exist.
        load.pointee.on_load_start = { selfPointer, browser, frame, _ in
            nd_cef_ref_release(browser)
            nd_cef_ref_release(frame)
            ndCefDeliver(selfPointer) { $0?.retryDeferredLoad() }
        }
        load.pointee.on_load_error = { selfPointer, browser, frame, errorCode, errorText, failedUrl in
            let message = ndCefString(errorText)
            let url = ndCefString(failedUrl)
            let code = errorCode.rawValue
            nd_cef_ref_release(browser)
            nd_cef_ref_release(frame)
            ndCefDeliver(selfPointer) { $0?.emitLoadFailed(url: url, message: message, code: code) }
        }
    }

    private func wireLifeSpan() {
        guard let lifeSpan else { return }
        // The no-stray-window invariant: returning 1 cancels the popup CEF was
        // about to create, and the app opens a tab from the emitted URL. This
        // is the same answer the WKWebView surface gives by returning nil from
        // createWebViewWith.
        lifeSpan.pointee.on_before_popup = {
            selfPointer, browser, frame, _, targetUrl, _, _, _, _, _, _, _, _, _ in
            let url = ndCefString(targetUrl)
            nd_cef_ref_release(browser)
            nd_cef_ref_release(frame)
            ndCefDeliver(selfPointer) { $0?.emitText("newWindow", url) }
            return 1
        }
        lifeSpan.pointee.on_before_dev_tools_popup = { selfPointer, browser, _, _, _, _, useDefaultWindow in
            nd_cef_ref_release(browser)
            // Allowed only when the app just asked through openDevTools; a
            // page cannot conjure the window on its own.
            var wanted = false
            ndCefDeliver(selfPointer) { view in
                if let view { wanted = view.takeDevToolsRequest() }
            }
            // Chrome style takes the Views-hosted route unconditionally, which
            // is what routes the devtools BrowserView through
            // on_popup_browser_view_created and into the page's own window as a
            // docked split. A default window there would be the stray Chromium
            // window this whole file exists to prevent.
            useDefaultWindow?.pointee = (wanted && !NDCefRuntime.isChromeStyle) ? 1 : 0
        }
        lifeSpan.pointee.on_after_created = { selfPointer, browser in
            guard let browser else { return }
            // Two references in hand: the one the callback arrived with, and
            // one for the view. Whichever the view does not take is released
            // here, so an orphaned create cannot leak a browser.
            nd_cef_ref_add(browser)
            let kept = UInt(bitPattern: browser)
            ndCefDeliver(selfPointer) { view in
                guard let created = ndCefBrowser(kept) else { return }
                if let view {
                    view.adoptBrowser(created)
                } else {
                    nd_cef_ref_release(created)
                }
            }
            nd_cef_ref_release(browser)
        }
        lifeSpan.pointee.on_before_close = { selfPointer, browser in
            let identifier = browser?.pointee.get_identifier?(browser) ?? -1
            nd_cef_ref_release(browser)
            ndCefDeliver(selfPointer) { $0?.forgetBrowser(identifier: identifier) }
        }
    }

    private func wireFind() {
        guard let find else { return }
        find.pointee.on_find_result = { selfPointer, browser, _, count, _, _, finalUpdate in
            nd_cef_ref_release(browser)
            let isFinal = finalUpdate != 0
            ndCefDeliver(selfPointer) { $0?.emitFindResult(count: count, final: isFinal) }
        }
    }

    private func wireDownload() {
        guard let download else { return }
        download.pointee.can_download = { selfPointer, browser, _, _ in
            nd_cef_ref_release(browser)
            return 1
        }
        // Cancelled by never continuing the callback: the app owns downloading,
        // exactly as on the WebKit surface where the response policy is
        // .cancel and the URL is handed to Bun.
        download.pointee.on_before_download = { selfPointer, browser, item, suggestedName, callback in
            let name = ndCefString(suggestedName)
            var url = ""
            if let item, let raw = item.pointee.get_url?(item) {
                url = ndCefString(raw)
                nd_cef_string_free(raw)
            }
            nd_cef_ref_release(browser)
            nd_cef_ref_release(item)
            nd_cef_ref_release(callback)
            ndCefDeliver(selfPointer) { $0?.emitDownload(url: url, suggestedName: name) }
            return 0
        }
    }

    private func wireJSDialog() {
        guard let jsDialog else { return }
        // alert/confirm/prompt park the page's JS until this answers, so every
        // path here calls cont() exactly once, on the same host-native sheet
        // the WebKit surface uses.
        jsDialog.pointee.on_jsdialog = {
            selfPointer, browser, _, dialogType, messageText, defaultPrompt, callback, suppressMessage in
            let message = ndCefString(messageText)
            let initial = ndCefString(defaultPrompt)
            nd_cef_ref_release(browser)
            suppressMessage?.pointee = 0
            guard let callback else { return 0 }
            nd_cef_ref_add(callback)
            let token = UInt(bitPattern: callback)
            ndCefDeliver(selfPointer) { view in
                NDCefDialogs.run(view: view, type: dialogType, message: message, initial: initial, callback: token)
            }
            nd_cef_ref_release(callback)
            return 1
        }
        jsDialog.pointee.on_before_unload_dialog = { selfPointer, browser, messageText, _, callback in
            let message = ndCefString(messageText)
            nd_cef_ref_release(browser)
            guard let callback else { return 0 }
            nd_cef_ref_add(callback)
            let token = UInt(bitPattern: callback)
            ndCefDeliver(selfPointer) { view in
                NDCefDialogs.run(view: view, type: JSDIALOGTYPE_CONFIRM, message: message, initial: "", callback: token)
            }
            nd_cef_ref_release(callback)
            return 1
        }
    }

    /// Chrome style answers a permission request with Chromium's own prompt, a
    /// Views bubble anchored to the toolbar this embedding does not have. On
    /// AppKit it becomes a window of its own that follows the anchor, so both
    /// routes are taken here and the request reaches the app as
    /// `permissionRequest`, answered with `respondPermission`. An unanswered id
    /// leaves the page waiting, as `schemeRequest` does.
    private func wirePermission() {
        guard let permission else { return }
        permission.pointee.on_show_permission_prompt = {
            selfPointer, browser, promptID, requestingOrigin, requestedPermissions, callback in
            let origin = ndCefString(requestingOrigin)
            nd_cef_ref_release(browser)
            guard let callback else { return 0 }
            nd_cef_ref_add(callback)
            let token = UInt(bitPattern: callback)
            ndCefDeliver(selfPointer) { view in
                NDCefPermissions.request(view: view, promptID: promptID, origin: origin,
                                         mask: requestedPermissions, callback: token, media: false)
            }
            nd_cef_ref_release(callback)
            return 1
        }
        permission.pointee.on_request_media_access_permission = {
            selfPointer, browser, frame, requestingOrigin, requestedPermissions, callback in
            let origin = ndCefString(requestingOrigin)
            nd_cef_ref_release(browser)
            nd_cef_ref_release(frame)
            guard let callback else { return 0 }
            nd_cef_ref_add(callback)
            let token = UInt(bitPattern: callback)
            ndCefDeliver(selfPointer) { view in
                NDCefPermissions.request(view: view, promptID: 0, origin: origin,
                                         mask: requestedPermissions, callback: token, media: true)
            }
            nd_cef_ref_release(callback)
            return 1
        }
        permission.pointee.on_dismiss_permission_prompt = { _, browser, promptID, _ in
            nd_cef_ref_release(browser)
            guard Thread.isMainThread else { return }
            MainActor.assumeIsolated { NDCefPermissions.dismiss(promptID: promptID) }
        }
    }

    private func wireDialog() {
        guard let dialog else { return }
        dialog.pointee.on_file_dialog = {
            selfPointer, browser, mode, title, defaultPath, _, extensions, _, callback in
            let heading = ndCefString(title)
            let initial = ndCefString(defaultPath)
            var suffixes: [String] = []
            for index in 0..<nd_cef_string_list_count(extensions) {
                var slot = cef_string_t()
                if nd_cef_string_list_at(extensions, index, &slot) != 0 {
                    suffixes.append(contentsOf: ndCefString(&slot).split(separator: ";").map(String.init))
                }
                nd_cef_string_clear(&slot)
            }
            nd_cef_ref_release(browser)
            guard let callback else { return 0 }
            nd_cef_ref_add(callback)
            let token = UInt(bitPattern: callback)
            ndCefDeliver(selfPointer) { view in
                NDCefDialogs.runFilePanel(
                    view: view, mode: mode, title: heading, initialPath: initial,
                    extensions: suffixes, callback: token)
            }
            nd_cef_ref_release(callback)
            return 1
        }
    }

    private func wireContextMenu() {
        guard let contextMenu else { return }
        // "suppress" mode answers the menu itself and shows nothing, so the
        // app's `contextMenu` event is the only outcome. "native" keeps
        // Chromium's menu and appends the app's matching items.
        // Chrome style draws the menu itself (NDCefContextMenu.swift): the
        // browser's widget is laid out against the anchor window, so the Views
        // menu Chromium would raise never sees a click the window server routed
        // to the host's own window. Alloy keeps CEF's menu runner, which builds
        // an NSMenu on the browser's own view and works as it stands.
        contextMenu.pointee.run_context_menu = { selfPointer, browser, frame, params, model, callback in
            nd_cef_ref_release(browser)
            nd_cef_ref_release(frame)
            var point = NSPoint.zero
            var link = ""
            if let params {
                point = NSPoint(
                    x: CGFloat(params.pointee.get_xcoord?(params) ?? 0),
                    y: CGFloat(params.pointee.get_ycoord?(params) ?? 0))
                if let raw = params.pointee.get_link_url?(params) {
                    link = ndCefString(raw)
                    nd_cef_string_free(raw)
                }
            }
            nd_cef_ref_release(params)
            let suppress = ndCefBox(selfPointer)?.view?.suppressContextMenu ?? false
            if suppress || !NDCefRuntime.isChromeStyle {
                nd_cef_ref_release(model)
                if suppress {
                    callback?.pointee.cancel?(callback)
                    nd_cef_ref_release(callback)
                    return 1
                }
                nd_cef_ref_release(callback)
                return 0
            }
            guard let model, let callback else {
                nd_cef_ref_release(model)
                nd_cef_ref_release(callback)
                return 0
            }
            // The model is only valid while this call is on the stack, so the
            // copy has to finish here. Both pointers cross the isolation
            // boundary as bit patterns, the same way every other one here does.
            let modelToken = UInt(bitPattern: model)
            let callbackToken = UInt(bitPattern: callback)
            var shown: Int32 = 0
            ndCefDeliver(selfPointer) { view in
                // The callback's own reference is what the run keeps and gives
                // back when it answers.
                shown = view?.showContextMenu(
                    model: modelToken, callback: callbackToken, at: point, link: link) == true ? 1 : 0
            }
            nd_cef_ref_release(model)
            if shown == 0 { nd_cef_ref_release(callback) }
            return shown
        }
        // The model is only valid while this call is on the stack: CEF shows
        // the menu the moment it returns, so population has to finish here.
        // ndCefDeliver satisfies that by running its closure INLINE on the CEF
        // UI thread (the main thread under this host's message loop); it never
        // queues. `populated` is the enforcement rather than a comment: if
        // delivery ever stops being synchronous, this says so instead of
        // silently opening a menu without the app's items.
        contextMenu.pointee.on_before_context_menu = { selfPointer, browser, frame, params, model in
            nd_cef_ref_release(browser)
            nd_cef_ref_release(frame)
            nd_cef_ref_release(params)
            guard let model else { return }
            defer { nd_cef_ref_release(model) }
            let token = UInt(bitPattern: model)
            var populated = false
            ndCefDeliver(selfPointer) { view in
                view?.ndAppendContextMenuItems(token)
                populated = true
            }
            if !populated {
                ndCefWarn("on_before_context_menu did not populate synchronously; the app's items were dropped")
            }
        }
        contextMenu.pointee.on_context_menu_command = { selfPointer, browser, frame, params, commandID, _ in
            nd_cef_ref_release(browser)
            nd_cef_ref_release(frame)
            nd_cef_ref_release(params)
            var handled: Int32 = 0
            ndCefDeliver(selfPointer) { view in
                guard let view else { return }
                handled = (view.ndRerouteOpenLink(commandID) || view.ndContextMenuCommand(commandID)) ? 1 : 0
            }
            return handled
        }
    }

    /// A page finishing a load must not move the app's keyboard focus. Chromium
    /// hands first responder to the newly loaded view by default, so a
    /// background `<webview>` reloading would silently take the caret out of
    /// whatever the user was typing in. WebKit never does this, and neither
    /// does the widget: focus moves when the app's `focus` command says so.
    private func wireFocus() {
        guard let focus else { return }
        focus.pointee.on_set_focus = { selfPointer, browser, source in
            nd_cef_ref_release(browser)
            return source == FOCUS_SOURCE_NAVIGATION ? 1 : 0
        }
    }

    /// The Edit menu's own chords reach the page here, not through
    /// `performKeyEquivalent`. Chromium's RenderWidgetHostViewCocoa is the first
    /// responder and answers YES to every command chord while it is, so the main
    /// menu never sees one; this is the callback CEF makes once the renderer and
    /// the page's own JavaScript have declined it, which is the same order
    /// Chrome's CommandDispatcher redispatches in. The menu item then runs
    /// `copy:`/`paste:`/`selectAll:`/`undo:` against that same first responder.
    private func wireKeyboard() {
        guard let keyboard else { return }
        keyboard.pointee.on_key_event = { _, browser, event, osEvent in
            nd_cef_ref_release(browser)
            guard let event, event.pointee.type == KEYEVENT_RAWKEYDOWN, let osEvent,
                  Thread.isMainThread else { return 0 }
            // The NSEvent crosses the isolation boundary as a bit pattern, the
            // same way every other pointer in this file does.
            let token = UInt(bitPattern: osEvent)
            return MainActor.assumeIsolated { ndCefMenuKeyEquivalent(token) }
        }
    }

    /// A registered scheme is served from here rather than from its handler
    /// factory. Chromium owns some scheme names outright (chrome-extension is
    /// the one that matters: its own loader answers ERR_BLOCKED_BY_CLIENT for
    /// an id it does not know, and a scheme handler factory is never
    /// consulted), and `disable_default_handling` is the documented way to
    /// take a request away from the default loader before it runs.
    private func wireRequest() {
        guard let request else { return }
        // Middle-click, cmd-click and the engine's own "open in new tab" reach
        // here rather than on_before_popup. Under Chrome style an allowed one
        // would become a Chromium tab in a window the user is not supposed to
        // have; the app gets the same `newWindow` event a popup produces.
        request.pointee.on_open_urlfrom_tab = { selfPointer, browser, frame, targetUrl, _, _ in
            let url = ndCefString(targetUrl)
            nd_cef_ref_release(browser)
            nd_cef_ref_release(frame)
            ndCefDeliver(selfPointer) { $0?.emitText("newWindow", url) }
            return 1
        }
        // Chrome style answers an HTTP auth challenge with Chromium's own login
        // window, which under this embedding is a window of its own and blocks
        // the app behind it. The framework has no way for an app to supply
        // credentials, so the challenge is refused: 0 cancels the request and
        // the page gets the 401 it would have got if the user pressed Cancel.
        request.pointee.get_auth_credentials = { _, browser, originUrl, _, _, _, _, _, callback in
            let origin = ndCefString(originUrl)
            nd_cef_ref_release(browser)
            if let callback { nd_cef_ref_release(callback) }
            ndCefWarn("HTTP authentication refused for \(origin): no credential surface in this engine")
            return 0
        }
        request.pointee.get_resource_request_handler = {
            selfPointer, browser, frame, request, _, _, _, disableDefaultHandling in
            var url = ""
            if let request, let raw = request.pointee.get_url?(request) {
                url = ndCefString(raw)
                nd_cef_string_free(raw)
            }
            nd_cef_ref_release(browser)
            nd_cef_ref_release(frame)
            nd_cef_ref_release(request)
            guard NDCefSchemes.handles(url) else { return nil }
            disableDefaultHandling?.pointee = 1
            return ndCefHandOut(ndCefBox(selfPointer)?.resourceRequest)
        }

        guard let resourceRequest else { return }
        resourceRequest.pointee.get_resource_handler = { selfPointer, browser, frame, request in
            var url = ""
            if let request, let raw = request.pointee.get_url?(request) {
                url = ndCefString(raw)
                nd_cef_string_free(raw)
            }
            let browserID = browser.flatMap { $0.pointee.get_identifier?($0) } ?? 0
            nd_cef_ref_release(browser)
            nd_cef_ref_release(frame)
            nd_cef_ref_release(request)
            guard NDCefSchemes.handles(url) else { return nil }
            return NDCefSchemeRequest.makeHandler(url: url, browserID: browserID)
        }
    }

    private func wireDevTools() {
        guard let devToolsObserver else { return }
        devToolsObserver.pointee.on_dev_tools_method_result = {
            selfPointer, browser, messageID, success, result, resultSize in
            nd_cef_ref_release(browser)
            // The payload crosses to the main actor as text: a parsed
            // [String: Any] is not Sendable, and re-parsing there is cheaper
            // than the alternatives.
            let raw = ndCefJSONText(result, resultSize)
            let ok = success != 0
            ndCefDeliver(selfPointer) {
                $0?.devTools.handleMethodResult(id: messageID, success: ok, json: ndCefParseJSONText(raw))
            }
        }
        devToolsObserver.pointee.on_dev_tools_event = { selfPointer, browser, method, params, paramsSize in
            let name = ndCefString(method)
            nd_cef_ref_release(browser)
            let raw = ndCefJSONText(params, paramsSize)
            ndCefDeliver(selfPointer) {
                $0?.devTools.handleEvent(method: name, params: ndCefParseJSONText(raw) ?? [:])
            }
        }
    }
}

/// Runs one key-down NSEvent against the app's main menu.
@MainActor private func ndCefMenuKeyEquivalent(_ token: UInt) -> Int32 {
    guard let raw = UnsafeMutableRawPointer(bitPattern: token) else { return 0 }
    let event = Unmanaged<NSEvent>.fromOpaque(raw).takeUnretainedValue()
    guard event.type == .keyDown, event.modifierFlags.contains(.command) else { return 0 }
    return NSApp.mainMenu?.performKeyEquivalent(with: event) == true ? 1 : 0
}

/// The inspector's own close button and dock-side menu are drawn only when the
/// frontend was told it can dock, and that is `can_dock` on the frontend URL.
/// CEF builds that URL inside `show_dev_tools` and takes no argument for it, so
/// the flag is added by re-pointing the frontend at its own address. Clicking
/// the button then reaches CEF as `closeWindow`, which closes the devtools
/// browser, and `on_browser_destroyed` retires the dock from there.
///
/// Answers whether the frontend is now pointed at a docking URL, which is what
/// ends the poll in `NDCefChromeWindow`: the inspector's browser is CEF's own,
/// not this client's, so there is no load callback to hang this on.
func ndCefDockFrontend(_ frame: UnsafeMutablePointer<cef_frame_t>?) -> Bool {
    guard let frame, frame.pointee.is_main?(frame) != 0 else { return false }
    guard let raw = frame.pointee.get_url?(frame) else { return false }
    let url = ndCefString(raw)
    nd_cef_string_free(raw)
    guard url.hasPrefix("devtools://") else { return false }
    if url.contains("can_dock=") { return true }
    var docked = cef_string_t()
    ndCefSetString(url + (url.contains("?") ? "&" : "?") + "can_dock=true", &docked)
    defer { nd_cef_string_clear(&docked) }
    frame.pointee.load_url?(frame, &docked)
    return true
}

/// Hands a handler struct to CEF with the reference the caller is owed.
func ndCefHandOut<T>(_ handler: UnsafeMutablePointer<T>?) -> UnsafeMutablePointer<T>? {
    guard let handler else { return nil }
    nd_cef_ref_add(handler)
    return handler
}

/// One refcounted capi struct, owned by |box|. This is the only place handler
/// objects are allocated: the base callbacks and the atomic count live in
/// CCef, never open-coded per handler.
func ndCefAlloc<T>(_ type: T.Type, _ box: NDCefHandlerBox) -> UnsafeMutablePointer<T>? {
    let owner = Unmanaged.passRetained(box).toOpaque()
    guard let raw = nd_cef_ref_alloc(MemoryLayout<T>.size, owner, { owner in
        guard let owner else { return }
        Unmanaged<NDCefHandlerBox>.fromOpaque(owner).release()
    }) else {
        Unmanaged<NDCefHandlerBox>.fromOpaque(owner).release()
        return nil
    }
    return raw.assumingMemoryBound(to: T.self)
}

/// The box behind a handler struct CEF is calling back into.
func ndCefBox(_ handler: UnsafeMutableRawPointer?) -> NDCefHandlerBox? {
    guard let handler, let owner = nd_cef_ref_owner(handler) else { return nil }
    return Unmanaged<NDCefHandlerBox>.fromOpaque(owner).takeUnretainedValue()
}

/// Runs a handler callback where AppKit needs it. Every handler wired above is
/// documented UI-thread, and CEF's UI thread IS the main thread on macOS under
/// the single-threaded message loop this host runs, so an off-main arrival is
/// a bug worth seeing rather than a case to smooth over.
func ndCefDeliver(
    _ handler: UnsafeMutableRawPointer?,
    _ body: @MainActor (NDCefWebView?) -> Void
) {
    guard let handler else { return }
    guard Thread.isMainThread else {
        ndCefWarn("CEF handler callback arrived off the main thread; dropped")
        return
    }
    // Pointers cross the isolation boundary as bit patterns: Swift's Unsafe
    // pointer types are deliberately not Sendable, and the alternative is
    // silencing the checker at every callback.
    let token = UInt(bitPattern: handler)
    MainActor.assumeIsolated { body(ndCefResolve(token)) }
}

@MainActor private func ndCefResolve(_ token: UInt) -> NDCefWebView? {
    guard let handler = UnsafeMutableRawPointer(bitPattern: token) else { return nil }
    return ndCefBox(handler)?.view
}

@MainActor private func ndCefBrowser(_ token: UInt) -> UnsafeMutablePointer<cef_browser_t>? {
    UnsafeMutableRawPointer(bitPattern: token)?.assumingMemoryBound(to: cef_browser_t.self)
}

/// `{"text": "..."}`, the same minimal escaping the WKWebView surface uses for
/// URLs and titles.
func ndCefTextJson(_ value: String) -> String {
    let escaped = value
        .replacingOccurrences(of: "\\", with: "\\\\")
        .replacingOccurrences(of: "\"", with: "\\\"")
        .replacingOccurrences(of: "\n", with: "\\n")
    return "{\"text\":\"\(escaped)\"}"
}

func ndCefParseJSON(_ raw: String) -> Any? {
    guard let data = raw.data(using: .utf8) else { return nil }
    return try? JSONSerialization.jsonObject(with: data, options: [.allowFragments])
}

/// The DevTools observer hands results and events over as raw JSON bytes.
func ndCefJSONText(_ bytes: UnsafeRawPointer?, _ count: Int) -> String {
    guard let bytes, count > 0 else { return "" }
    return String(decoding: UnsafeRawBufferPointer(start: bytes, count: count), as: UTF8.self)
}

func ndCefParseJSONText(_ raw: String) -> [String: Any]? {
    guard let data = raw.data(using: .utf8) else { return nil }
    return (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
}
/// The ordered shutdown the Chromium engine needs before `nd_shutdown` runs
/// `cef_shutdown`. Quitting without it leaves CEF to unwind by itself: it
/// closes the inspected browser first, faults in `CefBrowserInfo::RemoveFrame`,
/// and the inspector's browser is still there when `cef_shutdown` tries to join
/// the UI thread. Each phase waits for CEF's own callbacks, never a fixed
/// delay; the whole run is a few tens of milliseconds.
///
/// It runs a nested run loop because the caller is `applicationShouldTerminate`,
/// which has to answer before the callbacks it is waiting on can arrive.
@MainActor func ndCefCloseBrowsersInOrder(timeout: TimeInterval = 5) -> Bool {
    let views = NDCefWebView.liveViews.allObjects
    guard !views.isEmpty else { return true }

    // Each turn in its own pool: what the phases wait on is objects going away,
    // and an NSWindow released by Chromium's close task stays alive until the
    // pool that autoreleased it drains.
    func pump(until done: @MainActor () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while !done() {
            if Date() >= deadline { return false }
            autoreleasepool {
                RunLoop.current.run(mode: .default, before: Date().addingTimeInterval(0.01))
            }
        }
        return true
    }

    // A phase that runs out of time says which one and what it was still
    // waiting on: every later phase then runs against state the one before it
    // never reached, and "did not all close" on its own names neither.
    func phase(_ what: String, state: @MainActor () -> String, until done: @MainActor () -> Bool) -> Bool {
        let ok = pump(until: done)
        if !ok { ndCefWarn("quit: \(what) did not finish in \(Int(timeout))s (\(state()))") }
        if ProcessInfo.processInfo.environment["ND_WEBVIEW_TRACE"] == "1" {
            FileHandle.standardError.write("ND_WV quit \(what): \(ok ? "done" : "timed out")\n".data(using: .utf8)!)
        }
        return ok
    }

    // Before anything closes: a Chromium surface left as a child of a window
    // AppKit is tearing down is one more object in a quit path that already has
    // a crash of its own.
    NDCefSurfaceWindows.releaseAll()

    for view in views { view.closeDevToolsForShutdown() }
    let inspectorsClosed = phase(
        "the inspectors closing",
        state: { views.map(\.shutdownState).joined(separator: " ") },
        until: { !views.contains { $0.stillInspecting } })

    for view in views { view.releaseEngine() }
    let browsersClosed = phase(
        "the browsers closing",
        state: { views.map(\.shutdownState).joined(separator: " ") },
        until: { !views.contains { $0.stillOpen } })
    // This one times out on every quit, sound or not: it clears when the page
    // browser reports `on_before_close`, and that callback does not arrive once
    // the browsers have been closed from inside `applicationShouldTerminate`.
    let windowsClosed = phase(
        "the Views windows closing",
        state: { "a browser has not reported on_before_close" },
        until: { !NDCefChromeWindow.anyWindowOpen })

    return inspectorsClosed && browsersClosed && windowsClosed
}

/// release_node purge seam (Backend.swift's `ndPurgeNodeRegistries`).
@MainActor func ndCefPurge(_ view: NSView) {
    let engine = (view as? NDCefWebView) ?? (view as? NDWebView)?.cefEngine
    engine?.releaseEngine()
}

#endif
