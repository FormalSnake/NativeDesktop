---
title: Dialogs
description: showAlert, openFile, saveFile, and showAbout — native modal dialogs scoped to one <window>, driven as promise-wrapped imperative commands.
---

`@nativedesktop/react` exposes four native, per-window modal dialogs — a confirmation alert, an
open-file panel, a save-file panel, and the app's About panel — as promise-returning functions layered
over the `<window>` widget's [imperative commands](/core-concepts/imperative-commands/):
`NSAlert`/`NSOpenPanel`/`NSSavePanel` sheets on macOS, `AdwAlertDialog`/`GtkFileDialog` on GTK.

:::note
This is a **different mechanism** from the ACL-gated `dialog.*` API documented in
[System Capabilities](/native-platform/system-capabilities/#dialogs). Reach for `dialog.*` for an
app-level dialog with no particular window in mind; reach for these when the dialog is a modal sheet
scoped to one specific `<window>` node (and, for `showAlert`/`showAbout`, when you need buttons or
content `dialog.showMessage` doesn't cover).
:::

## Wiring a window for dialogs

Because a modal dialog is one-per-window on both backends, these calls correlate their result to the
`<window>`'s own wire id rather than a generated per-call token. That means the window needs a `ref`
and three result-event props wired back to the matching helper, once, regardless of how many places
in your tree trigger a dialog:

```tsx
import {
  showAlert, openFile, saveFile, showAbout,
  onAlertResult, onOpenFileResult, onSaveFileResult,
} from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";

function App() {
  const winRef = useRef<NdNodeRef<"window">>(null);
  const [result, setResult] = useState("(none yet)");

  async function handleDelete() {
    if (!winRef.current) return;
    const { buttonId } = await showAlert(winRef.current, {
      title: "Delete this item?",
      body: "This action cannot be undone.",
      buttons: [
        { id: "cancel", label: "Cancel" },
        { id: "delete", label: "Delete", style: "destructive" },
      ],
    });
    setResult(buttonId);
  }

  return (
    <window
      ref={winRef}
      title="My App"
      onAlertResult={(e) => onAlertResult(winRef.current!, e)}
      onOpenFileResult={(e) => onOpenFileResult(winRef.current!, e)}
      onSaveFileResult={(e) => onSaveFileResult(winRef.current!, e)}
    >
      <button label="Delete…" onClick={handleDelete} />
    </window>
  );
}
```

The `on*Result` props aren't optional boilerplate — skip one and its matching call's promise never
settles, because the result event is how the promise learns the dialog closed. `showAbout` has no
result event and needs no wiring; see below.

## API

| Function | Options | Resolves to |
| --- | --- | --- |
| `showAlert(node, options)` | `{ title, body?, buttons: { id, label, style? }[] }` | `{ buttonId }` — the clicked button's `id` |
| `openFile(node, options?)` | `{ multiple?, directories?, filters?: { name, extensions }[] }` | `{ canceled, paths }` — `paths: []` if canceled |
| `saveFile(node, options?)` | `{ suggestedName?, defaultDir?, filters? }` | `{ canceled, path }` — `path: null` if canceled |
| `showAbout(node, options)` | `{ appName, version, developer?, website? }` | — fire-and-forget, no promise |

`style` on an alert button is `"default" | "suggested" | "destructive"` — the destructive style is the
red/warning treatment (`NSAlertStyle.critical`-adjacent styling, `.destructive-action` on GTK).

## One dialog per window at a time

`showAlert`/`openFile`/`saveFile` each claim their window's single dialog slot for as long as they're
pending — neither backend has a way to stack two native sheets on one window. Calling a second one
before the first resolves **rejects immediately** with an error identifying which dialog is still
open, rather than queueing or silently clobbering the first caller's promise:

```
Error: <window> already has a "showAlert" dialog pending — only one modal dialog per window is allowed at a time
```

`showAbout` is the exception: it has no result event to correlate, so it doesn't claim the slot and
can be called freely alongside a pending `showAlert`/`openFile`/`saveFile`.

See `packages/react/src/dialogs.ts` for the full implementation and `examples/gallery/main.tsx`'s
"Dialogs" tab for all four calls wired to readouts.
