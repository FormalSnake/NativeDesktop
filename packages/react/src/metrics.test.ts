// The spacing scale is a per-design-language constant contract — apps type
// Spacing.md instead of magic numbers, so the values themselves are the API.
// Keyed on the backend once the handshake installs it (GTK on macOS still
// lays out libadwaita), with an OS fallback before that.
import { test, expect, afterEach } from "bun:test";
import { Spacing, ContentMargin } from "./metrics.ts";
import { setBackend } from "./platform.ts";

afterEach(() => {
  globalThis.__nd_platform = undefined;
});

test("before the handshake the scale falls back to the running OS", () => {
  if (process.platform === "darwin") {
    expect({ ...Spacing }).toEqual({ xs: 4, sm: 8, md: 12, lg: 20, xl: 24 });
    expect(ContentMargin).toBe(20);
  } else {
    expect({ ...Spacing }).toEqual({ xs: 3, sm: 6, md: 12, lg: 18, xl: 24 });
    expect(ContentMargin).toBe(12);
  }
});

test("the backend decides the scale once known: GTK is GNOME layout on any OS", () => {
  setBackend("gtk");
  expect({ ...Spacing }).toEqual({ xs: 3, sm: 6, md: 12, lg: 18, xl: 24 });
  expect(ContentMargin).toBe(12);
  setBackend("appkit");
  expect({ ...Spacing }).toEqual({ xs: 4, sm: 8, md: 12, lg: 20, xl: 24 });
  expect(ContentMargin).toBe(20);
});

test("md is the shared standard gutter on every platform", () => {
  expect(Spacing.md).toBe(12);
  expect(Spacing.xs).toBeLessThan(Spacing.sm);
  expect(Spacing.sm).toBeLessThan(Spacing.md);
  expect(Spacing.md).toBeLessThan(Spacing.lg);
  expect(Spacing.lg).toBeLessThan(Spacing.xl);
});
