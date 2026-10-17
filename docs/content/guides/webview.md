+++
title = "Webview"
description = "Show web pages in a window with WebView and webview(): the app-owned handle, commands, events, WKWebView and CEF, and what Linux and Windows don't have yet."
+++

A page is a `WebView` handle that your app owns. The `webview(id, &page)` element only shows it.
The whole design rests on that split. Switching tabs or moving the element to another window
never reloads the page, because the element doesn't hold it.

<!-- docs:example browser -->

On macOS, the framework ships WKWebView and, as an opt-in, Chromium through CEF. The Linux and
Windows engines aren't written yet. For details, see [what's missing](#linux-and-windows).

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
| Linux | Not written yet | Not packaged yet |
| Web | Not available | Not available |

The default engine is `[webview] engine` in `nativedesktop.toml`, set to `"system"` or `"cef"`.
The choice is fixed at build time. To override it for one page, use `WebViewOptions::engine`. To
override it for a whole build and run, set `ND_WEBVIEW_ENGINE=cef`, which `nd package` also
honors. `engine = "cef"` needs nd's `cef` feature. Without that feature, the app carries no
Chromium bytes, and packaging bundles Chromium only when the setting asks for it. The browser
example sets `engine = "system"` in its
[nativedesktop.toml](../../../examples/browser/nativedesktop.toml) and builds with CEF through the
environment variable.

The config accepts `[webview.cef] style = "chrome"`, a windowed Chrome-style mode for extensions,
but it isn't implemented. Only the off-screen style runs.

WKWebView's inspector opens through the private `_inspector` SPI, as wry does. Without it, you can
still inspect the page from Safari.

## Overlays over a page

A page is a native view, so a drawn element would normally sit under it. Popovers, menus, and
panels sit above web views and never pass input through to them. For details, see
[overlays](overlays.md#input-never-leaks). In the overlays example,
`native_surface(..).count_input()` counts the input that a page received, which is how the leak
gate proves the rule.

## Linux and Windows

On both platforms, `webview()` renders a notice with the reason, and every command fails with
that reason. The planned engines are WebView2 in composition mode on Windows and WPE WebKit on
Linux, with CEF as the fallback if WPE doesn't embed. No part of the engine matrix runs there yet.
Write the app against the handle as you would on macOS, and it works once the engines land. Until
then, test the web parts on macOS.
