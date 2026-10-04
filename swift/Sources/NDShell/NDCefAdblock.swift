#if canImport(CCef)
import CCef
import CNd
import Foundation

/// Built-in content blocking on the Chromium engine (src/adblock.zig holds the
/// engine; this is the AppKit host's half).
///
/// Network rules are answered on CEF's IO thread from the view's resource
/// request handler. Cosmetic rules and scriptlets reach every frame through
/// the helper's render process handler (swift/Sources/CCef/nd_cef.c): each new
/// main-world context fetches its own script from the `nd-adblock` scheme,
/// which the same request handler serves.
enum NDCefAdblock {
    static let scheme = "nd-adblock:"
    private static let tracing = ProcessInfo.processInfo.environment["ND_WEBVIEW_TRACE"] == "1"

    /// What `get_resource_request_handler` hands CEF for one request, or nil
    /// to let it load untouched. Runs on the IO thread.
    static func requestHandler(
        box: NDCefHandlerBox,
        browser: UnsafeMutablePointer<cef_browser_t>?,
        request: UnsafeMutablePointer<cef_request_t>?,
        url: String,
        initiator: String,
        disableDefaultHandling: UnsafeMutablePointer<Int32>?
    ) -> UnsafeMutablePointer<cef_resource_request_handler_t>? {
        guard let request else { return nil }
        if url.hasPrefix(scheme) {
            disableDefaultHandling?.pointee = 1
            var served = nd_adblock_served()
            nd_adblock_serve(url, mainFrameURL(browser), &served)
            defer { nd_adblock_served_free(&served) }
            let body = served.body.map { Data(bytes: $0, count: served.body_len) } ?? Data()
            let mime = served.mime.map { String(cString: $0) } ?? "application/javascript"
            return NDCefStaticResponse.requestHandler(body: body, mime: mime)
        }
        guard nd_adblock_active() else { return nil }
        let kind = Int32(request.pointee.get_resource_type?(request).rawValue ?? 0)
        if kind == Int32(RT_MAIN_FRAME.rawValue) {
            box.adblockCounter.reset(box)
            return nil
        }
        let top = mainFrameURL(browser)
        var decision = nd_adblock_decision()
        nd_adblock_decide(url, initiator, top, kind, &decision)
        defer { nd_adblock_decision_free(&decision) }
        if decision.action != Int32(ND_ADBLOCK_ALLOW.rawValue), tracing {
            FileHandle.standardError.write("ND_WV cef adblock \(decision.action) kind=\(kind) \(url)\n".data(using: .utf8)!)
        }
        switch decision.action {
        case Int32(ND_ADBLOCK_BLOCK.rawValue):
            box.adblockCounter.bump(box)
            return ndCefHandOut(box.adblockBlock)
        case Int32(ND_ADBLOCK_REDIRECT.rawValue):
            box.adblockCounter.bump(box)
            let body = decision.body.map { Data(bytes: $0, count: decision.body_len) } ?? Data()
            let mime = decision.mime.map { String(cString: $0) } ?? "text/plain"
            return NDCefStaticResponse.requestHandler(body: body, mime: mime)
        default:
            return nil
        }
    }

    private static func mainFrameURL(_ browser: UnsafeMutablePointer<cef_browser_t>?) -> String {
        guard let browser, let frame = browser.pointee.get_main_frame?(browser) else { return "" }
        defer { nd_cef_ref_release(frame) }
        guard let raw = frame.pointee.get_url?(frame) else { return "" }
        defer { nd_cef_string_free(raw) }
        return ndCefString(raw)
    }

    /// Wires the shared per-view handler that cancels a blocked request.
    /// Called from `build()`.
    static func wire(_ box: NDCefHandlerBox) {
        box.adblockBlock?.pointee.on_before_resource_load = { _, browser, frame, request, callback in
            nd_cef_ref_release(browser)
            nd_cef_ref_release(frame)
            nd_cef_ref_release(request)
            nd_cef_ref_release(callback)
            return RV_CANCEL
        }
    }
}

// The box is what CEF's IO-thread callbacks find behind a handler struct, and
// the only thing they touch on it off the main thread is the counter (locked)
// and the handler pointers (written once in `build()`).
extension NDCefHandlerBox: @unchecked Sendable {}

/// Blocked requests since the last main-frame navigation, counted on the IO
/// thread and reported as `contentBlocked` at most once per main-thread turn.
final class NDCefAdblockCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    private var reportQueued = false

    func bump(_ box: NDCefHandlerBox) {
        lock.lock()
        count += 1
        let queue = !reportQueued
        reportQueued = true
        lock.unlock()
        if queue { report(box) }
    }

    func reset(_ box: NDCefHandlerBox) {
        lock.lock()
        count = 0
        reportQueued = true
        lock.unlock()
        report(box)
    }

    private func report(_ box: NDCefHandlerBox) {
        ndCefOnMain { [self] in
            lock.lock()
            let value = count
            reportQueued = false
            lock.unlock()
            box.view?.emitData("contentBlocked", ["count": value])
        }
    }
}

/// A body served in place of a network response: a `redirect=` rule's
/// resource, or an `nd-adblock` scheme answer.
final class NDCefStaticResponse: @unchecked Sendable {
    private let body: Data
    private let mime: String
    private let lock = NSLock()
    private var offset = 0

    private init(body: Data, mime: String) {
        self.body = body
        self.mime = mime
    }

    static func requestHandler(body: Data, mime: String) -> UnsafeMutablePointer<cef_resource_request_handler_t>? {
        let owner = Unmanaged.passRetained(Box(body: body, mime: mime)).toOpaque()
        guard let raw = nd_cef_ref_alloc(MemoryLayout<cef_resource_request_handler_t>.size, owner, { owner in
            guard let owner else { return }
            Unmanaged<Box>.fromOpaque(owner).release()
        }) else {
            Unmanaged<Box>.fromOpaque(owner).release()
            return nil
        }
        let handler = raw.assumingMemoryBound(to: cef_resource_request_handler_t.self)
        handler.pointee.get_resource_handler = { selfPointer, browser, frame, request in
            nd_cef_ref_release(browser)
            nd_cef_ref_release(frame)
            nd_cef_ref_release(request)
            guard let owner = nd_cef_ref_owner(selfPointer) else { return nil }
            let box = Unmanaged<Box>.fromOpaque(owner).takeUnretainedValue()
            return NDCefStaticResponse(body: box.body, mime: box.mime).resourceHandler()
        }
        return handler
    }

    private final class Box {
        let body: Data
        let mime: String
        init(body: Data, mime: String) {
            self.body = body
            self.mime = mime
        }
    }

    private func resourceHandler() -> UnsafeMutablePointer<cef_resource_handler_t>? {
        let owner = Unmanaged.passRetained(self).toOpaque()
        guard let raw = nd_cef_ref_alloc(MemoryLayout<cef_resource_handler_t>.size, owner, { owner in
            guard let owner else { return }
            Unmanaged<NDCefStaticResponse>.fromOpaque(owner).release()
        }) else {
            Unmanaged<NDCefStaticResponse>.fromOpaque(owner).release()
            return nil
        }
        let handler = raw.assumingMemoryBound(to: cef_resource_handler_t.self)
        handler.pointee.open = { _, request, handleRequest, callback in
            nd_cef_ref_release(request)
            nd_cef_ref_release(callback)
            handleRequest?.pointee = 1
            return 1
        }
        handler.pointee.get_response_headers = { selfPointer, response, length, _ in
            guard let response, let owner = nd_cef_ref_owner(selfPointer) else { return }
            let me = Unmanaged<NDCefStaticResponse>.fromOpaque(owner).takeUnretainedValue()
            var type = cef_string_t()
            ndCefSetString(me.mime, &type)
            response.pointee.set_mime_type?(response, &type)
            nd_cef_string_clear(&type)
            response.pointee.set_status?(response, 200)
            // A redirected script or XHR is usually cross-origin to the page.
            var name = cef_string_t()
            var value = cef_string_t()
            ndCefSetString("Access-Control-Allow-Origin", &name)
            ndCefSetString("*", &value)
            response.pointee.set_header_by_name?(response, &name, &value, 1)
            nd_cef_string_clear(&name)
            nd_cef_string_clear(&value)
            length?.pointee = Int64(me.body.count)
        }
        handler.pointee.read = { selfPointer, dataOut, bytesToRead, bytesRead, callback in
            nd_cef_ref_release(callback)
            guard let owner = nd_cef_ref_owner(selfPointer) else { return 0 }
            let me = Unmanaged<NDCefStaticResponse>.fromOpaque(owner).takeUnretainedValue()
            return me.read(into: dataOut, capacity: Int(bytesToRead), produced: bytesRead)
        }
        handler.pointee.cancel = { _ in }
        return handler
    }

    private func read(into out: UnsafeMutableRawPointer?, capacity: Int, produced: UnsafeMutablePointer<Int32>?) -> Int32 {
        lock.lock()
        defer { lock.unlock() }
        guard let out, capacity > 0, offset < body.count else {
            produced?.pointee = 0
            return 0
        }
        let count = min(capacity, body.count - offset)
        body.withUnsafeBytes { raw in
            guard let base = raw.baseAddress else { return }
            out.copyMemory(from: base.advanced(by: offset), byteCount: count)
        }
        offset += count
        produced?.pointee = Int32(count)
        return 1
    }
}

#endif
