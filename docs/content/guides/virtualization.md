+++
title = "Virtualization"
description = "Lists, tables, grids and trees over keyed rows: ListState, versions, tail following, jump targets, selection, row actions and how the macOS and GPUI presenters differ."
+++

Any list that can pass a few hundred rows goes through [`nd-virtual`](../../../crates/nd-virtual/src/lib.rs).
You keep a `ListState<K>` in your view, give it keys, and render one row at a time for the keys
the presenter asks for. The same state drives `list`, `table`, `grid` and `tree`.

<!-- docs:example scroll -->

## A list

`ListState::new` ties the list to the view that owns it. Keys are anything `Clone + Eq + Hash +
Debug`. The estimate is the height assumed for rows nobody has measured yet.

<!-- docs:snippet examples/scroll/src/lib.rs#state -->

`list(id, &state, render_row)` is the element. The closure gets a key and returns one row. It
runs only for rows in or near the viewport, outside your view's `render` and outside any native
callback, so it cannot reach `self`. Read from entities or captured handles instead; the notes
list does it with `cx.processor`.

<!-- docs:snippet examples/scroll/src/lib.rs#render -->

## Keys and versions

`set_items` and `set_items_versioned` replace the rows by key. There is no splice and no reset.
Rows that stay keep their measured height, and the scroll position stays on its row. Inserts and
removals animate on macOS.

A row is its own view, so your view re-rendering does not re-render it. A row renders again when
its version changes or you call `invalidate(&key)`. Pass versions, such as a modification counter
or a hash of what the row shows, and unchanged rows cost nothing. With plain `set_items` every
rendered row renders again.

<!-- docs:snippet examples/notes/src/note_list.rs#refresh -->

If the row reads something outside its key, such as a setting, call `invalidate_all` when it
changes.

## Where the view sits

The scroll position is an anchor: a row and how far into it the viewport starts. It is never a
pixel offset, so these hold on both presenters:

- A height change above the anchor, an insert above it or a prepend moves nothing on screen.
- Rows can be taller than the estimate. Clamping uses measured rows only, so a low estimate can
  never put the end of the list out of reach.
- A resize keeps the anchor row in place.

Estimates matter only for the scrollbar and for how far a jump travels before it settles.

## Following the tail

A log or a chat stays at the bottom while rows arrive:

<!-- docs:snippet examples/scroll/src/lib.rs#log -->

`Follow::Tail` pins the list to its end while the user is at the end. Scrolling away stops it,
scrolling back to the end starts it again, and `is_following_tail()` says which. A last row that
grows after the list reached the end, as an image row does once it loads, stays fully in view.
`Alignment::Bottom` also rests short content against the bottom edge, for chat.

`on_near_start` and `on_near_end` fire when the viewport reaches the first or last rows. Load
the next page there and call `set_items_versioned` with the longer list.

## Jumping to a row

`scroll_to(key, align, offset, cx)` brings a row into place and animates short distances on the
frame clock. `jump_to` takes the same arguments without the animation. `Align` is `Start`,
`Center`, `End` or `Nearest`; `Nearest` scrolls only when the row is not fully visible. A
negative offset leaves room above the result. The jump settles after the rows on the way are
measured, so a row with a wrong estimate still lands where you asked.

The notes list uses `Nearest` so selecting a visible row does not move the list:

<!-- docs:snippet examples/notes/src/note_list.rs#select -->

`scroll_to_top`, `scroll_to_end` and `scroll_by` cover the rest.

## Selection, activation and row actions

`.selection(Selection::single())` or `Selection::multiple()` turns on selection, and `.on_select`
reports it with the clicked key and the whole set. `.on_activate` fires on double click. Set the
selection from code with `select(key)` or `set_selected(keys)`; keys no longer in the list are
dropped.

Row actions are a method of the list, because on macOS they are `NSTableViewRowAction`s that come
from the table's delegate. The closure runs for the row being swiped:

<!-- docs:snippet examples/notes/src/note_list.rs#list -->

`full_swipe` makes a long swipe on that edge run its first action. Off macOS the swipe is
drawn by the framework and arbitrated against the list's own scrolling, see
[gestures](gestures.md).

## Tables, grids and trees

The other three take the same state.

<!-- docs:snippet examples/scroll/src/lib.rs#table -->

- `table(id, &state, columns, render_cell)` renders one cell per column. `column(id, title)`
  takes `width`, `min_width`, `fixed`, `sortable` and `sorted`. Clicking a sortable header
  fires `on_sort` with the column id and direction. The list never reorders itself: sort your
  data and call `set_items` again. `on_column_resize` reports a resized column.
- `grid(id, &state, (width, height), render_item)` lays fixed-size cells in as many lanes as
  fit.
- `tree(id, &state, render_row)` takes `set_tree(nodes)` with `TreeNode { key, parent, version }`
  in display order, parents before children. Expansion is kept by key, so open branches survive
  an update; change it with `set_expanded` and listen with `on_expand`.

Pass `.empty(element)` for what shows when there are no rows, usually a `status_page`.

## What each platform does

Rows are the same on every backend. The container differs:

| | macOS | Linux, Windows, web |
| --- | --- | --- |
| List, table | view-based `NSTableView` | the GPUI presenter, an element of our own |
| Tree | `NSOutlineView` | GPUI presenter with drawn disclosure |
| Grid | `NSCollectionView` | GPUI presenter |
| Scrolling | `NSScrollView`: native momentum and elastic overscroll | framework momentum and rubber band |
| Row actions | `NSTableViewRowAction` | drawn, with a direction lock against the list |
| Keyed update | `insertRows`, `removeRows` | applied directly |

`nd-virtual` owns the height tree on both. On macOS AppKit's own row-height estimation is
turned off, `heightOfRow` answers from our tree, and rows near the viewport are measured outside
AppKit callbacks. The data source answers from rows already rendered. GPUI's own `ListState` is
not used: its rule that rows outside the viewport must not change height without a `splice` is
what makes the last rows vanish.

Not on the GPUI presenter yet: sticky headers, drag reorder, column resize and keyboard
selection. Type-ahead is not shipped on either. Trees and grids apply changes with a full reload on macOS, so they do not
animate inserts. See [TODO.md](../../plan/TODO.md) for the open list.

## The last-row bug class

Lists that scroll by pixel offset and trust their estimates lose rows at the end when real
heights differ, when a row grows after layout, or when rows load above. The conformance suite in
[conformance.rs](../../../crates/nd-virtual/src/conformance.rs) pins these down with rows whose
heights are far from the estimate:

- the last row is reachable by wheel alone and by a jump;
- it stays reachable after it grows or shrinks once the list reached the end;
- a height change above the anchor, or a prepend, leaves the visible rows where they were;
- tail follow stops on user scroll and resumes at the end;
- 100 000 rows stay inside the frame budget.

One suite runs against the model, the GPUI presenter in a headless app and a real `NSTableView`.
A new presenter has to pass it as is.

## Measuring

`nd diag scroll --rows 100000` launches the scroll example, scrolls it a fixed step every
display frame and reads the host's frame interval line back. It passes when no frame was missed
after the first second's warm-up and p95 stays within a quarter period of the display's rate.
`--seconds` and `--step` change the run; see [diagnostics](../tooling/diagnostics.md) for
budgets and what a failure prints.

`ND_SCROLL_STEP` and `ND_ROWS` give the same behaviour when you start the example by hand.
