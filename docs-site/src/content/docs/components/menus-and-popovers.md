---
title: Menus & Popovers
description: MenuButton, SplitButton, Popover, and Expander, the four widgets for content or actions that reveal on demand.
---

Four widgets share one idea: content or actions that stay hidden until the user asks for them,
rather than occupying permanent space. Two (`MenuButton`, `SplitButton`) reuse the same `<menu>`/
`<menuitem>` vocabulary as [the menu bar](/native-platform/menu-bar/); the other two (`Popover`,
`Expander`) host an arbitrary child tree instead of a menu.

## MenuButton (`<menubutton>`) and SplitButton (`<splitbutton>`)

`<menubutton>` is a single button that opens a dropdown menu. `<splitbutton>` fuses two actions
into one control: a primary click action plus a chevron that opens a dropdown for secondary
actions. It's `AdwSplitButton` on GTK and an `NSButton` + attached `NSMenu` on macOS.

Both take `<menuitem>` children, plus `<menu>` for a nested submenu, the same elements the
[`<menubar>`](/native-platform/menu-bar/) page documents, built into an `NSMenu`/`GMenuModel` instead
of the app's main menu:

```tsx
<menubutton label="Actions" iconName="open-menu">
  <menuitem label="Duplicate" onSelect={duplicate} />
  <menuitem label="Rename" onSelect={rename} />
  <menuitem role="separator" />
  <menuitem label="Delete" iconName="edit-delete" onSelect={remove} />
</menubutton>

<splitbutton label="Save" iconName="document-save" onClick={save}>
  <menuitem label="Save As…" onSelect={saveAs} />
  <menuitem label="Save a Copy" onSelect={saveCopy} />
</splitbutton>
```

| Widget | Props | Events |
| --- | --- | --- |
| `<menubutton>` | `label`, `iconName` (both createAndUpdate) | none; each `<menuitem>` fires its own `onSelect` |
| `<splitbutton>` | `label`, `iconName` (both createAndUpdate) | `clicked` → `onClick` (the primary action; the dropdown's items fire their own `onSelect`) |

## Popover (`<popover>`)

An anchored transient surface (`GtkPopover` / `NSPopover`) for a small piece of arbitrary content
attached to a trigger. Unlike `MenuButton`, its child is a full widget tree rather than
`<menuitem>`s, so it can hold anything a `<box>` could.

```tsx
const [open, setOpen] = useState(false);

<box orientation="horizontal" spacing={8}>
  <button label="Open Popover" onClick={() => setOpen(true)} />
  <popover open={open} position="bottom" onClosed={() => setOpen(false)}>
    <box orientation="vertical" spacing={8} style={{ padding: 12 }}>
      <label text="Popover content" />
      <button label="Close" onClick={() => setOpen(false)} />
    </box>
  </popover>
</box>;
```

| Prop | Type | Applied | Notes |
| --- | --- | --- | --- |
| `open` | bool | createAndUpdate | Controlled. Set it from `onClosed` when the user dismisses the popover by clicking outside or pressing Escape. |
| `position` | `top` \| `bottom` \| `left` \| `right` | createAndUpdate | Default `top`. |

`closed` → `onClosed` fires with no payload. A `<popover>` attaches to whatever widget is its own
tree parent on GTK (`gtk_widget_set_parent`), so put it in a `<box>` alongside the button that opens
it, as above, rather than off on its own.

## Expander (`<expander>`)

An inline disclosure widget (`AdwExpanderRow`-style on GTK, an `NSButton` disclosure triangle +
container on macOS) for content that should stay in the layout flow, unlike `Popover`, which floats
above it.

```tsx
const [open, setOpen] = useState(false);

<expander label="More options" expanded={open} onToggled={(e) => setOpen(e.checked)}>
  <box orientation="vertical" spacing={6} style={{ padding: 8 }}>
    <checkbox label="An option inside the expander" checked={/* ... */} onToggled={/* ... */} />
  </box>
</expander>;
```

| Prop | Type | Applied | Notes |
| --- | --- | --- | --- |
| `label` | string | createAndUpdate | |
| `expanded` | bool | createAndUpdate | Controlled. Set it from `onToggled`. |

`toggled` → `onToggled` fires `{ checked }`.

See `examples/gallery/main.tsx`'s "Popovers & Menus" tab for all four wired together, and the
[Widget Reference](/components/widget-reference/) for the generated prop tables.
