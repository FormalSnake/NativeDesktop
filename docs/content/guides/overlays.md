+++
title = "Overlays"
description = "Popovers, panels, tooltips, context menus, the command palette and toasts: how they open, how they close, and why a click inside one never reaches the page or button under it."
+++

An overlay is an element in your tree that floats above its anchor. You show it by rendering it
and hide it by not rendering it. The framework guarantees what happens to input while it is open.

<!-- docs:example overlays -->

## The three kinds

`popover(id)`, `panel(id)` and `tooltip()` come from the prelude. Each is a container: add
children as content and size it with the usual style methods. An overlay takes no room in its
parent's layout, and its parent is its anchor, so it closes with the anchor's subtree.

| Element | Placed | Outside presses | Use |
| --- | --- | --- | --- |
| `popover(id)` | below its anchor (`.placement(..)` moves it) | close it, and are consumed | menus of controls, pickers |
| `panel(id)` | over the window, near the top | consumed, panel stays | sheet-like forms, quick pickers |
| `tooltip()` | above its anchor | none: it takes no input | hints |

Those are the default modalities. `.modality(..)` changes one: `Modal`, `LightDismiss` or
`Passive`. `Placement` is `Below`, `Above`, `Leading` or `Trailing`; panels ignore it.

A popover hangs off the element it is a child of:

<!-- docs:snippet examples/overlays/src/main.rs#popover -->

A panel is the same call. Because it is modal, the rest of the window stays out of reach until
the app drops it:

<!-- docs:snippet examples/overlays/src/main.rs#panel -->

Most controls have a shortcut for tooltips, `.tooltip("Command palette")` on a button or icon
button. Use the `tooltip()` element when the hint needs content of its own.

## Closing

Dismissal is yours. An overlay never removes itself; it reports `Dismissed { reason }` to
`on_dismiss` and you stop rendering it. The reason is `Escape`, `OutsidePress` or
`Deactivated`, the app losing activation. In the example both handlers clear a flag and call
`cx.notify()`.

An overlay also closes when its anchor leaves the tree. It does not close when focus moves
between your own windows.

Every non-passive overlay is a focus scope. Opening saves focus, closing hands it back, a modal
one traps Tab, and an Escape that nothing inside took closes the innermost overlay.

## Context menus

`.context_menu(build)` on any element adds a native menu on right click, control-click or the
menu key. The closure runs when the menu opens, so the items can depend on current state. Each
item dispatches an action, which reaches the focused view like a key binding would.

<!-- docs:snippet examples/overlays/src/main.rs#menu -->

`item_destructive` marks an item as destructive and `submenu` nests a menu. The menu is an
`NSMenu` on macOS and a drawn menu in the overlay layer elsewhere. Notes attaches one to each
row of its list in [note_list.rs](../../../examples/notes/src/note_list.rs).

## Input never leaks

Input inside an overlay, a press, a scroll or hover, never reaches what lies under it. That
includes native views: a `WKWebView`, a CEF view, a GPU surface. The example puts a native
button and a live web page under every overlay and counts what each received, and the gate
fails if a count moves.

How it holds depends on the OS:

- macOS: each overlay is its own window above the owner, so the window server routes input and
  nothing relies on hit-testing inside one window. This matters for hover. Tracking areas fire
  for covered views, so an overlay drawn inside the window would still make a web view's
  `:hover` flicker. Popovers are `NSPopover`, panels and tooltips are borderless child panels
  that never become key, so the owner keeps its key chrome and keys keep reaching nd's dispatch.
  One event monitor applies the outside-press and Escape rules while any overlay is open.
- Linux, Windows, web: overlays draw in GPUI's deferred layer, above their parents, with a
  transparent backdrop that consumes presses. A light-dismiss backdrop lets scrolls through.
  Gesture hit testing starts inside the topmost overlay or hits nothing.

A press outside a light-dismiss overlay closes it and is consumed, release included, on every
OS. The element under the pointer sees nothing. This matches what macOS menus already do.

Declared accelerators run through the main menu before the window's views, so an embedded page
cannot swallow a shortcut while an overlay is up.

Not done: native surfaces that the OS composites above GPUI (WebView2, WPE) need either the
overlay composition surface under `nd-gpui/fast` or a child popup window. That lands with those
engines, see [webview](webview.md). The leak gate runs on macOS and on Hyprland.

## Command palette

`command_palette(id)` is a glass card near the top of the window with a text field, ranked
rows and keyboard navigation. It is controlled: you pass the open flag, the query and the rows,
and you react to the events. Notes puts it in the scaffold's overlay slot:

<!-- docs:snippet examples/notes/src/app.rs#palette -->

Rows are `palette_item(id, title)` with `.subtitle`, `.icon`, `.hint` (a shortcut label) and
`.completion`, the text the field completes inline when the query is a prefix. Notes ranks
notes first, then commands, so a title that starts with the query offers itself. The palette
shows exactly the rows you give it, in order; filtering is yours, and
[`fuzzy.rs`](../../../crates/nd-components/src/fuzzy.rs) has the ranker notes uses.
`on_submit` fires when nothing matched, with the typed text.

## Toasts

`toast_overlay(id)` shows one window's toasts, one at a time, at the bottom. The list is yours:
append a `toast(id, title)` to show one and remove it when `on_dismiss` fires.

<!-- docs:snippet examples/gallery/src/lib.rs#toasts -->

A toast goes away after 5 seconds. `.persistent()` keeps it until dismissed, `.timeout(..)`
changes the time, and `.action("Undo")` adds one button whose click arrives at `on_action`
before the dismissal. A `.high_priority()` toast interrupts the one on screen, which is
dismissed with reason `Interrupted`; normal ones queue in order.

The gallery has a page each for the palette and for toasts.
