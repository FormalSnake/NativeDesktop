import { test, expect } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { buildApp } from "./build.ts";

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
