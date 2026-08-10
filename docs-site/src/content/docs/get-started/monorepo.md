---
title: Monorepo & Code Sharing
description: How a NativeDesktop app shares hooks and logic with web and React Native apps in the same workspace, and how .desktop.tsx keeps desktop UI separate.
---

`@nativedesktop/react` declares `react` as a `peerDependency` (`^19.2.7`) rather than vendoring a
copy. That single detail lets a NativeDesktop app sit in a monorepo next to a web (`react-dom`) app
and a React Native app and share a hooks/logic package with both, instead of living in an isolated
checkout of its own.

## Why the peer dependency matters

A workspace-aware package manager (Bun workspaces, npm, pnpm) hoists a single `react` install for
every workspace member that asks for a compatible version. Before `@nativedesktop/react` declared
`react` as a peer, it vendored its own `react` and `react-reconciler` pair as regular dependencies.
A linked app and the package it linked could then resolve two different copies of `react`, and
React's hooks dispatcher only ever attaches to one of them, which surfaces as "Invalid hook call".
The old `template/` worked around it with a `postinstall` script (`scripts/dedupe-react.mjs`) that
re-pointed the app's `node_modules/react` at the linked package's copy by hand. That script is
gone. With `react` as a peer, the workspace hoist dedupes for free, and the guarantee covers every
workspace member rather than the one app the script patched.

## An illustrative layout

NativeDesktop's own repository ships no multi-target example, since `examples/*` are all desktop
apps. This is the shape the peer dependency is designed to support, for a product repo with more
than one client:

```text
my-product/
├── package.json          # workspaces: ["apps/*", "packages/*"]
├── apps/
│   ├── desktop/           # a NativeDesktop app (this framework)
│   ├── web/                # a react-dom app
│   └── mobile/             # a React Native app
└── packages/
    └── shared-hooks/       # cross-platform hooks: plain .ts, `from "react"`
```

`apps/desktop` depends on `@nativedesktop/react`, `nd`, and `packages/shared-hooks`. `apps/web` and
`apps/mobile` depend on `packages/shared-hooks` too. Because every app in the workspace resolves the
same hoisted `react`, `packages/shared-hooks` needs no NativeDesktop-specific code. You author it
the way you would for web or React Native alone.

## Writing a shared hook

`template/src/hooks/useToggle.ts` is the framework's worked example. It imports from `"react"` the
normal way, so the identical file could be handed to a web bundler or to Metro unmodified:

```ts
// template/src/hooks/useToggle.ts
import { useState, useCallback } from "react";

export function useToggle(initial = false): [boolean, () => void] {
  const [on, setOn] = useState(initial);
  const toggle = useCallback(() => setOn((v) => !v), []);
  return [on, toggle];
}
```

You do not rewrite that `"react"` import by hand. NativeDesktop rewrites it to
`@nativedesktop/react` in both places source gets transformed.

Under `nd build`, `babel-plugin-nativedesktop` runs as an ordinary Babel visitor from
`template/babel.config.json`, alongside `babel-plugin-react-compiler` and the JSX transform. It
walks every `ImportDeclaration` for `"react"` and splits the hook specifiers into a second
`import { ... } from "@nativedesktop/react"`, leaving default, namespace, and type-only imports on
`"react"`. It runs against every extension, `.ts` and `.tsx` alike.

Under `nd dev`, Babel does not run inside Bun's transpiler, so the dev path uses a Bun `onLoad`
plugin instead (`packages/babel-plugin-nativedesktop/bun-plugin.js`, registered once per process
through `template/bunfig.toml`'s `preload`). It does the textual equivalent of the same rewrite.

Only the hook subset that `packages/react/src/dev-react.ts` pins gets redirected: `useState`,
`useEffect`, `useLayoutEffect`, `useMemo`, `useCallback`, `useRef`, `useContext`, `useReducer`,
`useTransition`, `useDeferredValue`, `useSyncExternalStore`, `useId`, `use`, and `startTransition`.
A default import, a namespace import, and `import type { ... } from "react"` are left alone.

## Why the dev path rewrites `.ts` only

The Bun `onLoad` rewrite filters on `/\.ts$/` and deliberately excludes `.tsx` and `.desktop.tsx`.
Bun's runtime `onLoad` has no fall-through: a matched file must return contents, and once a plugin
returns contents for a file, `bun --hot` drops it from the watch set. Intercepting a component file
would silently kill its hot reload. So the rewrite touches shared, non-component `.ts` modules only,
which is where cross-platform hooks live anyway.

One consequence is worth knowing. A shared `.ts` hook is rewritten and pinned to
`@nativedesktop/react` at first eval, and because the dev-path rewrite runs once per process rather
than per hot edit, editing that hook needs a host restart. Its `.tsx` consumers keep hot-reloading
normally in the meantime. They import hooks from `@nativedesktop/react` directly, or from an
already-rewritten shared `.ts` hook, never from raw `"react"`, as
[State & Hot Reload](/core-concepts/state-hot-reload/) describes. `nd build` has no watcher to
preserve, so it rewrites every extension unconditionally.

## `.desktop.tsx` separates desktop UI from shared code

`.desktop.tsx` is a platform suffix, the same convention React Native uses for `.native.tsx`. It is
an ordinary `.tsx` file that TypeScript, ESLint, Prettier, and Bun all understand with no extra
configuration. It resolves through extensionless imports, so
`import { Panel } from "./Panel.desktop"` finds `Panel.desktop.tsx`, and it carries the
NativeDesktop JSX transform (`importSource: "@nativedesktop/react"` in
`template/babel.config.json`) like any other `.tsx` file. `template/src/App.tsx` and
`template/src/Panel.desktop.tsx` show the split:

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

`Panel.desktop.tsx` imports nothing from raw `"react"`. Its state comes from the shared `useToggle`
hook, and any hook it uses directly comes from `@nativedesktop/react`. That is the whole convention:
desktop-only UI in `.desktop.tsx`, shared logic in plain `.ts`, composed the same way `.native.tsx`
and shared hooks compose in a React Native codebase.

## Publishing a shared package

`@nativedesktop/react` itself is npm-publishable (`publishConfig.access: "public"`, a `files:
["dist"]` allowlist, a `postinstall` build step), though it isn't published yet; every package in
this repo links via `file:`/`workspace:` paths (see [Project Layout](/get-started/project-layout/)).
If you publish your own `shared-hooks` package, give it the same shape. Declare `react` as a
`peerDependency` rather than a regular dependency, so it keeps resolving to whatever single `react`
instance the consuming workspace hoists, whether that workspace is desktop, web, or React Native.
