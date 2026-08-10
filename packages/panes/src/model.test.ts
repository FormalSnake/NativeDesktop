import { describe, expect, test } from "bun:test";
import {
  clampPaneRatio,
  closePane,
  emptyPanes,
  focusNeighbor,
  focusPane,
  focusPaneAt,
  migratePanes,
  paneIdAt,
  paneLeaves,
  samePaneShape,
  seedPanes,
  setPaneRatio,
  splitPane,
  updatePane,
} from "./model.ts";
import type { PaneModel, PaneSplit } from "./model.ts";

describe("clampPaneRatio", () => {
  test("non-finite becomes 0.5, everything else clamps to [0.05, 0.95]", () => {
    expect(clampPaneRatio(NaN)).toBe(0.5);
    expect(clampPaneRatio(Infinity)).toBe(0.5);
    expect(clampPaneRatio(-Infinity)).toBe(0.5);
    expect(clampPaneRatio(-1)).toBe(0.05);
    expect(clampPaneRatio(0)).toBe(0.05);
    expect(clampPaneRatio(0.3)).toBe(0.3);
    expect(clampPaneRatio(2)).toBe(0.95);
  });
});

describe("seedPanes", () => {
  test("empty input is the empty model", () => {
    expect(seedPanes([])).toEqual(emptyPanes());
  });

  test("one leaf: root is that leaf, focused, nextId past it", () => {
    const m = seedPanes(["a"]);
    expect(m.root).toEqual({ kind: "leaf", id: "1", data: "a" });
    expect(m.focusedId).toBe("1");
    expect(m.nextId).toBe(2);
  });

  test("three leaves build a binary tree with all leaves in order", () => {
    const m = seedPanes(["a", "b", "c"]);
    expect(paneLeaves(m).map((l) => l.data)).toEqual(["a", "b", "c"]);
    expect(m.root!.kind).toBe("split");
  });

  test("custom id function is used", () => {
    const m = seedPanes(["a", "b"], (_d, i) => `pane-${i}`);
    expect(paneLeaves(m).map((l) => l.id)).toEqual(["pane-0", "pane-1"]);
  });
});

describe("splitPane", () => {
  test("splits a leaf, focuses the new leaf, mints ids from nextId", () => {
    const m0 = seedPanes(["a"]);
    const m1 = splitPane(m0, "1", "horizontal", "b");
    const root = m1.root as PaneSplit<string>;
    expect(root.kind).toBe("split");
    expect(root.id).toBe("s2");
    expect(root.orientation).toBe("horizontal");
    expect(root.ratio).toBe(0.5);
    expect(paneLeaves(m1).map((l) => l.id)).toEqual(["1", "2"]);
    expect(m1.focusedId).toBe("2");
    expect(m1.nextId).toBe(3);
  });

  test("unknown pane id is a same-reference no-op", () => {
    const m = seedPanes(["a"]);
    expect(splitPane(m, "nope", "vertical", "b")).toBe(m);
  });

  test("splitting a split id is a same-reference no-op", () => {
    const m = splitPane(seedPanes(["a"]), "1", "horizontal", "b");
    expect(splitPane(m, "s2", "vertical", "c")).toBe(m);
  });
});

describe("closePane", () => {
  test("closing one of two leaves collapses the split to the sibling", () => {
    const m1 = splitPane(seedPanes(["a"]), "1", "horizontal", "b");
    const m2 = closePane(m1, "2");
    expect(m2.root).toEqual({ kind: "leaf", id: "1", data: "a" });
    expect(m2.focusedId).toBe("1");
  });

  test("closing an unfocused leaf keeps the focus", () => {
    const m1 = splitPane(seedPanes(["a"]), "1", "horizontal", "b"); // focus 2
    const m2 = closePane(m1, "1");
    expect(m2.focusedId).toBe("2");
  });

  test("closing the root leaf empties the tree", () => {
    const m = closePane(seedPanes(["a"]), "1");
    expect(m.root).toBeUndefined();
    expect(m.focusedId).toBe("");
  });

  test("unknown id is a same-reference no-op", () => {
    const m = seedPanes(["a"]);
    expect(closePane(m, "9")).toBe(m);
  });

  test("nested close replaces the parent split with the sibling subtree", () => {
    let m = splitPane(seedPanes(["a"]), "1", "horizontal", "b"); // s2[1, 2]
    m = splitPane(m, "2", "vertical", "c"); // s2[1, s3[2, 3]]
    const closed = closePane(m, "3");
    const root = closed.root as PaneSplit<string>;
    expect(root.id).toBe("s2");
    expect(paneLeaves(closed).map((l) => l.id)).toEqual(["1", "2"]);
    expect(closed.focusedId).toBe("2"); // 3 was focused; sibling takes over
  });
});

describe("focus", () => {
  test("focusPane moves focus; already-focused is a same-reference no-op", () => {
    const m1 = splitPane(seedPanes(["a"]), "1", "horizontal", "b");
    const m2 = focusPane(m1, "1");
    expect(m2.focusedId).toBe("1");
    expect(focusPane(m2, "1")).toBe(m2);
  });

  test("focusPane on an unknown or split id is a no-op", () => {
    const m = splitPane(seedPanes(["a"]), "1", "horizontal", "b");
    expect(focusPane(m, "s2")).toBe(m);
    expect(focusPane(m, "zzz")).toBe(m);
  });

  test("focusPaneAt / paneIdAt index leaves in layout order", () => {
    let m = splitPane(seedPanes(["a"]), "1", "horizontal", "b");
    m = splitPane(m, "2", "vertical", "c");
    expect(paneIdAt(m, 0)).toBe("1");
    expect(paneIdAt(m, 2)).toBe("3");
    expect(paneIdAt(m, 3)).toBeUndefined();
    expect(focusPaneAt(m, 0).focusedId).toBe("1");
    expect(focusPaneAt(m, 9)).toBe(m);
  });

  test("focusNeighbor walks across the matching-axis split", () => {
    // s2 horizontal [1, s3 vertical [2, 3]]
    let m = splitPane(seedPanes(["a"]), "1", "horizontal", "b");
    m = splitPane(m, "2", "vertical", "c");
    m = focusPane(m, "1");
    const right = focusNeighbor(m, "right");
    expect(right.focusedId).toBe("2"); // nearest leaf along the shared edge
    const down = focusNeighbor(right, "down");
    expect(down.focusedId).toBe("3");
    const up = focusNeighbor(down, "up");
    expect(up.focusedId).toBe("2");
    const left = focusNeighbor(up, "left");
    expect(left.focusedId).toBe("1");
  });

  test("focusNeighbor with no neighbor on that side is a same-reference no-op", () => {
    const m = splitPane(seedPanes(["a"]), "1", "horizontal", "b"); // focus 2
    expect(focusNeighbor(m, "right")).toBe(m);
    expect(focusNeighbor(m, "up")).toBe(m);
  });
});

describe("setPaneRatio", () => {
  test("sets the clamped ratio on the split", () => {
    const m1 = splitPane(seedPanes(["a"]), "1", "horizontal", "b");
    const m2 = setPaneRatio(m1, "s2", 0.3);
    expect((m2.root as PaneSplit<string>).ratio).toBe(0.3);
    const m3 = setPaneRatio(m2, "s2", 99);
    expect((m3.root as PaneSplit<string>).ratio).toBe(0.95);
  });

  test("same clamped ratio is a same-reference no-op (echo guard)", () => {
    const m1 = splitPane(seedPanes(["a"]), "1", "horizontal", "b");
    const m2 = setPaneRatio(m1, "s2", 0.3);
    expect(setPaneRatio(m2, "s2", 0.3)).toBe(m2);
    expect(setPaneRatio(m2, "s2", NaN)).not.toBe(m2); // NaN clamps to 0.5
  });

  test("unknown or leaf id is a same-reference no-op", () => {
    const m = splitPane(seedPanes(["a"]), "1", "horizontal", "b");
    expect(setPaneRatio(m, "1", 0.3)).toBe(m);
    expect(setPaneRatio(m, "zzz", 0.3)).toBe(m);
  });
});

describe("updatePane", () => {
  test("replaces leaf data; identical data is a same-reference no-op", () => {
    const m1 = seedPanes([{ n: 1 }]);
    const m2 = updatePane(m1, "1", (d) => ({ ...d, n: 2 }));
    expect(paneLeaves(m2)[0]!.data).toEqual({ n: 2 });
    expect(updatePane(m2, "1", (d) => d)).toBe(m2);
    expect(updatePane(m2, "zzz", (d) => d)).toBe(m2);
  });
});

describe("samePaneShape", () => {
  test("ratio and data changes keep the shape; structure changes break it", () => {
    const m1 = splitPane(seedPanes(["a"]), "1", "horizontal", "b");
    const ratioChanged = setPaneRatio(m1, "s2", 0.2);
    const dataChanged = updatePane(m1, "1", () => "z");
    expect(samePaneShape(m1.root, ratioChanged.root)).toBe(true);
    expect(samePaneShape(m1.root, dataChanged.root)).toBe(true);
    expect(samePaneShape(m1.root, splitPane(m1, "2", "vertical", "c").root)).toBe(false);
    expect(samePaneShape(m1.root, closePane(m1, "2").root)).toBe(false);
    expect(samePaneShape(undefined, undefined)).toBe(true);
    expect(samePaneShape(m1.root, undefined)).toBe(false);
  });
});

describe("migratePanes", () => {
  test("round-trips a real model through JSON", () => {
    let m = splitPane(seedPanes(["a"]), "1", "horizontal", "b");
    m = splitPane(m, "2", "vertical", "c");
    m = setPaneRatio(m, "s2", 0.3);
    const revived = migratePanes<string>(JSON.parse(JSON.stringify(m)), (d): d is string => typeof d === "string");
    expect(revived).toEqual(m);
  });

  test("garbage input yields the empty model", () => {
    expect(migratePanes(null)).toEqual(emptyPanes());
    expect(migratePanes("nope")).toEqual(emptyPanes());
    expect(migratePanes({ root: { kind: "what" } })).toEqual(emptyPanes());
    expect(migratePanes({ root: { kind: "split", id: "s1", orientation: "diagonal", children: [] } })).toEqual(
      emptyPanes(),
    );
  });

  test("leaf data failing the guard rejects the whole tree", () => {
    const bad = { root: { kind: "leaf", id: "1", data: 42 }, focusedId: "1", nextId: 2 };
    expect(migratePanes<string>(bad, (d): d is string => typeof d === "string")).toEqual(emptyPanes());
  });

  test("ratios are clamped, focus falls back to the first leaf, nextId is recomputed", () => {
    const raw = {
      root: {
        kind: "split",
        id: "s9",
        orientation: "horizontal",
        ratio: 47,
        children: [
          { kind: "leaf", id: "4", data: "a" },
          { kind: "leaf", id: "7", data: "b" },
        ],
      },
      focusedId: "gone",
      nextId: "bogus",
    };
    const m = migratePanes<string>(raw, (d): d is string => typeof d === "string");
    expect((m.root as PaneSplit<string>).ratio).toBe(0.95);
    expect(m.focusedId).toBe("4");
    expect(m.nextId).toBe(10); // past s9
  });

  test("an explicitly empty root is preserved", () => {
    const m: PaneModel<string> = migratePanes({ root: null, focusedId: "", nextId: 5 });
    expect(m.root).toBeUndefined();
    expect(m.nextId).toBe(5);
  });
});
