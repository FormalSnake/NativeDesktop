# CEF engine for `<webview>`

Status: in progress on `cef-engine` (lanes: `cef-gtk`, `cef-mac`, `cef-pkg`).
Source research: a 2026-08-20 deep pass over CEF 151.3.23 (Chromium 151.0.7922.170),
the capi headers, and the open issue tracker. Facts below carry issue numbers so
they can be re-checked when CEF moves.

## What this is

Chromium as an opt-in engine behind the existing `<webview>` widget, per project
and per platform, exactly as `docs/webview.md`'s Engines section promises: never
linked into the host, loaded at runtime, zero Chromium bytes shipped unless the
app's config asks for it. App code does not change with the engine; the schema
contract (props `url`/`profile`/`contextMenuMode` plus the new `engine`, 21
events, 26 commands) is the parity bar, and the webview drive is the gate.

## The style decision (settled, do not relitigate per lane)

CEF has two browser styles. Chrome style carries Chrome's UI subsystems and the
native extension runtime, but only inside CEF Views windows that CEF owns.
Alloy style is what you get the moment a browser is parented into a caller's
view (`parent_view` on macOS, an X11 XID on Linux), and it cannot run Chrome's
extension runtime (removed in M128; loading is `--load-extension` only, Chrome
style only, forum-confirmed 2024-10).

We embed Alloy-style, windowed, into the host's own widgets:

- The native-UI premise of the framework is non-negotiable; CEF Views owning
  the window would replace AppKit/GTK chrome with CEF's widget set.
- Extension support in this ecosystem comes from the app-side broker (the
  NativeBrowser extension host), which runs on user scripts, isolated worlds,
  script messages, and custom schemes. On CEF those primitives are rebuilt on
  the DevTools protocol (below); the broker itself does not change.
- A Chrome-style/Views spike (native `--load-extension`) stays on the backlog
  as a separate experiment. It is a bet, not a plan.

## Hard invariant: no CEF-created window, ever

Prior art from the field: a stray Chromium window appearing over the app is the
single fastest way this integration falls apart. Every CEF path that can create
a native window is intercepted:

- Popups (`window.open`, `target=_blank`): `on_before_popup` returns 1 and
  emits the existing `newWindow` event; the app opens a tab. Same for
  `on_before_dev_tools_popup`.
- DevTools: `openDevTools` uses `show_dev_tools` into a window we create and
  own (top-level is acceptable there, it is a tool window we parent and title),
  never CEF's default. On Linux, parenting devtools into a raw GTK/X11 window
  crashes (issue #3165), so the devtools window is CEF-created but wrapped:
  frameless, sized and closed by us. If that cannot be made clean, M1 ships
  `executeDevToolsMethod` plumbing and no devtools window.
- JS dialogs (`alert`/`confirm`/`prompt`, onbeforeunload): `cef_jsdialog_handler`
  suppresses Chrome's dialog and routes into the same host-native sheet path
  WebKit uses today.
- File choosers: `cef_dialog_handler` routes to the host's native open/save
  panels.
- Permission prompts (`cef_permission_handler`), external-protocol launches,
  and print dialogs are answered programmatically or denied; nothing may
  auto-present.
- The browser is only created once the parent view is realized (NSView in a
  window, or a mapped GdkSurface with an XID); a null or unrealized parent
  silently produces a top-level Chromium window, which is exactly the reported
  failure. Creation asserts the parent first.

Drive assertion: after popup, devtools, dialog, and download legs, the OS-level
window census matches the app's own `windows()`. No foreign top-level appeared.

## Process and loading model

- Single binary, five roles. `cef_execute_process` runs at the very top of
  `main()` in every process. Linux re-execs the host binary with `--type=...`;
  set `browser_subprocess_path` to a small dedicated subprocess binary so the
  Bun child and GTK never load into renderers (recommended, not M1-blocking).
  macOS requires the five helper `.app` bundles (`""`, ` (Alerts)`, ` (GPU)`,
  ` (Plugin)`, ` (Renderer)`) under `Contents/Frameworks`, same executable,
  different plists.
- Runtime loading, never linked: macOS ports `cef_library_loader.mm` to Zig
  (dlopen of `Chromium Embedded Framework.framework/Chromium Embedded
  Framework` plus `cef_load_library`); Linux dlopens `libcef.so`. Resolution
  order: `ND_CEF_ROOT` env, then the app bundle (`Contents/Frameworks` or
  `lib/cef`), then the dev cache `~/.cache/nativedesktop/cef/<cef-version>/`.
  Absent CEF with `engine="chromium"` requested: `ND_WARN` once and fall back
  to the system engine; the widget still works.
- API pin: `CEF_API_VERSION=15101`; `cef_api_hash(version, 0)` is the first
  CEF call after load. The capi refcount contract (`base.size`, atomic
  add_ref/release, release-before-return on callback params) lives in one Zig
  harness (`src/cef/ref.zig`), never open-coded per handler.

## Message loops and GPU

- macOS: `cef_run_message_loop()` replaces `[NSApp run]`; NSApplication is
  subclassed at runtime (objc_allocateClassPair) to adopt `CefAppProtocol`.
  `external_message_pump` and `multi_threaded_message_loop` are unsupported on
  mac; do not try.
- Linux: `multi_threaded_message_loop=1`. CEF's UI thread runs a private
  non-default GMainContext (M86+), so the host's GTK4 default-context loop is
  untouched. `XInitThreads()` before GTK init. Never `external_message_pump`
  on Linux (#2002, #3782: broken text input).
- GPU: windowed embedding only. It inherits Chromium's real compositing path
  (IOSurface/CALayer delegation on mac, Ozone GPU process on Linux). OSR is
  rejected: 30fps default frame cap, no ProMotion, no partial damage (#3730).
  Linux VA-API video decode wants
  `--enable-features=AcceleratedVideoDecodeLinuxGL,AcceleratedVideoDecodeLinuxZeroCopyGL`
  behind a config flag.

## Linux display servers

Windowed embedding is compiled X11-only (`SUPPORTS_OZONE_X11`, no Wayland arm).
Under Wayland sessions the view embeds via XWayland: before `gtk_init` the GTK
host pins `gdk_set_allowed_backends("x11")` when (and only when) the CEF engine
is enabled for the app. Native `wl_subsurface` embedding is CEF PR #4233
(2026-08, unmerged) on top of issue #2804; track it, do not block on it.
Known-open upstream on this path, accepted for now with tests marking them:
IME with MTML + SetAsChild (#3474), keyboard focus (#2026), HiDPI oversize
(PR #4208). `--gtk-version=4` is passed; Chromium dlopens the already-loaded
GTK4 via RTLD_NOLOAD (GTK is not linked into libcef since 2018).

## Contract mapping (M2)

| `<webview>` surface | CEF |
| --- | --- |
| `profile` ("", named, `private…`) | one `root_cache_path`; per-profile `cef_request_context_create_context` with `cache_path` (empty = in-memory for private) |
| navigate/back/forward/reload/stop | `cef_browser_host` plus `can_go_back`/`forward` via `cef_display_handler`/`cef_load_handler` events |
| `setZoom` | `set_zoom_level` (CEF zoom is log-scale, convert from the linear factor) |
| `setUserAgent` | per-context `user_agent` at context creation; "" resets |
| cookies (`getCookies`/`setCookie`/`deleteCookie`) | `cef_cookie_manager_t` of the view's request context |
| custom schemes (`registerScheme`/`respondScheme`) | `cef_register_scheme_handler_factory` per request context |
| user scripts and worlds (`addUserScript`, world-scoped `executeJavaScript`, `registerScriptMessage`) | CDP over `execute_dev_tools_method`: `Page.addScriptToEvaluateOnNewDocument` (worldName), `Page.createIsolatedWorld`, `Runtime.evaluate` with executionContextId; script messages ride `Runtime.addBinding` and `Runtime.bindingCalled` |
| context menus (`contextMenuMode`, `setContextMenuItems`) | `on_before_context_menu` mutates the model (native mode); `run_context_menu` returns 1 and the host shows its own menu (suppress mode reuses the existing app path) |
| downloads (`downloadRequested`) | `on_before_download` cancels and emits, same shape as WebKit |
| find (`findStart`/`findNext`/...) | `cef_browser_host` find plus `cef_find_handler` |
| `securityChanged` | `cef_ssl_status_t` via navigation entry |
| favicons | `on_favicon_urls_change` plus `download_image` |
| JS round trip (`executeJavaScript` main world) | CDP `Runtime.evaluate`, promise-correlated like today |

## Packaging (`cef-pkg` lane)

- `nativedesktop.config.ts`: `webview: { engine: { mac?: "system"|"chromium",
  linux?: "system"|"chromium" }, cef?: { version?, locales? } }`. Dev override:
  `ND_WEBVIEW_ENGINE=chromium`.
- Schema seam: `engine` create-only prop on WebView ("system" default,
  "chromium"), codegen arms route it into the two hand-written engine files.
- `nd package` with CEF enabled: resolve the pinned version against
  `cef-builds.spotifycdn.com/index.json`, verify sha1, cache the tarball in
  `~/.cache/nativedesktop/cef/`, stage into the bundle. Linux: `strip` libcef.so
  (1.43 GB to ~269 MB), trim `locales/` to the config list (default en-US),
  ship `chrome-sandbox` mode 4755 alongside (userns fallback is automatic;
  never `--no-sandbox`). macOS: framework in `Versions/A` symlink layout
  (Xcode 26 signing requires it), five helper apps, JIT entitlements on
  renderer/GPU helpers, inside-out signing order before the outer deep-sign.
- Zero bytes when disabled stays a product claim; packaging must prove it
  (a doctor check: an engine=system app bundle contains no CEF).

## Milestones and gates

- M1 (per platform): a page renders in an embedded CEF view inside the existing
  host window; url/title/loading/progress/canGoBack/Forward events flow; the
  no-stray-window invariant holds for popups. Gate: `webview-probe` example
  boots with `ND_WEBVIEW_ENGINE=chromium`.
- M2: the contract table above; `scripts/webview-drive.ts` passes with the CEF
  engine on both platforms (Linux under weston `--xwayland`).
- M3: NativeBrowser's extension broker on the CDP substrate; the app's
  extensions drive green on CEF.
- M4: packaging end to end (`nd package linux|mac` with engine enabled), signed
  and doctor-checked.
- M5: NativeBrowser ships engine=chromium on both platforms; full app drive
  suite green; NativeDesktop release to npm; app repointed to npm packages.
