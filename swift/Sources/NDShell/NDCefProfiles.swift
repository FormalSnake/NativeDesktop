#if canImport(CCef)
import CCef
import Foundation

/// `profile` create-only prop, resolved to a CEF request context.
///
/// One `root_cache_path` is set once at initialize (NDCefEngine.swift); each
/// named profile hangs a `cache_path` under it, so the same profile always
/// resolves to the same jar across launches. An empty name is the shared
/// default context, and anything starting with "private" gets no cache path at
/// all, which is CEF's in-memory context. Peer of NDWebViewProfiles.
enum NDCefProfiles {
    nonisolated(unsafe) private static var contexts: [String: UnsafeMutablePointer<cef_request_context_t>] = [:]

    /// The context a view should be created in, or nil for CEF's global one.
    /// The returned reference stays owned by this table.
    static func context(for profile: String) -> UnsafeMutablePointer<cef_request_context_t>? {
        guard !profile.isEmpty else { return nil }
        if let existing = contexts[profile] { return existing }
        var settings = cef_request_context_settings_t()
        settings.size = MemoryLayout<cef_request_context_settings_t>.size
        var cachePath = cef_string_t()
        defer { nd_cef_string_clear(&cachePath) }
        if !profile.hasPrefix("private") {
            ndCefSetString(NDCefRuntime.profileCachePath(profile), &cachePath)
            settings.cache_path = cachePath
        }
        guard let context = nd_cef_request_context_create(&settings, nil) else {
            ndCefWarn("request context for profile \(profile) could not be created")
            return nil
        }
        contexts[profile] = context
        // Schemes registered before this context existed still have to reach
        // it: the factory is per-context on this engine.
        NDCefSchemes.install(into: context)
        return context
    }

    /// Every live context, for the process-wide operations (scheme
    /// registration) that must reach all of them.
    static var all: [UnsafeMutablePointer<cef_request_context_t>] {
        Array(contexts.values)
    }
}

// MARK: - Cookies

extension NDCefWebView {
    /// The cookie manager of the view's own request context, which is what
    /// makes `profileIsolation` real: two profiles are two managers.
    private func withCookieManager(_ body: (UnsafeMutablePointer<cef_cookie_manager_t>) -> Void) {
        guard let browserHost = browserHost() else { return }
        defer { nd_cef_ref_release(browserHost) }
        guard let context = browserHost.pointee.get_request_context?(browserHost) else { return }
        defer { nd_cef_ref_release(context) }
        guard let manager = context.pointee.get_cookie_manager?(context, nil) else { return }
        defer { nd_cef_ref_release(manager) }
        body(manager)
    }

    func ndGetCookies(_ obj: [String: Any]) {
        guard let id = obj["id"] as? String else {
            ndCefWarn("getCookies: missing id")
            return
        }
        let url = obj["url"] as? String ?? pendingURLForCookies
        let collector = NDCefCookieCollector(view: self, id: id)
        withCookieManager { manager in
            guard let visitor = collector.makeVisitor() else { return }
            var target = cef_string_t()
            ndCefSetString(url, &target)
            defer { nd_cef_string_clear(&target) }
            // The library takes the visitor reference and drops it when the
            // walk ends, which is what fires the result event. Nothing to
            // release here, on either outcome.
            _ = manager.pointee.visit_url_cookies?(manager, &target, 1, visitor)
        }
    }

    private var pendingURLForCookies: String {
        ndPageState.url ?? ""
    }

    func ndSetCookie(_ obj: [String: Any]) {
        let fields = (obj["cookie"] as? [String: Any]) ?? obj
        guard let name = fields["name"] as? String, let domain = fields["domain"] as? String else {
            ndCefWarn("setCookie: malformed cookie (name and domain are required)")
            return
        }
        let path = fields["path"] as? String ?? "/"
        let secure = (fields["secure"] as? NSNumber)?.boolValue ?? false
        let host = domain.hasPrefix(".") ? String(domain.dropFirst()) : domain
        let url = fields["url"] as? String ?? "\(secure ? "https" : "http")://\(host)\(path)"

        var cookie = cef_cookie_t()
        cookie.size = MemoryLayout<cef_cookie_t>.size
        var slots: [UnsafeMutablePointer<cef_string_t>] = []
        func fill(_ keyPath: WritableKeyPath<cef_cookie_t, cef_string_t>, _ value: String) {
            let slot = UnsafeMutablePointer<cef_string_t>.allocate(capacity: 1)
            slot.initialize(to: cef_string_t())
            ndCefSetString(value, slot)
            cookie[keyPath: keyPath] = slot.pointee
            slots.append(slot)
        }
        fill(\.name, name)
        fill(\.value, fields["value"] as? String ?? "")
        fill(\.domain, domain)
        fill(\.path, path)
        cookie.secure = secure ? 1 : 0
        cookie.httponly = ((fields["httpOnly"] as? NSNumber)?.boolValue ?? false) ? 1 : 0
        if let expires = (fields["expires"] as? NSNumber)?.doubleValue {
            cookie.has_expires = 1
            cookie.expires = cef_basetime_t(val: Int64((expires + 11_644_473_600) * 1_000_000))
        }
        switch (fields["sameSite"] as? String ?? "").lowercased() {
        case "lax": cookie.same_site = CEF_COOKIE_SAME_SITE_LAX_MODE
        case "strict": cookie.same_site = CEF_COOKIE_SAME_SITE_STRICT_MODE
        case "none": cookie.same_site = CEF_COOKIE_SAME_SITE_NO_RESTRICTION
        default: break
        }
        defer {
            for slot in slots {
                nd_cef_string_clear(slot)
                slot.deinitialize(count: 1)
                slot.deallocate()
            }
        }

        withCookieManager { manager in
            var target = cef_string_t()
            ndCefSetString(url, &target)
            defer { nd_cef_string_clear(&target) }
            if manager.pointee.set_cookie?(manager, &target, &cookie, nil) == 0 {
                ndCefWarn("setCookie: the engine refused \(name) for \(url)")
            }
        }
        emitData("cookiesChanged", [:])
    }

    func ndDeleteCookie(_ obj: [String: Any]) {
        let fields = (obj["cookie"] as? [String: Any]) ?? obj
        guard let name = fields["name"] as? String else {
            ndCefWarn("deleteCookie: missing name")
            return
        }
        let domain = fields["domain"] as? String ?? ""
        let path = fields["path"] as? String ?? "/"
        let host = domain.hasPrefix(".") ? String(domain.dropFirst()) : domain
        let url = fields["url"] as? String ?? (host.isEmpty ? "" : "http://\(host)\(path)")
        withCookieManager { manager in
            var target = cef_string_t()
            ndCefSetString(url, &target)
            var cookieName = cef_string_t()
            ndCefSetString(name, &cookieName)
            defer {
                nd_cef_string_clear(&target)
                nd_cef_string_clear(&cookieName)
            }
            // A null url deletes the cookie everywhere it exists, which is
            // what an app asking by name alone means.
            if url.isEmpty {
                _ = manager.pointee.delete_cookies?(manager, nil, &cookieName, nil)
            } else {
                _ = manager.pointee.delete_cookies?(manager, &target, &cookieName, nil)
            }
        }
        emitData("cookiesChanged", [:])
    }
}

/// Collects one `visit_url_cookies` walk and emits `cookiesResult` when CEF
/// releases the visitor, which is the only "done" signal the API gives.
final class NDCefCookieCollector: @unchecked Sendable {
    private let view: NDCefWebViewRef
    private let id: String
    private let lock = NSLock()
    private var cookies: [[String: Any]] = []
    private var finished = false

    @MainActor init(view: NDCefWebView, id: String) {
        self.view = NDCefWebViewRef(view)
        self.id = id
    }

    func makeVisitor() -> UnsafeMutablePointer<cef_cookie_visitor_t>? {
        let owner = Unmanaged.passRetained(self).toOpaque()
        guard let raw = nd_cef_ref_alloc(MemoryLayout<cef_cookie_visitor_t>.size, owner, { owner in
            // CEF dropping the visitor IS the end of the walk. The collector's
            // last reference travels to the main thread with the result, so it
            // cannot be freed before the event is emitted.
            guard let owner else { return }
            let token = UInt(bitPattern: owner)
            ndCefOnMain {
                guard let raw = UnsafeMutableRawPointer(bitPattern: token) else { return }
                let unmanaged = Unmanaged<NDCefCookieCollector>.fromOpaque(raw)
                unmanaged.takeUnretainedValue().finish()
                unmanaged.release()
            }
        }) else {
            Unmanaged<NDCefCookieCollector>.fromOpaque(owner).release()
            return nil
        }
        let visitor = raw.assumingMemoryBound(to: cef_cookie_visitor_t.self)
        visitor.pointee.visit = { selfPointer, cookie, _, _, deleteCookie in
            deleteCookie?.pointee = 0
            guard let selfPointer, let owner = nd_cef_ref_owner(selfPointer), let cookie else { return 1 }
            let collector = Unmanaged<NDCefCookieCollector>.fromOpaque(owner).takeUnretainedValue()
            collector.record(cookie.pointee)
            return 1
        }
        return visitor
    }

    /// The visit callback can arrive on a CEF thread of its own, so the
    /// accumulator is locked rather than actor-isolated.
    private func record(_ cookie: cef_cookie_t) {
        var copy = cookie
        var fields: [String: Any] = [
            "name": ndCefString(&copy.name),
            "value": ndCefString(&copy.value),
            "domain": ndCefString(&copy.domain),
            "path": ndCefString(&copy.path),
            "secure": copy.secure != 0,
            "httpOnly": copy.httponly != 0,
            "sameSite": Self.sameSiteName(copy.same_site),
        ]
        if copy.has_expires != 0 {
            // cef_basetime_t counts microseconds from the Windows epoch.
            fields["expires"] = Int(Double(copy.expires.val) / 1_000_000 - 11_644_473_600)
        } else {
            fields["expires"] = NSNull()
        }
        lock.lock()
        cookies.append(fields)
        lock.unlock()
    }

    private static func sameSiteName(_ value: cef_cookie_same_site_t) -> String {
        switch value {
        case CEF_COOKIE_SAME_SITE_LAX_MODE: return "Lax"
        case CEF_COOKIE_SAME_SITE_STRICT_MODE: return "Strict"
        default: return "None"
        }
    }

    @MainActor func finish() {
        lock.lock()
        let alreadyFinished = finished
        finished = true
        let collected = cookies
        lock.unlock()
        guard !alreadyFinished else { return }
        view.view?.emitData("cookiesResult", ["id": id, "ok": true, "cookies": collected])
    }
}

// MARK: - Custom URI schemes

/// `webviewEngine.registerScheme` on Chromium: one scheme handler factory per
/// request context, plus the pending-request table `respondScheme` answers.
///
/// Registration is late here in a way WebKit's is not. A CEF scheme can only
/// be made "standard" during `on_register_custom_schemes`, which runs inside
/// cef_initialize before any app code has asked for one, so a scheme
/// registered at runtime is a non-standard scheme: the factory sees every
/// request and the page loads, but the origin is opaque.
enum NDCefSchemes {
    nonisolated(unsafe) private static var schemes: [String] = []
    nonisolated(unsafe) private static var pending: [String: NDCefSchemeRequest] = [:]
    nonisolated(unsafe) private static var sequence = 0
    nonisolated(unsafe) private static var factory: UnsafeMutablePointer<cef_scheme_handler_factory_t>?

    enum RegisterError: Error {
        case alreadyRegistered
        case invalid
    }

    static func register(_ scheme: String) throws {
        let allowed = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyz0123456789+-.")
        guard !scheme.isEmpty, scheme.unicodeScalars.allSatisfy({ allowed.contains($0) }) else {
            throw RegisterError.invalid
        }
        guard !schemes.contains(scheme) else { throw RegisterError.alreadyRegistered }
        schemes.append(scheme)
        registerFactory(scheme, context: nil)
        for context in NDCefProfiles.all { registerFactory(scheme, context: context) }
    }

    /// A context created after `register` still has to learn about every
    /// scheme, since the factory is per-context.
    static func install(into context: UnsafeMutablePointer<cef_request_context_t>) {
        for scheme in schemes { registerFactory(scheme, context: context) }
    }

    private static func registerFactory(_ scheme: String, context: UnsafeMutablePointer<cef_request_context_t>?) {
        guard let factory = sharedFactory() else { return }
        var name = cef_string_t()
        ndCefSetString(scheme, &name)
        defer { nd_cef_string_clear(&name) }
        nd_cef_ref_add(factory)
        if let context {
            _ = context.pointee.register_scheme_handler_factory?(context, &name, nil, factory)
        } else {
            _ = nd_cef_register_scheme_handler_factory(&name, nil, factory)
        }
    }

    private static func sharedFactory() -> UnsafeMutablePointer<cef_scheme_handler_factory_t>? {
        if let factory { return factory }
        guard let raw = nd_cef_ref_alloc(MemoryLayout<cef_scheme_handler_factory_t>.size, nil, nil) else { return nil }
        let created = raw.assumingMemoryBound(to: cef_scheme_handler_factory_t.self)
        created.pointee.create = { _, browser, frame, _, request in
            var url = ""
            if let request, let raw = request.pointee.get_url?(request) {
                url = ndCefString(raw)
                nd_cef_string_free(raw)
            }
            // The browser id is what routes the request back to the view that
            // asked for it; the scheme itself is process-wide.
            let browserID = browser.flatMap { $0.pointee.get_identifier?($0) } ?? 0
            nd_cef_ref_release(browser)
            nd_cef_ref_release(frame)
            nd_cef_ref_release(request)
            return NDCefSchemeRequest.makeHandler(url: url, browserID: browserID)
        }
        factory = created
        return created
    }

    /// Called from the IO thread when a resource handler opens: parks the
    /// request and asks the app on the UI thread.
    @MainActor static func begin(_ request: NDCefSchemeRequest) {
        sequence += 1
        let id = "cef\(sequence)"
        pending[id] = request
        request.id = id
        let url = request.url
        let scheme = URL(string: url)?.scheme ?? ""
        NDCefSchemeRouter.emit(browser: request.browserID, ["id": id, "url": url, "scheme": scheme])
    }

    static func respond(_ obj: [String: Any]) {
        guard let id = obj["id"] as? String, let request = pending.removeValue(forKey: id) else { return }
        request.answer(obj)
    }

    static func cancel(_ request: NDCefSchemeRequest) {
        guard let id = request.id else { return }
        pending.removeValue(forKey: id)
    }
}

/// Where a `schemeRequest` event goes. The scheme is process-wide, but the
/// event rides a widget, so the request is routed back to the view whose
/// browser issued it.
@MainActor
enum NDCefSchemeRouter {
    private static var views: [Int32: NDCefWebViewRef] = [:]

    static func register(_ view: NDCefWebView, browser: UnsafeMutablePointer<cef_browser_t>) {
        guard let identifier = browser.pointee.get_identifier?(browser) else { return }
        views[identifier] = NDCefWebViewRef(view)
        views = views.filter { $0.value.view != nil }
    }

    static func emit(browser: Int32, _ fields: [String: Any]) {
        guard let view = views[browser]?.view ?? views.values.compactMap(\.view).first else {
            ndCefWarn("schemeRequest with no live chromium view to report it")
            return
        }
        view.emitData("schemeRequest", fields)
    }
}

/// Weak box, so a closed view drops out of the routing table on its own.
@MainActor final class NDCefWebViewRef {
    weak var view: NDCefWebView?
    init(_ view: NDCefWebView) { self.view = view }
}
#endif
