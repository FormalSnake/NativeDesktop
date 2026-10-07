---
title: Multi-Window
description: Render more than one <window> root from a single Solid tree, and move a live widget between windows without reloading it.
---

Render multiple `<window>` roots, typically as siblings inside a fragment, and each becomes an
independent OS window on both backends, all driven by the same Bun/Solid process. Windows sharing a
`tabGroup` prop render as one tabbed window instead; see [Native Tabs](/native-platform/tabs/).
`examples/multiwindow/main.tsx` is the reference app for this page.

![One of the multiwindow example's two windows hosting a live webview on macOS (AppKit)](../../../assets/screens/appkit/multiwindow.png)

![One of the multiwindow example's two windows hosting a live webview on GNOME (GTK)](../../../assets/screens/gtk/multiwindow.png)

## Rendering more than one window

```tsx
import { render } from "@nativedesktop/react";

function App() {
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

await render(() => <App />);
```

The core reconciler (`src/tree.zig`) pools window handles by node id, so a `--hot` edit rebinds
existing windows instead of reopening them, and a genuinely new `<window>` node opens a fresh OS
window. Every window is rendered by the same tree in one process, so sharing state between them is
ordinary signals and closures. No IPC, unlike a multi-window Electron app where each window is
its own renderer process.

Automation, the crash overlay, window chrome, and the ACL are all per-window correct:

- A node's geometry and visibility, and therefore the bounds `getTree` reports plus `click`,
  `setValue`, `type`, `scroll`, and `waitFor`'s `refVisible` check, resolve against that widget's
  own window rather than a global. GTK uses `gtk_widget_get_root()`; AppKit resolves the live
  content view of `view.window`.
- `screenshot` renders whichever window `params.window` names.
- A JS crash brings down every window's UI at once, so the crash overlay paints on every open
  window and clears on every window on restart.
- Each `<toolbarview>` and headerbar attaches to its own owning `NSWindow`/`GtkWindow`, not
  whichever window was created last.
- `core:window.create` is ACL-gated per target window id, so a grants manifest can scope window
  creation to a specific window. A window-0 grant still applies everywhere, matching the default
  policy.

`getTree` scopes per window too: pass `window` (a Window node ref) and the snapshot covers that
window's subtree. Without it, the RPC returns the root/first window's tree, with every other
window's nodes attached as orphans directly under that root; each orphan's own `geometry` is still
correct, resolved against its own window as above.

## Moving a widget between windows without reloading it

Plain JSX cannot express this move safely. A node under a new parent is a different position in
the owner tree, so Solid disposes the old instance and creates a fresh one, which the host turns into
a native destroy and create. For a `<webview>` that throws away the WKWebView/WebKitGTK instance and
rebuilds it: the page reloads and scroll position, form input, and JS state go with it.

Three exports from `@nativedesktop/react` (`packages/react/src/renderer.ts`) work around it:

```ts
function createPool(): Pool
function Portal(props: { pool?: Pool; children?: JSX.Element }): JSX.Element
function moveNode(node: NdNodeRef, toParent: NdNodeRef, before?: NdNodeRef | null): void
```

- **`<Portal pool?>`** renders its children into a stable, off-window pool instead of wherever it
  sits in the tree, but its owner stays at that call site. Because the owner never moves, Solid
  never disposes the children, no matter which window later shows them. If you omit `pool`, a
  single process-lifetime pool shared across the app is used; call `createPool()` yourself (once,
  at module scope, never inside a component) if you want more than one.
- **`moveNode(node, toParent, before?)`** relocates only the live native widget under `toParent`
  (optionally positioned before another node); it never touches the Solid tree. `node` and
  `toParent` are what a host-element `ref` resolves to (`NdNodeRef`, the same handle
  [Imperative Commands & Refs](/core-concepts/imperative-commands/) uses).

A node rendered via `createPortal` is a live native widget the moment it mounts. It is attached to
no window until the first `moveNode` call places it somewhere visible.

```tsx
import { render, Portal, moveNode } from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";
import { Show, createSignal } from "solid-js";

function App() {
  let tab: NdNodeRef<"webview"> | undefined;
  let slotA: NdNodeRef<"box"> | undefined;
  let slotB: NdNodeRef<"box"> | undefined;
  const [host, setHost] = createSignal<"A" | "B">("A");

  function show(slot: NdNodeRef<"box"> | undefined, name: "A" | "B") {
    if (tab && slot) {
      moveNode(tab, slot);
      setHost(name);
    }
  }

  return (
    <>
      {/* The tab, pinned in the pool. Its owner never moves, so it is
          never disposed when it moves between windows. */}
      <Portal>
        <webview
          ref={(w) => (tab = w)}
          url="https://example.com/"
          style={{ hexpand: true, vexpand: true }}
        />
      </Portal>

      <window title="Window A" defaultWidth={560} defaultHeight={380}>
        <box ref={(b) => (slotA = b)} orientation="vertical">
          <button label="Bring tab here" onClick={() => show(slotA, "A")} />
          <Show when={host() !== "A"}>
            <label text="(tab is in Window B)" />
          </Show>
        </box>
      </window>

      <window title="Window B" defaultWidth={560} defaultHeight={380}>
        <box ref={(b) => (slotB = b)} orientation="vertical">
          <button label="Bring tab here" onClick={() => show(slotB, "B")} />
          <Show when={host() !== "B"}>
            <label text="(tab is in Window A)" />
          </Show>
        </box>
      </window>
    </>
  );
}

await render(() => <App />);
```

Render the portal at a stable position (one per movable item, at or near the app root) so it outlives any single window it might currently be showing in.

### Why it is imperative

`moveNode` breaks from the declarative model the rest of the toolkit follows because the thing being
preserved, a widget's live native state, is exactly what disposing and recreating the node would destroy. It rides the
same `widgetCommand` channel as [`sendCommand`](/core-concepts/imperative-commands/) under a
reserved command name, into a `reparent_child` op on the host ABI vtable, so it reaches the native
widget through the same C-ABI seam as every other host operation with no protocol or schema change.
GTK brackets the move in a `g_object_ref`/`unref` pair; AppKit takes a retain across it. Either way
the widget is never transiently deallocated mid-reparent.
