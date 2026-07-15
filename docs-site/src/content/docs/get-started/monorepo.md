---
title: Monorepo & Code Sharing
description: How a NativeDesktop app shares hooks and logic with web and React Native apps in the same workspace, and how .desktop.tsx keeps desktop UI separate.
---

`@nativedesktop/react` declares `react` as a `peerDependency` (`^19.2.7`) rather than vendoring a
copy. That single detail lets a NativeDesktop app sit in a monorepo next to a web (`react-dom`) app
and a React Native app and share a hooks/logic package with both, instead of living in an isolated
checkout of its own.

## Why the peer dependency matters

A workspace-aware package manager (Bun's own workspaces, npm, pnpm) hoists a single `react` install
for every workspace member that asks for a compatible version. Before `@nativedesktop/react`
declared `react` as a peer, it vendored its own `react`/`react-reconciler` pair as regular
dependencies, so a linked app and the package it linked could each resolve a *different* copy of
`react`, and React's hooks dispatcher only ever attaches to one of them ("Invalid hook call"). The
old `template/` worked around this with a `postinstall` script
(`scripts/dedupe-react.mjs`) that re-pointed the app's own `node_modules/react` at the linked
package's copy by hand. That workaround is gone: with `react` as a peer, the workspace hoist does
the deduping for free, and the same guarantee extends to every workspace member instead of just
the one app the old script patched.

## An illustrative layout

NativeDesktop's own repository doesn't ship a multi-target example; `examples/*` are all desktop
apps. This is the shape the peer-dependency change is designed to support, for a product repo that
has more than one client:

```text
my-product/
├── package.json          # workspaces: ["apps/*", "packages/*"]
├── apps/
│   ├── desktop/           # a NativeDesktop app (this framework)
│   ├── web/                # a react-dom app
│   └── mobile/             # a React Native app
└── packages/
    └── shared-hooks/       # cross-platform hooks — plain .ts, `from "react"`
```

`apps/desktop` depends on `@nativedesktop/react`, `nd`, and `packages/shared-hooks`; `apps/web` and
`apps/mobile` depend on `packages/shared-hooks` too. Because every app in the workspace resolves the
same hoisted `react`, `packages/shared-hooks` needs no NativeDesktop-specific code at all — it's
authored exactly the way you'd write it for web or React Native alone.

## Writing a shared hook

`template/src/hooks/useToggle.ts` is the framework's own worked example of a shared hook — authored
the normal way, importing from `"react"`, so the identical file could be handed to a web bundler or
Metro unmodified:

```ts
// template/src/hooks/useToggle.ts
import { useState, useCallback } from "react";

export function useToggle(initial = false): [boolean, () => void] {
  const [on, setOn] = useState(initial);
  const toggle = useCallback(() => setOn((v) => !v), []);
  return [on, toggle];
}
```

You do not rewrite that `"react"` import by hand. NativeDesktop's build pipeline rewrites it to
`@nativedesktop/react` automatically, in both places source gets transformed:

- **`nd build`/`bun run compile`** — `babel-plugin-nativedesktop` runs as an ordinary Babel visitor
  (`template/babel.config.json`, alongside `babel-plugin-react-compiler` and the JSX transform). It
  walks every `ImportDeclaration` for `"react"` and splits any *hook* specifiers into a second
  `import { ... } from "@nativedesktop/react"`, leaving default/namespace/type-only imports on
  `"react"`. This runs against every extension, `.ts` and `.tsx`/`.desktop.tsx` alike.
- **`nd dev`/`bun --hot`** — Babel doesn't run under Bun's own transpiler, so the dev path uses a
  Bun `onLoad` plugin instead (`packages/babel-plugin-nativedesktop/bun-plugin.js`, registered once
  per process via `template/bunfig.toml`'s `preload = ["babel-plugin-nativedesktop/bun-plugin"]`).
  It does the textual equivalent of the same rewrite on raw source text.

Only the hook subset `packages/react/src/dev-react.ts` pins gets redirected — `useState`,
`useEffect`, `useLayoutEffect`, `useMemo`, `useCallback`, `useRef`, `useContext`, `useReducer`,
`useTransition`, `useDeferredValue`, `useSyncExternalStore`, `useId`, `use`, and `startTransition`.
A default import (`import React from "react"`), a namespace import, or a `import type { ... } from
"react"` is left alone.

## The dev-path nuance: `.ts` only, and why

The Bun `onLoad` rewrite (the `nd dev`/`bun --hot` path) filters on `/\.ts$/` and deliberately
excludes `.tsx` and `.desktop.tsx`. Bun's runtime `onLoad` has no fall-through: a matched file
must return contents, and once a plugin returns contents for a file, `bun --hot` drops that file
from its watch set. Intercepting a component file would silently kill its hot reload. So the
rewrite only ever touches shared, non-component `.ts` modules, which is exactly where
cross-platform hooks live.

That has a real consequence: a shared `.ts` hook is rewritten and pinned to `@nativedesktop/react`
at first eval, but because the dev-path rewrite runs once per process rather than per hot edit,
editing a shared `.ts` hook needs a host restart to take effect. Its `.tsx`/`.desktop.tsx`
consumers are unaffected and keep hot-reloading normally in the meantime — they still import their
hooks from `@nativedesktop/react` directly (or from an already-rewritten shared `.ts` hook), never
from raw `"react"`, exactly as [State & Hot Reload](/core-concepts/state-hot-reload/) describes. The
compiled path (`nd build`) has no watcher to preserve, so this restriction doesn't apply there; it
rewrites every extension unconditionally.

## `.desktop.tsx`: separating desktop UI from shared code

`.desktop.tsx` is a platform suffix, the same convention React Native uses for `.native.tsx`.
It's an ordinary `.tsx` file that TypeScript, ESLint, Prettier, and
Bun all understand with no extra configuration, it resolves through extensionless imports
(`import { Panel } from "./Panel.desktop"` finds `Panel.desktop.tsx`), and it carries the
NativeDesktop JSX transform (`importSource: "@nativedesktop/react"`, set in
`template/babel.config.json`) exactly like a plain `.tsx` file. `template/src/App.tsx` and
`template/src/Panel.desktop.tsx` are the framework's own example of the split:

```tsx
// template/src/Panel.desktop.tsx
import { useToggle } from "./hooks/useToggle.ts";

export function Panel(): React.ReactNode {
  const [open, toggle] = useToggle();
  return (
    <box orientation="vertical" spacing={8}>
      <label testID="panel-status" text={open ? "Panel: open" : "Panel: closed"} />
      <button testID="panel-toggle" label="Toggle panel" onClick={toggle} />
    </box>
  );
}
```

`Panel.desktop.tsx` imports nothing from raw `"react"` — its state comes from the shared
`useToggle` hook, and any hook it uses directly comes from `@nativedesktop/react`. That's the
convention: desktop-only UI lives in `.desktop.tsx`, shared logic lives in plain `.ts`, and the two
compose the same way `.native.tsx` and shared hooks compose in a React Native codebase.

## Publishing a shared package

`@nativedesktop/react` itself is npm-publishable (`publishConfig.access: "public"`, a `files:
["dist"]` allowlist, a `postinstall` build step), though it isn't published yet; every package in
this repo links via `file:`/`workspace:` paths (see [Project Layout](/get-started/project-layout/)).
If you publish your own `shared-hooks` package, give it the same shape: declare `react` as a
`peerDependency` rather than a regular dependency, so it keeps resolving to whatever single `react`
instance the consuming workspace hoists — desktop, web, or React Native.
