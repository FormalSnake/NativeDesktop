---
title: Monorepo & Code Sharing
description: How a NativeDesktop app shares signals and logic with other Solid apps in the same workspace, and how .desktop.tsx keeps desktop UI separate.
---

`@nativedesktop/react` declares `solid-js` as a `peerDependency` (`2.0.0-rc.13`) rather than
vendoring a copy, so a NativeDesktop app can sit in a monorepo next to a web app built on Solid and
share a logic package with it.

## Why the peer dependency matters

A workspace-aware package manager (Bun workspaces, npm, pnpm) hoists a single `solid-js` install
for every workspace member asking for a compatible version. Without the peer declaration, a linked
app and the package it links can resolve two different copies of `solid-js`. Each copy owns its own
reactive graph, so a signal created by one is invisible to an effect created by the other.

## An illustrative layout

This repository ships no multi-target example, since `examples/*` are all desktop apps. Here is the
shape the peer dependency supports, for a product repo with more than one client:

```text
my-product/
├── package.json          # workspaces: ["apps/*", "packages/*"]
├── apps/
│   ├── desktop/           # a NativeDesktop app (this framework)
│   └── web/                # a Solid web app
└── packages/
    └── shared-state/       # signals and stores: plain .ts, from "solid-js"
```

`apps/desktop` depends on `@nativedesktop/react`, `@nativedesktop/cli`, and `packages/shared-state`.
`apps/web` depends on `packages/shared-state` too. Every app in the workspace resolves the same
hoisted `solid-js`, so `packages/shared-state` needs no NativeDesktop-specific code.

## Writing shared state

`template/src/hooks/useToggle.ts` is the worked example. It imports from `"solid-js"` only, has no
JSX, and imports nothing from NativeDesktop, so the identical file works in a web Solid app:

```ts
// template/src/hooks/useToggle.ts
import { createSignal, type Accessor } from "solid-js";

export function useToggle(initial = false): [Accessor<boolean>, () => void] {
  const [on, setOn] = createSignal(initial);
  return [on, () => setOn((v) => !v)];
}
```

The `use` prefix is a naming habit, not a rule: Solid has no hook rules, and the function can be
called anywhere a signal can be created. A reader of the returned accessor tracks it like any other
signal.

## `.desktop.tsx` separates desktop UI from shared code

`.desktop.tsx` is a platform suffix. It is an ordinary `.tsx` file that TypeScript, ESLint,
Prettier, and Bun understand with no extra configuration. It resolves through extensionless
imports, so `import { Panel } from "./Panel.desktop"` finds `Panel.desktop.tsx`, and it goes
through the same Solid transform as any other `.tsx` in a package that depends on
`@nativedesktop/react`. `template/src/App.tsx` and `template/src/Panel.desktop.tsx` show the split:

```tsx
// template/src/Panel.desktop.tsx
import { useToggle } from "./hooks/useToggle.ts";

export function Panel() {
  const [open, toggle] = useToggle();
  return (
    <box orientation="vertical" spacing={8}>
      <label testID="panel-status" text={open() ? "Panel: open" : "Panel: closed"} />
      <button testID="panel-toggle" label="Toggle panel" onClick={toggle} />
    </box>
  );
}
```

State comes from the shared `useToggle`, and the markup, which renders to native widgets rather
than the DOM, stays in the desktop file.

## Which `.tsx` files get the transform

The Solid transform applies to a `.tsx` or `.jsx` file whose nearest `package.json` is
`@nativedesktop/react` or depends on it. A file outside such a package opts in with a pragma
that TypeScript reads as well:

```tsx
/** @jsxImportSource @nativedesktop/react */
```

Any other `.tsx` keeps Bun's own transform. A shared package that holds no NativeDesktop JSX needs
neither.

## Publishing a shared package

Give your own `shared-state` package the same shape as `@nativedesktop/react`: declare `solid-js` as
a `peerDependency` rather than a regular dependency, so it keeps resolving to whichever single
`solid-js` instance the consuming workspace hoists.

## Next

- [Architecture](/core-concepts/architecture/): how the two processes and the NDP protocol fit together.
- [State & Hot Reload](/core-concepts/state-hot-reload/): what an edit under `nd dev` preserves.
