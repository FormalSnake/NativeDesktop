+++
title = "Getting started"
description = "Install the nd CLI, create an app, run it on the desktop and in a browser, and run its tests."
+++

This page takes you from installing the `nd` CLI to a running, tested app.

<!-- docs:example hello -->

## Install

Releases are git tags on [github.com/FormalSnake/NativeDesktop](https://github.com/FormalSnake/NativeDesktop),
not crates.io: nd builds on a patched GPUI, and crates.io takes no git or patched dependencies.
You need [rustup](https://rustup.rs), git, and `cargo-nextest` (`nd test` runs through it). The
terminal widget (`nd::terminal`) also needs Zig 0.16 on `PATH`. Then install the CLI from the
release tag. It installs a binary named `nd`:

```sh
cargo install cargo-nextest --locked
cargo install --git https://github.com/FormalSnake/NativeDesktop --tag v0.6.0 --locked nd-cli
```

`--locked` builds with the versions nd was tested with; without it cargo takes the newest
releases, some of which have been yanked since. On macOS, install Xcode or the Command Line Tools
first (`xcode-select --install`): the AppKit backend links Swift. In a clone of the repository,
`nix develop` gives you all of it, and `cargo install --path crates/nd-cli` installs the CLI from
the checkout.

### Windows

In PowerShell:

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install --id Rustlang.Rustup
winget install --id Git.Git
```

The first line installs the MSVC compiler, linker and Windows SDK that the
`x86_64-pc-windows-msvc` toolchain links with. Open a new terminal so `cargo` and `git` are on
`PATH`, then run the two `cargo install` lines above. `nd package windows` also needs the .NET
SDK (`winget install --id Microsoft.DotNet.SDK.8`), which it uses to fetch Velopack's `vpk`.

A debug build of an app and its tests fills several gigabytes under `target`. If `C:` is tight,
set `CARGO_TARGET_DIR` to a folder on another drive.

## Create an app

```sh
nd new demo
```

The project depends on nd by the CLI's own release tag (`tag = "v0.6.0"` for CLI 0.6.0), and its
`[patch.crates-io]` block points the GPUI crates at the fork vendored in the same tag. `nd new`
also writes nd's `Cargo.lock`, so the first build resolves to the versions nd was released with.
Add `--standalone` when the new directory sits inside another Cargo workspace. The name must start
with a lowercase letter and may contain lowercase letters, digits, `-` and `_`. `--id` sets the
reverse-DNS app id; it defaults to `com.example.demo`. To work against a local checkout of nd
instead, pass `--nd-path /path/to/nativedesktop/crates/nd` (or set `ND_PATH`).

`nd new` writes these files and refuses a directory that is not empty:

| File | What it is |
| --- | --- |
| `src/app.rs` | The `Counter` view. This is the file you edit first. |
| `src/lib.rs`, `src/main.rs` | `app()`, `open()` and `run()`; `main` calls `run`. |
| `src/demo.rs` | Fixed state when `<NAME>_DEMO` is set (`DEMO_DEMO=1` here), for tests and screenshots. |
| `tests/app.rs` | A tier 1 test that clicks the button on the test backend. |
| `tests/appearance.rs` | The tier 2 appearance audit: one `.window(..)` per window you add. |
| `nativedesktop.toml` | App id, display name, update feed. |
| `build.rs` | One call, `nd::build::app()`, which links the app's Swift on macOS. |
| `justfile` | `dev`, `check` and `test` recipes. |
| `CLAUDE.md`, `README.md` | The rules an agent follows in this project, and a README with `nd:` placeholders. |
| `rustfmt.toml`, `clippy.toml`, `.config/nextest.toml` | Format, lint and test-tier settings. |

## Run it

```sh
cd demo
just dev
```

`just dev` is `cargo run`. On macOS the window is AppKit. On Linux and Windows it is the GPUI
backend. On a target with no backend, `run` returns `no platform backend is linked for this
target`.

## Check and test

```sh
nd check
nd test
```

`nd check` runs `cargo fmt --check`, clippy with the framework's deny table (`unwrap_used`,
`expect_used`, `dbg_macro`, `todo` and warnings), the framework lints, then the appearance audit.
`--no-audit` skips the audit and `--target web` adds the web lints. `nd test` runs tiers 1 and 2
through nextest with no window server. `nd test --tier real` runs tier 3, which opens real windows
one at a time. The [testing page](tooling/testing.md) covers the tiers.

## Keep nd up to date

```sh
nd upgrade
```

moves the `nd` dependencies and the GPUI patch block to the newest release tag, updates
`Cargo.lock` and prints the [changelog](https://github.com/FormalSnake/NativeDesktop/blob/main/CHANGELOG.md)
entries between the two versions. `--to v0.6.1` picks a tag. Install the matching CLI with the
same `cargo install --git ... --tag <tag> --locked nd-cli` line.

## Run it in a browser

```sh
nd serve --target web
```

This compiles the app to WebAssembly, runs `wasm-bindgen` and serves the bundle at
`http://127.0.0.1:8080/` (`--port` changes it). The first run installs a `wasm-bindgen-cli` that
matches your lockfile. `nd build --target web` writes the bundle without serving it. Some
framework calls do not exist in a browser; `nd check --target web` flags them, and the
[web guide](guides/web.md) covers the rest.

## The hello app

[Hello](../../examples/hello/src/lib.rs) is the scaffold's counter with a toolbar button. The
view is a struct and an `impl Render`:

<!-- docs:snippet examples/hello/src/lib.rs#counter -->

`scaffold()` is the window structure: a toolbar and a content area. `button("add", "Add one")` is
a toolbar item; the string `"add"` is its id, which tests use to find it. The click handler is
wrapped by `cx.listener` so it receives `&mut Counter`. `cx.notify()` marks this view dirty, and
the next frame runs `render` again. `cx.theme()` hands out colour tokens, so the secondary text
colour is not a literal.

Opening the window is two functions. `Application::new` takes the manifest that `build.rs` filled
from `nativedesktop.toml`, and `open` builds the root view as an entity:

<!-- docs:snippet examples/hello/src/lib.rs#open -->

`src/main.rs` is `hello::app().run(|cx| { hello::open(cx); })`. The test drives the same `open`:

<!-- docs:snippet examples/hello/tests/tier1.rs#tier1 -->

`#[nd::test]` runs natively and, compiled for `wasm32-unknown-unknown`, in headless Chromium.
Next: [the programming model](concepts/programming-model.md).
