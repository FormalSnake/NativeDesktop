# Webview: browser-grade apps on native UI

`<webview>` embeds the platform's web engine as an ordinary widget — WKWebView
on macOS, WebKitGTK (dlopen'd at runtime, graceful placeholder when absent) on
Linux. The surface is deliberately browser-grade: full web browsers with native
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

- **`loadProgress`** `{ value }` — estimated load progress 0..1, emitted on
  change only.
- **`loadFailed`** `{ data: { url, error } }` — a navigation failed. `url` is
  the *failing* URL (not the page navigated away from). Cancellations and
  policy-interruption noise are filtered on both backends; blocked-port loads
  (which WebKit silently turns into `about:blank`) are detected and reported.
- **`newWindow`** `{ text }` — the page requested a new window
  (`window.open`, `target=_blank`). The host always denies the native popup;
  the app decides what a "new window" means (usually a native tab).
- **`downloadRequested`** `{ data: { url, suggestedFilename? } }` — the engine
  hit a response it can't render (or an attachment). The engine-side download
  is cancelled; the app performs it itself — the Bun process has full
  network/fs, so `fetch` + `node:fs` (plus `getAppDataDir()`) is the intended
  path. `data:` URLs report `suggestedFilename: "Unknown"`.
- **`javaScriptResult`** `{ data: { id, ok, value?, error? } }` — completion of
  `executeJavaScript`. Apps normally never touch it directly: pass the exported
  `onJavaScriptResult` handler and use the promise helper.

## Command notes

- **`executeJavaScript`** arg `{ id, code }` — prefer the promise helper. JS
  exceptions reject with the real message (e.g. `Error: boom`).
- **`setZoom`** arg number — page zoom factor.
- **`setUserAgent`** arg string — custom UA; empty string restores the engine
  default.
- **`openDevTools`** — GTK opens the WebKit inspector window. macOS has no
  programmatic open: it sets `isInspectable`, then the inspector attaches via
  Safari's Develop menu.

Adding new webview *events* requires one-line routing entries in
`tools/codegen.ts` (`SIGNALS` and `SWIFT_SIGNALS`) plus the schema; new
*commands* are schema-only (dispatch forwards the raw command string to the
hand-written engine files `src/gtk/webview.zig` / `NDShell/NDWebView.swift`).

## Engines

The system engine is the default on both platforms and costs zero bundle bytes.
CEF (Chromium) is the planned opt-in alternative for apps that need Chromium
fidelity, enabled per project *and* per platform in `nativedesktop.config.ts`
(e.g. macOS on WebKit, Linux on CEF).

The opt-in is structural, not a flag:

- CEF is **never linked** into the host binary. It loads at runtime
  (`cef_load_library` on macOS, `dlopen` on Linux) from the app bundle, behind
  the same `<webview>` contract — app code does not change with the engine.
- Packaging stages the CEF framework/helpers **only** when the config enables
  it. An app that doesn't enable CEF ships **zero Chromium bytes** — the CEF
  distribution is fetched at package time (official builds:
  cef-builds.spotifycdn.com), never committed to the repo.
- On Linux, `libcef.so` ships largely unstripped (>1 GB on disk); the packaging
  step must run `strip` when CEF is enabled (~200 MB after).
- macOS additionally requires helper `.app` bundles (Renderer/GPU/Alerts/main)
  under `Contents/Frameworks` and inside-out codesigning — a packaging concern,
  invisible to app code.
- Linux caveat: CEF windowed embedding under native Wayland is still unshipped
  upstream (CEF issue #2804; ANGLE's Wayland support merged 2026-05); until it
  lands, embedded CEF on Wayland means off-screen rendering or XWayland.

Status: the extended `<webview>` API above is implemented on both backends
(runtime-verified on AppKit; GTK pending the Linux verification gate). CEF
integration exists as a macOS proof-of-concept via the native-plugin seam and
is not yet part of the framework.
