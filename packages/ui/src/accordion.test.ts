import { describe, expect, test } from "bun:test";
import { accordionDragPayload, nextExpandedIds, parseAccordionDrag, reorderedIds } from "./accordion.ts";

describe("nextExpandedIds", () => {
  test("single mode: opening one closes the others", () => {
    expect(nextExpandedIds(["a"], "b", true, false)).toEqual(["b"]);
    expect(nextExpandedIds([], "a", true, false)).toEqual(["a"]);
  });

  test("single mode: closing the open item empties the list", () => {
    expect(nextExpandedIds(["a"], "a", false, false)).toEqual([]);
  });

  test("single mode: redundant toggles are a same-reference no-op", () => {
    const open = ["a"];
    expect(nextExpandedIds(open, "a", true, false)).toBe(open);
    const closed: string[] = [];
    expect(nextExpandedIds(closed, "a", false, false)).toBe(closed);
  });

  test("multiple mode: several items can stay open at once", () => {
    expect(nextExpandedIds(["a"], "b", true, true)).toEqual(["a", "b"]);
    expect(nextExpandedIds(["a", "b"], "a", false, true)).toEqual(["b"]);
  });

  test("multiple mode: redundant toggles are a same-reference no-op", () => {
    const ids = ["a", "b"];
    expect(nextExpandedIds(ids, "a", true, true)).toBe(ids);
    expect(nextExpandedIds(ids, "z", false, true)).toBe(ids);
  });
});

describe("accordion reordering", () => {
  const ids = ["a", "b", "c"];

  test("round-trip payloads, and anything foreign is not a section drag", () => {
    expect(parseAccordionDrag(accordionDragPayload("a"))).toBe("a");
    for (const foreign of ["", "a", "nd-accordion:item:", "nd-dock:tab:a"]) {
      expect(parseAccordionDrag(foreign)).toBeUndefined();
    }
  });

  test("the dragged section takes the target's place", () => {
    expect(reorderedIds(ids, "c", "a")).toEqual(["c", "a", "b"]);
    expect(reorderedIds(ids, "a", "c")).toEqual(["b", "c", "a"]);
    expect(reorderedIds(ids, "b", "c")).toEqual(["a", "c", "b"]);
  });

  test("a drop that changes nothing is the same reference", () => {
    expect(reorderedIds(ids, "b", "b")).toBe(ids);
    expect(reorderedIds(ids, "gone", "a")).toBe(ids);
    expect(reorderedIds(ids, "a", "gone")).toBe(ids);
  });
});
