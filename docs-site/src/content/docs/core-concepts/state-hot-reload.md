---
title: State & Hot Reload
description: The hooks re-export contract that makes hot reload state-preserving, and React Compiler's opt-in status.
---

Status: landed, with one required convention for component files.

## Import hooks from `@nativedesktop/react`

`ND_DEV=1` runs the Bun child under `bun --hot`, which keeps the same OS process and NDP socket
across an edit but re-evaluates the entire module graph, `react` and `react-reconciler` included. A
hook imported straight from `react` resolves against a fresh module instance whose dispatcher was
never attached to any reconciler. That is the `Invalid hook call: resolveDispatcher().useState`
crash this convention avoids.

`packages/react/src/dev-react.ts` stashes the first-eval `react` module instance in
`globalThis` and re-exports its hooks through wrapper functions that always dispatch to that
stashed instance. The reconciler created on first boot closes over that same first-eval `react`
instance, so a hook resolved through `@nativedesktop/react` always talks to the dispatcher the live
reconciler actually drives.

```ts
import { useState, useEffect, useMemo } from "@nativedesktop/react"; // correct, every time
```

The rule applies to `.tsx` and `.desktop.tsx` component files. Shared platform-agnostic hooks, the
ones web or React Native code in the same monorepo also consumes, are the exception. Author those
the normal way in a plain `.ts` module with `import { useState } from "react"`, and
`babel-plugin-nativedesktop` rewrites the import to `@nativedesktop/react` for you, both under
`bun run compile` and under `bun --hot` through a Bun plugin preloaded from `bunfig.toml`.

The dev-path rewrite deliberately touches only `.ts` files. Intercepting a component file in Bun's
`onLoad` would drop it from `--hot`'s watch set and break its hot reload. The consequence is that a
shared `.ts` hook is pinned at first eval, so editing one needs a host restart, while its `.tsx`
consumers keep hot-reloading normally.

## What actually preserves state across an edit

`render()`'s hot-reload path doesn't call `updateContainer` again on re-eval: a fresh `<App/>`
element's `.type` is a new function reference every re-eval, which the reconciler would otherwise
treat as a type change and fully remount. Instead it calls `hotUpdateRoot()`, which re-registers the
new component type under the same `react-refresh` family key the first boot established, and asks
`react-refresh` to patch the live fiber tree in place. `react-refresh`'s family/root registries are
also pinned to the first-eval module instance via the same `globalThis` stash. No Bun-level module
aliasing is needed, since `react-refresh` has no internal `require()` calls of its own to trip over.

Bun's module aliasing was tried first and rejected, recorded here so nobody re-attempts it.
`Bun.plugin` aliasing throws `Requested module is already fetched` as soon as an aliased specifier
is touched by both ESM `import` and CJS `require()`. `react-reconciler`'s bundled CJS does exactly
that for `"react"` internally, and every app entry uses genuine ESM `import`, so the collision is
unavoidable.

## React Compiler, opt-in and off by default

Status: landed, opt-in. `babel-plugin-react-compiler@1.0.0` runs cleanly as a build pre-pass and the
compiled output runs correctly against `@nativedesktop/react`. It runs as a pre-pass rather than
inline because Bun's runtime transpiler does not run Babel plugins, and `bun --hot` re-evaluates
modules through Bun's own transpiler only.

The compile step runs three Babel plugins in one pass. `babel-plugin-react-compiler` does the
memoization transform. `@babel/plugin-transform-react-jsx` turns JSX into
`@nativedesktop/react/jsx-runtime` calls, chosen so the compiled output has no JSX syntax left for
Bun to pragma-select on. `babel-plugin-nativedesktop` applies the hook-import rewrite above across
every extension in the compiled path.

`nd dev` still points at uncompiled `src/`, so hot reload and react-refresh behave the same whether
or not the compiler is enabled. Use the compiled path for a production-style run:

```bash
bun run compile && ND_SCRIPT=dist/main.tsx ./zig-out/bin/nd-hello
```
