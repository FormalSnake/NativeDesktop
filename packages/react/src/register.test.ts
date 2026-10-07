import { test, expect } from "bun:test";
import { cpSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

// An app's own copy of the reactive core next to the renderer's: both have to
// load as one graph, or the renderer's effects never see the app's writes.
test("a second copy of solid-js and @solidjs/signals loads as the renderer's", () => {
  const dir = join(import.meta.dir, "..", ".foreign-core");
  rmSync(dir, { recursive: true, force: true });
  try {
    const solid = dirname(require.resolve("solid-js/package.json"));
    const signals = dirname(require.resolve("@solidjs/signals/package.json", { paths: [solid] }));
    cpSync(solid, join(dir, "node_modules", "solid-js"), { recursive: true, dereference: true });
    mkdirSync(join(dir, "node_modules", "@solidjs"), { recursive: true });
    cpSync(signals, join(dir, "node_modules", "@solidjs", "signals"), { recursive: true, dereference: true });
    const copy = join(dir, "node_modules", "solid-js", "dist", "solid.js");
    writeFileSync(
      join(dir, "probe.ts"),
      `import * as renderers from ${JSON.stringify(join(solid, "dist", "solid.js"))};\n` +
        `import * as apps from ${JSON.stringify(copy)};\n` +
        `console.log(String(apps.createSignal === renderers.createSignal && apps.flush === renderers.flush));\n`,
    );
    const run = Bun.spawnSync([process.execPath, "--preload", join(import.meta.dir, "register.ts"), join(dir, "probe.ts")], {
      stderr: "pipe",
    });
    expect(run.stderr.toString()).toBe("");
    expect(run.stdout.toString().trim()).toBe("true");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
