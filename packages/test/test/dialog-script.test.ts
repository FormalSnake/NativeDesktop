// dialogScriptEnv() must produce exactly the shape
// src/automation_dialogs.zig's loadFromJson() parses: a flat object mapping
// method name -> FIFO array of response entries, each entry serialized
// verbatim (see src/dialogs.ts's header comment for the real per-method
// result shapes this mirrors).
import { test, expect } from "bun:test";
import { dialogScriptEnv } from "../src/dialogs.ts";

test("serializes a per-method FIFO as a flat JSON object", () => {
  const env = dialogScriptEnv({
    "dialog.openFile": [["/tmp/a.txt"], []],
    "window.showAlert": [{ buttonId: "delete" }],
  });
  expect(JSON.parse(env)).toEqual({
    "dialog.openFile": [["/tmp/a.txt"], []],
    "window.showAlert": [{ buttonId: "delete" }],
  });
});

test("dialog.saveFile entries are raw string | null, not wrapped", () => {
  const env = dialogScriptEnv({ "dialog.saveFile": ["/tmp/out.txt", null] });
  expect(JSON.parse(env)).toEqual({ "dialog.saveFile": ["/tmp/out.txt", null] });
});

test("dialog.showMessage entries are raw button-index numbers", () => {
  const env = dialogScriptEnv({ "dialog.showMessage": [0, 1] });
  expect(JSON.parse(env)).toEqual({ "dialog.showMessage": [0, 1] });
});

test("window.openFile / window.saveFile carry the OpenFileResult / SaveFileResult shape", () => {
  const env = dialogScriptEnv({
    "window.openFile": [{ canceled: false, paths: ["/tmp/a.txt"] }],
    "window.saveFile": [{ canceled: true, path: null }],
  });
  expect(JSON.parse(env)).toEqual({
    "window.openFile": [{ canceled: false, paths: ["/tmp/a.txt"] }],
    "window.saveFile": [{ canceled: true, path: null }],
  });
});

test("an empty script serializes to an empty object", () => {
  expect(dialogScriptEnv({})).toBe("{}");
});
