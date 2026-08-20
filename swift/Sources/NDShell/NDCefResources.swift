#if canImport(CCef)
import AppKit
import CCef
import Foundation

/// Runs `body` on the main thread, inline when already there. CEF's IO thread
/// is a real second thread, unlike its UI thread, so the scheme and cookie
/// paths need this and the handler callbacks do not.
func ndCefOnMain(_ body: @escaping @MainActor @Sendable () -> Void) {
    if Thread.isMainThread {
        MainActor.assumeIsolated { body() }
    } else {
        DispatchQueue.main.async { MainActor.assumeIsolated { body() } }
    }
}

/// One in-flight custom-scheme request: a `cef_resource_handler_t` that stalls
/// on the IO thread until the app answers `respondScheme` on the UI thread.
///
/// The two threads meet on this object, so every field it shares is behind the
/// lock. CEF calls open/get_response_headers/read/cancel on the IO thread;
/// `answer` arrives on the UI thread.
final class NDCefSchemeRequest: @unchecked Sendable {
    let url: String
    let browserID: Int32
    var id: String?

    private let lock = NSLock()
    private var body = Data()
    private var mime = "application/octet-stream"
    private var status: Int32 = 200
    private var headers: [String: String] = [:]
    private var failure: String?
    private var offset = 0
    private var continuation: UnsafeMutablePointer<cef_callback_t>?
    private var answered = false

    init(url: String, browserID: Int32) {
        self.url = url
        self.browserID = browserID
    }

    /// A resource handler bound to a fresh request. The returned struct carries
    /// the reference CEF is owed.
    static func makeHandler(url: String, browserID: Int32) -> UnsafeMutablePointer<cef_resource_handler_t>? {
        let request = NDCefSchemeRequest(url: url, browserID: browserID)
        let owner = Unmanaged.passRetained(request).toOpaque()
        guard let raw = nd_cef_ref_alloc(MemoryLayout<cef_resource_handler_t>.size, owner, { owner in
            guard let owner else { return }
            Unmanaged<NDCefSchemeRequest>.fromOpaque(owner).release()
        }) else {
            Unmanaged<NDCefSchemeRequest>.fromOpaque(owner).release()
            return nil
        }
        let handler = raw.assumingMemoryBound(to: cef_resource_handler_t.self)
        handler.pointee.open = { selfPointer, request, handleRequest, callback in
            nd_cef_ref_release(request)
            guard let owner = nd_cef_ref_owner(selfPointer) else { return 0 }
            let pending = Unmanaged<NDCefSchemeRequest>.fromOpaque(owner).takeUnretainedValue()
            // 0 means "answer later on any thread", which is what lets the app
            // take as long as it needs.
            handleRequest?.pointee = 0
            pending.park(callback)
            return 1
        }
        handler.pointee.get_response_headers = { selfPointer, response, responseLength, _ in
            guard let owner = nd_cef_ref_owner(selfPointer) else { return }
            Unmanaged<NDCefSchemeRequest>.fromOpaque(owner).takeUnretainedValue()
                .fillResponse(response, responseLength)
        }
        handler.pointee.read = { selfPointer, dataOut, bytesToRead, bytesRead, callback in
            nd_cef_ref_release(callback)
            guard let owner = nd_cef_ref_owner(selfPointer) else { return 0 }
            return Unmanaged<NDCefSchemeRequest>.fromOpaque(owner).takeUnretainedValue()
                .read(into: dataOut, capacity: Int(bytesToRead), produced: bytesRead)
        }
        handler.pointee.cancel = { selfPointer in
            guard let owner = nd_cef_ref_owner(selfPointer) else { return }
            let pending = Unmanaged<NDCefSchemeRequest>.fromOpaque(owner).takeUnretainedValue()
            ndCefOnMain { NDCefSchemes.cancel(pending) }
        }
        return handler
    }

    private func park(_ callback: UnsafeMutablePointer<cef_callback_t>?) {
        lock.lock()
        continuation = callback
        let alreadyAnswered = answered
        lock.unlock()
        if alreadyAnswered {
            // The app answered before CEF opened the handler; nothing to wait
            // for, so release the parked reference straight away.
            resume()
            return
        }
        ndCefOnMain { NDCefSchemes.begin(self) }
    }

    /// `respondScheme`, on the UI thread.
    func answer(_ obj: [String: Any]) {
        lock.lock()
        if let message = obj["error"] as? String {
            failure = message
        } else if let encoded = obj["base64"] as? String, let data = Data(base64Encoded: encoded) {
            body = data
            mime = obj["mime"] as? String ?? "application/octet-stream"
            status = (obj["status"] as? NSNumber)?.int32Value ?? 200
            for (name, value) in obj["headers"] as? [String: Any] ?? [:] {
                guard let text = value as? String else { continue }
                headers[name] = text
            }
        } else {
            failure = "respondScheme: malformed base64 body"
        }
        answered = true
        lock.unlock()
        resume()
    }

    private func resume() {
        lock.lock()
        let callback = continuation
        continuation = nil
        lock.unlock()
        guard let callback else { return }
        callback.pointee.cont?(callback)
        nd_cef_ref_release(callback)
    }

    private func fillResponse(_ response: UnsafeMutablePointer<cef_response_t>?, _ length: UnsafeMutablePointer<Int64>?) {
        guard let response else { return }
        lock.lock()
        let failed = failure != nil
        let contentType = mime
        let code = status
        let extra = headers
        let count = body.count
        lock.unlock()

        if failed {
            _ = response.pointee.set_status?(response, 500)
            length?.pointee = 0
            return
        }
        var typeSlot = cef_string_t()
        ndCefSetString(contentType, &typeSlot)
        response.pointee.set_mime_type?(response, &typeSlot)
        nd_cef_string_clear(&typeSlot)
        response.pointee.set_status?(response, code)
        for (name, value) in extra {
            var nameSlot = cef_string_t()
            var valueSlot = cef_string_t()
            ndCefSetString(name, &nameSlot)
            ndCefSetString(value, &valueSlot)
            response.pointee.set_header_by_name?(response, &nameSlot, &valueSlot, 1)
            nd_cef_string_clear(&nameSlot)
            nd_cef_string_clear(&valueSlot)
        }
        length?.pointee = Int64(count)
    }

    private func read(into out: UnsafeMutableRawPointer?, capacity: Int, produced: UnsafeMutablePointer<Int32>?) -> Int32 {
        guard let out, capacity > 0 else { return 0 }
        lock.lock()
        defer { lock.unlock() }
        guard failure == nil, offset < body.count else {
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

// MARK: - Dialogs

/// JavaScript dialogs and file choosers, routed into the same host-native
/// sheets the WebKit surface uses. Nothing here may let Chromium present its
/// own window: that is the no-stray-window invariant, and a JS dialog is the
/// easiest place to break it.
enum NDCefDialogs {
    @MainActor static func run(
        view: NDCefWebView?,
        type: cef_jsdialog_type_t,
        message: String,
        initial: String,
        callback token: UInt
    ) {
        guard let callback = UnsafeMutableRawPointer(bitPattern: token)?
            .assumingMemoryBound(to: cef_jsdialog_callback_t.self) else { return }
        if let scripted = NDScriptedDialogAnswer.next() {
            answer(callback, accepted: scripted.accepted,
                   text: type == JSDIALOGTYPE_PROMPT ? scripted.text : "")
            return
        }
        // A view with no window has nothing to sheet onto, so it answers
        // straight away rather than parking the page's JS thread for good.
        guard let window = view?.window else {
            answer(callback, accepted: false, text: "")
            return
        }
        let alert = NSAlert()
        alert.messageText = title(for: type)
        alert.informativeText = message
        alert.addButton(withTitle: "OK")
        var field: NSTextField?
        if type != JSDIALOGTYPE_ALERT { alert.addButton(withTitle: "Cancel") }
        if type == JSDIALOGTYPE_PROMPT {
            let input = NSTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))
            input.stringValue = initial
            alert.accessoryView = input
            field = input
        }
        alert.beginSheetModal(for: window) { response in
            let accepted = response == .alertFirstButtonReturn
            answer(callback, accepted: accepted, text: accepted ? (field?.stringValue ?? "") : "")
        }
    }

    private static func answer(_ callback: UnsafeMutablePointer<cef_jsdialog_callback_t>, accepted: Bool, text: String) {
        var input = cef_string_t()
        ndCefSetString(text, &input)
        callback.pointee.cont?(callback, accepted ? 1 : 0, &input)
        nd_cef_string_clear(&input)
        nd_cef_ref_release(callback)
    }

    private static func title(for type: cef_jsdialog_type_t) -> String {
        switch type {
        case JSDIALOGTYPE_ALERT: return "The page says"
        case JSDIALOGTYPE_PROMPT: return "The page is asking for input"
        default: return "Confirm"
        }
    }

    @MainActor static func runFilePanel(
        view: NDCefWebView?,
        mode: cef_file_dialog_mode_t,
        title: String,
        initialPath: String,
        extensions: [String],
        callback token: UInt
    ) {
        guard let callback = UnsafeMutableRawPointer(bitPattern: token)?
            .assumingMemoryBound(to: cef_file_dialog_callback_t.self) else { return }
        let finish: ([String]) -> Void = { paths in
            if paths.isEmpty {
                callback.pointee.cancel?(callback)
            } else {
                let list = nd_cef_string_list_alloc()
                for path in paths {
                    var slot = cef_string_t()
                    ndCefSetString(path, &slot)
                    nd_cef_string_list_append(list, &slot)
                    nd_cef_string_clear(&slot)
                }
                callback.pointee.cont?(callback, list)
                nd_cef_string_list_free(list)
            }
            nd_cef_ref_release(callback)
        }
        guard let window = view?.window else {
            finish([])
            return
        }
        let types = extensions.map { $0.hasPrefix(".") ? String($0.dropFirst()) : $0 }.filter { !$0.isEmpty }
        if mode == FILE_DIALOG_SAVE {
            let panel = NSSavePanel()
            if !title.isEmpty { panel.title = title }
            if !initialPath.isEmpty { panel.nameFieldStringValue = (initialPath as NSString).lastPathComponent }
            panel.allowedFileTypes = types.isEmpty ? nil : types
            panel.beginSheetModal(for: window) { response in
                finish(response == .OK ? [panel.url?.path].compactMap { $0 } : [])
            }
            return
        }
        let panel = NSOpenPanel()
        if !title.isEmpty { panel.title = title }
        panel.canChooseFiles = mode != FILE_DIALOG_OPEN_FOLDER
        panel.canChooseDirectories = mode == FILE_DIALOG_OPEN_FOLDER
        panel.allowsMultipleSelection = mode == FILE_DIALOG_OPEN_MULTIPLE
        panel.allowedFileTypes = types.isEmpty ? nil : types
        panel.beginSheetModal(for: window) { response in
            finish(response == .OK ? panel.urls.map(\.path) : [])
        }
    }
}

// MARK: - Capture

/// Chromium's content lives in a remote CALayer, which AppKit's own render
/// paths (the automation snapshot ladder) cannot draw: an offscreen capture of
/// a CEF view comes back as the page's background colour and nothing else.
/// `Page.captureScreenshot` asks the renderer for the pixels instead, and the
/// newest answer is cached so the synchronous snapshot RPC has something to
/// composite.
enum NDCefCapture {
    /// Asks for a fresh frame. The result lands on `view.cachedFrame`.
    @MainActor static func refresh(_ view: NDCefWebView, completion: (() -> Void)? = nil) {
        guard view.hasBrowser else {
            completion?()
            return
        }
        view.devTools.call("Page.captureScreenshot", ["format": "png"]) { result, _ in
            if let encoded = result?["data"] as? String,
               let data = Data(base64Encoded: encoded),
               let image = NSImage(data: data) {
                view.cachedFrame = image
            }
            completion?()
        }
    }

    /// Refreshes every chromium view in `window` and waits, bounded, for the
    /// answers. CEF's pump is CFRunLoop-based here, so spinning the run loop is
    /// what lets the protocol replies land while the caller blocks.
    @MainActor static func refreshAll(in window: NSWindow, timeout: TimeInterval = 1.5) {
        var views: [NDCefWebView] = []
        collect(window.contentView, into: &views)
        guard !views.isEmpty else { return }
        var outstanding = views.count
        for view in views {
            refresh(view) { outstanding -= 1 }
        }
        let deadline = Date().addingTimeInterval(timeout)
        while outstanding > 0, Date() < deadline {
            RunLoop.current.run(mode: .default, before: Date().addingTimeInterval(0.01))
        }
    }

    /// Draws each chromium view's newest frame over the bitmap the ladder
    /// rendered without them. Same convention as ndCompositeSidebarPanes: the
    /// rep is in pixels, `content` is flipped, so point rects are scaled and
    /// their y measured from the bottom.
    @MainActor static func composite(_ rep: NSBitmapImageRep, _ content: NSView) -> NSBitmapImageRep {
        var views: [NDCefWebView] = []
        collect(content, into: &views)
        let drawable = views.filter {
            !$0.isHiddenOrHasHiddenAncestor && $0.cachedFrame != nil && $0.window === content.window
        }
        guard !drawable.isEmpty, let baseCG = rep.cgImage,
              content.bounds.width > 0, content.bounds.height > 0 else { return rep }
        let width = rep.pixelsWide
        let height = rep.pixelsHigh
        guard let context = CGContext(
            data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else { return rep }
        context.draw(baseCG, in: CGRect(x: 0, y: 0, width: width, height: height))
        let sx = CGFloat(width) / content.bounds.width
        let sy = CGFloat(height) / content.bounds.height
        for view in drawable {
            guard let frame = view.cachedFrame,
                  let frameCG = frame.cgImage(forProposedRect: nil, context: nil, hints: nil) else { continue }
            let r = view.convert(view.bounds, to: content)
            context.draw(frameCG, in: CGRect(
                x: r.origin.x * sx,
                y: (content.bounds.height - r.maxY) * sy,
                width: r.width * sx,
                height: r.height * sy
            ))
        }
        guard let merged = context.makeImage() else { return rep }
        return NSBitmapImageRep(cgImage: merged)
    }

    private static func collect(_ view: NSView?, into found: inout [NDCefWebView]) {
        guard let view else { return }
        if let cef = view as? NDCefWebView {
            found.append(cef)
            return
        }
        for child in view.subviews { collect(child, into: &found) }
    }
}
#endif
