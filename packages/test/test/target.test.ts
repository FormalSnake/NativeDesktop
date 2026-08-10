// resolveTarget() is the client-side half of §1.2a targeting: a bare string
// is a testID, a bare number is a ref, and an object descriptor passes
// through untouched (still exactly one of ref/testId — validated host-side).
import { test, expect } from "bun:test";
import { resolveTarget } from "../src/query.ts";

test("a string target resolves to testId", () => {
  expect(resolveTarget("note-list")).toEqual({ testId: "note-list" });
});

test("a number target resolves to ref", () => {
  expect(resolveTarget(42)).toEqual({ ref: 42 });
});

test("an object target passes through", () => {
  expect(resolveTarget({ testId: "note-list", window: 3 })).toEqual({ testId: "note-list", window: 3 });
  expect(resolveTarget({ ref: 7 })).toEqual({ ref: 7 });
});

test("an object target is copied, not aliased", () => {
  const original = { testId: "note-list" };
  const resolved = resolveTarget(original);
  resolved.testId = "mutated";
  expect(original.testId).toBe("note-list");
});
