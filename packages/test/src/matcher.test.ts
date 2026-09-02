// Selector engine against a real getTree capture. fixtures/gestures-tree.json
// came off examples/gestures on the AppKit host, so the field shapes (a
// Slider's numeric value, a Checkbox's boolean, a Label whose value repeats
// its text) are the host's, not a hand-written idea of them.
import { expect, test } from "bun:test";
import type { GetTreeResult } from "@nativedesktop/react/rpc";
import fixture from "./fixtures/gestures-tree.json" with { type: "json" };
import { asNdNode, nodeChecked, nodeName, renderValue, selectNodes, subtreeText, type NdNode } from "./matcher.ts";
import { parseSelector } from "./selectors.ts";

const root: NdNode = asNdNode((fixture as GetTreeResult).root);

function ids(selector: string): (string | null)[] {
  return selectNodes(root, parseSelector(selector)).map((n) => n.testID);
}

test("testid and type select one node", () => {
  expect(ids("testid=volume-slider")).toEqual(["volume-slider"]);
  expect(ids("type=Checkbox")).toEqual(["agree-check"]);
  expect(ids("testid=nope")).toEqual([]);
});

test("role selects by the schema-declared automation role", () => {
  expect(ids("role=slider")).toEqual(["volume-slider"]);
  expect(ids("role=window")).toEqual([null]);
  expect(ids("role=label")).toEqual(["volume-label", "agree-label", "echo-label", "activated-label", "hover-target"]);
});

test("role name matches the accessible name, exactly when quoted", () => {
  expect(ids('role=label[name="Agreed: no"]')).toEqual(["agree-label"]);
  expect(ids("role=label[name=agreed]")).toEqual(["agree-label"]);
  expect(ids('role=label[name="agreed"]')).toEqual([]);
  expect(ids("role=label[name=/^Volume/]")).toEqual(["volume-label"]);
});

test("checked falls back to a boolean value when the host sends no checked field", () => {
  expect(ids("role=checkbox[checked=false]")).toEqual(["agree-check"]);
  expect(ids("role=checkbox[checked]")).toEqual([]);
  expect(nodeChecked(selectNodes(root, parseSelector("testid=agree-check"))[0]!)).toBe(false);
  expect(nodeChecked(selectNodes(root, parseSelector("testid=volume-slider"))[0]!)).toBeUndefined();
});

test("disabled reads the enabled field", () => {
  expect(ids("role=label[disabled]")).toEqual([]);
  expect(ids("role=label[disabled=false]").length).toBe(5);
});

test("text is a case-insensitive substring unless quoted", () => {
  expect(ids("text=volume")).toEqual(["volume-label"]);
  expect(ids('text="Volume: 20"')).toEqual(["volume-label"]);
  expect(ids('text="volume: 20"')).toEqual([]);
  expect(ids("text=/right-click/")).toEqual(["hover-target"]);
});

test("nth, first and last cut the match set", () => {
  expect(ids("role=label >> nth=0")).toEqual(["volume-label"]);
  expect(ids("role=label >> last")).toEqual(["hover-target"]);
  expect(ids("role=label >> nth=-2")).toEqual(["activated-label"]);
  expect(ids("role=label >> nth=99")).toEqual([]);
});

test("chaining descends into the previous match", () => {
  expect(ids("role=group >> role=slider")).toEqual(["volume-slider"]);
  expect(ids("role=slider >> role=label")).toEqual([]);
  expect(ids("type=Box >> role=label >> first")).toEqual(["volume-label"]);
});

test("filter parts refine without descending", () => {
  expect(ids("type=Box >> has=(testid=volume-slider)")).toEqual([null]);
  expect(ids("type=Box >> has=(testid=nope)")).toEqual([]);
  expect(ids("type=Box >> has-not=(testid=nope)")).toEqual([null]);
  expect(ids("role=label >> has-text=Echo")).toEqual(["echo-label"]);
  expect(ids("role=label >> and=(type=Label) >> first")).toEqual(["volume-label"]);
});

test("node readers", () => {
  const slider = selectNodes(root, parseSelector("testid=volume-slider"))[0]!;
  expect(renderValue(slider.value)).toBe("20");
  expect(nodeName(slider)).toBe("");
  expect(nodeName(selectNodes(root, parseSelector("testid=agree-check"))[0]!)).toBe("I agree");
  expect(subtreeText(root)).toContain("Volume: 20");
  expect(subtreeText(root)).toContain("Hover / right-click target");
});
