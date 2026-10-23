+++
title = "The nd command"
description = "Every nd subcommand, its flags, and one example of each."
+++

`nd` is one binary ([crates/nd-cli](../../../crates/nd-cli/src/lib.rs)). Run it from an app's
directory, the one that holds `Cargo.toml` and `nativedesktop.toml`. `nd <command> --help` prints
the same flags that this page lists.

| Command | What it does |
| --- | --- |
| [`new`](#new) | Creates an app. |
| [`upgrade`](#upgrade) | Moves the app to a newer nd release. |
| [`check`](#check) | Runs fmt, clippy, the framework lints, and the appearance audit. |
| [`test`](#test) | Runs the test tiers through nextest. |
| [`build`](#build) | Builds the native release, or the web bundle. |
| [`serve`](#serve) | Builds and serves the web bundle. |
| [`diagnose`](#diagnose) | Summarizes incident bundles. |
| [`package`](#package) | Builds installers. |
| [`release`](#release) | Packs, signs, and publishes an update feed. |
| [`diag`](#diag) | Measures framework budgets. |
| [`docs`](#docs) | Checks, builds, and captures this documentation. |

## new

Creates a directory with an app that builds: `Cargo.toml`, `nativedesktop.toml`, a justfile, a
nextest config, tests, and the agent docs ([for agents](../agents/index.md)). It refuses a directory
that isn't empty and a name that isn't a lowercase package name.

- `<name>`: the directory to create. Its last component is the package name.
- `--id <id>`: the reverse-DNS app id. The default is `com.example.<name>`.
- `--nd-path <path>` (env `ND_PATH`): depend on a local checkout of the `nd` crate (`crates/nd`)
  instead of the CLI's release tag.
- `--nd-git <url>` (env `ND_GIT`): the repository to fetch nd from, for a mirror or a fork. The
  default is `https://github.com/FormalStudio/NativeDesktop`.
- `--standalone`: add an empty `[workspace]` so the app builds on its own inside another workspace.

```sh
nd new notes --id es.canarycoders.notes && cd notes && nd check && nd test
```

## upgrade

Moves every `nd` dependency and the GPUI `[patch.crates-io]` entries from their current tag to a
newer one. It then re-resolves `Cargo.lock` (other locked versions stay) and prints the
`CHANGELOG.md` entries between the two versions. If cargo can't resolve the new tag, `nd upgrade`
restores `Cargo.toml`.

- `--to <tag>`: the tag to move to. The default is the newest `vX.Y.Z` tag on the remote.
- `--path <dir>`: the app's directory. The default is the current directory.

```sh
nd upgrade --to v0.7.0
```

## check

Runs, in order: `cargo fmt --check`, the framework lints, `cargo clippy --all-targets` with the
framework lint table denied (`warnings`, `unwrap_used`, `expect_used`, `dbg_macro`, `todo`,
`print_stdout`), and the appearance audit of every package that has `tests/appearance.rs` (see
[testing](testing.md#appearance-audit)). It exits non-zero if any step fails. To silence a framework
lint, put `// nd-allow(rule): reason` on its line or the line above.

- `-p, --package <name>`: check only these packages. The default is the workspace's default members.
- `--release`: also fail on `<!-- nd: ... -->` placeholders left in markdown files.
- `--no-clippy`: skip clippy.
- `--no-audit`: skip the appearance audit.
- `--manifest-path <path>`: use another `Cargo.toml`.
- `--target native|web`: `web` adds the web lints, builds clippy for `wasm32-unknown-unknown` and skips the audit ([web](../guides/web.md)).

```sh
nd check --target web -p notes
```

## test

Maps a tier to a nextest profile and runs `cargo nextest run` ([testing](testing.md#nd-test)). It
needs `cargo-nextest`.

- `--tier fast|real|all`: `fast` (the default) is tiers 1 and 2. `real` is tier 3 and needs a window server. `all` runs fast, then real.
- `-p, --package <name>`: test only these packages.
- `<filters>...`: test name filters, as nextest takes them.

```sh
nd test --tier real -p hello real_add
```

## build

Without a target flag, `nd build` makes the native release build. With `--target web`, it compiles
the app for `wasm32-unknown-unknown`, runs the matching `wasm-bindgen`, and writes a static bundle
with an `index.html` ([web](../guides/web.md)).

- `--target native|web`: the default is `native`.
- `-p, --package <name>`: the default is the package in the current directory.
- `--debug`: build without optimizations.
- `--out <dir>`: where the web bundle goes. The default is `target/nd/web/<package>/`.

```sh
nd build --target web --out dist
```

## serve

Builds the web bundle and serves it on localhost. Pass `--no-build` to skip the build.

- `--target web`: the only target, and the default.
- `-p, --package <name>`, `--debug`, `--out <dir>`: as in `build`.
- `--port <n>`: the default is 8080.
- `--no-build`: serve the last bundle as it is.

```sh
nd serve --target web --port 3000
```

## diagnose

Summarizes the incident bundles that an app left behind: panics, crashes, hangs, and issue reports
([diagnostics](diagnostics.md)). With no argument, it summarizes the newest one.

- `<id>`: a bundle id, or a unique part of it, as `--list` prints it.
- `--last`: the newest bundle (the default).
- `--list`: list bundles, newest first.
- `--json`: machine-readable output.
- `--app <name>`: the app's Cargo package name, when the current directory doesn't identify it.
- `--manifest-path <path>`: use another `Cargo.toml`.

```sh
nd diagnose --list && nd diagnose --last --json
```

## package

Builds the installer for one OS through Velopack: a macOS `.app`, `.pkg`, and `.dmg`, a Linux
AppImage, or a Windows `Setup.exe` ([packaging](../guides/packaging.md)). The result goes to
`target/nd/releases`.

- `<os>`: `mac`, `linux`, or `windows`. The default is the current OS.
- `--dir <path>`: the app directory. The default is `.`.
- `--channel <name>`: one of `updates.channels`. The default is the first one.
- `--notes <file>`: markdown release notes for the app's "what's new".
- `--binary <path>`: package this prebuilt binary instead of running `cargo build --release` (cross-packing).
- `--runtime <id>`: the Velopack runtime id for cross-packing, such as `linux-arm64`.

```sh
nd package mac --channel beta --notes CHANGES.md
```

## release

Packs the way `package` does, then writes a delta against the newest full package, signs the feed
with an ed25519 key, and uploads it to `[updates.publish]` ([packaging](../guides/packaging.md)).
If `nativedesktop.toml` has no `[updates] feed` and `public_key`, it stops with a message.

- `--dir`, `--channel`, `--notes`, `--binary`: as in `package`.
- `--os mac|linux|windows`: the default is the current OS.
- `--key <file>`: the signing key (a base64 seed). Without it, `nd release` uses `ND_UPDATE_SIGNING_KEY` (the key itself), then the file that `keygen` wrote.
- `--no-publish`: write and sign the feed directory without uploading it.

```sh
nd release --channel stable --notes CHANGES.md
```

`nd release` has two subcommands:

- `nd release keygen [--dir <path>] [--out <file>]`: makes an ed25519 key, writes it to `--out` (default `~/.config/nd/keys/<app id>.key`), and prints the public key for `[updates] public_key`.
- `nd release serve [--dir <path>] [--root <dir>] [--host <addr>] [--port <n>]`: serves the feed directory over HTTP for update tests. `--root` replaces the default `target/nd/releases`. The host defaults to `127.0.0.1` and the port to 8790.

```sh
nd release keygen && nd release serve --port 8790
```

## diag

Measures a framework budget. Its only subcommand is `scroll` ([performance](performance.md#scroll)).

- `nd diag scroll --rows <n>`: the number of rows in the list. The default is 100000.
- `--seconds <s>`: how long to scroll. The default is 10.
- `--step <points>`: the points to scroll per display frame. The default is 40.
- `--bin <path>` (env `ND_SCROLL_BIN`): the `scroll-bench` binary. The default is the one next to `nd`.

It prints the frame percentiles. It exits non-zero when more than 2 frames are missed after warm-up,
or when p95 is over 1.25 refresh periods.

```sh
nd diag scroll --rows 100000 --seconds 10
```

## docs

Works on the documentation in `docs/content` (see `docs/WRITING.md`). Run it from anywhere inside
the repository.

- `nd docs check`: checks that every page loads (snippets, examples, and directives resolve),
  that links and anchors resolve, and that the prose rules hold over the pages and the `nd new`
  templates. It also prints a capture report per OS. `--require macos` fails on a missing or stale
  macOS capture. `--compile` checks every example natively and the web ones for wasm32. `--web`
  builds the site and loads each web example from it in headless Chrome.
- `nd docs build`: writes the static site to `target/nd/docs-site/`. That's an HTML page and the
  raw markdown for every page, `llms.txt`, `llms-full.txt`, the captures, and the web bundle
  under `run/` that runs the web examples live on their pages. `nd docs serve` builds and serves it. CI deploys it to
  https://native.formalsnake.dev on every push to main.
- `nd docs capture [example..]`: runs each example through the test harness on this OS, light
  and dark, and records its source hash in the capture manifest. `--stale` takes only the examples
  whose capture is missing or out of date. A changed example shows "not captured yet" until you
  capture it again.
  On Windows the examples are launched and captured by `ndshell agent`, which runs in the
  logged-in user's session (`ndshell agent install`), so the command also works over ssh.

```sh
nd docs check --require macos && nd docs capture --stale
```
