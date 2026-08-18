import { describe, expect, test } from "bun:test";
import {
  applyTileDrop,
  deserializeTiles,
  emptyTiles,
  findTile,
  moveTile,
  parseTileDrag,
  placeTile,
  raiseTile,
  removeTile,
  resizeTile,
  seedTiles,
  serializeTiles,
  tileCellAt,
  tileDragPayload,
  tileRows,
  updateTile,
} from "./tiles.ts";
import type { TileModel } from "./tiles.ts";

const isString = (d: unknown): d is string => typeof d === "string";

/** Two 2x2 tiles side by side on a 4-column grid, plus one below them. */
function base(): TileModel<string> {
  return seedTiles(
    [
      { x: 0, y: 0, w: 2, h: 2, data: "a" },
      { x: 2, y: 0, w: 2, h: 2, data: "b" },
      { x: 0, y: 2, w: 4, h: 1, data: "c" },
    ],
    4,
  );
}

function boxes(m: TileModel<string>): string[] {
  return m.tiles.map((t) => `${t.id}@${t.x},${t.y} ${t.w}x${t.h}`);
}

describe("placeTile", () => {
  test("mints sequential ids and keeps what fits where it was asked for", () => {
    expect(boxes(base())).toEqual(["1@0,0 2x2", "2@2,0 2x2", "3@0,2 4x1"]);
    expect(base().nextId).toBe(4);
  });

  test("the new tile keeps its cells and pushes the occupant down", () => {
    const m = placeTile(base(), { x: 0, y: 0, w: 4, h: 1, data: "d" });
    expect(boxes(m)).toEqual(["1@0,1 2x2", "2@2,1 2x2", "3@0,3 4x1", "4@0,0 4x1"]);
  });

  test("cells are clamped to the grid and to whole rows and columns", () => {
    const m = placeTile(emptyTiles<string>(4), { x: 9, y: -3, w: 99, h: 0.4, data: "a" });
    expect(boxes(m)).toEqual(["1@0,0 4x1"]);
    expect(boxes(placeTile(emptyTiles<string>(4), { x: 3, y: 0, w: 2, h: 1, data: "a" }))).toEqual(["1@2,0 2x1"]);
    expect(boxes(placeTile(emptyTiles<string>(4), { x: NaN, y: NaN, w: NaN, h: NaN, data: "a" }))).toEqual([
      "1@0,0 1x1",
    ]);
  });

  test("an explicit id advances nextId past it; a duplicate id is a no-op", () => {
    const m = placeTile(emptyTiles<string>(4), { id: "7", x: 0, y: 0, w: 1, h: 1, data: "a" });
    expect(m.nextId).toBe(8);
    expect(placeTile(m, { id: "7", x: 2, y: 0, w: 1, h: 1, data: "b" })).toBe(m);
  });
});

describe("moveTile", () => {
  test("pushes the tiles it lands on down, cascading", () => {
    // "1" onto "2"'s cells: 2 goes below it, and c below that.
    const m = moveTile(base(), "1", 2, 0);
    expect(boxes(m)).toEqual(["1@2,0 2x2", "2@2,2 2x2", "3@0,4 4x1"]);
  });

  test("tiles that did not move keep their object identity", () => {
    const m = base();
    const moved = moveTile(m, "3", 0, 3);
    expect(moved.tiles[0]).toBe(m.tiles[0]);
    expect(moved.tiles[1]).toBe(m.tiles[1]);
    expect(moved.tiles[2]).not.toBe(m.tiles[2]);
  });

  test("no upward compaction: a hole stays a hole", () => {
    const m = removeTile(base(), "1");
    expect(boxes(m)).toEqual(["2@2,0 2x2", "3@0,2 4x1"]);
    expect(boxes(moveTile(m, "3", 0, 6))).toEqual(["2@2,0 2x2", "3@0,6 4x1"]);
  });

  test("same cells, unknown id and an out-of-range move that clamps back are same-reference", () => {
    const m = base();
    expect(moveTile(m, "1", 0, 0)).toBe(m);
    expect(moveTile(m, "1", -5, -5)).toBe(m);
    expect(moveTile(m, "zzz", 1, 1)).toBe(m);
  });
});

describe("resizeTile", () => {
  test("growing over a neighbour pushes it down", () => {
    const m = resizeTile(base(), "1", 4, 2);
    expect(boxes(m)).toEqual(["1@0,0 4x2", "2@2,2 2x2", "3@0,4 4x1"]);
  });

  test("a width past the grid edge pulls the tile back inside", () => {
    expect(boxes(resizeTile(base(), "2", 4, 2))).toEqual(["1@0,2 2x2", "2@0,0 4x2", "3@0,4 4x1"]);
  });

  test("same size and unknown id are same-reference", () => {
    const m = base();
    expect(resizeTile(m, "1", 2, 2)).toBe(m);
    expect(resizeTile(m, "1", 0.2, -4)).not.toBe(m); // clamps to 1x1, which is a change
    expect(resizeTile(m, "zzz", 1, 1)).toBe(m);
  });
});

describe("raiseTile", () => {
  test("moves the tile last in layout order without moving any cells", () => {
    const m = raiseTile(base(), "1");
    expect(m.tiles.map((t) => t.id)).toEqual(["2", "3", "1"]);
    expect(boxes(m).sort()).toEqual(boxes(base()).sort());
  });

  test("already last, or unknown, is same-reference", () => {
    const m = base();
    expect(raiseTile(m, "3")).toBe(m);
    expect(raiseTile(m, "zzz")).toBe(m);
  });
});

describe("removeTile / updateTile", () => {
  test("removeTile drops the tile; unknown id is same-reference", () => {
    const m = base();
    expect(removeTile(m, "2").tiles.map((t) => t.id)).toEqual(["1", "3"]);
    expect(removeTile(m, "zzz")).toBe(m);
  });

  test("updateTile replaces data; identical data is same-reference", () => {
    const m = base();
    const updated = updateTile(m, "1", (d) => `${d}!`);
    expect(findTile(updated, "1")!.data).toBe("a!");
    expect(updateTile(updated, "1", (d) => d)).toBe(updated);
    expect(updateTile(updated, "zzz", (d) => d)).toBe(updated);
  });
});

describe("serializeTiles / deserializeTiles", () => {
  test("round-trips a layout through JSON", () => {
    const m = raiseTile(moveTile(base(), "1", 2, 0), "3");
    const back = deserializeTiles<string>(JSON.parse(JSON.stringify(serializeTiles(m))), 4, isString);
    expect(back).toEqual(m);
  });

  test("serializeTiles is a deep copy, not a view of the live model", () => {
    const m = base();
    const snapshot = serializeTiles(m);
    expect(boxes(moveTile(m, "1", 2, 0))[0]).toBe("1@2,0 2x2");
    expect(findTile(snapshot, "1")).toEqual({ id: "1", x: 0, y: 0, w: 2, h: 2, data: "a" });
  });

  test("malformed input falls back to the empty layout on the given columns", () => {
    const empty = emptyTiles<string>(6);
    expect(deserializeTiles<string>(null, 6, isString)).toEqual(empty);
    expect(deserializeTiles<string>({ tiles: "nope" }, 6, isString)).toEqual(empty);
    expect(deserializeTiles<string>({ tiles: [{ x: 0, y: 0, w: 1, h: 1, data: "a" }] }, 6, isString)).toEqual(empty);
    expect(deserializeTiles<string>({ tiles: [{ id: "1", x: "0", y: 0, w: 1, h: 1, data: "a" }] }, 6, isString)).toEqual(
      empty,
    );
    expect(deserializeTiles<string>({ tiles: [{ id: "1", x: 0, y: 0, w: 1, h: 1, data: 42 }] }, 6, isString)).toEqual(
      empty,
    );
  });

  test("a duplicate id rejects the layout", () => {
    const raw = {
      columns: 4,
      tiles: [
        { id: "1", x: 0, y: 0, w: 1, h: 1, data: "a" },
        { id: "1", x: 1, y: 0, w: 1, h: 1, data: "b" },
      ],
      nextId: 2,
    };
    expect(deserializeTiles<string>(raw, 4, isString)).toEqual(emptyTiles<string>(4));
  });

  test("a hand-edited overlapping layout is pushed apart on load, and nextId lands past every id", () => {
    const raw = {
      columns: 4,
      tiles: [
        { id: "1", x: 0, y: 0, w: 2, h: 2, data: "a" },
        { id: "9", x: 0, y: 0, w: 2, h: 2, data: "b" },
      ],
      nextId: 1,
    };
    const m = deserializeTiles<string>(raw, 4, isString);
    expect(boxes(m)).toEqual(["1@0,0 2x2", "9@0,2 2x2"]);
    expect(m.nextId).toBe(10);
  });
});

describe("tile drag payloads", () => {
  test("round-trip, and anything foreign is not a tile drag", () => {
    expect(parseTileDrag(tileDragPayload("3"))).toBe("3");
    for (const foreign of ["", "3", "nd-tiles:tile:", "nd-dock:tab:3"]) {
      expect(parseTileDrag(foreign)).toBeUndefined();
    }
  });
});

describe("tileCellAt", () => {
  // base() occupies 4 columns and 3 rows, so a 400x300 grid is 100px cells.
  const size = { width: 400, height: 300 };

  test("a point maps to the cell it lands in", () => {
    const m = base();
    expect(tileRows(m)).toBe(3);
    expect(tileCellAt(m, size, 0, 0)).toEqual({ column: 0, row: 0 });
    expect(tileCellAt(m, size, 250, 150)).toEqual({ column: 2, row: 1 });
    expect(tileCellAt(m, size, 399, 299)).toEqual({ column: 3, row: 2 });
  });

  test("cell boundaries belong to the cell they open", () => {
    const m = base();
    expect(tileCellAt(m, size, 99, 99)).toEqual({ column: 0, row: 0 });
    expect(tileCellAt(m, size, 100, 100)).toEqual({ column: 1, row: 1 });
  });

  test("a point outside the grid clamps to its edge column, and a degenerate size is cell 0", () => {
    const m = base();
    expect(tileCellAt(m, size, -20, -20)).toEqual({ column: 0, row: 0 });
    expect(tileCellAt(m, size, 900, 150)).toEqual({ column: 3, row: 1 });
    expect(tileCellAt(m, { width: 0, height: 0 }, 40, 40)).toEqual({ column: 0, row: 0 });
    expect(tileCellAt(m, { width: Number.NaN, height: 300 }, 40, 40)).toEqual({ column: 0, row: 0 });
  });
});

describe("applyTileDrop", () => {
  const size = { width: 400, height: 300 };

  test("the cell under the pointer becomes the tile's top-left", () => {
    // Tile 3 (row 2, full width) dropped on column 2 of row 0: it takes those
    // cells and pushes tiles 1 and 2 down past it.
    const m = applyTileDrop(base(), tileDragPayload("3"), size, 250, 40);
    expect(boxes(m)).toEqual(["1@0,1 2x2", "2@2,1 2x2", "3@0,0 4x1"]);
  });

  test("a tile dropped inside a neighbour lands on that neighbour's origin cell", () => {
    const m = applyTileDrop(base(), tileDragPayload("1"), size, 250, 40);
    expect(findTile(m, "1")).toMatchObject({ x: 2, y: 0 });
  });

  test("a tile dropped back on the cell it already occupies is the same model", () => {
    const m = base();
    // Anywhere inside tile 2's own 2x2 block resolves to a cell it already
    // covers, and clamping lands it back on its own origin.
    expect(applyTileDrop(m, tileDragPayload("2"), size, 250, 40)).toBe(m);
    expect(applyTileDrop(m, tileDragPayload("2"), size, 210, 10)).toBe(m);
  });

  test("a foreign drag and an unknown tile are ignored", () => {
    const m = base();
    expect(applyTileDrop(m, "https://example.com", size, 250, 40)).toBe(m);
    expect(applyTileDrop(m, tileDragPayload("gone"), size, 250, 40)).toBe(m);
  });
});
