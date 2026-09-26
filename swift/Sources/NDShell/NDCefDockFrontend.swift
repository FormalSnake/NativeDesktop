#if canImport(CCef)
import CCef
import Foundation

/// The docked inspector's frontend, as a source of geometry.
///
/// Chrome's inspector does not sit beside the page: the frontend fills the
/// browser's whole contents area and keeps a hole in its own layout for the
/// page, which the browser then draws into that rectangle. The frontend
/// announces the hole with `setInspectedPageBounds` every time it moves: the
/// user drags the splitter, the dock side changes, device mode turns on (the
/// rectangle is then the device's, which is how the phone is drawn inside the
/// page area).
///
/// CEF has no seam for that message: `CefDevToolsFrontend` answers a handful of
/// embedder methods and drops the rest, and the inspector's browser belongs to
/// CEF's own client, so no handler of ours ever sees it. The frontend is asked
/// for it instead, over its own DevTools protocol: a patch on its channel to
/// the embedder forwards every announcement into a `Runtime` binding, which
/// arrives back here as a protocol event.
@MainActor final class NDCefDockFrontend {
    /// Installed at document start and once on the live document. The frontend
    /// is a devtools:// page, so `DevToolsHost` is Chromium's own injection and
    /// is there before any frontend script runs; the retry covers the ordering
    /// not being contractual.
    private static let hook = """
    (() => {
      const send = (rect, tries) => {
        if (typeof window.ndDockBounds === 'function') { window.ndDockBounds(JSON.stringify(rect)); return; }
        if (tries < 100) setTimeout(() => send(rect, tries + 1), 50);
      };
      const install = (tries) => {
        const host = window.DevToolsHost;
        if (!host || !host.sendMessageToEmbedder) { if (tries < 100) setTimeout(() => install(tries + 1), 50); return; }
        if (host.__ndDockHook) return;
        const original = host.sendMessageToEmbedder.bind(host);
        host.sendMessageToEmbedder = (message) => {
          try {
            const parsed = typeof message === 'string' ? JSON.parse(message) : message;
            if (parsed && parsed.method === 'setInspectedPageBounds') send(parsed.params[0], 0);
          } catch (error) {}
          return original(message);
        };
        host.__ndDockHook = true;
        // A frontend that laid out before the hook went in has announced its
        // hole already and says nothing more until it moves, and a synthetic
        // resize does not move it. Its own placeholder is asked instead; the
        // module is the one the frontend already loaded, so this is the live
        // instance, and one that is not showing yet announces on its own.
        import(new URL('panels/emulation/emulation.js', location.href).href).then((module) => {
          const placeholder = module.InspectedPagePlaceholder.InspectedPagePlaceholder.instance();
          if (placeholder.isShowing()) placeholder.update(false);
        }).catch(() => {});
      };
      install(0);
    })()
    """

    private weak var window: NDCefChromeWindow?
    private var host: UnsafeMutablePointer<cef_browser_host_t>?
    private var registration: UnsafeMutablePointer<cef_registration_t>?
    private var nextMessageID: Int32 = 1
    private var handshake: Int32 = 0

    init(window: NDCefChromeWindow) {
        self.window = window
    }

    var isRunning: Bool { host != nil }

    /// `browserHost` is the inspector's own browser, not the page's. The
    /// observer has to be attached before the first method call, and nothing
    /// carrying parameters may go out until the agent has answered one that
    /// does not: a params dictionary handed to an unattached agent takes the
    /// process down.
    func start(host browserHost: UnsafeMutablePointer<cef_browser_host_t>,
               observer: UnsafeMutablePointer<cef_dev_tools_message_observer_t>?) {
        guard self.host == nil, let observer else { return }
        nd_cef_ref_add(browserHost)
        host = browserHost
        registration = browserHost.pointee.add_dev_tools_message_observer?(browserHost, ndCefHandOut(observer))
        handshake = send("Runtime.enable", nil)
    }

    func stop() {
        nd_cef_ref_release(registration)
        registration = nil
        nd_cef_ref_release(host)
        host = nil
        handshake = 0
    }

    func handleResult(id: Int32, ok: Bool) {
        guard id == handshake, handshake != 0 else { return }
        handshake = 0
        guard ok else { return }
        _ = send("Runtime.addBinding", ["name": "ndDockBounds"])
        _ = send("Page.enable", nil)
        _ = send("Page.addScriptToEvaluateOnNewDocument", ["source": Self.hook])
        _ = send("Runtime.evaluate", ["expression": Self.hook])
    }

    func handleEvent(method: String, json: String) {
        guard method == "Runtime.bindingCalled" else { return }
        guard let params = ndCefParseJSONText(json),
              params["name"] as? String == "ndDockBounds",
              let payload = params["payload"] as? String,
              let rect = Self.rect(from: payload) else { return }
        window?.pageBoundsAnnounced(rect)
    }

    private static func rect(from payload: String) -> cef_rect_t? {
        guard let object = ndCefParseJSONText(payload),
              let x = object["x"] as? NSNumber, let y = object["y"] as? NSNumber,
              let width = object["width"] as? NSNumber, let height = object["height"] as? NSNumber
        else { return nil }
        return cef_rect_t(x: x.int32Value, y: y.int32Value, width: width.int32Value, height: height.int32Value)
    }

    @discardableResult
    private func send(_ method: String, _ params: [String: String]?) -> Int32 {
        guard let host else { return 0 }
        let id = nextMessageID
        nextMessageID += 1
        var name = cef_string_t()
        ndCefSetString(method, &name)
        defer { nd_cef_string_clear(&name) }
        var dictionary: UnsafeMutablePointer<cef_dictionary_value_t>?
        if let params, !params.isEmpty {
            dictionary = nd_cef_dict_create()
            for (key, value) in params {
                var slot = cef_string_t()
                var text = cef_string_t()
                ndCefSetString(key, &slot)
                ndCefSetString(value, &text)
                _ = dictionary?.pointee.set_string?(dictionary, &slot, &text)
                nd_cef_string_clear(&slot)
                nd_cef_string_clear(&text)
            }
        }
        // The library takes the params reference, the same hand-over contract
        // as every other capi pass-in here.
        if host.pointee.execute_dev_tools_method?(host, id, &name, dictionary) == 0 {
            ndCefWarn("dock frontend: execute_dev_tools_method(\(method)) was refused")
            return 0
        }
        return id
    }
}
#endif
