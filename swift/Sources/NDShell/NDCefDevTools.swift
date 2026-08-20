#if canImport(CCef)
import CCef
import Foundation

/// The DevTools-protocol substrate every scriptable part of the `<webview>`
/// contract rides on when the engine is Chromium.
///
/// CEF's Alloy embedding has no V8 handle in the browser process and no user
/// script API, so what WebKit answers with `evaluateJavaScript`,
/// `WKUserScript` and `WKScriptMessageHandler` is answered here with
/// `Runtime.evaluate`, `Page.addScriptToEvaluateOnNewDocument` and
/// `Runtime.addBinding`. The event and result shapes are WebKit's, not CDP's,
/// so an app (or the extension broker) cannot tell which engine is under it.
///
/// Every method call is correlated by message id and answered on the CEF UI
/// thread, which is the main thread here. Results arrive as raw JSON bytes
/// from the observer.
@MainActor
final class NDCefDevTools {
    /// Reply to one `execute_dev_tools_method` call: the parsed result object,
    /// or nil when CEF reported failure (the protocol error is in `error`).
    typealias Reply = (_ result: [String: Any]?, _ error: String?) -> Void

    private weak var view: NDCefWebView?
    private var nextMessageID: Int32 = 1
    private var pending: [Int32: Reply] = [:]

    /// One isolated world in one frame. The frame half is load-bearing: a
    /// document-start script with a worldName is injected into EVERY frame, so
    /// a page with an iframe reports the same world name twice, and a cache
    /// keyed by name alone lets the subframe's context win. Every world-scoped
    /// eval would then run in the iframe.
    private struct WorldKey: Hashable {
        let world: String
        let frame: String
    }

    /// Execution contexts learned from `Runtime.executionContextCreated`,
    /// cleared per navigation so a world-scoped eval lands in the world
    /// belonging to the page that is actually loaded.
    private var worldContexts: [WorldKey: Int] = [:]
    /// World-scoped work that arrived before its context existed.
    private var worldWaiters: [WorldKey: [(Int?) -> Void]] = [:]
    /// The frame a world-scoped eval means when the caller names none. Learned
    /// from `Page.getFrameTree` and kept current by `Page.frameNavigated`.
    private var mainFrameID = ""

    init(view: NDCefWebView) {
        self.view = view
    }

    /// The DevTools agent attaches on the first method call, and nothing else
    /// may go out until it has answered: a params dictionary handed to an
    /// unattached agent takes the whole process down. `Runtime` carries the
    /// bindings and the world contexts, `Page` the document-start scripts.
    private var isReady = false
    private var queued: [(method: String, params: [String: Any], reply: Reply?)] = []
    var whenReady: (() -> Void)?

    func start() {
        send("Runtime.enable", [:], nil)
        send("Page.enable", [:]) { [weak self] _, _ in
            guard let self else { return }
            self.isReady = true
            let waiting = self.queued
            self.queued.removeAll()
            for item in waiting { self.send(item.method, item.params, item.reply) }
            self.resolveMainFrame { _ in }
            self.whenReady?()
        }
    }

    // MARK: - Calls

    func call(_ method: String, _ params: [String: Any] = [:], reply: Reply? = nil) {
        guard isReady else {
            queued.append((method, params, reply))
            return
        }
        send(method, params, reply)
    }

    private func send(_ method: String, _ params: [String: Any], _ reply: Reply?) {
        guard let host = view?.browserHost() else {
            reply?(nil, "no browser")
            return
        }
        defer { nd_cef_ref_release(host) }
        let id = nextMessageID
        nextMessageID += 1
        if let reply { pending[id] = reply }

        var name = cef_string_t()
        ndCefSetString(method, &name)
        defer { nd_cef_string_clear(&name) }

        // The library takes the params reference: an object passed INTO a capi
        // function is owned by the callee from that point on, so releasing it
        // here would be the second release. Same rule at every pass-in site.
        let dictionary = params.isEmpty ? nil : Self.dictionary(from: params)

        if host.pointee.execute_dev_tools_method?(host, id, &name, dictionary) == 0 {
            pending.removeValue(forKey: id)
            reply?(nil, "execute_dev_tools_method(\(method)) was refused")
        }
    }

    /// Every CDP call this file makes takes a flat params object, so the
    /// conversion stops at scalars rather than growing a general JSON bridge
    /// nothing would use.
    private static func dictionary(from params: [String: Any]) -> UnsafeMutablePointer<cef_dictionary_value_t>? {
        guard let dictionary = nd_cef_dict_create() else { return nil }
        for (key, value) in params {
            var name = cef_string_t()
            ndCefSetString(key, &name)
            switch value {
            case let text as String:
                var slot = cef_string_t()
                ndCefSetString(text, &slot)
                _ = dictionary.pointee.set_string?(dictionary, &name, &slot)
                nd_cef_string_clear(&slot)
            case let flag as Bool:
                _ = dictionary.pointee.set_bool?(dictionary, &name, flag ? 1 : 0)
            case let number as Int:
                _ = dictionary.pointee.set_int?(dictionary, &name, Int32(number))
            case let number as Double:
                _ = dictionary.pointee.set_double?(dictionary, &name, number)
            default:
                ndCefWarn("devtools param \(key) has an unsupported type")
            }
            nd_cef_string_clear(&name)
        }
        return dictionary
    }

    // MARK: - Observer callbacks

    func handleMethodResult(id: Int32, success: Bool, json: [String: Any]?) {
        guard let reply = pending.removeValue(forKey: id) else { return }
        if success {
            reply(json, nil)
        } else {
            // CEF hands the protocol error through in the same payload.
            let error = (json?["error"] as? [String: Any])?["message"] as? String
            reply(nil, error ?? "devtools call failed")
        }
    }

    func handleEvent(method: String, params: [String: Any]) {
        switch method {
        case "Page.frameNavigated":
            guard let frame = params["frame"] as? [String: Any],
                  let id = frame["id"] as? String,
                  frame["parentId"] == nil else { return }
            mainFrameID = id
        case "Runtime.executionContextCreated":
            guard let context = params["context"] as? [String: Any],
                  let id = context["id"] as? Int,
                  let name = context["name"] as? String, !name.isEmpty else { return }
            let auxData = context["auxData"] as? [String: Any]
            // Isolated worlds only: the default context has an empty name, and
            // an eval with no contextId already lands there.
            guard auxData?["isDefault"] as? Bool != true else { return }
            guard let frame = auxData?["frameId"] as? String else { return }
            let key = WorldKey(world: name, frame: frame)
            worldContexts[key] = id
            if let waiters = worldWaiters.removeValue(forKey: key) {
                for waiter in waiters { waiter(id) }
            }
        case "Runtime.executionContextDestroyed":
            guard let id = params["executionContextId"] as? Int else { return }
            worldContexts = worldContexts.filter { $0.value != id }
        case "Runtime.executionContextsCleared":
            worldContexts.removeAll()
        case "Runtime.bindingCalled":
            guard let payload = params["payload"] as? String else { return }
            view?.handleBindingPayload(payload)
        default:
            break
        }
    }

    // MARK: - Worlds

    /// Resolves a world name to its execution context in one frame, waiting if
    /// the document-start script that creates the world has not run yet. An
    /// empty name is the page's own world, which needs no id. `frame` defaults
    /// to the main frame, which is what an app means by "this view".
    func withWorldContext(_ world: String, frame: String? = nil, _ body: @escaping (Int?) -> Void) {
        if world.isEmpty {
            body(nil)
            return
        }
        guard let target = frame ?? (mainFrameID.isEmpty ? nil : mainFrameID) else {
            // The frame tree has not been read yet; read it and come back.
            resolveMainFrame { [weak self] resolved in
                guard let self, let resolved else {
                    body(nil)
                    return
                }
                self.withWorldContext(world, frame: resolved, body)
            }
            return
        }
        let key = WorldKey(world: world, frame: target)
        if let id = worldContexts[key] {
            body(id)
            return
        }
        worldWaiters[key, default: []].append(body)
        // A world with no document-start script would never appear on its own.
        // Asking for it directly is what makes `executeJavaScript` into a world
        // the app never injected into behave like WebKit's, which creates the
        // world on demand.
        createIsolatedWorld(world, frame: target)
    }

    private var creatingWorlds: Set<WorldKey> = []

    private func resolveMainFrame(_ body: @escaping (String?) -> Void) {
        call("Page.getFrameTree") { [weak self] result, _ in
            let frame = (result?["frameTree"] as? [String: Any])?["frame"] as? [String: Any]
            guard let id = frame?["id"] as? String else {
                body(nil)
                return
            }
            self?.mainFrameID = id
            body(id)
        }
    }

    private func createIsolatedWorld(_ world: String, frame: String) {
        let key = WorldKey(world: world, frame: frame)
        guard !creatingWorlds.contains(key) else { return }
        creatingWorlds.insert(key)
        call("Page.createIsolatedWorld", [
            "frameId": frame,
            "worldName": world,
            "grantUniveralAccess": true,
        ]) { [weak self] created, _ in
            guard let self else { return }
            self.creatingWorlds.remove(key)
            guard let id = created?["executionContextId"] as? Int else {
                self.failWorldWaiters(key)
                return
            }
            self.worldContexts[key] = id
            if let waiters = self.worldWaiters.removeValue(forKey: key) {
                for waiter in waiters { waiter(id) }
            }
        }
    }

    private func failWorldWaiters(_ key: WorldKey) {
        guard let waiters = worldWaiters.removeValue(forKey: key) else { return }
        for waiter in waiters { waiter(nil) }
    }

    // MARK: - Evaluate

    /// One JavaScript round trip, answered in WebKit's shape: a string value
    /// (nil for undefined) or a message from the thrown exception. `world` is
    /// "" for the page's own world.
    func evaluate(_ code: String, world: String, _ completion: @escaping (String?, String?) -> Void) {
        withWorldContext(world) { [weak self] contextID in
            guard let self else {
                completion(nil, "view is gone")
                return
            }
            if !world.isEmpty && contextID == nil {
                completion(nil, "no execution context for world \(world)")
                return
            }
            var params: [String: Any] = [
                "expression": code,
                "returnByValue": true,
                "awaitPromise": true,
            ]
            if let contextID { params["contextId"] = contextID }
            self.call("Runtime.evaluate", params) { result, error in
                if let error {
                    completion(nil, error)
                    return
                }
                if let details = result?["exceptionDetails"] as? [String: Any] {
                    completion(nil, Self.exceptionMessage(details))
                    return
                }
                completion(Self.stringify(result?["result"] as? [String: Any]), nil)
            }
        }
    }

    private static func exceptionMessage(_ details: [String: Any]) -> String {
        if let exception = details["exception"] as? [String: Any] {
            if let description = exception["description"] as? String { return description }
            if let value = exception["value"] { return String(describing: value) }
        }
        return details["text"] as? String ?? "JavaScript exception"
    }

    /// CDP's remote-object shape collapsed to the one WebKit's
    /// `evaluateJavaScript` produces: a string for everything, nil for
    /// undefined, JSON for objects and arrays.
    static func stringify(_ remote: [String: Any]?) -> String? {
        guard let remote, let type = remote["type"] as? String, type != "undefined" else { return nil }
        guard let value = remote["value"], !(value is NSNull) else { return nil }
        if let text = value as? String { return text }
        if let flag = value as? Bool, type == "boolean" { return flag ? "true" : "false" }
        if let number = value as? NSNumber, type == "number" {
            let double = number.doubleValue
            if double == double.rounded(), abs(double) < 1e15 { return String(Int64(double)) }
            return String(double)
        }
        if JSONSerialization.isValidJSONObject(value),
           let data = try? JSONSerialization.data(withJSONObject: value),
           let text = String(data: data, encoding: .utf8) {
            return text
        }
        return String(describing: value)
    }
}
#endif
