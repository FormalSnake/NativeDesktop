---
title: Data Views
description: "Table and TreeView render multi-column and hierarchical data with one shared rule: the native widget never reorders or re-nests your data."
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

See `examples/gallery/main.tsx`'s "Table" and "Tree" tabs for both wired to full controlled state
(including the JS-side sort), and the [Widget Reference](/components/widget-reference/) for the
generated prop tables.
