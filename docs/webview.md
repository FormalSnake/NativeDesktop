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

Create-only prop: `profile` (`""` = shared default, `private…` = ephemeral, any other name = its
own persistent partition).

## Context menus

`contextMenuMode` (create-and-update) decides who owns the menu:

- `"native"` (default): the engine's own menu opens, with everything a browser
  is expected to have: Back/Forward/Reload, Open Link, Copy Image, spell-check
  and the system text services on macOS, Inspect Element wherever developer
  extras are on. The app's items are appended to it after a separator.
- `"suppress"`: no engine menu opens at all; the `contextMenu` event fires and
  the app builds whatever it likes.

The `contextMenu` event fires in **both** modes: the hit test is read either
way, so knowing about the click costs nothing.

App items are declared per view and stored by the host until they are replaced:

```tsx
import { setContextMenuItems } from "@nativedesktop/react";

setContextMenuItems(wv.current!, [
  { id: "open-link", label: "Open Link in New Tab", contexts: ["link"] },
  { id: "save-image", label: "Save Image", contexts: ["image"] },
  { type: "separator" },
  {
    id: "tools",
    label: "Tools",
    contexts: ["all"],
    children: [
      { id: "tools-a", label: "Alpha" },
      { id: "tools-dark", label: "Dark Mode", type: "checkbox", checked: true },
    ],
  },
]);
```

| Field | Meaning |
| --- | --- |
| `id` | echoed back on `contextMenuItemClicked`; omit for a separator |
| `label` | the item's text; omit for a separator |
| `type` | `normal` (default), `checkbox`, `radio`, `separator` |
| `checked` | drawn state for `checkbox`/`radio` |
| `enabled` | `false` renders the item insensitive |
| `contexts` | subset of `page`, `link`, `image`, `selection`, `editable`, or `all`; defaults to `["page"]` |
| `targetUrlGlobs` | `*`-wildcard globs matched against the link or image URL under the pointer |
| `children` | a submenu; not depth-limited |

Which items a click earns is decided per invocation against that click's hit
test. `page` means the click landed on nothing more specific: a link, image,
selection or editable field wins over it, the way a browser's own menu behaves.
A submenu whose every child was filtered out is dropped rather than shown empty,
and a separator is only drawn when something survived on both sides of it.

`targetUrlGlobs` is deliberately NOT Chrome's match-pattern grammar: the
framework has no business knowing what an extension is. A caller holding
`targetUrlPatterns` passes them through as globs (the strings are shaped the
same, `*://*.example.com/*`) and re-checks the click against its own pattern
engine when it needs exactness, since a plain glob can also match a URL that
merely contains the pattern's text.

Contiguous `radio` siblings form one group, which is what makes GTK draw radios
rather than checkmarks.

`contextMenuItemClicked` carries
`{ id, pageUrl, linkUrl?, imageUrl?, selectionText?, editable, checked?, wasChecked? }`.
`checked`/`wasChecked` are present for `checkbox` and `radio` items only and
report the state the click *implies*: the framework never mutates its copy of
the tree, so the app stays the source of truth and answers with the next
`setContextMenuItems`.

**Accuracy, per backend.** WebKitGTK hands the `context-menu` signal a real
`WebKitHitTestResult`, so GTK's contexts and URLs are the engine's own. WKWebView
gives `willOpenMenu` no hit test at all, so macOS reads the click through this
file's page-side agent (the same private world `linkHover` uses), which posts
the hit as the DOM `contextmenu` event fires, ahead of WebKit's own menu
proposal on the same connection. If that report is missing or older than two
seconds, the contexts fall back to what WebKit's own menu item identifiers imply
(`WKMenuItemIdentifierCopyImage` means an image was clicked, and so on) and the
link URL falls back to the last hovered link; the image URL and selected text
are unavailable on that path.

**A script-message handler name is per view, not per world.** WebKitGTK routes
`script-message-received` by the handler name and refuses a name already
registered on that view, whatever world is asked for; WKUserContentController
keys on name *and* world. An app running two isolated worlds in one view must
therefore give each world its own name (`bridge_a`, `bridge_b`), and should
identify the sender by `name` rather than by the reported `world`, since the
name is what both engines actually route on. Registering the same name twice on
GTK is refused with an `ND_WARN` and no handler is connected, so the collision
cannot silently mis-deliver.

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
the request. Answer it with `respondScheme` (`{ id, base64, mime, status?,
headers? }`, or `{ id, error }` to fail it); an unanswered id leaves the page
waiting.

`headers` is a flat `{ name: value }` map applied to the response, which is how
an app serves `Content-Security-Policy`, `Cache-Control` or
`Access-Control-Allow-Origin` for its own scheme. GTK builds a
`SoupMessageHeaders` and hands it to
`webkit_uri_scheme_response_set_http_headers`; AppKit merges the map into the
`HTTPURLResponse`'s `headerFields` after the `Content-Type` and `Content-Length`
it derives from `mime`, so an app header wins over the default. Sending any
header (or a `status`) puts GTK on the
`webkit_uri_scheme_request_finish_with_response` path; a WebKitGTK too old for
that drops the headers with one `ND_WARN` and still serves the bytes.

`webviewEngine.registerScheme(scheme, { corsEnabled, secure })` honours both
flags on GTK, through the context's `WebKitSecurityManager`. Without
`corsEnabled` a page cannot read the scheme cross-origin at all, and without
`secure` its origins are not secure contexts, so `crypto.subtle`, IndexedDB and
service workers are unavailable to them. **AppKit accepts the flags and ignores
them**: WebKit's Cocoa API keeps that registry as SPI. Cross-origin reads still
work there through a scheme handler's own `Access-Control-Allow-Origin` header,
but a secure context cannot be granted at all.

`faviconChanged` carries `{ dataUrl }` on GTK (WebKit keeps a favicon database
and the icon is encoded to a PNG data URL, capped at 48 KB of PNG) and
`{ pageUrl, iconUrl }` on macOS, where WKWebView has no favicon API and the
app fetches the bytes itself. Handle both. The `dataUrl` goes straight into
`<button iconData>`, `<row iconData>` or a `SourceTreeNode.iconData` — all three
take a `data:` URL or a bare base64 payload and prefer it over `iconName`.

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

Nothing can open a real context menu from a script on either backend, so
`setContextMenuItems` is proven in three separate places instead: the parse and
hit-test matching are unit-tested (`src/gtk/context_menu.zig`, run by `zig build
test`), the command round trip is asserted from the host's own
`ND_WEBVIEW_TRACE` output by `scripts/headless-webview.sh`, and the menu the
user actually sees is verified by hand.

Status: the extended `<webview>` API above is implemented and runtime-verified
on both backends. CEF integration exists as a macOS proof-of-concept via the
native-plugin seam and is not yet part of the framework.
