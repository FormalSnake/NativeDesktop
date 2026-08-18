// Tiles model: freeform panels on a column grid, rendered through the
// existing <grid> widget (x/y/w/h map straight onto the gridColumn/gridRow/
// gridColumnSpan/gridRowSpan attached props). No schema change, no ABI change.
//
// Collision handling is the standard grid-layout one: the tile you just
// touched keeps the cells it asked for and every tile it overlaps is pushed
// down until it clears, cascading. There is no upward compaction, so a tile
// left at row 8 stays at row 8 and the layout is exactly what the caller
// placed.
//
// Same invariant as model.ts: an op that changes nothing returns the SAME
// reference, and every tile the op did not move keeps its object identity so
// a re-render diffs down to the tiles that actually moved.

export interface Tile<T> {
  id: string;
  /** Column of the tile's leading edge, 0-based. */
  x: number;
  /** Row of the tile's top edge, 0-based. */
  y: number;
  /** Width in columns, at least 1. */
  w: number;
  /** Height in rows, at least 1. */
  h: number;
  data: T;
}

export interface TileModel<T> {
  /** Column count the model clamps against. The native grid still sizes its
   * columns from the children, so this bounds placement, not pixels. */
  columns: number;
  /** Layout order, which is also paint order: later tiles draw over earlier
   * ones where the platform lets cells overlap. `raiseTile` moves one last. */
  tiles: Tile<T>[];
  nextId: number;
}

export interface TilePlacement<T> {
  id?: string;
  x: number;
  y: number;
  w: number;
  h: number;
  data: T;
}

/** Grid geometry is integer cells, and a non-finite span would panic a
 * Debug/ReleaseSafe host on the attached-prop write, so this is a safety
 * boundary like clampPaneRatio: everything rounds to an integer, spans floor
 * at 1, and a tile is kept inside the column count. */
function clampColumns(columns: number): number {
  if (!Number.isFinite(columns)) return 1;
  return Math.max(1, Math.trunc(columns));
}

function cell(value: number, fallback: number): number {
  return Number.isFinite(value) ? Math.round(value) : fallback;
}

function clampTile<T>(tile: Tile<T>, columns: number): Tile<T> {
  const w = Math.min(columns, Math.max(1, cell(tile.w, 1)));
  const h = Math.max(1, cell(tile.h, 1));
  const x = Math.min(columns - w, Math.max(0, cell(tile.x, 0)));
  const y = Math.max(0, cell(tile.y, 0));
  if (x === tile.x && y === tile.y && w === tile.w && h === tile.h) return tile;
  return { ...tile, x, y, w, h };
}

function overlaps<T>(a: Tile<T>, b: Tile<T>): boolean {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
}

/** Pushes every tile that collides with an already-placed one down past it.
 * `anchorId` is placed first, so the tile the caller just moved or resized is
 * the one that wins its cells. Terminates because each push strictly
 * increases a tile's y and heights are at least 1. */
function resolveOverlaps<T>(tiles: readonly Tile<T>[], anchorId: string): Tile<T>[] {
  const order = tiles.map((tile, index) => ({ tile, index }));
  order.sort((a, b) => a.tile.y - b.tile.y || a.tile.x - b.tile.x || a.index - b.index);
  const anchor = order.findIndex((entry) => entry.tile.id === anchorId);
  if (anchor > 0) order.unshift(order.splice(anchor, 1)[0]!);

  const placed: Tile<T>[] = [];
  const out = tiles.slice();
  for (const entry of order) {
    let current = entry.tile;
    for (;;) {
      const hit = placed.find((p) => overlaps(p, current));
      if (!hit) break;
      current = { ...current, y: hit.y + hit.h };
    }
    placed.push(current);
    out[entry.index] = current;
  }
  return out;
}

/** The model, or the one it was given when resolution moved nothing. */
function commit<T>(m: TileModel<T>, tiles: Tile<T>[], nextId = m.nextId): TileModel<T> {
  const changed = tiles.length !== m.tiles.length || tiles.some((tile, i) => tile !== m.tiles[i]);
  if (!changed && nextId === m.nextId) return m;
  return { columns: m.columns, tiles, nextId };
}

function numericId(id: string): number {
  const match = /^(\d+)$/.exec(id);
  return match ? Number(match[1]) : 0;
}

export function emptyTiles<T>(columns: number): TileModel<T> {
  return { columns: clampColumns(columns), tiles: [], nextId: 1 };
}

/** Places `items` in order, each one dropped at its requested cell and pushed
 * clear of what is already down. Ids default to "1".."n". */
export function seedTiles<T>(items: readonly TilePlacement<T>[], columns: number): TileModel<T> {
  let model = emptyTiles<T>(columns);
  for (const item of items) model = placeTile(model, item);
  return model;
}

export function findTile<T>(m: TileModel<T>, id: string): Tile<T> | undefined {
  return m.tiles.find((tile) => tile.id === id);
}

/** Adds a tile at the cells it asks for, pushing whatever was there down. An
 * id already in the model is rejected (same reference), the way addTab
 * rejects a duplicate tab id. */
export function placeTile<T>(m: TileModel<T>, placement: TilePlacement<T>): TileModel<T> {
  const id = placement.id ?? String(m.nextId);
  if (findTile(m, id)) return m;
  const tile = clampTile({ id, x: placement.x, y: placement.y, w: placement.w, h: placement.h, data: placement.data }, m.columns);
  const tiles = resolveOverlaps([...m.tiles, tile], id);
  return commit(m, tiles, Math.max(m.nextId + (placement.id === undefined ? 1 : 0), numericId(id) + 1));
}

export function removeTile<T>(m: TileModel<T>, id: string): TileModel<T> {
  if (!findTile(m, id)) return m;
  return commit(m, m.tiles.filter((tile) => tile.id !== id));
}

export function moveTile<T>(m: TileModel<T>, id: string, x: number, y: number): TileModel<T> {
  const tile = findTile(m, id);
  if (!tile) return m;
  const moved = clampTile({ ...tile, x, y }, m.columns);
  if (moved.x === tile.x && moved.y === tile.y) return m;
  return commit(m, resolveOverlaps(m.tiles.map((t) => (t.id === id ? moved : t)), id));
}

export function resizeTile<T>(m: TileModel<T>, id: string, w: number, h: number): TileModel<T> {
  const tile = findTile(m, id);
  if (!tile) return m;
  const sized = clampTile({ ...tile, w, h }, m.columns);
  if (sized.w === tile.w && sized.h === tile.h && sized.x === tile.x) return m;
  return commit(m, resolveOverlaps(m.tiles.map((t) => (t.id === id ? sized : t)), id));
}

/** Moves a tile last in layout order, which is the top of the paint order. */
export function raiseTile<T>(m: TileModel<T>, id: string): TileModel<T> {
  const index = m.tiles.findIndex((tile) => tile.id === id);
  if (index < 0 || index === m.tiles.length - 1) return m;
  const tiles = m.tiles.slice();
  tiles.push(tiles.splice(index, 1)[0]!);
  return { columns: m.columns, tiles, nextId: m.nextId };
}

export function updateTile<T>(m: TileModel<T>, id: string, fn: (data: T) => T): TileModel<T> {
  const tile = findTile(m, id);
  if (!tile) return m;
  const data = fn(tile.data);
  if (Object.is(data, tile.data)) return m;
  return commit(m, m.tiles.map((t) => (t.id === id ? { ...t, data } : t)));
}

// ============================================================================
// Drag and drop
// ============================================================================

/** Pixel extent of the grid the tiles are laid out in. A drop reports a point
 * in the grid's own coordinate space and the native grid never reports its
 * geometry, so this is what turns that point back into a cell. Feed it from
 * whatever sizes the grid. Without it a drop is ignored. */
export interface TileSize {
  width: number;
  height: number;
}

export interface TileCell {
  column: number;
  row: number;
}

const DRAG_PREFIX = "nd-tiles:tile:";

/** The `dragPayload` a tile carries, namespaced for the same reason
 * dockDragPayload is: text dragged in from another application arrives on the
 * same drop handler and must not be read as a tile id. */
export function tileDragPayload(id: string): string {
  return `${DRAG_PREFIX}${id}`;
}

export function parseTileDrag(payload: string): string | undefined {
  if (!payload.startsWith(DRAG_PREFIX)) return undefined;
  const id = payload.slice(DRAG_PREFIX.length);
  return id.length > 0 ? id : undefined;
}

/** Row count the layout currently occupies, at least 1. The native grid sizes
 * its rows from their children, so this is what a uniform-cell mapping has to
 * divide by. */
export function tileRows<T>(m: TileModel<T>): number {
  let rows = 1;
  for (const tile of m.tiles) rows = Math.max(rows, tile.y + tile.h);
  return rows;
}

/** Cell under a point in the grid's coordinate space. Cells are taken as
 * uniform, which is what the grid draws when every tile expands; a layout of
 * mixed intrinsic heights drifts from this by whatever the row heights
 * differ. Points outside the grid clamp to its edge cells. */
export function tileCellAt<T>(m: TileModel<T>, size: TileSize, x: number, y: number): TileCell {
  const columns = Math.max(1, m.columns);
  const rows = tileRows(m);
  const width = Number.isFinite(size.width) ? size.width : 0;
  const height = Number.isFinite(size.height) ? size.height : 0;
  const px = Number.isFinite(x) ? x : 0;
  const py = Number.isFinite(y) ? y : 0;
  const column = width > 0 ? Math.floor((px / width) * columns) : 0;
  const row = height > 0 ? Math.floor((py / height) * rows) : 0;
  return {
    column: Math.min(columns - 1, Math.max(0, column)),
    row: Math.max(0, row),
  };
}

/** Turns one drop into one model op: the cell under the pointer becomes the
 * dragged tile's top-left, and `moveTile` pushes whatever it lands on clear.
 * A tile dropped back on the cell it already occupies returns the SAME
 * reference, as does a payload from anything but a tile. */
export function applyTileDrop<T>(m: TileModel<T>, payload: string, size: TileSize, x: number, y: number): TileModel<T> {
  const id = parseTileDrag(payload);
  if (id === undefined || !findTile(m, id)) return m;
  const cell = tileCellAt(m, size, x, y);
  return moveTile(m, id, cell.column, cell.row);
}

/** JSON-safe deep copy, same contract as serializeDock. */
export function serializeTiles<T>(m: TileModel<T>): TileModel<T> {
  return { columns: m.columns, tiles: m.tiles.map((tile) => ({ ...tile })), nextId: m.nextId };
}

/** Rebuilds a tile layout from persisted unknown data. Anything malformed (a
 * missing id, a non-numeric cell, a duplicate id, data failing `isData`)
 * rejects the whole layout back to emptyTiles(columns); `columns` is also the
 * fallback when the persisted column count is unusable. Loaded tiles are
 * clamped and pushed clear of each other, so a hand-edited file cannot land
 * an overlapping layout. */
export function deserializeTiles<T>(raw: unknown, columns: number, isData?: (d: unknown) => d is T): TileModel<T> {
  const fallback = emptyTiles<T>(columns);
  if (typeof raw !== "object" || raw === null) return fallback;
  const candidate = raw as { columns?: unknown; tiles?: unknown; nextId?: unknown };
  if (!Array.isArray(candidate.tiles)) return fallback;
  const width = typeof candidate.columns === "number" ? clampColumns(candidate.columns) : fallback.columns;
  const seen = new Set<string>();
  const tiles: Tile<T>[] = [];
  let maxNumericId = 0;
  for (const entry of candidate.tiles) {
    if (typeof entry !== "object" || entry === null) return fallback;
    const t = entry as { id?: unknown; x?: unknown; y?: unknown; w?: unknown; h?: unknown; data?: unknown };
    if (typeof t.id !== "string" || t.id.length === 0 || seen.has(t.id)) return fallback;
    if (typeof t.x !== "number" || typeof t.y !== "number" || typeof t.w !== "number" || typeof t.h !== "number") {
      return fallback;
    }
    if (isData && !isData(t.data)) return fallback;
    seen.add(t.id);
    maxNumericId = Math.max(maxNumericId, numericId(t.id));
    tiles.push(clampTile({ id: t.id, x: t.x, y: t.y, w: t.w, h: t.h, data: t.data as T }, width));
  }
  const rawNextId = typeof candidate.nextId === "number" && Number.isInteger(candidate.nextId) ? candidate.nextId : 1;
  return {
    columns: width,
    tiles: tiles.length > 0 ? resolveOverlaps(tiles, tiles[0]!.id) : tiles,
    nextId: Math.max(rawNextId, maxNumericId + 1, 1),
  };
}
