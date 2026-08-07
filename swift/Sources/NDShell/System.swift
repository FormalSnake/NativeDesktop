import AppKit
import CNd
import Foundation
import Security
import UniformTypeIdentifiers
import UserNotifications

/// AppKit half of the "system capabilities" seam (vtable member #22,
/// `system_request`): dialogs, clipboard, notifications, recent documents, and
/// keychain credentials. The core pre-gates the ACL + unknown methods and
/// marshals every request onto the UI thread, so each `handleRequest` runs
/// MainActor-isolated and answers with exactly one `nd_system_response` (ok=true
/// carries a well-formed JSON result value; ok=false a plain error message).
/// App-level events (activation, OS open-file/url, notification click, file
/// drop) push out through `emitEvent` -> `nd_system_event`.
enum NDSystem {
    // MARK: - request dispatch

    @MainActor static func handleRequest(id: UInt32, method: String, paramsJson: String) {
        let params = parseProps(paramsJson)
        switch method {
        case "dialog.openFile": openFile(id, params)
        case "dialog.saveFile": saveFile(id, params)
        case "dialog.showMessage": showMessage(id, params)
        case "clipboard.readText":
            respondResult(id, jsonFragment(NSPasteboard.general.string(forType: .string) ?? ""))
        case "clipboard.readImage": readImage(id)
        case "clipboard.writeText":
            let text = propStr(params, "text") ?? ""
            let pb = NSPasteboard.general
            pb.clearContents()
            pb.setString(text, forType: .string)
            respondResult(id, "null")
        case "notification.show": showNotification(id, params)
        case "recent.add":
            if let path = propStr(params, "path") {
                NSDocumentController.shared.noteNewRecentDocumentURL(URL(fileURLWithPath: path))
            }
            respondResult(id, "null")
        case "recent.clear":
            NSDocumentController.shared.clearRecentDocuments(nil)
            respondResult(id, "null")
        case "credentials.set": credentialsSet(id, params)
        case "credentials.get": credentialsGet(id, params)
        case "credentials.delete": credentialsDelete(id, params)
        case "audio.play": NDAudio.play(id, params)
        case "audio.pause": NDAudio.pause(id, params)
        case "audio.resume": NDAudio.resume(id, params)
        case "audio.stop": NDAudio.stop(id, params)
        case "audio.seek": NDAudio.seek(id, params)
        case "audio.setVolume": NDAudio.setVolume(id, params)
        case "system.getAppearance":
            ensureAppearanceWatch()
            respondResult(id, jsonFragment(currentAppearance()))
        default:
            // The core already gates truly unknown methods before dispatch here.
            respondError(id, "not implemented")
        }
    }

    // MARK: - reply / event helpers

    /// ok=true reply: `jsonValue` must be a well-formed JSON value (the core
    /// splices it verbatim into `systemResponse.result`).
    static func respondResult(_ id: UInt32, _ jsonValue: String) {
        jsonValue.withCString { nd_system_response(gCtx, id, true, $0) }
    }

    /// ok=false reply: `message` is a plain error string (the core wraps it as
    /// the `errorMessage` field itself).
    static func respondError(_ id: UInt32, _ message: String) {
        message.withCString { nd_system_response(gCtx, id, false, $0) }
    }

    /// App-level (non-widget) event. `dataJson` must be a JSON object string.
    static func emitEvent(channel: String, dataJson: String) {
        channel.withCString { c in dataJson.withCString { d in nd_system_event(gCtx, c, d) } }
    }

    /// Encodes a JSON fragment (String/Int/[String]/NSNull/…) by round-tripping
    /// a one-element array through JSONSerialization and stripping the brackets
    /// — proper escaping for paths/URLs without hand-splicing (JSONSerialization
    /// rejects a bare scalar at top level, hence the array wrap).
    static func jsonFragment(_ value: Any) -> String {
        guard let data = try? JSONSerialization.data(withJSONObject: [value]),
              var s = String(data: data, encoding: .utf8), s.count >= 2 else { return "null" }
        s.removeFirst()
        s.removeLast()
        return s
    }

    // MARK: - clipboard image (WP-B1)

    @MainActor private static var imageCounter = 0

    /// clipboard.readImage: writes the clipboard's bitmap image to a host-local
    /// temp PNG and returns {path,width,height}. Privileged
    /// (core:clipboard.read.image, default-deny) — image bytes never enter NDP,
    /// only the resulting path does.
    @MainActor private static func readImage(_ id: UInt32) {
        let pb = NSPasteboard.general
        var png = pb.data(forType: .png)
        if png == nil, let tiff = pb.data(forType: .tiff), let rep = NSBitmapImageRep(data: tiff) {
            png = rep.representation(using: .png, properties: [:])
        }
        guard let data = png, let rep = NSBitmapImageRep(data: data) else {
            return respondError(id, "no image on clipboard")
        }
        let name = "nd-clip-\(ProcessInfo.processInfo.processIdentifier)-\(imageCounter).png"
        imageCounter += 1
        let path = (NSTemporaryDirectory() as NSString).appendingPathComponent(name)
        do {
            try data.write(to: URL(fileURLWithPath: path))
        } catch {
            return respondError(id, "write failed")
        }
        respondResult(id, "{\"path\":\(jsonFragment(path)),\"width\":\(rep.pixelsWide),\"height\":\(rep.pixelsHigh)}")
    }

    // MARK: - dialogs

    /// The window a dialog sheet parents to: the key window, else any visible
    /// app window, else nil (run a non-sheet modal).
    @MainActor private static func dialogParent() -> NSWindow? {
        NSApp.keyWindow ?? NSApp.windows.first(where: { $0.isVisible })
    }

    /// Runs a save/open panel as a sheet on the dialog parent when one exists,
    /// else as an app-modal panel; `completion` fires with the modal response
    /// on the main thread either way.
    @MainActor private static func runPanel(
        _ panel: NSSavePanel, completion: @escaping (NSApplication.ModalResponse) -> Void
    ) {
        if let parent = dialogParent() {
            panel.beginSheetModal(for: parent, completionHandler: completion)
        } else {
            completion(panel.runModal())
        }
    }

    /// `filters: [{name, extensions:[String]}]` -> UTTypes; unresolvable
    /// extensions are skipped.
    private static func contentTypes(from filters: [[String: Any]]) -> [UTType] {
        var types: [UTType] = []
        for filter in filters {
            guard let exts = propArray(filter, "extensions") else { continue }
            for ext in exts {
                if let type = UTType(filenameExtension: ext) { types.append(type) }
            }
        }
        return types
    }

    @MainActor private static func openFile(_ id: UInt32, _ p: [String: Any]) {
        let panel = NSOpenPanel()
        if let title = propStr(p, "title") { panel.message = title }
        panel.allowsMultipleSelection = propBool(p, "multiple") ?? false
        let directories = propBool(p, "directories") ?? false
        panel.canChooseDirectories = directories
        panel.canChooseFiles = !directories
        if !directories, let filters = propObjArray(p, "filters") {
            let types = contentTypes(from: filters)
            if !types.isEmpty { panel.allowedContentTypes = types }
        }
        if let defaultPath = propStr(p, "defaultPath") {
            panel.directoryURL = URL(fileURLWithPath: defaultPath)
        }
        runPanel(panel) { resp in
            let paths = resp == .OK ? panel.urls.map { $0.path } : []
            respondResult(id, jsonFragment(paths))
        }
    }

    @MainActor private static func saveFile(_ id: UInt32, _ p: [String: Any]) {
        let panel = NSSavePanel()
        if let title = propStr(p, "title") { panel.message = title }
        if let name = propStr(p, "defaultName") { panel.nameFieldStringValue = name }
        if let filters = propObjArray(p, "filters") {
            let types = contentTypes(from: filters)
            if !types.isEmpty { panel.allowedContentTypes = types }
        }
        if let defaultPath = propStr(p, "defaultPath") {
            panel.directoryURL = URL(fileURLWithPath: defaultPath)
        }
        runPanel(panel) { resp in
            if resp == .OK, let url = panel.url {
                respondResult(id, jsonFragment(url.path))
            } else {
                respondResult(id, "null")
            }
        }
    }

    @MainActor private static func showMessage(_ id: UInt32, _ p: [String: Any]) {
        let alert = NSAlert()
        alert.messageText = propStr(p, "message") ?? ""
        if let detail = propStr(p, "detail") { alert.informativeText = detail }
        switch propStr(p, "level") {
        case "warning": alert.alertStyle = .warning
        case "error": alert.alertStyle = .critical
        default: alert.alertStyle = .informational
        }
        let buttons = propArray(p, "buttons") ?? ["OK"]
        for label in buttons { alert.addButton(withTitle: label) }
        // First-added button is AppKit's default; honor an explicit
        // defaultButton by moving the Return key equivalent to it.
        if let def = propInt(p, "defaultButton"), def >= 0, def < alert.buttons.count {
            for (index, button) in alert.buttons.enumerated() {
                button.keyEquivalent = index == def ? "\r" : ""
            }
        }
        let handle: (NSApplication.ModalResponse) -> Void = { resp in
            let index = resp.rawValue - NSApplication.ModalResponse.alertFirstButtonReturn.rawValue
            respondResult(id, jsonFragment(index))
        }
        if let parent = dialogParent() {
            alert.beginSheetModal(for: parent, completionHandler: handle)
        } else {
            handle(alert.runModal())
        }
    }

    // MARK: - notifications

    nonisolated(unsafe) private static var unAuthorizationRequested = false

    @MainActor private static func showNotification(_ id: UInt32, _ p: [String: Any]) {
        let title = propStr(p, "title") ?? ""
        let body = propStr(p, "body") ?? ""
        let notifID = UUID().uuidString
        // UNUserNotificationCenter.current() traps for an unbundled process
        // (bundleProxyForCurrentProcess is nil). A bare SwiftPM executable has
        // no bundle identifier in dev, so fall back to the legacy
        // NSUserNotification API there; use the modern center only when bundled.
        if Bundle.main.bundleIdentifier != nil {
            let center = UNUserNotificationCenter.current()
            center.delegate = NDUNNotifier.shared
            if !unAuthorizationRequested {
                unAuthorizationRequested = true
                center.requestAuthorization(options: [.alert, .sound]) { _, _ in }
            }
            let content = UNMutableNotificationContent()
            content.title = title
            if !body.isEmpty { content.body = body }
            let request = UNNotificationRequest(identifier: notifID, content: content, trigger: nil)
            center.add(request) { _ in }
        } else {
            NDLegacyNotifier.shared.deliver(id: notifID, title: title, body: body)
        }
        respondResult(id, jsonFragment(notifID))
    }

    // MARK: - appearance

    nonisolated(unsafe) private static var appearanceObs: NSKeyValueObservation?

    /// `"dark"`/`"light"` best-match of the effective appearance — the same
    /// aqua/darkAqua pair AppKit resolves menu bars and system chrome against,
    /// so this tracks accent-color-independent light/dark switches (System
    /// Settings, or an app-level `NSApp.appearance` override).
    @MainActor private static func currentAppearance() -> String {
        NSApp.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua ? "dark" : "light"
    }

    /// Installs a one-time KVO watch on `NSApp.effectiveAppearance`, pushing an
    /// `appearance` system event on every change. Idempotent (first
    /// `system.getAppearance` call wins) — mirrors `unAuthorizationRequested`'s
    /// lazy-once pattern above.
    @MainActor private static func ensureAppearanceWatch() {
        guard appearanceObs == nil else { return }
        appearanceObs = NSApp.observe(\.effectiveAppearance, options: [.new]) { _, _ in
            DispatchQueue.main.async {
                let appearance = MainActor.assumeIsolated { currentAppearance() }
                emitEvent(channel: "appearance", dataJson: "{\"appearance\":\(jsonFragment(appearance))}")
            }
        }
    }

    // MARK: - keychain credentials

    private static func credentialsSet(_ id: UInt32, _ p: [String: Any]) {
        guard let service = propStr(p, "service"), let account = propStr(p, "account"),
              let secret = propStr(p, "secret") else {
            respondError(id, "missing service/account/secret")
            return
        }
        let data = Data(secret.utf8)
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        var addQuery = query
        addQuery[kSecValueData as String] = data
        var status = SecItemAdd(addQuery as CFDictionary, nil)
        if status == errSecDuplicateItem {
            status = SecItemUpdate(query as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        }
        if status == errSecSuccess {
            respondResult(id, "null")
        } else {
            respondError(id, "keychain error \(status)")
        }
    }

    private static func credentialsGet(_ id: UInt32, _ p: [String: Any]) {
        guard let service = propStr(p, "service"), let account = propStr(p, "account") else {
            respondError(id, "missing service/account")
            return
        }
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        switch status {
        case errSecSuccess:
            if let data = result as? Data, let secret = String(data: data, encoding: .utf8) {
                respondResult(id, jsonFragment(secret))
            } else {
                respondResult(id, "null")
            }
        case errSecItemNotFound:
            respondResult(id, "null")
        default:
            respondError(id, "keychain error \(status)")
        }
    }

    private static func credentialsDelete(_ id: UInt32, _ p: [String: Any]) {
        guard let service = propStr(p, "service"), let account = propStr(p, "account") else {
            respondError(id, "missing service/account")
            return
        }
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        let status = SecItemDelete(query as CFDictionary)
        if status == errSecSuccess || status == errSecItemNotFound {
            respondResult(id, "null")
        } else {
            respondError(id, "keychain error \(status)")
        }
    }
}

/// Modern notification delegate (bundled apps): banner presentation while
/// active + click delivery through `notification.click`. Only wired when a
/// bundle identifier exists, so it never touches UNUserNotificationCenter in
/// the unbundled dev path. Same `nonisolated(unsafe)` singleton idiom as
/// `EventDispatcher.shared`.
final class NDUNNotifier: NSObject, UNUserNotificationCenterDelegate {
    nonisolated(unsafe) static let shared = NDUNNotifier()

    func userNotificationCenter(
        _ center: UNUserNotificationCenter, willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .sound])
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        let notifID = response.notification.request.identifier
        DispatchQueue.main.async {
            NDSystem.emitEvent(channel: "notification.click", dataJson: "{\"id\":\(NDSystem.jsonFragment(notifID))}")
        }
        completionHandler()
    }
}

/// Legacy notification path for the unbundled dev executable, where
/// UNUserNotificationCenter is unusable. NSUserNotification is deprecated but
/// remains functional for bare processes; the deprecation warnings are isolated
/// to this small class.
final class NDLegacyNotifier: NSObject, NSUserNotificationCenterDelegate {
    nonisolated(unsafe) static let shared = NDLegacyNotifier()

    override init() {
        super.init()
        NSUserNotificationCenter.default.delegate = self
    }

    func deliver(id: String, title: String, body: String) {
        let notification = NSUserNotification()
        notification.identifier = id
        notification.title = title
        if !body.isEmpty { notification.informativeText = body }
        NSUserNotificationCenter.default.deliver(notification)
    }

    func userNotificationCenter(
        _ center: NSUserNotificationCenter, shouldPresent notification: NSUserNotification
    ) -> Bool {
        true
    }

    func userNotificationCenter(
        _ center: NSUserNotificationCenter, didActivate notification: NSUserNotification
    ) {
        let notifID = notification.identifier ?? ""
        NDSystem.emitEvent(channel: "notification.click", dataJson: "{\"id\":\(NDSystem.jsonFragment(notifID))}")
    }
}

/// Transparent, mouse-pass-through drop catcher (`hitTest` returns nil so it
/// never steals clicks from the app UI beneath it) that reports file drops as
/// the `fileDrop` system event. Installed over every window's frame view by
/// `ndInstallFileDrop`, so it survives the content-view swap a SplitView
/// performs (the frame view outlives the content view).
final class NDFileDropView: NSView {
    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        registerForDraggedTypes([.fileURL])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        hasFileURLs(sender) ? .copy : []
    }

    override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation {
        hasFileURLs(sender) ? .copy : []
    }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        let paths = fileURLs(sender).map { $0.path }
        guard !paths.isEmpty else { return false }
        NDSystem.emitEvent(
            channel: "fileDrop",
            dataJson: "{\"paths\":\(NDSystem.jsonFragment(paths)),\"windowId\":0}")
        return true
    }

    private func hasFileURLs(_ sender: NSDraggingInfo) -> Bool {
        sender.draggingPasteboard.canReadObject(
            forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true])
    }

    private func fileURLs(_ sender: NSDraggingInfo) -> [URL] {
        (sender.draggingPasteboard.readObjects(
            forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL]) ?? []
    }
}

/// Installs a `NDFileDropView` over `window`'s frame view (its content view's
/// superview, which persists across content-view/SplitView swaps). Idempotent
/// per window; called from `gWindow`'s observer so every `<window>` root gets
/// drop handling, not just the first.
func ndInstallFileDrop(on window: NSWindow) {
    MainActor.assumeIsolated {
        guard let host = window.contentView?.superview ?? window.contentView else { return }
        if host.subviews.contains(where: { $0 is NDFileDropView }) { return }
        let drop = NDFileDropView(frame: host.bounds)
        drop.autoresizingMask = [.width, .height]
        host.addSubview(drop, positioned: .above, relativeTo: nil)
    }
}
