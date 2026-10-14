+++
title = "Web"
description = "Build the same app for the browser with nd build --target web: one canvas, the plain GPUI look, and checks for what a browser cannot do."
+++

An app that builds for the desktop builds for the web unchanged. `nd build --target web` compiles
it to `wasm32-unknown-unknown`, and the page draws it on one canvas through WebGPU, with a WebGL2
fallback. The hello, overrides, gallery and scroll examples run this way, and the embeds on this
site run the live build.

<!-- docs:example hello -->

## Build and serve

```sh
nd build --target web -p hello       # target/nd/web/hello/
nd serve --target web -p hello       # build, then serve on 127.0.0.1:8080
```

The bundle is `index.html`, the wasm, its JavaScript loader and the app's `assets/` folder.
`--out` picks another folder (it refuses to clear one that holds something else), `--debug` skips
optimization, `--port` and `--no-build` apply to `serve`. `wasm-opt -Oz` runs when it is on the
`PATH`. `nd` installs a `wasm-bindgen` CLI that matches the locked `wasm-bindgen` crate into
`~/.cache/nd/tools`, never system-wide, or uses the one named by `ND_WASM_BINDGEN`. The app has to
depend on `wasm-bindgen`.

`[web] base_path` in [nativedesktop.toml](settings-and-config.md#app-manifest) adds a `<base>` tag
for an app served under a sub-path such as `/notes/`. There is no packaging step beyond this
folder: `nd package` builds installers for the three desktop systems only, and any static host
serves the bundle.

## What it looks like

The web draws the plain default GPUI look. It imitates no OS: there is no macOS, Fluent or
GNOME skin in the bundle, and the fonts are the bundled Noto Sans and Noto Sans Mono. The same
code stays native on each desktop. Per-platform closures reflect that: `.on_macos`, `.on_windows`
and `.on_linux` do not run on the web, and `.on_web` does.

<!-- docs:snippet examples/overrides/src/lib.rs#per-platform -->

## One canvas

The browser has one canvas, so every window of the app lives inside it. The first window fills
the canvas. Later windows float over it with a drawn title row, sheets sit over a scrim, and each
one blocks input to what is below it. The menu bar is a drawn row with dropdowns, and context
menus are drawn overlays. Window tabs are separate OS windows on the desktop and have no web
counterpart (see [windows and tabs](windows-and-tabs.md)).

## Feature checks

Not every API can work in a browser. `nd::platform::has(Feature)` answers at runtime and needs no
window or context, because the target decides:

<!-- docs:snippet examples/system/src/lib.rs#has -->

On the web only `Feature::Clipboard` is true: no file system, tray, notifications, credentials,
threads, CEF, webview, updates or IPC, and no native-only widgets. Clipboard reads may still need
a user gesture. The desktops answer true for everything except `MenuBar` and `NativeOnlyWidgets`
(Linux and Windows), and `Audio` (Windows).

An API that cannot work on the web returns `Err(Unsupported)` there and carries
`#[nd::unavailable(web)]`; the attribute takes `web`, `macos`, `windows` or `linux`. Calling it
without a `has` check is a warning under `nd check --target web`.

## Check the app builds for the browser

```sh
nd check --target web
```

Besides the usual lints it denies what compiles on wasm and panics when it runs: `std::time::Instant`
and `SystemTime` (rule `web-std-time`), `std::thread` (`web-std-thread`) and `std::fs`
(`web-std-fs`). It warns on each call to a `#[nd::unavailable(web)]` item that has no `has`
check (`web-unavailable`), and runs clippy for `wasm32-unknown-unknown` on the library and
binaries. `// nd-allow(rule): reason` silences one finding.

Use `nd::time::{Instant, SystemTime, UNIX_EPOCH}` for clocks: they are `web-time`'s types in the
browser and std's elsewhere. For storage use [`Store<T>`](settings-and-config.md#machine-state)
and the file dialogs, and the executors for async work.

## Settings and storage

`nd-settings` and `Store<T>` keep every file in `localStorage` under the file's path, with the same
API as on disk. There is no file watcher and no Nix layer. `nd::files` and `nd::clipboard` work on
every target: file open is a hidden `<input type=file>`, save is a download, and the clipboard is
the async Clipboard API.

## Tests

Tier 1 tests run in a browser too. `#[nd::test]` is libtest on the desktop and
`wasm-bindgen-test` in the browser, so one test body runs in both; `scripts/web-test hello
gallery` runs the tier 1 suites in headless Chrome and fetches a matching chromedriver into the
same tools folder. `hello` also has a golden of the web look, light and dark, drawn by headless
Chrome from a fresh `nd build --target web`:

<!-- docs:snippet examples/hello/tests/web_golden.rs#bless -->

It runs only with `ND_WEB_GOLDEN=1` (`just web-test` sets it). More in
[testing](../tooling/testing.md).

## Gaps

- `webview()` is not available. The iframe host waits for the native surface seam of the overlays
  work, and CEF is desktop only.
- Image URLs fail on the web until a fetch lands there.
- Diagnostics are the console panic hook only: no flight recorder, bundles or crash helper.
  `nd::report_issue` returns `None`.
- Updates and theme watchers are compiled out, so a web app updates by redeploying.
- `nd-data` (SQLite) and the tokio runtime helper are not available.
- Tray, notifications, credentials, media session and audio are desktop services (see
  [system](system.md#what-works-where)).
