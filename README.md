# NativeDesktop

NativeDesktop (`nd`) is a Rust toolkit for desktop apps. You describe a window as one tree of views and elements, and each platform's backend turns it into what that OS draws. Your app never names a backend. Desktop only: phones are left to React Native.

| Target | Backend |
| --- | --- |
| macOS | AppKit: a real `NSToolbar`, `NSButton`, `NSTableView` |
| Linux | GPUI with a libadwaita look and live matugen colors |
| Windows | GPUI with a Fluent skin (parts of the skin are still missing) |
| Web | GPUI compiled to WebAssembly |

Web pages show in a window through `webview()`, on WKWebView or, opt-in, CEF. Apps built on it: messages, formalmusic (YouTube Music on Kopuz) and nativebrowser.

Docs: https://native.formalsnake.dev (the sources are in [docs/content](docs/content)).

## Getting started

Releases are git tags, not crates.io versions, because nd builds on a patched GPUI. You need rustup, git and, on macOS, Xcode or the Command Line Tools. Install the CLI from a tag:

```sh
cargo install cargo-nextest --locked
cargo install --git https://github.com/FormalStudio/NativeDesktop --tag v0.6.1 --locked nd-cli
```

Create an app, run it, check it and test it:

```sh
nd new demo
cd demo
just dev          # runs cargo run
nd check          # fmt, clippy, framework lints, appearance audit
nd test           # tiers 1 and 2, no window
nd serve --target web
```

`nd serve --target web` builds the app to WebAssembly and serves it on `http://127.0.0.1:8080/`. `nd package` builds installers and `nd upgrade` moves an app to a newer tag. [docs/content/tooling/cli.md](docs/content/tooling/cli.md) lists every subcommand, and [docs/content/getting-started.md](docs/content/getting-started.md) has the Windows setup.

## Hello world

This is the counter from [examples/hello](examples/hello/src/lib.rs), the same view `nd new` scaffolds:

```rust
use nd::prelude::*;

pub struct Counter {
    count: u32,
}

impl Render for Counter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        scaffold()
            .toolbar(toolbar().item(button("add", "Add one").on_click(cx.listener(
                |this, _: &ClickEvent, _window, cx| {
                    this.count += 1;
                    cx.notify();
                },
            ))))
            .content(
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(text(self.count.to_string()).text_size(px(48.)).tabular_nums())
                    .child(text("clicks").text_color(theme.text.secondary)),
            )
    }
}
```

## Repo layout

- `crates/`: the framework. `nd` is the facade, `nd-backend-appkit` and `nd-backend-gpui` are the backends, `nd-cli` is the `nd` binary.
- `examples/`: runnable apps such as `hello`, `gallery` and `browser`. CI builds every one for every target.
- `docs/`: the docs site (`docs/app`, itself an nd app), its pages in `docs/content`, and the plan in `docs/plan`.

## Contributing

Dev setup for macOS and Linux, the test tiers and CI are in [docs/dev/setup.md](docs/dev/setup.md). Doc style is in [docs/WRITING.md](docs/WRITING.md). Release notes are in [CHANGELOG.md](CHANGELOG.md).

## License

MIT OR Apache-2.0, as declared in [Cargo.toml](Cargo.toml).
