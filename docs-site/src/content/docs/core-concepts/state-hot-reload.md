---
title: State & Hot Reload
description: What an edit under nd dev preserves, how Solid's refresh runtime patches components in place, and where state should live.
---

Components keep their state across an edit under `nd dev`. Edit a component and Solid's refresh
runtime patches it in place; the window, the native widgets, and the signals of every component you
did not touch stay as they were.

## What `nd dev` runs

`ND_DEV=1` runs the Bun child under `bun --hot`, which keeps the same OS process and NDP socket
across an edit but re-evaluates the module graph. Three pieces make that state-preserving, all in
`@nativedesktop/react/register`, the preload `nd dev` passes to Bun:

- **Refresh transform.** Each component module compiles with Solid's refresh transform. A component
  becomes a proxy over its current implementation, and the module registers an accept callback.
  When the edited module has evaluated, that callback swaps the live proxies to the new code.
- **Pinned modules.** `solid-js`, its reactive core, the universal renderer, and
  `@nativedesktop/react` keep the instance from first evaluation. A fresh copy would own a second
  reactive graph that the mounted tree knows nothing about, or lose the renderer's retained tree.
- **`render()` after the first call.** Re-evaluating the entry calls `render()` again. Under the
  refresh runtime it records the new `() => <App />` and leaves the mounted tree alone.

## What an edit preserves

Editing a component re-runs that component's body, in place. Its parent, its siblings, the native
window, and the signals and stores owned by components you did not edit keep their values. Only
the edited component's own state is created again.

Removing a component, or any change the runtime cannot patch, remounts the whole tree from the
newest `render()` call. The window count and node count stay the same; the state does not.

## Where state should live

State created inside a component body survives an edit to a sibling. State created at module scope
is created again whenever that module is re-evaluated, and `bun --hot` re-evaluates modules on
an edit. A module-scope signal in a file you edit therefore resets.

A fresh object created at module scope also counts as a changed dependency of the components that
read it, and the runtime remounts them. `examples/counter/main.tsx` creates its pending promise
inside `App` for this reason.

Put long-lived data in `createStore` from `@nativedesktop/react`, whose value is persisted to disk
and loaded before `render()` (see [App Data & Storage](/core-concepts/app-data-storage/)), or keep
it in a component that is not the one being edited. Split a component out when you want to edit
its markup without resetting its parent's state, as `ClicksLabel` is split from `App` in the
counter example.

## Production builds

`nd build` has no watcher. It runs the app's `compile` script, `nd-solid-build src/main.tsx --outdir
dist`, which applies the same JSX transform ahead of time and bundles the app's own modules into
`dist/main.js`. Packages stay external imports, so `solid-js` resolves through the register preload
when the app launches. A packaged app starts without loading Babel.

```bash
bun run compile
```

## Raw invocation

When you iterate on the framework's own host, run the child yourself and pass the preload:

```bash
BUN_OPTIONS=--preload=@nativedesktop/react/register ND_DEV=1 ND_SCRIPT=src/main.tsx <path-to-nd-host-binary>
```
