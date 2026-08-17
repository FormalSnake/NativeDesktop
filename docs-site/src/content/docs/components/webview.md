---
title: WebView
description: The <webview> widget embeds the operating system's own web engine (WKWebView on macOS, WebKitGTK on GTK) with no bundled browser.
---

`<webview>` embeds the platform's own web engine as a native subtree: WKWebView on macOS,
WebKitGTK on GTK. Pages render with the same engine the rest of the system uses, and nothing
Chromium-shaped is bundled.

![The webview widget rendering a page inside the browser example on macOS (AppKit)](../../../assets/screens/appkit/browser.png)

![The webview widget rendering a page inside the browser example on GNOME (GTK)](../../../assets/screens/gtk/browser.png)

```tsx
import { render, sendCommand, useRef, useState } from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";

function App() {
  const page = useRef<NdNodeRef<"webview">>(null);
  const [url, setUrl] = useState("https://example.com/");
  const [title, setTitle] = useState("Browser");

  return (
    <window title={title} defaultWidth={960} defaultHeight={640}>
      <box orientation="vertical" spacing={0}>
        <box orientation="horizontal" spacing={8} style={{ padding: 8 }}>
          <button
            label="←"
            cssClasses={["flat"]}
            onClick={() => { if (page.current) sendCommand(page.current, "goBack"); }}
          />
        </box>
        <separator />
        <webview
          ref={page}
          url={url}
          style={{ hexpand: true, vexpand: true }}
          onNavigate={(e) => setUrl(e.text)}
          onTitleChanged={(e) => setTitle(e.text || "Browser")}
        />
      </box>
    </window>
  );
}

await render(<App />);
```

The full version of this example lives at `examples/browser/main.tsx`.

## Props

| Prop                  | Type    | Default | Applied         | Notes                                                     |
| --------------------- | ------- | ------- | --------------- | --------------------------------------------------------- |
| `url`                 | string  | `""`    | createAndUpdate | The page to load. Setting it navigates; empty is ignored. |
| `profile`             | string  | `""`    | create          | Storage partition: `""` shares the default one, a name starting with `private` is ephemeral, any other name is a persistent partition of its own. |
| `contextMenuMode`     | `"native"` \| `"suppress"` | `"native"` | createAndUpdate | `"native"` shows the engine's own context menu with the app's `setContextMenuItems` merged into it; `"suppress"` shows no engine menu at all and leaves the whole menu to the app, off the `contextMenu` event. |
| `testID`              | string  | none    | meta            | Automation handle, not rendered.                          |

`url` is controlled: the widget navigates whenever it changes. The host holds an echo guard and
reloads only when the new `url` differs from the engine's current URI, so feeding `onNavigate` back
into your `url` state does not re-trigger a load. The widget fills whatever space its parent gives
it, since a zero-size web view would collapse inside a `<box>`.

## Events

Twenty-one events report navigation, page state and browser chrome. Their payloads mostly follow the schema's shared
shapes: `text` events carry `{ text }`, boolean events carry `{ checked }`, and `loadProgress`
carries a bare `{ value }`; `loadFailed`, `downloadRequested`, and `javaScriptResult` carry a
widget-specific object nested under `data` instead.

| Event               | Handler prop          | Payload                              | Fires when…                                        |
| -------------------- | --------------------- | ------------------------------------- | --------------------------------------------------- |
| `navigate`           | `onNavigate`           | `{ text }` (URL)                      | The page URL changes (links, redirects, history).   |
| `titleChanged`       | `onTitleChanged`       | `{ text }`                            | The document title changes.                         |
| `loadingChanged`     | `onLoadingChanged`     | `{ checked }`                         | A load starts or finishes.                          |
| `backAvailable`      | `onBackAvailable`      | `{ checked }`                         | Back-history availability changes.                  |
| `forwardAvailable`   | `onForwardAvailable`   | `{ checked }`                         | Forward-history availability changes.               |
| `loadProgress`       | `onLoadProgress`       | `{ value }`, `0..1`                   | Load progress changes (polled, rounded to 3 places). |
| `loadFailed`         | `onLoadFailed`         | `{ data: { url, error } }`            | A navigation fails to load. |
| `newWindow`          | `onNewWindow`          | `{ text }` (URL)                      | `target="_blank"`/`window.open()` requests a popup. |
| `downloadRequested`  | `onDownloadRequested`  | `{ data: { url, suggestedFilename? } }` | The engine hits a response it can't render itself (a download). |
| `javaScriptResult`   | `onJavaScriptResult`   | `{ data: { id, ok, value?, error? } }` | An `executeJavaScript` command completes. |
| `scriptMessage`      | `onScriptMessage`      | `{ data: { name, world, body } }`      | Page JS called `window.webkit.messageHandlers.<name>.postMessage(…)`. |
| `schemeRequest`      | `onSchemeRequest`      | `{ data: { id, url, scheme } }`        | The engine needs the app to serve a custom-scheme URL. |
| `cookiesResult`      | `onCookiesResult`      | `{ data: { id, ok, cookies?, error? } }` | A `getCookies` command completes. |
| `cookiesChanged`     | `onCookiesChanged`     | `{ data: {} }`                         | The profile's cookie jar changed (a ping, no payload). |
| `faviconChanged`     | `onFaviconChanged`     | `{ data: { dataUrl? } }` / `{ data: { pageUrl, iconUrl } }` | The page's icon resolved. |
| `findResult`         | `onFindResult`         | `{ data: { matchFound, matchCount?, done } }` | A find operation finished, or a match count arrived. |
| `securityChanged`    | `onSecurityChanged`    | `{ data: { secure, insecureContent, url?, error? } }` | The page's transport security state changed. |
| `linkHover`          | `onLinkHover`          | `{ text }` (URL, `""` on clear)        | The pointer entered or left a link. |
| `contextMenu`        | `onContextMenu`        | `{ data: { x, y, link?, image?, selection?, hasSelection, editable } }` | The user asked for a context menu. Fires in both modes. |
| `contextMenuItemClicked` | `onContextMenuItemClicked` | `{ data: { id, pageUrl, linkUrl?, imageUrl?, selectionText?, editable, checked?, wasChecked? } }` | One of the app's own context-menu items was chosen. |
| `sessionSaved`       | `onSessionSaved`       | `{ data: { id, state } }`              | A `saveSession` command completes. |
| `audioStateChanged`  | `onAudioStateChanged`  | `{ data: { playing, muted } }`         | The page started/stopped playing audio, or was muted. |

On macOS, `navigate`, `titleChanged`, `loadingChanged`, `backAvailable`, `forwardAvailable`, and
`loadProgress` are derived by polling the view's navigation properties on a 10 Hz timer and
emitting on change. Polling also catches single-page apps that change the URL via `pushState`
without a `WKNavigationDelegate` callback. On GTK those are wired to the corresponding WebKit
signals (`load-changed`, `notify::uri`, `notify::title`, `notify::estimated-load-progress`).
`loadFailed`, `newWindow`, `downloadRequested`, and `javaScriptResult` are delegate-driven on both
backends, since polling cannot observe them.

`target="_blank"` and `window.open()` never create a native popup. The host denies the popup and
emits `newWindow` with the requested URL, leaving the app to decide what to do with it, usually
opening a native tab. `examples/browser/main.tsx` is the worked pattern.

`downloadRequested` fires when the engine hits a response it cannot render. The in-engine download
is always cancelled and the app fetches the URL itself through Bun. GTK omits `suggestedFilename`,
which only WebKit's macOS delegate provides.

`loadFailed` filters two classes of routine navigation noise instead of firing on every cancelled
load: a newer navigation superseding an in-flight one, and the tail of a navigation the engine
cancelled itself, such as a response that turned into a `downloadRequested`.

## Commands

Set the `url` prop to load a page. History, load control, and page-level actions are one-shot
[imperative commands](/core-concepts/imperative-commands/). Take a `ref` on the `<webview>` and
call `sendCommand`:

```tsx
const page = useRef<NdNodeRef<"webview">>(null);
// …
sendCommand(page.current, "goBack");
sendCommand(page.current, "reload");
sendCommand(page.current, "setZoom", 1.5);
```

| Command             | Argument                | Effect                                                       |
| -------------------- | ------------------------ | ------------------------------------------------------------ |
| `goBack`             | none                     | Go back one entry (no-op when back history is empty).        |
| `goForward`          | none                     | Go forward one entry (no-op when forward history is empty).  |
| `reload`             | none                     | Reload the current page.                                     |
| `stop`               | none                     | Stop the in-flight load.                                     |
| `setZoom`            | number (`1.0` = 100%)    | Sets the page zoom factor.                                   |
| `setUserAgent`       | string                   | Overrides the UA string; `""` restores the engine's default. |
| `openDevTools`       | none                     | Opens the WebKit inspector (GTK); see the macOS note below.  |
| `executeJavaScript`  | `{ id, code, world? }`   | Runs `code` in the page (or in the named isolated world) and replies with a `javaScriptResult` event carrying the same `id`. |
| `addUserScript`      | `{ id, source, injectionTime?, world?, allowList?, blockList?, allFrames? }` | Injects `source` into every page load. `injectionTime` is `"start"` or `"end"` (default). Re-using an `id` replaces the script. |
| `removeUserScript`   | `{ id }`                 | Removes the script registered under `id`.                    |
| `clearUserScripts`   | `{ world? }` or none     | Removes every user script, or only those in one world.       |
| `registerScriptMessage`   | `{ name, world? }`  | Makes `window.webkit.messageHandlers.<name>` available to page JS. |
| `unregisterScriptMessage` | `{ name, world? }`  | Removes that handler.                                        |
| `respondScheme`      | `{ id, base64, mime, status? }` or `{ id, error }` | Answers a pending `schemeRequest`. |
| `getCookies`         | `{ id, url? }`           | Reads the profile's cookies, replying with `cookiesResult`.  |
| `setCookie`          | a cookie object          | Writes a cookie: `{ name, value, domain, path?, secure?, httpOnly?, expires?, sameSite? }`. |
| `deleteCookie`       | `{ name, domain?, path? }` | Deletes every matching cookie.                             |
| `findStart`          | `{ text, caseSensitive?, wrap? }` | Starts a find, replying with `findResult`.          |
| `findNext` / `findPrevious` | none              | Moves to the next/previous match.                            |
| `findStop`           | none                     | Ends the find and clears the highlight.                      |
| `saveSession`        | `{ id }`                 | Captures navigation history, replying with `sessionSaved`.   |
| `restoreSession`     | `{ state }`              | Restores a previously saved history blob.                    |
| `setMuted`           | bool                     | Mutes or unmutes the page's audio.                           |
| `setContextMenuItems` | `{ items: ContextMenuItem[] }` | Replaces the items merged into the engine's context menu (see below). |

`executeJavaScript` has no synchronous return path. Use the `executeJavaScript(node, code)` helper
from `@nativedesktop/react` rather than the raw command: it generates the `id`, sends the command,
and returns a `Promise<string>` that settles from the matching `javaScriptResult` event. Wire the
widget's `onJavaScriptResult` prop straight to the paired `onJavaScriptResult` export so the
promise has something to settle it:

```tsx
import { executeJavaScript, onJavaScriptResult } from "@nativedesktop/react";

<webview ref={page} url={url} onJavaScriptResult={onJavaScriptResult} />;
// …later:
const title = await executeJavaScript(page.current!, "document.title");
const hidden = await executeJavaScript(page.current!, "window.secret", "my-extension");
```

`getCookies` and `saveSession` follow the same pattern with the `onCookiesResult` and
`onSessionSaved` exports.

## Context menus

With the default `contextMenuMode="native"` the engine's own menu opens (Back/Forward/Reload,
Open Link, Copy Image, spell-check and the system text services on macOS, Inspect Element wherever
developer extras are on) and the app's items are appended to it after a separator. Declare them
per view with the `setContextMenuItems` helper; the host stores the tree until it is replaced.

```tsx
import { setContextMenuItems } from "@nativedesktop/react";

setContextMenuItems(page.current!, [
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

<webview ref={page} url={url} onContextMenuItemClicked={(e) => run(e.data as ContextMenuItemClick)} />;
```

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `id` | string | Echoed back on `contextMenuItemClicked`. Omit for a separator. |
| `label` | string | The item's text. Omit for a separator. |
| `type` | `"normal"` \| `"checkbox"` \| `"radio"` \| `"separator"` | Defaults to `"normal"`. |
| `checked` | bool | Drawn state for `checkbox` and `radio`. |
| `enabled` | bool | `false` renders the item insensitive. |
| `contexts` | `("page" \| "link" \| "image" \| "selection" \| "editable" \| "all")[]` | Where the item appears. Defaults to `["page"]`. |
| `targetUrlGlobs` | string[] | `*`-wildcard globs matched against the link or image URL under the pointer. |
| `children` | `ContextMenuItem[]` | A submenu. |

Which items a click earns is decided per invocation against that click's hit test. `page` means
the click landed on nothing more specific: a link, image, selection or editable field wins over
it, the way a browser's own menu behaves. A submenu whose every child was filtered out is dropped
rather than shown empty, a separator is only drawn when something survived on both sides of it,
and contiguous `radio` siblings form one group.

`checked` and `wasChecked` on the event report the state a click *implies*. The framework never
mutates its copy of the tree: the app owns the model and answers with the next
`setContextMenuItems`.

`openDevTools` opens the real WebKit inspector on GTK (`webkit_web_inspector_show`). WKWebView has
no programmatic "open the inspector" API on macOS: the command instead makes the view inspectable
(`isInspectable = true`, macOS 13.3+) and logs a reminder to attach through Safari's Develop menu.

Command names are checked against the schema at compile time (through `WidgetCommandNames`) and
again at runtime, so a stale string fails loudly. See
[Imperative Commands & Refs](/core-concepts/imperative-commands/). There is no `loadURL` command;
set the `url` prop.

## User scripts and isolated worlds

`addUserScript` injects JavaScript into every page the view loads, before or after the document
parses. A `world` name puts the script in an isolated JavaScript world: it shares the DOM with the
page but not its globals, so an extension-style script cannot be seen or tampered with by the page,
and two extensions cannot see each other. `executeJavaScript` takes the same world name, so the
host can read back what an injected script stored.

```tsx
sendCommand(page.current!, "addUserScript", {
  id: "dark-reader",
  source: "window.__theme = 'dark'",
  injectionTime: "start",
  world: "dark-reader",
  allowList: ["https://*.wikipedia.org/*"],
});
```

`allowList`/`blockList` are URL match patterns. WebKitGTK applies them natively; WKWebView has no
such API, so the framework compiles the patterns into a guard around the script source — the same
observable behaviour, expressed where WebKit allows it.

Scripts are a keyed registry: re-adding an `id` replaces it, `removeUserScript` drops one,
`clearUserScripts` drops all of them (or all in one world). On GTK a script is removed by identity;
WKUserContentController can only clear everything, so the AppKit side replays the surviving set.

## Talking to the page

`registerScriptMessage` creates a named channel, after which page JS can post to it and the app
receives `scriptMessage` with the decoded value:

```tsx
sendCommand(page.current!, "registerScriptMessage", { name: "bridge", world: "dark-reader" });
// page side (in that world): window.webkit.messageHandlers.bridge.postMessage({ ok: true })
<webview onScriptMessage={(e) => console.log(e.data)} />
```

`body` is the posted value itself, already decoded — an object arrives as an object, not a string.

A channel name is per view, not per world. WebKitGTK routes `script-message-received` by the name
and refuses a name already registered on that view whatever world is asked for; WKUserContentController
keys on name *and* world but still exposes one `window.webkit.messageHandlers.<name>` to the page. Two
isolated worlds in one view therefore need two names (`bridge_a`, `bridge_b`), and the app should read
the sender off `name` rather than off the reported `world`, because the name is what both engines route
on. A repeat registration of the same name and world is a no-op; a second world asking for a taken name
is refused with an `ND_WARN` rather than silently mis-delivering.

## Custom URI schemes

Register a scheme with `webviewEngine.registerScheme` and the app serves it. Both engines bind
scheme handlers to a configuration that is frozen once a web view exists, so **this must run before
the first `<webview>` mounts**; calling it later rejects with a clear error rather than silently
doing nothing.

```tsx
import { webviewEngine } from "@nativedesktop/react";

await webviewEngine.registerScheme("crx");
// then, on the webview that made the request:
<webview
  onSchemeRequest={(e) => {
    const { id, url } = e.data as { id: string; url: string };
    sendCommand(page.current!, "respondScheme", {
      id,
      base64: Buffer.from(bytesFor(url)).toString("base64"),
      mime: "text/html",
      status: 200,
    });
  }}
/>;
```

Answer with `{ id, error }` instead to fail the request. An id that never gets answered leaves the
page hanging on that resource, so always answer.

## Cookies and profiles

`getCookies`, `setCookie` and `deleteCookie` operate on the view's own storage partition, and
`cookiesChanged` fires when the jar changes. Deletion matches by name plus the domain and path you
supply; both backends look the live cookie up first, because both engines delete by identity.

`profile` picks that partition at create time. Two views with the same profile name share cookies,
cache and local storage; `private…` gets an ephemeral partition that leaves nothing on disk. On GTK
this is a `WebKitNetworkSession`, on macOS a `WKWebsiteDataStore` (named stores need macOS 14+).

## Engine resolution

On macOS the widget is a `WKWebView` subclass (`NDWebView`). WebKit is a system framework, always
present, nothing to detect.

On GTK, WebKitGTK is resolved at runtime rather than link time. The surface `dlopen`s
`libwebkitgtk-6.0.so.4` (falling back to `libwebkitgtk-6.0.so` / `.dylib`) and looks up the handful
of `webkit_web_view_*` symbols it needs. When the library is present you get a real
`WebKitWebView` and the log line `ND_WEBVIEW_ENGINE webkitgtk`.

Linking it at build time is not an option: WebKitGTK is a ~1 GB closure with frequent soname churn,
and Homebrew's GTK stack ships no webkitgtk at all. Resolving through `std.DynLib` keeps the build
independent of it. When webkitgtk is absent the widget shows a placeholder label reading "WebView
unavailable (webkitgtk not installed)" and logs `ND_WARN WebView unavailable`, so an app that uses
`<webview>` still builds and runs everywhere.

Every symbol beyond the base navigation set is looked up on its own, so a WebKitGTK too old for one
feature degrades only that feature, with one `ND_WARN` line naming the missing symbol. Cookies
additionally `dlopen` `libsoup-3.0` for `SoupCookie`, under the same rule.

### Engine differences worth knowing

- `findResult` carries `matchCount` on GTK (WebKit's counted-matches pass); `WKFindResult` reports
  only match/no-match, so treat the count as optional.
- `faviconChanged` carries a `dataUrl` on GTK, where WebKit keeps a favicon database. WKWebView has
  no favicon API, so macOS reports `{ pageUrl, iconUrl }` and the app fetches the bytes itself.
- `contextMenu` carries the selected text as `selection` on macOS. WebKitGTK's hit test reports
  only that a selection exists, so GTK sends `hasSelection` and no `selection`.
- Context-menu hit tests are the engine's own on GTK (`WebKitHitTestResult`). WKWebView hands
  `willOpenMenu` no hit test, so macOS reads the click through the page-side agent below; if that
  report is missing, contexts fall back to what WebKit's own menu item identifiers imply and the
  link URL to the last hovered link, with no image URL or selected text on that path.
- `linkHover`, `contextMenu` and the audio state are native signals on GTK. WKWebView exposes none
  of them, so on macOS the framework installs its own user script in a private world
  (`nd-internal`, handler `__ndInternal`) and observes them in the page. That world is reserved:
  do not use those names for your own scripts or handlers.
- TLS failures are never silently accepted. A certificate error fails the load and reports
  `securityChanged` with an `error`; there is no "proceed anyway" switch.
