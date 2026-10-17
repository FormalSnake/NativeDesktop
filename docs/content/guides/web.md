+++
title = "Web"
description = "Build the same app for the browser with nd build --target web: one canvas, the plain GPUI look, and checks for what a browser can't do."
+++

An app that builds for the desktop also builds for the web, unchanged. `nd build --target web`
compiles it to `wasm32-unknown-unknown`, and the page draws it on one canvas through WebGPU, with
a WebGL2 fallback. The hello, overrides, gallery, and scroll examples run this way, and the embeds
on this site run the live build.

<!-- docs:example hello -->

## Build and serve

```sh
nd build --target web -p hello       # target/nd/web/hello/
nd serve --target web -p hello       # build, then serve on 127.0.0.1:8080
```

The bundle is `index.html`, the wasm, its JavaScript loader, and the app's `assets/` folder.
`--out` picks another folder, and it refuses to clear a folder that holds something else.
`--debug` skips optimization. `--port` and `--no-build` apply to `serve`. `wasm-opt -Oz` runs when
it's on the `PATH`. `nd` installs a `wasm-bindgen` CLI that matches the locked `wasm-bindgen`
crate into `~/.cache/nd/tools`, never system-wide, or it uses the one that `ND_WASM_BINDGEN`
names. The app has to depend on `wasm-bindgen`.

`[web] base_path` in [nativedesktop.toml](settings-and-config.md#app-manifest) adds a `<base>` tag
for an app served under a sub-path such as `/notes/`. There's no packaging step beyond this
folder. `nd package` builds installers only for the three desktop systems, and any static host can
serve the bundle.

## What it looks like

The web draws the plain default GPUI look. It doesn't imitate an OS: the bundle has no macOS,
Fluent, or GNOME skin, and the fonts are the bundled Noto Sans and Noto Sans Mono. The same code
stays native on each desktop. Per-platform closures follow the same rule: `.on_macos`,
`.on_windows`, and `.on_linux` don't run on the web, and `.on_web` does.

<!-- docs:snippet examples/overrides/src/lib.rs#per-platform -->

## One canvas

The browser has one canvas, so every window of the app lives inside it. The first window fills
the canvas. Later windows float over it with a drawn title row, sheets sit over a scrim, and each
one blocks input to what's below it. The menu bar is a drawn row with dropdowns, and context
menus are drawn overlays. On the desktop, window tabs are separate OS windows, and they have no
web equivalent (see [windows and tabs](windows-and-tabs.md)).

## Feature checks

Some APIs can't work in a browser. `nd::platform::has(Feature)` answers at runtime and needs no
window or context, because the target decides the answer:

<!-- docs:snippet examples/system/src/lib.rs#has -->

On the web, only `Feature::Clipboard` is true. There's no file system, tray, notifications,
credentials, threads, CEF, webview, updates, or IPC, and no native-only widgets. Clipboard reads
can still need a user gesture. The desktops answer true for everything except `MenuBar` and
`NativeOnlyWidgets` (Linux and Windows), and `Audio` (Windows).

An API that can't work on the web returns `Err(Unsupported)` there and carries
`#[nd::unavailable(web)]`. The attribute takes `web`, `macos`, `windows`, or `linux`. If you call
such an API without a `has` check, `nd check --target web` warns.

## Check the app builds for the browser

```sh
nd check --target web
```

Besides the usual lints, it denies code that compiles on wasm but panics when it runs: `std::time::Instant`
and `SystemTime` (rule `web-std-time`), `std::thread` (`web-std-thread`), and `std::fs`
(`web-std-fs`). It warns on each call to a `#[nd::unavailable(web)]` item that has no `has`
check (`web-unavailable`), and it runs clippy for `wasm32-unknown-unknown` on the library and
binaries. To silence one finding, add `// nd-allow(rule): reason`.

For clocks, use `nd::time::{Instant, SystemTime, UNIX_EPOCH}`. They're `web-time`'s types in the
browser and std's types elsewhere. For storage, use [`Store<T>`](settings-and-config.md#machine-state)
and the file dialogs. For async work, use the executors.

## Settings and storage

`nd-settings` and `Store<T>` keep every file in `localStorage` under the file's path, with the
same API as on disk. There's no file watcher and no Nix layer. `nd::files` and `nd::clipboard`
work on every target. In the browser, opening a file uses a hidden `<input type=file>`, saving one
starts a download, and the clipboard is the async Clipboard API.

## Tests

Tier 1 tests also run in a browser. `#[nd::test]` is libtest on the desktop and
`wasm-bindgen-test` in the browser, so one test body runs in both. `scripts/web-test hello
gallery` runs the tier 1 suites in headless Chrome, and it fetches a matching chromedriver into
the same tools folder. `hello` also has a golden of the web look, in light and dark, that headless
Chrome draws from a fresh `nd build --target web`:

<!-- docs:snippet examples/hello/tests/web_golden.rs#bless -->

It runs only with `ND_WEB_GOLDEN=1` (`just web-test` sets it). For more, see
[testing](../tooling/testing.md).

## Gaps

- `webview()` isn't available. The iframe host waits on the native surface seam from the overlays
  work, and CEF is desktop only.
- Image URLs fail on the web until fetch support lands there.
- Diagnostics are only the console panic hook. There's no flight recorder, no bundles, and no
  crash helper. `nd::report_issue` returns `None`.
- Updates and theme watchers are compiled out, so you update a web app by redeploying it.
- `nd-data` (SQLite) and the tokio runtime helper aren't available.
- Tray, notifications, credentials, media session, and audio are desktop services (see
  [system](system.md#what-works-where)).
