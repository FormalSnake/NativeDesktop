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

Under Chrome style two more routes reach it, and both used to hand the app a URL
it could not use. A browser Chrome makes for itself (`chrome.tabs.create`,
`chrome.windows.create`, `chrome.runtime.openOptionsPage`) is reported through
`get_default_client`, and at `on_after_created` it has not started its
navigation yet, so its main frame's URL is empty: every extension-opened tab
arrived as a dead `about:blank`. The report now waits for that browser's first
navigation and carries the URL it was actually asked for; a browser that never
navigates is closed after 1.5 seconds and reported as nothing, since a tab the
app could only open on `about:blank` is the thing this exists to stop producing.
`ND_CEF sinkNewWindow` traces what was reported, and names the view the app
hears it on: the focused one, or any live one when the window that held the
focused view has since closed.

`window.open("about:blank")` followed by `w.location = …` from the opener is
still lost, and stays lost. Denying the popup makes `window.open` answer null
and the opener's next statement throw, so the destination exists nowhere. Letting
it through to the sink client instead was built and taken back out: Chrome
builds a real top-level for a popup, `on_after_created` runs before that window
has a handle so the engine has nothing to unmap, and `cef_window_info_t.bounds`
is ignored for a Chrome-style popup, so the window sat on screen at 1050x880
until the browser closed. Measured twice on 151.3.23, against the gate's own
top-level census.

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

`permissionRequest` carries `{ data: { id, origin, types } }` when a page asks
for geolocation, notifications, a camera or microphone, or any of the other
permissions Chromium prompts for. `types` is a comma-separated list, because one
request can carry several (`getUserMedia({audio, video})` asks for both). Answer
it with `respondPermission` (`{ id, allow }`); an unanswered id leaves the page
waiting, as `schemeRequest` does. Chromium engine only: it exists because Chrome
style would otherwise draw its own prompt, a Views bubble anchored to the
toolbar this embedding does not have.

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
- DevTools is docked inside the view: `openDevTools`, `closeDevTools`, F12 and
  ctrl+shift+I open Chrome's inspector as a second browser in the right-hand
  half of the same embedding window, and toggle it off again. The shortcut
  arrives as `IDC_DEV_TOOLS_TOGGLE`, which Chrome would answer with a
  DevToolsWindow of its own, so `cef_command_handler_t::on_chrome_command`
  takes it and closes the docked browser instead. The inspector's own close
  button is drawn by the frontend and only when it was told it can dock, which
  is `can_dock` on the frontend URL; CEF builds that URL inside `show_dev_tools`
  and takes no argument for it, so the frontend is re-pointed at its own address
  with the flag added once the document is up. The button then reaches CEF as
  `closeWindow`, which closes the devtools browser, and the dock comes down the
  same way the toggle takes it down. The dock's width is floored at the width
  that toolbar needs: narrower and it overflows to the right, taking the close
  button off screen with it. Alloy still uses CEF's separate
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
- Permission requests (geolocation, notifications, camera, microphone and the
  rest of Chromium's prompt list) are taken by `cef_permission_handler_t` and
  reported to the app as `permissionRequest`, so Chrome's own prompt is never
  built. Without it Chromium draws a Views bubble anchored to a toolbar that
  does not exist: on GTK that lands inside the browser's X window at the top
  left of the page, on AppKit it becomes a window of its own that follows the
  invisible anchor.
- What is left Chromium-drawn, measured by the gate's `dialogs` pass: the
  WebAuthn sheet (`navigator.credentials.get`/`create`), HTTP basic auth, the
  "save password" bubble and the autofill surfaces. On GTK all of them are
  painted inside the browser's own X window, so they cannot leave the app; the
  WebAuthn sheet is centred at the top of the page and Escape ends the request
  with `NotAllowedError`.
- On AppKit each of those is an `NSWindow` of its own, a `NativeWidgetMacNSWindow`
  Chromium parents to the invisible anchor, so it followed the anchor rather
  than the app. The host now adopts them (`NDCefSurfaceWindows`): a 200ms sweep
  of the process's window list finds the Views windows that are not the app's
  own and not an anchor, makes each a child window of the window hosting the
  browser that raised it, and places it centred at the top of that webview's
  screen rectangle, which is where Chrome puts a tab-modal sheet. They then move
  and resize with the app window, leave the screen with a hidden tab, and are
  handed back before the engine tears down. `didBecomeKey`/`didBecomeMain` are
  no use for finding them: a Views window is ordered in without ever becoming
  key or main.
- `cef_request_handler_t::get_auth_credentials` is implemented and refuses the
  challenge, which is what Alloy needs, but Chrome style never calls it on CEF
  151.3.23: the login window still comes up and the handler's own warning never
  prints. Chrome answers an auth challenge through its own LoginHandler, and
  there is no client seam in front of it.
- WebAuthn has no CEF callback at all. A request can be ended from the page
  with an `AbortController` (which is what the gate does) but not from the
  embedder, so on AppKit a page asking for a passkey still puts a window on
  screen that the app does not own. Closing that needs either a CEF patch or a
  Chromium feature switch that turns the sheet off, neither of which is in
  this branch.

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
- A compositor that owns XWayland placement can put a Chrome dialog back. The
  watcher marks each one transient for the host window and gives it
  `_NET_WM_WINDOW_TYPE_DIALOG`, which is what a compositor reads to float and
  centre a dialog on its parent, and then moves it over the view once.
  Hyprland with the owner's config wins anyway: a rule there matches any window
  with an empty class and an empty title (which is what one of these is until
  well after it is mapped) and moves it to the top right of the monitor, and it
  re-applies that within the frame, so the move is undone as fast as it is sent.
  Re-sending it on every tick only trades positions, and reparenting the dialog
  into the host's own window holds the position but breaks Chromium's hit
  testing, because Views keeps the screen bounds it had when it created the
  window. What is left for the app is the `chromeDialog` event, which reports
  where the dialog is either way.
- An AdwDialog the app presents in-window (`<commandpalette>`, `showAlert`,
  `showAbout`, the script dialog a WebKit view puts up) is drawn by the
  toplevel's own surface, and the X server stacks the child window a page is
  rendered into above everything its parent draws. Over a webview the dialog
  and its scrim were painted, reported by GTK as presented, and never seen: the
  window sat modal-blocked with nothing on it until Escape. Every one of them
  now goes through `src/gtk/dialogsurface.zig`, which tells the engine, and the
  pages in that window stand aside (the place a hidden tab's window waits)
  until the dialog closes. The page area is the window's background while one
  is up, under the dialog's own scrim. Whether a dialog is up is read back from
  `adw_window_get_visible_dialog` rather than counted, and the engine tick puts
  a page back that was left aside.
- Every window Chromium puts on the root (a dialog, the bubble it shows when a
  page goes fullscreen, the small parked ones) arrives with no `WM_CLASS` at
  all, and a compositor hands that straight to whatever enumerates windows: on
  Hyprland the app's own toplevel and an untitled second window are both
  `hyprctl clients` entries, which is what a screenshot picker built on that
  list offers, and a rule keyed on an empty class and title (the owner has one)
  moves it to a corner. The window watcher copies the app toplevel's own
  `WM_CLASS` onto each of them, once per window. The containers the pages are
  rendered into need nothing: they are children of the toplevel and the
  compositor never sees them.
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

`scripts/headless-app-chrome.sh` is the gate for this path, marker
`ND_APP_CHROME_OK`: it runs a real app twice, once on Xvfb under a reparenting
window manager and once on headless sway under XWayland, and drives resizing,
tiling, maximize and fullscreen, tab switching, docked devtools, keyboard focus
and the page popups with real X and compositor input rather than automation
calls. `ND_APP_DIR` points it at an app checkout; with none it falls back to
`examples/cef-probe` and reports the app-level legs as skips.

Open on this path, measured on CEF 151.3.23 (Chromium 151.0.7922.170):

- A `<select>` element opens no dropdown on a plain X server with software
  compositing. Chromium does create the popup: with `--vmodule` on, the GPU
  process reports `XGetWindowAttributes failed` for it in
  `x11_software_bitmap_presenter.cc` and a `CreateGC` DrawableError against the
  same id, and the popup is torn down before it is mapped. Nothing above it is
  involved: the window manager, a compositing manager (xcompmgr changes
  nothing) and Alloy all behave the same, and the same build opens the dropdown
  normally under XWayland.
- Under XWayland, a key pressed while the pointer is over the page reaches the
  browser whatever the app's focused widget is. Xwayland pins X input focus to
  the toplevel, and X delivers a key press to the window the pointer is inside
  when that window is below the focused one, so the browser's own child takes
  it; the engine has told the browser the keyboard is not its, so the key is
  dropped rather than typed into the page. On a plain X server the window
  manager's focus proxy takes the key out of the app's window tree and the
  focused widget gets it, which is why the same leg passes there. Moving X
  focus to a proxy of the engine's own does not help: GTK4 reads the keyboard
  through XI2, which delivers to the focus window without propagating to the
  ancestor GDK selected on, so with focus on any window but the toplevel
  surface the window stays key and every key press is dropped (measured with a
  1x1 child of the toplevel and with the one GDK makes for itself).

`scripts/cef-portal-drive.ts`, the gate's other drive
(`ND_ACCEPT_DRIVE=scripts/cef-portal-drive.ts ND_APP_SCRIPT=examples/multiwindow/main.tsx`),
covers a tab dragged into another window: `moveNode` relocates the live widget,
the engine reparents the X child the browser renders into under the window that
now shows it, and the drive asserts the debugger target, the JS state, the
scroll offset and the URL are the ones from before the move.

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
// What the MANIFEST declares. An extension can turn its popup off at runtime,
// and an app must read readExtensionAction before opening one (see below).

const sources = await watchExtensions(registry.current!, (change) => reload(change.reason));
// ["developerPrivate.onItemStateChanged", "management.onInstalled", …]
```

`watchExtensions` subscribes the registry view to Chromium's own change events,
which is the only way an app learns about an install it did not make: a Web
Store install happens entirely inside Chromium and reaches no other `<webview>`
callback. Which events a build exposes to `chrome://extensions` is not something
the host can know from a header, so nothing is assumed: the page feature-detects
`chrome.developerPrivate.onItemStateChanged` and the four `chrome.management`
events, attaches to the ones that are there, and the promise answers with that
list. An empty list is a rejection, not a silent no-op. Later changes arrive on
the `onExtensionsChanged` prop as `{ reason }`, carrying Chromium's own
vocabulary (`INSTALLED`, `UNINSTALLED`, `LOADED`, `PREFS_CHANGED`, …) or the
`management` event name; treat it as a hint and re-read the registry. The
subscription belongs to the document, so call it again after the view reloads.

`installExtension` takes an unpacked directory and loads it into the live
profile, with no relaunch and no `--load-extension`. There is no API that takes
a path: `chrome.developerPrivate.loadUnpacked` opens a directory chooser, so the
path is parked on the view and the engine's `cef_dialog_handler_t` answers the
chooser with it. It is bounded: 90 seconds, after which the promise rejects with
the path it was given rather than leaving the app waiting for the process's
life. Two things could stall it before that. Chromium unpacks and validates the
directory itself, which a large extension makes slow (a 46 MB one took over a
minute). And `loadUnpacked` can open the chooser a second time, with no path
parked for it; that used to fall through to CEF's own directory chooser, a
dialog nobody answers, so an unarmed chooser during an install is now cancelled
and the failure reaches the app. `ND_CEF installDialogAnswered` and
`ND_CEF installDialogUnarmed` tell the two apart in the host log. `uninstallExtension` goes through `chrome.management.uninstall`,
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

What the 1Password extension (`aeblfdkhhhdcdjpifhhbdiojplfjncoa`, 8.12.37.1)
does inside this embedding, measured headless on CEF 151.3.23 against an
unpacked copy loaded at launch beside a control extension whose content script
does nothing but mark the DOM:

- **Content scripts inject.** On a plain `http://127.0.0.1` login page with a
  username and a password field, `Runtime.executionContextCreated` reports an
  isolated world named "1Password – Password Manager" for the main frame, and
  evaluating in it answers `chrome.runtime.id`. The control extension's world is
  there beside it and its content script marked the document.
- **It injects no UI.** The same document has no `com-1password-*` element, no
  shadow host, and no `data-onepassword-*` attribute on either field, while the
  control extension's marker div and title suffix are both present. So the
  inline icon being absent is not this framework failing to run content scripts;
  1Password's own script runs and stays quiet. A never-configured extension with
  no account is the first thing to rule out, since 1Password shows its inline
  icon while merely locked.
- **Its service worker goes to sleep and does not come back.** It is a target at
  startup and gone by the end of the run, which is ordinary MV3 behaviour, but
  nothing here wakes it: an app that wants the worker running has the same one
  lever the Extensions page uses, `chrome.developerPrivate.openDevTools` with
  `isServiceWorker: true`.
- **Its welcome tab exists but nobody sees it.** A page target for
  `chrome-extension://<id>/app/app.html#/page/welcome?language=en` was present
  from startup: `chrome.tabs.create` worked and Chrome built a browser for it,
  which this engine keeps unmapped as the tabbed browser the install path needs.
  That is why signing in was never possible in the app: the page it wants is
  open, off screen, in a browser the app is not told the URL of. The fix is
  above, under what a Chrome-created browser reports.
- **`browser_info_manager.cc:858 Timeout of new browser info response for
  frame …` repeats every two seconds** for the whole run, with a fresh frame id
  each time. Frames are being created that the browser side never resolves.
  Unmeasured: whether those frames are 1Password's (its inline menu is an
  iframe) and whether a frame that never gets browser info is what leaves its
  popup and its inline UI blank.

The popup document does not hang, and the popup host type is not what was
wrong. Measured with the real extension in an app-owned view, CDP attached and
every `chrome.*` namespace wrapped from a document-start script:

- It boots. Nine calls, all `chrome.runtime.sendMessage`, all answered by its
  service worker, none left pending, no uncaught error and no failed request.
  It never calls `tabs.query`, `windows.getCurrent` or `extension.getViews`
  from the popup at all, so the three answers this document used to blame were
  never asked for. Its worker resolves the page for it and gets the right one:
  `get-active-tab` answered with the `<webview>` the app was showing, not with
  the popup.
- It renders. A React tree with 23 laid-out elements under `#root`, which is
  the skeleton it draws for the state its worker gave it:
  `{ state: "AccountPasswordRequired", details: { accounts: [] } }`. No account
  is configured, so there is nothing for the popup to show and onboarding lives
  in the tab it opens at install.
- **Chromium says the action has no popup.** `chrome.action.getPopup({})` and
  `getPopup({tabId})` both answer `""` for every tab, though the manifest
  declares `"default_popup": "popup/index.html"`. 1Password clears it at
  runtime while no account is configured, so that a click on Chrome's own
  toolbar button reaches `chrome.action.onClicked` and opens onboarding. An app
  that reads the manifest off disk and opens that popup anyway is showing a
  document the extension had switched off. That is the whole of what the owner
  saw.

`chrome.action.openPopup()` exists and refuses, which is the same fact from the
other side: `Extension does not have a popup on the active tab.` for the
windows whose tab it will consider, and `Cannot show popup for an inactive
window.` for the rest. It was tried against every window this engine creates
and against the browser Chrome keeps for itself; none of them made Chrome build
an extension popup, and the browser process survived each attempt.

So an action's live state is the thing to read, and `readExtensionAction` reads
it. Where from is forced: `chrome://extensions` cannot answer, measured on 151
by enumerating it (`developerPrivate` has no action field, `ExtensionInfo` has
none of `popup`, `badgeText` or `title`, and the WebUI has no `chrome.action`
and no `chrome.tabs`). A page of the extension has the whole API, so the
command is sent to a view showing one, which for an app drawing its own toolbar
is the popup view it mounts for a click:

```tsx
import { readExtensionAction } from "@nativedesktop/react";

const state = await readExtensionAction(popupView.current!);
// { id, tabId, tabUrl, popupUrl, badgeText, badgeColor, title, enabled }
if (state.popupUrl === "") {
  // The extension has no popup right now. Opening the manifest's one anyway is
  // the bug above.
}
```

Which tab the state is read for is Chromium's own answer, not this framework's.
`cef_browser_t::get_identifier` says in its header that it "is also used as the
tabId for extension APIs" (`include/capi/cef_browser_capi.h:132`); under Chrome
style it is not. A view's identifier is CEF's own small counter, and passing one
to `chrome.action.getBadgeText({tabId})` answers `No tab with id: 1` while the
real tabs are session ids in the hundreds of millions. So the command asks
`chrome.tabs.query({ active: true, lastFocusedWindow: true })`, which is what an
extension itself uses and what 1Password's own worker used to find the page,
and reports the `tabId` and `tabUrl` it read for. Read those back: the browser
Chrome keeps for itself is the profile's only `normal` window, so when no view
has taken focus yet it is what `lastFocusedWindow` resolves to, and the state
comes back for whatever is parked in it. Measured: with no view focused, the
answer for 1Password carried
`tabUrl: "chrome-extension://…/app/app.html#/page/welcome"`. A click on the
app's own toolbar follows focus in one of its views, which is the case this is
read in.

What is still missing is the click itself. An action whose runtime popup is `""`
is one whose click Chrome answers with `chrome.action.onClicked`, and there is
no toolbar button here for Chromium to consider clicked, so the app has to
decide what that click means: for 1Password, opening its onboarding page.

The host still cannot reach an extension's service worker. Its whole CDP
substrate is `cef_browser_host_t::execute_dev_tools_method` (`src/cef/cdp.zig`),
which is per browser and page-target only; a `service_worker` target exists only
on the remote debugging port, which the host does not speak. Reading the action
from a page of the extension is what that constraint leaves, and it is what
ships.

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

Its `dialogs` pass covers the surfaces Chromium draws itself: a passkey request
with and without conditional mediation, `navigator.credentials.create`, the
geolocation, notification and camera prompts, `alert`/`confirm`/`prompt`, HTTP
basic auth, a download and a password-form submit. Each one is fired from
`scripts/fixtures/dialog-surfaces.ts` and answered by two questions, where the
UI landed and whether the page's own promise ever settled. The page is served
through `localhost` rather than `127.0.0.1` because WebAuthn refuses an
IP-address origin.

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
