# Agent notes — NativeDesktop app

This app is built on the NativeDesktop framework (Zig host + Bun/React renderer). The framework's
full agent docs are copied into `docs/agents/` at scaffold time — read `docs/agents/README.md` first.

Three load-bearing rules:

1. **Run with `bun run dev` (== `nd dev`) for hot reload + the crash-restart overlay.** This wraps
   `ND_DEV=1 ND_SCRIPT=src/main.tsx <resolved-host-binary>` — see `docs/agents/README.md` for the
   marker vocabulary, what `ND_DEV` actually changes, and the raw invocation to use instead when
   iterating on the framework's Zig host itself (`nd dev` runs a prebuilt host binary, not a fresh
   `zig build`). **`.tsx`/`.desktop.tsx` component files must still import hooks from
   `@nativedesktop/react`, not `react`** (`import { useState } from "@nativedesktop/react"`) — this is
   what makes a hot edit preserve state instead of crashing or resetting. Shared, non-component `.ts`
   hooks are the one exception: they may import from plain `"react"` and get rewritten/pinned
   automatically; see `docs/agents/README.md`'s HMR and hook-rewrite sections for why, and for the
   `.desktop.tsx` convention.
2. **Styling is not web CSS.** No `flex`/`grid`/`position`/`display`/`justifyContent`. See
   `docs/agents/styling.md`.
3. **Zig idiom in this framework is 0.16, not the pre-2025 APIs most training data assumes.** If you touch
   the framework itself (not just this app), read `docs/agents/zig-idiom.md` before writing Zig.

Automation/testing surface (getTree, click, screenshot, waitFor, etc.) is documented in
`docs/agents/automation.md`.
