---
title: WebView
description: The <webview> widget embeds the operating system's own web engine (WKWebView on macOS, WebKitGTK on GTK) with no bundled browser.
---

`<webview>` embeds the platform's own web engine as a native subtree (WKWebView on macOS,
WebKitGTK on GTK), so a page renders with the same engine the rest of the system uses. There is
no bundled Chromium: the OS engine is the widget, the same real-native-widgets contract the rest
of the toolkit follows.

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
| `testID` | string | —       | meta             | Automation handle, not rendered.                          |

`url` is a controlled prop: the widget navigates to it whenever it changes, but the host holds an
echo guard and reloads only when the new `url` differs from the engine's current URI, so
feeding `onNavigate` back into your `url` state does not re-trigger a load. A `<webview>` also expands
into whatever space its parent gives it (a zero-size web view would collapse inside a `<box>`), so it
is set to fill by default.

## Events

Navigation state comes back through five events. Their payloads follow the schema's shared shapes —
`text` events carry `{ text }`, boolean events carry `{ checked }`.

| Event              | Handler prop        | Payload           | Fires when…                                  |
| ------------------ | ------------------- | ----------------- | -------------------------------------------- |
| `navigate`         | `onNavigate`        | `{ text }` (URL)  | The page URL changes (links, redirects, history). |
| `titleChanged`     | `onTitleChanged`    | `{ text }`        | The document title changes.                  |
| `loadingChanged`   | `onLoadingChanged`  | `{ checked }`     | A load starts or finishes.                   |
| `backAvailable`    | `onBackAvailable`   | `{ checked }`     | Back-history availability changes.           |
| `forwardAvailable` | `onForwardAvailable`| `{ checked }`     | Forward-history availability changes.        |

On macOS these are derived by polling the view's navigation properties on a 10 Hz timer and emitting
an event on change, the same poll-don't-push idiom the terminal surface uses. Polling also catches
single-page apps that change the URL via `pushState` without a `WKNavigationDelegate` callback. On
GTK the events are wired to the corresponding WebKit signals (`load-changed`, `notify::uri`,
`notify::title`).

## Driving it with commands

Navigation to a URL is the `url` prop, but history and load control are one-shot imperative
actions with no declarative state to bind, so they are exposed as
[imperative commands](/core-concepts/imperative-commands/). Take a `ref` on the `<webview>`, then
call `sendCommand`:

```tsx
const page = useRef<NdNodeRef<"webview">>(null);
// …
sendCommand(page.current, "goBack");
sendCommand(page.current, "reload");
```

| Command     | Effect                                                    |
| ----------- | --------------------------------------------------------- |
| `goBack`    | Go back one entry (no-op when back history is empty).     |
| `goForward` | Go forward one entry (no-op when forward history is empty). |
| `reload`    | Reload the current page.                                  |
| `stop`      | Stop the in-flight load.                                  |

Command names are checked against the schema both at compile time (through `WidgetCommandNames`) and
again at runtime, so a stale string fails loudly. See
[Imperative Commands & Refs](/core-concepts/imperative-commands/) for the full mechanism. There is no
`loadURL` command — to load a page, set the `url` prop.

## How it works

On macOS the widget is a `WKWebView` subclass (`NDWebView`). WebKit is a system framework, so it is
always present and there is nothing to detect.

On GTK, WebKitGTK is resolved at runtime, not at link time. The surface `dlopen`s
`libwebkitgtk-6.0.so.4` (falling back to `libwebkitgtk-6.0.so` / `.dylib`) and looks up the handful
of `webkit_web_view_*` symbols it needs. If the library is present, you get a real `WebKitWebView`
and the log line `ND_WEBVIEW_ENGINE webkitgtk`.

This runtime-load design is deliberate. WebKitGTK is a ~1 GB closure with frequent soname churn and no
headless-CI story, so making it a link-time dependency would bloat every build and break the pinned
Nix flake and the Homebrew GTK stack (Homebrew ships no webkitgtk). Because the symbols are resolved
with `std.DynLib` instead, the build stays untouched. When webkitgtk is absent, the widget shows a
placeholder label reading "WebView unavailable (webkitgtk not installed)" and logs
`ND_WARN WebView unavailable` rather than failing to link. An app that uses `<webview>` still
builds and runs everywhere; it just shows no live page where the engine is missing.
