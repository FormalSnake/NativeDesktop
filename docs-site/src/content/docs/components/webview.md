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

| Prop     | Type   | Default | Applied          | Notes                                                     |
| -------- | ------ | ------- | ---------------- | --------------------------------------------------------- |
| `url`    | string | `""`    | createAndUpdate  | The page to load. Setting it navigates; empty is ignored. |
| `testID` | string | none    | meta             | Automation handle, not rendered.                          |

`url` is controlled: the widget navigates whenever it changes. The host holds an echo guard and
reloads only when the new `url` differs from the engine's current URI, so feeding `onNavigate` back
into your `url` state does not re-trigger a load. The widget fills whatever space its parent gives
it, since a zero-size web view would collapse inside a `<box>`.

## Events

Ten events report navigation and page state. Most follow the schema's shared payload shapes:
`text` events carry `{ text }`, boolean events carry `{ checked }`, `loadProgress` carries
`{ value }`. `loadFailed`, `downloadRequested`, and `javaScriptResult` carry a widget-specific
object nested under `data`.

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
| `executeJavaScript`  | `{ id, code }`           | Runs `code` in the page and replies with a `javaScriptResult` event carrying the same `id`. |

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
```

`openDevTools` opens the real WebKit inspector on GTK (`webkit_web_inspector_show`). WKWebView has
no programmatic "open the inspector" API on macOS: the command instead makes the view inspectable
(`isInspectable = true`, macOS 13.3+) and logs a reminder to attach through Safari's Develop menu.

Command names are checked against the schema at compile time (through `WidgetCommandNames`) and
again at runtime, so a stale string fails loudly. See
[Imperative Commands & Refs](/core-concepts/imperative-commands/). There is no `loadURL` command;
set the `url` prop.

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
