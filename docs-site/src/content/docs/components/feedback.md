---
title: Feedback
description: Banner, StatusPage, and ToastOverlay, three widgets for telling the user something happened.
---

Three widgets for telling the user something happened. `Banner` stays visible until dismissed,
`StatusPage` replaces the content entirely, and `ToastOverlay` floats briefly above everything.

## Banner (`<banner>`)

A dismissible in-flow strip (`AdwBanner` on GTK, an equivalent bar on macOS) for a persistent
notice such as "a new version is available", visible until the user acts or you hide it.

```tsx
const [revealed, setRevealed] = useState(true);

<banner
  title="A new version is available"
  buttonLabel="Update Now"
  revealed={revealed}
  onButtonClicked={() => setRevealed(false)}
/>;
```

| Prop | Type | Applied | Notes |
| --- | --- | --- | --- |
| `title` | string | createAndUpdate | |
| `buttonLabel` | string | createAndUpdate | Omit to render the banner with no action button. |
| `revealed` | bool | createAndUpdate | Controlled visibility, defaulting to `false`. |

`buttonClicked` → `onButtonClicked` fires with no payload. Set `revealed={false}` yourself if
clicking the button should dismiss the banner.

## StatusPage (`<statuspage>`)

A full-pane empty, error, or success state (`AdwStatusPage` on GTK) for a view with nothing to
show: an empty list, a failed load. Takes children, usually a `<button>` for the page's action.

```tsx
<statuspage iconName="folder" title="No files yet" description="Add your first file to get started.">
  <button label="Add File" onClick={addFile} />
</statuspage>;
```

| Prop | Type | Applied | Notes |
| --- | --- | --- | --- |
| `iconName` | string | createAndUpdate | |
| `title` | string | createAndUpdate | |
| `description` | string | createAndUpdate | |

No events of its own. Wire up whatever action widget you place inside it.

## ToastOverlay (`<toastoverlay>`) and the toast helpers

`<toastoverlay>` is a wrapping container (`childModel: single`). Mount it around your whole window
content rather than one tab or panel, so a toast floats above every screen the user might be on
when you queue it:

```tsx
import { showToast, onToastButtonClicked, onToastDismissed } from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";

const toastRef = useRef<NdNodeRef<"toastoverlay">>(null);

<toastoverlay
  ref={toastRef}
  onToastButtonClicked={onToastButtonClicked}
  onToastDismissed={onToastDismissed}
>
  {/* the rest of your app tree */}
</toastoverlay>;

// ...later, from an event handler:
async function handleDelete() {
  await deleteItem();
  const result = await showToast(toastRef.current!, {
    title: "Item deleted",
    buttonLabel: "Undo",
    timeoutSeconds: 6,
  });
  if (result.buttonClicked) await undoDelete();
}
```

`showToast` and `dismissToast` (from `@nativedesktop/react`, backed by `packages/react/src/toast.ts`)
are [imperative commands](/core-concepts/imperative-commands/) wrapped in a promise:

| Function | Signature | Resolves to |
| --- | --- | --- |
| `showToast(node, options)` | `options: { title, buttonLabel?, timeoutSeconds?, priority? }` | `{ buttonClicked: boolean }` |
| `dismissToast(node, id?)` | dismisses the toast matching `id`, or whichever is currently visible | none |

`priority` is `"normal"` by default, or `"high"` to jump the overlay's queue instead of waiting
behind an already-showing toast. `showToast`'s promise resolves once, however the toast goes away:
`{ buttonClicked: true }` when the user clicks the action button, `{ buttonClicked: false }` on a
timeout, Escape, or the queue advancing past it.

Pass `onToastButtonClicked` and `onToastDismissed` directly as the `<toastoverlay>`'s event props,
not wrapped in an inline arrow function. They read the `id` off the payload themselves and settle
whichever `showToast()` call is pending for it.

See `examples/gallery/main.tsx`'s Status & Banner and Toasts tabs, and the
[Widget Reference](/components/widget-reference/) for the generated prop tables.
