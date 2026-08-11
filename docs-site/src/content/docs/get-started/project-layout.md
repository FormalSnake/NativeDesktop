---
title: Project Layout
description: Where the widget schema, generated bindings, core, and app code live in the NativeDesktop repository.
---

NativeDesktop is a monorepo. These are the pieces that matter when you build an app, or when you
work on the framework itself.

## `schema/widgets.json`, the single source of truth

Every widget's props, defaults, events, commands, and automation role is declared once, here.
Nothing about a widget's shape is hand-written anywhere else. `tools/codegen.ts` reads this file and
generates:

- `src/generated/`, the Zig bindings for the GTK backend: widget construction, prop application,
  event wiring.
- TypeScript intrinsics and schema metadata consumed by `packages/react`.
- The Swift arms of the AppKit backend under `swift/Sources/NDGen/`.
- The generated reference docs, `docs/widgets.md` and `docs/styling.md`.

Adding or changing a widget means editing `schema/widgets.json` and running
`scripts/regen-bindings.sh`. You never write a binding by hand.

Two more schemas feed the same pipeline: `schema/protocol.json` for the NDP wire frames and
`schema/rpc.json` for the automation methods.

## `src/`, the Zig core

The flat `.zig` files directly under `src/` are the GTK-free core that both backends link against:
`tree.zig` (the reconciler and retained widget tree), `runtime.zig`, `protocol.zig`,
`automation.zig`, `acl.zig`, and the C-ABI seam in `abi.zig` plus `abi_backend.zig`.

- `src/core/` is `libnd`'s module root (`root.zig`) alongside the terminal, remote-terminal, and
  update subsystems. `zig build libnd -Dbackend=abi` builds this as the static library the Swift
  shell links.
- `src/gtk/` is the GTK4 and libadwaita backend: widget creation, style and CSS-class application,
  the webview, the terminal, the main loop. It compiles into the `nd-hello` binary.
- `src/generated/` is codegen output. Do not hand-edit it.

## `swift/`, the macOS shell

A Swift and AppKit shell over the C-ABI core, following the same pattern as Ghostty's `libghostty`.
`swift/Sources/NDShell/` is hand-written: `Backend.swift` for widget creation and prop application,
`HeaderBar.swift`, `SplitController.swift`, and `Layout.swift` for native chrome, `Icons.swift` for
the freedesktop-to-SF-Symbol mapping (see [Icons](/native-platform/icons/)), plus `Automation.swift`
and `Events.swift`. `swift/Sources/NDGen/` holds the generated arms, and `swift/Sources/CNd/`
bridges `libnd.a`.

## `packages/`

| Package | What it is |
|---|---|
| `@nativedesktop/react` | The renderer app code imports. Turns React commits into NDP `CommitBatch` ops and re-exports the hooks your app uses. `react` is a `peerDependency`, so one hoisted instance is shared across the app and the linked package. See [State & Hot Reload](/core-concepts/state-hot-reload/). |
| `@nativedesktop/cli` | The `nd` CLI (bin `nd`). `nd dev [entry]` wraps the raw `ND_DEV=1 ND_SCRIPT=<entry> <host-binary>` invocation, `nd build` runs the Babel and React Compiler pre-pass, `nd package` assembles the platform bundle, `nd doctor` checks toolchain/config readiness. See [Quick Start](/get-started/quick-start/) and [Packaging](/packaging/). |
| `@nativedesktop/host` | `resolveHostBinary()` finds the prebuilt host for the current platform under `bin/<os>-<arch>/`, and builds one on first run inside this checkout. |
| `@nativedesktop/data` | Worker-backed `bun:sqlite`, so queries never block React's commit loop. See [App Data & Storage](/core-concepts/app-data-storage/). |
| `@nativedesktop/native` | Support for app-owned native plugins. |
| `@nativedesktop/rpc` | A resilient JSON-RPC 2.0 client (reconnect ladder, call queueing, liveness watchdog) for an app that talks to a daemon or remote server. See [RPC Client](/core-concepts/rpc-client/). |
| `@nativedesktop/panes` | Pane-tree state for resizable split layouts (split/close/focus/resize a tree of panes) plus a `PaneTree`/`usePaneTree` React binding. |
| `@nativedesktop/test` | `launchApp`/`AppHandle`: spawn a host, connect, and drive it over the automation socket from a Bun test or drive script. See [Test Harness](/automation-testing/test-harness/). |
| `@nativedesktop/mcp` | A stdio MCP server bridging the automation socket to MCP tool calls. See [MCP Tools](/automation-testing/mcp-tools/). |
| `babel-plugin-nativedesktop` | Rewrites `react` hook imports to `@nativedesktop/react` in shared logic modules. |

## `examples/`

Eighteen driven apps that stress-test the framework. The ones to read first:

- `examples/counter/`: the minimal app. State, a click handler, `Suspense`, and an interval, in one
  `<window>`.
- `examples/notes/`: a two-pane notes app exercising native chrome (`<splitview>`, `<headerbar>`,
  `<toolbarview>`), `cssClasses`, and search.
- `examples/gallery/`: a broad widget gallery, including a 100k-row `<listview>` regression case.
- `examples/browser/`: tabbed browsing over `<webview>`, including native tabs and cross-window tab
  drag.
- `examples/terminal/` and `examples/remote-terminal/`: the libghostty-vt terminal widget, local and
  over SSH.

## `template/`

What `scripts/new-app.sh` copies to start a new app: a `package.json` linking `@nativedesktop/react`,
`@nativedesktop/native`, `@nativedesktop/cli` (bin `nd`), and transitively `@nativedesktop/host`
through `file:` paths into this checkout (a scaffold made from a checkout exercises the checkout, not
the npm registry the template's own `^0.1.0` ranges point at), a `src/main.tsx` entry, a
`babel.config.json` for the opt-in React Compiler and hook-import rewrite, and a `bunfig.toml` that
preloads the `bun --hot` twin of that rewrite.

## `tools/` and `scripts/`

`tools/` holds build-time scripts invoked directly with `bun`: `tools/codegen.ts` (schemas to
bindings and docs), `tools/package.ts` (a thin shim that packages the gallery example through the
real `nd package` implementation in `packages/nd/src/package/`, see [Packaging](/packaging/)), and
`tools/ndshot/` for macOS screen capture. The `nd` CLI covers `nd dev`, `nd build`, `nd package`,
and `nd doctor`. There is no `nd codegen`.

`scripts/` holds the headless drive scripts the CI gate runs, `new-app.sh`, and
`regen-bindings.sh`.
