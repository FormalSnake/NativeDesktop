#!/usr/bin/env bun
// scripts/mac/region-capture-drive.ts: ND_AUTOMATION_CAPTURE=region puts what
// the app stacks over its window into the screenshot. Each leg launches a
// fresh host, takes a baseline, opens one overlay (a real alert sheet or open
// panel from examples/dialogs with no dialog script), and requires the capture helper to
// have composited more windows than the baseline did. Needs the host binary
// granted Screen Recording (`NDShell --nd-grant` with SIP off, or System
// Settings). Marker: ND_REGION_CAPTURE_OK. PNGs and host logs land in
// $ND_REGION_OUT (default /tmp/nd-region-drive).
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { type AppHandle, launchApp } from "../../packages/test/src/index.ts";

const out = process.env.ND_REGION_OUT ?? "/tmp/nd-region-drive";
mkdirSync(out, { recursive: true });
const hostBinary = process.env.ND_HOST_BINARY ?? resolve(import.meta.dir, "../../swift/.build/release/NDShell");
const failures: string[] = [];

async function leg(name: string, entry: string, open: (app: AppHandle) => Promise<void>) {
  let composited = 0;
  let rung = "";
  const app = await launchApp({
    entry,
    hostBinary,
    env: { ND_AUTOMATION_CAPTURE: "region" },
    logPath: `${out}/${name}.log`,
    onStderr: (line) => {
      const m = /ND_SNAPSHOT_REGION windows=(\d+)/.exec(line);
      if (m) composited = Number(m[1]);
      const r = /ND_SNAPSHOT_RUNG rung=(\S+)/.exec(line);
      if (r) rung = r[1];
    },
  });
  try {
    const shot = async (file: string) => {
      composited = 0;
      rung = "";
      const r = await app.screenshot(`${out}/${file}.png`);
      // stderr is piped line by line; give the last lines a moment to land.
      await Bun.sleep(100);
      return { size: `${r.width}x${r.height}`, composited, rung };
    };
    const base = await shot(`${name}-base`);
    if (base.rung !== "0") failures.push(`${name}: baseline fell back to rung ${base.rung}, see ${name}.log`);
    await open(app);
    // A sheet slides in, and the first open panel of a process takes a second
    // or more while its service starts, so poll for the overlay.
    let over = await shot(name);
    for (let i = 0; i < 10 && over.rung === "0" && over.composited <= base.composited; i++) {
      await Bun.sleep(300);
      over = await shot(name);
    }
    if (over.rung !== "0") failures.push(`${name}: overlay shot fell back to rung ${over.rung}`);
    else if (over.composited <= base.composited) {
      failures.push(`${name}: ${over.composited} window(s) composited, baseline had ${base.composited}`);
    } else {
      console.log(`ND_REGION_CHECK ${name}: ok (${base.composited} -> ${over.composited} windows, ${over.size})`);
    }
  } finally {
    await app.close();
  }
}

await leg("alert-sheet", "examples/dialogs/main.tsx", (app) => app.getByTestId("window-show-alert-button").click());
await leg("open-panel", "examples/dialogs/main.tsx", (app) => app.getByTestId("window-open-file-button").click());

if (failures.length) {
  console.error(`ND_REGION_CAPTURE_FAIL\n  ${failures.join("\n  ")}`);
  process.exit(1);
}
console.log("ND_REGION_CAPTURE_OK");
