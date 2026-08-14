# Webview: browser-grade apps on native UI

`<webview>` embeds the platform's web engine as an ordinary widget: WKWebView
on macOS, WebKitGTK on Linux (dlopen'd at runtime, with a placeholder when
absent). The surface is deliberately browser-grade: full web browsers with native
chrome (tabs, toolbar, address bar as real native widgets) are a supported
target and the robustness baseline for the toolkit.

## React API

```tsx
import { useRef } from "react";
import {
  sendCommand,
  executeJavaScript,
  onJavaScriptResult,
  type NdNodeRef,
} from "@nativedesktop/react";

function BrowserTab({ url, onOpenTab }: { url: string; onOpenTab: (u: string) => void }) {
  const wv = useRef<NdNodeRef<"webview">>(null);
  return (
    <webview
      ref={wv}
      url={url}
      onNavigate={({ text }) => setAddressBar(text)}
      onTitleChanged={({ text }) => setTabTitle(text)}
      onLoadProgress={({ value }) => setProgress(value)}          // 0..1
      onLoadFailed={({ data }) => showErrorPage(data)}            // { url, error }
      onNewWindow={({ text }) => onOpenTab(text)}                 // target=_blank / window.open
      onDownloadRequested={({ data }) => download(data)}          // { url, suggestedFilename? }
      onJavaScriptResult={onJavaScriptResult}                     // wires the promise helper
    />
  );
}

// imperative commands via ref
sendCommand(wv.current!, "goBack");
sendCommand(wv.current!, "setZoom", 1.25);
sendCommand(wv.current!, "setUserAgent", "MyBrowser/1.0");        // "" resets to default
sendCommand(wv.current!, "openDevTools");

// JS round-trip (requires onJavaScriptResult prop above)
const ua = await executeJavaScript(wv.current!, "navigator.userAgent");
```

## Extension surface

```tsx
import { webviewEngine, executeJavaScript, getCookies, onCookiesResult } from "@nativedesktop/react";

// Custom schemes bind to a frozen engine configuration — register BEFORE the
// first <webview> mounts, then answer `schemeRequest` with `respondScheme`.
await webviewEngine.registerScheme("crx");

// User scripts, in the page's world or a named isolated one.
sendCommand(wv.current!, "addUserScript", {
  id: "content-script",
  source: "window.__ext = 1",
  injectionTime: "start",          // "start" | "end" (default)
  world: "ext",                     // omit for the page's own world
  allFrames: true,
  allowList: ["https://*.example.com/*"],
});
sendCommand(wv.current!, "removeUserScript", { id: "content-script" });
sendCommand(wv.current!, "clearUserScripts", { world: "ext" });

// Page -> app messages: window.webkit.messageHandlers.bridge.postMessage(v)
sendCommand(wv.current!, "registerScriptMessage", { name: "bridge", world: "ext" });

// World-scoped eval reads what the isolated script stored.
const value = await executeJavaScript(wv.current!, "window.__ext", "ext");

// Cookies on the view's own profile.
const cookies = await getCookies(wv.current!, "https://example.com/");
sendCommand(wv.current!, "setCookie", { name: "a", value: "1", domain: "example.com", path: "/" });
sendCommand(wv.current!, "deleteCookie", { name: "a", domain: "example.com", path: "/" });
```

Create-only props: `profile` (`""` = shared default, `private…` = ephemeral, any other name = its
own persistent partition) and `suppressContextMenu` (the engine menu is suppressed and the app
shows a native one off the `contextMenu` event).

## Event semantics

`loadProgress` carries `{ value }`, an estimated 0..1 load progress, emitted on
change only.

`loadFailed` carries `{ data: { url, error } }` when a navigation fails. `url`
is the *failing* URL, not the page navigated away from. Cancellations and
policy-interruption noise are filtered on both backends, and blocked-port loads
(which WebKit silently turns into `about:blank`) are detected and reported.

`newWindow` carries `{ text }` when the page requests a new window through
`window.open` or `target=_blank`. The host always denies the native popup and
lets the app decide what a new window means, usually a native tab.

`downloadRequested` carries `{ data: { url, suggestedFilename? } }` when the
engine hits a response it cannot render, or an attachment. The engine-side
download is cancelled and the app performs it. The Bun process has full network
and filesystem access, so `fetch` with `node:fs` and `getAppDataDir()` is the
intended path. `data:` URLs report `suggestedFilename: "Unknown"`.

The event fires ONCE per download, on the view that asked for it, however many
views are alive. That is worth stating because the signal it rides on GTK
(`download-started`) lives on the network session, which every view without a
`profile` shares — the report is routed by the download's own originating view,
not by which views happen to exist.

`javaScriptResult` carries `{ data: { id, ok, value?, error? } }` when
`executeJavaScript` completes. Apps normally never touch it: pass the exported
`onJavaScriptResult` handler and use the promise helper. `cookiesResult` and
`sessionSaved` follow the same id-correlated pattern behind `getCookies` and
`saveSession`.

`scriptMessage` carries `{ data: { name, world, body } }`. `body` is the posted
value already decoded, so an object arrives as an object.

`schemeRequest` carries `{ data: { id, url, scheme } }` on the view that made
the request. Answer it with `respondScheme` (`{ id, base64, mime, status? }`, or
`{ id, error }` to fail it); an unanswered id leaves the page waiting.

`faviconChanged` carries `{ dataUrl }` on GTK (WebKit keeps a favicon database
and the icon is encoded to a PNG data URL, capped at 48 KB of PNG) and
`{ pageUrl, iconUrl }` on macOS, where WKWebView has no favicon API and the
app fetches the bytes itself. Handle both.

`findResult` carries `{ matchFound, matchCount?, done }`. `done: false` is a
match-count update, which only WebKitGTK produces; `WKFindResult` reports
match/no-match with no total, so `matchCount` is optional.

`securityChanged` carries `{ secure, insecureContent, url?, error? }` on
navigation commit, on mixed-content detection, and on a TLS failure. TLS errors
are never auto-accepted: the load fails and the event carries the reason.

`linkHover` (`{ text }`, empty on clear), `contextMenu`
(`{ data: { x, y, link?, image?, selection?, hasSelection, editable } }`) and
`audioStateChanged` (`{ data: { playing, muted } }`) are native WebKit signals
on GTK. WKWebView exposes none of them, so on macOS they are observed in the
page by the framework's own user script in a private world (`nd-internal`,
handler `__ndInternal`) — those two names are reserved. `selection` carries the
selected text only on macOS; WebKitGTK's hit test reports `hasSelection` alone.

## Command notes

`executeJavaScript` takes `{ id, code, world? }`. Prefer the promise helper
(`executeJavaScript(node, code, world?)`). JS exceptions reject with the real
message, for example `Error: boom`.

`addUserScript` is a keyed registry: re-adding an `id` replaces it. GTK removes
a script by identity through `WebKitUserContentManager`; WKUserContentController
can only clear everything, so the AppKit side replays the surviving set on each
mutation. `allowList`/`blockList` are native on GTK and compiled into a guard
around the source on macOS, which WebKit gives no other way to express.

`getCookies`/`setCookie`/`deleteCookie` act on the view's own profile.
Deletion matches by name plus whichever of domain/path is given, and both
backends read the live cookie first — each engine's jar deletes by identity, so
a synthesized cookie never matches.

`setZoom` takes a number, the page zoom factor. `setUserAgent` takes a string,
and an empty string restores the engine default.

`openDevTools` opens the WebKit inspector window on GTK. macOS has no
programmatic open, so it sets `isInspectable` and the inspector attaches through
Safari's Develop menu.

`focus` puts the keyboard focus in the view. It is the cross-cutting widget
command — `<button>`, `<textinput>`, `<textarea>` and `<searchinput>` take it
too — and it is the only way to move focus programmatically, since neither
backend synthesises input for automation on Linux.

Adding a new webview *event* needs one-line routing entries in `tools/codegen.ts`
(`SIGNALS` and `SWIFT_SIGNALS`) plus the schema. New *commands* are schema-only:
dispatch forwards the raw command string to the hand-written engine files,
`src/gtk/webview.zig` and `NDShell/NDWebView.swift`.

## Engines

The system engine is the default on both platforms and costs zero bundle bytes.
CEF (Chromium) is the planned opt-in alternative for apps that need Chromium
fidelity, enabled per project *and* per platform in `nativedesktop.config.ts`
(e.g. macOS on WebKit, Linux on CEF).

The opt-in is structural rather than a runtime flag:

- CEF is never linked into the host binary. It loads at runtime
  (`cef_load_library` on macOS, `dlopen` on Linux) from the app bundle, behind
  the same `<webview>` contract, so app code does not change with the engine.
- Packaging stages the CEF framework/helpers only when the config enables it.
  An app that doesn't enable CEF ships zero Chromium bytes: the CEF
  distribution is fetched at package time (official builds:
  cef-builds.spotifycdn.com) and is never committed to the repo.
- On Linux, `libcef.so` ships largely unstripped (>1 GB on disk); the packaging
  step must run `strip` when CEF is enabled (~200 MB after).
- macOS additionally requires helper `.app` bundles (Renderer/GPU/Alerts/main)
  under `Contents/Frameworks` and inside-out codesigning. Both are packaging
  concerns, invisible to app code.
- Linux caveat: CEF windowed embedding under native Wayland is still unshipped
  upstream (CEF issue #2804; ANGLE's Wayland support merged 2026-05); until it
  lands, embedded CEF on Wayland means off-screen rendering or XWayland.

## Verification

`scripts/headless-webview.sh` runs `examples/webview-probe` under weston and
drives it with `scripts/webview-drive.ts` (marker `ND_WEBVIEW2_OK`). The probe
hosts its own HTTP fixture and answers its own custom scheme, so the whole
round trip stays in one process. The same drive script runs against the AppKit
host directly.

GTK4 removed app-constructible input events, so the automation socket cannot
synthesize a pointer move or a right-click: `linkHover` and `contextMenu` report
`skip` on GTK and are runtime-verified on AppKit, where they ride the page-side
agent and a JS-dispatched event exercises the whole path. Everything else is
runtime-verified on both backends.

Status: the extended `<webview>` API above is implemented and runtime-verified
on both backends. CEF integration exists as a macOS proof-of-concept via the
native-plugin seam and is not yet part of the framework.
