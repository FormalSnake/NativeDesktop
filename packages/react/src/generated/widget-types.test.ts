import { test, expect } from "bun:test";
import { WIDGET_TYPE, WIDGET_TYPE_RESERVED } from "./widget-types";

test("widget types are 1-based schema order, 0 reserved", () => {
  expect(WIDGET_TYPE_RESERVED).toBe(0);
  expect(WIDGET_TYPE.Window).toBe(1);
  expect(WIDGET_TYPE.Box).toBe(2);
  expect(WIDGET_TYPE.Label).toBe(3);
  expect(WIDGET_TYPE.Button).toBe(4);
  expect(WIDGET_TYPE.ListView).toBe(18);
  expect(WIDGET_TYPE.WebView).toBe(19);
});
