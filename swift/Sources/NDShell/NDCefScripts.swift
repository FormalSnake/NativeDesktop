#if canImport(CCef)
import AppKit
import CCef
import Foundation

/// One entry of the app's `addUserScript` registry, kept in insertion order.
/// CDP has no "replace one script" call any more than WKUserContentController
/// does, so every mutation replays the whole set.
struct NDCefUserScript {
    let id: String
    let source: String
    let world: String
    let atStart: Bool
    let allFrames: Bool
}

/// A script-message channel is a name AND a world: the same name may live in
/// two worlds at once, and removing one must not take the other with it.
struct NDCefMessageChannel: Hashable {
    let name: String
    let world: String
}

extension NDCefWebView {
    /// The private world the framework's own page-side agent lives in, and the
    /// binding every world posts through. Both are namespaced so an app and the
    /// framework can never collide on a handler name, the same contract the
    /// WebKit surface documents.
    static let internalWorldName = "nd-internal"
    static let internalHandlerName = "__ndInternal"
    static let bindingName = "__ndPost"

    // MARK: - Document-start scripts

    /// Installs everything this view injects, once, when the browser appears.
    /// After that each mutation touches only its own key: CDP hands identifiers
    /// back asynchronously, so replaying the whole set on every `addUserScript`
    /// would race its own in-flight installs and leave duplicates behind.
    func ndInstallScripts() {
        guard hasBrowser else { return }
        installInternalAgent()
        for world in Set(messageChannels.map(\.world)) { installShim(for: world) }
        for script in userScripts { installUserScript(script) }
    }

    func installInternalAgent() {
        guard hasBrowser else { return }
        bindWorld(Self.internalWorldName)
        install(
            key: "__internal",
            source: Self.internalAgentSource(suppressContextMenu: suppressContextMenu),
            world: Self.internalWorldName
        )
    }

    private func installUserScript(_ script: NDCefUserScript) {
        guard hasBrowser else { return }
        var source = Self.mainFrameGuard(script)
        if !script.atStart {
            // CDP injects at document start only. Deferring inside the same
            // script is the closest equivalent to WKUserScript's
            // .atDocumentEnd, and it keeps ordering relative to other scripts.
            source = """
            (function () {
              var run = function () { \(source) };
              if (document.readyState === "loading") {
                document.addEventListener("DOMContentLoaded", run, { once: true });
              } else {
                run();
              }
            })();
            """
        }
        install(key: "user:\(script.id)", source: source, world: script.world)
    }

    private func install(key: String, source: String, world: String) {
        guard hasBrowser else { return }
        removeScript(key)
        let generation = scriptGenerations[key, default: 0]
        var params: [String: Any] = ["source": source, "runImmediately": true]
        if !world.isEmpty { params["worldName"] = world }
        devTools.call("Page.addScriptToEvaluateOnNewDocument", params) { [weak self] result, error in
            guard let self else { return }
            guard let identifier = result?["identifier"] as? String else {
                ndCefWarn("addScriptToEvaluateOnNewDocument(\(key)) failed: \(error ?? "no identifier")")
                return
            }
            guard self.scriptGenerations[key, default: 0] == generation else {
                // Superseded while the install was in flight; drop it rather
                // than letting two copies of the same script run.
                self.devTools.call("Page.removeScriptToEvaluateOnNewDocument", ["identifier": identifier])
                return
            }
            self.scriptIdentifiers[key] = identifier
        }
    }

    private func removeScript(_ key: String) {
        scriptGenerations[key, default: 0] += 1
        guard let identifier = scriptIdentifiers.removeValue(forKey: key) else { return }
        devTools.call("Page.removeScriptToEvaluateOnNewDocument", ["identifier": identifier])
    }

    /// WKUserScript's forMainFrameOnly has no CDP equivalent (every frame gets
    /// the script), so the restriction is expressed in the page.
    private static func mainFrameGuard(_ entry: NDCefUserScript) -> String {
        guard !entry.allFrames else { return entry.source }
        return """
        (function () {
          if (window.top !== window.self) return;
        \(entry.source)
        })();
        """
    }

    func ndAddUserScript(_ obj: [String: Any]) {
        guard let id = obj["id"] as? String, let source = obj["source"] as? String else {
            ndCefWarn("addUserScript: id and source are required")
            return
        }
        let guarded = Self.applyURLLists(
            source,
            allowList: obj["allowList"] as? [String] ?? [],
            blockList: obj["blockList"] as? [String] ?? []
        )
        let entry = NDCefUserScript(
            id: id,
            source: guarded,
            world: obj["world"] as? String ?? "",
            atStart: (obj["injectionTime"] as? String) == "start",
            allFrames: (obj["allFrames"] as? NSNumber)?.boolValue ?? false
        )
        userScripts.removeAll { $0.id == id }
        userScripts.append(entry)
        installUserScript(entry)
    }

    func ndRemoveUserScript(_ id: String) {
        userScripts.removeAll { $0.id == id }
        removeScript("user:\(id)")
    }

    func ndClearUserScripts(world: String?) {
        let doomed = userScripts.filter { world == nil || world!.isEmpty || $0.world == world! }
        userScripts.removeAll { entry in doomed.contains { $0.id == entry.id } }
        for entry in doomed { removeScript("user:\(entry.id)") }
    }

    private static func applyURLLists(_ source: String, allowList: [String], blockList: [String]) -> String {
        if allowList.isEmpty && blockList.isEmpty { return source }
        return """
        (function(){
          var __ndAllow = \(jsonArrayLiteral(allowList)), __ndBlock = \(jsonArrayLiteral(blockList));
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
              let text = String(data: data, encoding: .utf8) else { return "[]" }
        return text
    }

    // MARK: - Script messages

    /// `window.webkit.messageHandlers.<name>.postMessage(body)` is the API both
    /// engines expose, because it is what page scripts and the extension broker
    /// already speak. On Chromium it is a shim over one `Runtime.addBinding`
    /// per world.
    private func installShim(for world: String) {
        guard hasBrowser else { return }
        let names = messageChannels.filter { $0.world == world }.map(\.name)
        guard !names.isEmpty else { return }
        bindWorld(world)
        let source = """
        (function () {
          window.webkit = window.webkit || {};
          window.webkit.messageHandlers = window.webkit.messageHandlers || {};
          var post = function (name, body) {
            try { window.\(Self.bindingName)(JSON.stringify({ n: name, w: \(Self.jsString(world)), b: body })); } catch (e) {}
          };
          \(Self.jsonArrayLiteral(names)).forEach(function (name) {
            window.webkit.messageHandlers[name] = {
              postMessage: function (body) { post(name, body); },
            };
          });
        })();
        """
        install(key: "shim:\(world)", source: source, world: world)
    }

    private func bindWorld(_ world: String) {
        guard !boundWorlds.contains(world) else { return }
        boundWorlds.insert(world)
        var params: [String: Any] = ["name": Self.bindingName]
        if !world.isEmpty { params["executionContextName"] = world }
        devTools.call("Runtime.addBinding", params)
    }

    func ndRegisterScriptMessage(_ obj: [String: Any]) {
        guard let name = obj["name"] as? String, !name.isEmpty else {
            ndCefWarn("registerScriptMessage: missing name")
            return
        }
        let world = obj["world"] as? String ?? ""
        messageChannels.insert(NDCefMessageChannel(name: name, world: world))
        installShim(for: world)
    }

    func ndUnregisterScriptMessage(_ obj: [String: Any]) {
        guard let name = obj["name"] as? String else { return }
        let world = obj["world"] as? String ?? ""
        messageChannels.remove(NDCefMessageChannel(name: name, world: world))
        if messageChannels.contains(where: { $0.world == world }) {
            installShim(for: world)
        } else {
            removeScript("shim:\(world)")
        }
    }

    /// Every binding call from every world arrives here. The envelope carries
    /// the handler name and world so one binding serves them all.
    func handleBindingPayload(_ payload: String) {
        guard let data = payload.data(using: .utf8),
              let envelope = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let name = envelope["n"] as? String else { return }
        let world = envelope["w"] as? String ?? ""
        let body = envelope["b"] ?? NSNull()
        if name == Self.internalHandlerName {
            ndHandleInternalMessage(body)
            return
        }
        emitData("scriptMessage", ["name": name, "world": world, "body": body])
    }

    // MARK: - Evaluate

    func ndExecuteJavaScript(_ obj: [String: Any]) {
        guard let id = obj["id"] as? String, let code = obj["code"] as? String else {
            ndCefWarn("malformed executeJavaScript arg")
            return
        }
        devTools.evaluate(code, world: obj["world"] as? String ?? "") { [weak self] value, error in
            var fields: [String: Any] = ["id": id]
            if let error {
                fields["ok"] = false
                fields["error"] = error
            } else {
                fields["ok"] = true
                if let value { fields["value"] = value }
            }
            self?.emitData("javaScriptResult", fields)
        }
    }

    // MARK: - The framework's own page-side agent

    /// Hovered link, context-menu hit test and media state: three things
    /// neither engine reports natively in a shape the widget can use, observed
    /// in the page and posted back over the private world. Kept in step with
    /// the WebKit surface's own agent, which this is a port of.
    private static func internalAgentSource(suppressContextMenu: Bool) -> String {
        """
        (function () {
          var post = function (m) {
            try { window.\(bindingName)(JSON.stringify({ n: \(jsString(internalHandlerName)), w: \(jsString(internalWorldName)), b: m })); } catch (e) {}
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
          var suppress = \(suppressContextMenu ? "true" : "false");
          window.__ndSetSuppressMenu = function (v) { suppress = !!v; };
          document.addEventListener("contextmenu", function (e) {
            var el = e.target;
            var editable = !!(el && (el.isContentEditable || el.tagName === "INPUT" || el.tagName === "TEXTAREA"));
            var image = (el && el.tagName === "IMG" && el.src) ? el.src : "";
            var sel = String(window.getSelection ? window.getSelection() : "");
            post({ k: "menu", x: e.clientX, y: e.clientY, link: linkOf(el), image: image, selection: sel, editable: editable });
            if (suppress) e.preventDefault();
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
            document.addEventListener(name, function (event) {
              if (muted && event && event.target && "muted" in event.target) event.target.muted = true;
              report();
            }, true);
          });
        })();
        """
    }

    func ndHandleInternalMessage(_ body: Any) {
        guard let message = body as? [String: Any], let kind = message["k"] as? String else { return }
        switch kind {
        case "hover":
            lastHoveredLink = message["url"] as? String ?? ""
            emitText("linkHover", lastHoveredLink)
        case "menu":
            lastMenuHit = NDContextMenuHit(
                link: message["link"] as? String ?? "",
                image: message["image"] as? String ?? "",
                selection: message["selection"] as? String ?? "",
                editable: (message["editable"] as? NSNumber)?.boolValue ?? false,
                at: Date()
            )
            emitData("contextMenu", [
                "x": (message["x"] as? NSNumber)?.intValue ?? 0,
                "y": (message["y"] as? NSNumber)?.intValue ?? 0,
                "link": lastMenuHit.link,
                "image": lastMenuHit.image,
                "selection": lastMenuHit.selection,
                "hasSelection": !lastMenuHit.selection.isEmpty,
                "editable": lastMenuHit.editable,
            ])
        case "audio":
            emitData("audioStateChanged", [
                "playing": (message["playing"] as? NSNumber)?.boolValue ?? false,
                "muted": (message["muted"] as? NSNumber)?.boolValue ?? false,
            ])
        default:
            break
        }
    }

    static func jsString(_ value: String) -> String {
        guard let data = try? JSONSerialization.data(withJSONObject: [value]),
              let text = String(data: data, encoding: .utf8) else { return "\"\"" }
        return String(text.dropFirst().dropLast())
    }

    // MARK: - Context menu

    /// Chromium's own menu stays, and the app's matching items are appended
    /// after a separator, which is what `native` mode means on both engines.
    /// The hit comes from the page-side agent above, since the params CEF
    /// hands the handler are not available by the time this runs on the main
    /// thread.
    func ndAppendContextMenuItems(_ modelToken: UInt) {
        contextMenuCommands.removeAll()
        guard !contextMenuItems.isEmpty,
              let raw = UnsafeMutableRawPointer(bitPattern: modelToken) else { return }
        let model = raw.assumingMemoryBound(to: cef_menu_model_t.self)
        var hit = lastMenuHit
        if Date().timeIntervalSince(hit.at) > 2 {
            hit = NDContextMenuHit()
            hit.link = lastHoveredLink
            hit.at = Date()
        }
        let survivors = contextMenuItems.filter { $0.kind != .separator && $0.survives(hit) }
        guard !survivors.isEmpty else { return }
        model.pointee.add_separator?(model)
        append(contextMenuItems, to: model, hit: hit)
    }

    private func append(_ items: [NDContextMenuItem], to model: UnsafeMutablePointer<cef_menu_model_t>, hit: NDContextMenuHit) {
        var appended = 0
        var pendingSeparator = false
        for item in items {
            if item.kind == .separator {
                if appended > 0 { pendingSeparator = true }
                continue
            }
            guard item.survives(hit) else { continue }
            if pendingSeparator {
                model.pointee.add_separator?(model)
                pendingSeparator = false
            }
            var label = cef_string_t()
            ndCefSetString(item.label, &label)
            defer { nd_cef_string_clear(&label) }
            if !item.children.isEmpty {
                let commandID = nextContextMenuCommandID()
                if let submenu = model.pointee.add_sub_menu?(model, commandID, &label) {
                    append(item.children, to: submenu, hit: hit)
                    nd_cef_ref_release(submenu)
                }
            } else {
                let commandID = nextContextMenuCommandID()
                contextMenuCommands[commandID] = item
                if item.kind == .checkbox || item.kind == .radio {
                    _ = model.pointee.add_check_item?(model, commandID, &label)
                    _ = model.pointee.set_checked?(model, commandID, item.checked ? 1 : 0)
                } else {
                    _ = model.pointee.add_item?(model, commandID, &label)
                }
                if !item.enabled { _ = model.pointee.set_enabled?(model, commandID, 0) }
            }
            appended += 1
        }
    }

    private func nextContextMenuCommandID() -> Int32 {
        let id = nextContextMenuCommand
        nextContextMenuCommand += 1
        return id
    }

    /// The framework reports the new state a checkbox or radio click implies
    /// and does NOT mutate its own copy of the tree: the app owns the model and
    /// answers with the next `setContextMenuItems`.
    func ndContextMenuCommand(_ commandID: Int32) -> Bool {
        guard let item = contextMenuCommands[commandID] else { return false }
        var payload: [String: Any] = [
            "id": item.id,
            "pageUrl": ndPageState.url ?? "",
            "editable": lastMenuHit.editable,
        ]
        if !lastMenuHit.link.isEmpty { payload["linkUrl"] = lastMenuHit.link }
        if !lastMenuHit.image.isEmpty { payload["imageUrl"] = lastMenuHit.image }
        if !lastMenuHit.selection.isEmpty { payload["selectionText"] = lastMenuHit.selection }
        switch item.kind {
        case .checkbox:
            payload["checked"] = !item.checked
            payload["wasChecked"] = item.checked
        case .radio:
            payload["checked"] = true
            payload["wasChecked"] = item.checked
        default:
            break
        }
        emitData("contextMenuItemClicked", payload)
        return true
    }
}
#endif
