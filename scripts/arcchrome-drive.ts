#!/usr/bin/env bun
// scripts/arcchrome-drive.ts -- drives examples/arcchrome via
// @nativedesktop/test. Acceptance for the Arc window shape:
//
//   leg 1  the window's own controls sit in the sidebar's first row, centred
//          on the row's buttons (getTree, and on AppKit the close button's
//          red found in a region capture), at two window widths
//   leg 2  the content pane is an inset card on the sidebar's surface: its
//          margins, and on AppKit a corner pixel that is not the page's colour
//   leg 3  a button in the controls row takes its own click (the title bar
//          band does not swallow it)
//   leg 4  hiding the sidebar hides the controls with it and the card takes
//          the window's width inside its margin
//   leg 5  edge reveal: the pointer at the leading edge (AppKit) or the
//          `revealSidebar` command (GTK) slides the sidebar in over the page
//          without resizing the card; leaving it (or `concealSidebar`) hides it
//   leg 6  AppKit: a drag on the controls row's empty run moves the window
//
// Moves the real cursor on AppKit: hold the mac gate lock
// (scripts/mac/cef-gate-lock.sh) and grant the host once (`--nd-grant`).
// Marker: ND_ARCCHROME_OK.
import { connectApp, findNode, launchApp, poll } from "../packages/test/src/index.ts";
import type { JsonNode } from "../packages/test/src/index.ts";
import type { Backend } from "@nativedesktop/host";

const backend = process.argv[2] as Backend | undefined;
const gtk = process.env.ND_BACKEND === "gtk" || backend === "gtk";
const attached = process.env.ND_AUTOMATION_SOCKET != null;
const T = 8000;
const SHOTS = process.env.ND_ARC_SHOTS ?? "/tmp/nd-arcchrome";
Bun.spawnSync(["mkdir", "-p", SHOTS]);

const app = attached ? await connectApp(undefined, { pid: Number(process.env.ND_HOST_PID) || undefined }) : await launchApp({ entry: "examples/arcchrome/main.tsx", backend });
const hostPid = Number(process.env.ND_HOST_PID ?? ("pid" in app ? app.pid : 0));
const ndshot = process.env.ND_NDSHOT;

type Rect = { x: number; y: number; w: number; h: number };
const tree = async () => (await app.tree()).root as JsonNode;
const rectOf = async (id: string): Promise<Rect> => {
  const n = findNode(await tree(), id);
  const g = n?.geometry;
  if (!g || g.w <= 0) throw new Error(`${id} has no geometry (${JSON.stringify(g)})`);
  return g;
};
const textOf = async (id: string) => findNode(await tree(), id)?.text ?? "";
const mid = (r: Rect) => ({ x: r.x + r.w / 2, y: r.y + r.h / 2 });
const near = (a: number, b: number, tol = 1) => Math.abs(a - b) <= tol;
const fail = (m: string): never => {
  throw new Error(m);
};

async function settle(id: string): Promise<Rect> {
  let last = "";
  return poll(async () => rectOf(id), (r) => {
    const k = JSON.stringify(r);
    const same = k === last;
    last = k;
    return same;
  }, { timeoutMs: T });
}

async function windowRect(): Promise<Rect> {
  const g = (await app.windows()).windows[0]?.geometry;
  return g ?? fail("the app reports no window geometry");
}

/// AppKit region capture of the window, and a pixel reader over it.
async function capture(name: string): Promise<{ path: string; scale: number } | null> {
  if (gtk || !ndshot) return null;
  const list = Bun.spawnSync([ndshot, "list"]).stdout.toString().trim().split("\n").map((l) => JSON.parse(l));
  const win = list.find((w: { pid: number; onScreen: boolean; title: string }) => w.pid === hostPid && w.onScreen && w.title !== "");
  if (!win) fail("ndshot sees no window for the host");
  const path = `${SHOTS}/${name}.png`;
  const r = Bun.spawnSync([ndshot, "capture", "--window-id", String(win.windowID), "--region", "--out", path]);
  if (r.exitCode !== 0) fail(`ndshot capture failed: ${r.stderr.toString()}`);
  const scale = Number(r.stdout.toString().trim().split(" ").pop()!.split("x")[0]) / win.width;
  console.log(`  capture ${path}`);
  return { path, scale };
}

/// Runs a small PIL query over a capture; points are window points.
function pixels(shot: { path: string; scale: number }, script: string): unknown {
  const r = Bun.spawnSync(["python3", "-c", `
import json,sys
from PIL import Image
im=Image.open(sys.argv[1]).convert("RGB"); s=float(sys.argv[2])
def px(x,y): return im.getpixel((int(round(x*s)),int(round(y*s))))
${script}
`, shot.path, String(shot.scale)]);
  if (r.exitCode !== 0) fail(`pixel probe failed: ${r.stderr.toString()}`);
  return JSON.parse(r.stdout.toString());
}

/// The close button's red, as a centroid in window points, inside `box`.
function redCentroid(shot: { path: string; scale: number }, box: Rect): { x: number; y: number; n: number } {
  return pixels(shot, `
x0,y0,w,h=${box.x},${box.y},${box.w},${box.h}
xs=[];ys=[]
for yy in range(int(y0*s),int((y0+h)*s)):
  for xx in range(int(x0*s),int((x0+w)*s)):
    r,g,b=im.getpixel((xx,yy))
    if r>200 and g<120 and b<120: xs.append(xx); ys.append(yy)
n=len(xs)
print(json.dumps({"x":(sum(xs)/n/s if n else -1),"y":(sum(ys)/n/s if n else -1),"n":n}))
`) as { x: number; y: number; n: number };
}

const PAGE_BLUE = [0x2f, 0x6b, 0xff];
function isPageBlue(rgb: number[]): boolean {
  return rgb.every((c, i) => Math.abs(c - PAGE_BLUE[i]!) < 40);
}

/// The side that holds buttons: macOS puts all three at the start, and a GTK
/// desktop's decoration layout decides (GNOME's default is close at the end).
async function controlsSlotId(): Promise<string> {
  const start = findNode(await tree(), "controls-start")?.geometry;
  return start && start.w > 0 ? "controls-start" : "controls-end";
}

async function controlsLeg(width: number): Promise<void> {
  await app.setWindowSize(width, 700);
  await settle("toggle");
  const slot = await settle(await controlsSlotId());
  const toggle = await rectOf("toggle");
  const row = await rectOf("controls-row");
  const line = `width=${width} slot=${JSON.stringify(slot)} toggle=${JSON.stringify(toggle)} row=${JSON.stringify(row)}`;
  if (!near(mid(slot).y, mid(toggle).y)) fail(`the window controls are off the row's centre: ${line}`);
  if (!near(mid(slot).y, mid(row).y)) fail(`the window controls are off the row's centre: ${line}`);
  if (slot.x < row.x || slot.x + slot.w > row.x + row.w + 1) fail(`the window controls sit outside their row: ${line}`);
  const shot = await capture(`controls-${width}`);
  if (shot) {
    const red = redCentroid(shot, { x: slot.x - 4, y: 0, w: slot.w + 8, h: row.y + row.h + 8 });
    if (red.n < 20) fail(`no close button drawn in the controls slot: ${line}`);
    // The close button is the slot's first 14 pt.
    if (!near(red.y, mid(slot).y, 1) || red.x < slot.x || red.x > slot.x + 16) {
      fail(`the close button is at ${red.x.toFixed(1)},${red.y.toFixed(1)}, not on its slot: ${line}`);
    }
    console.log(`  close button centre ${red.x.toFixed(1)},${red.y.toFixed(1)}`);
  }
  console.log(`  ND_ARC_CONTROLS_OK ${line}`);
}

async function cardLeg(expectLeading: number | null, tag: string): Promise<Rect> {
  const card = await settle("card");
  const win = await windowRect();
  const M = 8;
  const line = `${tag} card=${JSON.stringify(card)} window=${win.w}x${win.h}`;
  if (!near(card.y, M)) fail(`the card's top margin is not ${M}: ${line}`);
  if (!near(win.w - (card.x + card.w), M)) fail(`the card's trailing margin is not ${M}: ${line}`);
  if (!near(win.h - (card.y + card.h), M)) fail(`the card's bottom margin is not ${M}: ${line}`);
  if (expectLeading !== null && !near(card.x, expectLeading)) fail(`the card's leading edge is not at ${expectLeading}: ${line}`);
  const shot = await capture(`card-${tag}`);
  if (shot) {
    const [corner, inside] = pixels(shot, `
print(json.dumps([list(px(${card.x + 0.5},${card.y + card.h - 1})), list(px(${card.x + 30},${card.y + card.h - 30}))]))
`) as number[][];
    if (!isPageBlue(inside!)) fail(`the page does not reach the card's inside: ${JSON.stringify(inside)} ${line}`);
    if (isPageBlue(corner!)) fail(`the page's square corner shows past the card's curve: ${JSON.stringify(corner)} ${line}`);
  }
  console.log(`  ND_ARC_CARD_OK ${line}`);
  return card;
}

try {
  // ---- legs 1 and 2 at a normal and a narrow width ------------------------
  for (const width of [1100, 760]) {
    await controlsLeg(width);
    const sidebar = await rectOf("sidebar");
    await cardLeg(null, `open-${width}`);
    void sidebar;
  }

  // ---- leg 3: a click in the controls row lands on its button -------------
  if (gtk) await app.getByTestId("reload").click();
  else await app.cursor.click(app.getByTestId("reload"));
  await poll(() => textOf("state"), (t) => t.includes("clicks=1"), { timeoutMs: T });
  console.log("  ND_ARC_ROW_CLICK_OK the reload button in the controls row took its click");

  // ---- leg 4: hide the sidebar --------------------------------------------
  if (gtk) await app.getByTestId("toggle").click();
  else await app.cursor.click(app.getByTestId("toggle"));
  await poll(() => textOf("card-state"), (t) => t.includes("hidden=true"), { timeoutMs: T });
  const hiddenCard = await cardLeg(8, "hidden");
  const shot = await capture("hidden");
  if (shot) {
    const red = redCentroid(shot, { x: 0, y: 0, w: 120, h: 60 });
    if (red.n > 4) fail(`the window controls are still drawn with the sidebar hidden (${red.n} red pixels)`);
  }
  console.log("  ND_ARC_HIDDEN_OK the controls went with the sidebar and the card spans the window");

  // ---- leg 5: edge reveal --------------------------------------------------
  if (gtk) await app.getByTestId("reveal").click();
  else await app.cursor.move({ x: 2, y: 360 }, { steps: 6 });
  await poll(() => textOf("card-state"), (t) => t.includes("revealed=true"), { timeoutMs: T });
  // The panel is where the controls went: they are on screen again, on the
  // row inside the floating sidebar, and the page under it kept its size.
  await Bun.sleep(400);
  const slot = await rectOf(await controlsSlotId());
  const toggle = await rectOf("toggle");
  if (!near(mid(slot).y, mid(toggle).y)) fail(`revealed controls are off their row: ${JSON.stringify({ slot, toggle })}`);
  const cardNow = await rectOf("card");
  if (JSON.stringify(cardNow) !== JSON.stringify(hiddenCard)) {
    fail(`the reveal resized the page: ${JSON.stringify(hiddenCard)} -> ${JSON.stringify(cardNow)}`);
  }
  const revealShot = await capture("revealed");
  if (revealShot) {
    const red = redCentroid(revealShot, { x: slot.x - 4, y: 0, w: slot.w + 8, h: slot.y + slot.h + 12 });
    if (red.n < 20 || !near(red.y, mid(slot).y, 1)) fail(`the revealed sidebar's close button is not on its slot (${JSON.stringify(red)})`);
  }
  console.log(`  ND_ARC_REVEAL_OK the sidebar came in over an unchanged card ${JSON.stringify(cardNow)}`);

  if (gtk) await app.getByTestId("conceal").click();
  else await app.cursor.move({ x: 700, y: 360 }, { steps: 6 });
  await poll(() => textOf("card-state"), (t) => t.includes("revealed=false"), { timeoutMs: T });
  console.log("  ND_ARC_CONCEAL_OK the sidebar left when the pointer did");

  // Back to a shown sidebar for the drag.
  if (gtk) await app.getByTestId("show").click();
  else await app.cursor.click(app.getByTestId("show"));
  await poll(() => textOf("card-state"), (t) => t.includes("hidden=false"), { timeoutMs: T });

  // ---- leg 6: the controls row moves the window ---------------------------
  if (!gtk) {
    const before = await windowRect();
    const gap = await rectOf("controls-gap");
    const from = mid(gap);
    await app.cursor.drag(from, { x: from.x + 60, y: from.y + 40 });
    const after = await poll(windowRect, (w) => w.x !== before.x || w.y !== before.y, { timeoutMs: T });
    if (!near(after.x - before.x, 60, 3) || !near(after.y - before.y, 40, 3)) {
      fail(`the window moved by ${after.x - before.x},${after.y - before.y}, not 60,40`);
    }
    console.log(`  ND_ARC_HANDLE_OK the controls row moved the window by ${after.x - before.x},${after.y - before.y}`);
  }

  console.log("ND_ARCCHROME_OK");
} finally {
  if ("close" in app && typeof app.close === "function") await app.close();
}
