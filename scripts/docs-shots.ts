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
import { mkdirSync, mkdtempSync, copyFileSync, existsSync, renameSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { type AppHandle, type JsonNode, launchApp, findMatchingNode, pngSize } from "@nativedesktop/test";
import type { Geometry } from "@nativedesktop/react/rpc";

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
  /** testID of the widget the shot documents. The window capture is cropped
   * to that node's geometry; absent, the whole window is kept. */
  crop?: string;
  /** Air around `crop`, in logical units. */
  cropMargin?: number;
  /** Backends where `crop` is skipped and the full window is kept instead,
   * even though `crop` is set (so the other backend still gets a component
   * crop). For AppKit's `<dialog>`/`<sheet>`: they present through a real
   * NSWindow sheet, separate from the captured parent window, so the node's
   * geometry is expressed in that sheet window's own coordinate space and
   * cropping the parent-window capture to it frames the wrong region. */
  noCropOn?: ("appkit" | "gtk")[];
}

const CROP_MARGIN = 16;

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
  // gpui-parity gallery: one shot per documented widget. Every entry renders
  // the same example and differs in which sidebar section it selects, which
  // widget it crops to, and whatever puts that widget in a state worth
  // documenting.
  parity("display", "parity-avatar", "display-avatar-group"),
  parity("display", "parity-badge", "display-badge-group"),
  parity("display", "parity-tag", "display-tag-group"),
  parity("display", "parity-kbd", "display-kbd-group"),
  parity("display", "parity-levelindicator", "display-level-group"),
  parity("input", "parity-combobox", "input-group"),
  parity("input", "parity-enabled", "input-enabled-group"),
  parity("navigation", "parity-breadcrumb", "nav-breadcrumb-group"),
  parity("navigation", "parity-pagination", "nav-pagination-group"),
  parity("navigation", "parity-stepper", "nav-stepper-group"),
  parity("composition", "parity-accordion", "composition-accordion-group"),
  parity("composition", "parity-descriptionlist", "composition-description-list"),
  parity("composition", "parity-searchablelist", "composition-searchable-group"),
  parity("composition", "parity-form", "composition-form"),
  // Left empty: a per-cell setValue lands out of order against the controlled
  // `value` round-trip, so the digits on screen would not match the status
  // line under them.
  parity("composition", "parity-otp", "composition-otp-group"),
  parity("composition", "parity-buttongroup", "composition-buttongroup-group"),
  parity("composition", "parity-hovercard", "composition-hovercard-group"),
  // A status bar is a ~24pt strip; the default margin leaves a crop too thin
  // to read as a screenshot.
  parity("composition", "parity-statusbar", "composition-statusbar", { cropMargin: 40 }),
  parity("data", "parity-table", "data-table"),
  // On AppKit, Dialog and Sheet each present through a real NSWindow sheet,
  // separate from the captured parent window; the node's geometry is
  // expressed in that sheet window's own coordinate space, so cropping the
  // parent capture to it frames the wrong region (near the origin, since
  // the sheet's local coordinates read as parent-window coordinates near
  // (0,0)). GTK presents both in-window (AdwDialog), where the crop is
  // correct, so only AppKit skips it here.
  parity("overlays", "parity-dialog", "overlays-dialog", {
    noCropOn: ["appkit"],
    interact: async (app) => {
      await app.click("overlays-dialog-open-button");
      await app.waitFor({ testId: "overlays-dialog-save", state: "visible" }, { timeoutMs: 4000 });
      await sleep(600);
    },
  }),
  parity("overlays", "parity-sheet", "overlays-sheet", {
    noCropOn: ["appkit"],
    interact: async (app) => {
      // `bottom` is the edge AdwDialog presents as a real bottom sheet; the
      // other three fall back to its floating presentation.
      await app.click("overlays-sheet-open-bottom");
      await app.waitFor({ testId: "overlays-sheet-close", state: "visible" }, { timeoutMs: 4000 });
      await sleep(600);
    },
  }),
  parity("richtext", "parity-richtext", "richtext-view"),
  // The Types tab (the default) draws all six chart types at once. Switching
  // to the Live tab would need a TabView setValue the semantic-action
  // dispatch does not implement on either backend.
  parity("charts", "parity-charts", "charts-types-grid", { settleMs: 2000, cropMargin: 8 }),
  parity("charts", "parity-chart-line", "charts-line", { settleMs: 2000 }),
  parity("progress", "parity-progress", "progress-live-group", {
    interact: async (app) => {
      // One slider drives the bar and the circle: move it off its seeded
      // value so the shot shows a live fraction, not the initial render.
      await app.setValue("progress-live-slider", 0.62);
      await sleep(500);
    },
  }),
  parity("progress", "parity-progresscircle", "progress-circle-group"),
  parity("progress", "parity-spinner", "progress-spinner-group"),
  // Seeded unloaded, which is the skeleton state progress-loading.md documents.
  parity("loading", "parity-skeleton", "loading-list"),
  // DockView derives per-panel testIDs from its `testID` but never puts the
  // bare id on a node of its own (TilesView does), so the crop frames the
  // example's dock canvas instead.
  parity("dock", "parity-dock", "dock-canvas"),
  parity("tiles", "parity-tiles", "parity-tiles"),
  // No drag in flight: GTK cannot synthesize input (-32003) and the AppKit
  // drag RPC posts a whole gesture in one marshal, which ends with the drop
  // already committed. The board itself is the documentable state.
  parity("dragdrop", "parity-dragdrop", "dragdrop-board"),
  // Diagnostics are pinned in the example, so the squiggles are on screen
  // without any interaction. The default margin clips the settings group's
  // caption line sitting right above the editor; a bit more headroom keeps
  // it whole.
  parity("codeeditor", "parity-codeeditor", "codeeditor-widget", { cropMargin: 32 }),
];

/** examples/parity/main.tsx renders one section at a time behind a
 * <sourcetree>; each parity shot selects its section by node id (the same
 * id-addressed setValue scripts/sourcetree-drive.ts uses), then crops the
 * "Parity Gallery" window down to the widget it documents. */
function parity(
  section: string,
  name: string,
  crop: string,
  opts: {
    settleMs?: number;
    cropMargin?: number;
    noCropOn?: ("appkit" | "gtk")[];
    interact?: (app: AppHandle) => Promise<void>;
  } = {},
): Shot {
  return {
    name,
    entry: "examples/parity/main.tsx",
    settleMs: opts.settleMs ?? 1500,
    crop,
    cropMargin: opts.cropMargin,
    noCropOn: opts.noCropOn,
    interact: async (app) => {
      await app.setValue("parity-nav", section);
      await sleep(800);
      if (opts.interact) await opts.interact(app);
    },
  };
}

interface Located {
  node: JsonNode;
  /** The window the node lives in: its geometry is relative to this one. */
  window: JsonNode | null;
  /** Nearest scrolling ancestor, if any. */
  scroller: JsonNode | null;
}

function locate(root: JsonNode, testId: string): Located | null {
  let hit: Located | null = null;
  const walk = (node: JsonNode, window: JsonNode | null, scroller: JsonNode | null): void => {
    if (hit) return;
    if (node.testID === testId) {
      hit = { node, window, scroller };
      return;
    }
    const kind = node.type.toLowerCase();
    const win = kind === "window" ? node : window;
    const sv = kind === "scrollview" ? node : scroller;
    for (const child of node.children) walk(child, win, sv);
  };
  walk(root, null, null);
  return hit;
}

/** Scrolls a crop target fully inside its nearest scrolling ancestor, so the
 * capture holds the whole widget before it is cropped to. Which sign of `dy`
 * moves content which way is a backend detail, so a correction that made the
 * gap bigger flips the sign instead of being trusted. */
async function scrollIntoView(app: AppHandle, testId: string, margin: number): Promise<void> {
  let sign = 1;
  let previous = Infinity;
  for (let attempt = 0; attempt < 4; attempt++) {
    const found = locate((await app.tree()).root, testId);
    const g = found?.node.geometry;
    const view = found?.scroller?.geometry;
    if (!found?.scroller || !g || !view) return;
    const topGap = g.y - margin - view.y;
    const bottomOver = g.y + g.h + margin - (view.y + view.h);
    // Both positive means the widget is taller than the viewport; aligning its
    // top is the best available framing, and topGap is the smaller move.
    const need = bottomOver > 0 ? Math.min(bottomOver, topGap) : topGap < 0 ? topGap : 0;
    if (Math.abs(need) < 1) return;
    if (Math.abs(need) >= previous) sign = -sign;
    previous = Math.abs(need);
    await app.scroll({ ref: found.scroller.ref }, { dy: need * sign });
    await sleep(350);
  }
}

/** The rect a crop should actually frame. An overlay container (`<dialog>`,
 * `<sheet>`) is allocated the whole window and centres its card inside, so
 * cropping to the node itself would frame the window; descend to the single
 * child that carries the visible presentation. On AppKit the container node
 * itself reports a degenerate 0x0 (its real presentation is a separate sheet
 * window, not a subview with window-relative geometry) while its child
 * already carries the sheet's real content frame, so a 0x0 node also counts
 * as "not the real rect" and triggers the same descent. */
function contentRect(node: JsonNode, win: Geometry): Geometry | null {
  let current = node;
  for (;;) {
    const g = current.geometry;
    const isRealRect = g && g.w > 0 && g.h > 0 && (g.w < win.w || g.h < win.h);
    if (isRealRect) return g;
    const child = current.children.length === 1 ? current.children[0] : undefined;
    const cg = child?.geometry;
    if (!child || !cg || cg.w <= 0 || cg.h <= 0) return g ?? null;
    current = child;
  }
}

/** Crops a window capture down to one widget plus `margin` logical units of
 * air, in place. */
async function cropToNode(app: AppHandle, path: string, testId: string, margin: number): Promise<void> {
  const found = locate((await app.tree()).root, testId);
  if (!found) throw new Error(`crop target "${testId}" is not in the tree`);
  const win = found.window?.geometry;
  if (!win || win.w <= 0) throw new Error(`crop target "${testId}" has no window geometry to scale against`);
  const g = contentRect(found.node, win);
  if (!g || g.w <= 0 || g.h <= 0) throw new Error(`crop target "${testId}" has no usable geometry: ${JSON.stringify(g)}`);
  const png = await pngSize(path);
  // Tree geometry is logical; the PNG is device pixels (2x on a Retina Mac,
  // 1x under weston). Derive the factor from the window instead of assuming.
  const scale = png.width / win.w;
  const x = Math.max(0, Math.round((g.x - margin) * scale));
  const y = Math.max(0, Math.round((g.y - margin) * scale));
  const w = Math.min(png.width, Math.round((g.x + g.w + margin) * scale)) - x;
  const h = Math.min(png.height, Math.round((g.y + g.h + margin) * scale)) - y;
  if (w < 80 || h < 80) throw new Error(`crop "${testId}" came out ${w}x${h}: too small to be the widget`);
  if (w === png.width && h === png.height) {
    throw new Error(`crop "${testId}" is the whole ${png.width}x${png.height} window: the geometry lookup missed`);
  }
  const tmp = `${path}.crop.png`;
  const r = Bun.spawnSync(["magick", path, "-crop", `${w}x${h}+${x}+${y}`, "+repage", tmp]);
  if (r.exitCode !== 0) throw new Error(`magick crop failed: ${r.stderr.toString()}`);
  renameSync(tmp, path);
}

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
 * real window via `ndshot list` and capture by --window-id. `pick: "secondary"`
 * takes the second-largest onscreen window instead of the largest: a sheet
 * or dialog opens its own NSWindow alongside the (larger) main window, and
 * that extra window is otherwise unaddressable by title (sheets are untitled). */
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
    const margin = shot.cropMargin ?? CROP_MARGIN;
    const doCrop = Boolean(shot.crop) && !shot.noCropOn?.includes(backend);
    if (doCrop) await scrollIntoView(app, shot.crop!, margin);
    if (backend === "appkit") {
      await captureNdshot(app.pid, outPath, shot.windowTitle);
    } else {
      let window: number | undefined;
      if (shot.windowTitle) {
        const wins = await app.windows();
        window = wins.windows.find((w: { title: string; ref: number }) => w.title === shot.windowTitle)?.ref;
      }
      await app.screenshot(outPath, { window, minBytes: 4000 });
    }
    if (doCrop) await cropToNode(app, outPath, shot.crop!, margin);
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
