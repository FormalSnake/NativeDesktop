---
title: Data Views
description: "Table, TreeView and SourceTree render multi-column and hierarchical data with one shared rule: the native widget never reorders or re-nests your data."
---

`<table>` and `<treeview>` both take a plain data prop (`rows`/`nodes`) you own in React state, and
both follow the same contract: the native widget renders exactly what you give it and asks you, via an
event, when the user wants something to change. It never reorders or re-nests the data itself, so
your React state stays the single source of truth, the same discipline `<listview>`'s `items`
already follows.

## Table (`<table>`)

A multi-column list, backed by `GtkColumnView` on GTK and `NSTableView` on macOS. You describe it
with a columns array and a rows array kept separate, so a column resize or reorder never has to touch
every row.

```tsx
import type { TableColumn, TableRow } from "@nativedesktop/react";

const columns: TableColumn[] = [
  { id: "name", title: "Name" },
  { id: "role", title: "Role" },
  { id: "department", title: "Department", width: 160 },
];

function handleSortChanged(e: { data: unknown }) {
  const { columnId, direction } = e.data as { columnId: string; direction: "ascending" | "descending" };
  setEmployees((prev) => {
    const sorted = [...prev].sort((a, b) => {
      const cmp = String(a[columnId]).localeCompare(String(b[columnId]));
      return direction === "ascending" ? cmp : -cmp;
    });
    return sorted;
  });
}

<table
  columns={columns}
  rows={employees.map((e): TableRow => ({ id: e.id, cells: [e.name, e.role, e.department] }))}
  selectedIndex={selectedIndex}
  onSelectionChanged={(e) => setSelectedIndex(e.index)}
  onRowActivated={(e) => setActivatedIndex(e.index)}
  onSortChanged={handleSortChanged}
/>;
```

| Prop | Type | Applied | Notes |
| --- | --- | --- | --- |
| `columns` | `TableColumn[]` | createAndUpdate | `{ id, title, width? }`. `id` is echoed back in `sortChanged`, never shown. |
| `rows` | `TableRow[]` | createAndUpdate | `{ id?, cells }`. `cells` is positional, indexed by `columns`' order. |
| `selectedIndex` | int | createAndUpdate | Default `-1`. |
| `showRowSeparators` | bool | createAndUpdate | Default `true`. |

| Event | Handler | Payload |
| --- | --- | --- |
| `selectionChanged` | `onSelectionChanged` | `{ index }` |
| `rowActivated` | `onRowActivated` | `{ index }` (double-click / Enter) |
| `sortChanged` | `onSortChanged` | `{ data: { columnId, direction } }` |

Clicking a column header fires `sortChanged` and stops there: the native widget shows the sort
indicator arrow but never reorders `rows` itself. Your `onSortChanged` handler owns sorting: re-sort
`rows` in JS as above and pass the new array back down. This mirrors `<listview>`'s "native never
mutates your data" contract and keeps sorting logic in one place, where you can test it outside the
UI.

## TreeView (`<treeview>`)

A hierarchical outline, backed by `GtkTreeListView`/`GtkColumnView` on GTK and `NSOutlineView` on
macOS. You describe it as a flat array keyed by `id`/`parentId`, not nested JS objects. Root nodes
omit `parentId`.

```tsx
import type { TreeNode } from "@nativedesktop/react";

const nodeMeta: Omit<TreeNode, "expanded">[] = [
  { id: "fruits", title: "Fruits", hasChildren: true },
  { id: "apple", parentId: "fruits", title: "Apple", hasChildren: false },
  { id: "banana", parentId: "fruits", title: "Banana", hasChildren: false },
];

const [expanded, setExpanded] = useState<Set<string>>(new Set(["fruits"]));
const nodes: TreeNode[] = nodeMeta.map((n) => ({ ...n, expanded: expanded.has(n.id) }));

<treeview
  nodes={nodes}
  indentationPerLevel={16}
  onSelectionChanged={(e) => setSelected((e.data as { nodeId: string | null }).nodeId)}
  onNodeExpanded={(e) => {
    const { nodeId } = e.data as { nodeId: string };
    setExpanded((prev) => new Set(prev).add(nodeId));
  }}
  onNodeCollapsed={(e) => {
    const { nodeId } = e.data as { nodeId: string };
    setExpanded((prev) => { const next = new Set(prev); next.delete(nodeId); return next; });
  }}
/>;
```

| Prop | Type | Applied | Notes |
| --- | --- | --- | --- |
| `nodes` | `TreeNode[]` | createAndUpdate | `{ id, parentId?, title, badge?, iconName?, hasChildren, expanded }`, a flat list rather than a nested one. |
| `selectedIndex` | int | createAndUpdate | Default `-1`, indexes the flattened *visible* rows. |
| `indentationPerLevel` | int | create | Pixels per depth level, default `16`. |

| Event | Handler | Payload |
| --- | --- | --- |
| `selectionChanged` | `onSelectionChanged` | `{ data: { nodeId } }` |
| `rowActivated` | `onRowActivated` | `{ data: { nodeId } }` |
| `nodeExpanded` | `onNodeExpanded` | `{ data: { nodeId } }` |
| `nodeCollapsed` | `onNodeCollapsed` | `{ data: { nodeId } }` |

Expansion is controlled state, not native state: the widget asks (via `nodeExpanded`/
`nodeCollapsed`) rather than deciding on its own, so a re-render triggered by something else in your
app can never silently collapse a branch the user opened. Track expansion yourself (a `Set<string>`
of expanded ids, as above) and feed it back into every node's `expanded` field.

## SourceTree (`<sourcetree>`)

A hierarchical *sidebar*: section headers, id/parentId rows with a caption line, badges, and
trailing per-row actions. GTK renders a `navigation-sidebar` GtkListBox of AdwActionRows; macOS a
`.sourceList`-style `NSOutlineView`. Use `<sourcelist>` for a flat index-addressed sidebar and
`<treeview>` for a generic disclosure tree in a content pane; `<sourcetree>` is the one that owns
"app chrome sidebar with structure".

Rows are the same flat `id`/`parentId` shape as TreeView, with sidebar extras:

```tsx
import type { SourceTreeAction, SourceTreeNode } from "@nativedesktop/react";

const actions: SourceTreeAction[] = [
  { id: "new-run", iconName: "list-add-symbolic", label: "New Run" },
  { id: "close-run", iconName: "window-close-symbolic", tooltip: "Close run", destructive: true },
];

const nodes: SourceTreeNode[] = [
  { id: "sec-hosts", title: "Hosts", section: true, hasChildren: true, expanded: true },
  { id: "host-mac", parentId: "sec-hosts", title: "macbook", caption: "connected",
    iconName: "computer-symbolic", hasChildren: true, expanded: true },
  { id: "run-1", parentId: "host-mac", title: "fix sidebar", caption: "running · 2m",
    badge: "3", actionIds: ["close-run"], testID: "run-row-1" },
];

<sourcetree
  nodes={nodes}
  actions={actions}
  selectedId={selectedId}
  onSelectionChanged={(e) => setSelectedId((e.data as { nodeId: string | null }).nodeId ?? "")}
  onActionClicked={(e) => {
    const { nodeId, actionId } = e.data as { nodeId: string; actionId: string };
    // ...
  }}
/>;
```

| Prop | Type | Applied | Notes |
| --- | --- | --- | --- |
| `nodes` | `SourceTreeNode[]` | createAndUpdate | Flat `{ id, parentId?, title, caption?, iconName?, captionIconName?, badge?, section?, hasChildren?, expanded?, selectable?, actionIds?, testID? }`. |
| `actions` | `SourceTreeAction[]` | createAndUpdate | The action catalog rows reference by `actionIds`: `{ id, iconName, label?, tooltip?, destructive? }`. |
| `selectedId` | string | createAndUpdate | Controlled selection by node id; `""` means none. There is no `selectedIndex`. |
| `actionVisibility` | `"hover"` \| `"always"` | create | Default `"hover"`: action buttons show only while the pointer is over the row. |
| `indentationPerLevel` | int | create | Pixels per depth level, default `14`. |

| Event | Handler | Payload |
| --- | --- | --- |
| `selectionChanged` | `onSelectionChanged` | `{ data: { nodeId } }` (`nodeId` null on deselect) |
| `rowActivated` | `onRowActivated` | `{ data: { nodeId } }` |
| `nodeExpanded` | `onNodeExpanded` | `{ data: { nodeId } }` |
| `nodeCollapsed` | `onNodeCollapsed` | `{ data: { nodeId } }` |
| `actionClicked` | `onActionClicked` | `{ data: { nodeId, actionId } }` |

Semantics shared with TreeView: expansion is controlled state (track a `Set<string>` of expanded
ids and feed it back into `expanded`), and events address rows by `nodeId` because visible indexes
shift under expand/collapse. `section: true` marks a group header: non-selectable, styled as a
native section label on both platforms; a section with `hasChildren` is a collapsible shelf.
`selectable: false` makes an informational row (an empty-state line) unselectable without making it
a header. Hover is the widget's own native affordance; there is deliberately no `hoverChanged`
event, so apps stop hand-rolling per-row hover state.

Per-node `testID` surfaces in the automation tree's `rows` (see the getTree docs), so tests target
rows without depending on row order. Platform asymmetries: GTK draws a manual disclosure button per
parent row (GtkListBox has no native outline affordance) and renders `captionIconName` as a small
second prefix icon; macOS sections have no disclosure triangle (native source-list group look) and
inline the caption icon on the caption line.

## Native empty states

Every collection widget — `<sourcelist>`, `<listview>`, `<table>`, `<treeview>`, `<sourcetree>` —
takes three optional createAndUpdate props: `emptyIconName`, `emptyTitle`, `emptyDescription`. When
the item array is empty AND at least one of them is set, the widget shows platform empty-state
chrome in place of the blank list (a compact `AdwStatusPage` on Linux, a centered
icon/title/description stack on macOS) and swaps the real list back the moment items return:

```tsx
<table
  columns={columns}
  rows={rows}
  emptyIconName="folder-open"
  emptyTitle="No results"
  emptyDescription="Try a broader filter."
/>
```

Unset (the default) means no swap ever happens — existing lists are unaffected. This replaces the
hand-composed `<statuspage>`-next-to-an-empty-list pattern.

See `examples/gallery/main.tsx`'s "Table", "Tree" and "SourceTree" tabs for all three wired to full
controlled state (including the JS-side sort), and the
[Widget Reference](/components/widget-reference/) for the generated prop tables.
