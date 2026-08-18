import { describe, expect, test } from "bun:test";
import { defaultFilter, filterItems } from "./searchable-list.ts";
import type { SearchableListItem } from "./searchable-list.ts";

const items: SearchableListItem[] = [
  { id: "1", label: "Apple" },
  { id: "2", label: "Banana" },
  { id: "3", label: "Blueberry" },
  { id: "4", label: "Cherry" },
];

describe("filterItems", () => {
  test("empty query returns every item, same reference", () => {
    expect(filterItems(items, "")).toBe(items as SearchableListItem[]);
  });

  test("matches case-insensitively against the label", () => {
    expect(filterItems(items, "b").map((i) => i.id)).toEqual(["2", "3"]);
    expect(filterItems(items, "BLUE").map((i) => i.id)).toEqual(["3"]);
  });

  test("no matches returns an empty list", () => {
    expect(filterItems(items, "zzz")).toEqual([]);
  });

  test("a custom predicate overrides the default label match", () => {
    const byId: typeof defaultFilter = (item, query) => item.id === query;
    expect(filterItems(items, "3", byId).map((i) => i.label)).toEqual(["Blueberry"]);
  });
});
