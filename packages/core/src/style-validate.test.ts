import { expect, test } from "bun:test";
import { validateStyle, StyleError } from "./style-validate.ts";

test("accepts valid style", () => {
  validateStyle({ background: "#fff", padding: 8, font: { fontSize: 14, fontWeight: "bold" } });
});
test("rejects display with fix-it", () => {
  expect(() => validateStyle({ display: "flex" })).toThrow(StyleError);
  try { validateStyle({ display: "flex" }); } catch (e) {
    expect((e as Error).message).toContain("GTK styling is not web CSS");
    expect((e as Error).message).toContain("docs/styling.md");
  }
});
test("suggests nearest key for a typo", () => {
  try { validateStyle({ colour: "#000" }); } catch (e) {
    expect((e as Error).message).toContain('Did you mean "color"');
  }
});
test("rejects unknown nested font key", () => {
  expect(() => validateStyle({ font: { fontStyle: "italic" } })).toThrow(StyleError);
});
