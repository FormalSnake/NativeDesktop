+++
title = "Windows and tabs"
description = "Open windows with WindowOptions, use several windows and monitors, place window controls and drag regions, group window tabs with tab_group, and use the in-window tab, split, and pane widgets."
+++

A window is a root view that you pass to `cx.open_window`. This page covers what you can ask of
the window itself, and then tabs. Tabs come in two kinds: window tabs, where each tab is a
window, and in-window tabs, where the tabs are one view.

<!-- docs:example tabs -->

## Opening a window

<!-- docs:snippet examples/hello/src/lib.rs#open -->

`WindowOptions::main(id)` takes an id and these methods:

| Method | Effect |
| --- | --- |
| `.title(..)` | The title. On a native tab, it's the tab's label. To change it later, call `cx.set_window_title`. |
| `.size(..)`, `.min_size(..)` | The initial and minimum content size |
| `.titlebar(Titlebar::Custom)` | Content runs under a transparent title bar, and you draw the row |
| `.kind(..)` | `Normal`, `Sheet { parent }`, or `Settings` |
| `.tab_group(..)` | Opens the window as a tab of a group (see [window tabs](#window-tabs)) |

Sheets and the settings window have helpers, so you don't build the options by hand.
`window.open_sheet(cx, build)` opens a window-modal sheet on macOS and a dialog with a scrim on
other platforms:

<!-- docs:snippet examples/notes/src/app.rs#sheet -->

`open_settings(cx, build)` opens the settings window. If it's already open, the call brings it
forward and returns `None`. On macOS, it's a preference-style window, and the app menu's
**Settings** item (Cmd-,) opens it when the app binds that key.

<!-- docs:snippet examples/chrome/src/lib.rs#settings-window -->

## Several windows

You can open as many windows as you need. Each `open_window` call creates an independent OS
window with its own root view. Windows share the app's entities, so to share state between them,
clone an `Entity` into each root. The pacing example opens two:

<!-- docs:snippet examples/pacing/src/main.rs -->

To close a window, call `cx.close_window(id)`. When a window has a `Window::on_should_close`
handler, a close by the user goes to it first. For details, see [held closes](#held-closes).

## Monitors and frame pacing

Each window runs its own display clock at the refresh rate of the display that it's on. If you
drag one window to a 240 Hz monitor, the other one stays at 144 Hz. `window.frame_time()` gives
the clock's `FrameTime`: the frame count, when the last frame was presented, and when the frame
being prepared will be presented. Sample animations at `window.now()`, which is the target time,
and they stay smooth at any rate.

An idle window requests no frames. To animate, call `cx.notify()` during render so that the next
frame happens, as the pacing example does:

<!-- docs:snippet examples/pacing/src/lib.rs#clock -->

The framework throttles only occluded or minimized windows. A visible window that isn't focused,
such as a lyrics window on the second monitor, keeps its display's rate while it animates.

`ND_PACING=1` makes the GPUI host print `nd-pacing:` lines with the measured rate of each
window. The macOS host uses a display link per window. The Windows host paces each monitor on its
own vblank, checked at 240 Hz and 144 Hz. For details, see [STATUS.md](../../plan/STATUS.md).

## Window controls and drag regions

The close, minimize, and zoom controls belong to the OS. You choose where they sit.

<!-- docs:snippet examples/hello/src/lib.rs#controls -->

- `window_controls().at(point(x, y))` pins the traffic lights at window coordinates.
- `window_controls().embedded()` takes a slot in the layout, and the backend moves the real
  buttons onto it on every layout pass. The buttons keep their hover, menu, and accessibility
  behavior. Notes puts one at the leading edge of its toolbar.
- With no arguments, the controls stay where the OS puts them.

On Linux, the framework draws the controls only when the compositor leaves decorations to the
client and lists the action. Under Hyprland, nothing is drawn and nothing takes space.

`drag_region()` is a `div` whose background moves the window. Pointer-down starts the OS move. A
double click zooms the window on macOS and maximizes it on other platforms. Buttons and other
interactive children keep their clicks.

### Titlebar insets

With `Titlebar::Custom`, you draw the title row, and `titlebar()` reserves the space that the OS
controls need. That space is on the leading side on macOS, the trailing side on Windows, and
whichever side the client-decorated Linux window uses. Under Hyprland, there's none. Where the
row is empty, it drags the window.

<!-- docs:snippet examples/overrides/src/lib.rs#titlebar -->

To lay out your own row, call `window.titlebar_insets()`. It returns the `leading`, `trailing`,
and `height` values that the backend measured, and `.titlebar_insets()` on a `div` pads it by
them. Don't tune a margin by hand, because the values change with each OS release.

## Window tabs

Windows that name the same group show as tabs of one window. On macOS, this uses `NSWindow`
tabbing. On other platforms, it's a drawn tab bar in one window. The framework reads the group
when the window opens, and groups with different names never merge.

<!-- docs:snippet examples/tabs/src/lib.rs#open-tab -->

The tabs example is a browser. Each tab is a `TabWindow` that holds a `Page`, and the app keeps a
`Browser` entity with the list of tabs. Here's what the handlers do:

- `on_new_tab` runs when the user selects the tab bar's `+`. In it, you open another window in
  the same group.
- `on_should_close` decides what a close by the user does (the close button, Cmd-W, or a drawn
  tab's close button). Cmd-T and Cmd-W reach the app as the actions that you bind and put in the
  menus.
- `cx.window_command(id, WindowCommand::ToggleTabOverview)` shows or hides the group's overview,
  which is the macOS **Show All Tabs** view:

<!-- docs:snippet examples/tabs/src/lib.rs#overview -->

### Held closes

The platform holds a tab's close until the app agrees. The handler returns `false` to keep the
platform from closing the window, drops the tab's page, and then closes the window itself.

<!-- docs:snippet examples/tabs/src/lib.rs#close-tab -->

This order matters for a tab that has a native surface in it. If the platform closed the window
first, it would tear down a web view before the app saw the close. Without a handler, a close
request closes the window immediately.

### Moving tabs

Drag a tab out of the bar to make a new window, or into another window of the same group to
move it there. A view that moves to another window keeps its native nodes, so a `webview` doesn't
reload. Tabs from different groups refuse each other.

- macOS: The system's tab bar, with drag-out, drag-in, `+`, and **Show All Tabs**.
- GPUI backends: One OS window per group, with a drawn libadwaita tab bar and an overview.
  Dragging a tab out opens a new window on release. Hyprland can't place that window at the
  release point, and tear-off hasn't been tested on a real Hyprland session. The overview cards
  have no thumbnails.
- Web: Not available, because the page is one canvas.

## Tabs inside a window

`tab_view(id)` is a tab strip over a stack of pages. `tab_strip(id)` is the strip alone, for when
you switch the content yourself. Selection is controlled: the strip reports `TabChanged`, and you
pass the id back with `.selected(..)`.

<!-- docs:snippet examples/gallery/src/layout.rs#tab-view -->

- The pages share one grid cell, and unselected pages stay mounted and hidden. So the view
  measures as its tallest page, nothing below it moves when the selection changes, and page
  state survives. `.homogeneous(false)` mounts only the selected page and takes its size.
- `TabStyle::Automatic` is a segmented control on macOS and a view switcher on GPUI backends.
  `TabStyle::Strip` draws closable document tabs with an optional `+` (`.add_button(true)`) and
  reports `on_close` and `on_add`. macOS has no in-window document tab bar, so on macOS it's a
  segmented control, and closing a tab goes through your own command. For document tabs on
  macOS, use window tabs.
- `tab_page(id, title)` takes `.icon(..)` and `.closable()`.

Settings windows that put their tabs in the toolbar use `toolbar().tab(..)`, as shown in
[chrome](../../../examples/chrome/src/lib.rs).

## Split views

`split(id, Axis)` has two panes and a divider that the user drags. It's an `NSSplitView` on macOS
and a drawn divider on GPUI backends.

```rust
split("body", Axis::Horizontal).ratio(0.3).first(sidebar).second(content).on_resize(listener)
```

The ratio is controlled. A drag reports `SplitResized { ratio }`, and passing the same ratio back
changes nothing. `split_pane()` wraps a pane to give it a `min_size` and `max_size`, and a drag
stops at those limits. On macOS, split panes lay out again on the next turn of the run loop.

## Pane trees

For splits that the user creates, `panes` is a pure model plus the elements that draw it. The
model is a binary tree of splits and leaves, with one focused leaf. When nothing changes, every
operation returns a value equal to its input that shares its `Rc`. So a native echo of a ratio
that the model already holds can't start a render-and-save loop. To compare two trees, use
`PaneTree::same`.

<!-- docs:snippet examples/gallery/src/layout.rs#pane-model -->

`seed_panes`, `split_pane`, `close_pane`, `focus_pane`, `focus_neighbor`, and `set_pane_ratio`
cover editing, and `serialize_panes` and `migrate_panes` persist a layout. `pane_tree` draws a
tree. Your closure draws each leaf and receives whether that leaf is the focused one.

<!-- docs:snippet examples/gallery/src/layout.rs#pane-tree -->

Two more views follow the same approach. `dock_view` renders tab stacks as leaves, keeps every
tab mounted, and has operations to add, close, move, and undock tabs and to dock one panel
against another. `tiles_view` places freeform tiles on a column grid and pushes other tiles down
when they collide. The gallery's **Split view** page runs the pane tree.
