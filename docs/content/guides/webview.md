+++
title = "Webview"
description = "Show web pages in a window with WebView and webview(): the app-owned handle, commands, events, WKWebView and CEF, and what Linux and Windows do not have yet."
+++

A page is a `WebView` handle that your app owns. The `webview(id, &page)` element only shows it.
That split is the whole design: switching tabs or moving the element to another window never
reloads the page, because the element is not what holds it.

<!-- docs:example browser -->

On macOS this is shipped with WKWebView and, as an opt-in, Chromium through CEF. On Linux and
Windows the engines are not written yet, see [what is missing](#linux-and-windows).

## The handle and the element

Create a page, keep it in your state, and mount it:

<!-- docs:snippet examples/browser/src/lib.rs#options -->

`WebView::with_options(options())` makes the handle and `page.load(url)` starts it. `WebView::new()`
and `WebView::with_url(url)` are the short forms. The options are fixed at creation:

| Option | Effect |
| --- | --- |
| `.profile(..)` | `Default`, `Private` (memory only, gone with the last page) or `Named(name)` (its own persistent store) |
| `.engine(..)` | `System` or `Cef` for this page |
| `.adopt_popups()` | `window.open` hands you a live page that keeps `window.opener`, not only a URL |
| `.inspectable(..)` | allows Web Inspector or DevTools, on by default in debug builds |
| `.user_agent(..)` | the user agent string |

The element goes in your render, keyed by an id of your choosing, with a handler for each event
you care about:

<!-- docs:snippet examples/browser/src/lib.rs#mount -->

A page can be mounted by one element at a time, the last one. Unmounting parks it: it keeps its
state and keeps loading. It closes when the last clone of the handle drops. Clones share the
page and compare with `==`.

## Commands

Commands are methods on the handle, and they are safe to call before the page has a view. They
queue and replay in order once it does. The browser example maps its toolbar actions onto them:

<!-- docs:snippet examples/browser/src/lib.rs#commands -->

The set: `load`, `load_html`, `go_back`, `go_forward`, `reload`, `stop`, `set_zoom`,
`set_user_agent`, `open_dev_tools`, `find` with `find_next`, `find_previous` and `stop_find`,
`set_muted`, `focus`, `request_close`, `start_download`, `save_session` and `restore_session`,
the cookie calls, and the user script and script message calls below. State reads are
immediate: `url()`, `title()`, `is_loading()`, `can_go_back()`, `can_go_forward()`.

Anything that has an answer takes a callback instead of returning. nd has no executor, so the
answer arrives as an `FnOnce(T, &mut App)`, as with the dialogs:

```rust
page.execute_javascript("document.title", |title, cx| { /* Result<JsValue, JsError> */ });
```

A thrown exception is an `Err` with its message and a returned promise is awaited.
`get_cookies` and `save_session` answer the same way. `execute_javascript_in` runs in a named
isolated world.

## Events

Handlers are `on_navigate`, `on_title_changed`, `on_loading_changed`, `on_load_progress`,
`on_load_failed`, `on_new_window`, `on_window_closed`, `on_download_requested`,
`on_download_updated`, `on_context_menu_item_clicked`, `on_script_message` and the rest of
those in [events.rs](../../../crates/nd-webview/src/events.rs). Each delivers on the UI thread,
never inside an engine callback.

### Popups and tabs

With `adopt_popups`, `NewWindow` carries `popup: Some(page)`: a page the engine already
started, with `window.opener` intact. Add it as a tab and mount it. Without a popup you get the
URL and the `Disposition`, such as `BackgroundTab`, and open a page yourself.

<!-- docs:snippet examples/browser/src/lib.rs#popups -->

### Downloads

A response the engine does not render, or an attachment, arrives as a `Download` that waits for
you. Choose where it goes with `save_to(path)` or `save_to_downloads()`, or `cancel()` it. `on_download_updated` then reports
`Running`, `Done`, `Failed` or `Cancelled` with the byte counts.

<!-- docs:snippet examples/browser/src/lib.rs#downloads -->

## Requests the app can leave unanswered

Permission prompts (camera, microphone, geolocation, notifications, clipboard), custom scheme
requests and downloads each wait for an answer through a responder on the event. If the app
drops the event without answering, the engine falls back instead of leaving the page waiting:
WebKit shows its own permission prompt, a scheme request fails, a download is cancelled. A page
never hangs on an app that registered no handler.

To answer a permission request, handle `on_permission_request` and call `respond(decision)` on
its responder. A custom scheme is registered with `nd::webview::register_scheme("app")` before the
first page is created and served through `on_scheme_request`.

## Scripts and messages

`add_user_script(UserScript::new(id, source))` injects a script into every page load, optionally
in an isolated world. `register_script_message("name", None)` lets the page call
`window.webkit.messageHandlers.name.postMessage(value)` on both engines, and
`on_script_message` receives the `ScriptMessage { name, body, .. }` as JSON.

## Context menus

By default the page shows the engine's own context menu. `context_menu_items(vec![..])` merges
your items into it in place, filtered by `Contexts` (link, image and so on), and a chosen one
arrives as `ContextMenuItemClicked` with the page and link URLs. `context_menu_mode` can
suppress the engine's menu or leave it native. The browser example adds "Open Link in New Tab"
this way.

## Engines

| | System | CEF |
| --- | --- | --- |
| macOS | WKWebView, an `NSView` beside the window's other views | Chromium rendering off-screen, composited into the window |
| Windows | not written yet | not packaged yet |
| Linux | not written yet | not packaged yet |
| Web | not available | not available |

The default engine is `[webview] engine` in `nativedesktop.toml`: `"system"` or `"cef"`. The
choice is baked at build time. Override it per page with `WebViewOptions::engine`, and for a
whole build and run with `ND_WEBVIEW_ENGINE=cef`, which `nd package` also honours. `engine =
"cef"` needs nd's `cef` feature; without it the app carries no Chromium bytes, and packaging
bundles Chromium only when the setting asks for it. The browser example sets `engine = "system"`
in its [nativedesktop.toml](../../../examples/browser/nativedesktop.toml) and builds with CEF
through the environment variable.

`[webview.cef] style = "chrome"`, a windowed Chrome-style mode for extensions, is accepted by the
config but not implemented. Only the off-screen style runs.

WKWebView's inspector opens through the private `_inspector` SPI, as wry does. Without it the
page stays inspectable from Safari.

## Overlays over a page

A page is a native view, so a drawn element would normally sit under it. Popovers, menus and
panels are above web views and never pass input through to them. See
[overlays](overlays.md#input-never-leaks). `native_surface(..).count_input()` in the overlays
example counts what a page received, which is how the leak gate proves it.

## Linux and Windows

On both, `webview()` renders a notice with the reason, and every command fails with it. The
planned engines are WebView2 in composition mode on Windows and WPE WebKit on Linux, with CEF as
the fallback if WPE does not embed. Nothing of the engine matrix runs there. Write the app
against the handle as on macOS and it works once the engines land; until then, test the web
parts on macOS.
