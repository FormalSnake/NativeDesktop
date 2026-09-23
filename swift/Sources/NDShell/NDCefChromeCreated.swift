#if canImport(CCef)
import AppKit
import CCef

/// Browsers Chrome creates on its own, under Chrome style.
///
/// `chrome.windows.create`, `chrome.tabs.create` into such a window and
/// `chrome.runtime.openOptionsPage` make Chrome build a browser window of its
/// own: a full "Chromium" window with a tab strip and an omnibox, reached
/// through no popup, open-URL or command callback. CEF asks
/// `get_default_client` for exactly those browsers (nd_cef.c), and the client
/// installed here keeps the window off screen, hands the app the destination as
/// `newWindow` (the contract every other route uses) and closes the browser.
/// The peer of the sink client in src/cef/engine.zig.
@MainActor enum NDCefChromeCreated {
    nonisolated(unsafe) private static var client: UnsafeMutablePointer<cef_client_t>?
    nonisolated(unsafe) private static var lifeSpan: UnsafeMutablePointer<cef_life_span_handler_t>?
    nonisolated(unsafe) private static var request: UnsafeMutablePointer<cef_request_handler_t>?

    /// A browser whose destination has not arrived yet, by browser id, with the
    /// host held so it can be closed.
    private static var pending: [Int32: UInt] = [:]
    /// The first browser Chrome makes for itself is kept, hidden, and every one
    /// after it closed. An extension install holds a raw pointer to a tabbed
    /// browser across an asynchronous wait, and one closed under it is used
    /// after it is freed (see `kept_window` in src/cef/engine.zig).
    private static var kept: Int32?
    /// Chrome's own browser windows are `BrowserNativeWidgetWindow`, a class
    /// no surface and no anchor uses: the anchors are CEF's Views windows and
    /// Chromium's sheets and bubbles are plain `NativeWidgetMacNSWindow`. The
    /// browser's view is not in its window yet when `on_after_created` runs,
    /// and Views shows the window again whenever Chrome picks the kept
    /// browser, so the surface sweep hides every one of them on every tick.
    private static let browserWindowClass = "BrowserNativeWidgetWindow"

    /// How long a browser may go without starting a navigation before it is
    /// closed unreported: a tab with no destination is a dead about:blank tab.
    private static let urlWait: TimeInterval = 1.5

    static func install() {
        guard client == nil else { return }
        client = nd_cef_ref_alloc(MemoryLayout<cef_client_t>.size, nil, nil)?
            .assumingMemoryBound(to: cef_client_t.self)
        lifeSpan = nd_cef_ref_alloc(MemoryLayout<cef_life_span_handler_t>.size, nil, nil)?
            .assumingMemoryBound(to: cef_life_span_handler_t.self)
        request = nd_cef_ref_alloc(MemoryLayout<cef_request_handler_t>.size, nil, nil)?
            .assumingMemoryBound(to: cef_request_handler_t.self)
        guard let client, let lifeSpan, let request else { return }
        client.pointee.get_life_span_handler = { _ in ndCefHandOut(NDCefChromeCreated.lifeSpan) }
        client.pointee.get_request_handler = { _ in ndCefHandOut(NDCefChromeCreated.request) }
        lifeSpan.pointee.on_after_created = { _, browser in
            guard let browser else { return }
            let token = UInt(bitPattern: browser)
            MainActor.assumeIsolated { NDCefChromeCreated.created(token) }
            nd_cef_ref_release(browser)
        }
        lifeSpan.pointee.on_before_close = { _, browser in
            let id = browser?.pointee.get_identifier?(browser) ?? -1
            nd_cef_ref_release(browser)
            MainActor.assumeIsolated { NDCefChromeCreated.closed(id) }
        }
        request.pointee.on_before_browse = { _, browser, frame, request, _, _ in
            let id = browser?.pointee.get_identifier?(browser) ?? -1
            var url = ""
            if let request, let raw = request.pointee.get_url?(request) {
                url = ndCefString(raw)
                nd_cef_string_free(raw)
            }
            nd_cef_ref_release(browser)
            nd_cef_ref_release(frame)
            nd_cef_ref_release(request)
            return MainActor.assumeIsolated { NDCefChromeCreated.browse(id, url) } ? 1 : 0
        }
        nd_cef_set_default_client(client)
        // Chrome activates the window it shows, which is the earliest moment
        // one is seen; hiding it there, rather than on the next sweep, is what
        // keeps it from ever reaching the screen and the app its key status.
        for name in [NSWindow.didBecomeKeyNotification, NSWindow.didBecomeMainNotification] {
            NotificationCenter.default.addObserver(forName: name, object: nil, queue: nil) { note in
                guard let window = note.object as? NSWindow else { return }
                let token = UInt(bitPattern: Unmanaged.passUnretained(window).toOpaque())
                MainActor.assumeIsolated { NDCefChromeCreated.activated(token) }
            }
        }
    }

    private static func activated(_ token: UInt) {
        guard let raw = UnsafeRawPointer(bitPattern: token) else { return }
        let window = Unmanaged<NSWindow>.fromOpaque(raw).takeUnretainedValue()
        guard owns(window) else { return }
        hide(window)
        let host = NDCefChromeWindow.liveWindows.lazy.compactMap(\.hostWindow).first { $0.isVisible }
        host?.makeKeyAndOrderFront(nil)
    }

    /// Whether `window` is a browser window Chrome made for itself.
    static func owns(_ window: NSWindow) -> Bool {
        NSStringFromClass(type(of: window)).contains(browserWindowClass)
    }

    private static func created(_ token: UInt) {
        guard let browser = UnsafeMutablePointer<cef_browser_t>(bitPattern: token),
              let host = browser.pointee.get_host?(browser) else { return }
        let id = browser.pointee.get_identifier?(browser) ?? -1
        // Hidden before anything else, when it already has a window; the sweep
        // catches the ones that do not yet.
        for window in NSApp.windows where owns(window) { hide(window) }
        if kept == nil { kept = id }
        pending[id] = UInt(bitPattern: host)
        trace("created id=\(id) kept=\(kept == id)")
        DispatchQueue.main.asyncAfter(deadline: .now() + urlWait) {
            MainActor.assumeIsolated { NDCefChromeCreated.expire(id) }
        }
    }

    /// The first navigation is where the destination exists. It is reported and
    /// cancelled, so nothing of it is fetched or drawn.
    private static func browse(_ id: Int32, _ url: String) -> Bool {
        guard let hostToken = pending.removeValue(forKey: id) else { return false }
        report(url)
        finish(id, hostToken)
        return true
    }

    private static func expire(_ id: Int32) {
        guard let hostToken = pending.removeValue(forKey: id) else { return }
        trace("expired id=\(id) with no destination")
        finish(id, hostToken)
    }

    private static func finish(_ id: Int32, _ hostToken: UInt) {
        guard let host = UnsafeMutablePointer<cef_browser_host_t>(bitPattern: hostToken) else { return }
        if id != kept { host.pointee.close_browser?(host, 1) }
        nd_cef_ref_release(host)
    }

    private static func closed(_ id: Int32) {
        if id == kept { kept = nil }
        if let hostToken = pending.removeValue(forKey: id),
           let host = UnsafeMutablePointer<cef_browser_host_t>(bitPattern: hostToken) {
            nd_cef_ref_release(host)
        }
    }

    /// The app hears it from the webview in the key window, the one a user reads
    /// as "this window"; any live one if no app window is key.
    private static func report(_ url: String) {
        let owners = NDCefChromeWindow.liveWindows
        let key = NSApp.keyWindow ?? NSApp.mainWindow
        let target = owners.first { $0.hostWindow != nil && $0.hostWindow === key }?.webView
            ?? owners.lazy.compactMap(\.webView).first
        trace("newWindow \(url) to node \(target?.host?.ndNodeID ?? 0)")
        target?.emitText("newWindow", url)
    }

    static func hide(_ window: NSWindow) {
        window.alphaValue = 0
        window.ignoresMouseEvents = true
        if window.isVisible { window.orderOut(nil) }
    }

    private static func trace(_ message: String) {
        guard ProcessInfo.processInfo.environment["ND_WEBVIEW_TRACE"] == "1" else { return }
        FileHandle.standardError.write("ND_WV cef chromeCreated \(message)\n".data(using: .utf8)!)
    }
}
#endif
