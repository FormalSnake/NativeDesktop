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
/// `URL.origin` form: scheme, host and a non-default port, with no trailing
/// slash. CEF serializes an origin as a GURL, which always ends in one, and an
/// app comparing it against `location.origin` would never match.
func ndCefOriginForm(_ raw: String) -> String {
    guard let sep = raw.range(of: "://") else {
        return String(raw.reversed().drop(while: { $0 == "/" }).reversed())
    }
    let scheme = String(raw[raw.startIndex..<sep.lowerBound])
    let rest = raw[sep.upperBound...]
    var authority = String(rest.prefix(while: { $0 != "/" }))
    let defaultPort: String? = scheme == "http" ? ":80" : (scheme == "https" || scheme == "wss" ? ":443" : nil)
    if let defaultPort, authority.hasSuffix(defaultPort) {
        authority = String(authority.dropLast(defaultPort.count))
    }
    return "\(scheme)://\(authority)"
}

/// The URL of a browser's main frame, read while the browser reference is still
/// held by the callback that was handed it.
func ndCefMainFrameURL(_ browser: UnsafeMutablePointer<cef_browser_t>?) -> String {
    guard let browser, let frame = browser.pointee.get_main_frame?(browser) else { return "" }
    defer { nd_cef_ref_release(frame) }
    guard let raw = frame.pointee.get_url?(frame) else { return "" }
    defer { nd_cef_string_free(raw) }
    return ndCefString(raw)
}

/// One owned reference to the browser's own request context, which is where its
/// content settings live whether it was created on a named profile or the
/// global one.
func ndCefBrowserContext(_ browser: UnsafeMutablePointer<cef_browser_t>?) -> UnsafeMutablePointer<cef_request_context_t>? {
    guard let browser, let host = browser.pointee.get_host?(browser) else { return nil }
    defer { nd_cef_ref_release(host) }
    return host.pointee.get_request_context?(host)
}

/// Permission requests, parked until the app answers `respondPermission`.
/// Chromium would otherwise draw its own prompt, which under Chrome style with
/// no toolbar becomes a window of its own placed against the invisible anchor.
///
/// The app owns persistence, so Chromium must not also own it. ACCEPT and DENY
/// are both explicit user actions to Chromium and are written into the
/// profile's content settings, after which that origin never asks again. CEF
/// 151 has no one-time grant to answer with (`cef_permission_request_result_t`
/// is accept/deny/dismiss/ignore), so the answer is put back to the profile
/// default through `cef_request_context_t::set_website_setting` as soon as CEF
/// says it is done with the prompt. `resetPermissions` is the same clearing on
/// demand.
enum NDCefPermissions {
    enum Answer { case allow, deny, dismiss }

    private struct Parked {
        let callback: UInt
        /// Non-zero on the getUserMedia route, where the mask has to come back
        /// exactly as it went out or CEF rejects it.
        let mediaMask: UInt32
        /// Chromium's own id for the prompt, so a dismissal can forget a
        /// request whose callback CEF has already torn down. Zero on the
        /// getUserMedia route, which has no dismissal callback.
        let promptID: UInt64
        /// The bits CEF asked for, whichever route they came in on. Recorded
        /// against the origin when the app answers, so `resetPermissions`
        /// clears only what Chromium has actually prompted for.
        let requestMask: UInt32
        let origin: String
        /// The media route has no dismissal callback to clear the answer from,
        /// so it carries the context it has to clear itself. One owned
        /// reference, released with the parked request.
        let context: UInt
    }

    /// The content settings an answered permission can be written into, so it
    /// can be written back out. A permission type CEF 151 names no content
    /// setting for is left alone rather than guessed at; geolocation has two
    /// because Chromium splits the precise/approximate choice into its own
    /// setting.
    private static let contentSettings: [(UInt32, [cef_content_setting_types_t])] = [
        (UInt32(CEF_PERMISSION_TYPE_AR_SESSION.rawValue), [CEF_CONTENT_SETTING_TYPE_AR]),
        (UInt32(CEF_PERMISSION_TYPE_CAMERA_PAN_TILT_ZOOM.rawValue), [CEF_CONTENT_SETTING_TYPE_CAMERA_PAN_TILT_ZOOM]),
        (UInt32(CEF_PERMISSION_TYPE_CAMERA_STREAM.rawValue), [CEF_CONTENT_SETTING_TYPE_MEDIASTREAM_CAMERA]),
        (UInt32(CEF_PERMISSION_TYPE_CAPTURED_SURFACE_CONTROL.rawValue), [CEF_CONTENT_SETTING_TYPE_CAPTURED_SURFACE_CONTROL]),
        (UInt32(CEF_PERMISSION_TYPE_CLIPBOARD.rawValue), [CEF_CONTENT_SETTING_TYPE_CLIPBOARD_READ_WRITE]),
        (UInt32(CEF_PERMISSION_TYPE_TOP_LEVEL_STORAGE_ACCESS.rawValue), [CEF_CONTENT_SETTING_TYPE_TOP_LEVEL_STORAGE_ACCESS]),
        (UInt32(CEF_PERMISSION_TYPE_DISK_QUOTA.rawValue), [CEF_CONTENT_SETTING_TYPE_PERSISTENT_STORAGE]),
        (UInt32(CEF_PERMISSION_TYPE_LOCAL_FONTS.rawValue), [CEF_CONTENT_SETTING_TYPE_LOCAL_FONTS]),
        (UInt32(CEF_PERMISSION_TYPE_GEOLOCATION.rawValue),
         [CEF_CONTENT_SETTING_TYPE_GEOLOCATION, CEF_CONTENT_SETTING_TYPE_GEOLOCATION_WITH_OPTIONS]),
        (UInt32(CEF_PERMISSION_TYPE_HAND_TRACKING.rawValue), [CEF_CONTENT_SETTING_TYPE_HAND_TRACKING]),
        (UInt32(CEF_PERMISSION_TYPE_IDLE_DETECTION.rawValue), [CEF_CONTENT_SETTING_TYPE_IDLE_DETECTION]),
        (UInt32(CEF_PERMISSION_TYPE_MIC_STREAM.rawValue), [CEF_CONTENT_SETTING_TYPE_MEDIASTREAM_MIC]),
        (UInt32(CEF_PERMISSION_TYPE_MIDI_SYSEX.rawValue), [CEF_CONTENT_SETTING_TYPE_MIDI_SYSEX]),
        (UInt32(CEF_PERMISSION_TYPE_MULTIPLE_DOWNLOADS.rawValue), [CEF_CONTENT_SETTING_TYPE_AUTOMATIC_DOWNLOADS]),
        (UInt32(CEF_PERMISSION_TYPE_NOTIFICATIONS.rawValue), [CEF_CONTENT_SETTING_TYPE_NOTIFICATIONS]),
        (UInt32(CEF_PERMISSION_TYPE_KEYBOARD_LOCK.rawValue), [CEF_CONTENT_SETTING_TYPE_KEYBOARD_LOCK]),
        (UInt32(CEF_PERMISSION_TYPE_POINTER_LOCK.rawValue), [CEF_CONTENT_SETTING_TYPE_POINTER_LOCK]),
        // CEF_CONTENT_SETTING_TYPE_PROTECTED_MEDIA_IDENTIFIER is deliberately
        // absent: desktop Chromium does not register it, and asking for its
        // pattern scope aborts the browser process.
        (UInt32(CEF_PERMISSION_TYPE_REGISTER_PROTOCOL_HANDLER.rawValue), [CEF_CONTENT_SETTING_TYPE_PROTOCOL_HANDLERS]),
        (UInt32(CEF_PERMISSION_TYPE_STORAGE_ACCESS.rawValue), [CEF_CONTENT_SETTING_TYPE_STORAGE_ACCESS]),
        (UInt32(CEF_PERMISSION_TYPE_VR_SESSION.rawValue), [CEF_CONTENT_SETTING_TYPE_VR]),
        (UInt32(CEF_PERMISSION_TYPE_WEB_APP_INSTALLATION.rawValue), [CEF_CONTENT_SETTING_TYPE_WEB_APP_INSTALLATION]),
        (UInt32(CEF_PERMISSION_TYPE_WINDOW_MANAGEMENT.rawValue), [CEF_CONTENT_SETTING_TYPE_WINDOW_MANAGEMENT]),
        (UInt32(CEF_PERMISSION_TYPE_LOCAL_NETWORK.rawValue), [CEF_CONTENT_SETTING_TYPE_LOCAL_NETWORK]),
        (UInt32(CEF_PERMISSION_TYPE_LOOPBACK_NETWORK.rawValue), [CEF_CONTENT_SETTING_TYPE_LOOPBACK_NETWORK]),
        (UInt32(CEF_PERMISSION_TYPE_SENSORS.rawValue), [CEF_CONTENT_SETTING_TYPE_SENSORS]),
    ]

    private static let mediaContentSettings: [(UInt32, [cef_content_setting_types_t])] = [
        (UInt32(CEF_MEDIA_PERMISSION_DEVICE_AUDIO_CAPTURE.rawValue), [CEF_CONTENT_SETTING_TYPE_MEDIASTREAM_MIC]),
        (UInt32(CEF_MEDIA_PERMISSION_DEVICE_VIDEO_CAPTURE.rawValue), [CEF_CONTENT_SETTING_TYPE_MEDIASTREAM_CAMERA]),
    ]

    /// The permission names the app sees. Chromium's own enum is a bitmask, so
    /// a request carrying two of them reports both.
    private static let names: [(UInt32, String)] = [
        (UInt32(CEF_PERMISSION_TYPE_AR_SESSION.rawValue), "arSession"),
        (UInt32(CEF_PERMISSION_TYPE_CAMERA_PAN_TILT_ZOOM.rawValue), "cameraPanTiltZoom"),
        (UInt32(CEF_PERMISSION_TYPE_CAMERA_STREAM.rawValue), "camera"),
        (UInt32(CEF_PERMISSION_TYPE_CAPTURED_SURFACE_CONTROL.rawValue), "capturedSurfaceControl"),
        (UInt32(CEF_PERMISSION_TYPE_CLIPBOARD.rawValue), "clipboard"),
        (UInt32(CEF_PERMISSION_TYPE_TOP_LEVEL_STORAGE_ACCESS.rawValue), "topLevelStorageAccess"),
        (UInt32(CEF_PERMISSION_TYPE_DISK_QUOTA.rawValue), "diskQuota"),
        (UInt32(CEF_PERMISSION_TYPE_LOCAL_FONTS.rawValue), "localFonts"),
        (UInt32(CEF_PERMISSION_TYPE_GEOLOCATION.rawValue), "geolocation"),
        (UInt32(CEF_PERMISSION_TYPE_HAND_TRACKING.rawValue), "handTracking"),
        (UInt32(CEF_PERMISSION_TYPE_IDENTITY_PROVIDER.rawValue), "identityProvider"),
        (UInt32(CEF_PERMISSION_TYPE_IDLE_DETECTION.rawValue), "idleDetection"),
        (UInt32(CEF_PERMISSION_TYPE_MIC_STREAM.rawValue), "microphone"),
        (UInt32(CEF_PERMISSION_TYPE_MIDI_SYSEX.rawValue), "midiSysex"),
        (UInt32(CEF_PERMISSION_TYPE_MULTIPLE_DOWNLOADS.rawValue), "multipleDownloads"),
        (UInt32(CEF_PERMISSION_TYPE_NOTIFICATIONS.rawValue), "notifications"),
        (UInt32(CEF_PERMISSION_TYPE_KEYBOARD_LOCK.rawValue), "keyboardLock"),
        (UInt32(CEF_PERMISSION_TYPE_POINTER_LOCK.rawValue), "pointerLock"),
        (UInt32(CEF_PERMISSION_TYPE_PROTECTED_MEDIA_IDENTIFIER.rawValue), "protectedMediaIdentifier"),
        (UInt32(CEF_PERMISSION_TYPE_REGISTER_PROTOCOL_HANDLER.rawValue), "registerProtocolHandler"),
        (UInt32(CEF_PERMISSION_TYPE_STORAGE_ACCESS.rawValue), "storageAccess"),
        (UInt32(CEF_PERMISSION_TYPE_VR_SESSION.rawValue), "vrSession"),
        (UInt32(CEF_PERMISSION_TYPE_WEB_APP_INSTALLATION.rawValue), "webAppInstallation"),
        (UInt32(CEF_PERMISSION_TYPE_WINDOW_MANAGEMENT.rawValue), "windowManagement"),
        (UInt32(CEF_PERMISSION_TYPE_FILE_SYSTEM_ACCESS.rawValue), "fileSystemAccess"),
        (UInt32(CEF_PERMISSION_TYPE_LOCAL_NETWORK.rawValue), "localNetwork"),
        (UInt32(CEF_PERMISSION_TYPE_LOOPBACK_NETWORK.rawValue), "loopbackNetwork"),
        (UInt32(CEF_PERMISSION_TYPE_SENSORS.rawValue), "sensors"),
    ]

    private static let mediaNames: [(UInt32, String)] = [
        (UInt32(CEF_MEDIA_PERMISSION_DEVICE_AUDIO_CAPTURE.rawValue), "microphone"),
        (UInt32(CEF_MEDIA_PERMISSION_DEVICE_VIDEO_CAPTURE.rawValue), "camera"),
        (UInt32(CEF_MEDIA_PERMISSION_DESKTOP_AUDIO_CAPTURE.rawValue), "desktopAudio"),
        (UInt32(CEF_MEDIA_PERMISSION_DESKTOP_VIDEO_CAPTURE.rawValue), "desktopVideo"),
    ]

    @MainActor private static var pending: [String: Parked] = [:]
    @MainActor private static var sequence = 0
    /// What an outstanding prompt asked for. Read back in `dismiss`.
    @MainActor private static var prompts: [UInt64: (origin: String, mask: UInt32)] = [:]
    /// What each origin has been asked for, so `resetPermissions` has something
    /// to clear: CEF 151 can remove one origin's setting, but has no clear-all
    /// for a content type. Recording the bits rather than sweeping the whole
    /// table is also what keeps the sweep safe, see `clear`.
    @MainActor private static var answeredOrigins: [String: (prompt: UInt32, media: UInt32)] = [:]

    @MainActor private static func remember(_ origin: String, media: UInt32, prompt: UInt32) {
        guard !origin.isEmpty else { return }
        var seen = answeredOrigins[origin] ?? (prompt: 0, media: 0)
        seen.prompt |= prompt
        seen.media |= media
        answeredOrigins[origin] = seen
    }

    @MainActor static func request(
        view: NDCefWebView?,
        promptID: UInt64,
        origin rawOrigin: String,
        mainFrameURL: String,
        frameURL: String?,
        isMainFrame: Bool,
        mask: UInt32,
        callback token: UInt,
        context: UInt,
        media: Bool
    ) {
        let origin = ndCefOriginForm(rawOrigin)
        let parked = Parked(callback: token, mediaMask: media ? mask : 0, promptID: promptID,
                            requestMask: mask, origin: origin, context: context)
        guard let view else {
            answer(parked, result: .dismiss)
            return
        }
        sequence += 1
        let id = "cefpermission-\(sequence)"
        pending[id] = parked
        if !media { prompts[promptID] = (origin, mask) }
        let types = (media ? mediaNames : names)
            .filter { mask & $0.0 != 0 }
            .map { $0.1 }
            .joined(separator: ",")
        view.ndTrace("permissionRequest id=\(id) origin=\(origin) types=\(types) frame=\(frameURL ?? "-") main=\(isMainFrame)")
        var payload: [String: Any] = ["id": id, "origin": origin, "types": types, "mainFrameUrl": mainFrameURL]
        if let frameURL {
            payload["frameUrl"] = frameURL
            payload["isMainFrame"] = isMainFrame
        }
        view.emitData("permissionRequest", payload)
    }

    /// CEF is done with the prompt: either the app answered it, or Chromium
    /// retired it on its own (a navigation, a closed browser). Chromium has
    /// written its content setting by now, so this is where it is written back
    /// out; a request still parked is one nobody answered, and the app is told
    /// the id is dead.
    @MainActor static func dismiss(view: NDCefWebView?, promptID: UInt64,
                                   result: cef_permission_request_result_t,
                                   context: UnsafeMutablePointer<cef_request_context_t>?) {
        guard promptID != 0 else { return }
        if let record = prompts.removeValue(forKey: promptID) {
            // A dismissal records no content setting, so there is nothing to undo.
            if result != CEF_PERMISSION_RESULT_DISMISS, let context {
                clearLater(context: context, origin: record.origin, mask: record.mask, media: false)
            }
        }
        guard let key = pending.first(where: { $0.value.promptID == promptID })?.key else { return }
        guard let parked = pending.removeValue(forKey: key) else { return }
        if let view {
            view.ndTrace("permissionRequestDismissed id=\(key)")
            view.emitData("permissionRequestDismissed", ["id": key])
        }
        if let raw = UnsafeMutableRawPointer(bitPattern: parked.callback) {
            nd_cef_ref_release(raw.assumingMemoryBound(to: cef_permission_prompt_callback_t.self))
        }
        releaseContext(parked)
    }

    /// `respondPermission`, on the UI thread.
    @MainActor static func respond(_ obj: [String: Any]) {
        guard let id = obj["id"] as? String else {
            ndCefWarn("respondPermission: missing id")
            return
        }
        guard let parked = pending.removeValue(forKey: id) else {
            // An id this engine handed out but no longer has parked: the app
            // answered it already, or Chromium retired it and the app's answer
            // was in flight. Neither is a mistake worth a warning.
            if !wasIssued(id) { ndCefWarn("respondPermission: unknown request id \(id)") }
            return
        }
        answer(parked, result: result(from: obj))
    }

    @MainActor private static func result(from obj: [String: Any]) -> Answer {
        if let name = obj["result"] as? String {
            switch name {
            case "allow": return .allow
            case "deny": return .deny
            case "dismiss": return .dismiss
            default: ndCefWarn("respondPermission: unknown result \(name)")
            }
        }
        let allow = (obj["allow"] as? NSNumber)?.boolValue ?? (obj["allow"] as? Bool ?? false)
        return allow ? .allow : .deny
    }

    @MainActor private static func wasIssued(_ id: String) -> Bool {
        let prefix = "cefpermission-"
        guard id.hasPrefix(prefix), let n = Int(id.dropFirst(prefix.count)) else { return false }
        return n >= 1 && n <= sequence
    }

    @MainActor private static func answer(_ parked: Parked, result: Answer) {
        defer { releaseContext(parked) }
        guard let raw = UnsafeMutableRawPointer(bitPattern: parked.callback) else { return }
        if parked.mediaMask != 0 {
            let callback = raw.assumingMemoryBound(to: cef_media_access_callback_t.self)
            // The media callback has no dismiss of its own: cancelling is the
            // only way to end the request without granting it.
            if result == .allow {
                callback.pointee.cont?(callback, parked.mediaMask)
            } else {
                callback.pointee.cancel?(callback)
            }
            nd_cef_ref_release(callback)
            if result != .dismiss, let context = UnsafeMutableRawPointer(bitPattern: parked.context) {
                let ctx = context.assumingMemoryBound(to: cef_request_context_t.self)
                clearLater(context: ctx, origin: parked.origin, mask: parked.mediaMask, media: true)
                remember(parked.origin, media: parked.mediaMask, prompt: 0)
            }
            return
        }
        let callback = raw.assumingMemoryBound(to: cef_permission_prompt_callback_t.self)
        let code: cef_permission_request_result_t
        switch result {
        case .allow: code = CEF_PERMISSION_RESULT_ACCEPT
        case .deny: code = CEF_PERMISSION_RESULT_DENY
        case .dismiss: code = CEF_PERMISSION_RESULT_DISMISS
        }
        callback.pointee.cont?(callback, code)
        nd_cef_ref_release(callback)
        if result != .dismiss { remember(parked.origin, media: 0, prompt: parked.requestMask) }
    }

    @MainActor private static func releaseContext(_ parked: Parked) {
        guard let raw = UnsafeMutableRawPointer(bitPattern: parked.context) else { return }
        nd_cef_ref_release(raw.assumingMemoryBound(to: cef_request_context_t.self))
    }

    /// Always on a later turn, never inline: `on_dismiss_permission_prompt`
    /// runs while Chromium is still finishing the decision, and a clear made
    /// there is overwritten by the content setting the decision then writes.
    /// Measured on 151.3.23: clearing inline from the dismissal left the origin
    /// blocked and it never asked again.
    @MainActor static func clearLater(context: UnsafeMutablePointer<cef_request_context_t>,
                                      origin: String, mask: UInt32, media: Bool) {
        guard mask != 0, !origin.isEmpty else { return }
        nd_cef_ref_add(context)
        let token = UInt(bitPattern: context)
        DispatchQueue.main.async {
            MainActor.assumeIsolated {
                guard let raw = UnsafeMutableRawPointer(bitPattern: token) else { return }
                let ctx = raw.assumingMemoryBound(to: cef_request_context_t.self)
                clear(context: ctx, origin: origin, mask: mask, media: media)
                nd_cef_ref_release(ctx)
            }
        }
    }

    /// Removes the settings an answer may have written, so the same origin asks
    /// again and the app's own store stays the only record of the decision.
    ///
    /// `set_website_setting` with a null value, not `set_content_setting` with
    /// CEF_CONTENT_SETTING_VALUE_DEFAULT: the latter reaches
    /// `HostContentSettingsMap::SetContentSettingDefaultScope`, which traps the
    /// browser process for a type Chromium keeps as a website setting
    /// (geolocation's precise/approximate choice is one). Measured on 151.3.23:
    /// the first answered geolocation prompt took the host down there.
    ///
    /// Both URLs, never a null top level: CEF derives the rule's pattern pair
    /// from them, and a type scoped to the requesting origin alone ignores the
    /// second. `mask` only ever carries bits Chromium itself raised a prompt
    /// for, which is what keeps this safe: a type Chromium has registered no
    /// pattern scope for traps the process.
    @MainActor private static func clear(context: UnsafeMutablePointer<cef_request_context_t>,
                                         origin: String, mask: UInt32, media: Bool) {
        guard !origin.isEmpty, let set = context.pointee.set_website_setting else { return }
        var url = cef_string_t()
        ndCefSetString(origin, &url)
        defer { nd_cef_string_clear(&url) }
        for (bit, settings) in (media ? mediaContentSettings : contentSettings) where mask & bit != 0 {
            for setting in settings {
                set(context, &url, &url, setting, nil)
            }
        }
    }

    /// `resetPermissions`: puts Chromium's stored decisions for an origin back
    /// to the profile default, so a site the app blocked long ago asks again.
    /// With no `origin` it covers every origin this process has answered for,
    /// which is as wide as CEF 151 goes: there is no clear-all for a type.
    @MainActor static func reset(_ obj: [String: Any], context: UnsafeMutablePointer<cef_request_context_t>?) {
        guard let context else { return }
        var filter = UInt32.max
        var mediaFilter = UInt32.max
        if let wanted = obj["types"] as? [String] {
            filter = names.filter { wanted.contains($0.1) }.reduce(0) { $0 | $1.0 }
            mediaFilter = mediaNames.filter { wanted.contains($0.1) }.reduce(0) { $0 | $1.0 }
        }
        let only = (obj["origin"] as? String).map(ndCefOriginForm)
        for (origin, seen) in answeredOrigins where only == nil || only == origin {
            clearLater(context: context, origin: origin, mask: seen.prompt & filter, media: false)
            clearLater(context: context, origin: origin, mask: seen.media & mediaFilter, media: true)
        }
    }
}

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

// MARK: - Downloads

/// Downloads, parked until the app answers `respondDownload`, then reported to
/// it as `downloadUpdated` until they end. The CEF download item id names a
/// download from `on_before_download` to its last update.
enum NDCefDownloads {
    struct Update {
        let itemID: UInt32
        let received: Int64
        let total: Int64
        let path: String
        let state: String

        init(_ item: UnsafeMutablePointer<cef_download_item_t>) {
            itemID = item.pointee.get_id?(item) ?? 0
            received = item.pointee.get_received_bytes?(item) ?? 0
            total = item.pointee.get_total_bytes?(item) ?? -1
            var full = ""
            if let raw = item.pointee.get_full_path?(item) {
                full = ndCefString(raw)
                nd_cef_string_free(raw)
            }
            path = full
            if item.pointee.is_complete?(item) == 1 {
                state = "done"
            } else if item.pointee.is_canceled?(item) == 1 {
                state = "cancelled"
            } else if item.pointee.is_interrupted?(item) == 1 {
                state = "failed"
            } else {
                state = "running"
            }
        }
    }

    @MainActor private static var pending: [String: UInt] = [:]
    /// Downloads the app gave a path, with the last state reported, so an
    /// update that changes nothing is not sent again.
    @MainActor private static var running: [String: String] = [:]

    @MainActor private static var lastStarted: Date?

    /// A download began within the last few seconds, while Chrome may still
    /// be putting up its download-started animation.
    @MainActor static var startedRecently: Bool {
        guard let lastStarted else { return false }
        return Date().timeIntervalSince(lastStarted) < 5
    }

    private static func key(_ itemID: UInt32) -> String { "cefdownload-\(itemID)" }

    @MainActor static func request(view: NDCefWebView?, itemID: UInt32, url: String, suggestedName: String, callback token: UInt) {
        guard let view else {
            release(token)
            return
        }
        let id = key(itemID)
        if let stale = pending.removeValue(forKey: id) { release(stale) }
        pending[id] = token
        view.ndTrace("downloadRequested id=\(id) \(url)")
        var fields: [String: Any] = ["id": id, "url": url]
        if !suggestedName.isEmpty { fields["suggestedFilename"] = suggestedName }
        view.emitData("downloadRequested", fields)
    }

    /// `respondDownload`, on the UI thread. `path` is the full destination;
    /// without one the download is cancelled.
    @MainActor static func respond(_ obj: [String: Any]) {
        guard let id = obj["id"] as? String else {
            ndCefWarn("respondDownload: missing id")
            return
        }
        guard let token = pending.removeValue(forKey: id),
              let raw = UnsafeMutableRawPointer(bitPattern: token) else {
            ndCefWarn("respondDownload: unknown download id \(id)")
            return
        }
        let callback = raw.assumingMemoryBound(to: cef_before_download_callback_t.self)
        if let path = obj["path"] as? String, !path.isEmpty {
            running[id] = ""
            lastStarted = Date()
            var target = cef_string_t()
            ndCefSetString(path, &target)
            callback.pointee.cont?(callback, &target, 0)
            nd_cef_string_clear(&target)
        }
        nd_cef_ref_release(callback)
    }

    @MainActor static func updated(view: NDCefWebView?, _ update: Update) {
        let id = key(update.itemID)
        guard let last = running[id] else { return }
        let signature = "\(update.state)/\(update.received)"
        guard signature != last else { return }
        if update.state == "running" {
            running[id] = signature
        } else {
            running.removeValue(forKey: id)
        }
        guard let view else { return }
        view.ndTrace("downloadUpdated id=\(id) state=\(update.state) received=\(update.received)")
        view.emitData("downloadUpdated", [
            "id": id, "state": update.state, "received": update.received, "total": update.total, "path": update.path,
        ])
    }

    private static func release(_ token: UInt) {
        guard let raw = UnsafeMutableRawPointer(bitPattern: token) else { return }
        nd_cef_ref_release(raw.assumingMemoryBound(to: cef_before_download_callback_t.self))
    }
}
#endif
