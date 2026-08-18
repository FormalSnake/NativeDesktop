// Dock model: a split tree whose leaves are PANELS, and each panel holds a
// stack of tabs. It is exactly PaneModel<DockPanel<T>>, so ratios, directional
// focus, collapse-on-close and migration come from model.ts unchanged and this
// file only adds the panel/tab vocabulary on top.
//
// Same invariant as model.ts, and for the same reason: an op that changes
// nothing returns the SAME reference. Docking a panel where it already sits,
// activating the active tab or moving a tab to its current index all hand back
// the model they were given, so a drop handler that fires twice cannot loop a
// render+persist cycle.

import {
  clampPaneRatio,
  closePane,
  emptyPanes,
  findPaneNode,
  mapPaneNode,
  migratePanes,
  paneLeaves,
  parentPaneSplit,
  seedPanes,
  updatePane,
} from "./model.ts";
import type { PaneLeaf, PaneModel, PaneNode, SplitOrientation } from "./model.ts";

/** `closePanel` is `closePane`: a panel IS a pane leaf, so closing one
 * collapses its split and hands the space to the sibling. */
export { closePane as closePanel } from "./model.ts";

export type DockZone = "left" | "right" | "top" | "bottom" | "center";
export type DockEdgeZone = Exclude<DockZone, "center">;

export interface DockTab<T> {
  id: string;
  title: string;
  /** Platform icon name, passed straight to the tab child's `tabIcon`. */
  icon?: string;
  data: T;
}

export interface DockPanel<T> {
  tabs: DockTab<T>[];
  activeTabId: string;
}

export type DockModel<T> = PaneModel<DockPanel<T>>;

export interface DockTabLocation<T> {
  panelId: string;
  index: number;
  tab: DockTab<T>;
}

/** Axis and side each edge zone splits on. Side 0 is the leading child of the
 * <paned>, so left/top put the incoming panel first. */
const EDGE: Record<DockEdgeZone, { orientation: SplitOrientation; side: 0 | 1 }> = {
  left: { orientation: "horizontal", side: 0 },
  right: { orientation: "horizontal", side: 1 },
  top: { orientation: "vertical", side: 0 },
  bottom: { orientation: "vertical", side: 1 },
};

export function emptyDock<T>(): DockModel<T> {
  return emptyPanes<DockPanel<T>>();
}

/** A panel over `tabs`, active tab defaulting to the first. An empty tab list
 * is not a legal panel (closeTab collapses the panel instead), so callers get
 * an empty stack back only if they ask for one. */
export function dockPanelOf<T>(tabs: readonly DockTab<T>[], activeTabId?: string): DockPanel<T> {
  const copy = tabs.slice();
  const active = activeTabId !== undefined && copy.some((t) => t.id === activeTabId) ? activeTabId : (copy[0]?.id ?? "");
  return { tabs: copy, activeTabId: active };
}

/** Balanced horizontal dock over one panel per tab group. Groups with no tabs
 * are dropped, so every leaf starts with at least one tab. */
export function seedDock<T>(groups: readonly (readonly DockTab<T>[])[], id?: (panel: DockPanel<T>, i: number) => string): DockModel<T> {
  const panels = groups.filter((g) => g.length > 0).map((g) => dockPanelOf(g));
  return seedPanes(panels, id);
}

export function dockPanels<T>(m: DockModel<T>): PaneLeaf<DockPanel<T>>[] {
  return paneLeaves(m);
}

export function findDockPanel<T>(m: DockModel<T>, panelId: string): DockPanel<T> | undefined {
  const node = findPaneNode(m.root, panelId);
  return node && node.kind === "leaf" ? node.data : undefined;
}

export function findDockTab<T>(m: DockModel<T>, tabId: string): DockTabLocation<T> | undefined {
  for (const leaf of paneLeaves(m)) {
    const index = leaf.data.tabs.findIndex((t) => t.id === tabId);
    if (index >= 0) return { panelId: leaf.id, index, tab: leaf.data.tabs[index]! };
  }
  return undefined;
}

export function activeDockTab<T>(panel: DockPanel<T>): DockTab<T> | undefined {
  return panel.tabs.find((t) => t.id === panel.activeTabId);
}

/** Index of the active tab, for <tabview>'s selectedIndex. Falls back to 0 so
 * a panel whose active id went stale still renders a selected tab. */
export function activeDockTabIndex<T>(panel: DockPanel<T>): number {
  const index = panel.tabs.findIndex((t) => t.id === panel.activeTabId);
  return index < 0 ? 0 : index;
}

function clampIndex(index: number, max: number): number {
  if (!Number.isFinite(index)) return max;
  return Math.min(max, Math.max(0, Math.trunc(index)));
}

/** Drops `tabId` from `panel`. Returns undefined when that was the last tab,
 * which is the caller's signal to collapse the panel instead. */
function withoutTab<T>(panel: DockPanel<T>, tabId: string): DockPanel<T> | undefined {
  const index = panel.tabs.findIndex((t) => t.id === tabId);
  if (index < 0) return panel;
  const tabs = panel.tabs.slice();
  tabs.splice(index, 1);
  if (tabs.length === 0) return undefined;
  // Closing the active tab hands over to the one that slid into its place,
  // and to the one before it when the last tab in the stack went.
  const activeTabId = panel.activeTabId === tabId ? (tabs[index] ?? tabs[index - 1]!).id : panel.activeTabId;
  return { tabs, activeTabId };
}

function withTab<T>(panel: DockPanel<T>, tab: DockTab<T>, index: number, activate: boolean): DockPanel<T> {
  const tabs = panel.tabs.slice();
  tabs.splice(clampIndex(index, tabs.length), 0, tab);
  return { tabs, activeTabId: activate ? tab.id : panel.activeTabId };
}

function setPanel<T>(m: DockModel<T>, panelId: string, panel: DockPanel<T>): DockModel<T> {
  const root = mapPaneNode(m.root!, panelId, (leaf) => ({ ...(leaf as PaneLeaf<DockPanel<T>>), data: panel }));
  return { ...m, root };
}

/** Inserts `node` beside the leaf `targetId` on `zone`'s side, minting the
 * split id from nextId the way splitPane does. */
function insertBeside<T>(
  m: DockModel<T>,
  targetId: string,
  zone: DockEdgeZone,
  node: PaneNode<DockPanel<T>>,
): DockModel<T> {
  const { orientation, side } = EDGE[zone];
  const root = mapPaneNode(m.root!, targetId, (target) => ({
    kind: "split",
    id: `s${m.nextId}`,
    orientation,
    ratio: 0.5,
    children: side === 0 ? [node, target] : [target, node],
  }));
  return { root, focusedId: m.focusedId, nextId: m.nextId + 1 };
}

/** True when `panelId` is already the immediate `zone`-side neighbour of
 * `targetId`. Re-docking there would rebuild an identical tree under a fresh
 * split id, so this is what keeps a repeated drop reference-stable. */
function alreadyDocked<T>(m: DockModel<T>, panelId: string, targetId: string, zone: DockEdgeZone): boolean {
  const parent = parentPaneSplit(m.root, targetId);
  if (!parent) return false;
  const { orientation, side } = EDGE[zone];
  if (parent.orientation !== orientation) return false;
  return parent.children[side].id === panelId && parent.children[side === 0 ? 1 : 0].id === targetId;
}

/** Adds a tab to an existing panel and activates it. A tab id already used
 * anywhere in the dock is rejected (same reference), since every op here
 * addresses tabs by id. */
export function addTab<T>(m: DockModel<T>, panelId: string, tab: DockTab<T>, index?: number): DockModel<T> {
  const panel = findDockPanel(m, panelId);
  if (!panel || findDockTab(m, tab.id)) return m;
  return setPanel(m, panelId, withTab(panel, tab, index ?? panel.tabs.length, true));
}

/** Closes a tab; the last tab in a panel closes the panel, which collapses
 * the split and gives the space back to the sibling. */
export function closeTab<T>(m: DockModel<T>, tabId: string): DockModel<T> {
  const found = findDockTab(m, tabId);
  if (!found) return m;
  const panel = findDockPanel(m, found.panelId)!;
  const next = withoutTab(panel, tabId);
  return next ? setPanel(m, found.panelId, next) : closePane(m, found.panelId);
}

/** Selects a tab inside its own panel. Focus is a separate op (`focusPane`):
 * activating a tab in a background panel should not steal the focus ring. */
export function activateTab<T>(m: DockModel<T>, tabId: string): DockModel<T> {
  const found = findDockTab(m, tabId);
  if (!found) return m;
  const panel = findDockPanel(m, found.panelId)!;
  if (panel.activeTabId === tabId) return m;
  return setPanel(m, found.panelId, { tabs: panel.tabs, activeTabId: tabId });
}

/** Moves a tab to `index` of `targetPanelId`. Within one panel this is a
 * reorder and leaves the active tab alone; across panels the tab is activated
 * in its new home, which takes the focus with it, and a source panel left
 * empty collapses. */
export function moveTab<T>(m: DockModel<T>, tabId: string, targetPanelId: string, index?: number): DockModel<T> {
  const found = findDockTab(m, tabId);
  const target = findDockPanel(m, targetPanelId);
  if (!found || !target) return m;
  const source = findDockPanel(m, found.panelId)!;

  if (found.panelId === targetPanelId) {
    const rest = source.tabs.slice();
    rest.splice(found.index, 1);
    const at = clampIndex(index ?? rest.length, rest.length);
    if (at === found.index) return m;
    rest.splice(at, 0, found.tab);
    return setPanel(m, found.panelId, { tabs: rest, activeTabId: source.activeTabId });
  }

  const landed = setPanel(m, targetPanelId, withTab(target, found.tab, index ?? target.tabs.length, true));
  const drained = withoutTab(source, tabId);
  const next = drained ? setPanel(landed, found.panelId, drained) : closePane(landed, found.panelId);
  return { ...next, focusedId: targetPanelId };
}

/** Pulls a tab out of its panel into a new panel beside it. A panel holding
 * one tab has nothing to undock, so that is a same-reference no-op. */
export function undockTab<T>(m: DockModel<T>, tabId: string, zone: DockEdgeZone = "right"): DockModel<T> {
  const found = findDockTab(m, tabId);
  if (!found) return m;
  const source = findDockPanel(m, found.panelId)!;
  if (source.tabs.length < 2) return m;
  const drained = setPanel(m, found.panelId, withoutTab(source, tabId)!);
  const panelId = String(m.nextId);
  const leaf: PaneLeaf<DockPanel<T>> = { kind: "leaf", id: panelId, data: dockPanelOf([found.tab]) };
  const next = insertBeside(drained, found.panelId, zone, leaf);
  return { ...next, focusedId: panelId };
}

/** Relocates a whole panel against another one. `center` merges its tabs into
 * the target's stack; the four edges split the target and take that side.
 * `applyDockDrop` routes a `dropped` event here. */
export function dockPanel<T>(m: DockModel<T>, panelId: string, targetPanelId: string, zone: DockZone): DockModel<T> {
  if (panelId === targetPanelId) return m;
  const source = findDockPanel(m, panelId);
  const target = findDockPanel(m, targetPanelId);
  if (!source || !target) return m;

  if (zone === "center") {
    const merged: DockPanel<T> = {
      tabs: [...target.tabs, ...source.tabs],
      activeTabId: source.activeTabId,
    };
    const next = closePane(setPanel(m, targetPanelId, merged), panelId);
    return { ...next, focusedId: targetPanelId };
  }

  if (alreadyDocked(m, panelId, targetPanelId, zone)) return m;
  const removed = closePane(m, panelId);
  const leaf: PaneLeaf<DockPanel<T>> = { kind: "leaf", id: panelId, data: source };
  const next = insertBeside(removed, targetPanelId, zone, leaf);
  return { ...next, focusedId: panelId };
}

export interface DockRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Which zone a pointer at (px, py) lands in over `rect`. Pure geometry, so
 * it lives with the model rather than in whichever backend eventually reports
 * the drag: the outer `edge` fraction of each side is that side's zone, the
 * middle is `center`. A point past an edge still names that edge, and a
 * degenerate rect is all center. */
export function hitTestDockZone(rect: DockRect, px: number, py: number, edge = 0.25): DockZone {
  if (!(rect.width > 0) || !(rect.height > 0) || !Number.isFinite(px) || !Number.isFinite(py)) return "center";
  const band = Number.isFinite(edge) ? Math.min(0.5, Math.max(0.01, edge)) : 0.25;
  const u = (px - rect.x) / rect.width;
  const v = (py - rect.y) / rect.height;
  const distance: [DockEdgeZone, number][] = [
    ["left", u],
    ["right", 1 - u],
    ["top", v],
    ["bottom", 1 - v],
  ];
  let nearest = distance[0]!;
  for (const candidate of distance) {
    if (candidate[1] < nearest[1]) nearest = candidate;
  }
  return nearest[1] < band ? nearest[0] : "center";
}

// ============================================================================
// Drag and drop
// ============================================================================

/** Pixel extent of the whole dock surface. Widget drops report a point in the
 * TARGET panel's own coordinate space, and nothing on the wire says how big
 * that panel is, so the panel rects are derived here from the split ratios,
 * which are fractions of this. Feed it from whatever sizes the dock (a
 * window's sizeChanged, say). Without it every drop resolves to `center`. */
export interface DockSize {
  width: number;
  height: number;
}

export type DockDragKind = "tab" | "panel";

export interface DockDrag {
  kind: DockDragKind;
  id: string;
}

const DRAG_PREFIX = "nd-dock:";

/** The `dragPayload` a dock drag source carries. Namespaced because the drop
 * side also receives plain text dragged in from other applications (both
 * backends carry the payload as a system string type), and that must not be
 * read as a tab id. */
export function dockDragPayload(kind: DockDragKind, id: string): string {
  return `${DRAG_PREFIX}${kind}:${id}`;
}

/** The inverse, and the guard: anything not minted by dockDragPayload is
 * undefined, which every drop path treats as "not ours, ignore". */
export function parseDockDrag(payload: string): DockDrag | undefined {
  if (!payload.startsWith(DRAG_PREFIX)) return undefined;
  const rest = payload.slice(DRAG_PREFIX.length);
  const sep = rest.indexOf(":");
  if (sep <= 0) return undefined;
  const kind = rest.slice(0, sep);
  const id = rest.slice(sep + 1);
  if (id.length === 0 || (kind !== "tab" && kind !== "panel")) return undefined;
  return { kind, id };
}

/** Every panel's rect inside a dock of `size`, from the split ratios alone.
 * Approximate by exactly the divider thickness the platform draws between
 * two panes, which is a couple of pixels against a zone band of a quarter of
 * the panel. */
export function dockPanelRects<T>(m: DockModel<T>, size: DockSize): Map<string, DockRect> {
  const out = new Map<string, DockRect>();
  const width = Number.isFinite(size.width) ? Math.max(0, size.width) : 0;
  const height = Number.isFinite(size.height) ? Math.max(0, size.height) : 0;

  function walk(node: PaneNode<DockPanel<T>>, rect: DockRect): void {
    if (node.kind === "leaf") {
      out.set(node.id, rect);
      return;
    }
    const ratio = clampPaneRatio(node.ratio);
    if (node.orientation === "horizontal") {
      const lead = rect.width * ratio;
      walk(node.children[0], { x: rect.x, y: rect.y, width: lead, height: rect.height });
      walk(node.children[1], { x: rect.x + lead, y: rect.y, width: rect.width - lead, height: rect.height });
      return;
    }
    const lead = rect.height * ratio;
    walk(node.children[0], { x: rect.x, y: rect.y, width: rect.width, height: lead });
    walk(node.children[1], { x: rect.x, y: rect.y + lead, width: rect.width, height: rect.height - lead });
  }

  if (m.root) walk(m.root, { x: 0, y: 0, width, height });
  return out;
}

/** Zone a drop point over `panelId` lands in. `x`/`y` are the panel-local
 * coordinates a dragOver/dropped event reports, so the hit test runs against
 * a rect at the origin: only the panel's extent matters, not where it sits.
 * No size, or a panel that is not in the model, means `center`, which is the
 * one zone that needs no geometry. */
export function dockZoneAt<T>(
  m: DockModel<T>,
  panelId: string,
  x: number,
  y: number,
  size?: DockSize,
  edge?: number,
): DockZone {
  if (!size) return "center";
  const rect = dockPanelRects(m, size).get(panelId);
  if (!rect) return "center";
  return hitTestDockZone({ x: 0, y: 0, width: rect.width, height: rect.height }, x, y, edge);
}

/** Turns one drop into one model op, and it is the whole drop policy:
 *
 *  - a panel drag is `dockPanel`, merging into the target's tab stack on
 *    `center` and splitting the target on an edge;
 *  - a tab dropped on `center` is `moveTab` into the target's stack;
 *  - a tab dropped on an edge is `moveTab` followed by `undockTab`, which
 *    lands it in a new panel on that side. A tab that is the only one in its
 *    panel takes the `dockPanel` path instead: the panel IS that tab, so
 *    moving it keeps its id (and its already-docked no-op) rather than
 *    collapsing the panel and minting a new one.
 *
 * A drag that ends where it started returns the SAME reference, so a drop is
 * free to fire against a layout it does not change. */
export function applyDockDrop<T>(
  m: DockModel<T>,
  payload: string,
  targetPanelId: string,
  zone: DockZone,
): DockModel<T> {
  const drag = parseDockDrag(payload);
  if (!drag) return m;
  if (drag.kind === "panel") return dockPanel(m, drag.id, targetPanelId, zone);

  const found = findDockTab(m, drag.id);
  if (!found || !findDockPanel(m, targetPanelId)) return m;
  const source = findDockPanel(m, found.panelId)!;

  if (zone === "center") {
    return found.panelId === targetPanelId ? m : moveTab(m, drag.id, targetPanelId);
  }
  if (source.tabs.length === 1) return dockPanel(m, found.panelId, targetPanelId, zone);
  if (found.panelId === targetPanelId) return undockTab(m, drag.id, zone);
  return undockTab(moveTab(m, drag.id, targetPanelId), drag.id, zone);
}

/** JSON-safe deep copy of the layout. The model is already plain data, so the
 * copy is the point: a store flush that lands after further edits persists the
 * layout as it was when serialized. */
export function serializeDock<T>(m: DockModel<T>): DockModel<T> {
  function copy(node: PaneNode<DockPanel<T>> | undefined): PaneNode<DockPanel<T>> | undefined {
    if (!node) return undefined;
    if (node.kind === "leaf") {
      const data: DockPanel<T> = {
        tabs: node.data.tabs.map((tab) => ({ ...tab })),
        activeTabId: node.data.activeTabId,
      };
      return { kind: "leaf", id: node.id, data };
    }
    return {
      kind: "split",
      id: node.id,
      orientation: node.orientation,
      ratio: node.ratio,
      children: [copy(node.children[0])!, copy(node.children[1])!],
    };
  }
  return { root: copy(m.root), focusedId: m.focusedId, nextId: m.nextId };
}

function normalizePanel<T>(raw: unknown, isData?: (d: unknown) => d is T): DockPanel<T> | undefined {
  if (typeof raw !== "object" || raw === null) return undefined;
  const panel = raw as { tabs?: unknown; activeTabId?: unknown };
  if (!Array.isArray(panel.tabs) || panel.tabs.length === 0) return undefined;
  const tabs: DockTab<T>[] = [];
  for (const entry of panel.tabs) {
    if (typeof entry !== "object" || entry === null) return undefined;
    const tab = entry as { id?: unknown; title?: unknown; icon?: unknown; data?: unknown };
    if (typeof tab.id !== "string" || tab.id.length === 0) return undefined;
    if (typeof tab.title !== "string") return undefined;
    if (isData && !isData(tab.data)) return undefined;
    const next: DockTab<T> = { id: tab.id, title: tab.title, data: tab.data as T };
    if (typeof tab.icon === "string") next.icon = tab.icon;
    tabs.push(next);
  }
  const active = typeof panel.activeTabId === "string" ? panel.activeTabId : "";
  return { tabs, activeTabId: tabs.some((t) => t.id === active) ? active : tabs[0]!.id };
}

/** Rebuilds a dock from persisted unknown data, on migratePanes' terms:
 * anything malformed rejects the whole tree back to emptyDock(). A panel with
 * no tabs, a tab without an id, tab data failing `isData` and a tab id used
 * twice all count as malformed; an activeTabId naming a tab that is gone is
 * repaired to the panel's first tab instead. */
export function deserializeDock<T>(raw: unknown, isData?: (d: unknown) => d is T): DockModel<T> {
  const guard = (d: unknown): d is DockPanel<T> => normalizePanel(d, isData) !== undefined;
  const migrated = migratePanes<DockPanel<T>>(raw, guard);
  const seen = new Set<string>();
  let model = migrated;
  for (const leaf of paneLeaves(migrated)) {
    const panel = normalizePanel<T>(leaf.data, isData)!;
    for (const tab of panel.tabs) {
      if (seen.has(tab.id)) return emptyDock<T>();
      seen.add(tab.id);
    }
    model = updatePane(model, leaf.id, () => panel);
  }
  return model;
}
