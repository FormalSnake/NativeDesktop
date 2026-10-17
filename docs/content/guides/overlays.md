+++
title = "Overlays"
description = "Popovers, panels, tooltips, context menus, the command palette, and toasts: how they open and close, and why a click inside one never reaches the page or button under it."
+++

An overlay is an element in your tree that floats above its anchor. To show it, render it. To
hide it, stop rendering it. While it's open, the framework guarantees what happens to input.

<!-- docs:example overlays -->

## The three kinds

`popover(id)`, `panel(id)`, and `tooltip()` come from the prelude. Each one is a container: add
children as content, and size it with the usual style methods. An overlay takes no room in its
parent's layout. Its parent is its anchor, so it closes with the anchor's subtree.

| Element | Placed | Outside presses | Use |
| --- | --- | --- | --- |
| `popover(id)` | Below its anchor (`.placement(..)` moves it) | Close it, and are consumed | Menus of controls, pickers |
| `panel(id)` | Over the window, near the top | Consumed, and the panel stays | Sheet-like forms, quick pickers |
| `tooltip()` | Above its anchor | None, because it takes no input | Hints |

Those are the default modalities. To change one, call `.modality(..)` with `Modal`,
`LightDismiss`, or `Passive`. `Placement` is `Below`, `Above`, `Leading`, or `Trailing`. Panels
ignore it.

A popover hangs off the element that it's a child of:

<!-- docs:snippet examples/overlays/src/main.rs#popover -->

A panel uses the same call. Because a panel is modal, the rest of the window stays out of reach
until the app drops it:

<!-- docs:snippet examples/overlays/src/main.rs#panel -->

Most controls have a shortcut for tooltips, such as `.tooltip("Command palette")` on a button or
an icon button. Use the `tooltip()` element when the hint needs content of its own.

## Closing

You decide when an overlay closes. An overlay never removes itself. It reports
`Dismissed { reason }` to `on_dismiss`, and you stop rendering it. The reason is `Escape`,
`OutsidePress`, or `Deactivated` (the app lost activation). In the example, both handlers clear a
flag and call `cx.notify()`.

An overlay also closes when its anchor leaves the tree. It doesn't close when focus moves between
your own windows.

Every overlay that isn't passive is a focus scope. Opening it saves focus, and closing it hands
focus back. A modal overlay traps Tab. An Escape press that nothing inside handles closes the
innermost overlay.

## Context menus

`.context_menu(build)` on any element adds a native menu that opens on right-click, Control-click,
or the menu key. The closure runs when the menu opens, so the items can depend on the current
state. Each item dispatches an action, which reaches the focused view the same way a key binding
does.

<!-- docs:snippet examples/overlays/src/main.rs#menu -->

`item_destructive` marks an item as destructive, and `submenu` nests a menu. On macOS, the menu is
an `NSMenu`. On other platforms, it's a drawn menu in the overlay layer. Notes attaches one to each
row of its list in [note_list.rs](../../../examples/notes/src/note_list.rs).

## Input never leaks

Input inside an overlay (a press, a scroll, or a hover) never reaches what's under it. That
includes native views, such as a `WKWebView`, a CEF view, or a GPU surface. The example puts a
native button and a live web page under every overlay and counts the input that each one
receives. The gate fails if a count changes.

How the framework enforces this depends on the OS:

- macOS: Each overlay is its own window above the owner, so the window server routes input, and
  nothing relies on hit-testing inside one window. This matters for hover. Tracking areas fire
  for covered views, so an overlay drawn inside the window would still make a web view's
  `:hover` flicker. Popovers are `NSPopover`. Panels and tooltips are borderless child panels
  that never become key, so the owner keeps its key chrome and keys keep reaching nd's dispatch.
  While any overlay is open, one event monitor applies the outside-press and Escape rules.
- Linux, Windows, and web: Overlays draw in GPUI's deferred layer, above their parents, with a
  transparent backdrop that consumes presses. A light-dismiss backdrop lets scrolls through.
  Gesture hit-testing starts inside the topmost overlay or hits nothing.

On every OS, a press outside a light-dismiss overlay closes it and is consumed, release included.
The element under the pointer sees nothing. macOS menus already behave this way.

Declared accelerators run through the main menu before the window's views, so an embedded page
can't swallow a shortcut while an overlay is up.

Native surfaces that the OS composites above GPUI (WebView2, WPE) aren't covered yet. They need
either the overlay composition surface under `nd-gpui/fast` or a child popup window, which lands
with those engines. For details, see [webview](webview.md). The leak gate runs on macOS and on
Hyprland.

## Command palette

`command_palette(id)` is a glass card near the top of the window with a text field, ranked
rows, and keyboard navigation. It's a controlled element: you pass the open flag, the query, and
the rows, and you react to the events. Notes puts it in the scaffold's overlay slot:

<!-- docs:snippet examples/notes/src/app.rs#palette -->

Rows are `palette_item(id, title)` with `.subtitle`, `.icon`, `.hint` (a shortcut label), and
`.completion` (the text that the field completes inline when the query is a prefix). Notes ranks
notes first and commands second, so a title that starts with the query offers itself. The palette
shows exactly the rows you give it, in order. You do the filtering, and
[`fuzzy.rs`](../../../crates/nd-components/src/fuzzy.rs) has the ranker that notes uses.
When nothing matches, `on_submit` fires with the typed text.

## Toasts

`toast_overlay(id)` shows one window's toasts at the bottom, one at a time. You own the list:
append a `toast(id, title)` to show one, and remove it when `on_dismiss` fires.

<!-- docs:snippet examples/gallery/src/lib.rs#toasts -->

A toast goes away after 5 seconds. `.persistent()` keeps it until it's dismissed, `.timeout(..)`
changes the time, and `.action("Undo")` adds one button. A click on that button arrives at
`on_action` before the dismissal. A `.high_priority()` toast interrupts the one on screen, which
is dismissed with the reason `Interrupted`. Normal toasts queue in order.

The gallery has a page for the palette and a page for toasts.
