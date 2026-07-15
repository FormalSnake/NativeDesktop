---
title: Imperative Commands & Refs
description: A schema-typed channel for one-shot imperative actions on a live widget — sendCommand + a host-element ref, marshaled to the UI thread over the widgetCommand NDP frame.
---

Most of what an app does to a widget is **declarative**: you set a prop and the host reconciles the
native widget to match. A few things aren't. "Go back in history", "reload", "stop the load" are
**one-shot actions** with no state to bind — there is no `didGoBack` prop that makes sense to hold in
React. NativeDesktop models these as **imperative commands**: you take a `ref` on a widget and call
`sendCommand(ref, command)`. This is the escape hatch for genuinely imperative operations; anything
that *is* stateful should stay a prop.

## Declaring commands in the schema

A widget opts into the channel with a `commands` array in `schema/widgets.json`, listing the command
names it accepts:

```json
{
  "name": "WebView",
  "intrinsic": "webview",
  "commands": ["goBack", "goForward", "reload", "stop"],
  …
}
```

`tools/codegen.ts` turns that one declaration into every piece of the pipeline: the TypeScript
`WidgetCommandNames` map that types `sendCommand`, the runtime `widgetCommands` validation table, and
a per-backend dispatch arm on each host (Zig and Swift). A widget with a non-empty `commands` array
that lacks a host dispatch template makes codegen throw — the same fail-loud contract the create/apply
templates use — so the three sides can never drift. `<webview>` (`goBack`/`goForward`/`reload`/`stop`)
was the first widget on this channel; `<window>` (`showAlert`/`openFile`/`saveFile`/`showAbout`, see
[Dialogs](/components/dialogs/)) and `<toastoverlay>` (`showToast`/`dismissToast`, see
[Feedback](/components/feedback/)) followed, both
wrapped in a promise-correlating helper rather than called through raw `sendCommand`.

## Getting a ref and calling sendCommand

Every intrinsic accepts a `ref`. It resolves to an `NdNodeRef<T>` — the node's wire id plus its
intrinsic type, `{ id, type }` — which is the handle `sendCommand` addresses:

```tsx
import { sendCommand, useRef } from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";

const page = useRef<NdNodeRef<"webview">>(null);

// …later, from an event handler:
sendCommand(page.current, "goBack");
```

```ts
sendCommand<T>(node: NdNodeRef<T>, command: WidgetCommandNames[T], arg?: unknown): void
```

The `command` argument is typed to the commands that *this* widget declares, so
`sendCommand(page.current, "loadURL")` is a compile error on a `<webview>`. At runtime `sendCommand`
validates the name again against the `widgetCommands` table and throws if it isn't allowed (or if it
is called before `render()` has opened the NDP connection), so a stale string fails loudly on the app
side rather than being silently dropped by the host. The optional `arg` is JSON-serialized and passed
through to the host; no current command uses it, but the channel carries it for commands that will.

Call `sendCommand` from an event handler — a click, a menu selection — never from render. It is a
side effect, not derived state.

## What happens on the wire

`sendCommand` emits a `widgetCommand` NDP frame, `{ nodeId, command, arg }`, from the runtime to the
host. On the host it is handled exactly like a commit: it is **marshaled onto the UI thread**, because
it touches live native widgets. Socket FIFO ordering guarantees a command sent right after a commit is
applied after that commit, so a node created in the previous batch is always resolvable by the time
its command runs. The host resolves `nodeId` to the widget, looks up its kind, and calls the
generated `widgetCommand` dispatcher, which routes to the widget's arm (`goBack` etc.). Unknown
node ids or command names are dropped host-side with an `ND_WARN` line.

The channel is a dedicated `widget_command` entry on the `nd_backend` ABI vtable, so a command reaches
the native widget through the same C ABI as every other host operation — there is no widget-specific
side path.

## Capability gating

A widget command mutates live UI, so it goes through the **same capability gate as commit
application**: the runtime checks `core:commit` before dispatching. If the app's grants manifest
denies it, the command is refused with `ND_ACL_DENY permission=core:commit` and an `error` frame
("capability denied") goes back to the app instead of touching the widget. An app that is allowed to
render is therefore allowed to command; one that is sandboxed out of committing cannot drive widgets
imperatively either.
