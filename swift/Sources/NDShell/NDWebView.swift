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

    init(url: String?) {
        super.init(frame: .zero, configuration: WKWebViewConfiguration())
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

    func ndHandleCommand(_ command: String) {
        switch command {
        case "goBack": if canGoBack { goBack() }
        case "goForward": if canGoForward { goForward() }
        case "reload": reload()
        case "stop": stopLoading()
        default:
            FileHandle.standardError.write("ND_WARN unknown WebView command \(command)\n".data(using: .utf8)!)
        }
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

/// Generated `ndWidgetCommand` WebView arm (widgetCommand NDP frame, M14).
func ndWebViewCommand(_ view: NSView, _ command: String, _ argJson: String) {
    _ = argJson // no WebView command takes an argument yet
    guard let wv = view as? NDWebView else { return }
    wv.ndHandleCommand(command)
}
