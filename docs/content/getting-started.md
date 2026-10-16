+++
title = "Getting started"
description = "Install the nd CLI, create an app, run it on the desktop and in a browser, and run its tests."
+++

This page takes you from a clone of the repository to a running, tested app.

<!-- docs:example hello -->

## Install

You need a Rust toolchain, `cargo-nextest` (`nd test` runs through it) and, because the terminal
widget builds `libghostty-vt`, Zig 0.16. The repository's dev shell has all of it:

```sh
nix develop
```

With direnv, the checked-in `.envrc` (`use flake`) does that on `cd`. Without Nix, install
[rustup](https://rustup.rs); `rust-toolchain.toml` pins 1.95.0 with the `wasm32-unknown-unknown`
target and rustup fetches both on the first `cargo` call. Then:

```sh
cargo install cargo-nextest --locked
```

Build the CLI from the repository. It installs a binary named `nd`:

```sh
cargo install --path crates/nd-cli
```

To skip the install, run it in place with `cargo run -p nd-cli -- <args>`; every `nd` command on
this page works that way.

## Create an app

```sh
nd new demo --nd-path /path/to/nativedesktop
```

The crates are not on crates.io yet (`publish = false`), so the project has to point at your
checkout. `--nd-path` does that, and so does the `ND_PATH` environment variable. Add
`--standalone` when the new directory sits inside another Cargo workspace. The name must start
with a lowercase letter and may contain lowercase letters, digits, `-` and `_`. `--id` sets the
reverse-DNS app id; it defaults to `com.example.demo`.

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

`just dev` is `cargo run`. On macOS the window is AppKit. On Linux it is the GPUI backend. Windows
code exists but has not been run, so treat a Windows build as untested. On a target with no
backend, `run` returns `no platform backend is linked for this target`.

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
