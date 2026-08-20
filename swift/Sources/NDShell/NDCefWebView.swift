#if canImport(CCef)
import AppKit
import CCef
import Foundation

/// The Chromium (CEF) surface behind `<webview>`, presenting the same internal
/// contract as the WKWebView in NDWebView.swift: `ndSetURL`, an `ndHandleCommand`
/// switch, and the schema's events out through `nd_emit_event`. NDWebView owns
/// one of these and forwards to it when `ND_WEBVIEW_ENGINE=chromium` started
/// successfully, so the generated widget code (NDGen/Widgets.swift) never
/// learns there are two engines.
///
/// Embedding is Alloy-style and windowed: CEF creates its own NSView inside
/// this one. The browser is only created once this view is in a window, which
/// is not a nicety. An unparented create silently produces a top-level
/// Chromium window over the app, the one failure the spec bans outright.
///
/// Where the WKWebView surface polls the view's navigation properties, this
/// one is push-only: CEF's display and load handlers report the same six state
/// changes directly, so there is no timer here.
final class NDCefWebView: NSView {
    /// The `<webview>` node this view reports as. Emits go through it because
    /// the node id and the event names belong to the widget, not the engine.
    weak var host: NDWebView?

    /// Retained by every handler struct below and holding this view weakly, so
    /// a callback arriving after the view is gone resolves to nothing instead
    /// of a freed object. CEF outlives the view whenever a browser is still
    /// closing.
    nonisolated(unsafe) private let box = NDCefHandlerBox()

    nonisolated(unsafe) private var browser: UnsafeMutablePointer<cef_browser_t>?
    private var createRequested = false
    /// The address to open, held until the browser exists.
    private var pendingURL: String

    // Last-emitted navigation state; events fire only on change, matching the
    // WKWebView surface. It doubles as the answer to the automation
    // `webviewInfo` RPC, which has no engine-side property to read here.
    private var lastTitle = ""
    private var lastLoading = false
    private var lastCanGoBack = false
    private var lastCanGoForward = false
    private var lastProgress: Double = -1

    init(url: String) {
        pendingURL = url
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

    override func layout() {
        super.layout()
        // CEF sizes its NSView from window_info at creation; the autoresizing
        // mask carries it from there, and this covers the layout passes that
        // move the view without resizing its superview.
        for child in subviews { child.frame = bounds }
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

        var url = cef_string_t()
        ndCefSetString(pendingURL.isEmpty ? "about:blank" : pendingURL, &url)
        defer { nd_cef_string_clear(&url) }

        if nd_cef_create_browser(&info, box.client, &url, &settings, nil, nil) == 0 {
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
        guard let browser, let browserHost = browser.pointee.get_host?(browser) else { return }
        browserHost.pointee.was_hidden?(browserHost, hidden ? 1 : 0)
        nd_cef_ref_release(browserHost)
    }

    /// The CEF view is what takes keystrokes, so the widget's `focus` command
    /// has to reach past this container.
    override func becomeFirstResponder() -> Bool {
        guard let target = subviews.first else { return super.becomeFirstResponder() }
        return window?.makeFirstResponder(target) ?? false
    }

    // MARK: - Props and commands

    /// createAndUpdate `url` prop. Same echo guard as the WKWebView surface:
    /// the app feeds `onNavigate` back into the prop, so a load only starts
    /// when the address actually differs from where the view already is.
    func ndSetURL(_ value: String) {
        guard !value.isEmpty, value != currentURL else { return }
        guard let browser, let frame = browser.pointee.get_main_frame?(browser) else {
            pendingURL = value
            return
        }
        defer { nd_cef_ref_release(frame) }
        var url = cef_string_t()
        ndCefSetString(value, &url)
        defer { nd_cef_string_clear(&url) }
        frame.pointee.load_url?(frame, &url)
    }

    private var currentURL: String {
        pendingURL
    }

    func ndHandleCommand(_ command: String, argJson: String) {
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
            guard let zoom = ndCefParseJSON(argJson) as? NSNumber else {
                ndCefWarn("malformed setZoom arg")
                return
            }
            setZoom(zoom.doubleValue)
        default:
            // The rest of the 26-command contract rides the DevTools protocol
            // and per-context objects, which is M2.
            ndCefWarn("WebView command \(command) is not implemented on the chromium engine yet")
        }
    }

    /// CEF zoom is a log scale around 1.0; the widget's factor is linear.
    private func setZoom(_ factor: Double) {
        guard factor > 0, let browser, let browserHost = browser.pointee.get_host?(browser) else { return }
        defer { nd_cef_ref_release(browserHost) }
        browserHost.pointee.set_zoom_level?(browserHost, log(factor) / log(1.2))
    }

    // MARK: - Events

    fileprivate func emitText(_ name: String, _ value: String) {
        host?.emitEvent(name, json: ndCefTextJson(value))
    }

    fileprivate func emitAddress(_ value: String) {
        pendingURL = value
        emitText("navigate", value)
    }

    fileprivate func emitTitle(_ value: String) {
        lastTitle = value
        emitText("titleChanged", value)
    }

    /// What the automation `webviewInfo` RPC reports for a chromium view.
    var ndPageState: NDWebViewPageState {
        NDWebViewPageState(
            url: pendingURL.isEmpty ? nil : pendingURL,
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
}

// MARK: - Handlers

/// Owns the four capi structs for one view. CEF holds references to them for
/// as long as a browser exists, which can outlast the NSView, so the structs
/// point here and this points back weakly.
final class NDCefHandlerBox {
    weak var view: NDCefWebView?
    fileprivate(set) var client: UnsafeMutablePointer<cef_client_t>?
    fileprivate(set) var display: UnsafeMutablePointer<cef_display_handler_t>?
    fileprivate(set) var load: UnsafeMutablePointer<cef_load_handler_t>?
    fileprivate(set) var lifeSpan: UnsafeMutablePointer<cef_life_span_handler_t>?

    func build() {
        display = ndCefAlloc(cef_display_handler_t.self, self)
        load = ndCefAlloc(cef_load_handler_t.self, self)
        lifeSpan = ndCefAlloc(cef_life_span_handler_t.self, self)
        client = ndCefAlloc(cef_client_t.self, self)
        wireDisplay()
        wireLoad()
        wireLifeSpan()
        wireClient()
    }

    /// Drops the view's own reference on each struct. CEF may still hold its
    /// own, in which case the block survives with a nil `view`.
    func teardown() {
        nd_cef_ref_release(client)
        nd_cef_ref_release(display)
        nd_cef_ref_release(load)
        nd_cef_ref_release(lifeSpan)
        client = nil
        display = nil
        load = nil
        lifeSpan = nil
    }

    private func wireClient() {
        guard let client else { return }
        // Every getter hands back a NEW reference: the caller owns what it
        // receives, per the capi contract.
        client.pointee.get_display_handler = { selfPointer in
            guard let box = ndCefBox(selfPointer), let handler = box.display else { return nil }
            nd_cef_ref_add(handler)
            return handler
        }
        client.pointee.get_load_handler = { selfPointer in
            guard let box = ndCefBox(selfPointer), let handler = box.load else { return nil }
            nd_cef_ref_add(handler)
            return handler
        }
        client.pointee.get_life_span_handler = { selfPointer in
            guard let box = ndCefBox(selfPointer), let handler = box.lifeSpan else { return nil }
            nd_cef_ref_add(handler)
            return handler
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
    }

    private func wireLoad() {
        guard let load else { return }
        load.pointee.on_loading_state_change = { selfPointer, browser, isLoading, canGoBack, canGoForward in
            nd_cef_ref_release(browser)
            let loading = isLoading != 0
            let back = canGoBack != 0
            let forward = canGoForward != 0
            ndCefDeliver(selfPointer) {
                $0?.updateLoadState(loading: loading, canGoBack: back, canGoForward: forward)
            }
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
            // No CEF-created devtools window; M2 routes this through a window
            // the host owns.
            useDefaultWindow?.pointee = 0
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
}

/// One refcounted capi struct, owned by |box|. This is the only place handler
/// objects are allocated: the base callbacks and the atomic count live in
/// CCef, never open-coded per handler.
private func ndCefAlloc<T>(_ type: T.Type, _ box: NDCefHandlerBox) -> UnsafeMutablePointer<T>? {
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
private func ndCefBox(_ handler: UnsafeMutableRawPointer?) -> NDCefHandlerBox? {
    guard let handler, let owner = nd_cef_ref_owner(handler) else { return nil }
    return Unmanaged<NDCefHandlerBox>.fromOpaque(owner).takeUnretainedValue()
}

/// Runs a handler callback where AppKit needs it. Every handler wired above is
/// documented UI-thread, and CEF's UI thread IS the main thread on macOS under
/// the single-threaded message loop this host runs, so an off-main arrival is
/// a bug worth seeing rather than a case to smooth over.
private func ndCefDeliver(
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
private func ndCefTextJson(_ value: String) -> String {
    let escaped = value
        .replacingOccurrences(of: "\\", with: "\\\\")
        .replacingOccurrences(of: "\"", with: "\\\"")
        .replacingOccurrences(of: "\n", with: "\\n")
    return "{\"text\":\"\(escaped)\"}"
}

private func ndCefParseJSON(_ raw: String) -> Any? {
    guard let data = raw.data(using: .utf8) else { return nil }
    return try? JSONSerialization.jsonObject(with: data, options: [.allowFragments])
}
#endif
