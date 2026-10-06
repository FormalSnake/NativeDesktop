import { test, expect } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { buildApp } from "./build.ts";
import { isSolidModule } from "./solid-source.ts";

test("an ahead-of-time build emits universal output importing its helpers from @nativedesktop/solid", async () => {
  const outdir = mkdtempSync(join(tmpdir(), "nd-solid-build-"));
  try {
    const path = await buildApp({ entry: join(import.meta.dir, "fixtures/uncaught-render.tsx"), outdir });
    expect(path).toBe(join(outdir, "uncaught-render.js"));
    const code = await Bun.file(path).text();
    expect(code).toContain('from "@nativedesktop/solid"');
    expect(code).toContain('createElement("window")');
    expect(code).not.toContain("jsx");
    expect(code).toMatch(/from "solid-js"/);
  } finally {
    rmSync(outdir, { recursive: true, force: true });
  }
});

test("a .tsx outside any solid package is solid by its jsxImportSource pragma", () => {
  const outside = join(tmpdir(), "nd-no-package", "app.tsx");
  expect(isSolidModule(outside, "/** @jsxImportSource @nativedesktop/solid */\n<window />")).toBe(true);
  expect(isSolidModule(outside, "<window />")).toBe(false);
  expect(isSolidModule(join(import.meta.dir, "x.tsx"), "<window />")).toBe(true);
});
