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

`@nativedesktop/react` is the package app code imports. It vendors a matched `react` +
`react-reconciler` pair, implements the host config that turns React commits into NDP
`CommitBatch` operations, and re-exports the hooks your app uses (see
[State & Hot Reload](/core-concepts/state-hot-reload/) for why the re-export matters).

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
`@nativedesktop/react` via a `file:` path into this checkout (not yet published to npm — that lands
with packaging), a `src/main.tsx` entry point, and a `babel.config.json` for the opt-in React
Compiler pre-pass.

## `tools/`

Build-time scripts invoked as documented conventions, not through a packaged CLI (there is no
`bin/nd` dispatcher yet): `tools/codegen.ts` (schema → bindings + docs), `tools/package.ts` /
`tools/package-linux.ts` / `tools/package-mac.ts` (see [Packaging](/packaging/)), `tools/manifest.ts`
(update manifests).

## `packages/mcp`

A stdio MCP server bridging the automation socket to MCP tool calls — see
[MCP Tools](/automation-testing/mcp-tools/).
