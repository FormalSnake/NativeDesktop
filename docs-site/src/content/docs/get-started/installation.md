---
title: Installation
description: Install NativeDesktop from npm, understand the prebuilt host packages, and check your setup with nd doctor.
---

NativeDesktop ships on npm. You install two packages plus React; the native host binary for your
platform comes along automatically.

## Requirements

- **Bun 1.3 or newer.** The `nd` CLI and your app both run on Bun.
- **macOS**: version 15 or newer on Apple silicon. The host binary is self-contained and links only
  system frameworks.
- **Linux**: x86_64 with GTK4 and libadwaita 1.7 or newer at run time. Installing your distro's
  libadwaita package (`libadwaita-1-0` on Debian and Ubuntu, `libadwaita` on Fedora) pulls in
  everything required. The full table, including the optional libraries behind `<webview>`, audio,
  and credentials, is in
  [runtime-deps.md](https://github.com/FormalSnake/NativeDesktop/blob/main/docs/runtime-deps.md).
- **React 19.** `@nativedesktop/react` declares `react@^19.2.7` as a peer dependency.

## Install

```bash
bun add @nativedesktop/cli @nativedesktop/react react
```

This gives you:

- `@nativedesktop/cli`: the `nd` command (`nd dev`, `nd build`, `nd package`, `nd doctor`).
- `@nativedesktop/react`: the renderer, the intrinsic widgets, hooks, and the system APIs.
- `react`: a real peer copy, shared with any web or React Native code beside it.

For editor support and typechecking, add the type packages:

```bash
bun add -d typescript @types/react @types/bun
```

## The platform host packages

Every NativeDesktop app is two processes: a native host that owns the OS event loop and the widgets,
and a Bun child that runs your React code. The host is a prebuilt binary, shipped in a per-platform
npm package:

| Package | Binary | Platform |
| --- | --- | --- |
| `@nativedesktop/host-darwin-arm64` | `nd-shell` (AppKit) | macOS on Apple silicon |
| `@nativedesktop/host-linux-x64` | `nd-hello` (GTK4 + libadwaita) | x86_64 Linux |

Both are `optionalDependencies` of `@nativedesktop/host`, which `@nativedesktop/cli` depends on.
Each declares `os` and `cpu` fields, so your package manager installs only the one matching your
machine. This is the same model Electron and esbuild use.

When you run `nd dev`, `@nativedesktop/host` resolves the binary from the installed platform
package. Inside a source checkout of the framework it prefers a freshly built local binary and
builds one on first run; from an npm install, the prebuilt is the only path, and a missing binary is
a hard error that names the package to reinstall.

## From source

To work on the framework itself, or to run on a platform without a prebuilt host, build from a
checkout. You need Zig 0.16.0 and Bun; `nix develop` in the repo pins both.

```bash
git clone https://github.com/FormalSnake/NativeDesktop
cd NativeDesktop && bun install
cd examples/counter && bun run dev
```

`nd dev` builds the host on first run inside the checkout. On macOS, the GTK backend additionally
needs Homebrew's GTK stack (`brew install libadwaita`); the AppKit backend does not. Scaffold an app
wired to the checkout with `./scripts/new-app.sh ../my-app`.

## Pointing at a host binary yourself

`ND_HOST_BINARY=<path>` skips resolution entirely and runs that binary, for `nd dev`, `nd package`
and `nd doctor` alike. It exists for the machines resolution cannot serve: a distro whose loader
rejects a generic prebuilt (NixOS without `programs.nix-ld`), an arch with no published package, or
the GTK backend on macOS. Build the host in a checkout and export the path:

```bash
export ND_HOST_BINARY="$HOME/src/NativeDesktop/zig-out/bin/nd-hello"
bun run dev
```

A path that does not exist is a hard error rather than a silent fallback, and `nd doctor` prints the
override so it always reports the binary `nd dev` will actually run. In `@nativedesktop/test`, the
per-launch equivalent is `launchApp({ hostBinary })`.

## Troubleshooting

Run `nd doctor` in your project directory. It checks the entry point, the Bun install, the host
binary, and the packaging toolchain, and exits non-zero only on real gaps:

```bash
bunx nd doctor
```

```
warn  config       no nativedesktop.config.ts here (defaults apply)
warn  app.id       app.id not set (required for icons, file associations, and updates)
ok    entry        src/main.tsx
ok    bun          /usr/local/bin/bun
ok    host         node_modules/@nativedesktop/host-darwin-arm64/bin/nd-shell
ok    codesign     codesign available
warn  updates      package.updates not configured: the shipped app has no updater
```

Warnings are advisory. A fresh project without a `nativedesktop.config.ts` is fine for development;
the config matters when you package.

Common failures:

- **`no appkit host binary` or `no gtk host binary`**: the platform package is missing. Reinstall
  without `--no-optional`. If you are on a platform with no prebuilt (for example macOS on Intel),
  build from a source checkout instead.
- **App does not start on Linux**: a required library is missing. Check with
  `ldd node_modules/@nativedesktop/host-linux-x64/bin/nd-hello`; any `not found` line names the
  package to install.
- **NixOS: `Could not start dynamically linked executable`**: the prebuilt is a generic-glibc binary
  and NixOS ships no loader for one unless `programs.nix-ld` is enabled. Either enable it with the
  GTK stack in `programs.nix-ld.libraries`, or build the host in a checkout
  (`nix develop -c zig build`) and set `ND_HOST_BINARY` to `zig-out/bin/nd-hello`.
- **`<webview>` shows an unavailable placeholder on Linux**: install `libwebkitgtk-6.0` and
  `glib-networking`. The webview engine is loaded at run time and degrades when absent.

## Next

- [Quick Start](/get-started/quick-start/): a running window in under five minutes.
- [Build a Counter](/get-started/tutorial-counter/): the first tutorial.
