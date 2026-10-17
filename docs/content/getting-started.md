+++
title = "Getting started"
description = "Install the nd CLI, create an app, run it on the desktop and in a browser, and run its tests."
+++

This page takes you from installing the `nd` CLI to a running, tested app.

<!-- docs:example hello -->

## Install

nd releases are git tags on [github.com/FormalSnake/NativeDesktop](https://github.com/FormalSnake/NativeDesktop),
not crates.io versions. nd builds on a patched GPUI, and crates.io doesn't accept git or patched
dependencies.

1. Install [rustup](https://rustup.rs) and git.
2. On macOS, install Xcode or the Command Line Tools (`xcode-select --install`). The AppKit
   backend links Swift.
3. If you use the terminal widget (`nd::terminal`), put Zig 0.16 on `PATH`.
4. Install `cargo-nextest`, which `nd test` runs through, and then the CLI from the release tag.
   The CLI installs a binary named `nd`.

```sh
cargo install cargo-nextest --locked
cargo install --git https://github.com/FormalSnake/NativeDesktop --tag v0.6.0 --locked nd-cli
```

`--locked` builds with the versions that nd was tested with. Without it, cargo takes the newest
releases, and some of those have since been yanked. In a clone of the repository, `nix develop`
gives you all of these tools, and `cargo install --path crates/nd-cli` installs the CLI from the
checkout.

### Windows

Run these commands in PowerShell:

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install --id Rustlang.Rustup
winget install --id Git.Git
```

The first command installs the MSVC compiler, the linker, and the Windows SDK that the
`x86_64-pc-windows-msvc` toolchain links with. Then:

1. Open a new terminal so that `cargo` and `git` are on `PATH`.
2. Run the two `cargo install` commands from the previous section.

`nd package windows` also needs the .NET SDK (`winget install --id Microsoft.DotNet.SDK.8`), which
it uses to fetch Velopack's `vpk`.

A debug build of an app and its tests fills several gigabytes under `target`. If `C:` is low on
space, set `CARGO_TARGET_DIR` to a folder on another drive.

## Create an app

```sh
nd new demo
```

The new project depends on nd at the CLI's own release tag (`tag = "v0.6.0"` for CLI 0.6.0), and its
`[patch.crates-io]` block points the GPUI crates at the fork vendored in the same tag. `nd new`
also writes nd's `Cargo.lock`, so the first build resolves to the versions nd was released with.
Add `--standalone` if the new directory is inside another Cargo workspace. The name must start
with a lowercase letter and can contain lowercase letters, digits, `-`, and `_`. `--id` sets the
reverse-DNS app id, which defaults to `com.example.demo`. To work against a local checkout of nd
instead, pass `--nd-path /path/to/nativedesktop/crates/nd` (or set `ND_PATH`).

`nd new` refuses a directory that isn't empty. It writes these files:

| File | What it is |
| --- | --- |
| `src/app.rs` | The `Counter` view. This is the file you edit first. |
| `src/lib.rs`, `src/main.rs` | `app()`, `open()` and `run()`; `main` calls `run`. |
| `src/demo.rs` | Fixed state when `<NAME>_DEMO` is set (`DEMO_DEMO=1` here), for tests and screenshots. |
| `tests/app.rs` | A tier 1 test that clicks the button on the test backend. |
| `tests/appearance.rs` | The tier 2 appearance audit: one `.window(..)` per window you add. |
| `nativedesktop.toml` | App id, display name, and update feed. |
| `build.rs` | One call, `nd::build::app()`, which links the app's Swift on macOS. |
| `justfile` | `dev`, `check`, and `test` recipes. |
| `CLAUDE.md`, `README.md` | The rules an agent follows in this project, and a README with `nd:` placeholders. |
| `rustfmt.toml`, `clippy.toml`, `.config/nextest.toml` | Format, lint, and test-tier settings. |

## Run it

```sh
cd demo
just dev
```

`just dev` runs `cargo run`. On macOS, the window uses AppKit. On Linux and Windows, it uses the
GPUI backend. On a target that has no backend, `run` returns `no platform backend is linked for this
target`.

## Check and test

```sh
nd check
nd test
```

`nd check` runs `cargo fmt --check`, clippy with the framework's deny table (`unwrap_used`,
`expect_used`, `dbg_macro`, `todo`, and warnings), the framework lints, and then the appearance
audit. Add `--no-audit` to skip the audit, or `--target web` to add the web lints. `nd test` runs
tiers 1 and 2 through nextest with no window server. `nd test --tier real` runs tier 3, which opens
real windows one at a time. For details on each tier, see the [testing page](tooling/testing.md).

## Keep nd up to date

```sh
nd upgrade
```

This moves the `nd` dependencies and the GPUI patch block to the newest release tag, updates
`Cargo.lock`, and prints the [changelog](https://github.com/FormalSnake/NativeDesktop/blob/main/CHANGELOG.md)
entries between the two versions. To pick a tag, add `--to v0.6.1`. Then install the matching CLI
with the same `cargo install --git ... --tag <tag> --locked nd-cli` command.

## Run it in a browser

```sh
nd serve --target web
```

This compiles the app to WebAssembly, runs `wasm-bindgen` and serves the bundle at
`http://127.0.0.1:8080/` (`--port` changes it). The first run installs a `wasm-bindgen-cli` that
matches your lockfile. `nd build --target web` writes the bundle without serving it. Some
framework calls don't exist in a browser. `nd check --target web` flags them, and the
[web guide](guides/web.md) covers the rest.

## The hello app

[Hello](../../examples/hello/src/lib.rs) is the scaffold's counter with a toolbar button. The
view is a struct and an `impl Render`:

<!-- docs:snippet examples/hello/src/lib.rs#counter -->

`scaffold()` is the window structure: a toolbar and a content area. `button("add", "Add one")` is
a toolbar item; the string `"add"` is its id, which tests use to find it. `cx.listener` wraps
the click handler so that it receives `&mut Counter`. `cx.notify()` marks this view dirty, and
the next frame runs `render` again. `cx.theme()` provides color tokens, so the secondary text
color isn't a literal.

Two functions open the window. `Application::new` takes the manifest that `build.rs` filled
from `nativedesktop.toml`, and `open` builds the root view as an entity:

<!-- docs:snippet examples/hello/src/lib.rs#open -->

`src/main.rs` is `hello::app().run(|cx| { hello::open(cx); })`. The test drives the same `open`:

<!-- docs:snippet examples/hello/tests/tier1.rs#tier1 -->

A `#[nd::test]` test runs natively, and in headless Chromium when you compile it for
`wasm32-unknown-unknown`.
Next: [the programming model](concepts/programming-model.md).
