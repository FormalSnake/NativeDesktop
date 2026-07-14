---
title: Multi-Window
description: Render more than one <window> root from a single React tree, and move a live widget between windows without reloading it.
---

A NativeDesktop app isn't limited to one `<window>`. Render multiple `<window>` roots — a sibling
list, typically inside a fragment — and each becomes an independent OS window on both backends, all
driven by the same Bun/React process. `examples/multiwindow/main.tsx` is the reference app for
everything on this page.

## Rendering more than one window

```tsx
import { render } from "@nativedesktop/react";

function App(): React.ReactNode {
  return (
    <>
      <window title="Window A" defaultWidth={560} defaultHeight={380}>
        {/* … */}
      </window>
      <window title="Window B" defaultWidth={560} defaultHeight={380}>
        {/* … */}
      </window>
    </>
  );
}

await render(<App />);
```

The core reconciler (`src/tree.zig`) pools window handles by node id, so a `--hot` edit rebinds
existing windows instead of reopening them, and a genuinely new `<window>` node opens a fresh OS
window. Because every window is rendered by the same tree in one process, sharing state between them
is just ordinary React state and closures — there's no IPC to wire up, unlike a multi-window
Electron app where each window is its own renderer process.

**Known limitation:** several pieces of window-scoped machinery still target only the *last*
rendered window — the overlay, automation's `getTree`/screenshot, and the toolbar/headerbar manager.
Per-window ACL also still checks window 0. Don't rely on automation or the crash-restart overlay
being window-aware yet in a multi-window app.

## Moving a widget between windows without reloading it

Cross-window reparenting is the harder problem multi-window raises: how do you move a *live* widget
— say, a browser tab — from Window A to Window B without losing its state?

Plain React can't express this move safely. Moving a node to a new parent is a different position in
the fiber tree, and React's model is to unmount the old instance and mount a fresh one at the new
position. The host turns that unmount+mount into a native destroy+create — which for a `<webview>`
means the WKWebView/WebKitGTK instance is thrown away and rebuilt, so the page reloads and every bit
of in-page state (scroll position, form input, JS state) is lost. This isn't a bug to fix; it's what
`UI = f(state)` means; a plain re-render can't "know" to preserve a widget identity across a
parent change.

The fix works around it with two functions from `@nativedesktop/react`
(`packages/react/src/renderer.ts`):

```ts
function createPool(): Pool
function createPortal(children: ReactNode, pool?: Pool): ReactPortal
function moveNode(node: NdNodeRef, toParent: NdNodeRef, before?: NdNodeRef | null): void
```

- **`createPortal(children, pool?)`** renders `children` into a stable, off-window **pool** instead
  of wherever it's called from in the tree — but its React fiber stays at that call site. Because the
  fiber's *position* never changes, React never unmounts it, no matter which window later shows it.
  If you omit `pool`, a single process-lifetime pool shared across the app is used; call
  `createPool()` yourself (once, at module scope or in a ref — never inside render) if you want more
  than one.
- **`moveNode(node, toParent, before?)`** relocates only the *live native widget* under `toParent`
  (optionally positioned before another node) — it never touches the React tree. `node` and
  `toParent` are what a host-element `ref` resolves to (`NdNodeRef`, the same handle
  [Imperative Commands & Refs](/core-concepts/imperative-commands/) uses).

A node rendered via `createPortal` is a live, real native widget the moment it mounts — it's just
attached to no window (the pool) until the first `moveNode` call places it somewhere visible.

```tsx
import { render, createPortal, moveNode, useEffect, useRef, useState } from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";

function App(): React.ReactNode {
  const tab = useRef<NdNodeRef<"webview">>(null);
  const slotA = useRef<NdNodeRef<"box">>(null);
  const slotB = useRef<NdNodeRef<"box">>(null);
  const [host, setHost] = useState<"A" | "B">("A");

  function show(slot: NdNodeRef<"box"> | null, name: "A" | "B") {
    if (tab.current && slot) {
      moveNode(tab.current, slot);
      setHost(name);
    }
  }

  return (
    <>
      {/* The tab, pinned in the pool. Its React position never changes, so it's
          never unmounted when it moves between windows. */}
      {createPortal(
        <webview ref={tab} url="https://example.com/" style={{ hexpand: true, vexpand: true }} />,
      )}

      <window title="Window A" defaultWidth={560} defaultHeight={380}>
        <box ref={slotA} orientation="vertical" style={{ hexpand: true, vexpand: true }}>
          <button label="Bring tab here" onClick={() => show(slotA.current, "A")} />
          {host !== "A" && <label text="(tab is in Window B)" />}
        </box>
      </window>

      <window title="Window B" defaultWidth={560} defaultHeight={380}>
        <box ref={slotB} orientation="vertical" style={{ hexpand: true, vexpand: true }}>
          <button label="Bring tab here" onClick={() => show(slotB.current, "B")} />
          {host !== "B" && <label text="(tab is in Window A)" />}
        </box>
      </window>
    </>
  );
}

await render(<App />);
```

Render the portal at a stable position — one per movable item, keyed by its own id, at (or near) the
app root — so it outlives any single window it might currently be showing in.

### Why this is imperative, on purpose

`moveNode` deliberately breaks from the declarative "set a prop, let the reconciler figure it out"
model the rest of the toolkit follows — because the thing being preserved (a widget's live native
state) is exactly what React's own model would otherwise destroy. `moveNode` rides the same
`widgetCommand` channel as [`sendCommand`](/core-concepts/imperative-commands/), under a reserved
command name, into an appended `reparent_child` op on the host ABI vtable — so it reaches the native
widget through the same C-ABI seam as every other host operation, with no protocol or schema change
needed. On GTK the move is bracketed in a `g_object_ref`/`unref` pair; on AppKit the core takes a
retain across the move — both exist so the widget is never transiently deallocated mid-reparent.
