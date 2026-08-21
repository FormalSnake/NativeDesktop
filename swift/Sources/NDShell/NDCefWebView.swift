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
        if let browser {
            if let browserHost = browser.pointee.get_host?(browser) {
                browserHost.pointee.close_browser?(browserHost, 1)
                nd_cef_ref_release(browserHost)
            }
            nd_cef_ref_release(browser)
        }
        box.teardown()
    }

    // MARK: - Browser lifetime

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        createBrowserIfNeeded()
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
    }

    override func viewDidHide() {
        super.viewDidHide()
        setBrowserHidden(true)
    }

    override func viewDidUnhide() {
        super.viewDidUnhide()
        setBrowserHidden(false)
    }

    private func createBrowserIfNeeded() {
        guard !createRequested, NDCefRuntime.isActive else { return }
        guard window != nil else { return }
        createRequested = true

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

        createdURL = requestedURL.isEmpty ? "about:blank" : requestedURL
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

    fileprivate func adoptBrowser(_ created: UnsafeMutablePointer<cef_browser_t>) {
        guard browser == nil else {
            nd_cef_ref_release(created)
            return
        }
        browser = created
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
        guard let handle = browserHost.pointee.get_window_handle?(browserHost) else { return }
        let view = Unmanaged<NSView>.fromOpaque(handle).takeUnretainedValue()
        view.frame = bounds
        view.autoresizingMask = [.width, .height]
        if isHiddenOrHasHiddenAncestor {
            browserHost.pointee.was_hidden?(browserHost, 1)
        }
    }

    fileprivate func forgetBrowser() {
        guard let browser else { return }
        self.browser = nil
        nd_cef_ref_release(browser)
    }

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

    override func becomeFirstResponder() -> Bool {
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
    private func openDevTools(_ obj: [String: Any]) {
        guard let browserHost = browserHost() else { return }
        defer { nd_cef_ref_release(browserHost) }
        devToolsRequested = true
        var windowInfo = cef_window_info_t()
        if let x = (obj["x"] as? NSNumber)?.int32Value, let y = (obj["y"] as? NSNumber)?.int32Value {
            var point = cef_point_t(x: x, y: y)
            browserHost.pointee.show_dev_tools?(browserHost, &windowInfo, nil, nil, &point)
        } else {
            browserHost.pointee.show_dev_tools?(browserHost, &windowInfo, nil, nil, nil)
        }
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
    fileprivate(set) var dialog: UnsafeMutablePointer<cef_dialog_handler_t>?
    fileprivate(set) var contextMenu: UnsafeMutablePointer<cef_context_menu_handler_t>?
    fileprivate(set) var focus: UnsafeMutablePointer<cef_focus_handler_t>?
    fileprivate(set) var request: UnsafeMutablePointer<cef_request_handler_t>?
    fileprivate(set) var resourceRequest: UnsafeMutablePointer<cef_resource_request_handler_t>?
    fileprivate(set) var devToolsObserver: UnsafeMutablePointer<cef_dev_tools_message_observer_t>?
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
        dialog = ndCefAlloc(cef_dialog_handler_t.self, self)
        contextMenu = ndCefAlloc(cef_context_menu_handler_t.self, self)
        focus = ndCefAlloc(cef_focus_handler_t.self, self)
        request = ndCefAlloc(cef_request_handler_t.self, self)
        resourceRequest = ndCefAlloc(cef_resource_request_handler_t.self, self)
        devToolsObserver = ndCefAlloc(cef_dev_tools_message_observer_t.self, self)
        client = ndCefAlloc(cef_client_t.self, self)
        wireDisplay()
        wireLoad()
        wireLifeSpan()
        wireFind()
        wireDownload()
        wireJSDialog()
        wireDialog()
        wireContextMenu()
        wireFocus()
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
            request.map(UnsafeMutableRawPointer.init),
            resourceRequest.map(UnsafeMutableRawPointer.init),
            devToolsObserver.map(UnsafeMutableRawPointer.init),
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
        request = nil
        resourceRequest = nil
        devToolsObserver = nil
        devToolsRegistration = nil
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
        client.pointee.get_request_handler = { selfPointer in
            ndCefHandOut(ndCefBox(selfPointer)?.request)
        }
    }

    private func wireDisplay() {
        guard let display else { return }
        display.pointee.on_address_change = { selfPointer, browser, frame, url in
            let value = ndCefString(url)
            nd_cef_ref_release(browser)
            nd_cef_ref_release(frame)
            ndCefDeliver(selfPointer) { $0?.emitAddress(value) }
        }
        display.pointee.on_title_change = { selfPointer, browser, title in
            let value = ndCefString(title)
            nd_cef_ref_release(browser)
            ndCefDeliver(selfPointer) { $0?.emitTitle(value) }
        }
        display.pointee.on_loading_progress_change = { selfPointer, browser, progress in
            nd_cef_ref_release(browser)
            ndCefDeliver(selfPointer) { $0?.emitProgress(progress) }
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
            useDefaultWindow?.pointee = wanted ? 1 : 0
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
            nd_cef_ref_release(browser)
            ndCefDeliver(selfPointer) { $0?.forgetBrowser() }
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
        contextMenu.pointee.run_context_menu = { selfPointer, browser, frame, _, model, callback in
            nd_cef_ref_release(browser)
            nd_cef_ref_release(frame)
            nd_cef_ref_release(model)
            let suppress = ndCefBox(selfPointer)?.view?.suppressContextMenu ?? false
            if suppress {
                callback?.pointee.cancel?(callback)
                nd_cef_ref_release(callback)
                return 1
            }
            nd_cef_ref_release(callback)
            return 0
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
                handled = (view?.ndContextMenuCommand(commandID) ?? false) ? 1 : 0
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

    /// A registered scheme is served from here rather than from its handler
    /// factory. Chromium owns some scheme names outright (chrome-extension is
    /// the one that matters: its own loader answers ERR_BLOCKED_BY_CLIENT for
    /// an id it does not know, and a scheme handler factory is never
    /// consulted), and `disable_default_handling` is the documented way to
    /// take a request away from the default loader before it runs.
    private func wireRequest() {
        guard let request else { return }
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

/// Hands a handler struct to CEF with the reference the caller is owed.
private func ndCefHandOut<T>(_ handler: UnsafeMutablePointer<T>?) -> UnsafeMutablePointer<T>? {
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
#endif
