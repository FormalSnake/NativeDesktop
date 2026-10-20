+++
title = "Virtualization"
description = "Lists, tables, grids, and trees over keyed rows: ListState, versions, tail following, jump targets, selection, row actions, and how the macOS and GPUI presenters differ."
+++

Any list that can grow past a few hundred rows goes through [`nd-virtual`](../../../crates/nd-virtual/src/lib.rs).
You keep a `ListState<K>` in your view, give it keys, and render one row at a time for the keys
that the presenter asks for. The same state drives `list`, `table`, `grid`, and `tree`.

<!-- docs:example scroll -->

## A list

`ListState::new` ties the list to the view that owns it. A key can be any type that's `Clone + Eq +
Hash + Debug`. The estimate is the height that the list assumes for rows it hasn't measured yet.

<!-- docs:snippet examples/scroll/src/lib.rs#state -->

`list(id, &state, render_row)` is the element. The closure gets a key and returns one row. It
runs only for rows in or near the viewport, outside your view's `render` and outside any native
callback, so it can't reach `self`. Read from entities or captured handles instead. The notes
list does this with `cx.processor`.

<!-- docs:snippet examples/scroll/src/lib.rs#render -->

## Keys and versions

`set_items` and `set_items_versioned` replace the rows by key. There's no splice and no reset.
Rows that stay keep their measured height, and the scroll position stays on its row. Inserts and
removals animate on macOS.

A row is its own view, so when your view renders again, the row doesn't. A row renders again when
its version changes or when you call `invalidate(&key)`. Pass versions, such as a modification
counter or a hash of what the row shows, and unchanged rows cost nothing. With plain `set_items`,
every rendered row renders again.

<!-- docs:snippet examples/notes/src/note_list.rs#refresh -->

If a row reads something outside its key, such as a setting, call `invalidate_all` when that
value changes.

## Where the view sits

The scroll position is an anchor: a row, and how far into that row the viewport starts. It's
never a pixel offset, so these rules hold on both presenters:

- A height change above the anchor, an insert above it, or a prepend doesn't move anything on
  screen.
- Rows can be taller than the estimate. Clamping uses measured rows only, so a low estimate can't
  put the end of the list out of reach.
- A resize keeps the anchor row in place.

Estimates matter only for the scrollbar and for how far a jump travels before it settles.

## Following the tail

A log or a chat stays at the bottom while rows arrive:

<!-- docs:snippet examples/scroll/src/lib.rs#log -->

`Follow::Tail` pins the list to its end while the user is at the end. When the user scrolls away,
following stops. When they scroll back to the end, it starts again. `is_following_tail()` tells
you which state the list is in. If the last row grows after the list reaches the end, as an image
row does once its image loads, the row stays fully in view. For chat, `Alignment::Bottom` also
rests short content against the bottom edge.

`on_near_start` and `on_near_end` fire when the viewport reaches the first or last rows. Load
the next page there, and call `set_items_versioned` with the longer list.

## Jumping to a row

`scroll_to(key, align, offset, cx)` brings a row into place and animates short distances on the
frame clock. `jump_to` takes the same arguments and doesn't animate. `Align` is `Start`,
`Center`, `End`, or `Nearest`. `Nearest` scrolls only when the row isn't fully visible. A
negative offset leaves room above the target row. The jump settles after the list measures the
rows on the way, so a row with a wrong estimate still lands where you asked.

The notes list uses `Nearest`, so selecting a visible row doesn't move the list:

<!-- docs:snippet examples/notes/src/note_list.rs#select -->

For everything else, use `scroll_to_top`, `scroll_to_end`, and `scroll_by`.

## Selection, activation, and row actions

To turn on selection, pass `.selection(Selection::single())` or `Selection::multiple()`.
`.on_select` reports the clicked key and the whole selected set. `.on_activate` fires on a double
click. To set the selection from code, call `select(key)` or `set_selected(keys)`. The list drops
keys that are no longer in it.

Row actions are a method of the list, which asks for them per row. The closure runs for the row
that the user swipes:

<!-- docs:snippet examples/notes/src/note_list.rs#list -->

`full_swipe` makes a long swipe on that edge run its first action. The framework draws the swipe
on every platform, with a spring that carries the release velocity and a haptic at the full-swipe
threshold. On macOS the table takes horizontal trackpad gestures on rows that have actions and
leaves the rest to its scroll view; `NSTableViewRowAction` is not used because it narrows the
button as the row closes. Elsewhere the swipe is arbitrated against the list's own scrolling. For
details, see [gestures](gestures.md).

## Tables, grids, and trees

The other three elements take the same state.

<!-- docs:snippet examples/scroll/src/lib.rs#table -->

- `table(id, &state, columns, render_cell)` renders one cell per column. `column(id, title)`
  takes `width`, `min_width`, `fixed`, `sortable`, and `sorted`. Selecting a sortable header
  fires `on_sort` with the column id and direction. The list never reorders itself, so sort your
  data and call `set_items` again. `on_column_resize` reports a resized column.
- `grid(id, &state, (width, height), render_item)` lays out fixed-size cells in as many lanes as
  fit.
- `tree(id, &state, render_row)` takes `set_tree(nodes)` with `TreeNode { key, parent, version }`
  in display order, parents before children. The tree keeps expansion by key, so open branches
  survive an update. Change expansion with `set_expanded`, and listen for it with `on_expand`.

Pass `.empty(element)` to set what shows when there are no rows. This is usually a `status_page`.

## What each platform does

Rows are the same on every backend. Only the container differs:

| | macOS | Linux, Windows, web |
| --- | --- | --- |
| List, table | View-based `NSTableView` | The GPUI presenter, an element of our own |
| Tree | `NSOutlineView` | GPUI presenter with drawn disclosure |
| Grid | `NSCollectionView` | GPUI presenter |
| Scrolling | `NSScrollView`: native momentum and elastic overscroll | Framework momentum and rubber band |
| Row actions | Drawn in the row view, from the table's horizontal scroll gestures | Drawn, with a direction lock against the list |
| Keyed update | `insertRows`, `removeRows` | Applied directly |

`nd-virtual` owns the height tree on both. On macOS, AppKit's own row-height estimation is off,
`heightOfRow` answers from our tree, and the framework measures rows near the viewport outside
AppKit callbacks. The data source answers from rows that are already rendered. The framework
doesn't use GPUI's own `ListState`. That type requires that rows outside the viewport never change
height without a `splice`, and that rule is what makes the last rows vanish.

The GPUI presenter doesn't have sticky headers, drag reorder, column resize, or keyboard selection
yet. Neither presenter ships type-ahead. On macOS, trees and grids apply changes with a full
reload, so they don't animate inserts. For the open list, see [TODO.md](../../plan/TODO.md).

## The last-row bug class

Lists that scroll by pixel offset and trust their estimates lose rows at the end when real
heights differ, when a row grows after layout, or when rows load above. The conformance suite in
[conformance.rs](../../../crates/nd-virtual/src/conformance.rs) pins these cases down with rows
whose heights are far from the estimate:

- The last row is reachable by the wheel alone and by a jump.
- It stays reachable when it grows or shrinks after the list has reached the end.
- A height change above the anchor, or a prepend, leaves the visible rows where they were.
- Tail follow stops when the user scrolls and resumes at the end.
- 100,000 rows stay inside the frame budget.

One suite runs against the model, the GPUI presenter in a headless app, and a real `NSTableView`.
A new presenter has to pass it without changes.

## Measuring

`nd diag scroll --rows 100000` launches the scroll example, scrolls it a fixed step every
display frame, and reads back the host's frame interval line. The run passes when no frame is
missed after the first second of warm-up and p95 stays within a quarter of the display's frame
period. To change the run, use `--seconds` and `--step`. For budgets and what a failure prints,
see [diagnostics](../tooling/diagnostics.md).

When you start the example yourself, `ND_SCROLL_STEP` and `ND_ROWS` give the same behavior.
