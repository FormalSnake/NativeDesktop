import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { findAppDir, preloadEnv, SOLID_PRELOAD, withPreload } from "./preload.ts";

function app(pkg: object): string {
  const dir = mkdtempSync(join(tmpdir(), "nd-preload-"));
  writeFileSync(join(dir, "package.json"), JSON.stringify(pkg));
  return dir;
}

describe("preloadEnv", () => {
  test("a Solid app gets the register preload", () => {
    const dir = app({ dependencies: { "@nativedesktop/solid": "0.4.54" } });
    expect(preloadEnv(dir, undefined)).toEqual({ BUN_OPTIONS: `--preload=${SOLID_PRELOAD}` });
  });

  test("an app without a runtime Solid dependency gets nothing", () => {
    expect(preloadEnv(app({ dependencies: { "@nativedesktop/core": "0.4.54" } }), undefined)).toEqual({});
    expect(preloadEnv(app({ devDependencies: { "@nativedesktop/solid": "0.4.54" } }), undefined)).toEqual({});
  });

  test("existing BUN_OPTIONS are kept and the flag is not repeated", () => {
    expect(withPreload("--smol", SOLID_PRELOAD)).toBe(`--smol --preload=${SOLID_PRELOAD}`);
    const once = withPreload("--smol", SOLID_PRELOAD);
    expect(withPreload(once, SOLID_PRELOAD)).toBe(once);
  });
});

test("findAppDir walks up to the nearest package.json", () => {
  const dir = app({});
  mkdirSync(join(dir, "src", "views"), { recursive: true });
  expect(findAppDir(join(dir, "src", "views"))).toBe(dir);
});
