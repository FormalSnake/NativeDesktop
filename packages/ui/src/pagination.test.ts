import { describe, expect, test } from "bun:test";
import { computePaginationRange } from "./pagination.ts";

describe("computePaginationRange", () => {
  test("everything fits: no dots", () => {
    expect(computePaginationRange(1, 5)).toEqual([1, 2, 3, 4, 5]);
    expect(computePaginationRange(3, 7)).toEqual([1, 2, 3, 4, 5, 6, 7]);
  });

  test("near the start: dots only on the right", () => {
    expect(computePaginationRange(1, 10)).toEqual([1, 2, 3, 4, 5, "dots-end", 10]);
    expect(computePaginationRange(3, 10)).toEqual([1, 2, 3, 4, 5, "dots-end", 10]);
  });

  test("near the end: dots only on the left", () => {
    expect(computePaginationRange(10, 10)).toEqual([1, "dots-start", 6, 7, 8, 9, 10]);
    expect(computePaginationRange(8, 10)).toEqual([1, "dots-start", 6, 7, 8, 9, 10]);
  });

  test("in the middle: dots on both sides", () => {
    expect(computePaginationRange(5, 10)).toEqual([1, "dots-start", 4, 5, 6, "dots-end", 10]);
  });

  test("siblingCount widens the visible window", () => {
    expect(computePaginationRange(5, 10, 2)).toEqual([1, "dots-start", 3, 4, 5, 6, 7, "dots-end", 10]);
  });

  test("edges: pageCount 0, out-of-range page clamps into range", () => {
    expect(computePaginationRange(1, 0)).toEqual([]);
    expect(computePaginationRange(99, 10)).toEqual([1, "dots-start", 6, 7, 8, 9, 10]);
    expect(computePaginationRange(-5, 10)).toEqual([1, 2, 3, 4, 5, "dots-end", 10]);
  });
});
