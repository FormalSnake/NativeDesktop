// Pure split-pane tree model over the existing <paned> widget: no schema
// change, no new widget, no ABI change. Every op returns the SAME reference
// when nothing changed (unknown id, already focused, same clamped ratio),
// which is what stops a native positionChanged echo from looping through a
// render+persist cycle.

export type SplitOrientation = "horizontal" | "vertical";

export interface PaneLeaf<T> {
  kind: "leaf";
  id: string;
  data: T;
}

export interface PaneSplit<T> {
  kind: "split";
  id: string;
  orientation: SplitOrientation;
  ratio: number;
  children: [PaneNode<T>, PaneNode<T>];
}

export type PaneNode<T> = PaneLeaf<T> | PaneSplit<T>;

export interface PaneModel<T> {
  root: PaneNode<T> | undefined;
  focusedId: string;
  nextId: number;
}

/** Non-finite ratios panic a Debug/ReleaseSafe host, so this is a safety
 * boundary, not a nicety: non-finite becomes 0.5, everything else clamps to
 * [0.05, 0.95] so neither side collapses to an undraggable sliver. */
export function clampPaneRatio(ratio: number): number {
  if (!Number.isFinite(ratio)) return 0.5;
  return Math.min(0.95, Math.max(0.05, ratio));
}

export function emptyPanes<T>(): PaneModel<T> {
  return { root: undefined, focusedId: "", nextId: 1 };
}

/** Builds a balanced horizontal-split tree over `leaves` (even column widths
 * via per-split ratios). Leaf ids default to "1".."n". */
export function seedPanes<T>(leaves: readonly T[], id?: (data: T, i: number) => string): PaneModel<T> {
  if (leaves.length === 0) return emptyPanes<T>();
  const leafNodes: PaneLeaf<T>[] = leaves.map((data, i) => ({
    kind: "leaf",
    id: id ? id(data, i) : String(i + 1),
    data,
  }));
  let counter = leaves.length + 1;
  function build(lo: number, hi: number): PaneNode<T> {
    if (hi - lo === 1) return leafNodes[lo]!;
    const mid = lo + Math.ceil((hi - lo) / 2);
    const splitId = `s${counter++}`;
    return {
      kind: "split",
      id: splitId,
      orientation: "horizontal",
      ratio: (mid - lo) / (hi - lo),
      children: [build(lo, mid), build(mid, hi)],
    };
  }
  const root = build(0, leafNodes.length);
  return { root, focusedId: leafNodes[0]!.id, nextId: counter };
}

function findNode<T>(node: PaneNode<T> | undefined, id: string): PaneNode<T> | undefined {
  if (!node) return undefined;
  if (node.id === id) return node;
  if (node.kind === "split") return findNode(node.children[0], id) ?? findNode(node.children[1], id);
  return undefined;
}

function firstLeaf<T>(node: PaneNode<T>): PaneLeaf<T> {
  return node.kind === "leaf" ? node : firstLeaf(node.children[0]);
}

/** Path-copies the subtree containing `id`, applying `fn` to that node.
 * Returns undefined when `id` is not in the tree. */
function mapNode<T>(node: PaneNode<T>, id: string, fn: (node: PaneNode<T>) => PaneNode<T>): PaneNode<T> | undefined {
  if (node.id === id) return fn(node);
  if (node.kind === "leaf") return undefined;
  const [a, b] = node.children;
  const ma = mapNode(a, id, fn);
  if (ma) return { ...node, children: [ma, b] };
  const mb = mapNode(b, id, fn);
  if (mb) return { ...node, children: [a, mb] };
  return undefined;
}

/** Splits the leaf `paneId` in two: the existing leaf keeps side 0, the new
 * leaf (holding `data`) takes side 1 and the focus. */
export function splitPane<T>(
  m: PaneModel<T>,
  paneId: string,
  orientation: SplitOrientation,
  data: T,
  id?: string,
): PaneModel<T> {
  const target = findNode(m.root, paneId);
  if (!target || target.kind !== "leaf") return m;
  const newLeafId = id ?? String(m.nextId);
  const newLeaf: PaneLeaf<T> = { kind: "leaf", id: newLeafId, data };
  const root = mapNode(m.root!, paneId, (leaf) => ({
    kind: "split",
    id: `s${m.nextId}`,
    orientation,
    ratio: 0.5,
    children: [leaf, newLeaf],
  }));
  return { root, focusedId: newLeafId, nextId: m.nextId + 1 };
}

/** Removes the leaf `paneId`; its sibling subtree replaces the parent split.
 * A focused close moves the focus to the sibling's first leaf. */
export function closePane<T>(m: PaneModel<T>, paneId: string): PaneModel<T> {
  if (!m.root) return m;
  let sibling: PaneNode<T> | undefined;
  // false: not in this subtree; null: this whole node was the closed leaf.
  function removeFrom(node: PaneNode<T>): PaneNode<T> | null | false {
    if (node.kind === "leaf") return node.id === paneId ? null : false;
    const [a, b] = node.children;
    const ra = removeFrom(a);
    if (ra === null) {
      sibling = b;
      return b;
    }
    if (ra !== false) return { ...node, children: [ra, b] };
    const rb = removeFrom(b);
    if (rb === null) {
      sibling = a;
      return a;
    }
    if (rb !== false) return { ...node, children: [a, rb] };
    return false;
  }
  const removed = removeFrom(m.root);
  if (removed === false) return m;
  if (removed === null) return { root: undefined, focusedId: "", nextId: m.nextId };
  const focusedId = m.focusedId === paneId ? firstLeaf(sibling!).id : m.focusedId;
  return { root: removed, focusedId, nextId: m.nextId };
}

export function focusPane<T>(m: PaneModel<T>, paneId: string): PaneModel<T> {
  if (m.focusedId === paneId) return m;
  const node = findNode(m.root, paneId);
  if (!node || node.kind !== "leaf") return m;
  return { ...m, focusedId: paneId };
}

export function focusPaneAt<T>(m: PaneModel<T>, index: number): PaneModel<T> {
  const id = paneIdAt(m, index);
  return id === undefined ? m : focusPane(m, id);
}

export function paneIdAt<T>(m: PaneModel<T>, index: number): string | undefined {
  return paneLeaves(m)[index]?.id;
}

/** Ancestor-walking directional focus: exact for a binary tree, no layout
 * round-trip. Finds the nearest ancestor split on the move's axis where the
 * focused subtree sits on the departing side, then descends into the other
 * child along the shared edge. */
export function focusNeighbor<T>(m: PaneModel<T>, dir: "left" | "right" | "up" | "down"): PaneModel<T> {
  if (!m.root || !m.focusedId) return m;
  const path: PaneNode<T>[] = [];
  function collect(node: PaneNode<T>): boolean {
    path.push(node);
    if (node.id === m.focusedId) return true;
    if (node.kind === "split" && (collect(node.children[0]) || collect(node.children[1]))) return true;
    path.pop();
    return false;
  }
  if (!collect(m.root)) return m;
  const axis: SplitOrientation = dir === "left" || dir === "right" ? "horizontal" : "vertical";
  const forward = dir === "right" || dir === "down";
  for (let i = path.length - 2; i >= 0; i--) {
    const split = path[i] as PaneSplit<T>;
    const childIndex = split.children[0] === path[i + 1] ? 0 : 1;
    if (split.orientation !== axis || childIndex !== (forward ? 0 : 1)) continue;
    let target = split.children[forward ? 1 : 0];
    while (target.kind === "split") {
      target = target.orientation === axis ? target.children[forward ? 0 : 1] : target.children[0];
    }
    return focusPane(m, target.id);
  }
  return m;
}

export function setPaneRatio<T>(m: PaneModel<T>, splitId: string, ratio: number): PaneModel<T> {
  const node = findNode(m.root, splitId);
  if (!node || node.kind !== "split") return m;
  const clamped = clampPaneRatio(ratio);
  if (clamped === node.ratio) return m;
  const root = mapNode(m.root!, splitId, (split) => ({ ...(split as PaneSplit<T>), ratio: clamped }));
  return { ...m, root };
}

export function updatePane<T>(m: PaneModel<T>, paneId: string, fn: (d: T) => T): PaneModel<T> {
  const node = findNode(m.root, paneId);
  if (!node || node.kind !== "leaf") return m;
  const next = fn(node.data);
  if (Object.is(next, node.data)) return m;
  const root = mapNode(m.root!, paneId, (leaf) => ({ ...(leaf as PaneLeaf<T>), data: next }));
  return { ...m, root };
}

/** Leaves in layout order (side 0 before side 1, depth-first). */
export function paneLeaves<T>(m: PaneModel<T>): PaneLeaf<T>[] {
  const out: PaneLeaf<T>[] = [];
  function walk(node: PaneNode<T> | undefined): void {
    if (!node) return;
    if (node.kind === "leaf") out.push(node);
    else {
      walk(node.children[0]);
      walk(node.children[1]);
    }
  }
  walk(m.root);
  return out;
}

/** Structural equality: ids, kinds and orientations, ignoring ratios and leaf
 * data. This is the "structural change" test for a persist-on-close flush. */
export function samePaneShape<T>(a: PaneNode<T> | undefined, b: PaneNode<T> | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  if (a.kind !== b.kind || a.id !== b.id) return false;
  if (a.kind === "leaf" || b.kind === "leaf") return true;
  return (
    a.orientation === b.orientation &&
    samePaneShape(a.children[0], b.children[0]) &&
    samePaneShape(a.children[1], b.children[1])
  );
}

/** Rebuilds a model from persisted unknown data. Anything malformed (wrong
 * kinds, missing ids, non-binary children, data failing `isData`) rejects the
 * whole tree back to emptyPanes(); ratios are clamped, focus falls back to
 * the first leaf, and nextId is recomputed past every numeric id. */
export function migratePanes<T>(raw: unknown, isData?: (d: unknown) => d is T): PaneModel<T> {
  if (typeof raw !== "object" || raw === null) return emptyPanes<T>();
  const candidate = raw as { root?: unknown; focusedId?: unknown; nextId?: unknown };
  let maxNumericId = 0;
  function noteId(id: string): void {
    const match = /^s?(\d+)$/.exec(id);
    if (match) maxNumericId = Math.max(maxNumericId, Number(match[1]));
  }
  function node(v: unknown): PaneNode<T> | null {
    if (typeof v !== "object" || v === null) return null;
    const n = v as { kind?: unknown; id?: unknown };
    if (typeof n.id !== "string" || n.id.length === 0) return null;
    if (n.kind === "leaf") {
      const data = (v as { data?: unknown }).data;
      if (isData && !isData(data)) return null;
      noteId(n.id);
      return { kind: "leaf", id: n.id, data: data as T };
    }
    if (n.kind === "split") {
      const s = v as { orientation?: unknown; ratio?: unknown; children?: unknown };
      if (s.orientation !== "horizontal" && s.orientation !== "vertical") return null;
      if (!Array.isArray(s.children) || s.children.length !== 2) return null;
      const a = node(s.children[0]);
      const b = node(s.children[1]);
      if (!a || !b) return null;
      noteId(n.id);
      return {
        kind: "split",
        id: n.id,
        orientation: s.orientation,
        ratio: clampPaneRatio(typeof s.ratio === "number" ? s.ratio : 0.5),
        children: [a, b],
      };
    }
    return null;
  }
  const root = candidate.root === undefined || candidate.root === null ? undefined : node(candidate.root);
  if (root === null) return emptyPanes<T>();
  const model: PaneModel<T> = { root, focusedId: "", nextId: 1 };
  const leaves = paneLeaves(model);
  if (leaves.length > 0) {
    const focused = typeof candidate.focusedId === "string" ? candidate.focusedId : "";
    model.focusedId = leaves.some((l) => l.id === focused) ? focused : leaves[0]!.id;
  }
  const rawNextId = typeof candidate.nextId === "number" && Number.isInteger(candidate.nextId) ? candidate.nextId : 1;
  model.nextId = Math.max(rawNextId, maxNumericId + 1, 1);
  return model;
}
