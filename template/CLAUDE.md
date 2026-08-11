# CLAUDE.md — NativeDesktop app

See `AGENTS.md` in this directory for the three load-bearing rules (dev mode, hook imports, styling),
the `nd` CLI surface (`@nativedesktop/cli`: `nd dev` / `nd build` / `nd package` / `nd doctor`), the
`@nativedesktop/*` package set, and the error-policy/settings-store defaults.
The full framework agent docs live in `docs/agents/` (copied in at scaffold time):

- `docs/agents/README.md` — entry point, MCP tools, marker vocabulary, HMR/crash-debugging story.
- `docs/agents/zig-idiom.md` — Zig 0.16 wrong-vs-right pairs (only relevant if you touch the framework).
- `docs/agents/styling.md` — pointer to the generated styling reference.
- `docs/agents/automation.md` — the full RPC surface for driving/testing this app.

Read those before making framework-level changes. For app-level changes (this `src/` tree), standard
React rules apply — `App.tsx` is a normal React component tree rendered to native widgets, not the DOM.
