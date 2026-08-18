import { describe, expect, test } from "bun:test";
import {
  activateTab,
  activeDockTab,
  activeDockTabIndex,
  addTab,
  applyDockDrop,
  closePanel,
  closeTab,
  deserializeDock,
  dockDragPayload,
  dockPanel,
  dockPanelOf,
  dockPanelRects,
  dockZoneAt,
  emptyDock,
  findDockPanel,
  findDockTab,
  hitTestDockZone,
  moveTab,
  parseDockDrag,
  seedDock,
  serializeDock,
  undockTab,
} from "./dock.ts";
import type { DockModel, DockTab, DockZone } from "./dock.ts";
import { paneLeaves, setPaneRatio } from "./model.ts";
import type { PaneSplit } from "./model.ts";

type Panel = ReturnType<typeof dockPanelOf<string>>;

function tab(id: string): DockTab<string> {
  return { id, title: id.toUpperCase(), data: id };
}

/** s3 horizontal [ panel 1 (a, b), panel 2 (c) ], focus on panel 1. */
function base(): DockModel<string> {
  return seedDock([[tab("a"), tab("b")], [tab("c")]]);
}

const isString = (d: unknown): d is string => typeof d === "string";

function tabIds(m: DockModel<string>, panelId: string): string[] {
  return findDockPanel(m, panelId)!.tabs.map((t) => t.id);
}

function panelIds(m: DockModel<string>): string[] {
  return paneLeaves(m).map((l) => l.id);
}

describe("seedDock", () => {
  test("one panel per tab group, first tab active, first panel focused", () => {
    const m = base();
    expect(panelIds(m)).toEqual(["1", "2"]);
    expect(tabIds(m, "1")).toEqual(["a", "b"]);
    expect(findDockPanel(m, "1")!.activeTabId).toBe("a");
    expect(m.focusedId).toBe("1");
    expect((m.root as PaneSplit<Panel>).orientation).toBe("horizontal");
  });

  test("empty tab groups are dropped; nothing left is the empty dock", () => {
    expect(panelIds(seedDock([[tab("a")], []]))).toEqual(["1"]);
    expect(seedDock<string>([[], []])).toEqual(emptyDock<string>());
  });
});

describe("dockPanel edges", () => {
  test("left puts the moved panel on side 0 of a horizontal split", () => {
    const m = dockPanel(base(), "1", "2", "left");
    const root = m.root as PaneSplit<Panel>;
    expect(root.orientation).toBe("horizontal");
    expect(root.children.map((c) => c.id)).toEqual(["1", "2"]);
    expect(m.focusedId).toBe("1");
  });

  test("right puts it on side 1", () => {
    const m = dockPanel(base(), "1", "2", "right");
    const root = m.root as PaneSplit<Panel>;
    expect(root.orientation).toBe("horizontal");
    expect(root.children.map((c) => c.id)).toEqual(["2", "1"]);
  });

  test("top splits vertically with the moved panel first", () => {
    const root = dockPanel(base(), "1", "2", "top").root as PaneSplit<Panel>;
    expect(root.orientation).toBe("vertical");
    expect(root.children.map((c) => c.id)).toEqual(["1", "2"]);
  });

  test("bottom splits vertically with the moved panel second", () => {
    const root = dockPanel(base(), "1", "2", "bottom").root as PaneSplit<Panel>;
    expect(root.orientation).toBe("vertical");
    expect(root.children.map((c) => c.id)).toEqual(["2", "1"]);
  });

  test("docking out of a nested split collapses the split it left", () => {
    // s3 [1, s4 [2, 4]] -> dock 4 to the left of 1 -> s3 [s5 [4, 1], 2]
    let m = seedDock([[tab("a")], [tab("b")]]);
    m = undockTab(addTab(m, "2", tab("c")), "c");
    expect(panelIds(m)).toEqual(["1", "2", "4"]);
    const moved = dockPanel(m, "4", "1", "left");
    const root = moved.root as PaneSplit<Panel>;
    expect(root.id).toBe("s3");
    expect((root.children[0] as PaneSplit<Panel>).children.map((c) => c.id)).toEqual(["4", "1"]);
    expect(root.children[1].id).toBe("2"); // s4 collapsed when 4 left it
    expect(panelIds(moved)).toEqual(["4", "1", "2"]);
  });

  test("tabs and the active tab ride along", () => {
    const m = dockPanel(activateTab(base(), "b"), "1", "2", "top");
    expect(tabIds(m, "1")).toEqual(["a", "b"]);
    expect(findDockPanel(m, "1")!.activeTabId).toBe("b");
  });
});

describe("dockPanel center", () => {
  test("merges the moved panel's tabs into the target stack and collapses", () => {
    const m = dockPanel(base(), "2", "1", "center");
    expect(m.root).toMatchObject({ kind: "leaf", id: "1" });
    expect(tabIds(m, "1")).toEqual(["a", "b", "c"]);
    expect(findDockPanel(m, "1")!.activeTabId).toBe("c"); // the moved panel's active tab
    expect(m.focusedId).toBe("1");
  });

  test("merge is append, and the target keeps its own tab order", () => {
    const m = dockPanel(base(), "1", "2", "center");
    expect(tabIds(m, "2")).toEqual(["c", "a", "b"]);
    expect(panelIds(m)).toEqual(["2"]);
  });
});

describe("dockPanel no-ops", () => {
  test("same panel, unknown panel and unknown target are same-reference", () => {
    const m = base();
    expect(dockPanel(m, "1", "1", "left")).toBe(m);
    expect(dockPanel(m, "9", "1", "center")).toBe(m);
    expect(dockPanel(m, "1", "9", "bottom")).toBe(m);
    expect(dockPanel(m, "s3", "1", "left")).toBe(m); // a split is not a panel
  });

  test("docking a panel where it already sits is same-reference", () => {
    const m = base(); // s3 horizontal [1, 2]
    expect(dockPanel(m, "2", "1", "right")).toBe(m);
    expect(dockPanel(m, "1", "2", "left")).toBe(m);
    expect(dockPanel(m, "2", "1", "bottom")).not.toBe(m); // different axis
  });
});

describe("tabs", () => {
  test("addTab appends and activates; a duplicate id is a no-op", () => {
    const m = addTab(base(), "2", tab("d"));
    expect(tabIds(m, "2")).toEqual(["c", "d"]);
    expect(findDockPanel(m, "2")!.activeTabId).toBe("d");
    expect(addTab(m, "2", tab("a"))).toBe(m); // "a" lives in panel 1
    expect(addTab(m, "9", tab("z"))).toBe(m);
  });

  test("addTab honours an index", () => {
    expect(tabIds(addTab(base(), "1", tab("d"), 0), "1")).toEqual(["d", "a", "b"]);
    expect(tabIds(addTab(base(), "1", tab("d"), 99), "1")).toEqual(["a", "b", "d"]);
  });

  test("activateTab selects inside the panel and leaves the focus alone", () => {
    const m = activateTab(base(), "c");
    expect(findDockPanel(m, "2")!.activeTabId).toBe("c");
    expect(m.focusedId).toBe("1");
    expect(activateTab(m, "c")).toBe(m);
    expect(activateTab(m, "zzz")).toBe(m);
  });

  test("closeTab hands the active slot to the neighbour", () => {
    const m = closeTab(base(), "a");
    expect(tabIds(m, "1")).toEqual(["b"]);
    expect(findDockPanel(m, "1")!.activeTabId).toBe("b");
    expect(closeTab(m, "zzz")).toBe(m);
  });

  test("closing the last tab in a panel collapses the split", () => {
    const m = closeTab(base(), "c");
    expect(m.root).toMatchObject({ kind: "leaf", id: "1" });
    expect(panelIds(m)).toEqual(["1"]);
  });

  test("closing the last tab of the last panel empties the dock", () => {
    let m = closeTab(base(), "c");
    m = closeTab(closeTab(m, "a"), "b");
    expect(m.root).toBeUndefined();
  });

  test("closePanel is closePane: the sibling takes the space", () => {
    const m = closePanel(base(), "1");
    expect(m.root).toMatchObject({ kind: "leaf", id: "2" });
    expect(m.focusedId).toBe("2");
  });
});

describe("moveTab", () => {
  test("reorders inside one panel without touching the active tab", () => {
    const m = moveTab(base(), "a", "1", 1);
    expect(tabIds(m, "1")).toEqual(["b", "a"]);
    expect(findDockPanel(m, "1")!.activeTabId).toBe("a");
    expect(m.focusedId).toBe("1");
  });

  test("moving to the index it already holds is a same-reference no-op", () => {
    const m = base();
    expect(moveTab(m, "a", "1", 0)).toBe(m);
    expect(moveTab(m, "b", "1")).toBe(m); // default index is the end, where b is
    expect(moveTab(m, "zzz", "1", 0)).toBe(m);
    expect(moveTab(m, "a", "9", 0)).toBe(m);
  });

  test("moving across panels activates the tab there and takes the focus", () => {
    const m = moveTab(base(), "a", "2", 0);
    expect(tabIds(m, "1")).toEqual(["b"]);
    expect(tabIds(m, "2")).toEqual(["a", "c"]);
    expect(findDockPanel(m, "2")!.activeTabId).toBe("a");
    expect(m.focusedId).toBe("2");
  });

  test("emptying the source panel collapses it", () => {
    const m = moveTab(base(), "c", "1");
    expect(panelIds(m)).toEqual(["1"]);
    expect(tabIds(m, "1")).toEqual(["a", "b", "c"]);
    expect(m.focusedId).toBe("1");
  });
});

describe("undockTab", () => {
  test("pulls the tab into a new panel beside its host and focuses it", () => {
    const m = undockTab(base(), "b");
    const root = m.root as PaneSplit<Panel>;
    expect(panelIds(m)).toEqual(["1", "4", "2"]);
    expect(tabIds(m, "1")).toEqual(["a"]);
    expect(tabIds(m, "4")).toEqual(["b"]);
    expect(m.focusedId).toBe("4");
    expect((root.children[0] as PaneSplit<Panel>).orientation).toBe("horizontal");
  });

  test("the zone picks the side, default right", () => {
    const left = undockTab(base(), "b", "left").root as PaneSplit<Panel>;
    const host = left.children[0] as PaneSplit<Panel>;
    expect(host.children.map((c) => c.id)).toEqual(["4", "1"]);
    const down = undockTab(base(), "b", "bottom").root as PaneSplit<Panel>;
    expect((down.children[0] as PaneSplit<Panel>).orientation).toBe("vertical");
  });

  test("a panel holding one tab has nothing to undock", () => {
    const m = base();
    expect(undockTab(m, "c")).toBe(m);
    expect(undockTab(m, "zzz")).toBe(m);
  });
});

describe("lookups", () => {
  test("findDockTab reports the panel and index; activeDockTab reads the stack", () => {
    const m = base();
    expect(findDockTab(m, "b")).toMatchObject({ panelId: "1", index: 1 });
    expect(findDockTab(m, "zzz")).toBeUndefined();
    expect(activeDockTab(findDockPanel(m, "1")!)!.id).toBe("a");
    expect(activeDockTabIndex(findDockPanel(m, "1")!)).toBe(0);
    expect(activeDockTabIndex({ tabs: [tab("a")], activeTabId: "gone" })).toBe(0);
  });
});

describe("hitTestDockZone", () => {
  const rect = { x: 0, y: 0, width: 100, height: 100 };

  test("the outer quarter of each side is that side's zone", () => {
    expect(hitTestDockZone(rect, 5, 50)).toBe("left");
    expect(hitTestDockZone(rect, 95, 50)).toBe("right");
    expect(hitTestDockZone(rect, 50, 5)).toBe("top");
    expect(hitTestDockZone(rect, 50, 95)).toBe("bottom");
    expect(hitTestDockZone(rect, 50, 50)).toBe("center");
    expect(hitTestDockZone(rect, 30, 50)).toBe("center"); // just inside the band
  });

  test("the rect origin is honoured and a point past an edge still names it", () => {
    expect(hitTestDockZone({ x: 200, y: 100, width: 100, height: 100 }, 205, 150)).toBe("left");
    expect(hitTestDockZone(rect, -40, 50)).toBe("left");
    expect(hitTestDockZone(rect, 50, 140)).toBe("bottom");
  });

  test("the band is configurable and clamped to a half", () => {
    expect(hitTestDockZone(rect, 30, 50, 0.4)).toBe("left");
    expect(hitTestDockZone(rect, 49, 50, 9)).toBe("left"); // 0.5 band, nothing is center
  });

  test("a degenerate rect or a non-finite pointer is center", () => {
    expect(hitTestDockZone({ x: 0, y: 0, width: 0, height: 100 }, 0, 50)).toBe("center");
    expect(hitTestDockZone({ x: 0, y: 0, width: 100, height: NaN }, 50, 50)).toBe("center");
    expect(hitTestDockZone(rect, NaN, 50)).toBe("center");
  });
});

describe("serializeDock / deserializeDock", () => {
  test("round-trips a real layout through JSON", () => {
    const m = activateTab(dockPanel(addTab(base(), "2", tab("d")), "1", "2", "bottom"), "d");
    const back = deserializeDock<string>(JSON.parse(JSON.stringify(serializeDock(m))), isString);
    expect(back).toEqual(m);
  });

  test("serializeDock is a deep copy, not a view of the live model", () => {
    const m = base();
    const snapshot = serializeDock(m);
    const grown = addTab(m, "1", tab("d"));
    expect(findDockPanel(grown, "1")!.tabs).toHaveLength(3);
    expect(findDockPanel(snapshot, "1")!.tabs).toHaveLength(2);
  });

  test("malformed input rejects the whole layout", () => {
    const empty = emptyDock<string>();
    expect(deserializeDock<string>(null, isString)).toEqual(empty);
    expect(deserializeDock<string>({ root: { kind: "leaf", id: "1", data: { tabs: [] } } }, isString)).toEqual(empty);
    expect(
      deserializeDock<string>({ root: { kind: "leaf", id: "1", data: { tabs: [{ title: "A", data: "a" }] } } }, isString),
    ).toEqual(empty);
    const badData = { root: { kind: "leaf", id: "1", data: { tabs: [{ id: "a", title: "A", data: 42 }] } } };
    expect(deserializeDock<string>(badData, isString)).toEqual(empty);
  });

  test("a tab id used in two panels rejects the layout", () => {
    const clash = serializeDock(base()) as unknown as { root: { children: { data: { tabs: DockTab<string>[] } }[] } };
    clash.root.children[1]!.data.tabs[0]!.id = "a";
    expect(deserializeDock<string>(clash, isString)).toEqual(emptyDock<string>());
  });

  test("an activeTabId naming a tab that is gone is repaired, not rejected", () => {
    const raw = {
      root: { kind: "leaf", id: "1", data: { tabs: [{ id: "a", title: "A", data: "a" }], activeTabId: "gone" } },
      focusedId: "1",
      nextId: 2,
    };
    expect(findDockPanel(deserializeDock<string>(raw, isString), "1")!.activeTabId).toBe("a");
  });

  test("unknown tab fields are dropped and the icon survives", () => {
    const raw = {
      root: {
        kind: "leaf",
        id: "1",
        data: { tabs: [{ id: "a", title: "A", icon: "folder-symbolic", data: "a", stale: true }], activeTabId: "a" },
      },
      focusedId: "1",
      nextId: 2,
    };
    expect(findDockPanel(deserializeDock<string>(raw, isString), "1")!.tabs[0]).toEqual({
      id: "a",
      title: "A",
      icon: "folder-symbolic",
      data: "a",
    });
  });
});

describe("dock drag payloads", () => {
  test("round-trip, and anything foreign is not a dock drag", () => {
    expect(parseDockDrag(dockDragPayload("tab", "a"))).toEqual({ kind: "tab", id: "a" });
    expect(parseDockDrag(dockDragPayload("panel", "1"))).toEqual({ kind: "panel", id: "1" });
    // A tab id may hold the separator itself; only the first one splits.
    expect(parseDockDrag(dockDragPayload("tab", "file:///a.txt"))).toEqual({ kind: "tab", id: "file:///a.txt" });
    for (const foreign of ["", "https://example.com", "nd-dock:", "nd-dock:tab:", "nd-dock:widget:x"]) {
      expect(parseDockDrag(foreign)).toBeUndefined();
    }
  });
});

describe("dockPanelRects", () => {
  test("panel rects come from the split ratios", () => {
    const rects = dockPanelRects(base(), { width: 800, height: 600 });
    expect(rects.get("1")).toEqual({ x: 0, y: 0, width: 400, height: 600 });
    expect(rects.get("2")).toEqual({ x: 400, y: 0, width: 400, height: 600 });
  });

  test("a vertical split divides the height, and a moved divider moves the rects", () => {
    const m = setPaneRatio(dockPanel(base(), "2", "1", "bottom"), "s4", 0.25);
    const rects = dockPanelRects(m, { width: 800, height: 600 });
    expect(rects.get("1")).toEqual({ x: 0, y: 0, width: 800, height: 150 });
    expect(rects.get("2")).toEqual({ x: 0, y: 150, width: 800, height: 450 });
  });
});

describe("dockZoneAt", () => {
  const size = { width: 800, height: 600 };

  test("the outer quarter of each side is that side's zone, the middle is center", () => {
    const m = base();
    expect(dockZoneAt(m, "1", 10, 300, size)).toBe("left");
    expect(dockZoneAt(m, "1", 390, 300, size)).toBe("right");
    expect(dockZoneAt(m, "1", 200, 10, size)).toBe("top");
    expect(dockZoneAt(m, "1", 200, 590, size)).toBe("bottom");
    expect(dockZoneAt(m, "1", 200, 300, size)).toBe("center");
  });

  test("coordinates are the target panel's own, so panel 2 hit-tests against its own width", () => {
    const m = base();
    // 200 is the middle of panel 2's own 400px, though it sits inside panel
    // 1's half of the dock; 410 is past panel 2's right edge, which still
    // names that edge.
    expect(dockZoneAt(m, "2", 10, 300, size)).toBe("left");
    expect(dockZoneAt(m, "2", 200, 300, size)).toBe("center");
    expect(dockZoneAt(m, "2", 410, 300, size)).toBe("right");
  });

  test("the nearest edge wins a corner", () => {
    expect(dockZoneAt(base(), "2", 390, 10, size)).toBe("top");
  });

  test("no size and an unknown panel are center, the zone that needs no geometry", () => {
    expect(dockZoneAt(base(), "1", 10, 300)).toBe("center");
    expect(dockZoneAt(base(), "nope", 10, 300, size)).toBe("center");
  });
});

describe("applyDockDrop", () => {
  const drop = (m: DockModel<string>, payload: string, panelId: string, zone: DockZone) =>
    applyDockDrop(m, payload, panelId, zone);

  test("a tab dropped on another panel's center moves into its stack", () => {
    const m = drop(base(), dockDragPayload("tab", "a"), "2", "center");
    expect(tabIds(m, "1")).toEqual(["b"]);
    expect(tabIds(m, "2")).toEqual(["c", "a"]);
    expect(findDockPanel(m, "2")!.activeTabId).toBe("a");
  });

  test("a tab dropped on an edge of another panel lands in a new panel on that side", () => {
    const m = drop(base(), dockDragPayload("tab", "a"), "2", "top");
    expect(tabIds(m, "1")).toEqual(["b"]);
    expect(tabIds(m, "2")).toEqual(["c"]);
    const fresh = panelIds(m).find((id) => id !== "1" && id !== "2")!;
    expect(tabIds(m, fresh)).toEqual(["a"]);
    expect((m.root as PaneSplit<Panel>).children[1]).toMatchObject({ kind: "split", orientation: "vertical" });
  });

  test("a tab dropped on an edge of its own panel undocks it there", () => {
    const m = drop(base(), dockDragPayload("tab", "b"), "1", "right");
    expect(tabIds(m, "1")).toEqual(["a"]);
    const fresh = panelIds(m).find((id) => id !== "1" && id !== "2")!;
    expect(tabIds(m, fresh)).toEqual(["b"]);
  });

  test("the last tab of a panel takes the panel path, so the panel keeps its id", () => {
    const m = drop(base(), dockDragPayload("tab", "c"), "1", "left");
    expect(panelIds(m)).toEqual(["2", "1"]);
    expect(tabIds(m, "2")).toEqual(["c"]);
  });

  test("a panel payload docks the whole panel", () => {
    const m = drop(base(), dockDragPayload("panel", "2"), "1", "center");
    expect(panelIds(m)).toEqual(["1"]);
    expect(tabIds(m, "1")).toEqual(["a", "b", "c"]);
  });
});

describe("applyDockDrop reference stability", () => {
  test("a tab dropped back on its own panel's center changes nothing", () => {
    const m = base();
    expect(applyDockDrop(m, dockDragPayload("tab", "a"), "1", "center")).toBe(m);
    expect(applyDockDrop(m, dockDragPayload("tab", "b"), "1", "center")).toBe(m);
  });

  test("a panel dropped on itself, or on the side it already occupies, changes nothing", () => {
    const m = base();
    expect(applyDockDrop(m, dockDragPayload("panel", "2"), "2", "center")).toBe(m);
    expect(applyDockDrop(m, dockDragPayload("panel", "2"), "1", "right")).toBe(m);
    // Same drop through the single-tab tab path, which is the one a real
    // drag takes: panel 2 holds only "c".
    expect(applyDockDrop(m, dockDragPayload("tab", "c"), "1", "right")).toBe(m);
  });

  test("a lone tab dropped on an edge of its own panel has nothing to undock", () => {
    const m = base();
    expect(applyDockDrop(m, dockDragPayload("tab", "c"), "2", "top")).toBe(m);
  });

  test("a foreign drag and an unknown id are ignored", () => {
    const m = base();
    expect(applyDockDrop(m, "https://example.com", "1", "left")).toBe(m);
    expect(applyDockDrop(m, dockDragPayload("tab", "gone"), "1", "left")).toBe(m);
    expect(applyDockDrop(m, dockDragPayload("panel", "9"), "1", "left")).toBe(m);
    expect(applyDockDrop(m, dockDragPayload("tab", "a"), "9", "center")).toBe(m);
  });
});
