---
title: Menu Bar
description: One <menubar> declaration becomes a real NSApp.mainMenu on macOS and a primary GtkMenuButton on GNOME.
---

One `<menubar>` declaration renders as each platform's own idiom for where an app's commands live:
a real `NSApp.mainMenu` on macOS, a primary hamburger menu on GNOME.

## Shape

`<menubar>` is a `<window>` child, a sibling of the window's content, not nested inside it. It
contains `<menu label>` entries, which in turn contain `<menuitem>` entries:

```tsx
<window title="ND Notes">
  <menubar>
    <menu label="File">
      <menuitem
        label="New Note"
        iconName="document-new"
        accelerator="primary+n"
        onSelect={createNote}
      />
    </menu>
    <menu label="Note">
      <menuitem label="Pin" accelerator="primary+p" onSelect={togglePin} />
      <menuitem role="separator" />
      <menuitem
        label="Delete"
        iconName="edit-delete"
        accelerator="primary+backspace"
        onSelect={deleteNote}
      />
    </menu>
  </menubar>
  <splitview>{/* … */}</splitview>
</window>
```

(Adapted from `examples/notes/menubar-probe.tsx`, the headless acceptance fixture.)

## MenuItem: role or onSelect, never both

A `<menuitem>` takes either a `role`, meaning platform-provided behavior, or an `onSelect` handler
for custom behavior. Set both and `onSelect` wins; the role contributes nothing.
`role="separator"` renders a native separator and ignores `label`, `iconName`, and `accelerator`.

The full prop set (the [Widget Reference](/components/widget-reference/) has the generated table):

| Prop | Meaning |
|---|---|
| `label` | The item's text. |
| `iconName` | A freedesktop name, the same vocabulary as `Button.iconName` (see [Icons](/native-platform/icons/)), resolved through the same SF Symbol map on macOS. Deliberately not rendered on GNOME, where popover menus have no item icons and the HIG discourages them. |
| `accelerator` | Grammar `mod+…+key`. Mods are `primary`, `shift`, `alt`, `ctrl`, where `primary` is ⌘ on macOS and Ctrl on GNOME. The key is one printable character or a named key: `enter`, `escape`, `backspace`, `delete`, `space`, `tab`, `f1` through `f12`, `left`, `right`, `up`, `down`, `comma`, `period`. |
| `role` | `none` by default, plus a fixed vocabulary: `separator`, `about`, `settings`, `quit`, `undo`, `redo`, `cut`, `copy`, `paste`, `delete`, `selectAll`, `close`, `minimize`, `zoom`, `fullscreen`. |
| `enabled` | createAndUpdate. A disabled item does not fire `onSelect` on either platform. |
| `onSelect` | Fires the `selected` event. |

## Platform rendering

### macOS: a real main menu, with or without a declared `<menubar>`

macOS installs the full standard main menu at window creation even when the tree has no `<menubar>`:
App, File, Edit, View, Window, Help, each wired to the responder chain via `NSText` selectors and
`NSApplication` actions. That is why Copy, Cut, Paste, Select All, Quit, and Minimize work in every
text field with no app code. They are responder-chain selectors with a nil target, not custom
handlers.

A declared `<menu label>` is merged into that default chrome:

- A label that matches a default top-level title (`File`, `Edit`, `View`, `Window`, `Help`)
  gets its items appended to that menu after a separator.
- Any other label becomes a new top-level menu, inserted before Window (after View) in
  declaration order.

`<menubar defaults={false}>` opts out of the merge: only the App menu (About/Hide/Quit) plus your
declared menus are installed. The default File/Edit/View/Window/Help menus are not.

### GNOME: no top menubar

GNOME Shell shows no top-level menubar, which is correct GNOME design. `<menubar>` renders instead
as a primary menu button (`GtkMenuButton`, `open-menu-symbolic`, tooltip "Main Menu") homed in the
last headerbar in document order, by GNOME convention the content pane's header. With no headerbar
in the tree at all, it falls back to `gtk.Application.setMenubar`, an in-window menu strip.

Roles that would duplicate window or system chrome are dropped per the GNOME HIG: `quit`, `close`,
`minimize`, `undo`, `cut`, `copy`, `paste`, `delete`, `selectAll`, `zoom`, `fullscreen`. A menu whose
items all drop is omitted entirely. The two surviving roles get GNOME-native treatment rather than a
plain menu item: `about` opens an `AdwAboutDialog`, and `settings` renders as a Preferences item that
emits `onSelect` like any custom item.

## Beyond the menu bar

`<menu>` and `<menuitem>` are not exclusive to `<menubar>`. The same elements are the dropdown
content for [`<menubutton>` and `<splitbutton>`](/components/menus-and-popovers/), and for a macOS
[`<trayitem>`](/native-platform/platform-support/#platform-only-widgets)'s right-click menu. The prop
set, `role`/`onSelect` exclusivity, and accelerator grammar above apply identically in all three
places.

## Automation

Menu nodes appear in `getTree` (see [Automation Socket](/automation-testing/automation-socket/))
with labels as their text and nominal, non-visual bounds, since they are chrome rather than
on-screen widgets. The `click` RPC on a `menuitem` ref dispatches `onSelect` (or fires the role's
native action) on both backends, so an agent exercises File > New Note the same way it clicks a
button.

## Caveats

- `iconName` on a `<menuitem>` has no visible effect on GNOME. Do not rely on the icon for
  information the label does not already carry.
- macOS has one merge rule: append after a separator on a title match, otherwise insert before
  Window. There is no way to target Edit/View/Help insertion order beyond that.
- `role` and `onSelect` are not additive. Setting both drops the role's native behavior, so
  `role="quit"` plus an `onSelect` runs your handler and does not quit.
