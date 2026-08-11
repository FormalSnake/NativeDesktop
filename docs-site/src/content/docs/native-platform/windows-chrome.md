---
title: Windows & Chrome
description: How <window>, <headerbar>, <toolbarview>, and <splitview> compose into real native chrome on each platform.
---

Native chrome (window titlebars, toolbars, and sidebar splits) is built from ordinary widget
intrinsics, declared as JSX children (see [App Model](/core-concepts/app-model/)). This page covers
the two-pane sidebar/content `<splitview>` shape. A third `list` pane and a `<menubar>` widget
compose into the same chrome; see [Menu Bar](/native-platform/menu-bar/) and
[Split Views](/native-platform/split-views/). Both `examples/notes/main.tsx` and
`examples/gallery/` exercise the two-pane shape end to end on both platforms.

## `<window>`

The JSX root of an app. `title`, `defaultWidth`, and `defaultHeight` are honored on both platforms;
`title` is live-updatable, the size props are create-only (they set the initial window size, not a
constraint). See the [Widget Reference](/components/widget-reference/) for the full
prop table. Multiple `<window>` roots open [multiple OS windows](/native-platform/multi-window/),
and windows sharing a `tabGroup` render as one tabbed window with
[native system tabs](/native-platform/tabs/).

Three create-only props shape the window chrome further:

- `toolbarStyle` (`unified` default, `unifiedCompact`, `expanded`, `preference`) picks the macOS
  `NSWindow.ToolbarStyle` once a `<headerbar>` attaches the unified toolbar — a settings window
  declares `preference` and gets labelled toolbar items with the window title visible; the
  `unified` styles keep the transparent-titlebar sidebar treatment. GTK's native chrome is the one
  `AdwHeaderBar` idiom, so the prop is deliberately inert there.
- `frameAutosaveName` persists the window's frame across launches under that key
  (`NSWindow.setFrameAutosaveName`), and doubles as the toolbar-customization autosave key: with it
  set, the unified toolbar allows user customization and saves the configuration. GTK4 dropped
  session geometry persistence; persist sizes with the settings store there.
- `density` (`standard` default, `compact`) opts the window's content into compact control metrics
  (`prefersCompactControlSizeMetrics` on macOS 26+, a tightened CSS block on GTK).

## `<splitview>`: sidebar + content

`<splitview sidebarWidth={0.28}>` takes two children, distinguished by the `slot` attached prop
(`sidebar` or `content`):

```tsx
<splitview sidebarWidth={0.28}>
  <toolbarview slot="sidebar">…</toolbarview>
  <toolbarview slot="content">…</toolbarview>
</splitview>
```

- On Linux, this is a real `AdwOverlaySplitView`; the window runs as an `AdwApplicationWindow`
  so the split fills edge-to-edge, GNOME-style.
- On macOS, this is a real `NSSplitViewController`-managed split (using the
  `sidebarWithViewController:` API), which is what gives the sidebar system vibrancy/Liquid Glass
  material rather than a manually composited effect view.

`collapsed` is live-updatable on both; `sidebarWidth` sets the initial split proportion at creation.

## `<toolbarview>` + `<headerbar>`: per-pane headers

Each pane is wrapped in a `<toolbarview>` (`AdwToolbarView` on Linux) whose first child is a
`<headerbar>`, so the sidebar and the content pane each carry their own header instead of one
shared window titlebar:

```tsx
<toolbarview slot="sidebar">
  <headerbar>
    <button iconName="document-new" onClick={createNote} slot="start" />
  </headerbar>
  {/* sidebar content */}
</toolbarview>
```

The two platforms render this identically-shaped tree differently, on purpose:

- On Linux, each `<toolbarview>` adds its `<headerbar>` as a real top bar
  (`AdwToolbarView.addTopBar`). You get two independent `AdwHeaderBar`s, one per pane, the
  native GNOME idiom for a sidebar app.
- On macOS, the two `<headerbar>`s don't each create their own bar. Their items merge into
  one unified `NSToolbar` spanning the window's top edge, split by an
  `NSTrackingSeparatorToolbarItem` aligned to the split's divider: the sidebar's items sit left of
  it, the content pane's items sit right of it. This is the native macOS idiom (Notes.app, Mail),
  achieved via `.fullSizeContentView` + `titlebarAppearsTransparent` so the sidebar's vibrancy
  reaches the very top, with the traffic-light window controls floating over it.

`<headerbar title="…">` sets the pane's (or, on macOS, the toolbar's) title; on children, the `slot`
attached prop (`start`/`end`) positions items on either side of the title.

On macOS, header `<button>` children are promoted to **system-drawn toolbar items** (image/action,
no embedded custom view): the Tahoe item glass is the only bezel, runs of adjacent buttons group
into one `NSToolbarItemGroup`, every item carries a real label for the overflow menu and VoiceOver
(derived from the button's label, tooltip, or icon name), and the `prominent`/`badge` Button props
render as `.prominent` tint and a native `NSItemBadge`. Non-interactive header children (labels,
spinners, images) opt out of the item treatment. A `<togglebutton>` keeps its own view so its
on/off state stays visible.

## Inspector pane and edge-to-edge content

`<splitview>` also accepts a `slot="inspector"` pane — the HIG utility pane alongside content
(edge-to-edge glass, unlike the sidebar's floating pane). On macOS it is a real
`NSSplitViewItem(inspectorWithViewController:)` pinned to the trailing edge with no tracking
separator on its divider; on GTK it renders as an end-positioned `AdwOverlaySplitView` sidebar.
`examples/inspector/` exercises it together with `prominent`/`badge` toolbar items.

Panes whose content root scrolls (a `<scrollview>`, `<textarea>`, or a box holding just one)
extend edge-to-edge under the floating glass chrome: the scroll view insets its content via the
safe area, so content passes under the toolbar and AppKit draws the scroll edge effect, with the
pane's background mirrored into the unsafe regions (`NSBackgroundExtensionView`). Non-scrolling
panes keep the safe-area inset so controls never start under the titlebar.

## Where this is headed

A three-pane `<splitview>` (`sidebar`/`list`/`content`) and a dedicated `<menubar>` widget have
landed; see [Split Views](/native-platform/split-views/) and [Menu Bar](/native-platform/menu-bar/).
A `<window>` that composes more than one independent split is on the roadmap. Check back here
once it lands rather than assuming prop names in advance.
