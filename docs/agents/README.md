# NativeDesktop — agent docs

Entry point for coding agents working on or building with NativeDesktop. See also:
`zig-idiom.md` (Zig 0.16 corrective idiom), `styling.md` (style prop pointer), `automation.md`
(the full RPC surface).

## What this framework is

NativeDesktop is a two-process framework: a Zig host owns a real native window (GTK4 on Linux,
AppKit/Win32 planned) and a Bun/TypeScript child renders a React tree into it over a local
protocol (NDP) — no DOM, no WebView, no Electron. The host is automation-first: every widget the
React tree creates is tracked and answerable over a JSON-RPC socket, so an agent can inspect and
drive the app the same way a user would. Full design: `docs/superpowers/specs/2026-07-09-nativedesktop-design.md`.

## Running an app

```
ND_SCRIPT=<entry.tsx> NATIVE_AUTOMATION=1 ./zig-out/bin/nd-hello
```

`ND_SCRIPT` points at the Bun/TSX entry point (e.g. `src/main.tsx`); `NATIVE_AUTOMATION=1` turns on
the automation RPC socket. Marker vocabulary to grep for in the host's stderr (all `ND_*` markers
print to stderr — capture `2>&1`):

| Marker | Meaning | Status |
|---|---|---|
| `ND_CHILD_CONNECTED` | the Bun child connected over the NDP socket | landed |
| `ND_COMMIT_APPLIED commitId=…` | a CommitBatch was applied to the retained tree | landed |
| `ND_AUTOMATION_LISTENING path=…` | the automation RPC socket is ready | landed |
| `ND_CHILD_EXITED` | the child disconnected (crash, `kill -9`, or clean exit) | landed |
| `ND_OVERLAY_SHOWN dev=…` | the host painted the crash overlay | landed |
| `ND_GC_SWEEP gen=… removed=…` | generation GC swept orphaned widgets after a reload | landed |
| `ND_RUNTIME_ERROR_REPORTED …` | the runtime reported an uncaught error before dying | landed |

`NDP_TRACE=1` (env var on the host) enables verbose per-frame NDP tracing — landed, useful when a
commit isn't showing up as expected.

`ND_DEV=1` (env var on the host) selects `bun --hot` for the child process and enables the
crash-overlay's Restart button. A packaged `nd dev` command that wraps `ND_DEV=1` plus toolchain
discovery is **M9 scope** (packaging), not M8 — do not attempt to build an `nd` CLI binary against
this doc set.

## HMR: what actually preserves state

**Landed, with one required convention: import hooks from `@nativedesktop/react`, not `react`.**
`ND_DEV=1` runs the Bun child under `bun --hot`, which keeps the same OS process and socket across
an edit, but re-evaluates the *entire* module graph on every edit — `react`, `react-reconciler`,
and `@nativedesktop/react` included (verified empirically against Bun 1.3.13 with a
`globalThis`-identity probe). Two mechanisms were tried and rejected before landing on the one
below — recorded here so nobody re-attempts them:

- **Bun-level module aliasing** (`Bun.plugin`'s `builder.module()`, loaded via
  `bun --hot --preload <script>`) can genuinely intercept bare specifiers like `"react"` and
  `"react-reconciler"` and survives re-evals (the preload script itself runs once per process, not
  per edit) — but Bun 1.3.13 throws `Requested module is already fetched` the moment an aliased
  specifier is touched by both an ESM `import` and a CJS `require()` anywhere in the process.
  `react-reconciler`'s bundled cjs does `require("react")` internally while this codebase and every
  app entry use genuine ESM `import` — that combination is unavoidable, so this mechanism cannot
  work without rewriting every consumer to `require()`.
- **`react-refresh`'s family-registration machinery**, wired through `reconciler.setRefreshHandler`
  (`packages/react/src/hmr.ts`), needs its own module instance pinned across re-evals too (it keeps
  its family/root registries in module-local closures, not on any object it hands out) — this part
  *does* work (react-refresh has no internal `require()`s of its own, so a `globalThis` stash is
  sufficient, no Bun-level aliasing needed) and is what actually makes a hot edit state-preserving:
  `render()` registers the root component under a fixed family key on first boot, and on every
  hot re-eval calls `hotUpdateRoot()` (not `updateContainer` again — a fresh `<App/>` element's
  `.type` is a new function reference every re-eval, which the reconciler would otherwise treat as
  a type change and fully remount) to re-register the new type under the same family and ask
  react-refresh to patch the live fiber tree in place.

What ships: `packages/react/src/dev-react.ts` stashes the **first-eval** `react` module instance in
`globalThis` and re-exports its hooks (`useState`, `useEffect`, `useMemo`, `use`, `Suspense`, etc.)
through wrapper functions that always dispatch to that stashed instance. The reconciler created on
first boot (`renderer.ts`, guarded by the existing `globalThis`-keyed singleton) already closes over
that same first-eval `react` instance internally, so a hook resolved through
`@nativedesktop/react` always talks to the dispatcher the live reconciler actually drives. A hook
imported from `react` directly would resolve against a fresh, re-evaluated instance whose
dispatcher is never attached to any reconciler — that's the `Invalid hook call:
resolveDispatcher().useState` crash this convention avoids. **Convention:** app code must write
`import { useState } from "@nativedesktop/react"`, not `from "react"` — `examples/counter` and
`template/` both follow this; `scripts/headless-m8.sh`'s HMR leg exercises the real
`examples/counter` app (not a synthetic fixture) end to end: click to a known state, edit a label
string in a temp copy, and assert the label changed, the click count survived, and the child never
disconnected.

## React Compiler: honest status (M8-D7)

**Opt-in, working, off by default.** `babel-plugin-react-compiler@1.0.0` runs cleanly as a build
pre-pass over the template's `src/` and the compiled output runs correctly against
`@nativedesktop/react` — verified headless (3 clicks, label updated, screenshot captured) this
session. It is a pre-pass, not inline, because Bun's runtime transpiler does not run babel plugins
and `bun --hot` re-evaluates modules through Bun's own transpiler only. The template's `bun run
compile` script (`template/package.json`) runs two babel plugins in one pass:
`babel-plugin-react-compiler` (the memoization transform) plus
`@babel/plugin-transform-react-jsx` (JSX to `@nativedesktop/react/jsx-runtime` calls — chosen
deliberately so the compiled output contains no JSX syntax left for Bun to pragma-select on, since
Bun's dev-vs-prod jsx-runtime selection is undocumented and version-fragile: it depends on
`NODE_ENV=production`, which is not honored consistently by `bun run` across 1.3.x). Fixed
alongside this: `packages/react/src/jsx-runtime.ts` re-exported the type-only `JSX` namespace as a
value (`export { …, JSX }`), which crashed any hand-authored `import … from
"@nativedesktop/react/jsx-runtime"` (exactly what the babel JSX transform emits) with `export 'JSX'
not found` — Bun's own automatic-JSX-runtime injection happened to elide it, masking the bug until
a real consumer imported the path directly. Changed to `export type { JSX }`. Not enabled by
default: `bun run dev` (`ND_DEV=1` + `--hot`) still points at uncompiled `src/`, so hot reload and
react-refresh are unaffected; use `bun run compile && ND_SCRIPT=dist/main.tsx <host-binary>` for a
compiled run.

## MCP tools

`packages/mcp` is a stdio MCP server that bridges to the host's automation socket. Four tools,
today:

- `nd_get_tree` — snapshot the widget tree (refs, testIDs, text, geometry).
- `nd_screenshot` — render the window to a PNG at an absolute path.
- `nd_click` — semantic click on a widget by ref.
- `nd_wait_for` — poll a tree condition (`textContains` or `refVisible`) until it holds or times out.

These are a thin pass-through to the raw RPC methods — see `automation.md` for the full method
list (including `setValue`/`type`/`scroll`, which exist on the raw socket but do not yet have MCP
tool wrappers) and the error-code contract.

## Crash debugging for agents

**Not yet landed** (lands with the M8 overlay task). Once shipped: when the Bun child crashes or
disconnects, the host paints an in-window overlay and tracks its widgets under a reserved
generation (`0xFFFF`) specifically so `getTree` keeps answering through a crash — an agent reads
the crash the same way it reads any other tree state. The plan: `getTree` will expose
`nd-overlay-title` / `nd-overlay-error` / `nd-overlay-restart` testIDs; read `nd-overlay-error`'s
`text` for the failure message, and (dev-mode only) `click` `nd-overlay-restart` to respawn the
child and recover. Today, a crash simply prints `ND_CHILD_EXITED` and the window goes stale with no
tracked recovery path — treat any crash as fatal to the current run until this lands.
