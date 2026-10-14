+++
title = "Windows and tabs"
description = "Opening windows with WindowOptions, several windows and monitors, window controls and drag regions, window tabs with tab_group, and the in-window tab, split and pane widgets."
+++

A window is a root view you hand to `cx.open_window`. This page covers what you can ask of the
window itself, then tabs, which come in two kinds: window tabs, where each tab is a window, and
in-window tabs, where the tabs are one view.

<!-- docs:example tabs -->

## Opening a window

<!-- docs:snippet examples/hello/src/lib.rs#open -->

`WindowOptions::main(id)` takes an id and these methods:

| Method | Effect |
| --- | --- |
| `.title(..)` | the title; on a native tab it is the tab's label, and `cx.set_window_title` changes it later |
| `.size(..)`, `.min_size(..)` | initial and minimum content size |
| `.titlebar(Titlebar::Custom)` | content runs under a transparent title bar and you draw the row |
| `.kind(..)` | `Normal`, `Sheet { parent }` or `Settings` |
| `.tab_group(..)` | open as a tab of a group, see below |

Sheets and the settings window have helpers so you do not build the options by hand.
`window.open_sheet(cx, build)` opens a window-modal sheet on macOS and a dialog with a scrim
elsewhere:

<!-- docs:snippet examples/notes/src/app.rs#sheet -->

`open_settings(cx, build)` opens the settings window, or brings the open one forward and
returns `None`. On macOS it is a preference-style window and the app menu's Settings item
(Cmd-,) opens it when the app binds that key.

<!-- docs:snippet examples/chrome/src/lib.rs#settings-window -->

## Several windows

Open as many as you like; each `open_window` call is an independent OS window with its own root
view. Windows share the app's entities, so state shared between them is an `Entity` you clone
into each root. The pacing example opens two:

<!-- docs:snippet examples/pacing/src/main.rs -->

Close with `cx.close_window(id)`. A user close goes to `Window::on_should_close` first when the
window has one, see [held closes](#held-closes).

## Monitors and frame pacing

Each window runs its own display clock at the refresh rate of the display it is on. Drag one
window to a 240 Hz monitor and the other stays at 144 Hz. `window.frame_time()` gives the
clock's `FrameTime`: the frame count, when the last frame was presented and when the one being
prepared will be. Sample animations at `window.now()`, which is the target time, and they stay
smooth at any rate.

An idle window requests no frames. To animate, call `cx.notify()` during render so the next
frame happens, as the pacing example does:

<!-- docs:snippet examples/pacing/src/lib.rs#clock -->

Only occluded or minimised windows are throttled. A visible window that is not focused, such as
a lyrics window on the second monitor, keeps its display's rate while it animates.

`ND_PACING=1` makes the GPUI host print `nd-pacing:` lines with the measured rate of each
window. The macOS host uses a display link per window. The Windows per-monitor path has not
run on a machine with two refresh rates yet, see [STATUS.md](../../plan/STATUS.md).

## Window controls and drag regions

The close, minimise and zoom controls are the OS's. You choose where they sit.

<!-- docs:snippet examples/hello/src/lib.rs#controls -->

- `window_controls().at(point(x, y))` pins the traffic lights at window coordinates.
- `window_controls().embedded()` takes a slot in layout and the backend moves the real buttons
  onto it every layout pass, keeping hover, menu and accessibility. Notes puts one at the
  leading edge of its toolbar.
- With no arguments the controls stay where the OS puts them.

On Linux the controls are drawn only when the compositor leaves decorations to the client and
lists the action. Under Hyprland nothing is drawn and nothing takes space.

`drag_region()` is a `div` whose background moves the window: pointer-down starts the OS move
and a double click zooms on macOS and maximises elsewhere. Buttons and other interactive
children keep their clicks.

### Titlebar insets

With `Titlebar::Custom` you draw the title row, and `titlebar()` reserves the space the OS
controls need: leading on macOS, trailing on Windows, whichever side the client-decorated Linux
window uses, none under Hyprland. The row drags the window where it is empty.

<!-- docs:snippet examples/overrides/src/lib.rs#titlebar -->

To lay out your own row, `window.titlebar_insets()` returns the `leading`, `trailing` and
`height` the backend measured, and `.titlebar_insets()` on a `div` pads it by them. Do not tune
a margin by hand; the values change with the OS release.

## Window tabs

Windows that name the same group show as tabs of one window: `NSWindow` tabbing on macOS, a
drawn tab bar in one window elsewhere. The group is read when the window opens, and groups with
different names never merge.

<!-- docs:snippet examples/tabs/src/lib.rs#open-tab -->

The tabs example is a browser. Each tab is a `TabWindow` holding a `Page`, and the app keeps a
`Browser` entity with the list of tabs. What the handlers do:

- `on_new_tab` runs when the user presses the tab bar's `+`, and you open another window in the
  same group.
- `on_should_close` decides what a user close does (the close button, Cmd-W, a drawn tab's
  close). Cmd-T and Cmd-W reach the app as the actions you bind and put in the menus.
- `cx.window_command(id, WindowCommand::ToggleTabOverview)` shows or hides the group's overview,
  which is macOS's Show All Tabs:

<!-- docs:snippet examples/tabs/src/lib.rs#overview -->

### Held closes

A tab's close is held until the app agrees. The handler returns `false` to keep the platform
from closing the window, drops the tab's page, then closes the window itself.

<!-- docs:snippet examples/tabs/src/lib.rs#close-tab -->

This order matters for a tab with a native surface in it. If the platform closed the window
first, a web view would be torn down before the app saw the close. Without a handler a close
request closes the window straight away.

### Moving tabs

Drag a tab out of the bar to make a new window, or into another window of the same group to
move it. A view moved to another window keeps its native nodes, so a `webview` does not reload.
Tabs from different groups refuse each other.

- macOS: the system's tab bar, drag-out and drag-in, `+` and Show All Tabs.
- GPUI backends: one OS window per group with a drawn libadwaita tab bar and an overview.
  Dragging out opens a new window on release. Hyprland cannot place that window at the release
  point, and tear-off is untested on a real Hyprland session. The overview cards have no
  thumbnails.
- Web: not available, the page is one canvas.

## Tabs inside a window

`tab_view(id)` is a tab strip over a stack of pages. `tab_strip(id)` is the strip alone, for when
you switch the content yourself. Selection is controlled: the strip reports `TabChanged` and you
pass the id back with `.selected(..)`.

<!-- docs:snippet examples/gallery/src/layout.rs#tab-view -->

- The pages share one grid cell and unselected ones stay mounted and hidden, so the view measures
  as its tallest page, nothing below it moves when the selection changes, and page state
  survives. `.homogeneous(false)` mounts only the selected page and takes its size.
- `TabStyle::Automatic` is a segmented control on macOS and a view switcher on GPUI backends.
  `TabStyle::Strip` draws closable document tabs with an optional `+` (`.add_button(true)`),
  and reports `on_close` and `on_add`. macOS has no in-window document tab bar, so there it is a
  segmented control, and closing a tab goes through your own command. For document tabs on
  macOS use window tabs.
- `tab_page(id, title)` takes `.icon(..)` and `.closable()`.

Settings windows that put their tabs in the toolbar do it with `toolbar().tab(..)`, shown in
[chrome](../../../examples/chrome/src/lib.rs).

## Split views

`split(id, Axis)` has two panes and a divider the user drags. It is an `NSSplitView` on macOS
and a drawn divider on GPUI backends.

```rust
split("body", Axis::Horizontal).ratio(0.3).first(sidebar).second(content).on_resize(listener)
```

The ratio is controlled. A drag reports `SplitResized { ratio }`; feeding the same ratio back
sets nothing. `split_pane()` wraps a pane to give it `min_size` and `max_size`, and a drag stops
there. On macOS split panes lay out again on the next run loop turn.

## Pane trees

For splits the user creates, `panes` is a pure model plus the elements that draw it. The model
is a binary tree of splits and leaves with one focused leaf, and every operation returns a value
equal to its input, sharing its `Rc`, when nothing changed. A native echo of a ratio the model
already holds therefore cannot start a render and save loop. Compare with `PaneTree::same`.

<!-- docs:snippet examples/gallery/src/layout.rs#pane-model -->

`seed_panes`, `split_pane`, `close_pane`, `focus_pane`, `focus_neighbor` and `set_pane_ratio`
cover the editing, and `serialize_panes` with `migrate_panes` persist a layout. `pane_tree`
draws a tree; your closure draws each leaf and is told whether it is the focused one.

<!-- docs:snippet examples/gallery/src/layout.rs#pane-tree -->

Two more views use the same discipline. `dock_view` renders tab stacks as leaves, with ops to
add, close, move and undock tabs and dock one panel against another, and keeps every tab
mounted. `tiles_view` places freeform tiles on a column grid and pushes others down on
collision. The gallery's Split view page runs the pane tree.
