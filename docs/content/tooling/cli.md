+++
title = "The nd command"
description = "Every nd subcommand with its flags and one example each."
+++

`nd` is one binary ([crates/nd-cli](../../../crates/nd-cli/src/lib.rs)). Run it from an app's
directory (the one holding `Cargo.toml` and `nativedesktop.toml`). `nd <command> --help` prints the
same flags as below.

| Command | Does |
| --- | --- |
| [`new`](#new) | Creates an app. |
| [`upgrade`](#upgrade) | Moves the app to a newer nd release. |
| [`check`](#check) | fmt, clippy, framework lints, appearance audit. |
| [`test`](#test) | Runs the test tiers through nextest. |
| [`build`](#build) | Native release build, or the web bundle. |
| [`serve`](#serve) | Builds and serves the web bundle. |
| [`diagnose`](#diagnose) | Summarises incident bundles. |
| [`package`](#package) | Builds installers. |
| [`release`](#release) | Packs, signs and publishes an update feed. |
| [`diag`](#diag) | Measures framework budgets. |
| [`docs`](#docs) | Checks, builds and captures this documentation. |

## new

Creates a directory with a buildable app: `Cargo.toml`, `nativedesktop.toml`, a justfile, a nextest
config, tests, and the agent docs ([for agents](../agents/index.md)). Refuses a non-empty directory
and names that are not a lowercase package name.

- `<name>`: directory to create; its last component is the package name.
- `--id <id>`: reverse-DNS app id, default `com.example.<name>`.
- `--nd-path <path>` (env `ND_PATH`): depend on a local checkout of the `nd` crate (`crates/nd`)
  instead of the CLI's release tag.
- `--nd-git <url>` (env `ND_GIT`): the repository nd is fetched from, default
  `https://github.com/FormalSnake/NativeDesktop`, for a mirror or a fork.
- `--standalone`: add an empty `[workspace]` so the app builds on its own inside another workspace.

```sh
nd new notes --id es.canarycoders.notes && cd notes && nd check && nd test
```

## upgrade

Moves every `nd` dependency and the GPUI `[patch.crates-io]` entries from their current tag to a
newer one, re-resolves `Cargo.lock` (other locked versions stay) and prints the `CHANGELOG.md`
entries between the two versions. If cargo cannot resolve the new tag, `Cargo.toml` is put back.

- `--to <tag>`: the tag to move to, default the newest `vX.Y.Z` tag on the remote.
- `--path <dir>`: the app's directory, default the current one.

```sh
nd upgrade --to v0.6.1
```

## check

Runs, in order: `cargo fmt --check`, the framework lints, `cargo clippy --all-targets` with the
framework lint table denied (`warnings`, `unwrap_used`, `expect_used`, `dbg_macro`, `todo`,
`print_stdout`), and the appearance audit of every package that has `tests/appearance.rs` (see
[testing](testing.md#appearance-audit)). Exits non-zero if any step failed. A framework lint is
silenced with `// nd-allow(rule): reason` on its line or the line above.

- `-p, --package <name>`: only these packages, default the workspace's default members.
- `--release`: also fail on `<!-- nd: ... -->` placeholders left in markdown files.
- `--no-clippy`: skip clippy.
- `--no-audit`: skip the appearance audit.
- `--manifest-path <path>`: another `Cargo.toml`.
- `--target native|web`: `web` adds the web lints, builds clippy for `wasm32-unknown-unknown` and skips the audit ([web](../guides/web.md)).

```sh
nd check --target web -p notes
```

## test

Maps a tier to a nextest profile and runs `cargo nextest run` ([testing](testing.md#nd-test)).
Needs `cargo-nextest`.

- `--tier fast|real|all`: `fast` (default) is tiers 1 and 2, `real` is tier 3 and needs a window server, `all` runs fast then real.
- `-p, --package <name>`: only these packages.
- `<filters>...`: test name filters, as nextest takes them.

```sh
nd test --tier real -p hello real_add
```

## build

Without a target flag this is the native release build. With `--target web` it compiles the app
for `wasm32-unknown-unknown`, runs the matching `wasm-bindgen`, and writes a static bundle with an
`index.html` ([web](../guides/web.md)).

- `--target native|web`: default `native`.
- `-p, --package <name>`: default the package in the current directory.
- `--debug`: build without optimisations.
- `--out <dir>`: where the web bundle goes, default `target/nd/web/<package>/`.

```sh
nd build --target web --out dist
```

## serve

Builds the web bundle (unless told not to) and serves it on localhost.

- `--target web`: the only target, and the default.
- `-p, --package <name>`, `--debug`, `--out <dir>`: as in `build`.
- `--port <n>`: default 8080.
- `--no-build`: serve the last bundle as it is.

```sh
nd serve --target web --port 3000
```

## diagnose

Summarises the incident bundles an app left behind: panics, crashes, hangs and issue reports
([diagnostics](diagnostics.md)). With no argument it summarises the newest.

- `<id>`: a bundle id or a unique part of it, as `--list` prints it.
- `--last`: the newest bundle (the default).
- `--list`: list bundles, newest first.
- `--json`: machine-readable output.
- `--app <name>`: the app's Cargo package name when the current directory does not say.
- `--manifest-path <path>`.

```sh
nd diagnose --list && nd diagnose --last --json
```

## package

Builds the installer for one OS: a macOS `.app`, `.pkg` and `.dmg`, a Linux AppImage, a Windows
`Setup.exe`, all through Velopack ([packaging](../guides/packaging.md)). The result lands in
`target/nd/releases`.

- `<os>`: `mac`, `linux` or `windows`; default is this OS.
- `--dir <path>`: app directory, default `.`.
- `--channel <name>`: one of `updates.channels`, default the first.
- `--notes <file>`: markdown release notes for the app's "what's new".
- `--binary <path>`: package this prebuilt binary instead of running `cargo build --release` (cross-packing).
- `--runtime <id>`: Velopack runtime id for cross-packing, such as `linux-arm64`.

```sh
nd package mac --channel beta --notes CHANGES.md
```

## release

Packs like `package`, then writes a delta against the newest full package, signs the feed with an
ed25519 key, and uploads it to `[updates.publish]` ([packaging](../guides/packaging.md)).
Without `[updates] feed` and `public_key` in `nativedesktop.toml` it stops with a message.

- `--dir`, `--channel`, `--notes`, `--binary`: as in `package`.
- `--os mac|linux|windows`: default this OS.
- `--key <file>`: signing key (base64 seed). Else `ND_UPDATE_SIGNING_KEY` (the key itself), else the file `keygen` wrote.
- `--no-publish`: write and sign the feed directory without uploading it.

```sh
nd release --channel stable --notes CHANGES.md
```

Two subcommands:

- `nd release keygen [--dir <path>] [--out <file>]`: makes an ed25519 key, writes it to `--out` (default `~/.config/nd/keys/<app id>.key`) and prints the public key for `[updates] public_key`.
- `nd release serve [--dir <path>] [--root <dir>] [--host <addr>] [--port <n>]`: serves the feed directory over HTTP for update tests. `--root` replaces the default `target/nd/releases`; host defaults to `127.0.0.1`, port to 8790.

```sh
nd release keygen && nd release serve --port 8790
```

## diag

Measures a framework budget. The one subcommand is `scroll` ([performance](performance.md#scroll)).

- `nd diag scroll --rows <n>`: rows in the list, default 100000.
- `--seconds <s>`: how long to scroll, default 10.
- `--step <points>`: points scrolled per display frame, default 40.
- `--bin <path>` (env `ND_SCROLL_BIN`): the `scroll-bench` binary, default the one next to `nd`.

It prints the frame percentiles and exits non-zero when more than 2 frames are missed past warm-up
or p95 is over 1.25 refresh periods.

```sh
nd diag scroll --rows 100000 --seconds 10
```

## docs

Works on the documentation in `docs/content` (see `docs/WRITING.md`) from anywhere inside the
repository.

- `nd docs check`: every page loads (snippets, examples and directives resolve), links and
  anchors resolve, prose rules hold over the pages and the `nd new` templates, and a capture
  report per OS. `--require macos` fails on a missing or stale macOS capture, `--compile` checks
  every example natively and the web ones for wasm32, `--web` builds the site and each web
  example and loads them in headless Chrome.
- `nd docs build`: the static site in `target/nd/docs-site/`: an HTML page and the raw markdown
  for every page, `llms.txt`, `llms-full.txt`, and the docs app's web bundle under `app/`.
  `nd docs serve` builds and serves it. CI deploys it to https://nativedesktop.formalsnake.dev on
  every push to main.
- `nd docs capture [example..]`: runs each example through the test harness on this OS, light
  and dark, and records its source hash in the capture manifest. `--stale` takes only examples
  whose capture is missing or out of date. A changed example shows "not captured yet" until it
  is captured again.

```sh
nd docs check --require macos && nd docs capture --stale
```
