#!/usr/bin/env bun
// scripts/blur-activate-drive.ts: a field's `activate` means Return, never
// focus leaving it. AppKit's NSTextField and NSSearchField send their action
// when editing ends by default, which made a browser address field navigate
// on blur. For each field: focus it, type, move focus to a button, and expect
// no `activate`; then focus it again, press Return, and expect exactly one.
//
//   ND_HOST_BINARY=<host> [DISPLAY=:N] bun scripts/blur-activate-drive.ts
//
// AppKit types and presses through the `keys` RPC. GTK synthesises no keys,
// so on Linux the rig must be an X server and xdotool does it
// (scripts/headless-blur-activate.sh). Marker: ND_BLUR_ACTIVATE_OK.
import { resolve } from "node:path";
import { SOLID_PRELOAD, withPreload } from "@nativedesktop/host";
import { type AppHandle, launchApp } from "../packages/test/src/index.ts";

const MAC = process.platform === "darwin";
const failures: string[] = [];

function xdo(...args: string[]): void {
  const r = Bun.spawnSync(["xdotool", ...args]);
  if (r.exitCode !== 0) throw new Error(`xdotool ${args.join(" ")}: ${r.stderr.toString().trim()}`);
}

async function typeKeys(app: AppHandle, text: string): Promise<void> {
  if (MAC) await app.keyboard.type(text);
  else {
    xdo("search", "--sync", "--name", "Blur activate", "windowactivate", "--sync");
    xdo("type", "--delay", "40", text);
  }
  await Bun.sleep(300);
}

async function pressReturn(app: AppHandle): Promise<void> {
  if (MAC) await app.keys("return");
  else {
    xdo("search", "--sync", "--name", "Blur activate", "windowactivate", "--sync");
    xdo("key", "--clearmodifiers", "Return");
  }
  await Bun.sleep(400);
}

async function counts(app: AppHandle): Promise<Record<string, number>> {
  const text = String((await app.find("count"))?.text ?? "");
  return Object.fromEntries(text.split(" ").map((p) => p.split(":")).map(([k, v]) => [k!, Number(v)]));
}

const app = await launchApp({
  entry: "scripts/blur-activate-app.tsx",
  cwd: resolve(import.meta.dir, ".."),
  hostBinary: process.env.ND_HOST_BINARY,
  // The fixture sits outside any Solid package, so the register preload is passed by hand.
  env: { BUN_OPTIONS: withPreload(process.env.BUN_OPTIONS, SOLID_PRELOAD) },
});
try {
  await Bun.sleep(1000);
  for (const field of ["search", "text"]) {
    const before = (await counts(app))[field] ?? 0;
    await app.getByTestId(field).focus();
    await Bun.sleep(200);
    await typeKeys(app, "example.com");
    const typed = String((await app.find(field))?.value ?? "");
    if (typed !== "example.com") failures.push(`${field}: typing left ${JSON.stringify(typed)} in the field`);
    await app.getByTestId("button").focus();
    await Bun.sleep(500);
    const afterBlur = (await counts(app))[field] ?? 0;
    if (afterBlur !== before) failures.push(`${field}: focus leaving the field fired activate ${afterBlur - before} time(s)`);
    await app.getByTestId(field).focus();
    await Bun.sleep(200);
    await pressReturn(app);
    const afterReturn = (await counts(app))[field] ?? 0;
    if (afterReturn !== afterBlur + 1) failures.push(`${field}: Return fired activate ${afterReturn - afterBlur} time(s), want 1`);
    // Leaving again after the Return must not add a second one.
    await app.getByTestId("button").focus();
    await Bun.sleep(500);
    const afterSecondBlur = (await counts(app))[field] ?? 0;
    if (afterSecondBlur !== afterReturn) failures.push(`${field}: focus leaving after Return fired activate again`);
    console.log(`${field}: blur ${afterBlur - before}, Return ${afterReturn - afterBlur}, blur after Return ${afterSecondBlur - afterReturn}`);
  }
} catch (e) {
  failures.push((e as Error).message);
} finally {
  await app.close();
}

if (failures.length) {
  console.error(`ND_BLUR_ACTIVATE_FAIL\n  ${failures.join("\n  ")}`);
  process.exit(1);
}
console.log("ND_BLUR_ACTIVATE_OK");
