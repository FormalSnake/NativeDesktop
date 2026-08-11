// The spacing scale is a per-OS constant contract — apps type Spacing.md
// instead of magic numbers, so the values themselves are the API.
import { test, expect } from "bun:test";
import { Spacing, ContentMargin } from "./metrics.ts";

test("Spacing matches the running OS's design-language scale", () => {
  if (process.platform === "darwin") {
    expect(Spacing).toEqual({ xs: 4, sm: 8, md: 12, lg: 20, xl: 24 });
    expect(ContentMargin).toBe(20);
  } else {
    expect(Spacing).toEqual({ xs: 3, sm: 6, md: 12, lg: 18, xl: 24 });
    expect(ContentMargin).toBe(12);
  }
});

test("md is the shared standard gutter on every platform", () => {
  expect(Spacing.md).toBe(12);
  expect(Spacing.xs).toBeLessThan(Spacing.sm);
  expect(Spacing.sm).toBeLessThan(Spacing.md);
  expect(Spacing.md).toBeLessThan(Spacing.lg);
  expect(Spacing.lg).toBeLessThan(Spacing.xl);
});
