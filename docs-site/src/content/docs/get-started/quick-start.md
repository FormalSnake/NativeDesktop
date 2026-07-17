---
title: Quick Start
description: Build the host binary and run a real NativeDesktop example, in dev mode or a plain run.
---

The fastest way to see a real native window driven by React is `bun run dev` in one of the
framework's own examples: it runs the **native backend for your platform** — the AppKit shell on
macOS, the GTK host on Linux — with no build step to remember.

```bash
cd examples/counter && bun run dev        # == nd dev main.tsx — native backend, hot reload
bun run dev -- --backend gtk              # cross-check the same app on the GTK host
```

`nd dev` resolves the backend's binary via `@nativedesktop/host` and, inside this checkout, builds
it on first run (`nd: building appkit host (first run)…`). The rest of this page covers the raw
host invocation and manual builds, which you only need while hacking on the host itself.

## Prerequisites

The repo pins its toolchain via a Nix flake: Zig 0.16.0, Bun 1.3.13, and (on Linux) GTK4 +
libadwaita + a headless Weston compositor for CI. Enter it with:

```bash
nix develop
```

On macOS, nixpkgs' `libadwaita` doesn't build under Nix (an `appstream` dependency issue), so the
Mac devshell borrows Homebrew's GTK stack instead — install it once with
`brew install libadwaita` (this also pulls in `gtk4`). The GTK backend still runs natively on macOS
via GTK4's Quartz `gdk` backend; the AppKit backend (below) is the one that ships.

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

Watch stderr for the marker vocabulary every NativeDesktop host prints (all `ND_*` markers go to
stderr — capture with `2>&1`):

| Marker | Meaning |
|---|---|
| `ND_CHILD_CONNECTED` | the Bun child connected over the NDP socket |
| `ND_COMMIT_APPLIED commitId=…` | a `CommitBatch` was applied to the retained widget tree |
| `ND_AUTOMATION_LISTENING path=…` | the automation RPC socket is ready, and at what path |
| `ND_CHILD_EXITED` | the child disconnected (crash, `kill -9`, or clean exit) |

`examples/notes/` is a larger example (a two-pane notes app exercising native chrome — see
[Windows & Chrome](/native-platform/windows-chrome/)); run it the same way with
`ND_SCRIPT=examples/notes/main.tsx`.

Every `examples/*` package also declares `"dev": "nd dev main.tsx"` (the `nd` CLI, `packages/nd`),
so `cd examples/counter && bun run dev` is equivalent to the raw invocation above minus
`NATIVE_AUTOMATION=1` — `nd dev` doesn't set that for you, so export it first if you need the
automation socket. `nd dev` picks the native backend (appkit on macOS, gtk on Linux; override with
`--backend gtk|appkit` or `ND_BACKEND`), resolving a prebuilt binary via `@nativedesktop/host` and
building it on first run inside this checkout. Use the raw `ND_SCRIPT` form above when you want to
run a specific freshly built host while iterating on the host itself.

## Dev mode: hot reload

Set `ND_DEV=1` on the host to run the Bun child under `bun --hot` and enable the crash-overlay's
Restart button:

```bash
ND_DEV=1 ND_SCRIPT=examples/counter/main.tsx NATIVE_AUTOMATION=1 ./zig-out/bin/nd-hello
```

`bun --hot` keeps the same OS process and socket across an edit but re-evaluates the entire module
graph, so hooks must be imported from `@nativedesktop/react`, never directly from `react`:

```ts
// Correct — resolves against the reconciler's live dispatcher across hot reloads
import { useState } from "@nativedesktop/react";

// Wrong — resolves against a fresh module instance with no attached dispatcher
import { useState } from "react";
```

See [State & Hot Reload](/core-concepts/state-hot-reload/) for why this convention exists.

## Verbose tracing

`NDP_TRACE=1` (set on the host) turns on verbose per-frame NDP tracing — useful when a commit isn't
showing up the way you expect.

## Building the macOS shell by hand

`nd dev` (and `resolveHostBinary({ backend: "appkit" })`) build the AppKit shell for you on first
run, so you rarely need this. It's the recipe to reach for when you're changing the Swift host
itself and want to drive a specific build. The AppKit backend is a Swift shell linked against a
GTK-free static build of the Zig core:

```bash
zig build libnd -Dbackend=abi
```

Zig's archiver emits object members Apple's linker rejects; repack the archive with the system
tools before the Swift link:

```bash
workdir="$(mktemp -d)"
( cd "$workdir" && ar x "$PWD"/../zig-out/lib/libnd.a && chmod 644 *.o && libtool -static -o ../zig-out/lib/libnd.a *.o )
```

Then build and run the shell — `-u SDKROOT -u DEVELOPER_DIR` avoids a stale SDK path a Nix devshell
can leak into the system Swift toolchain:

```bash
( cd swift && env -u SDKROOT -u DEVELOPER_DIR swift build -c release )
ND_SCRIPT=examples/counter/main.tsx NATIVE_AUTOMATION=1 swift/.build/release/NDShell
```
