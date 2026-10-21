+++
title = "Webview"
description = "Show web pages in a window with WebView and webview(): the app-owned handle, commands, events, WKWebView and CEF, the Linux setup, and what Windows doesn't have yet."
+++

A page is a `WebView` handle that your app owns. The `webview(id, &page)` element only shows it.
The whole design rests on that split. Switching tabs or moving the element to another window
never reloads the page, because the element doesn't hold it.

<!-- docs:example browser -->

On macOS, the framework ships WKWebView and, as an opt-in, Chromium through CEF. Linux renders
with Chromium ([Linux](#linux)). The Windows engine isn't written yet
([what's missing](#windows)).

## The handle and the element

Create a page, keep it in your state, and mount it:

<!-- docs:snippet examples/browser/src/lib.rs#options -->

`WebView::with_options(options())` makes the handle, and `page.load(url)` starts it.
`WebView::new()` and `WebView::with_url(url)` are the short forms. You set the options when you
create the page, and they don't change afterward:

| Option | Effect |
| --- | --- |
| `.profile(..)` | `Default`, `Private` (memory only, gone with the last page), or `Named(name)` (its own persistent store) |
| `.engine(..)` | `System` or `Cef` for this page |
| `.adopt_popups()` | `window.open` hands you a live page that keeps `window.opener`, instead of only a URL |
| `.inspectable(..)` | Allows Web Inspector or DevTools. On by default in debug builds. |
| `.user_agent(..)` | The user agent string |

Put the element in your render, keyed by an id that you choose, with a handler for each event
that you need:

<!-- docs:snippet examples/browser/src/lib.rs#mount -->

Only one element can mount a page at a time, and the last one to mount it wins. Unmounting parks
the page: it keeps its state and keeps loading. The page closes when the last clone of the handle
drops. Clones share the page and compare with `==`.

## Commands

Commands are methods on the handle. You can call them before the page has a view. They queue and
replay in order once it has one. The browser example maps its toolbar actions onto them:

<!-- docs:snippet examples/browser/src/lib.rs#commands -->

The commands are `load`, `load_html`, `go_back`, `go_forward`, `reload`, `stop`, `set_zoom`,
`set_user_agent`, `open_dev_tools`, `find` with `find_next`, `find_previous`, and `stop_find`,
`set_muted`, `focus`, `request_close`, `start_download`, `save_session` and `restore_session`,
the cookie calls, and the user script and script message calls described later on this page.
State reads return immediately: `url()`, `title()`, `is_loading()`, `can_go_back()`, and
`can_go_forward()`.

A command that has an answer takes a callback instead of returning a value. nd has no executor,
so the answer arrives as an `FnOnce(T, &mut App)`, as it does with the dialogs:

```rust
page.execute_javascript("document.title", |title, cx| { /* Result<JsValue, JsError> */ });
```

A thrown exception arrives as an `Err` with its message, and the framework awaits a returned
promise. `get_cookies` and `save_session` answer the same way. `execute_javascript_in` runs in a
named isolated world.

## Events

The handlers are `on_navigate`, `on_title_changed`, `on_loading_changed`, `on_load_progress`,
`on_load_failed`, `on_new_window`, `on_window_closed`, `on_download_requested`,
`on_download_updated`, `on_context_menu_item_clicked`, `on_script_message`, and the rest of
those in [events.rs](../../../crates/nd-webview/src/events.rs). Each one delivers on the UI
thread, never inside an engine callback.

### Popups and tabs

With `adopt_popups`, `NewWindow` carries `popup: Some(page)`, a page that the engine already
started with `window.opener` intact. Add it as a tab and mount it. Without a popup, you get the
URL and the `Disposition` (such as `BackgroundTab`), and you open a page yourself.

<!-- docs:snippet examples/browser/src/lib.rs#popups -->

### Downloads

A response that the engine doesn't render, or an attachment, arrives as a `Download` that waits
for you. Choose where it goes with `save_to(path)` or `save_to_downloads()`, or call `cancel()`.
`on_download_updated` then reports `Running`, `Done`, `Failed`, or `Cancelled` with the byte
counts.

<!-- docs:snippet examples/browser/src/lib.rs#downloads -->

## Requests the app can leave unanswered

Permission prompts (camera, microphone, geolocation, notifications, and clipboard), custom scheme
requests, and downloads each wait for an answer through a responder on the event. If the app
drops the event without answering, the engine falls back instead of leaving the page waiting.
WebKit shows its own permission prompt, a scheme request fails, and a download is canceled. A
page never hangs because the app registered no handler.

To answer a permission request, handle `on_permission_request`, and call `respond(decision)` on
its responder. To serve a custom scheme, register it with `nd::webview::register_scheme("app")`
before you create the first page, and then answer it through `on_scheme_request`.

## Scripts and messages

`add_user_script(UserScript::new(id, source))` injects a script into every page load, optionally
in an isolated world. `register_script_message("name", None)` lets the page call
`window.webkit.messageHandlers.name.postMessage(value)` on both engines, and
`on_script_message` receives the `ScriptMessage { name, body, .. }` as JSON.

## Context menus

By default, the page shows the engine's own context menu. `context_menu_items(vec![..])` merges
your items into it in place, filtered by `Contexts` (such as link or image). When the user
chooses one of your items, it arrives as `ContextMenuItemClicked` with the page and link URLs.
`context_menu_mode` can suppress the engine's menu or leave it native. The browser example adds
**Open Link in New Tab** this way.

## Engines

| | System | CEF |
| --- | --- | --- |
| macOS | WKWebView, an `NSView` beside the window's other views | Chromium rendering off-screen, composited into the window |
| Windows | Not written yet | Not packaged yet |
| Linux | Chromium (the same as CEF; see [Linux](#linux)) | Chromium rendering off-screen, drawn by the window |
| Web | Not available | Not available |

The default engine is `[webview] engine` in `nativedesktop.toml`, set to `"system"` or `"cef"`.
The choice is fixed at build time. To override it for one page, use `WebViewOptions::engine`. To
override it for a whole build and run, set `ND_WEBVIEW_ENGINE=cef`, which `nd package` also
honors. `engine = "cef"` needs nd's `cef` feature. Without that feature, the app carries no
Chromium bytes, and packaging bundles Chromium only when the setting asks for it. The browser
example sets `engine = "system"` in its
[nativedesktop.toml](../../../examples/browser/nativedesktop.toml) and builds with CEF through the
environment variable.

`[webview.cef] style = "chrome"` hosts CEF pages in Chrome style instead of off-screen, for
Chromium's extension runtime and `chrome://` pages. `WebViewOptions::cef_style` overrides it per
page and `ND_CEF_STYLE` per build and run. The page is a windowed browser inside the page's
element: on macOS Chromium's views move into the page's `NSView`, on Linux Chromium's X11 window
is a child of the app's window. Chrome's own browser UI stays hidden. The commands that would
show it (new tab, history, downloads, bookmarks, tab search, settings) reach the app as
`on_chrome_command` with a `BrowserCommand`, page commands such as back and zoom run, and the rest
are refused. On Linux an app whose default style is Chrome puts its windows on X11 (XWayland under
a Wayland compositor). A Chrome-style page in a Wayland window falls back to off-screen with a
warning.

WKWebView's inspector opens through the private `_inspector` SPI, as wry does. Without it, you can
still inspect the page from Safari.

## Overlays over a page

A page is a native view, so a drawn element would normally sit under it. Popovers, menus, and
panels sit above web views and never pass input through to them. For details, see
[overlays](overlays.md#input-never-leaks). In the overlays example,
`native_surface(..).count_input()` counts the input that a page received, which is how the leak
gate proves the rule.

## Linux

Linux has no OS engine that embeds in a GPUI Wayland window: WPE WebKit isn't in nixpkgs, and
webkitgtk can't draw into a window it doesn't own. Web views render with Chromium (CEF)
off-screen, and the window draws the frames. `Engine::System` resolves to `Engine::Cef` on Linux,
so `.engine(Engine::System)` and the default both run Chromium, and the app code is the same as on
macOS. The decision is in [the architecture plan](../../plan/ARCHITECTURE.md#81-linux-renders-web-views-with-cef-decision-2026-10-20).

### Turn it on

1. Enable nd's `webview` feature in the app's `Cargo.toml`. On Linux it brings in Chromium
   (`nd-os-linux/cef`); no `[webview] engine` setting is needed.
2. For a Nix build, `mkApp` reads the same feature and its `webview` argument (true when `nd` has
   the `webview` feature). It builds against the CEF distribution from
   [nix/cef.nix](../../../nix/cef.nix), which must match the version of the `cef` crate in
   `crates/nd-cef/Cargo.toml`, and it keeps `libcef.so` and its resources beside the executable
   in `libexec/<pname>`.
3. For a dev run outside Nix, use the dev shell: it sets `CEF_PATH`, carries cmake and ninja for
   the C++ wrapper, and puts the libraries below on `LD_LIBRARY_PATH`.
4. `ND_WEBVIEW_ENGINE=cef` and `[webview] engine = "cef"` are accepted and change nothing on
   Linux. `ND_WEBVIEW_ENGINE=system` also resolves to Chromium.

The executable's run path is `$ORIGIN`, so `libcef.so` loads from beside the binary. The app is
its own helper process; there is no separate helper binary on Linux.

`nd doctor` checks that the runtime beside a built executable is complete and names what's missing.

### Package it

`nd package linux` detects that the executable links `libcef.so` and copies the runtime from the
build output into the AppDir's binary directory: `libcef.so`, `icudtl.dat`,
`v8_context_snapshot.bin`, `resources.pak`, `chrome_100_percent.pak`, `chrome_200_percent.pak`,
`libvk_swiftshader.so`, `vk_swiftshader_icd.json`, `libvulkan.so.1`, and `locales/`. It fails if
any are missing. Velopack then packs the AppImage. Expect roughly the size of the CEF
distribution added to the package.

### Libraries the host needs

The AppImage doesn't bundle the system libraries `libcef.so` links. The host must have:

- nss and nspr
- gtk3, pango, cairo, glib, atk, and at-spi2-atk with at-spi2-core
- cups
- libdrm and libgbm
- alsa-lib
- dbus, expat, libxkbcommon, libGL, udev, and the X libraries (libxcb, libX11, libXcomposite,
  libXdamage)

The list mirrors `cefLibs` in [flake.nix](../../../flake.nix). Debian package names are
`libnss3`, `libgtk-3-0`, `libatk-bridge2.0-0`, `libcups2`, `libdrm2`, `libgbm1`, and
`libasound2`. The packager doesn't write a `.deb` yet (see [packaging](packaging.md#linux)), so a
`.deb` you build yourself should declare these as dependencies.

## Windows

`webview()` renders a notice with the reason, and every command fails with that reason. The
planned engine is WebView2 in composition mode. Write the app against the handle as you would on
macOS, and it works once the engine lands. Until then, test the web parts on macOS or Linux.
