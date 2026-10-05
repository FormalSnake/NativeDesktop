#if canImport(CCef)
import CCef
import Foundation

/// The extension registry and action commands on the Chromium engine, the
/// AppKit peer of the ones in src/cef/engine.zig. The page scripts are the
/// same documents; see that file for why each one reads what it reads.
///
/// Registry commands run against a view showing chrome://extensions, the one
/// page `chrome.developerPrivate` exists on. `readExtensionAction` runs against
/// a page of the extension. `triggerExtensionAction` is the click itself and
/// goes over the browser target (NDCefBrowserProtocol.swift).
extension NDCefWebView {
    static let extensionsChangedBinding = "__ndExtensionsChanged"

    private static let registryBudget: TimeInterval = 30
    private static let installBudget: TimeInterval = 90

    private static let listExtensionsJS = """
    (async () => {
      if (typeof chrome === "undefined" || !chrome.developerPrivate) {
        throw new Error("listExtensions needs a view showing chrome://extensions");
      }
      const list = await new Promise((resolve) => chrome.developerPrivate.getExtensionsInfo(
        { includeDisabled: true, includeTerminated: true }, resolve));
      return JSON.stringify(list.filter((e) => e.type === "EXTENSION" && e.id !== "\(NDCefFrameworkExtension.id)").map((e) => ({
        id: e.id,
        name: e.name,
        version: e.version,
        enabled: e.state === "ENABLED",
        iconUrl: e.iconUrl || "",
        optionsUrl: (e.optionsPage && e.optionsPage.url) || "",
      })));
    })()
    """

    private static let watchExtensionsJS = """
    (() => {
      if (typeof chrome === "undefined" || !chrome.developerPrivate) {
        throw new Error("watchExtensions needs a view showing chrome://extensions");
      }
      if (globalThis.__ndExtensionsWatch) return JSON.stringify(globalThis.__ndExtensionsWatch);
      const send = (reason, id) => {
        try { globalThis.__ndExtensionsChanged(JSON.stringify({ reason: String(reason), id: id ? String(id) : "" })); } catch (e) {}
      };
      const sources = [];
      const item = chrome.developerPrivate.onItemStateChanged;
      if (item && typeof item.addListener === "function") {
        item.addListener((e) => send((e && e.event_type) || "itemStateChanged", e && e.item_id));
        sources.push("developerPrivate.onItemStateChanged");
      }
      for (const name of ["onInstalled", "onUninstalled", "onEnabled", "onDisabled"]) {
        const ev = chrome.management && chrome.management[name];
        if (ev && typeof ev.addListener === "function") {
          ev.addListener((info) => send(name, typeof info === "string" ? info : info && info.id));
          sources.push("management." + name);
        }
      }
      if (sources.length === 0) throw new Error("watchExtensions: this page exposes no registry events");
      globalThis.__ndExtensionsWatch = sources;
      return JSON.stringify(sources);
    })()
    """

    private static let listExtensionActionsJS = """
    (async () => {
      if (typeof chrome === "undefined" || !chrome.developerPrivate) {
        throw new Error("listExtensionActions needs a view showing chrome://extensions");
      }
      const list = await new Promise((resolve) => chrome.developerPrivate.getExtensionsInfo(
        { includeDisabled: true, includeTerminated: true }, resolve));
      return JSON.stringify(list.filter((e) => e.type === "EXTENSION" && e.id !== "\(NDCefFrameworkExtension.id)").map((e) => ({
        id: e.id,
        name: e.name,
        version: e.version,
        enabled: e.state === "ENABLED",
        iconUrl: e.iconUrl || "",
        path: e.path || "",
      })));
    })()
    """

    private static let readActionJS = """
    (async () => {
      if (typeof chrome === "undefined" || !chrome.action) {
        throw new Error("readExtensionAction needs a view showing a page of the extension");
      }
      const active = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
      const tab = active.length ? active[0] : null;
      const where = tab ? { tabId: tab.id } : {};
      const color = await chrome.action.getBadgeBackgroundColor(where);
      return JSON.stringify({
        id: chrome.runtime.id,
        tabId: tab ? tab.id : 0,
        tabUrl: tab ? (tab.url || "") : "",
        popupUrl: await chrome.action.getPopup(where),
        badgeText: await chrome.action.getBadgeText(where),
        badgeColor: Array.isArray(color) ? color : [],
        title: await chrome.action.getTitle(where),
        enabled: tab ? await chrome.action.isEnabled(tab.id) : true,
      });
    })()
    """

    /// The three registry changes run their one step and answer with the list
    /// the registry became. One function expression: Runtime.evaluate leaves a
    /// top-level `const` in the page's global scope.
    private static func mutation(_ body: String) -> String {
        """
        (async () => {
          if (typeof chrome === "undefined" || !chrome.developerPrivate) {
            throw new Error("this command needs a view showing chrome://extensions");
          }
          const listed = async () => {
            const list = await new Promise((resolve) => chrome.developerPrivate.getExtensionsInfo(
              { includeDisabled: true, includeTerminated: true }, resolve));
            return JSON.stringify(list.filter((e) => e.type === "EXTENSION" && e.id !== "\(NDCefFrameworkExtension.id)").map((e) => ({
              id: e.id,
              name: e.name,
              version: e.version,
              enabled: e.state === "ENABLED",
              iconUrl: e.iconUrl || "",
              optionsUrl: (e.optionsPage && e.optionsPage.url) || "",
            })));
          };
        \(body)
          return await listed();
        })()
        """
    }

    /// `developerPrivate.loadUnpacked` takes no path: it opens a directory
    /// chooser, which `answerInstallDialog` fills with the parked one.
    private static let installBody = """
      await new Promise((resolve) => chrome.developerPrivate.updateProfileConfiguration(
        { inDeveloperMode: true }, resolve));
      const loaded = await new Promise((resolve, reject) => chrome.developerPrivate.loadUnpacked(
        { failQuietly: true, populateError: true },
        (result) => chrome.runtime.lastError
          ? reject(new Error(chrome.runtime.lastError.message))
          : resolve(result)));
      if (loaded && loaded.error) throw new Error(loaded.error);
    """

    // MARK: - Dispatch

    /// True when `command` was one of these.
    func ndHandleExtensionCommand(_ command: String, _ obj: [String: Any]) -> Bool {
        switch command {
        case "listExtensions", "watchExtensions", "listExtensionActions", "readExtensionAction",
             "installExtension", "uninstallExtension", "setExtensionEnabled", "triggerExtensionAction":
            break
        default:
            return false
        }
        guard let id = obj["id"] as? String else {
            ndCefWarn("\(command): malformed arg (expected {id})")
            return true
        }
        switch command {
        case "listExtensions":
            runJSON(id, event: "extensionsList", key: "extensions", what: command,
                    budget: Self.registryBudget, code: Self.listExtensionsJS)
        case "watchExtensions":
            if !extensionsWatched {
                devTools.call("Runtime.addBinding", ["name": Self.extensionsChangedBinding])
                extensionsWatched = true
            }
            runJSON(id, event: "extensionsChanged", key: "sources", what: command,
                    budget: Self.registryBudget, code: Self.watchExtensionsJS)
        case "listExtensionActions":
            listExtensionActions(id)
        case "readExtensionAction":
            runJSON(id, event: "extensionActions", key: "action", what: command,
                    budget: Self.registryBudget, code: Self.readActionJS)
        case "installExtension":
            guard let path = obj["path"] as? String else {
                ndCefWarn("installExtension: malformed arg (expected {id, path})")
                return true
            }
            pendingInstallPath = path
            installInFlight = true
            runJSON(id, event: "extensionsList", key: "extensions", what: "installExtension \(path)",
                    budget: Self.installBudget, code: Self.mutation(Self.installBody)) { [weak self] in
                self?.installInFlight = false
                self?.pendingInstallPath = nil
            }
        case "uninstallExtension":
            guard let target = obj["extensionId"] as? String else {
                ndCefWarn("uninstallExtension: malformed arg (expected {id, extensionId})")
                return true
            }
            uninstallExtension(id, extensionID: target)
        case "setExtensionEnabled":
            guard let target = obj["extensionId"] as? String else {
                ndCefWarn("setExtensionEnabled: malformed arg (expected {id, extensionId, enabled})")
                return true
            }
            let enabled = (obj["enabled"] as? Bool) ?? true
            let body = """
              await new Promise((resolve, reject) => chrome.management.setEnabled(\(Self.jsLiteral(target)), \(enabled),
                () => chrome.runtime.lastError ? reject(new Error(chrome.runtime.lastError.message)) : resolve()));
            """
            runJSON(id, event: "extensionsList", key: "extensions", what: command,
                    budget: Self.registryBudget, code: Self.mutation(body))
        case "triggerExtensionAction":
            guard let extensionID = obj["extensionId"] as? String else {
                ndCefWarn("triggerExtensionAction: malformed arg (expected {id, extensionId})")
                return true
            }
            triggerExtensionAction(id, extensionID: extensionID)
        default:
            break
        }
        return true
    }

    // MARK: - Page commands

    /// One page script whose string answer is JSON, emitted as `event` with the
    /// parsed value under `key`. A script that never settles (a confirmation
    /// nobody answers, a chooser that never opened) fails at `budget`.
    private func runJSON(
        _ id: String, event: String, key: String, what: String, budget: TimeInterval, code: String,
        done: (@MainActor @Sendable () -> Void)? = nil
    ) {
        let once = NDCefOnce()
        let finish: @MainActor @Sendable (Any?, String?) -> Void = { [weak self] value, error in
            guard once.claim() else { return }
            done?()
            var payload: [String: Any] = ["id": id, "ok": error == nil]
            if let error { payload["error"] = error } else { payload[key] = value ?? NSNull() }
            self?.emitData(event, payload)
        }
        Timer.scheduledTimer(withTimeInterval: budget, repeats: false) { _ in
            MainActor.assumeIsolated { finish(nil, "\(what): no answer within \(Int(budget)) s") }
        }
        devTools.evaluate(code, world: "", userGesture: true) { text, error in
            if let error {
                finish(nil, error)
                return
            }
            guard let data = text?.data(using: .utf8),
                  let value = try? JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed]) else {
                finish(nil, "\(what): unreadable answer")
                return
            }
            finish(value, nil)
        }
    }

    private func listExtensionActions(_ id: String) {
        devTools.evaluate(Self.listExtensionActionsJS, world: "", userGesture: false) { [weak self] text, error in
            guard let self else { return }
            if let error {
                self.emitData("extensionActions", ["id": id, "ok": false, "error": error])
                return
            }
            guard let data = text?.data(using: .utf8),
                  let rows = (try? JSONSerialization.jsonObject(with: data)) as? [[String: Any]] else {
                self.emitData("extensionActions", ["id": id, "ok": false, "error": "listExtensionActions: unreadable answer"])
                return
            }
            self.emitData("extensionActions", ["id": id, "ok": true, "actions": self.extensionActions(rows)])
        }
    }

    /// The action is declared in the manifest and nowhere Chromium hands it
    /// over, so each row's manifest is read off disk: `path` for an unpacked
    /// extension, the profile's `Extensions/<id>/<version>_<n>` for one from
    /// the store, where `n` counts reinstalls of the same version.
    private func extensionActions(_ rows: [[String: Any]]) -> [[String: Any]] {
        var out: [[String: Any]] = []
        for row in rows {
            guard let id = row["id"] as? String else { continue }
            let version = row["version"] as? String ?? ""
            let path = row["path"] as? String ?? ""
            guard let manifest = Self.readManifest(id: id, version: version, path: path, profileDirectory: extensionProfileDirectory),
                  let action = Self.manifestAction(manifest) else { continue }
            let name = row["name"] as? String ?? ""
            var iconURL = row["iconUrl"] as? String ?? ""
            if let icon = Self.actionIcon(action) { iconURL = "chrome-extension://\(id)/\(icon)" }
            var popupURL = ""
            if let popup = action["default_popup"] as? String { popupURL = "chrome-extension://\(id)/\(popup)" }
            out.append([
                "id": id,
                "name": name,
                "enabled": row["enabled"] as? Bool ?? false,
                "title": action["default_title"] as? String ?? name,
                "iconUrl": iconURL,
                "popupUrl": popupURL,
                "badgeText": "",
            ])
        }
        return out
    }

    /// The directory Chromium keeps this view's profile in.
    private var extensionProfileDirectory: String {
        profile.isEmpty ? "\(NDCefRuntime.rootCachePath)/Default" : NDCefRuntime.profileCachePath(profile)
    }

    private static func readManifest(id: String, version: String, path: String, profileDirectory: String) -> [String: Any]? {
        var candidates: [String] = []
        if !path.isEmpty {
            candidates.append("\(path)/manifest.json")
        } else {
            for n in 0..<4 { candidates.append("\(profileDirectory)/Extensions/\(id)/\(version)_\(n)/manifest.json") }
        }
        for candidate in candidates {
            guard let data = FileManager.default.contents(atPath: candidate), data.count <= 1 << 20,
                  let manifest = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { continue }
            return manifest
        }
        return nil
    }

    /// MV2 spells the action `browser_action` or `page_action`; both still load.
    private static func manifestAction(_ manifest: [String: Any]) -> [String: Any]? {
        for key in ["action", "browser_action", "page_action"] {
            if let action = manifest[key] as? [String: Any] { return action }
        }
        return nil
    }

    /// `default_icon` is one path or a size-keyed map; the largest size wins.
    private static func actionIcon(_ action: [String: Any]) -> String? {
        if let single = action["default_icon"] as? String { return single }
        guard let sizes = action["default_icon"] as? [String: Any] else { return nil }
        return sizes.compactMap { key, value -> (Int, String)? in
            guard let path = value as? String else { return nil }
            return (Int(key) ?? 0, path)
        }.max { $0.0 < $1.0 }?.1
    }

    private static func jsLiteral(_ value: String) -> String {
        guard let data = try? JSONSerialization.data(withJSONObject: [value]),
              let text = String(data: data, encoding: .utf8) else { return "\"\"" }
        return String(text.dropFirst().dropLast())
    }

    /// Answers the directory chooser `installExtension` armed. Returns false for
    /// any other dialog, which goes to the platform panel as before. A chooser
    /// during an install with nothing parked is cancelled rather than shown,
    /// so `loadUnpacked` fails instead of putting a real panel over the app.
    func answerInstallDialog(_ token: UInt) -> Bool {
        guard installInFlight,
              let callback = UnsafeMutablePointer<cef_file_dialog_callback_t>(bitPattern: token) else { return false }
        guard let path = pendingInstallPath else {
            callback.pointee.cancel?(callback)
            return true
        }
        pendingInstallPath = nil
        let list = nd_cef_string_list_alloc()
        defer { nd_cef_string_list_free(list) }
        var entry = cef_string_t()
        ndCefSetString(path, &entry)
        nd_cef_string_list_append(list, &entry)
        nd_cef_string_clear(&entry)
        callback.pointee.cont?(callback, list)
        return true
    }

    func emitExtensionsChanged(_ payload: String) {
        let fields = (try? JSONSerialization.jsonObject(with: Data(payload.utf8))) as? [String: Any]
        emitData("extensionsChanged", [
            "reason": fields?["reason"] as? String ?? "changed",
            "extensionId": fields?["id"] as? String ?? "",
        ])
    }

    // MARK: - Removal

    /// `chrome.management.uninstall` from anything but the extension itself
    /// always puts up Chrome's "Remove ...?" dialog (management_api.cc forces
    /// it unless the caller is the target), and that dialog hangs off the
    /// toolbar the app never shows. So the extension removes itself:
    /// `uninstallSelf` in a hidden target on its manifest.json, a document
    /// every extension has, in no tab strip and no window. The app asks the
    /// person first.
    private func uninstallExtension(_ id: String, extensionID: String) {
        let once = NDCefOnce()
        let answer: @MainActor (String?) -> Void = { [weak self] error in
            guard let self, once.claim() else { return }
            if let error {
                self.emitData("extensionsList", ["id": id, "ok": false, "error": "uninstallExtension: \(error)"])
                return
            }
            self.awaitRemoval(id, extensionID: extensionID)
        }
        guard !extensionID.isEmpty, extensionID.allSatisfy({ ("a"..."p").contains($0) }) else {
            answer("not an extension id")
            return
        }
        guard NDCefBrowserProtocol.isAvailable else {
            answer("no browser protocol pipe in this process")
            return
        }
        Timer.scheduledTimer(withTimeInterval: Self.registryBudget, repeats: false) { _ in
            MainActor.assumeIsolated { answer("no answer from the browser within \(Int(Self.registryBudget)) s") }
        }
        NDCefBrowserProtocol.call(
            "Target.createTarget",
            ["url": "chrome-extension://\(extensionID)/manifest.json", "hidden": true, "background": true]
        ) { result, error in
            guard let target = result?["targetId"] as? String else {
                answer("no document of the extension to remove it from (\(error ?? "createTarget gave no target"))")
                return
            }
            Self.uninstallSelf(in: target, extensionID: extensionID, answer)
        }
    }

    /// A new target starts on about:blank, where `chrome.management` does not
    /// exist, so the call waits for the extension's own document to announce
    /// its context and runs there. The context goes away with the extension,
    /// so the uninstall itself is not awaited: the registry, read next, is
    /// what says it worked.
    private static func uninstallSelf(in target: String, extensionID: String, _ done: @escaping @MainActor (String?) -> Void) {
        NDCefBrowserProtocol.call("Target.attachToTarget", ["targetId": target, "flatten": true]) { result, error in
            guard let session = result?["sessionId"] as? String else {
                done("attach failed (\(error ?? "no session"))")
                return
            }
            let origin = "chrome-extension://\(extensionID)"
            NDCefBrowserProtocol.onEvents(sessionID: session) { method, params in
                guard method == "Runtime.executionContextCreated",
                      let context = params["context"] as? [String: Any],
                      context["origin"] as? String == origin,
                      let contextID = context["id"] as? Int else { return }
                NDCefBrowserProtocol.onEvents(sessionID: session, nil)
                NDCefBrowserProtocol.call(
                    "Runtime.evaluate",
                    [
                        "expression": "chrome.management.uninstallSelf({ showConfirmDialog: false }).catch(() => {}), 'sent'",
                        "contextId": contextID,
                        "returnByValue": true,
                        "userGesture": true,
                    ],
                    sessionID: session
                ) { result, _ in
                    let thrown = (result?["exceptionDetails"] as? [String: Any]).map { details in
                        ((details["exception"] as? [String: Any])?["description"] as? String) ?? (details["text"] as? String ?? "exception")
                    }
                    NDCefBrowserProtocol.call("Target.closeTarget", ["targetId": target]) { _, _ in }
                    done(thrown)
                }
            }
            NDCefBrowserProtocol.call("Runtime.enable", sessionID: session) { _, _ in }
        }
    }

    /// Answers with the registry once the extension has left it.
    private func awaitRemoval(_ id: String, extensionID: String) {
        let body = """
          const gone = async () => !JSON.parse(await listed()).some((e) => e.id === \(Self.jsLiteral(extensionID)));
          for (let i = 0; !(await gone()); i++) {
            if (i >= 100) throw new Error(\(Self.jsLiteral(extensionID)) + " is still installed");
            await new Promise((r) => setTimeout(r, 200));
          }
        """
        runJSON(id, event: "extensionsList", key: "extensions", what: "uninstallExtension",
                budget: Self.registryBudget, code: Self.mutation(body))
    }

    // MARK: - The click

    /// What Chrome does when its toolbar button is clicked: the popup when the
    /// action has one for this tab, otherwise `action.onClicked` (or
    /// `browserAction.onClicked`) with the tab, and an `activeTab` grant on it.
    /// `Extensions.triggerAction` runs that same `ExecuteUserAction`. It needs
    /// Chromium's ExtensionsContainer, which only a browser with a toolbar has
    /// (NDCefChromeWindow keeps one, hidden), and the view's tab target.
    private func triggerExtensionAction(_ id: String, extensionID: String) {
        let answer: (String?) -> Void = { [weak self] error in
            var payload: [String: Any] = ["id": id, "ok": error == nil, "triggered": extensionID]
            if let error { payload["error"] = error }
            self?.emitData("extensionActions", payload)
        }
        guard NDCefBrowserProtocol.isAvailable else {
            answer("triggerExtensionAction: no browser protocol pipe in this process")
            return
        }
        resolveTabTarget { tab, error in
            guard let tab else {
                answer("triggerExtensionAction: \(error ?? "no tab target")")
                return
            }
            NDCefBrowserProtocol.call("Extensions.triggerAction", ["id": extensionID, "targetId": tab]) { [weak self] _, error in
                // A stale id (the tab target went with its contents) is read
                // again on the next click.
                if error != nil { self?.tabTargetID = nil }
                answer(error.map { "triggerExtensionAction: \($0)" })
            }
        }
    }

    /// A top-level page target reports no parent (render_frame_devtools_agent_
    /// host.cc GetParentId), so the tab is found the way a tab-mode client
    /// finds it: a tab session with auto-attach names its page child. Tabs on
    /// this view's address go first; the answer is kept for the view's life.
    private func resolveTabTarget(_ done: @escaping (String?, String?) -> Void) {
        if let tabTargetID {
            done(tabTargetID, nil)
            return
        }
        devTools.call("Target.getTargetInfo") { [weak self] result, error in
            guard let info = result?["targetInfo"] as? [String: Any],
                  let page = info["targetId"] as? String else {
                done(nil, "page target unknown (\(error ?? "no targetInfo"))")
                return
            }
            let url = info["url"] as? String ?? ""
            NDCefBrowserProtocol.call("Target.getTargets", ["filter": [["type": "tab"]]]) { result, error in
                let tabs = (result?["targetInfos"] as? [[String: Any]] ?? [])
                    .sorted { ($0["url"] as? String == url ? 0 : 1) < ($1["url"] as? String == url ? 0 : 1) }
                    .compactMap { $0["targetId"] as? String }
                guard !tabs.isEmpty else {
                    done(nil, "no tab targets (\(error ?? "empty list"))")
                    return
                }
                Self.findTab(owning: page, among: tabs[...]) { tab in
                    if let tab { self?.tabTargetID = tab }
                    done(tab, tab == nil ? "no tab target owns page \(page)" : nil)
                }
            }
        }
    }

    private static func findTab(owning page: String, among tabs: ArraySlice<String>, _ done: @escaping (String?) -> Void) {
        guard let tab = tabs.first else {
            done(nil)
            return
        }
        let rest = tabs.dropFirst()
        NDCefBrowserProtocol.call("Target.attachToTarget", ["targetId": tab, "flatten": true]) { result, _ in
            guard let session = result?["sessionId"] as? String else {
                findTab(owning: page, among: rest, done)
                return
            }
            var owns = false
            NDCefBrowserProtocol.onEvents(sessionID: session) { method, params in
                guard method == "Target.attachedToTarget",
                      let child = params["targetInfo"] as? [String: Any] else { return }
                if child["targetId"] as? String == page { owns = true }
            }
            // Existing children are announced before the reply to setAutoAttach.
            NDCefBrowserProtocol.call(
                "Target.setAutoAttach",
                ["autoAttach": true, "waitForDebuggerOnStart": false, "flatten": true],
                sessionID: session
            ) { _, _ in
                NDCefBrowserProtocol.onEvents(sessionID: session, nil)
                NDCefBrowserProtocol.call("Target.detachFromTarget", ["sessionId": session]) { _, _ in }
                if owns { done(tab) } else { findTab(owning: page, among: rest, done) }
            }
        }
    }
}
/// A reply that may be settled by its answer or by its deadline, whichever
/// comes first.
@MainActor
final class NDCefOnce {
    private var done = false
    func claim() -> Bool {
        if done { return false }
        done = true
        return true
    }
}
#endif
