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
CEF (Chromium) is the opt-in alternative for apps that need Chromium
fidelity, enabled per project *and* per platform in `nativedesktop.config.ts`
(e.g. macOS on WebKit, Linux on CEF).

The config surface, and the create-only `engine` prop it feeds:

```ts
// nativedesktop.config.ts
export default defineConfig({
  webview: {
    engine: { mac: "system", linux: "chromium" },
    cef: { version: "151.3.23", locales: ["en-US", "de"], schemes: ["myapp"], style: "alloy" },
  },
});
```

`nd dev` resolves the current platform's entry and exports it to the host as
`ND_WEBVIEW_ENGINE`, which is also the dev override: `ND_WEBVIEW_ENGINE=chromium
nd dev` wins over the config without editing it. Per view, `<webview engine>`
takes the same two values and defaults to `"system"`; it is create-only, since
swapping engines under a live view would mean rebuilding it.

`cef.schemes` names the custom schemes the app serves. A scheme is standard,
secure and CORS-enabled only if every process was told about it before
`cef_initialize`, which is long before app code runs, so it is declared here and
exported as `ND_CEF_SCHEMES` (comma separated, and the same dev override);
`registerScheme` then installs the handler behind it. A packaged app carries
engine and schemes itself: macOS reads them out of `nd-app.json` at startup,
Linux out of the generated `AppRun`.

`cef.style` picks CEF's browser style, `"alloy"` (default) or `"chrome"`, and
reaches the host as `ND_CEF_STYLE` with the same dev override. Alloy is the
Chromium content layer with CEF's extra client callbacks. Chrome style is
Chrome's own browser layer, which is what runs Chromium's extension system:
with it, `--load-extension=<dir>` on the app's command line loads an unpacked
Chrome extension, service worker and all, and `chrome://extensions` lists it.
The framework keeps the no-top-level invariant on both styles, so Chrome style
brings no toolbar and no window of its own; what it costs is listed under
[Chrome style](#chrome-style) below.

`nd doctor` reports the resolved engine and, for a chromium one, the style;
fails when a chromium config has no CEF dist to resolve; and audits the last
packaged bundle: an `engine: "system"` build containing Chromium bytes is an
error, not a warning.

### Chrome style

Linux only for now. The browser is still embedded in the host's own window: the
extension runtime, the docked inspector and Chrome's command handling come with
it, Chrome's window and toolbar do not.

- Extensions load from the command line (`--load-extension=<dir>`, repeatable,
  comma separated), which Chromium applies on every launch: the unpacked
  directories are the app's to re-declare, while everything the extension stores
  (`chrome.storage`, its settings, its granted permissions) lives in the
  profile and survives a restart.
- Every route that would put a Chromium window on screen is intercepted.
  `window.open`, `target=_blank`, middle- and ctrl-click and the context menu's
  "open link in …" items reach the app as `newWindow`; so do
  `chrome.windows.create`, `chrome.tabs.create` and `chrome.runtime.openOptionsPage`,
  which Chrome answers with a browser window of its own that this engine unmaps
  and closes before it is presented. Chrome's own accelerators for a new window,
  tab, incognito window, view-source, print, history, downloads and the
  extensions page are refused through `cef_command_handler_t::on_chrome_command`.
- DevTools is docked inside the view: `openDevTools`, F12 and ctrl+shift+I open
  Chrome's inspector as a second browser in the right-hand half of the same
  embedding window, and toggle it off again. Alloy still uses CEF's separate
  devtools window (parenting that into GTK crashes, CEF #3165). Quitting closes
  the devtools browser and waits for its `on_before_close` before closing the
  browser it inspects; left to CEF's own order the inspected browser goes first,
  its frames are then deleted through a freed `CefBrowserContentsDelegate`
  (SIGSEGV in `CefBrowserInfo::RemoveFrame`), and the devtools browser that is
  left behind never reports closed, so `cef_shutdown` hangs joining the UI
  thread.
- The page context menu is a GTK one. `run_context_menu` returns 1, Chromium's
  `cef_menu_model_t` is copied and drawn as a GtkPopoverMenu parented to the
  `<webview>` widget at the click, and the pick is handed back through
  `cef_run_context_menu_callback_t`. Everything Chromium puts in the model is
  there: Back/Forward/Reload, the link, image, selection and editable variants,
  spell-check languages and writing direction as radio and check items, an
  extension's `chrome.contextMenus` items with their submenus, and Inspect.
  Items whose command `cef_command_handler_t` refuses are dropped rather than
  drawn (Print, View page source, Cast, Create QR code); the "open link in …"
  items stay, because they reach the app as `newWindow`. The app's own
  `setContextMenuItems` tree is appended after a separator as it is under
  Alloy, and `contextMenuMode="suppress"` still shows nothing.
  Two differences from Chrome's own menu: a submenu slides the popover to its
  own pane rather than flying out, and accelerators are drawn for the keys
  Chromium reports in the model.
- The app menu, the page action icons and the toolbar buttons are all reported
  invisible, so no Chrome UI is created for the browser.

On macOS the same style is driven against the real browser app by
`scripts/mac/app-chrome-style.sh` (marker `ND_APP_CHROME_MAC_OK`), which covers
window resize, fullscreen, tabs, the app's own accelerators, its popovers over
the web contents and the quit paths; `scripts/mac/cef-reparent.sh` (marker
`ND_CEF_REPARENT_MAC_OK`) moves one live `<webview>` between two host windows
with `moveNode` and asserts the page survives it on both styles.

Chrome's own windows and dialogs, measured on CEF 151.3.23 (Chromium
151.0.7922.170):

- A **Chrome Web Store install works end to end**. "Add to Chrome" raises
  Chromium's own "Add <name>?" prompt, accepting it downloads and installs the
  CRX, and the extension is enabled, has its service worker, and is still there
  after a restart with no `--load-extension` anywhere. It used to take the host
  down the moment the install landed: `ExtensionInstallUIDesktop::OnInstallSuccess`
  asks `ScopedTabbedBrowserDisplayer` for a tabbed browser and holds a raw
  `BrowserWindowInterface*` to it across the asynchronous wait in
  `extensions::TriggerPostInstallDialog`. CEF makes every BrowserView-hosted
  browser a `TYPE_POPUP` (`chrome_browser_host_impl.cc`), so Chrome never finds
  one of this engine's, builds its own, and this engine closed that one inside
  `on_after_created`; the dialog then dereferenced freed memory. The first
  browser Chrome makes for itself is now kept, unmapped, as the tabbed browser
  every later lookup finds, and everything after it is closed as before.
- Chrome's dialogs that belong to no browser (the install prompt, the
  post-install dialog, the "Remove <name>?" confirmation) are Views widgets: no
  CEF callback is consulted about them, and they arrive as top-level windows on
  the X server. Chrome style watches the root for top-levels carrying this
  process's `_NET_WM_PID` that GDK does not know (which is what tells them from
  the app's own windows, its popovers and the page's GTK context menu) and that
  are not the kept browser, moves each one over the view that has focus, and
  reports it to the app as `chromeDialog`
  with `{x, y, width, height}`. The dialog is still Chrome's, drawn by Views, but
  it lands on the app's content instead of wherever Views put it.
- The kept browser is closed first by `closeBrowsersInOrder`, before the views
  and before `cef_shutdown`. Left for CEF's own teardown to unwind, with the
  post-install dialog still anchored to it, it took the host down on the way out
  of the process after an install.
- `ND_CEF_VERBOSE=1` puts CEF's log severity at verbose, which is what makes
  Chromium's own `--vmodule` output reachable; without it `cef_settings_t`
  pins the severity at warning and every VLOG is dropped.

Listing extensions, installing them, and their actions:

```tsx
import {
  installExtension, listExtensionActions, listExtensions, onExtensionActions,
  onExtensionsList, setExtensionEnabled, uninstallExtension,
} from "@nativedesktop/react";

// Chromium exposes its extension registry to chrome://extensions and nowhere
// else, so every one of these is sent to a view showing that page. A hidden
// one does.
<webview
  ref={registry}
  engine="chromium"
  url="chrome://extensions"
  onExtensionsList={onExtensionsList}
  onExtensionActions={onExtensionActions}
  onChromeDialog={(e) => setDialog(e.data)}
/>;

const installed = await listExtensions(registry.current!);
// [{ id, name, version, enabled, iconUrl: "data:image/png;…", optionsUrl }]

await installExtension(registry.current!, "/path/to/unpacked");
await setExtensionEnabled(registry.current!, id, false);
await uninstallExtension(registry.current!, id);
// each answers with the registry as it now stands, same shape as listExtensions

const actions = await listExtensionActions(registry.current!);
// [{ id, name, enabled, title, iconUrl, popupUrl, badgeText }]
```

`installExtension` takes an unpacked directory and loads it into the live
profile, with no relaunch and no `--load-extension`. There is no API that takes
a path: `chrome.developerPrivate.loadUnpacked` opens a directory chooser, so the
path is parked on the view and the engine's `cef_dialog_handler_t` answers the
chooser with it. `uninstallExtension` goes through `chrome.management.uninstall`,
which always draws Chrome's own "Remove <name>?" confirmation when the caller is
not the extension being removed (`developerPrivate` has no `uninstall`, and its
`removeMultipleExtensions` refuses its own documented signature on 151), so the
promise settles when that is answered and the dialog arrives as `chromeDialog`.

`listExtensionActions` reports what an extension's manifest declares, read off
disk by the host: `chrome://extensions` cannot fetch
`chrome-extension://<id>/manifest.json` (not a web-accessible resource, and the
WebUI origin is not the extension's) and `developerPrivate` reports commands and
pinning but not the action's popup, title or icon.

An extension's action popup and its options page are ordinary pages: put one in
a `<webview>` sized to the popover the app draws, with the `chrome-extension://`
URL set at create time. Chromium refuses a renderer-initiated navigation to an
extension page, so setting `url` on a view that already exists does not work;
mount a new view instead.

Against real Chrome, extension actions still differ. There is no toolbar button,
so `chrome.action.onClicked` never fires and the app decides what a click on its
own button does; an app with a popup opens it, and an extension whose action has
no popup cannot be triggered at all. `chrome.action.setPopup`, `setBadgeText`,
`setIcon` and `setTitle` are recorded by Chromium but answered only to the
extension itself, so `badgeText` is always empty and a popup URL changed at
runtime is not seen. A popup in an app-owned view does not close on blur and is
not sized by the popup document, so the app owns both. And the `activeTab` grant
Chrome issues when its own toolbar button is clicked is never issued.

Two paths that would close that gap are blocked in CEF 151, both on the same
missing piece:

- `cef_browser_view_t::get_chrome_toolbar` is unreachable. A browser created
  with a native `parent_window` under Chrome style goes through
  `chrome_child_window::MaybeCreateChildBrowser`, which builds the
  `CefBrowserView` with CEF's own internal `ChildBrowserViewDelegate`; that
  delegate does not override `GetChromeToolbarType`, so the toolbar is
  `CEF_CTT_NONE` and there is no embedder seam to change it.
- The `Extensions.triggerAction` CDP command, which is the genuine "user clicked
  the action" path (`ToolbarActionViewModel::ExecuteUserAction` with
  `InvocationSource::kCdp`), segfaults the browser process:
  `chrome/browser/devtools/protocol/extensions_handler.cc:195` dereferences
  `ExtensionsContainer::From(*browser)`, which is null for a browser with no
  toolbar. It is also a browser-target command, and `CefBrowserHost::
  ExecuteDevToolsMethod` reaches only page targets, where the whole `Extensions`
  domain answers "Method not available."

The smallest CEF patch that would fix both: give `CefBrowserViewDelegate` a way
to ask for a toolbar on a child-window browser, by having
`ChildBrowserViewDelegate::GetChromeToolbarType` answer from the
`CefWindowInfo`/`CefBrowserSettings` instead of the default, and let the
embedder hide the toolbar view afterwards through `GetChromeToolbar`. That is
`cef/libcef/browser/chrome/views/chrome_child_window.cc` plus a field in
`cef/include/internal/cef_types.h`, on the order of 60 lines, no
`patch/patches/` change. It would make `ExtensionsContainer::From` non-null,
which is what both the real action click and Chrome's post-install "pinned by
default" UI need.

The opt-in is structural rather than a runtime flag:

- CEF is never linked into the host binary. It loads at runtime
  (`cef_load_library` on macOS, `dlopen` on Linux) from the app bundle, behind
  the same `<webview>` contract, so app code does not change with the engine.
- Packaging stages the CEF framework/helpers only when the config enables it.
  An app that doesn't enable CEF ships zero Chromium bytes: the CEF
  distribution is fetched at package time (official builds:
  cef-builds.spotifycdn.com) and is never committed to the repo.
- On Linux, `libcef.so` ships largely unstripped (1.43 GB on disk); packaging
  runs `strip` on the staged copy when CEF is enabled, which takes it to about
  269 MB. `locales/` is trimmed to `cef.locales` (default `en-US`) rather than
  shipping all 100-odd .pak files.
- `chrome-sandbox` is staged with the mode the dist ships. The setuid sandbox
  wants it root-owned 4755, which only an installer can do; the user-namespace
  sandbox is the automatic fallback, and `--no-sandbox` never is.
- macOS additionally requires the five helper `.app` bundles (main, Alerts, GPU,
  Plugin, Renderer) under `Contents/Frameworks`, signed inside-out with JIT
  entitlements on the renderer and GPU helpers before the outer app is signed.
  All of it is a packaging concern, invisible to app code.
- Linux caveat: CEF windowed embedding under native Wayland is still unshipped
  upstream (CEF issue #2804; ANGLE's Wayland support merged 2026-05); until it
  lands, embedded CEF on Wayland means off-screen rendering or XWayland.

## Verification

`scripts/headless-webview-cef-chrome.sh` is the Chrome-style gate (marker
`ND_CEF_CHROME_OK`): the extension runtime, every route that would open a
Chromium window, docked devtools, the registry commands (install an unpacked
directory, list its action, disable, enable, uninstall through Chrome's
confirmation), and the extension and its storage across a restart. The top-level
census holds through every leg. `ND_CEF_CHROME_STORE=1` adds the Web Store legs
(`ND_CEF_CHROME_STORE_OK`), which are opt-in because they need the network and
Google's consent interstitial.

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

Under Chrome style the menu is driven for real. `scripts/cef-menu-drive.ts`,
the gate's `menu` pass, right-clicks a link, an image, a selection and an input
with real X11 events, reads back what the host drew (`ND_CEF menuShown`, one
line per rendered item, against `ND_CEF menuItem` for the model it came from),
walks the extension's submenu with the keyboard and checks the extension's own
handler wrote to `chrome.storage`, and asserts that Escape, a click away and a
navigation each answer the callback exactly once. Spelling *suggestions* are
not covered: Chromium downloads its hunspell dictionary on first use and the
gate has no network, so a misspelled word offers the Spell check submenu
without the corrections.

Status: the extended `<webview>` API above is implemented and runtime-verified
on both backends. CEF integration exists as a macOS proof-of-concept via the
native-plugin seam and is not yet part of the framework.
