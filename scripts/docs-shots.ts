#!/usr/bin/env bun
// scripts/docs-shots.ts — captures the docs-site screenshot set for one
// backend into an output directory, one PNG per example (asset contract:
// docs-site/src/assets/screens/<backend>/<example>.png).
//
//   bun scripts/docs-shots.ts <appkit|gtk> <outDir> [example ...]
//
// AppKit captures go through tools/ndshot (pixel-true ScreenCaptureKit, the
// window must be focused first — unfocused captures show dimmed selection);
// GTK captures use the automation screenshot RPC, which renders headless
// under weston (see scripts/headless-*.sh for the compositor recipe).
// Examples that look empty on a bare launch are interacted with first
// (palette opened, panes split, a line typed into the terminal).
import { mkdirSync, mkdtempSync, copyFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { type AppHandle, launchApp, findMatchingNode } from "@nativedesktop/test";

const backend = process.argv[2] as "appkit" | "gtk";
const outDir = process.argv[3];
const only = process.argv.slice(4);
if ((backend !== "appkit" && backend !== "gtk") || !outDir) {
  console.error("usage: bun scripts/docs-shots.ts <appkit|gtk> <outDir> [example ...]");
  process.exit(2);
}
mkdirSync(outDir, { recursive: true });

const NDSHOT = resolve(import.meta.dir, "..", "tools", "ndshot", "bin", "ndshot");

interface Shot {
  name: string;
  /** Entry file when the shot is not named after its example dir. */
  entry?: string;
  /** Backends this shot exists on (default: both). */
  backends?: ("appkit" | "gtk")[];
  /** Extra settle after ready+focus, before interact/capture. */
  settleMs?: number;
  interact?: (app: AppHandle) => Promise<void>;
  /** Pick a window by title (ndshot --title / windows() lookup on GTK). */
  windowTitle?: string;
  /** Additional copies of the same capture (component-focused names). */
  alsoAs?: string[];
  env?: Record<string, string>;
  /** AppKit capture path. ndshot (default) is pixel-true SCK; "rpc" is the
   * automation screenshot for windows SCK serves stale frames for. */
  via?: "ndshot" | "rpc";
}

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

const SHOTS: Shot[] = [
  { name: "counter", settleMs: 1500 },
  { name: "notes", settleMs: 1200 },
  { name: "settings", settleMs: 1000 },
  { name: "gallery", settleMs: 1500 },
  {
    name: "sourcetree",
    settleMs: 800,
    interact: async (app) => {
      await app.setValue("st-tree", "run-1");
      await sleep(500);
    },
  },
  {
    name: "panes",
    settleMs: 800,
    // Fresh store dir per run: the shot always starts from the seeded single
    // pane and splits deterministically instead of restoring old layouts.
    env: { ND_STORE_DIR: mkdtempSync(resolve(tmpdir(), "nd-shots-panes-")) },
    // SCK returns stale frames for this window (content committed after
    // launch never reaches the composite it captures); the RPC path is
    // correct here.
    via: "rpc",
    interact: async (app) => {
      // The panes store persists across runs; only split when the restored
      // layout is still the single seeded root pane.
      const tree = await app.tree();
      const labels: string[] = [];
      findMatchingNode(tree.root, (n) => {
        if (n.testID?.startsWith("pane-label-")) labels.push(n.testID);
        return false;
      });
      if (labels.length === 1) {
        const h = findMatchingNode(tree.root, (n) => n.testID?.startsWith("split-h-") ?? false);
        if (h?.testID) await app.click(h.testID);
        await sleep(400);
        const after = await app.tree();
        const vs: string[] = [];
        findMatchingNode(after.root, (n) => {
          if (n.testID?.startsWith("split-v-")) vs.push(n.testID);
          return false;
        });
        const last = vs[vs.length - 1];
        if (last) await app.click(last);
        await sleep(400);
      }
    },
  },
  {
    name: "terminal",
    settleMs: 1800,
    interact: async (app) => {
      if (app.backend !== "appkit") return; // GTK keys RPC is -32003
      // The terminal view is not first responder after launch; unconsumed
      // plain keys then fall through to macOS 26's Window > Fill shortcut
      // and blow the window up to the visible frame. A real click into the
      // grid makes it first responder so keystrokes reach the PTY.
      const tree = await app.tree();
      const term = findMatchingNode(tree.root, (n) => n.type.toLowerCase() === "terminal");
      if (!term?.geometry) throw new Error("no terminal node in tree");
      const cx = term.geometry.x + term.geometry.w / 2;
      const cy = term.geometry.y + term.geometry.h / 2;
      await app.rpc.call("pointer", { phase: "down", x: cx, y: cy });
      await app.rpc.call("pointer", { phase: "up", x: cx, y: cy });
      await sleep(300);
      await app.keys("echo hello from a real PTY");
      await app.keys("return");
      await sleep(800);
    },
  },
  { name: "browser", settleMs: 6000 },
  {
    // Two native system tabs (cmd+t through the real menu key equivalent).
    // GTK has no keys RPC and the AdwTabBar "+" is framework chrome outside
    // the a11y tree, so this component shot is macOS-only.
    name: "tabs",
    entry: "examples/terminal/main.tsx",
    backends: ["appkit"],
    settleMs: 1800,
    interact: async (app) => {
      await app.keys("cmd+t");
      await sleep(1500);
    },
  },
  {
    name: "dialogs",
    settleMs: 800,
    interact: async (app) => {
      await app.click("window-show-alert-button");
      await sleep(1200);
    },
  },
  { name: "gestures", settleMs: 1000 },
  { name: "tasks", settleMs: 1000 },
  {
    name: "multiwindow",
    settleMs: 2500,
    windowTitle: "Window A",
    interact: async (app) => {
      // Window B is key after launch; raise A so the captured window is not
      // the dimmed-traffic-lights one.
      if (app.backend === "appkit") {
        osa(
          `tell application "System Events" to tell (first application process whose unix id is ${app.pid}) to perform action "AXRaise" of window "Window A"`,
        );
        await sleep(600);
      }
    },
  },
  {
    name: "errors",
    settleMs: 800,
    interact: async (app) => {
      await app.click("throw-caught");
      await app.waitForText("caught:", { timeoutMs: 4000 });
      await sleep(300);
    },
  },
  { name: "inspector", settleMs: 1200 },
  {
    name: "command-palette",
    settleMs: 800,
    alsoAs: ["commandpalette-open"],
    interact: async (app) => {
      const palette = await app.mustFind("palette");
      await app.click("open-button");
      await app.waitFor({ refVisible: palette.ref }, { timeoutMs: 4000 });
      await sleep(600);
    },
  },
];

function osa(script: string): void {
  const r = Bun.spawnSync(["osascript", "-e", script]);
  if (r.exitCode !== 0) throw new Error(`osascript failed: ${r.stderr.toString()}`);
}

function focusMac(pid: number): void {
  osa(`tell application "System Events" to set frontmost of (first application process whose unix id is ${pid}) to true`);
}

interface NdshotWindow {
  pid: number;
  windowID: number;
  title: string;
  width: number;
  height: number;
  onScreen: boolean;
}

/** The bare --pid path matches the app's menu-bar strip first; resolve the
 * real window via `ndshot list` (largest on-screen window, title filter
 * when one is given) and capture by --window-id. */
async function captureNdshot(pid: number, path: string, title?: string): Promise<void> {
  const list = Bun.spawnSync([NDSHOT, "list"]);
  if (list.exitCode !== 0) throw new Error(`ndshot list failed: ${list.stderr.toString()}`);
  const windows = list.stdout
    .toString()
    .split("\n")
    .filter((l) => l.trim().startsWith("{"))
    .map((l) => JSON.parse(l) as NdshotWindow)
    .filter((w) => w.pid === pid && w.onScreen && w.height > 100)
    .filter((w) => !title || w.title.toLowerCase().includes(title.toLowerCase()))
    .sort((a, b) => b.width * b.height - a.width * a.height);
  const target = windows[0];
  if (!target) throw new Error(`no capturable window for pid ${pid}${title ? ` title~"${title}"` : ""}`);
  const proc = Bun.spawn([NDSHOT, "capture", "--out", path, "--window-id", String(target.windowID)], {
    stdout: "ignore",
    stderr: "pipe",
  });
  if ((await proc.exited) !== 0) {
    throw new Error(`ndshot failed: ${await new Response(proc.stderr).text()}`);
  }
}

async function captureOne(shot: Shot): Promise<void> {
  const outPath = resolve(outDir, `${shot.name}.png`);
  const app = await launchApp({
    entry: shot.entry ?? `examples/${shot.name}/main.tsx`,
    backend,
    env: { ...(backend === "appkit" ? { ND_APPEARANCE: "light" } : {}), ...shot.env },
    readyTimeoutMs: 30_000,
    rpcTimeoutMs: 15_000,
  });
  try {
    await sleep(600);
    if (backend === "appkit") {
      focusMac(app.pid);
      await sleep(500);
    }
    await sleep(shot.settleMs ?? 500);
    if (shot.interact) await shot.interact(app);
    if (backend === "appkit" && shot.via !== "rpc") {
      await captureNdshot(app.pid, outPath, shot.windowTitle);
    } else if (backend === "appkit") {
      await app.screenshot(outPath, { minBytes: 4000 });
    } else {
      let window: number | undefined;
      if (shot.windowTitle) {
        const wins = await app.windows();
        window = wins.windows.find((w: { title: string; ref: number }) => w.title === shot.windowTitle)?.ref;
      }
      await app.screenshot(outPath, { window, minBytes: 4000 });
    }
    for (const extra of shot.alsoAs ?? []) {
      copyFileSync(outPath, resolve(outDir, `${extra}.png`));
    }
    console.log(`SHOT_OK ${shot.name}`);
  } finally {
    await app.close();
  }
}

let failed = 0;
for (const shot of SHOTS) {
  if (only.length && !only.includes(shot.name)) continue;
  if (shot.backends && !shot.backends.includes(backend)) continue;
  try {
    await captureOne(shot);
  } catch (e) {
    failed++;
    console.error(`SHOT_FAIL ${shot.name}: ${(e as Error).message}`);
  }
}
if (!existsSync(outDir)) process.exit(1);
process.exit(failed === 0 ? 0 : 1);
