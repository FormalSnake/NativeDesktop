---
title: Layout
description: How <box> packs its children on both backends, and when hexpand/vexpand/halign/valign actually matter.
---

`<box>` is the one layout primitive: an `orientation` ("horizontal" or "vertical") plus a main axis
and a cross axis. The same `style` keys drive both backends (`hexpand`, `vexpand`, `halign`, `valign`,
`padding`, `margin`, `spacing`, `minWidth`, `minHeight`), but GTK and AppKit resolve the cross axis
differently, and a flag set somewhere it can't do anything is the most common reason an example still
carries a wrapper box it no longer needs.

## Main axis: pack, then split the leftover

Children lay out in order at their natural size, separated by `spacing`. Any leftover space in the
box goes to the children whose main-axis expand flag is set (`vexpand` in a vertical box, `hexpand`
in a horizontal one), split evenly across however many of them there are. A child with no expand flag
never grows past its natural size, no matter how much room the box has.

This is a real, per-relationship flag: a box needs `vexpand: true` to receive leftover height from
*its own* parent, and separately, a child inside that box needs `vexpand: true` to receive leftover
height from *it*. Nesting two boxes means checking the flag at each level, not assuming one expand
higher up covers everything below it.

## Cross axis: `halign`/`valign`, resolved differently per backend

The cross axis (horizontal in a vertical box, vertical in a horizontal box) is controlled by
`halign`/`valign`: `"fill"`, `"start"`, `"center"`, or `"end"`.

**GTK stretches every child across the box's perpendicular axis by default.** `halign`/`valign` are
plain GTK widget properties (`gtk_widget_set_halign`/`set_valign`), and the default for a widget with
no explicit alignment is fill. `hexpand`/`vexpand` on a child that is already filling the cross axis
by default do nothing extra there; they only matter on the main axis, or on the cross axis if you
deliberately set `halign`/`valign` away from fill and want the child sized wider than that would allow.

**AppKit fills by default only for shapes that don't have a native "correct" size.** Stretching an
`NSButton`, popup, switch, checkbox, segmented control, stepper, or date/color picker to the box's
full width is never how those controls look on macOS, so `swift/Sources/NDShell/Layout.swift`'s
`ndHasIntrinsicCrossSize` keeps them at their natural cross size unless the app explicitly asks for
`"fill"` (or sets the cross-axis expand flag). Everything else fills by default:

- every container and scroll shape (`<box>`, `<splitview>`, `<scrollview>`, `<tabview>`, `<grid>`,
  `<table>`, `<treeview>`, `<sourcelist>`, `<sourcetree>`, `<listview>`, `<toolbarview>`,
  `<headerbar>`, `<terminal>`, `<webview>`, `<video>`, and a few more internal ones): these exist to
  fill whatever they're given
- an editable or bezeled text field (`<textinput>`, `<searchinput>`): an empty field's natural width
  is a few points, so holding it there would render an unusable stub
- a non-editable, non-bezeled `<label>`: sized by its text on both axes either way, so "fill" and
  "natural" agree
- `<slider>` and a determinate `<progressbar>` fill their *length* axis and stay natural on their
  *thickness* axis (a horizontal slider doesn't stretch to a row's full height, but it does stretch
  along the row)

A `<box>` nested inside another box always falls into the "fill" case on AppKit too: `NSStackView`
reports no intrinsic size on either axis, so there's nothing for `ndHasIntrinsicCrossSize` to hold it
at. Setting `hexpand`/`vexpand` on a nested `<box>` for cross-axis fill is therefore redundant on
both backends: it was already going to fill without them.

## Spacing, padding, margin

`spacing` is the gap between a box's children along its main axis. Its default, `-1`, means "platform
standard": 6 on GTK (the Adwaita gutter), 8 on AppKit. Import `Spacing`/`ContentMargin` from
`@nativedesktop/react` instead of hardcoding either number; see
[Spacing scale](/core-concepts/styling-design-language/#spacing-scale).

`padding` insets a container's own children. On GTK it's CSS `padding` on the box. On AppKit it maps
per widget shape, because AppKit has no single content-inset API: an `NSStackView` gets its own
`edgeInsets`, a button or text field inflates `intrinsicContentSize`, a `<textarea>`'s scroll view
gets `textContainerInset`, and any other `<scrollview>` (which is also what `<table>`, `<treeview>`,
`<sourcetree>`, `<sourcelist>`, and `<listview>` are built on) gets `contentInsets`. A handful of leaf
shapes with no native inset API (`<image>`, `<separator>`, `<slider>`, `<progressbar>`, `<spinner>`,
pickers, `<grid>`, `<tabview>`, the plain-button controls, `<searchinput>`) drop an app-declared
`padding` with one `ND_WARN`; give the parent box padding instead.

`margin` is a GTK widget property (`gtk_widget_set_margin_*`), not CSS, because it affects the space a
widget claims from its parent rather than anything the widget draws.

## `minWidth`/`minHeight`

Both map to a floor, not a fixed size: GTK's `gtk_widget_set_size_request`, and on AppKit a
`>=` width/height constraint at priority 999 (so a conflicting frame-based constraint wins instead of
crashing on an unsatisfiable layout). Either key dropping out of `style` clears its floor.

## Minimum vs. natural size, and the window's own minimum

Every widget has a natural size (what it asks for) and, on GTK, a separate minimum size (the smallest
it can be compressed to before content is clipped). GTK's window minimum comes from that same
negotiation: each widget's minimum size bubbles up through every box it sits in, so a GTK window
can't be resized smaller than its content's aggregate minimum unless something in the tree can keep
shrinking (a scroll view, an ellipsized label, a compressible text field). `defaultWidth`/
`defaultHeight` only seed `gtk_window_set_default_size`, the *initial* size; the resize floor is
whatever the content tree computes underneath it.

AppKit windows in this framework don't run that negotiation: the window is created directly at
`defaultWidth`x`defaultHeight` (schema defaults 480x320 when unset) with no constraint tying its
minimum size back to content, so it can be resized down as far as the OS allows and content
compresses, scrolls, or clips accordingly, the same way any of its individual leaf controls would
without a `minWidth`/`minHeight` floor of their own.

## Wrapping and ellipsize

There's no `wrap` prop. `<label>` has one text-overflow control, `ellipsize` (bool, default `false`):
set it and the label truncates to one line with a trailing ellipsis instead of forcing its parent
wider. On GTK that's `gtk_label_set_ellipsize(.end)` plus `max_width_chars(1)` (so the label's
*minimum* request drops to one ellipsis, not its full text) with `hexpand` forced on (so it still
fills whatever width the row hands it). On AppKit it's `lineBreakMode = .byTruncatingTail` with the
label's horizontal compression resistance lowered, so the layout can compress it below its natural
text width instead of the label winning that fight. For real multi-line wrapping, use `<textarea>`
(or a non-editable one for read-only wrapped text) rather than a `<label>`.

## TabView pages size as one unit

Switching tabs doesn't resize the window on either backend. GTK's `<tabview>` is an `AdwViewStack`,
which measures every page and sizes itself from the largest one (`homogeneous`, its own default),
not just whichever page happens to be visible. AppKit's `<tabview>` is an `NSTabViewController`
holding one container view that each page is embedded into in turn, so the container's size comes
from the box the `<tabview>` sits in, not from the page's own content.

## Before/after: a wrapper that isn't needed

A single-child `<box>` whose only job was to carry `hexpand`/`vexpand`/`halign`/`valign` fill, next to
a child that already sets the same flags itself, doesn't change the child's size on either backend:

```tsx
// Before: the wrapper repeats flags <terminal> already has, and does nothing else.
<toolbarview>
  <headerbar title="Terminal" />
  <box style={{ hexpand: true, vexpand: true, halign: "fill", valign: "fill" }}>
    <terminal cols={100} rows={30} fontSize={13} style={{ hexpand: true, vexpand: true }} />
  </box>
</toolbarview>

// After: <terminal> is the pane's own content. Its own hexpand/vexpand still
// govern the same main-axis relationship the wrapper used to carry.
<toolbarview>
  <headerbar title="Terminal" />
  <terminal cols={100} rows={30} fontSize={13} style={{ hexpand: true, vexpand: true }} />
</toolbarview>
```

`<toolbarview>`'s content slot is a single-child container: it hands its one child the full pane
regardless of that child's own flags, on both backends. The wrapper box added a layer without adding
a layout decision. See `examples/terminal/main.tsx` for the shipped version.
