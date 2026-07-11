import { test, expect } from "bun:test";
import { validateCssClasses, CssClassError } from "./css-classes-validate.ts";

test("accepts known Adwaita classes", () => {
  expect(() => validateCssClasses(["navigation-sidebar", "pill", "title-2"])).not.toThrow();
});

test("accepts undefined/empty", () => {
  expect(() => validateCssClasses(undefined)).not.toThrow();
  expect(() => validateCssClasses([])).not.toThrow();
});

test("rejects an unknown class with a fix-it hint", () => {
  expect(() => validateCssClasses(["navigation-sidbar"])).toThrow(CssClassError);
  try {
    validateCssClasses(["navigation-sidbar"]);
  } catch (e) {
    expect((e as Error).message).toContain('Did you mean "navigation-sidebar"');
  }
});

test("rejects a totally unknown class by listing valid ones", () => {
  expect(() => validateCssClasses(["flexbox"])).toThrow(CssClassError);
});

test("rejects a non-string array element", () => {
  expect(() => validateCssClasses([42 as unknown as string])).toThrow(CssClassError);
});
