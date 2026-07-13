---
title: Project Layout
description: Where the widget schema, generated bindings, core, and app code live in the NativeDesktop repository.
---

NativeDesktop is a monorepo. The pieces that matter for building an app — or for building the
framework itself — are:

## `schema/widgets.json` — the single source of truth

Every widget's props, defaults, events, and automation role are declared once, here. Nothing about
a widget's shape is hand-written anywhere else; `tools/codegen.ts` reads this file and generates:

- `src/generated/` — Zig bindings for the GTK backend (widget construction, prop application,
  event wiring).
- TypeScript intrinsics and schema metadata consumed by `packages/react`.
- Swift arms in the AppKit backend under `swift/Sources/NDShell/` (the generated `Widgets.swift`).
- The generated docs themselves: `docs/widgets.md` and `docs/styling.md`.

If you add or change a widget, you change `schema/widgets.json` and regenerate — never a
hand-written binding.

## `packages/react` — the React renderer

`@nativedesktop/react` is the package app code imports. It implements the host config that turns
React commits into NDP `CommitBatch` operations and re-exports the hooks your app uses (see
[State & Hot Reload](/core-concepts/state-hot-reload/) for why the re-export matters). `react` is a
`peerDependency` (not vendored), so a single hoisted `react` instance is shared across an app and
the linked package.

## `packages/nd` and `packages/host` — the `nd` CLI

`nd dev [entry]` / `nd build` (`packages/nd`) wrap the raw `ND_DEV=1 ND_SCRIPT=<entry>
<host-binary>` invocation and the babel/react-compiler pre-pass, respectively — see
[Quick Start](/get-started/quick-start/). `@nativedesktop/host`'s `resolveHostBinary()` finds the
prebuilt `nd-hello` for the current platform under `bin/<os>-<arch>/`; there's no CI binary matrix
yet, so today that binary comes from a local `zig build` copied in by hand.

## `src/` — the Zig core

- `src/core/` — the GTK-free core (widget tree, automation server, protocol handling) that both
  backends link against. This is what `zig build libnd -Dbackend=abi` produces as a static library
  for the Swift shell.
- `src/gtk/` — the GTK4/libadwaita backend: widget creation, style/CSS-class application, the main
  loop.
- `src/generated/` — codegen output from `schema/widgets.json` (do not hand-edit).

## `swift/Sources/NDShell/` — the macOS shell

A thin Swift/AppKit shell over the C-ABI Zig core, following the same pattern as Ghostty's
`libghostty`: `Backend.swift` (widget creation and prop application), `HeaderBar.swift` /
`SplitController.swift` / `Layout.swift` (native chrome), `Icons.swift` (freedesktop → SF Symbol
mapping — see [Icons](/native-platform/icons/)), `Automation.swift`, `Events.swift`, and the
generated `Widgets.swift`.

## `examples/`

Real, driven apps used as framework-suitability stress tests, not toy snippets:

- `examples/counter/` — the minimal example: state, a click handler, `Suspense`, and a `useMemo`'d
  interval, in one `<window>`.
- `examples/notes/` — a two-pane notes app exercising native chrome (`<splitview>`, `<headerbar>`,
  `<toolbarview>`), `cssClasses`, and search.
- `examples/gallery/` — a broader widget gallery, including a 100k-row `<listview>` regression case.

## `template/` — the app scaffold

What `scripts/new-app.sh` copies to start a new app: a `package.json` that links
`@nativedesktop/react`, `nd`, and (transitively) `@nativedesktop/host` via `file:` paths into this
checkout (none are published to npm yet), a `src/main.tsx` entry point, a `babel.config.json` for the
opt-in React Compiler + hook-import-rewrite pre-pass, and a `bunfig.toml` that preloads the
`bun --hot`-path twin of that hook rewrite.

## `tools/`

Build-time scripts invoked as documented conventions: `tools/codegen.ts` (schema → bindings + docs),
`tools/package.ts` / `tools/package-linux.ts` / `tools/package-mac.ts` (see
[Packaging](/packaging/)), `tools/manifest.ts` (update manifests). The `nd` CLI (`packages/nd`) only
covers running/building an *app* (`nd dev`/`nd build`) — these `tools/` scripts have no `nd`
subcommand equivalent (no `nd package`, `nd codegen`, etc.) and are invoked directly with `bun`.

## `packages/mcp`

A stdio MCP server bridging the automation socket to MCP tool calls — see
[MCP Tools](/automation-testing/mcp-tools/).
