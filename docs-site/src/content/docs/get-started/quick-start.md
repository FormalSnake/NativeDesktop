---
title: Quick Start
description: Build the host binary and run a real NativeDesktop example, in dev mode or a plain run.
---

The fastest way to see a real native window driven by React is `bun run dev` in one of the
framework's own examples. It runs the native backend for your platform, the AppKit shell on macOS or
the GTK host on Linux, with no build step to remember.

```bash
cd examples/counter && bun run dev        # == nd dev main.tsx: native backend, hot reload
bun run dev -- --backend gtk              # cross-check the same app on the GTK host
```

`nd dev` resolves the backend's binary through `@nativedesktop/host` and, inside this checkout,
builds it on first run (`nd: building appkit host (first run)…`). The rest of this page covers the
raw host invocation and manual builds, which you only need while working on the host itself.

## Prerequisites

The repo pins its toolchain via a Nix flake: Zig 0.16.0, Bun 1.3.13, and (on Linux) GTK4 +
libadwaita + a headless Weston compositor for CI. Enter it with:

```bash
nix develop
```

On macOS, nixpkgs' `libadwaita` fails to build under Nix (an `appstream` dependency issue), so the
Mac devshell borrows Homebrew's GTK stack. Install it once with `brew install libadwaita`, which
also pulls in `gtk4`. The GTK backend still runs natively on macOS through GTK4's Quartz `gdk`
backend, but the AppKit backend covered below is the one that ships.

## Build the host

```bash
nix develop -c zig build
```

This produces `zig-out/bin/nd-hello`, the Zig host executable. `zig build test` runs the unit test
suite the same way.

## Run an example

Point the host at a React entry point via `ND_SCRIPT`, and turn on the automation socket:

```bash
ND_SCRIPT=examples/counter/main.tsx NATIVE_AUTOMATION=1 ./zig-out/bin/nd-hello
```

Watch stderr for the markers every NativeDesktop host prints. All `ND_*` markers go to stderr, so
capture them with `2>&1`:

| Marker | Meaning |
|---|---|
| `ND_CHILD_CONNECTED` | the Bun child connected over the NDP socket |
| `ND_COMMIT_APPLIED commitId=…` | a `CommitBatch` was applied to the retained widget tree |
| `ND_AUTOMATION_LISTENING path=…` | the automation RPC socket is ready, and at what path |
| `ND_CHILD_EXITED` | the child disconnected (crash, `kill -9`, or clean exit) |

`examples/notes/` is a larger example, a two-pane notes app that exercises native chrome (see
[Windows & Chrome](/native-platform/windows-chrome/)). Run it the same way with
`ND_SCRIPT=examples/notes/main.tsx`.

Every `examples/*` package also declares `"dev": "nd dev main.tsx"`, so
`cd examples/counter && bun run dev` matches the raw invocation above except for
`NATIVE_AUTOMATION=1`. `nd dev` does not set that, so export it first if you need the automation
socket. `nd dev` picks the native backend (AppKit on macOS, GTK on Linux, overridable with
`--backend gtk|appkit` or `ND_BACKEND`), resolves a prebuilt binary through `@nativedesktop/host`,
and builds it on first run inside this checkout. Use the raw `ND_SCRIPT` form when you want to run
one specific freshly built host while iterating on the host itself.

## Dev mode: hot reload

Set `ND_DEV=1` on the host to run the Bun child under `bun --hot` and enable the crash-overlay's
Restart button:

```bash
ND_DEV=1 ND_SCRIPT=examples/counter/main.tsx NATIVE_AUTOMATION=1 ./zig-out/bin/nd-hello
```

`bun --hot` keeps the same OS process and socket across an edit but re-evaluates the entire module
graph, so import hooks from `@nativedesktop/react`, never directly from `react`:

```ts
// Correct: resolves against the reconciler's live dispatcher across hot reloads
import { useState } from "@nativedesktop/react";

// Wrong: resolves against a fresh module instance with no attached dispatcher
import { useState } from "react";
```

See [State & Hot Reload](/core-concepts/state-hot-reload/) for why this convention exists.

## Verbose tracing

Set `NDP_TRACE=1` on the host for verbose per-frame NDP tracing. Reach for it when a commit is not
showing up the way you expect.

## Building the macOS shell by hand

`nd dev` and `resolveHostBinary({ backend: "appkit" })` build the AppKit shell for you on first run,
so you rarely need this. Use it when you are changing the Swift host itself and want to drive one
specific build. The AppKit backend is a Swift shell linked against a GTK-free static build of the
Zig core:

```bash
zig build libnd -Dbackend=abi
```

Zig's archiver emits object members Apple's linker rejects; repack the archive with the system
tools before the Swift link:

```bash
workdir="$(mktemp -d)"
( cd "$workdir" && ar x "$PWD"/../zig-out/lib/libnd.a && chmod 644 *.o && libtool -static -o ../zig-out/lib/libnd.a *.o )
```

Then build and run the shell. `-u SDKROOT -u DEVELOPER_DIR` avoids a stale SDK path that a Nix
devshell can leak into the system Swift toolchain:

```bash
( cd swift && env -u SDKROOT -u DEVELOPER_DIR swift build -c release )
ND_SCRIPT=examples/counter/main.tsx NATIVE_AUTOMATION=1 swift/.build/release/NDShell
```
