import { expect, test } from "bun:test";
import { formatSelector, intendedName, parseSelector } from "./selectors.ts";

test("parses every engine", () => {
  expect(parseSelector("testid=save-btn")).toEqual([{ kind: "testid", value: "save-btn" }]);
  expect(parseSelector("type=Button")).toEqual([{ kind: "type", value: "Button" }]);
  expect(parseSelector("text=Save")).toEqual([{ kind: "text", value: "Save" }]);
  expect(parseSelector('text="Save As"')).toEqual([{ kind: "text", value: "Save As", exact: true }]);
  expect(parseSelector("text=/sa.e/i")).toEqual([{ kind: "text", regex: /sa.e/i }]);
  expect(parseSelector("label=Volume")).toEqual([{ kind: "label", value: "Volume" }]);
  expect(parseSelector("placeholder=Search")).toEqual([{ kind: "placeholder", value: "Search" }]);
});

test("parses a role with attributes", () => {
  expect(parseSelector('role=button[name="Save"][exact]')).toEqual([
    { kind: "role", role: "button", name: { value: "Save", exact: true } },
  ]);
  expect(parseSelector("role=checkbox[checked][disabled]")).toEqual([
    { kind: "role", role: "checkbox", checked: true, disabled: true },
  ]);
  expect(parseSelector("role=checkbox[checked=false]")).toEqual([
    { kind: "role", role: "checkbox", checked: false },
  ]);
  expect(parseSelector("role=button[name=/save/i]")).toEqual([
    { kind: "role", role: "button", name: { regex: /save/i } },
  ]);
});

test("chains parts on >> and reads the positional shorthands", () => {
  expect(parseSelector('role=button[name="Save"] >> nth=1')).toEqual([
    { kind: "role", role: "button", name: { value: "Save", exact: true } },
    { kind: "nth", index: 1 },
  ]);
  expect(parseSelector("testid=list >> first")).toEqual([
    { kind: "testid", value: "list" },
    { kind: "nth", index: 0 },
  ]);
  expect(parseSelector("testid=list >> last")).toEqual([
    { kind: "testid", value: "list" },
    { kind: "nth", index: -1 },
  ]);
  expect(parseSelector("nth=-1")).toEqual([{ kind: "nth", index: -1 }]);
});

test("a >> inside a quoted value or a regex is not a separator", () => {
  expect(parseSelector('text=">> keep"')).toEqual([{ kind: "text", value: ">> keep", exact: true }]);
  expect(parseSelector("text=/a>>b/")).toEqual([{ kind: "text", regex: /a>>b/ }]);
});

test("parses nested filter parts", () => {
  expect(parseSelector("type=Row >> has=(role=button)")).toEqual([
    { kind: "type", value: "Row" },
    { kind: "has", parts: [{ kind: "role", role: "button" }] },
  ]);
  expect(parseSelector("type=Row >> has-not=(testid=x) >> has-text=Ada")).toEqual([
    { kind: "type", value: "Row" },
    { kind: "has-not", parts: [{ kind: "testid", value: "x" }] },
    { kind: "has-text", value: "Ada" },
  ]);
});

test("format round-trips through parse", () => {
  const cases = [
    "testid=save-btn",
    'role=button[name="Save"]',
    "role=button[name=Save]",
    "role=checkbox[checked][disabled]",
    "text=/sa.e/i",
    "type=Table >> has-text=Ada >> nth=-1",
    "type=Row >> has=(role=button >> nth=0)",
    "label=Volume >> and=(type=Label)",
  ];
  for (const selector of cases) {
    const parsed = parseSelector(selector);
    expect(formatSelector(parsed)).toBe(selector);
    expect(parseSelector(formatSelector(parsed))).toEqual(parsed);
  }
});

test("rejects a malformed selector", () => {
  expect(() => parseSelector("button")).toThrow(/no engine/);
  expect(() => parseSelector("nope=1")).toThrow(/unknown engine/);
  expect(() => parseSelector("role=button[bogus]")).toThrow(/unknown role attribute/);
  expect(() => parseSelector("nth=x")).toThrow(/not an integer/);
});

test("intendedName picks the last naming part", () => {
  expect(intendedName(parseSelector("testid=save-btn >> nth=1"))).toBe("save-btn");
  expect(intendedName(parseSelector('role=button[name="Save"]'))).toBe("Save");
  expect(intendedName(parseSelector("role=button"))).toBe("button");
  expect(intendedName(parseSelector("testid=a >> text=Ada"))).toBe("Ada");
  expect(intendedName([])).toBeUndefined();
});
