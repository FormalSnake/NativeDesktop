import AppKit
import CryptoKit
import Foundation
import WebKit
import CNd

/// WKWebView-backed surface for the `<webview>` widget (peer of the GTK
/// surface in `src/gtk/webview.zig`). The system engine IS the widget: no
/// browser is bundled, per the toolkit's real-native-widgets contract.
///
/// Event flow: the schema's navigate/titleChanged/loadingChanged/
/// backAvailable/forwardAvailable events are derived by polling the view's
/// own navigation properties on a 10 Hz timer and emitting on change — the
/// same poll-don't-push idiom NDTerminalView ships (correct first, fast
/// later), and it catches SPA pushState URL changes that WKNavigationDelegate
/// callbacks miss. Imperative goBack/goForward/reload/stop arrive through the
/// generated `ndWidgetCommand` WebView arm (widgetCommand NDP frame).
///
/// The browser/extension surface (user scripts, script messages, custom URI
/// schemes, cookies, find, favicons, TLS state, link hover, context menus,
/// profiles, session state, audio) mirrors the GTK file's contract. Where
/// WKWebView has no native hook — hovered link, context-menu hit test, media
/// playback state — the implementation uses this file's OWN internal user
/// script in a private content world (`nd-internal`, handler `__ndInternal`),
/// which is exactly the machinery the public commands expose, so apps and the
/// framework never collide on a handler name.
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

    private let suppressContextMenu: Bool
    /// App-registered user scripts, in insertion order. WKUserContentController
    /// can only remove ALL scripts, so every add/remove rebuilds the whole set
    /// (internal bootstrap first, then the app's) from this registry.
    private var userScripts: [(id: String, script: WKUserScript, world: String)] = []
    private var messageHandlers: [String: WKContentWorld] = [:]
    private var cookieObserver: NDCookieObserver?
    private var lastSecure: Bool?

    static let internalWorldName = "nd-internal"
    static let internalHandlerName = "__ndInternal"

    init(url: String?, profile: String, suppressContextMenu: Bool) {
        self.suppressContextMenu = suppressContextMenu
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = NDWebViewProfiles.dataStore(for: profile)
        NDWebViewSchemes.install(into: configuration)
        super.init(frame: .zero, configuration: configuration)
        // Self as navigationDelegate/uiDelegate is additive to the poll loop
        // below: polling stays authoritative for url/title/isLoading/
        // canGoBack/canGoForward (it's what catches SPA pushState changes);
        // the delegate only covers cases polling can't observe — load
        // failures, downloads, and window.open popups.
        navigationDelegate = self
        uiDelegate = self
        installInternalMachinery()
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

    // MARK: - Command dispatch

    func ndHandleCommand(_ command: String, argJson: String) {
        let arg = ndWebViewParseJSON(argJson)
        let obj = arg as? [String: Any] ?? [:]
        switch command {
        case "goBack": if canGoBack { goBack() }
        case "goForward": if canGoForward { goForward() }
        case "reload": reload()
        case "stop": stopLoading()
        case "executeJavaScript":
            guard let id = obj["id"] as? String, let code = obj["code"] as? String else {
                ndWarn("malformed executeJavaScript arg")
                return
            }
            let world = Self.contentWorld(named: obj["world"] as? String ?? "")
            evaluateJavaScript(code, in: nil, in: world) { [weak self] result in
                switch result {
                case .success(let value): self?.ndEmitJavaScriptResult(id: id, result: value, error: nil)
                case .failure(let error): self?.ndEmitJavaScriptResult(id: id, result: nil, error: error)
                }
            }
        case "setZoom":
            guard let zoom = arg as? NSNumber else {
                ndWarn("malformed setZoom arg")
                return
            }
            pageZoom = CGFloat(zoom.doubleValue)
        case "setUserAgent":
            guard let ua = arg as? String else {
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
        case "addUserScript": ndAddUserScript(obj)
        case "removeUserScript":
            guard let id = obj["id"] as? String else {
                ndWarn("removeUserScript: missing id")
                return
            }
            userScripts.removeAll { $0.id == id }
            rebuildUserScripts()
        case "clearUserScripts":
            if let world = obj["world"] as? String, !world.isEmpty {
                userScripts.removeAll { $0.world == world }
            } else {
                userScripts.removeAll()
            }
            rebuildUserScripts()
        case "registerScriptMessage": ndRegisterScriptMessage(obj)
        case "unregisterScriptMessage": ndUnregisterScriptMessage(obj)
        case "respondScheme": NDWebViewSchemes.respond(obj)
        case "getCookies": ndGetCookies(obj)
        case "setCookie": ndSetCookie(obj)
        case "deleteCookie": ndDeleteCookie(obj)
        case "findStart", "findNext", "findPrevious": ndFind(command, obj)
        case "findStop":
            lastFindText = ""
        case "saveSession": ndSaveSession(obj)
        case "restoreSession": ndRestoreSession(obj)
        case "setMuted":
            let muted = (arg as? NSNumber)?.boolValue ?? (obj["muted"] as? NSNumber)?.boolValue ?? false
            ndSetMuted(muted)
        default:
            ndWarn("unknown WebView command \(command)")
        }
    }

    // MARK: - User scripts

    /// A named world maps to a client content world; an empty name is the
    /// page's own world, matching WebKitGTK's NULL world_name.
    static func contentWorld(named name: String) -> WKContentWorld {
        name.isEmpty ? .page : .world(name: name)
    }

    private func ndAddUserScript(_ obj: [String: Any]) {
        guard let id = obj["id"] as? String, let source = obj["source"] as? String else {
            ndWarn("addUserScript: id and source are required")
            return
        }
        let world = obj["world"] as? String ?? ""
        let atStart = (obj["injectionTime"] as? String) == "start"
        let allFrames = (obj["allFrames"] as? NSNumber)?.boolValue ?? false
        // WKUserScript has no allow/block URL lists, so the framework compiles
        // them into a guard around the source — same observable behaviour as
        // WebKitGTK's native lists, expressed in the only place WebKit gives us.
        let guarded = Self.guardSource(
            source,
            allowList: obj["allowList"] as? [String] ?? [],
            blockList: obj["blockList"] as? [String] ?? []
        )
        let script = WKUserScript(
            source: guarded,
            injectionTime: atStart ? .atDocumentStart : .atDocumentEnd,
            forMainFrameOnly: !allFrames,
            in: Self.contentWorld(named: world)
        )
        userScripts.removeAll { $0.id == id }
        userScripts.append((id: id, script: script, world: world))
        rebuildUserScripts()
    }

    /// WKUserContentController only supports removing every script at once, so
    /// a keyed registry has to be replayed in full on each mutation. The
    /// internal bootstrap always goes back in first.
    private func rebuildUserScripts() {
        configuration.userContentController.removeAllUserScripts()
        configuration.userContentController.addUserScript(Self.internalBootstrapScript(suppressContextMenu: suppressContextMenu))
        for entry in userScripts { configuration.userContentController.addUserScript(entry.script) }
    }

    private static func guardSource(_ source: String, allowList: [String], blockList: [String]) -> String {
        if allowList.isEmpty && blockList.isEmpty { return source }
        let allow = jsonArrayLiteral(allowList)
        let block = jsonArrayLiteral(blockList)
        return """
        (function(){
          var __ndAllow = \(allow), __ndBlock = \(block);
          function __ndMatch(pattern, url) {
            var rx = new RegExp("^" + pattern.split("*").map(function (p) {
              return p.replace(/[.+?^${}()|[\\]\\\\]/g, "\\\\$&");
            }).join(".*") + "$");
            return rx.test(url);
          }
          var __ndUrl = location.href;
          if (__ndBlock.some(function (p) { return __ndMatch(p, __ndUrl); })) return;
          if (__ndAllow.length && !__ndAllow.some(function (p) { return __ndMatch(p, __ndUrl); })) return;
        \(source)
        })();
        """
    }

    private static func jsonArrayLiteral(_ values: [String]) -> String {
        guard let data = try? JSONSerialization.data(withJSONObject: values),
              let s = String(data: data, encoding: .utf8) else { return "[]" }
        return s
    }

    // MARK: - Script messages

    private func ndRegisterScriptMessage(_ obj: [String: Any]) {
        guard let name = obj["name"] as? String, !name.isEmpty else {
            ndWarn("registerScriptMessage: missing name")
            return
        }
        let worldName = obj["world"] as? String ?? ""
        let world = Self.contentWorld(named: worldName)
        if messageHandlers[name] != nil {
            configuration.userContentController.removeScriptMessageHandler(forName: name, contentWorld: messageHandlers[name]!)
        }
        messageHandlers[name] = world
        configuration.userContentController.add(NDScriptMessageProxy(owner: self, world: worldName), contentWorld: world, name: name)
    }

    private func ndUnregisterScriptMessage(_ obj: [String: Any]) {
        guard let name = obj["name"] as? String, let world = messageHandlers.removeValue(forKey: name) else { return }
        configuration.userContentController.removeScriptMessageHandler(forName: name, contentWorld: world)
    }

    fileprivate func ndEmitScriptMessage(name: String, world: String, body: Any) {
        emitData("scriptMessage", ["name": name, "world": world, "body": ndWebViewJSONSafe(body)])
    }

    // MARK: - Internal machinery (hover / context menu / audio)

    private func installInternalMachinery() {
        configuration.userContentController.add(
            NDInternalMessageProxy(owner: self),
            contentWorld: WKContentWorld.world(name: Self.internalWorldName),
            name: Self.internalHandlerName
        )
        rebuildUserScripts()
    }

    /// The framework's own page-side agent: WKWebView exposes no hovered-link
    /// callback, no context-menu hit test, and no media-playback state, so all
    /// three are observed in the page and posted back over an internal handler
    /// in a private world.
    private static func internalBootstrapScript(suppressContextMenu: Bool) -> WKUserScript {
        let source = """
        (function () {
          var post = function (m) {
            try { window.webkit.messageHandlers.\(internalHandlerName).postMessage(m); } catch (e) {}
          };
          var lastHover = "";
          var linkOf = function (el) {
            while (el && el !== document) {
              if (el.tagName === "A" && el.href) return el.href;
              el = el.parentElement;
            }
            return "";
          };
          document.addEventListener("mouseover", function (e) {
            var href = linkOf(e.target);
            if (href !== lastHover) { lastHover = href; post({ k: "hover", url: href }); }
          }, true);
          document.addEventListener("mouseout", function (e) {
            if (!e.relatedTarget && lastHover !== "") { lastHover = ""; post({ k: "hover", url: "" }); }
          }, true);
          document.addEventListener("contextmenu", function (e) {
            var el = e.target;
            var editable = !!(el && (el.isContentEditable || el.tagName === "INPUT" || el.tagName === "TEXTAREA"));
            var image = (el && el.tagName === "IMG" && el.src) ? el.src : "";
            var sel = String(window.getSelection ? window.getSelection() : "");
            post({ k: "menu", x: e.clientX, y: e.clientY, link: linkOf(el), image: image, selection: sel, editable: editable });
            if (\(suppressContextMenu ? "true" : "false")) e.preventDefault();
          }, true);
          var muted = false;
          var report = function () {
            var media = document.querySelectorAll("video, audio");
            var playing = false;
            for (var i = 0; i < media.length; i++) if (!media[i].paused && !media[i].muted) playing = true;
            post({ k: "audio", playing: playing, muted: muted });
          };
          window.__ndSetMuted = function (m) {
            muted = !!m;
            var media = document.querySelectorAll("video, audio");
            for (var i = 0; i < media.length; i++) media[i].muted = muted;
            report();
          };
          ["play", "pause", "ended", "volumechange"].forEach(function (name) {
            document.addEventListener(name, function () {
              if (muted && event && event.target && "muted" in event.target) event.target.muted = true;
              report();
            }, true);
          });
        })();
        """
        return WKUserScript(source: source, injectionTime: .atDocumentStart, forMainFrameOnly: false, in: WKContentWorld.world(name: internalWorldName))
    }

    fileprivate func ndHandleInternalMessage(_ body: Any) {
        guard let msg = body as? [String: Any], let kind = msg["k"] as? String else { return }
        switch kind {
        case "hover":
            emitEvent("linkHover", json: ndWebViewTextJson(msg["url"] as? String ?? ""))
        case "menu":
            emitData("contextMenu", [
                "x": (msg["x"] as? NSNumber)?.intValue ?? 0,
                "y": (msg["y"] as? NSNumber)?.intValue ?? 0,
                "link": msg["link"] as? String ?? "",
                "image": msg["image"] as? String ?? "",
                "selection": msg["selection"] as? String ?? "",
                "hasSelection": !((msg["selection"] as? String) ?? "").isEmpty,
                "editable": (msg["editable"] as? NSNumber)?.boolValue ?? false,
            ])
        case "audio":
            emitData("audioStateChanged", [
                "playing": (msg["playing"] as? NSNumber)?.boolValue ?? false,
                "muted": (msg["muted"] as? NSNumber)?.boolValue ?? false,
            ])
        default:
            break
        }
    }

    /// Native backstop for `suppressContextMenu`: the page-side
    /// `preventDefault()` covers pages that don't stop propagation first, and
    /// emptying the menu here covers the rest.
    override func willOpenMenu(_ menu: NSMenu, with event: NSEvent) {
        if suppressContextMenu {
            menu.removeAllItems()
            return
        }
        super.willOpenMenu(menu, with: event)
    }

    private func ndSetMuted(_ muted: Bool) {
        // WKWebView has no public mute API (only the WKWebExtensionTab
        // protocol, which is a host callback, not a control); muting the
        // page's media elements from the internal world is the real
        // equivalent, and it survives new elements through the same agent.
        evaluateJavaScript("window.__ndSetMuted(\(muted))", in: nil, in: WKContentWorld.world(name: Self.internalWorldName)) { _ in }
    }

    // MARK: - Cookies

    private var cookieStore: WKHTTPCookieStore { configuration.websiteDataStore.httpCookieStore }

    private func ndGetCookies(_ obj: [String: Any]) {
        guard let id = obj["id"] as? String else {
            ndWarn("getCookies: missing id")
            return
        }
        let host = (obj["url"] as? String).flatMap { URL(string: $0)?.host }
        cookieStore.getAllCookies { [weak self] cookies in
            let filtered = host == nil ? cookies : cookies.filter { Self.cookieMatchesHost($0, host: host!) }
            self?.emitData("cookiesResult", [
                "id": id,
                "ok": true,
                "cookies": filtered.map(Self.cookieDictionary),
            ])
        }
    }

    private static func cookieMatchesHost(_ cookie: HTTPCookie, host: String) -> Bool {
        let domain = cookie.domain.hasPrefix(".") ? String(cookie.domain.dropFirst()) : cookie.domain
        return host == domain || host.hasSuffix("." + domain)
    }

    private static func cookieDictionary(_ cookie: HTTPCookie) -> [String: Any] {
        [
            "name": cookie.name,
            "value": cookie.value,
            "domain": cookie.domain,
            "path": cookie.path,
            "secure": cookie.isSecure,
            "httpOnly": cookie.isHTTPOnly,
            "expires": cookie.expiresDate.map { Int($0.timeIntervalSince1970) } ?? NSNull(),
            "sameSite": sameSiteName(cookie),
        ]
    }

    private static func sameSiteName(_ cookie: HTTPCookie) -> String {
        guard let policy = cookie.sameSitePolicy else { return "None" }
        if policy == .sameSiteStrict { return "Strict" }
        if policy == .sameSiteLax { return "Lax" }
        return "None"
    }

    private static func buildCookie(_ obj: [String: Any]) -> HTTPCookie? {
        guard let name = obj["name"] as? String, let domain = obj["domain"] as? String else { return nil }
        var properties: [HTTPCookiePropertyKey: Any] = [
            .name: name,
            .value: obj["value"] as? String ?? "",
            .domain: domain,
            .path: obj["path"] as? String ?? "/",
        ]
        if let secure = (obj["secure"] as? NSNumber)?.boolValue, secure { properties[.secure] = "TRUE" }
        if let expires = (obj["expires"] as? NSNumber)?.doubleValue {
            properties[.expires] = Date(timeIntervalSince1970: expires)
        }
        if let sameSite = obj["sameSite"] as? String {
            switch sameSite.lowercased() {
            case "lax": properties[.sameSitePolicy] = HTTPCookieStringPolicy.sameSiteLax
            case "strict": properties[.sameSitePolicy] = HTTPCookieStringPolicy.sameSiteStrict
            default: break
            }
        }
        return HTTPCookie(properties: properties)
    }

    private func ndSetCookie(_ obj: [String: Any]) {
        let fields = (obj["cookie"] as? [String: Any]) ?? obj
        guard let cookie = Self.buildCookie(fields) else {
            ndWarn("setCookie: malformed cookie (name and domain are required)")
            return
        }
        cookieStore.setCookie(cookie, completionHandler: nil)
    }

    private func ndDeleteCookie(_ obj: [String: Any]) {
        let fields = (obj["cookie"] as? [String: Any]) ?? obj
        guard let name = fields["name"] as? String else {
            ndWarn("deleteCookie: missing name")
            return
        }
        let domain = fields["domain"] as? String
        let path = fields["path"] as? String
        // WKHTTPCookieStore deletes by identity, so the live cookie has to be
        // looked up first — a synthesized HTTPCookie with the same fields is
        // not guaranteed to match.
        cookieStore.getAllCookies { store in
            for cookie in store where cookie.name == name
                && (domain == nil || cookie.domain == domain!)
                && (path == nil || cookie.path == path!) {
                self.cookieStore.delete(cookie, completionHandler: nil)
            }
        }
    }

    fileprivate func ndEmitCookiesChanged() {
        emitData("cookiesChanged", [:])
    }

    // MARK: - Find in page

    private var lastFindText = ""

    private func ndFind(_ command: String, _ obj: [String: Any]) {
        let text: String
        if command == "findStart" {
            guard let t = obj["text"] as? String else {
                ndWarn("findStart: missing text")
                return
            }
            text = t
            lastFindText = t
        } else {
            text = lastFindText
            guard !text.isEmpty else { return }
        }
        let configuration = WKFindConfiguration()
        configuration.caseSensitive = (obj["caseSensitive"] as? NSNumber)?.boolValue ?? false
        configuration.wraps = (obj["wrap"] as? NSNumber)?.boolValue ?? true
        configuration.backwards = command == "findPrevious"
        find(text, configuration: configuration) { [weak self] result in
            // WKFindResult reports match/no-match only; WebKitGTK's counted
            // total has no AppKit equivalent, so `matchCount` is omitted here
            // and apps must treat it as optional.
            self?.emitData("findResult", ["matchFound": result.matchFound, "done": true])
        }
    }

    // MARK: - Session state

    private func ndSaveSession(_ obj: [String: Any]) {
        let id = obj["id"] as? String ?? ""
        guard #available(macOS 12.0, *), let state = interactionState as? Data else {
            ndWarn("saveSession requires macOS 12+")
            return
        }
        emitData("sessionSaved", ["id": id, "state": state.base64EncodedString()])
    }

    private func ndRestoreSession(_ obj: [String: Any]) {
        guard let encoded = obj["state"] as? String, let data = Data(base64Encoded: encoded) else {
            ndWarn("restoreSession: malformed state")
            return
        }
        guard #available(macOS 12.0, *) else {
            ndWarn("restoreSession requires macOS 12+")
            return
        }
        interactionState = data
    }

    // MARK: - Security

    private func ndEmitSecurity() {
        let isHTTPS = url?.scheme == "https"
        let secure = isHTTPS && hasOnlySecureContent
        if lastSecure == secure && isHTTPS == hasOnlySecureContent { return }
        lastSecure = secure
        emitData("securityChanged", ["secure": secure, "insecureContent": isHTTPS && !hasOnlySecureContent])
    }

    // MARK: - Favicon

    /// WKWebView has no favicon API. The page's own `<link rel=icon>` (or the
    /// origin's /favicon.ico) is resolved in-page and reported as `iconUrl`;
    /// the app fetches the bytes itself. WebKitGTK reports `dataUrl` instead,
    /// so the event shape carries whichever the engine can produce.
    private func ndEmitFavicon() {
        let js = """
        (function () {
          var best = "", bestSize = -1;
          var links = document.querySelectorAll("link[rel~='icon' i], link[rel='shortcut icon' i]");
          for (var i = 0; i < links.length; i++) {
            var sizes = (links[i].getAttribute("sizes") || "").split("x")[0];
            var n = parseInt(sizes, 10); if (isNaN(n)) n = 0;
            if (n >= bestSize) { bestSize = n; best = links[i].href; }
          }
          if (!best && location.protocol.indexOf("http") === 0) best = location.origin + "/favicon.ico";
          return JSON.stringify({ pageUrl: location.href, iconUrl: best });
        })()
        """
        evaluateJavaScript(js, in: nil, in: WKContentWorld.world(name: Self.internalWorldName)) { [weak self] result in
            guard case .success(let value) = result, let raw = value as? String,
                  let data = raw.data(using: .utf8),
                  let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let iconUrl = obj["iconUrl"] as? String, !iconUrl.isEmpty else { return }
            self?.emitData("faviconChanged", ["pageUrl": obj["pageUrl"] as? String ?? "", "iconUrl": iconUrl])
        }
    }

    // MARK: - executeJavaScript result

    /// `executeJavaScript` completion: builds the `javaScriptResult`
    /// "data"-envelope `{id, ok, value?, error?}`. `value` is always a string
    /// (dictionaries/arrays JSON-serialize, everything else goes through
    /// `String(describing:)`) and is omitted when the JS result is nil.
    private func ndEmitJavaScriptResult(id: String, result: Any?, error: Error?) {
        var fields: [String: Any] = ["id": id]
        if let error {
            let nsError = error as NSError
            // The thrown JS message lives in WKJavaScriptExceptionMessage;
            // localizedDescription is only the generic "A JavaScript exception
            // occurred", so prefer the real message when WebKit provides it.
            let message = (nsError.userInfo["WKJavaScriptExceptionMessage"] as? String) ?? nsError.localizedDescription
            fields["ok"] = false
            fields["error"] = message
        } else {
            fields["ok"] = true
            if let value = Self.stringifyJSResult(result) { fields["value"] = value }
        }
        emitData("javaScriptResult", fields)
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
        // A TLS failure is a security state change, not just a load error, and
        // the framework never silently proceeds past one.
        if nsError.domain == NSURLErrorDomain, Self.tlsErrorCodes.contains(nsError.code) {
            lastSecure = false
            emitData("securityChanged", [
                "secure": false,
                "insecureContent": false,
                "url": failingURL,
                "error": nsError.localizedDescription,
            ])
        }
    }

    private static let tlsErrorCodes: Set<Int> = [
        NSURLErrorSecureConnectionFailed,
        NSURLErrorServerCertificateHasBadDate,
        NSURLErrorServerCertificateUntrusted,
        NSURLErrorServerCertificateHasUnknownRoot,
        NSURLErrorServerCertificateNotYetValid,
        NSURLErrorClientCertificateRejected,
        NSURLErrorClientCertificateRequired,
    ]

    private static func failingURL(from nsError: NSError) -> String? {
        if let s = nsError.userInfo[NSURLErrorFailingURLStringErrorKey] as? String { return s }
        if let u = nsError.userInfo[NSURLErrorFailingURLErrorKey] as? URL { return u.absoluteString }
        return nil
    }

    private func emitLoadFailed(url: String, message: String) {
        emitData("loadFailed", ["url": url, "error": message])
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

    fileprivate func emitEvent(_ name: String, json: String) {
        json.withCString { cJson in
            name.withCString { cName in
                nd_emit_event(gCtx, ndNodeID, cName, cJson)
            }
        }
    }

    fileprivate func emitData(_ name: String, _ fields: [String: Any]) {
        emitEvent(name, json: ndWebViewDataJson(fields))
    }

    fileprivate func ndStartCookieObserver() {
        guard cookieObserver == nil else { return }
        let observer = NDCookieObserver(owner: self)
        cookieObserver = observer
        cookieStore.add(observer)
    }
}

// MARK: - Message-handler proxies
// WKScriptMessageHandler retains its handler, so NDWebView cannot be its own
// handler without a retain cycle that keeps the view (and its WebContent
// process) alive forever. Both proxies hold the view weakly.

private final class NDScriptMessageProxy: NSObject, WKScriptMessageHandler {
    private weak var owner: NDWebView?
    private let world: String

    init(owner: NDWebView, world: String) {
        self.owner = owner
        self.world = world
    }

    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        owner?.ndEmitScriptMessage(name: message.name, world: world, body: message.body)
    }
}

private final class NDInternalMessageProxy: NSObject, WKScriptMessageHandler {
    private weak var owner: NDWebView?

    init(owner: NDWebView) { self.owner = owner }

    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        owner?.ndHandleInternalMessage(message.body)
    }
}

private final class NDCookieObserver: NSObject, WKHTTPCookieStoreObserver {
    private weak var owner: NDWebView?

    init(owner: NDWebView) { self.owner = owner }

    func cookiesDidChange(in cookieStore: WKHTTPCookieStore) {
        owner?.ndEmitCookiesChanged()
    }
}

// MARK: - Profiles

/// `profile` create-only prop: "" is the shared default store, a name starting
/// with "private" is ephemeral, and any other name is a persistent store keyed
/// by a UUID derived from the name so the same profile always resolves to the
/// same jar across launches.
enum NDWebViewProfiles {
    nonisolated(unsafe) private static var stores: [String: WKWebsiteDataStore] = [:]

    static func dataStore(for profile: String) -> WKWebsiteDataStore {
        if profile.isEmpty { return .default() }
        if profile.hasPrefix("private") { return .nonPersistent() }
        if let existing = stores[profile] { return existing }
        guard #available(macOS 14.0, *) else {
            FileHandle.standardError.write("ND_WARN WebView profile \"\(profile)\": named data stores need macOS 14+; using the default store\n".data(using: .utf8)!)
            return .default()
        }
        let store = WKWebsiteDataStore(forIdentifier: identifier(for: profile))
        stores[profile] = store
        return store
    }

    private static func identifier(for profile: String) -> UUID {
        let digest = Array(SHA256.hash(data: Data(profile.utf8)))
        var bytes = (0..<16).map { digest[$0] }
        // RFC 4122 version/variant bits: WKWebsiteDataStore rejects the nil UUID.
        bytes[6] = (bytes[6] & 0x0F) | 0x50
        bytes[8] = (bytes[8] & 0x3F) | 0x80
        return UUID(uuid: (bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                           bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]))
    }
}

// MARK: - Custom URI schemes

/// `webviewEngine.registerScheme` (systemRequest, ACL `core:webview`). WebKit
/// binds scheme handlers to a WKWebViewConfiguration, which is frozen once a
/// view exists, so registration MUST happen before the first `<webview>`
/// mounts — the same constraint the GTK side documents, enforced identically.
enum NDWebViewSchemes {
    nonisolated(unsafe) private static var schemes: [String] = []
    nonisolated(unsafe) private static var anyViewCreated = false
    nonisolated(unsafe) private static var pending: [String: (request: WKURLSchemeTask, view: NDWebView)] = [:]
    nonisolated(unsafe) private static var seq = 0

    enum RegisterError: Error {
        case tooLate
        case alreadyRegistered
        case invalid
    }

    static func register(_ scheme: String) throws {
        let allowed = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyz0123456789+-.")
        guard !scheme.isEmpty, scheme.unicodeScalars.allSatisfy({ allowed.contains($0) }) else { throw RegisterError.invalid }
        if schemes.contains(scheme) { throw RegisterError.alreadyRegistered }
        if anyViewCreated { throw RegisterError.tooLate }
        schemes.append(scheme)
    }

    static func install(into configuration: WKWebViewConfiguration) {
        anyViewCreated = true
        let handler = NDURLSchemeHandler()
        for scheme in schemes { configuration.setURLSchemeHandler(handler, forURLScheme: scheme) }
    }

    static func begin(_ task: WKURLSchemeTask, view: NDWebView) {
        seq += 1
        let id = "sch\(seq)"
        pending[id] = (request: task, view: view)
        let url = task.request.url
        view.emitData("schemeRequest", [
            "id": id,
            "url": url?.absoluteString ?? "",
            "scheme": url?.scheme ?? "",
        ])
    }

    static func cancel(_ task: WKURLSchemeTask) {
        for (id, entry) in pending where entry.request === task { pending.removeValue(forKey: id) }
    }

    static func respond(_ obj: [String: Any]) {
        guard let id = obj["id"] as? String, let entry = pending.removeValue(forKey: id) else { return }
        let task = entry.request
        if let message = obj["error"] as? String {
            task.didFailWithError(NSError(domain: "NDWebView", code: 1, userInfo: [NSLocalizedDescriptionKey: message]))
            return
        }
        guard let encoded = obj["base64"] as? String, let data = Data(base64Encoded: encoded) else {
            task.didFailWithError(NSError(domain: "NDWebView", code: 2, userInfo: [NSLocalizedDescriptionKey: "respondScheme: malformed base64 body"]))
            return
        }
        let mime = obj["mime"] as? String ?? "application/octet-stream"
        let status = (obj["status"] as? NSNumber)?.intValue ?? 200
        let url = task.request.url ?? URL(string: "about:blank")!
        let response = HTTPURLResponse(
            url: url,
            statusCode: status,
            httpVersion: "HTTP/1.1",
            headerFields: ["Content-Type": mime, "Content-Length": String(data.count)]
        )!
        task.didReceive(response)
        task.didReceive(data)
        task.didFinish()
    }
}

private final class NDURLSchemeHandler: NSObject, WKURLSchemeHandler {
    func webView(_ webView: WKWebView, start urlSchemeTask: WKURLSchemeTask) {
        guard let view = webView as? NDWebView else {
            urlSchemeTask.didFailWithError(NSError(domain: "NDWebView", code: 3, userInfo: [NSLocalizedDescriptionKey: "scheme request from a foreign web view"]))
            return
        }
        NDWebViewSchemes.begin(urlSchemeTask, view: view)
    }

    func webView(_ webView: WKWebView, stop urlSchemeTask: WKURLSchemeTask) {
        NDWebViewSchemes.cancel(urlSchemeTask)
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
/// WebView arm). One call wires every schema event.
func ndWebViewConnect(_ view: NSView, nodeID: UInt32) {
    guard let wv = view as? NDWebView else { return }
    wv.ndNodeID = nodeID
    wv.ndStartCookieObserver()
}

/// Generated `ndWidgetCommand` WebView arm (widgetCommand NDP frame).
func ndWebViewCommand(_ view: NSView, _ command: String, _ argJson: String) {
    guard let wv = view as? NDWebView else { return }
    wv.ndHandleCommand(command, argJson: argJson)
}

/// `webviewEngine.registerScheme` bridge for System.swift.
func ndWebViewRegisterScheme(_ scheme: String) -> String? {
    do {
        try NDWebViewSchemes.register(scheme)
        return nil
    } catch NDWebViewSchemes.RegisterError.tooLate {
        return "registerScheme must be called before the first <webview> mounts"
    } catch NDWebViewSchemes.RegisterError.alreadyRegistered {
        return "scheme already registered"
    } catch {
        return "invalid scheme name (lowercase letters, digits, '+', '-' and '.' only)"
    }
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
        ndEmitSecurity()
        if committed == "about:blank", !requested.isEmpty, requested != "about:blank" {
            emitLoadFailed(url: requested, message: "The address could not be loaded.")
        }
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        ndEmitSecurity()
        ndEmitFavicon()
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        pendingProvisionalURL = ""
        ndEmitLoadFailed(error)
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        pendingProvisionalURL = ""
        ndEmitLoadFailed(error)
    }

    /// Download detection: a response the engine can't render itself
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
            var fields: [String: Any] = ["url": navigationResponse.response.url?.absoluteString ?? ""]
            if let name = navigationResponse.response.suggestedFilename { fields["suggestedFilename"] = name }
            emitData("downloadRequested", fields)
            return .cancel
        }
        return .allow
    }

    /// `target="_blank"`/`window.open` popups: no native window gets
    /// created; the app opens a native tab from the emitted URL instead,
    /// so this always returns nil.
    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration, for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        if let u = navigationAction.request.url?.absoluteString {
            emitEvent("newWindow", json: ndWebViewTextJson(u))
        }
        return nil
    }
}

// MARK: - JSON helpers for this file's "data"-kind envelopes

/// Wraps an already-built dictionary under `"data"` — the `EventPayload.data`
/// field (`src/generated/protocol.zig`) every "data"-kind event rides on the
/// wire as (peer of NativeView's `nativeEvent`, which wraps a plugin payload
/// the same way at the core, `src/abi.zig`'s `pluginEmit`).
private func ndWebViewDataJson(_ fields: [String: Any]) -> String {
    let wrapped: [String: Any] = ["data": fields]
    guard JSONSerialization.isValidJSONObject(wrapped),
          let data = try? JSONSerialization.data(withJSONObject: wrapped),
          let s = String(data: data, encoding: .utf8) else {
        return "{\"data\":{}}"
    }
    return s
}

/// Page-supplied values (script-message bodies) can be any JS type; anything
/// JSONSerialization would reject is degraded to its description rather than
/// dropping the whole event.
private func ndWebViewJSONSafe(_ value: Any) -> Any {
    if value is NSNull || value is NSNumber || value is String { return value }
    if let dict = value as? [String: Any] { return dict.mapValues(ndWebViewJSONSafe) }
    if let list = value as? [Any] { return list.map(ndWebViewJSONSafe) }
    return String(describing: value)
}

/// Raw-fragment JSON decode for command args (`setZoom`'s bare number,
/// `setUserAgent`'s bare string, `executeJavaScript`'s object) — `parseProps`
/// (NDGen/Widgets.swift) only accepts a top-level JSON object, so command
/// args need their own decode since an arg can be any JSON type.
private func ndWebViewParseJSON(_ raw: String) -> Any? {
    guard let data = raw.data(using: .utf8) else { return nil }
    return try? JSONSerialization.jsonObject(with: data, options: [.allowFragments])
}
