# CEF changes nd needs upstream

nd takes CEF as the prebuilt minimal distribution `cef-dll-sys` downloads
(`cef_binary_154.0.34+g14c5a08+chromium-154.0.8037.98`, the Spotify builds). nd neither builds
nor patches CEF, so a change to CEF's own code reaches nd only through a CEF release.

## The two-second hold on contents that are not CEF browsers

What nd sees: an extension's action popup (and every hidden page nd opens through the DevTools
pipe: the `chrome://extensions` registry and the `manifest.json` probes) draws nothing for two
seconds after it is created. The gate
`real_extensions_cef_chrome` counts it in its `popup_ms`.

Why: every new `WebView` in a renderer asks the browser process for its CEF browser
synchronously (`CefRenderManager::MaybeCreateBrowser`, `GetNewBrowserInfo`, in
`libcef/renderer/render_manager.cc`). The browser answers at once for a frame of a CEF browser
and for an excluded frame (`CefBrowserInfoManager::IsExcludedFrameHost`: the PDF process,
WebUI preload contents, `kUIElement` WebUI without a browser, guests, print preview). Anything
else waits for a CEF browser to claim it, which for Chromium's own extension hosts never
happens, until `kNewBrowserInfoResponseTimeoutMs` (2000 ms) in
`libcef/browser/browser_info_manager.cc` answers `browser_id = -1`. The renderer then treats the
view as excluded, the same as it would have at once.

What nd can do without a CEF change: nothing that keeps the popup Chrome's.

- No `CefSettings` field or preference controls the wait. The one switch,
  `--disable-new-browser-info-timeout` (present in 154.0.34), removes the timeout, so these
  renderers would wait forever.
- The popup is created by Chrome's `ExtensionPopup` (`ExtensionViewHost`); no CEF callback runs
  for it, so nd cannot create it as a CEF browser. Opening the popup page in a browser of nd's
  own would make it a tab, not the action's popup: `activeTab`, `currentWindow` and
  `window.close()` would mean the wrong thing.
- The hidden pages could be CEF browsers. An install by id already runs in one (a windowless
  browser, `store_browser.rs`), because Chromium needs a CEF browser to download the package.
  The registry and the probes need `chrome://extensions` and an extension's own pages, which
  only Chrome style has, and Chrome style needs a Views window per page: that trades the two
  seconds for a window per hidden page.

The change, in `CefBrowserInfoManager::IsExcludedFrameHost`
(`libcef/browser/browser_info_manager.cc`), before the final `return false`:

```cpp
  // Extension hosts that Chrome creates itself (action popups, background
  // pages, offscreen documents, side panels) never get a CefBrowserHost, so
  // their renderer must not wait for one.
  switch (extensions::GetViewType(web_contents)) {
    case extensions::mojom::ViewType::kExtensionPopup:
    case extensions::mojom::ViewType::kExtensionBackgroundPage:
    case extensions::mojom::ViewType::kOffscreenDocument:
    case extensions::mojom::ViewType::kExtensionSidePanel:
      return !CefBrowserHostBase::GetBrowserForHost(rfh);
    default:
      break;
  }
```

with `#include "extensions/browser/view_type_utils.h"` (the file already depends on
`//extensions/browser` through `browser_guest_util.h` in builds with extensions enabled).
`CheckExcludedNewBrowserInfoOnUIThread` already runs `IsExcludedFrameHost` right after the
request arrives, so these frames get their answer in the same task instead of two seconds
later; nothing else changes, since the renderer ends up with a `CefExcludedView` either way.

Hidden DevTools targets (`Target.createTarget` with `hidden: true`) carry no view type of their
own; covering them needs a marker on the contents `ChromeDevToolsManagerDelegate` creates, which
is a Chromium change, not a CEF one. nd opens those at startup and once per extension, off the
path a click waits on, so the popup is the one that matters.

Where to file: the CEF issue tracker (github.com/chromiumembedded/cef/issues), as a bug with
the timing (popup visible about 2 s after `ExecuteUserAction`, the `LOG(ERROR) Timeout of new
browser info response` line in the browser log) and this diff as a PR.

On Linux nd shows an extension's popup in a page of its own (`run_action_in_view` in
`crates/nd-cef/src/extensions.rs`), which is a CEF browser, so the hold applies to macOS only.

### The timeout logged at every Chrome-style launch

Every launch with a Chrome-style page logs one `Timeout of new browser info response` about two
seconds after the first page is created, with no extension involved (seen in examples/browser
on macOS and in Lynk Browser on Linux). It is not the page itself: a test page's script ran
20 ms after its request. It is not Chromium's omnibox popup WebUI either, nor nd's hidden
`chrome://extensions` page: turning the first off and timing the second left it in place. No
DevTools target carries the frame's token, so it is contents Chromium makes alongside the
browser window, and nothing on screen waits on it. nd leaves it as is.
