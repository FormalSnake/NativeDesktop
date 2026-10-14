+++
title = "Elements"
description = "The element vocabulary: div, flex helpers, text, styling, layout, identity and children."
+++

`render` returns elements. An element is a description, not a widget: it says what should exist, and
the reconciler decides what to create, keep or remove. `use nd::prelude::*;` brings the whole
vocabulary into scope.

<!-- docs:example hello -->

## The building blocks

- `div()` is a box. `v_flex()` and `h_flex()` are a `div` with a column or row flex direction.
- `text("..")` is a run of text.
- `button(id, label)`, `text_field(id)`, `list(..)` and the other [components](../components/index.md)
  are elements that become native controls where the platform has one.
- `scaffold()`, `toolbar()` and `sidebar()` describe window structure rather than boxes. The OS
  draws its own version of each.

Children go in with `.child(x)` for one and `.children(iter)` for many. Anything that implements
`IntoElement` can be a child, and so can an `Entity<V>` of a view: that child renders on its own.

<!-- docs:snippet examples/hello/src/lib.rs#counter -->

## Styling

Elements take GPUI's `Styled` methods: `size_full()`, `gap_2()`, `px_3()`, `items_center()`,
`justify_center()`, `flex_1()`, `min_h_0()`, `text_size(px(48.))`, `text_color(..)` and so on. A
method writes one field of the element's style. Text has a few of its own, such as
`tabular_nums()` for digits that do not shift as a count changes.

Take colours, spacing and type sizes from `cx.theme()`: `theme.text.secondary`,
`theme.spacing.content_margin`, `theme.type_scale.title`. A literal colour fails `nd check`, and
the tokens follow the platform's light and dark appearance and a matugen palette. The
[theming page](theming.md) has the full list.

## Layout

Layout is flexbox, computed by Taffy with the same style mapping on every backend. A `v_flex` stacks
children top to bottom; `.flex_1()` gives a child the remaining space; `.min_w_0()` lets a child
shrink below its content width. Native controls report their own size (`measure`), so a button
keeps the size the OS gives it.

Some containers lay out their own children: a native list's rows, a split view's panes, a
toolbar's items and a scroll view's document. Those are layout boundaries. The container tells the
child how much room it has, the framework lays out inside, and the child's height or intrinsic size
goes back to the container.

## Identity

The reconciler matches each new element to a retained node by identity:

- With an explicit id, by element type and that id: `text("..").id("status")`, `button("add", ..)`.
- Without one, by element type, the source location of the constructor and the occurrence index
  among siblings that share that location.

Location-based identity is what makes `.when(..)` safe. A conditional child leaves no hole in the
child list, so a sibling stays the same native node when the condition flips, and keeps its focus,
text selection and scroll position.

<!-- docs:snippet examples/browser/src/lib.rs#conditional -->

Here the progress bar appears and disappears without touching the page view below it. Give an
explicit id to anything stateful that you create in a loop or a helper function: text fields, lists,
tables, web views. An explicit id also serves as the element's test id, which is how
`click(cx, window, "add")` finds a node.
