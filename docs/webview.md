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

`javaScriptResult` carries `{ data: { id, ok, value?, error? } }` when
`executeJavaScript` completes. Apps normally never touch it: pass the exported
`onJavaScriptResult` handler and use the promise helper.

## Command notes

`executeJavaScript` takes `{ id, code }`. Prefer the promise helper. JS
exceptions reject with the real message, for example `Error: boom`.

`setZoom` takes a number, the page zoom factor. `setUserAgent` takes a string,
and an empty string restores the engine default.

`openDevTools` opens the WebKit inspector window on GTK. macOS has no
programmatic open, so it sets `isInspectable` and the inspector attaches through
Safari's Develop menu.

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

Status: the extended `<webview>` API above is implemented on both backends
(runtime-verified on AppKit; GTK pending the Linux verification gate). CEF
integration exists as a macOS proof-of-concept via the native-plugin seam and
is not yet part of the framework.
