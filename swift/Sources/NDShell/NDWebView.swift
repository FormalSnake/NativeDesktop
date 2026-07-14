import AppKit
import Foundation
import WebKit
import CNd

/// WKWebView-backed surface for the `<webview>` widget (M14, peer of the GTK
/// surface in `src/gtk/webview.zig`). The system engine IS the widget — no
/// bundled browser, matching the toolkit's real-native-widgets contract.
///
/// Event flow: the schema's navigate/titleChanged/loadingChanged/
/// backAvailable/forwardAvailable events are derived by polling the view's
/// own navigation properties on a 10 Hz timer and emitting on change — the
/// same poll-don't-push idiom NDTerminalView ships (correct first, fast
/// later), and it catches SPA pushState URL changes that WKNavigationDelegate
/// callbacks miss. Imperative goBack/goForward/reload/stop arrive through the
/// generated `ndWidgetCommand` WebView arm (widgetCommand NDP frame).
final class NDWebView: WKWebView {
    /// Node id recorded by `ndWebViewConnect` (generated ndConnectEvents arm,
    /// same create-op batch as construction). 0 = not yet wired; emits gate
    /// on it so nothing fires into node 0.
    var ndNodeID: UInt32 = 0
    nonisolated(unsafe) private var pollTimer: Timer?

    // Last-emitted navigation state; events fire only on change.
    private var lastURL = ""
    private var lastTitle = ""
    private var lastLoading = false
    private var lastCanGoBack = false
    private var lastCanGoForward = false
    // Rounded to 3 decimals so float noise in estimatedProgress doesn't fire
    // spurious diffs; -1 sentinel so the first real value (even 0.0) still emits.
    private var lastProgress: Double = -1
    // URL of the in-flight provisional navigation, captured at didStart so
    // didCommit can detect WebKit's silent about:blank substitution (blocked
    // ports, some malformed URLs) — those never fire a didFail* callback.
    private var pendingProvisionalURL = ""

    init(url: String?) {
        super.init(frame: .zero, configuration: WKWebViewConfiguration())
        // Self as navigationDelegate/uiDelegate is additive to the poll loop
        // below: polling stays authoritative for url/title/isLoading/
        // canGoBack/canGoForward (it's what catches SPA pushState changes);
        // the delegate only covers cases polling can't observe — load
        // failures, downloads, and window.open popups.
        navigationDelegate = self
        uiDelegate = self
        if let u = url, !u.isEmpty, let real = URL(string: u) {
            load(URLRequest(url: real))
        }
        pollTimer = Timer.scheduledTimer(withTimeInterval: 1.0 / 10.0, repeats: true) { [weak self] _ in
            self?.pollNavigationState()
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("NDWebView is not NSCoding-decodable") }

    deinit {
        pollTimer?.invalidate()
    }

    /// createAndUpdate `url` prop (generated ndApplyProps arm): navigate iff
    /// it differs from the current URL — the echo guard, since onNavigate
    /// feeds the URL back into app state, which re-applies the prop.
    func ndSetURL(_ u: String) {
        guard !u.isEmpty, u != url?.absoluteString, let real = URL(string: u) else { return }
        load(URLRequest(url: real))
    }

    func ndHandleCommand(_ command: String, argJson: String) {
        switch command {
        case "goBack": if canGoBack { goBack() }
        case "goForward": if canGoForward { goForward() }
        case "reload": reload()
        case "stop": stopLoading()
        case "executeJavaScript":
            guard let obj = ndWebViewParseJSON(argJson) as? [String: Any],
                  let id = obj["id"] as? String,
                  let code = obj["code"] as? String else {
                ndWarn("malformed executeJavaScript arg")
                return
            }
            evaluateJavaScript(code) { [weak self] result, error in
                self?.ndEmitJavaScriptResult(id: id, result: result, error: error)
            }
        case "setZoom":
            guard let zoom = ndWebViewParseJSON(argJson) as? NSNumber else {
                ndWarn("malformed setZoom arg")
                return
            }
            pageZoom = CGFloat(zoom.doubleValue)
        case "setUserAgent":
            guard let ua = ndWebViewParseJSON(argJson) as? String else {
                ndWarn("malformed setUserAgent arg")
                return
            }
            customUserAgent = ua.isEmpty ? nil : ua
        case "openDevTools":
            if #available(macOS 13.3, *) {
                isInspectable = true
                ndWarn("openDevTools: no programmatic open on macOS — attach via Safari's Develop menu")
            } else {
                ndWarn("openDevTools requires macOS 13.3+")
            }
        default:
            ndWarn("unknown WebView command \(command)")
        }
    }

    /// `executeJavaScript` completion (M14+): builds the `javaScriptResult`
    /// "data"-envelope `{id, ok, value?, error?}`. `value` is always a string
    /// — dictionaries/arrays JSON-serialize, everything else goes through
    /// `String(describing:)` — and is omitted when the JS result is nil.
    private func ndEmitJavaScriptResult(id: String, result: Any?, error: Error?) {
        var fields: [(String, NDWebViewJSONValue)] = [("id", .string(id))]
        if let error {
            let nsError = error as NSError
            // The thrown JS message lives in WKJavaScriptExceptionMessage;
            // localizedDescription is only the generic "A JavaScript exception
            // occurred", so prefer the real message when WebKit provides it.
            let message = (nsError.userInfo["WKJavaScriptExceptionMessage"] as? String) ?? nsError.localizedDescription
            fields.append(("ok", .bool(false)))
            fields.append(("error", .string(message)))
        } else {
            fields.append(("ok", .bool(true)))
            if let value = Self.stringifyJSResult(result) {
                fields.append(("value", .string(value)))
            }
        }
        emitEvent("javaScriptResult", json: ndWebViewDataJson(ndWebViewJsonObject(fields)))
    }

    private static func stringifyJSResult(_ result: Any?) -> String? {
        guard let result, !(result is NSNull) else { return nil }
        if result is [String: Any] || result is [Any],
           JSONSerialization.isValidJSONObject(result),
           let data = try? JSONSerialization.data(withJSONObject: result),
           let s = String(data: data, encoding: .utf8) {
            return s
        }
        return String(describing: result)
    }

    /// `didFail`/`didFailProvisionalNavigation` -> `loadFailed`. Two classes of
    /// navigation error are routine noise, not real failures:
    ///   - NSURLErrorCancelled (-999): a newer load superseding an in-flight one.
    ///   - WebKitErrorDomain 102 (frame load interrupted by policy): the tail of
    ///     a navigation we cancelled ourselves — a response turned into a
    ///     download (see the response-policy delegate) or a resource WebKit
    ///     refused. Surfacing it would fire a bogus loadFailed carrying the
    ///     PREVIOUS page's URL right after the real downloadRequested event.
    private func ndEmitLoadFailed(_ error: Error) {
        let nsError = error as NSError
        if nsError.domain == NSURLErrorDomain, nsError.code == NSURLErrorCancelled { return }
        if nsError.domain == "WebKitErrorDomain", nsError.code == 102 { return }
        // The failing URL comes from the error, never `url` (which still points
        // at the last committed page — the stale previous URL). WebKit populates
        // NSErrorFailingURLKey (an NSURL) here, not the *String* key the docs
        // suggest, so read both.
        let failingURL = Self.failingURL(from: nsError) ?? url?.absoluteString ?? ""
        emitLoadFailed(url: failingURL, message: nsError.localizedDescription)
    }

    private static func failingURL(from nsError: NSError) -> String? {
        if let s = nsError.userInfo[NSURLErrorFailingURLStringErrorKey] as? String { return s }
        if let u = nsError.userInfo[NSURLErrorFailingURLErrorKey] as? URL { return u.absoluteString }
        return nil
    }

    private func emitLoadFailed(url: String, message: String) {
        emitEvent("loadFailed", json: ndWebViewDataJson(ndWebViewJsonObject([
            ("url", .string(url)),
            ("error", .string(message)),
        ])))
    }

    private func ndWarn(_ message: String) {
        FileHandle.standardError.write("ND_WARN \(message)\n".data(using: .utf8)!)
    }

    private func pollNavigationState() {
        guard ndNodeID != 0 else { return }
        let u = url?.absoluteString ?? ""
        if u != lastURL, !u.isEmpty {
            lastURL = u
            emitEvent("navigate", json: ndWebViewTextJson(u))
        }
        let t = title ?? ""
        if t != lastTitle {
            lastTitle = t
            emitEvent("titleChanged", json: ndWebViewTextJson(t))
        }
        if isLoading != lastLoading {
            lastLoading = isLoading
            emitEvent("loadingChanged", json: "{\"checked\":\(isLoading)}")
        }
        if canGoBack != lastCanGoBack {
            lastCanGoBack = canGoBack
            emitEvent("backAvailable", json: "{\"checked\":\(canGoBack)}")
        }
        if canGoForward != lastCanGoForward {
            lastCanGoForward = canGoForward
            emitEvent("forwardAvailable", json: "{\"checked\":\(canGoForward)}")
        }
        let progress = (estimatedProgress * 1000).rounded() / 1000
        if progress != lastProgress {
            lastProgress = progress
            emitEvent("loadProgress", json: "{\"value\":\(progress)}")
        }
    }

    private func emitEvent(_ name: String, json: String) {
        json.withCString { cJson in
            name.withCString { cName in
                nd_emit_event(gCtx, ndNodeID, cName, cJson)
            }
        }
    }
}

/// `{"text": "..."}` with the same minimal escaping as Events.swift's private
/// jsonObject (URLs/titles are the only free-form strings on this path).
private func ndWebViewTextJson(_ value: String) -> String {
    let escaped = value
        .replacingOccurrences(of: "\\", with: "\\\\")
        .replacingOccurrences(of: "\"", with: "\\\"")
        .replacingOccurrences(of: "\n", with: "\\n")
    return "{\"text\":\"\(escaped)\"}"
}

/// Records the node id so the poll loop can emit (generated `ndConnectEvents`
/// WebView arm). One call wires all five schema events.
func ndWebViewConnect(_ view: NSView, nodeID: UInt32) {
    guard let wv = view as? NDWebView else { return }
    wv.ndNodeID = nodeID
}

/// Generated `ndWidgetCommand` WebView arm (widgetCommand NDP frame, M14+).
func ndWebViewCommand(_ view: NSView, _ command: String, _ argJson: String) {
    guard let wv = view as? NDWebView else { return }
    wv.ndHandleCommand(command, argJson: argJson)
}

// MARK: - WKNavigationDelegate / WKUIDelegate (additive to the poll loop above)

extension NDWebView: WKNavigationDelegate, WKUIDelegate {
    func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
        pendingProvisionalURL = webView.url?.absoluteString ?? ""
    }

    /// WebKit silently substitutes `about:blank` for navigations it refuses
    /// (blocked ports like `:9`, and some malformed URLs) — it commits the blank
    /// document with NO `didFail*` callback, so the app would otherwise never
    /// learn the requested address failed to load. Detect the substitution
    /// (a non-blank provisional URL committing as `about:blank`) and surface it.
    func webView(_ webView: WKWebView, didCommit navigation: WKNavigation!) {
        let committed = webView.url?.absoluteString ?? ""
        let requested = pendingProvisionalURL
        pendingProvisionalURL = ""
        if committed == "about:blank", !requested.isEmpty, requested != "about:blank" {
            emitLoadFailed(url: requested, message: "The address could not be loaded.")
        }
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        pendingProvisionalURL = ""
        ndEmitLoadFailed(error)
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        pendingProvisionalURL = ""
        ndEmitLoadFailed(error)
    }

    /// Download detection (M14+): a response the engine can't render itself
    /// (`canShowMIMEType == false`) is the app's cue to hand the URL to Bun
    /// for the real download — cancel it here instead of letting WebKit try to
    /// display it. Every OTHER response must still explicitly `.allow`; this
    /// delegate has no default policy once implemented. `decidePolicyFor
    /// navigationAction` is deliberately left to WebKit's default `.allow` so
    /// normal navigation (and the url-prop echo guard in `ndSetURL`) isn't
    /// disturbed. NOTE: this MUST be the `async` form — WebKit does not invoke
    /// the `decisionHandler:` completion-handler variant of this WK_SWIFT_ASYNC
    /// method on this SDK, so the closure form silently never fires.
    func webView(_ webView: WKWebView, decidePolicyFor navigationResponse: WKNavigationResponse) async -> WKNavigationResponsePolicy {
        guard navigationResponse.canShowMIMEType else {
            emitEvent("downloadRequested", json: ndWebViewDataJson(ndWebViewJsonObject([
                ("url", .string(navigationResponse.response.url?.absoluteString ?? "")),
                ("suggestedFilename", .optionalString(navigationResponse.response.suggestedFilename)),
            ])))
            return .cancel
        }
        return .allow
    }

    /// `target="_blank"`/`window.open` popups (M14+): no native window gets
    /// created — the app opens a native tab from the emitted URL instead —
    /// so this always returns nil.
    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration, for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        if let u = navigationAction.request.url?.absoluteString {
            emitEvent("newWindow", json: ndWebViewTextJson(u))
        }
        return nil
    }
}

// MARK: - minimal JSON helpers for this file's "data"-kind envelopes (peer of
// Events.swift's private jsonObject; kept local since only WebView needs
// object-shaped payloads outside that dispatcher).

private enum NDWebViewJSONValue {
    case string(String)
    case optionalString(String?)
    case bool(Bool)
}

private func ndWebViewJsonObject(_ fields: [(String, NDWebViewJSONValue)]) -> String {
    var parts: [String] = []
    for (k, v) in fields {
        switch v {
        case .string(let s):
            parts.append("\"\(k)\":\"\(ndWebViewJsonEscape(s))\"")
        case .optionalString(let s):
            guard let s else { continue }
            parts.append("\"\(k)\":\"\(ndWebViewJsonEscape(s))\"")
        case .bool(let b):
            parts.append("\"\(k)\":\(b)")
        }
    }
    return "{" + parts.joined(separator: ",") + "}"
}

private func ndWebViewJsonEscape(_ value: String) -> String {
    value
        .replacingOccurrences(of: "\\", with: "\\\\")
        .replacingOccurrences(of: "\"", with: "\\\"")
        .replacingOccurrences(of: "\n", with: "\\n")
}

/// Wraps an already-built JSON object under `"data"` — the `EventPayload.data`
/// field (`src/generated/protocol.zig`) every "data"-kind event rides on the
/// wire as (peer of NativeView's `nativeEvent`, which wraps a plugin payload
/// the same way at the core, `src/abi.zig`'s `pluginEmit`).
private func ndWebViewDataJson(_ inner: String) -> String {
    "{\"data\":\(inner)}"
}

/// Raw-fragment JSON decode for command args (`setZoom`'s bare number,
/// `setUserAgent`'s bare string, `executeJavaScript`'s object) — `parseProps`
/// (NDGen/Widgets.swift) only accepts a top-level JSON object, so command
/// args need their own decode since an arg can be any JSON type.
private func ndWebViewParseJSON(_ raw: String) -> Any? {
    guard let data = raw.data(using: .utf8) else { return nil }
    return try? JSONSerialization.jsonObject(with: data, options: [.allowFragments])
}
