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
| `ND_OVERLAY_SHOWN dev=…` | the host painted the crash overlay | **lands with the M8 overlay task** |
| `ND_GC_SWEEP gen=… removed=…` | generation GC swept orphaned widgets after a reload | **lands with the M8 overlay task** |
| `ND_RUNTIME_ERROR_REPORTED …` | the runtime reported an uncaught error before dying | **lands with the M8 overlay task** |

`NDP_TRACE=1` (env var on the host) enables verbose per-frame NDP tracing — landed, useful when a
commit isn't showing up as expected.

`ND_DEV=1` is the documented **future** entry point for hot reload + the crash-restart button
(M8-D1): it is not yet read by the host. A packaged `nd dev` command that wraps `ND_DEV=1` plus
toolchain discovery is **M9 scope** (packaging), not M8 — do not attempt to build an `nd` CLI binary
against this doc set.

## HMR: what actually preserves state

**Not yet landed.** The M8 plan's design (see `docs/superpowers/plans/2026-07-10-m8-dx.md`) is:
`ND_DEV=1` runs the Bun child under `bun --hot`, which keeps the same OS process and socket across
an edit (verified empirically against Bun 1.3.13: same pid, `globalThis` state survives, but the
entry's top-level statements re-run on every graph edit — so `render()` needs a `globalThis`-keyed
singleton guard to stay idempotent). React's hook-state preservation across an edit is meant to be
wired through `react-reconciler`'s `setRefreshHandler` plus `react-refresh`'s runtime, manually
registered (Bun's `--hot` does **not** auto-inject `$RefreshReg$`/`$RefreshSig$` the way a
babel-based toolchain would). A bounded `globalThis`-backed-store fallback is defined if manual
family registration proves too fragile. **Until this lands, every edit under this framework is a
full process restart** — there is no partial reload today. Re-read this section once the HMR task
ships; it will name which of the two mechanisms (react-refresh proper, or the store fallback)
actually shipped.

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
