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
    expect(code).toContain('createElement("window")');
    expect(code).not.toContain("jsx");
    // The renderer and solid-js are in the bundle, from their production builds.
    expect(code).not.toMatch(/from "(solid-js|@solidjs\/[^"]+|@nativedesktop\/(solid|core))"/);
    expect(code).toContain("@solidjs/signals/dist/prod/");
  } finally {
    rmSync(outdir, { recursive: true, force: true });
  }
});

test("a built app runs solid-js's client build without the register preload", async () => {
  const outdir = mkdtempSync(join(tmpdir(), "nd-solid-build-"));
  try {
    const path = await buildApp({ entry: join(import.meta.dir, "fixtures/aot-reactive.ts"), outdir });
    const proc = Bun.spawn(["bun", path], { cwd: outdir, env: { ...process.env, BUN_OPTIONS: "" }, stdout: "pipe", stderr: "pipe" });
    const out = await new Response(proc.stdout).text();
    expect(await proc.exited).toBe(0);
    expect(out.trim()).toBe("seen=0,1");
  } finally {
    rmSync(outdir, { recursive: true, force: true });
  }
});

test("a built app keeps packages that import no solid-js external and bundles the ones that do", async () => {
  const outdir = mkdtempSync(join(tmpdir(), "nd-solid-build-"));
  try {
    const path = await buildApp({ entry: join(import.meta.dir, "fixtures/aot-external.ts"), outdir });
    const code = await Bun.file(path).text();
    expect(code).toMatch(/from "@nativedesktop\/data"/);
    expect(code).not.toContain('"@nativedesktop/data/solid"');
    expect(code).not.toMatch(/from "solid-js"/);
  } finally {
    rmSync(outdir, { recursive: true, force: true });
  }
});
