---
title: State & Hot Reload
description: The hooks re-export contract that makes hot reload state-preserving, and React Compiler's opt-in status.
---

## Import hooks from `@nativedesktop/react`

```ts
import { useState, useEffect, useMemo } from "@nativedesktop/react";
```

`ND_DEV=1` runs the Bun child under `bun --hot`, which keeps the same OS process and NDP socket
across an edit but re-evaluates the entire module graph, `react` and `react-reconciler` included. A
hook imported straight from `react` resolves against a fresh module instance whose dispatcher was
never attached to any reconciler, which surfaces as
`Invalid hook call: resolveDispatcher().useState`.

`packages/react/src/dev-react.ts` stashes the first-eval `react` module instance in `globalThis` and
re-exports its hooks through wrappers that always dispatch to that stashed instance. The reconciler
created on first boot closes over the same instance, so a hook resolved through
`@nativedesktop/react` always talks to the dispatcher the live reconciler drives.

The rule applies to `.tsx` and `.desktop.tsx` component files. Shared platform-agnostic hooks, the
ones web or React Native code in the same monorepo also consumes, are the exception: author those in
a plain `.ts` module with `import { useState } from "react"` and `babel-plugin-nativedesktop`
rewrites the import for you, both under `bun run compile` and under `bun --hot` through a Bun plugin
preloaded from `bunfig.toml`.

The dev-path rewrite touches `.ts` files only. Intercepting a component file in Bun's `onLoad` would
drop it from `--hot`'s watch set and break its hot reload. One consequence: a shared `.ts` hook is
pinned at first eval, so editing one needs a host restart, while its `.tsx` consumers keep
hot-reloading normally.

## What preserves state across an edit

`render()`'s hot-reload path does not call `updateContainer` again on re-eval. A fresh `<App/>`
element's `.type` is a new function reference every re-eval, which the reconciler would treat as a
type change and fully remount. Instead it calls `hotUpdateRoot()`, which re-registers the new
component type under the same `react-refresh` family key the first boot established and asks
`react-refresh` to patch the live fiber tree in place. `react-refresh`'s family and root registries
are pinned to the first-eval module instance via the same `globalThis` stash.

Bun-level module aliasing does not work here, so do not reach for it: `Bun.plugin` aliasing throws
`Requested module is already fetched` as soon as an aliased specifier is touched by both ESM
`import` and CJS `require()`, and `react-reconciler`'s bundled CJS requires `"react"` internally
while every app entry uses ESM.

## React Compiler

Opt-in, off by default. `babel-plugin-react-compiler@1.0.0` runs as a build pre-pass and its output
runs correctly against `@nativedesktop/react`. It has to be a pre-pass because Bun's runtime
transpiler does not run Babel plugins, and `bun --hot` re-evaluates modules through Bun's own
transpiler.

The compile step runs three Babel plugins in one pass:

- `babel-plugin-react-compiler` does the memoization transform.
- `@babel/plugin-transform-react-jsx` turns JSX into `@nativedesktop/react/jsx-runtime` calls, so
  the compiled output has no JSX syntax left for Bun to pragma-select on.
- `babel-plugin-nativedesktop` applies the hook-import rewrite across every extension.

`nd dev` still points at uncompiled `src/`, so hot reload and react-refresh behave the same whether
or not the compiler is enabled. For a production-style run:

```bash
bun run compile && ND_SCRIPT=dist/main.tsx ./zig-out/bin/nd-hello
```
