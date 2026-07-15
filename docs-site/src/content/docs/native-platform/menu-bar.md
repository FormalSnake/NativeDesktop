---
title: Menu Bar
description: One <menubar> declaration becomes a real NSApp.mainMenu on macOS and a primary GtkMenuButton on GNOME.
---

A `<menubar>` is the clearest demonstration of the design-language philosophy (see
[App Model](/core-concepts/app-model/)): the same declared tree adopts each platform's own idiom for
where the app's commands live, rather than drawing one widget identically everywhere. On macOS
that's a real menu bar; on GNOME it's a primary hamburger menu.

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

(Adapted from `examples/notes/menubar-probe.tsx`, the headless acceptance fixture for this
machinery.)

## MenuItem: role or onSelect, never both in effect

A `<menuitem>` has either a `role` (native, platform-provided behavior) or an `onSelect` handler
(custom). If both are set, `onSelect` wins and the role contributes nothing beyond documentation.
`role="separator"` renders a native separator — `label`/`iconName`/`accelerator` are ignored on a
separator item.

The full prop set (see the [Widget Reference](/components/widget-reference/) for the generated
table):

- **`label`** — the item's text.
- **`iconName`** — a freedesktop name, the same vocabulary as `Button.iconName` (see
  [Icons](/native-platform/icons/)). Resolved through the same mac SF Symbol map. Deliberately not
  rendered on GNOME: popover menus don't render item icons and the HIG discourages them, so
  omitting them is the platform-correct choice.
- **`accelerator`** — grammar `mod+…+key`: mods are `primary`, `shift`, `alt`, `ctrl` (`primary` is
  ⌘ on macOS, Ctrl on GNOME); the key is one printable character or a named key (`enter`,
  `escape`, `backspace`, `delete`, `space`, `tab`, `f1`…`f12`, `left`/`right`/`up`/`down`, `comma`,
  `period`).
- **`role`** — `none` (the default) plus a fixed vocabulary: `separator`, `about`, `settings`,
  `quit`, `undo`, `redo`, `cut`, `copy`, `paste`, `delete`, `selectAll`, `close`, `minimize`,
  `zoom`, `fullscreen`.
- **`enabled`** — createAndUpdate; a disabled item doesn't fire `onSelect`, on either platform.
- **`onSelect`** — fires the `selected` event.

## Platform rendering

### macOS: a real main menu, with or without a declared `<menubar>`

macOS installs the full standard main menu (App, File, Edit, View, Window, Help, each wired to the
responder chain via `NSText` selectors and `NSApplication` actions) at window creation, even when
the tree has no `<menubar>` at all. That's why `Edit > Copy`, `Cut`, `Paste`, `Select
All`, `Quit`, `Minimize`, and friends work in every text field with no app code: they're
responder-chain selectors with a nil target, not custom handlers.

A declared `<menu label>` is merged into that default chrome:

- A label that matches a default top-level title (`File`, `Edit`, `View`, `Window`, `Help`)
  gets its items appended to that menu after a separator.
- Any other label becomes a new top-level menu, inserted before Window (after View) in
  declaration order.

`<menubar defaults={false}>` opts out of the merge: only the App menu (About/Hide/Quit) plus your
declared menus are installed. The default File/Edit/View/Window/Help menus are not.

### GNOME: no top menubar, by design

GNOME Shell doesn't show a top-level menubar; that's correct GNOME design. Instead, `<menubar>`
renders as a primary menu button (`GtkMenuButton`, `open-menu-symbolic`,
tooltip "Main Menu") homed in the last headerbar in document order (by GNOME convention, the
content pane's header). If the tree has no headerbar at all, it falls back to
`gtk.Application.setMenubar`, an in-window menu strip.

Roles that would duplicate window or system chrome are dropped per the GNOME HIG: `quit`,
`close`, `minimize`, `undo`, `cut`, `copy`, `paste`, `delete`, `selectAll`, `zoom`, `fullscreen`. A
menu whose items all drop is omitted entirely. The two roles that do survive get GNOME-native
treatment instead of a plain menu item: `about` opens an `AdwAboutDialog`, and `settings` renders as
a "Preferences" item that emits `onSelect` like any custom item.

## Beyond the menu bar

`<menu>`/`<menuitem>` aren't exclusive to `<menubar>`. The same elements are also the dropdown
content for [`<menubutton>` and `<splitbutton>`](/components/menus-and-popovers/), and for a macOS
[`<trayitem>`](/native-platform/platform-support/#platform-only-widgets)'s right-click menu. The prop
set, `role`/`onSelect` exclusivity, and accelerator grammar documented above apply identically in all
three places; only the owning widget differs.

## Automation

Menu nodes appear in `getTree` (see [Automation Socket](/automation-testing/automation-socket/))
with labels as their text and nominal (non-visual) bounds, since they're chrome rather than an
on-screen widget. `semanticClick` on a `menuitem` ref dispatches `onSelect` (or fires the role's
native action) on both backends, so an automation-driving agent can exercise a File > New Note
command the same way it clicks a button.

## Caveats

- `iconName` on a `<menuitem>` has no visible effect on GNOME. This is intentional; don't rely on
  the icon for information the label doesn't already carry.
- macOS has one merge rule (append after a separator on a title match, otherwise insert
  before Window). There's no way to target Edit/View/Help insertion order beyond that.
- `role` and `onSelect` are not additive: setting both silently drops the role's native behavior in
  favor of the custom handler, so don't expect e.g. `role="quit"` plus an `onSelect` to run your
  handler *and* quit.
