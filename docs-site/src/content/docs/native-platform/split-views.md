---
title: Split Views
description: <splitview> grows a third `list` slot for a folders/list/content three-pane layout, on both platforms.
---

The two-pane sidebar/content `<splitview>` (see [Windows & Chrome](/native-platform/windows-chrome/))
extends to a third pane — `list` — for the "folders / list / content" shape apps like Notes, Mail,
and Files all share: a source list on the left, a filtered list of items in the middle, and a detail
view on the right.

## Shape

Panes are still distinguished by the `slot` attached prop, now with a third value:

```tsx
<splitview sidebarWidth={0.25} listWidth={0.3}>
  <toolbarview slot="sidebar">
    <headerbar title="Folders" />
    {/* folder rows */}
  </toolbarview>

  <toolbarview slot="list">
    <headerbar title="Notes" />
    {/* list rows */}
  </toolbarview>

  <toolbarview slot="content">
    <headerbar title="Editor" />
    {/* detail pane */}
  </toolbarview>
</splitview>
```

(Adapted from `examples/notes/threepane-probe.tsx`, the headless acceptance fixture for this
machinery.) A `<splitview>` with only `sidebar`/`content` children keeps behaving exactly as
before — the `list` slot is additive, not a breaking change to the two-pane shape.

`sidebarWidth` and the new `listWidth` are both create-only fractions setting each split's initial
proportion; `collapsed` remains live-updatable. See the [Widget Reference](/components/widget-reference/)
for the full prop table.

## Platform rendering

### macOS: NSSplitViewController with three items

Each pane becomes an `NSSplitViewItem` on the same `NSSplitViewController` (see
[Windows & Chrome](/native-platform/windows-chrome/) for why that API is what gives the sidebar its
Liquid Glass material):

- `sidebar` → `NSSplitViewItem(sidebarWithViewController:)` — the glass-treated source list.
- `list` → `NSSplitViewItem(contentListWithViewController:)` — a contentList column.
- `content` → a default split item — the detail pane.

The unified `NSToolbar` splits per divider: with three panes there are two
`NSTrackingSeparatorToolbarItem`s (one per divider), each pane's headerbar items landing in their
own section of the one toolbar spanning the window's top edge, same idiom as the two-pane case.

### GNOME: nested AdwOverlaySplitViews

GTK builds this as **two nested `AdwOverlaySplitView`s** — the outer split's sidebar is the
`sidebar` pane, and its content is an inner split whose own sidebar is the `list` pane and whose
content is the `content` pane. This is the same nesting pattern GNOME Files uses. Each pane keeps
its own `<toolbarview>` + `<headerbar>`, so you still get three independent per-pane headers, not
one shared bar.

## Caveat: initial sizing is floor-dominated on macOS

`sidebarWidth`/`listWidth` set the initial split *proportion*, but on macOS each pane also carries a
hard `minimumThickness` — 180pt for the sidebar item, 240pt for the list (contentList) item — and
those floors win over the fraction whenever the window is narrow enough that the fraction would
otherwise ask for less. In practice this means: don't assume a small `listWidth` (e.g. `0.15`) will
render a genuinely narrow list column at typical window widths — measure against the 240pt floor
before relying on an exact initial pixel width.

## Automation

Three-pane trees expose the same `getTree`/`semanticClick` contract as any other widget nesting (see
[Automation Socket](/automation-testing/automation-socket/)) — each pane's rows are ordinary nodes
at their real geometry, so an automation script can assert x-order across panes (e.g. a sidebar row
sits left of a list row, which sits left of the content pane) the same way it would for the
two-pane layout.
