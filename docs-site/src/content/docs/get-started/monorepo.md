---
title: Monorepo & Code Sharing
description: How a NativeDesktop app shares hooks and logic with web and React Native apps in the same workspace, and how .desktop.tsx keeps desktop UI separate.
---

`@nativedesktop/react` declares `react` as a `peerDependency` (`^19.2.7`) rather than vendoring a
copy, so a NativeDesktop app can sit in a monorepo next to a web (`react-dom`) app and a React
Native app and share a hooks/logic package with both.

## Why the peer dependency matters

A workspace-aware package manager (Bun workspaces, npm, pnpm) hoists a single `react` install for
every workspace member asking for a compatible version. Without the peer declaration, a linked app
and the package it links can resolve two different copies of `react`. React's hooks dispatcher only
ever attaches to one of them, which surfaces as "Invalid hook call".

## An illustrative layout

This repository ships no multi-target example, since `examples/*` are all desktop apps. Here is the
shape the peer dependency supports, for a product repo with more than one client:

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
`apps/mobile` depend on `packages/shared-hooks` too. Every app in the workspace resolves the same
hoisted `react`, so `packages/shared-hooks` needs no NativeDesktop-specific code. Author it the way
you would for web or React Native alone.

## Writing a shared hook

`template/src/hooks/useToggle.ts` is the worked example. It imports from `"react"` the normal way,
so the identical file could go to a web bundler or to Metro unmodified:

```ts
// template/src/hooks/useToggle.ts
import { useState, useCallback } from "react";

export function useToggle(initial = false): [boolean, () => void] {
  const [on, setOn] = useState(initial);
  const toggle = useCallback(() => setOn((v) => !v), []);
  return [on, toggle];
}
```

Do not rewrite that `"react"` import by hand. NativeDesktop rewrites it to `@nativedesktop/react` in
both places source gets transformed.

Under `nd build`, `babel-plugin-nativedesktop` runs as an ordinary Babel visitor from
`template/babel.config.json`, alongside `babel-plugin-react-compiler` and the JSX transform. It
walks every `ImportDeclaration` for `"react"` and splits the hook specifiers into a second
`import { ... } from "@nativedesktop/react"`, leaving default, namespace, and type-only imports on
`"react"`. It runs against `.ts` and `.tsx` alike.

Under `nd dev`, Babel does not run inside Bun's transpiler, so the dev path uses a Bun `onLoad`
plugin instead (`packages/babel-plugin-nativedesktop/bun-plugin.js`, registered once per process
through `template/bunfig.toml`'s `preload`). It does the textual equivalent of the same rewrite.

Only the hook subset `packages/react/src/dev-react.ts` pins gets redirected: `useState`,
`useEffect`, `useLayoutEffect`, `useMemo`, `useCallback`, `useRef`, `useContext`, `useReducer`,
`useTransition`, `useDeferredValue`, `useSyncExternalStore`, `useId`, `use`, and `startTransition`.
Default imports, namespace imports, and `import type { ... } from "react"` are left alone.

## Why the dev path rewrites `.ts` only

The Bun `onLoad` rewrite filters on `/\.ts$/` and excludes `.tsx` and `.desktop.tsx`. Bun's runtime
`onLoad` has no fall-through: a matched file must return contents, and once a plugin returns
contents for a file, `bun --hot` drops it from the watch set. Intercepting a component file would
kill its hot reload. So the rewrite touches shared non-component `.ts` modules only, which is where
cross-platform hooks live anyway.

One consequence: a shared `.ts` hook is rewritten and pinned to `@nativedesktop/react` at first
eval, and since the dev-path rewrite runs once per process rather than per hot edit, editing that
hook needs a host restart. Its `.tsx` consumers keep hot-reloading in the meantime. They import
hooks from `@nativedesktop/react` directly, or from an already-rewritten shared `.ts` hook, never
from raw `"react"`, as [State & Hot Reload](/core-concepts/state-hot-reload/) describes. `nd build`
has no watcher to preserve, so it rewrites every extension unconditionally.

## `.desktop.tsx` separates desktop UI from shared code

`.desktop.tsx` is a platform suffix, the same convention React Native uses for `.native.tsx`. It is
an ordinary `.tsx` file that TypeScript, ESLint, Prettier, and Bun understand with no extra
configuration. It resolves through extensionless imports, so
`import { Panel } from "./Panel.desktop"` finds `Panel.desktop.tsx`, and it carries the
NativeDesktop JSX transform (`importSource: "@nativedesktop/react"` in
`template/babel.config.json`) like any other `.tsx`. `template/src/App.tsx` and
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
hook, and any hook it uses directly comes from `@nativedesktop/react`. That is the convention:
desktop-only UI in `.desktop.tsx`, shared logic in plain `.ts`.

## Publishing a shared package

Give your own `shared-hooks` package the same shape as `@nativedesktop/react`: declare `react` as a
`peerDependency` rather than a regular dependency, so it keeps resolving to whichever single `react`
instance the consuming workspace hoists, desktop, web, or React Native.

## Next

- [Architecture](/core-concepts/architecture/): how the two processes and the NDP protocol fit together.
- [State & Hot Reload](/core-concepts/state-hot-reload/): the hooks re-export convention this page's rewrite mechanics build on.
