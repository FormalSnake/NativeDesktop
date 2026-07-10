# Agent notes — NativeDesktop app

This app is built on the NativeDesktop framework (Zig host + Bun/React renderer). The framework's
full agent docs are copied into `docs/agents/` at scaffold time — read `docs/agents/README.md` first.

Three load-bearing rules:

1. **Run with `ND_DEV=1` for hot reload + the crash-restart overlay.** `ND_DEV=1 ND_SCRIPT=src/main.tsx
   <path-to-nd-host-binary>` — see `docs/agents/README.md` for the marker vocabulary and what `ND_DEV`
   actually changes. (A packaged `nd dev` command that wraps this invocation ships in a later milestone;
   today `ND_DEV=1` on the host binary is the whole mechanism.)
2. **Styling is not web CSS.** No `flex`/`grid`/`position`/`display`/`justifyContent`. See
   `docs/agents/styling.md`.
3. **Zig idiom in this framework is 0.16, not the pre-2025 APIs most training data assumes.** If you touch
   the framework itself (not just this app), read `docs/agents/zig-idiom.md` before writing Zig.

Automation/testing surface (getTree, click, screenshot, waitFor, etc.) is documented in
`docs/agents/automation.md`.
