#!/usr/bin/env bun
// The legs scripts/headless-app-chrome.sh runs against a real app under
// ND_CEF_STYLE=chrome, once per rig (ND_ACCEPT_RIG: "x11" for Xvfb plus a
// reparenting window manager, "wlr" for headless sway plus XWayland).
//
// Three sources have to agree before a leg passes: the X window tree (the
// embedding container's geometry against CEF's own window inside it), CDP
// against the page (innerWidth/innerHeight, focus, typed text, scroll
// position), and a capture the leg reads pixels out of. Input is real: the
// pointer moves through the compositor and buttons and keys go through XTEST,
// because the whole class of bug here is windows and input, which an
// automation-socket shortcut steps over.
import { readFileSync } from "node:fs";
import { connectApp } from "@nativedesktop/test";
import { Session, targets, waitForTarget } from "./cdp.ts";

const rig = (process.env.ND_ACCEPT_RIG ?? "x11") as "x11" | "wlr";
const port = Number(process.env.ND_CDP_PORT ?? "9555");
const fixture = process.env.ND_ACCEPT_FIXTURE ?? "http://127.0.0.1:9557/";
const shots = process.env.ND_ACCEPT_SHOTS ?? "/tmp";
const hostLog = process.env.ND_ACCEPT_HOST_LOG ?? "";
const scale = Number(process.env.ND_ACCEPT_SCALE ?? "1");

const hostPid = Number(process.env.ND_ACCEPT_HOST_PID ?? "0");
const legBudgetMs = Number(process.env.ND_ACCEPT_LEG_BUDGET_MS ?? "180000");

const failures: string[] = [];
const skipped: string[] = [];

let lastProgress = Date.now();
let lastLeg = "startup";

function check(name: string, ok: boolean, detail: string): void {
  console.log(`  ${name}: ${ok ? "ok" : "FAIL"} (${detail})`);
  if (!ok) failures.push(`${name}: ${detail}`);
  lastLeg = name;
  lastProgress = Date.now();
}

function skip(name: string, why: string): void {
  console.log(`  ${name}: skip (${why})`);
  skipped.push(`${name}: ${why}`);
  lastLeg = name;
  lastProgress = Date.now();
}

function hostLogTail(lines = 25): string {
  if (!hostLog) return "";
  try {
    return readFileSync(hostLog, "utf8").split("\n").slice(-lines).join("\n");
  } catch {
    return "";
  }
}

function die(why: string): never {
  console.error(`ND_APP_CHROME_FAIL(${rig}) ${why}`);
  console.error(`  last leg to report: ${lastLeg}`);
  const tail = hostLogTail();
  if (tail) console.error(`  host log tail:\n${tail}`);
  process.exit(1);
}

/// Nothing in a leg is allowed to wait forever: a browser that dies takes the
/// debugger socket and the automation socket with it, and a driver parked on
/// either is a gate that never finishes instead of one that reports a failure.
setInterval(() => {
  if (hostPid > 0) {
    try {
      process.kill(hostPid, 0);
    } catch {
      die(`the host process ${hostPid} is gone`);
    }
  }
  if (Date.now() - lastProgress > legBudgetMs) {
    die(`no leg reported for ${Math.round((Date.now() - lastProgress) / 1000)}s`);
  }
}, 5000);

function sh(...argv: string[]): string {
  const out = Bun.spawnSync(argv, { env: process.env as Record<string, string> });
  return out.stdout.toString().trim();
}

// ============================================================================
// The rig: everything the two environments do differently
// ============================================================================

const swaymsg = (...args: string[]): string => sh("swaymsg", "-t", "command", "--", ...args);

/// Absolute pointer position, through XTEST on both rigs. XWayland implements
/// XTestFakeMotionEvent against its own virtual pointer, so GTK sees an
/// ordinary crossing sequence; wlrctl's zwlr_virtual_pointer_v1 device is
/// created and destroyed per invocation, and the leave that its removal
/// produces takes GTK's crossing state with it, which loses every click on a
/// native widget.
function pointerAt(): { x: number; y: number } {
  const m = sh("xdotool", "getmouselocation").match(/x:(-?\d+)\s+y:(-?\d+)/);
  return { x: Number(m?.[1] ?? NaN), y: Number(m?.[2] ?? NaN) };
}

function pointerTo(x: number, y: number): void {
  for (let attempt = 0; attempt < 5; attempt++) {
    sh("xdotool", "mousemove", "--sync", String(x), String(y));
    const at = pointerAt();
    if (Math.abs(at.x - x) <= 1 && Math.abs(at.y - y) <= 1) return;
    // The wlroots seat has no pointer at all until something binds one, and
    // XTEST motion has nothing to drive until then.
    if (rig === "wlr") sh("wlrctl", "pointer", "move", "0", "0");
    Bun.sleepSync(150);
  }
}

const click = (button = 1) => sh("xdotool", "click", String(button));
const press = (button: number) => sh("xdotool", "mousedown", String(button));
const release = (button: number) => sh("xdotool", "mouseup", String(button));
const key = (chord: string) => sh("xdotool", "key", "--clearmodifiers", chord);
const typeText = (text: string) => sh("xdotool", "type", "--delay", "25", text);

function capture(path: string): void {
  if (rig === "x11") sh("import", "-window", "root", path);
  else sh("grim", path);
}

function resizeToplevel(id: string, w: number, h: number): void {
  if (rig === "x11") {
    sh("xdotool", "windowsize", id, String(w), String(h));
    return;
  }
  swaymsg("floating", "enable");
  swaymsg("resize", "set", "width", `${w}px`, "height", `${h}px`);
}

function moveToplevel(id: string, x: number, y: number): void {
  if (rig === "x11") sh("xdotool", "windowmove", id, String(x), String(y));
  else swaymsg("move", "position", String(x), String(y));
}

function maximize(id: string, on: boolean): void {
  if (rig === "x11") {
    sh("wmctrl", "-i", "-r", id, "-b", `${on ? "add" : "remove"},maximized_vert,maximized_horz`);
    return;
  }
  // Tiled to the workspace is what a wlroots session calls maximized.
  swaymsg("floating", on ? "disable" : "enable");
}

function fullscreen(id: string, on: boolean): void {
  if (rig === "x11") sh("wmctrl", "-i", "-r", id, "-b", `${on ? "add" : "remove"},fullscreen`);
  else swaymsg("fullscreen", on ? "enable" : "disable");
}

function closeFocusedWindow(id: string): void {
  if (rig === "x11") sh("wmctrl", "-i", "-c", id);
  else swaymsg("kill");
}

// ============================================================================
// The X window tree
// ============================================================================

interface Geom { x: number; y: number; w: number; h: number; mapped: boolean }

function geom(id: string): Geom | null {
  const out = sh("xwininfo", "-id", id);
  const num = (label: string) => Number(out.match(new RegExp(`${label}:\\s+(-?\\d+)`))?.[1] ?? NaN);
  const x = num("Absolute upper-left X");
  if (Number.isNaN(x)) return null;
  return {
    x,
    y: num("Absolute upper-left Y"),
    w: num("Width"),
    h: num("Height"),
    mapped: out.includes("Map State: IsViewable"),
  };
}

/// Top-levels a user could see: mapped, at least 200x200, and not an
/// override-redirect popup. Chromium keeps several 1x1 and 10x10 utility
/// windows on the root that are never presented, and its menus and dropdowns
/// are override-redirect by construction; counting either would make the
/// no-top-level invariant unfalsifiable. Keyed by window id alone, because a
/// top-level that merely changed size is what half these legs do on purpose.
function census(): Map<string, string> {
  const rows = new Map<string, string>();
  for (const line of sh("xwininfo", "-root", "-children").split("\n")) {
    const m = line.match(/^\s*(0x[0-9a-f]+)\s+(".*?"|\(has no name\)).*?\s(\d+)x(\d+)\+/);
    if (!m) continue;
    const [, id, name, w, h] = m;
    if (Number(w) < 200 || Number(h) < 200) continue;
    const info = sh("xwininfo", "-id", id);
    if (!info.includes("Map State: IsViewable")) continue;
    if (info.includes("Override Redirect State: yes")) continue;
    rows.set(id, `${id} ${name} ${w}x${h}`);
  }
  return rows;
}

/// Override-redirect windows are how Chromium draws every popup it owns: the
/// context menu, a <select> dropdown, a tooltip, autofill. They are children of
/// the root rather than of the app, so this is what proves one appeared.
function overrideRedirect(minW = 30, minH = 20): Array<{ id: string } & Geom> {
  const found: Array<{ id: string } & Geom> = [];
  for (const line of sh("xwininfo", "-root", "-children").split("\n")) {
    const id = line.match(/^\s*(0x[0-9a-f]+)\s/)?.[1];
    if (!id) continue;
    const info = sh("xwininfo", "-id", id);
    if (!info.includes("Override Redirect State: yes")) continue;
    if (!info.includes("Map State: IsViewable")) continue;
    const g = geom(id);
    if (!g || g.w < minW || g.h < minH) continue;
    if (g.x < -100 || g.y < -100) continue;
    found.push({ id, ...g });
  }
  return found;
}

/// The app's toplevel: the biggest mapped window the host owns. `xdotool
/// search` matches on WM_CLASS, which the host sets to its binary name.
function toplevelId(): string {
  const ids = sh("xdotool", "search", "--classname", "nd-hello").split("\n").filter(Boolean);
  let best = "";
  let area = 0;
  for (const dec of ids) {
    const id = `0x${Number(dec).toString(16)}`;
    const g = geom(id);
    if (!g || !g.mapped) continue;
    if (g.w * g.h > area) { area = g.w * g.h; best = id; }
  }
  return best;
}

/// node id -> { container, cef } straight out of the host's own trace, which
/// names both windows as it creates them. Deriving them from the tree instead
/// would have to guess which child of the toplevel is an embedding container,
/// and that guess breaks the moment devtools docks a second window inside one.
function embeddedViews(): Map<number, { container: string; cef: string; target: string }> {
  const map = new Map<number, { container: string; cef: string; target: string }>();
  if (!hostLog) return map;
  let lines: string[] = [];
  try {
    lines = readFileSync(hostLog, "utf8").split("\n");
  } catch {
    return map;
  }
  for (const line of lines) {
    const e = line.match(/embed node=(\d+) parent=\S+ container=(0x[0-9a-f]+)/);
    if (e) {
      const prev = map.get(Number(e[1]));
      map.set(Number(e[1]), { container: e[2]!, cef: prev?.cef ?? "", target: prev?.target ?? "" });
    }
    const c = line.match(/created node=(\d+) cefWindow=(0x[0-9a-f]+)/);
    if (c) {
      const prev = map.get(Number(c[1]));
      if (prev) prev.cef = c[2]!;
    }
    // The main frame's id IS the debugger target id, and this is the host
    // saying which view it belongs to; matching a session by window size was a
    // guess that put the drive on a parked tab.
    const f = line.match(/mainFrame node=(\d+) frame=(\S+)/);
    if (f) {
      const prev = map.get(Number(f[1]));
      if (prev) prev.target = f[2]!;
    }
  }
  for (const [node, v] of [...map]) if (!v.cef || v.cef === "0x0") map.delete(node);
  return map;
}

/// The view the page is showing: its container is on screen rather than parked
/// off to the side, and CEF's window is inside it.
function shownView(): { node: number; container: Geom; cef: Geom; ids: { container: string; cef: string; target: string } } | null {
  for (const [node, ids] of embeddedViews()) {
    const container = geom(ids.container);
    const cef = geom(ids.cef);
    if (!container || !cef) continue;
    if (container.x < -1000 || !container.mapped) continue;
    return { node, container, cef, ids };
  }
  return null;
}

function parkedViews(): Array<{ node: number; container: Geom }> {
  const out: Array<{ node: number; container: Geom }> = [];
  for (const [node, ids] of embeddedViews()) {
    const container = geom(ids.container);
    if (container && container.x < -1000) out.push({ node, container });
  }
  return out;
}

// ============================================================================
// The page
// ============================================================================

interface Metrics {
  w: number; h: number; dpr: number; focus: boolean; probe: string; active: string;
  scrollY: number; picked: string; selection: string; fullscreen: string; events: string[];
}

const app = await connectApp();
let page: Session;

async function metrics(): Promise<Metrics> {
  return JSON.parse(await page.eval<string>("JSON.stringify(nd.metrics())"));
}

async function rect(id: string): Promise<{ x: number; y: number; w: number; h: number }> {
  return JSON.parse(await page.eval<string>(`JSON.stringify(nd.rect(${JSON.stringify(id)}))`));
}

/// Page CSS pixels to root-window pixels. The container's origin is already in
/// device pixels, which is the space the X server and every capture speak.
async function pageToScreen(id: string): Promise<{ x: number; y: number }> {
  let view = shownView();
  // A leg that has just changed the app's tabs can be ahead of the view coming
  // back; waiting beats throwing and taking the rest of the run with it.
  for (let i = 0; i < 40 && !view; i++) {
    await Bun.sleep(150);
    view = shownView();
  }
  if (!view) throw new Error("no shown view");
  const r = await rect(id);
  const m = await metrics();
  // Clamped to the viewport: an element taller than the window (the scroll
  // target is 4000px) has its centre off screen, and a pointer parked there is
  // not over the page at all.
  const cx = Math.min(Math.max(r.x + r.w / 2, 8), m.w - 8);
  const cy = Math.min(Math.max(r.y + r.h / 2, 8), m.h - 8);
  return { x: Math.round(view.container.x + cx * m.dpr), y: Math.round(view.container.y + cy * m.dpr) };
}

/// The screen position of the window's top-left in logical units, calibrated
/// against the one widget whose screen position is known exactly: the view,
/// whose embedding container the X server can be asked about. Deriving it from
/// the toplevel's own X geometry instead is off by the client-side decorations,
/// which differ between the two rigs.
async function windowOrigin(): Promise<{ x: number; y: number } | null> {
  const view = shownView();
  if (!view) return null;
  const tree = await app.tree();
  let box: { x: number; y: number } | null = null;
  type Node = { type?: string; geometry?: { x: number; y: number; w: number; h: number } | null; children?: Node[] };
  const walk = (n: Node): void => {
    // The view that is on screen, matched by size: a background tab is a
    // WebView node too, and it reports a geometry of its own.
    if (n.type === "WebView" && n.geometry
      && Math.abs(n.geometry.w * scale - view.container.w) <= 2
      && Math.abs(n.geometry.h * scale - view.container.h) <= 2) {
      box = { x: n.geometry.x, y: n.geometry.y };
    }
    for (const child of n.children ?? []) walk(child);
  };
  walk((tree as { root: Node }).root);
  if (!box) return null;
  return { x: view.container.x - box.x * scale, y: view.container.y - box.y * scale };
}

/// A native widget's centre in root-window pixels. getTree reports logical
/// units relative to the window's top-left.
async function widgetToScreen(testId: string): Promise<{ x: number; y: number } | null> {
  const box = await app.getByTestId(testId).boundingBox().catch(() => null);
  const origin = await windowOrigin();
  if (!box || !origin) return null;
  return { x: Math.round(origin.x + (box.x + box.width / 2) * scale), y: Math.round(origin.y + (box.y + box.height / 2) * scale) };
}

/// Puts the keyboard on the app's own chrome rather than the page, the way a
/// user does: the pointer goes to the address field and clicks it.
async function focusNativeChrome(at: { x: number; y: number }): Promise<void> {
  pointerTo(at.x, at.y);
  click(1);
}

/// Polls until the browser's own window matches the container it sits in and
/// the page has laid out for it. One frame budget is generous here on purpose:
/// the layout crosses a thread hop into CEF and then a compositor frame.
async function settled(timeoutMs = 4000): Promise<{ ok: boolean; detail: string }> {
  const deadline = Date.now() + timeoutMs;
  let detail = "no view";
  while (Date.now() < deadline) {
    const view = shownView();
    if (view) {
      const m = await metrics().catch(() => null);
      const cefMatches = view.cef.w === view.container.w && view.cef.h === view.container.h;
      const pageMatches = m !== null
        && Math.abs(Math.round(m.w * m.dpr) - view.container.w) <= 1
        && Math.abs(Math.round(m.h * m.dpr) - view.container.h) <= 1;
      detail = `container ${view.container.w}x${view.container.h}, cef ${view.cef.w}x${view.cef.h}`
        + (m ? `, page ${m.w}x${m.h}@${m.dpr}` : ", page unreachable");
      if (cefMatches && pageMatches) return { ok: true, detail };
    }
    await Bun.sleep(100);
  }
  return { ok: false, detail };
}


/// The two directions of keyboard routing, asserted with the pointer parked on
/// the OTHER half of the window each time: where a key press goes has to
/// follow the focused widget, and a user who moved the mouse over the page
/// while typing a URL must still be typing a URL.
async function focusRouting(label: string): Promise<void> {
  const field = await widgetToScreen("omnibox");
  const pageAt = await pageToScreen("probe");
  if (!field) {
    skip(`${label}.toField`, "no omnibox bounding box");
    skip(`${label}.toPage`, "no omnibox bounding box");
    return;
  }

  // Each direction starts from a known state: these legs run several times over
  // one session, and a caret left mid-string turns an exact comparison into
  // noise.
  await page.eval("document.getElementById('probe').value=''; document.getElementById('probe').blur(); 'reset'");
  await app.getByTestId("omnibox").fill("nd").catch(() => {});

  // Address field focused, pointer over the page.
  pointerTo(field.x, field.y);
  click(1);
  await Bun.sleep(700);
  key("End");
  const fieldBefore = (await app.getByTestId("omnibox").inputValue().catch(() => "")) ?? "";
  const pageBefore = (await metrics()).probe;
  pointerTo(pageAt.x, pageAt.y);
  await Bun.sleep(300);
  typeText("URLBAR");
  await Bun.sleep(900);
  const fieldAfter = (await app.getByTestId("omnibox").inputValue().catch(() => "")) ?? "";
  const pageAfterField = (await metrics()).probe;
  check(
    `${label}.toField`,
    fieldAfter === `${fieldBefore}URLBAR` && pageAfterField === pageBefore,
    `field ${JSON.stringify(fieldBefore)} -> ${JSON.stringify(fieldAfter)}, page ${JSON.stringify(pageBefore)} -> ${JSON.stringify(pageAfterField)}, pointer over the page`,
  );
  key("Escape");
  await Bun.sleep(300);

  // Page focused, pointer over the address field.
  await page.eval("document.getElementById('probe').value=''; 'reset'");
  pointerTo(pageAt.x, pageAt.y);
  click(1);
  await Bun.sleep(900);
  key("End");
  const fieldBefore2 = (await app.getByTestId("omnibox").inputValue().catch(() => "")) ?? "";
  const pageBefore2 = (await metrics()).probe;
  pointerTo(field.x, field.y);
  await Bun.sleep(300);
  typeText("INPAGE");
  await Bun.sleep(900);
  const fieldAfter2 = (await app.getByTestId("omnibox").inputValue().catch(() => "")) ?? "";
  const pageAfter2 = (await metrics()).probe;
  check(
    `${label}.toPage`,
    pageAfter2 === `${pageBefore2}INPAGE` && fieldAfter2 === fieldBefore2,
    `page ${JSON.stringify(pageBefore2)} -> ${JSON.stringify(pageAfter2)}, field ${JSON.stringify(fieldBefore2)} -> ${JSON.stringify(fieldAfter2)}, pointer over the address field`,
  );
}

/// The debugger session has to be on the view that is on screen. Switching
/// tabs moves which of the app's browsers that is, and a session left on a
/// parked one reports the park size and sees none of the input the legs send.
let pageTargetId = "";

async function resyncPage(): Promise<void> {
  const view = shownView();
  if (!view || !view.ids.target || view.ids.target === pageTargetId) return;
  const wanted = (await targets(port)).find((t) => t.id === view.ids.target && t.webSocketDebuggerUrl);
  if (!wanted) return;
  const candidate = await Session.open(wanted.webSocketDebuggerUrl!).catch(() => null);
  if (!candidate) return;
  page.close();
  page = candidate;
  pageTargetId = wanted.id;
  await page.send("Runtime.enable");
}

/// Fails the leg if any top-level appeared that was not there before it. A
/// Chromium window of its own is the invariant chrome style exists to keep.
let baseline = new Map<string, string>();
function noStray(name: string): void {
  const after = census();
  const added = [...after].filter(([id]) => !baseline.has(id)).map(([, row]) => row);
  baseline = after;
  if (added.length > 0) check(`${name}.census`, false, `stray top-level ${added.join(" | ")}`);
}

// ============================================================================
// Legs
// ============================================================================

const pageTarget = await waitForTarget(port, (t) => t.type === "page" && t.url === fixture, 120000);
page = await Session.open(pageTarget.webSocketDebuggerUrl!);
pageTargetId = pageTarget.id;
await page.send("Runtime.enable");
await resyncPage();

const top = toplevelId();
if (!top) { console.error("ND_APP_CHROME_FAIL no nd-hello toplevel on this display"); process.exit(1); }
// Start from a known shape on both rigs: floating and 1280x800.
if (rig === "wlr") swaymsg("floating", "enable");
resizeToplevel(top, 1280, 800);
moveToplevel(top, 40, 40);
await Bun.sleep(1500);
baseline = census();

const hasApp = (await app.getByTestId("omnibox").isVisible().catch(() => false)) === true;

{
  const s = await settled();
  check("initialSize", s.ok, s.detail);
  noStray("initialSize");
}

for (const [w, h] of [[1500, 950], [820, 620], [1760, 1080], [700, 520]] as Array<[number, number]>) {
  resizeToplevel(top, w, h);
  await Bun.sleep(400);
  const s = await settled();
  check(`resize ${w}x${h}`, s.ok, s.detail);
  noStray(`resize ${w}x${h}`);
}

{
  // Twenty sizes with no settle between them, then one settle at the end: a
  // layout that only converges when it is left alone still has to converge.
  let rand = 1234567;
  const next = (lo: number, hi: number) => {
    rand = (rand * 1103515245 + 12345) & 0x7fffffff;
    return lo + (rand % (hi - lo));
  };
  for (let i = 0; i < 20; i++) {
    resizeToplevel(top, next(640, 1900), next(480, 1150));
    await Bun.sleep(60);
  }
  resizeToplevel(top, 1280, 800);
  const s = await settled(6000);
  check("resizeRapid", s.ok, `after 20 sizes: ${s.detail}`);
  noStray("resizeRapid");
}

{
  maximize(top, true);
  await Bun.sleep(900);
  const big = geom(top)!;
  const s1 = await settled();
  check("maximize", s1.ok && big.w >= 1900, `${big.w}x${big.h}; ${s1.detail}`);
  maximize(top, false);
  await Bun.sleep(900);
  if (rig === "wlr") resizeToplevel(top, 1280, 800);
  const s2 = await settled();
  check("unmaximize", s2.ok, s2.detail);
  noStray("maximize");
}

{
  fullscreen(top, true);
  await Bun.sleep(1200);
  const s1 = await settled();
  const full = geom(top)!;
  check("fullscreen", s1.ok && full.w >= 1900 && full.h >= 1190, `${full.w}x${full.h}; ${s1.detail}`);
  fullscreen(top, false);
  await Bun.sleep(1200);
  if (rig === "wlr") resizeToplevel(top, 1280, 800);
  const s2 = await settled();
  check("unfullscreen", s2.ok, s2.detail);
  noStray("fullscreen");
}

{
  const before = shownView();
  moveToplevel(top, 260, 150);
  await Bun.sleep(800);
  const after = shownView();
  const moved = !!before && !!after && after.container.x !== before.container.x;
  const s = await settled();
  check("moveWindow", s.ok && moved, `container origin ${before?.container.x},${before?.container.y} -> ${after?.container.x},${after?.container.y}; ${s.detail}`);
  noStray("moveWindow");
}

if (hasApp) {
  const before = shownView()!;
  await app.getByTestId("layout-toggle").click();
  await Bun.sleep(1200);
  const s1 = await settled();
  const compact = shownView()!;
  check(
    "compactLayout",
    s1.ok && compact.container.x < before.container.x && compact.container.w > before.container.w,
    `container ${before.container.w}@${before.container.x} -> ${compact.container.w}@${compact.container.x}; ${s1.detail}`,
  );
  await app.getByTestId("layout-toggle").click();
  await Bun.sleep(1200);
  const s2 = await settled();
  check("sidebarLayout", s2.ok, s2.detail);
  noStray("compactLayout");
} else {
  skip("compactLayout", "the app under test has no layout-toggle");
  skip("sidebarLayout", "the app under test has no layout-toggle");
}

if (hasApp) {
  // Two tabs, switched with the app's own accelerator so the key travels the
  // real path. The tab that is not showing must be parked off screen, and the
  // one that comes back has to be the size the window is NOW, not the size it
  // was when it was last shown.
  key("ctrl+Tab");
  await Bun.sleep(3000);
  const second = shownView();
  const parked = parkedViews();
  check("twoTabsSwitch", !!second && parked.length >= 1, `showing node ${second?.node}, ${parked.length} parked`);
  resizeToplevel(top, 1500, 1000);
  await Bun.sleep(900);
  key("ctrl+Tab");
  await Bun.sleep(2500);
  const back = await settled(8000);
  check("tabResizedWhileHidden", back.ok, back.detail);
  const stillParked = parkedViews();
  check("hiddenTabParked", stillParked.length >= 1, `${stillParked.length} container(s) off screen`);
  noStray("twoTabs");
  resizeToplevel(top, 1280, 800);
  await Bun.sleep(700);
  await settled();
  await resyncPage();
  await focusRouting("focusRoutingAfterTabSwitch");
} else {
  skip("twoTabsSwitch", "the app under test has one view");
  skip("tabResizedWhileHidden", "the app under test has one view");
  skip("hiddenTabParked", "the app under test has one view");
}

{
  // Keyboard into the page: a real click to put focus in the field, real keys
  // after it. Nothing here goes through the automation socket.
  await page.eval("document.getElementById('probe').value=''; '1'");
  const at = await pageToScreen("probe");
  pointerTo(at.x, at.y);
  click(1);
  await Bun.sleep(1200);
  key("End");
  const clicked = await metrics();
  typeText("chrome-accept");
  await Bun.sleep(1000);
  const m = await metrics();
  check(
    "pageTyping",
    m.probe === "chrome-accept" && m.active === "probe",
    `probe=${JSON.stringify(m.probe)} active=${m.active} focus after the click=${clicked.focus}, after typing=${m.focus}`,
  );
  noStray("pageTyping");
}

if (hasApp) {
  // And out of it again: the native address field takes the keyboard back, and
  // what is typed must not also reach the page.
  const at = await widgetToScreen("omnibox");
  if (!at) {
    skip("addressFieldTyping", "no omnibox bounding box");
  } else {
    await focusNativeChrome(at);
    await Bun.sleep(600);
    typeText("nd-address");
    await Bun.sleep(800);
    const value = (await app.getByTestId("omnibox").inputValue().catch(() => "")) ?? "";
    const m = await metrics();
    check(
      "addressFieldTyping",
      value.includes("nd-address") && m.probe === "chrome-accept",
      `clicked ${at.x},${at.y}; omnibox=${JSON.stringify(value.slice(0, 60))} page probe=${JSON.stringify(m.probe)}`,
    );
    key("Escape");
    await Bun.sleep(400);
  }
  noStray("addressFieldTyping");
} else {
  skip("addressFieldTyping", "the app under test has no omnibox");
}

if (hasApp) {
  await focusRouting("focusRouting");
  noStray("focusRouting");
} else {
  skip("focusRouting.toField", "the app under test has no omnibox");
  skip("focusRouting.toPage", "the app under test has no omnibox");
}

if (hasApp) {
  // An accelerator the app owns, pressed while the page holds the keyboard.
  const pageAt = await pageToScreen("probe");
  pointerTo(pageAt.x, pageAt.y);
  click(1);
  await Bun.sleep(800);
  const before = await app.tree();
  const countTabs = (tree: unknown): number => {
    let n = 0;
    const walk = (x: { testID?: string | null; children?: unknown[] }): void => {
      if (typeof x.testID === "string" && /^tabs-menu-\d+$/.test(x.testID)) n++;
      for (const child of (x.children ?? []) as never[]) walk(child);
    };
    walk((tree as { root: never }).root);
    return n;
  };
  const wasCompact = countTabs(before);
  key("ctrl+t");
  await Bun.sleep(2000);
  const after = await app.tree();
  const opened = JSON.stringify(after).length !== JSON.stringify(before).length;
  check("appShortcutWhilePageFocused", opened, `tree changed=${opened} (compact rows before ${wasCompact})`);
  // ctrl+t leaves a tab with no page in it, and every leg below needs a view
  // on screen; the app's own close-tab accelerator puts it back.
  key("ctrl+w");
  await Bun.sleep(2500);
  const restored = await settled(8000);
  check("appShortcutUndone", restored.ok, restored.detail);
  noStray("appShortcut");
} else {
  skip("appShortcutWhilePageFocused", "the app under test has no accelerators");
}

await resyncPage();
{
  const at = await pageToScreen("tall");
  pointerTo(at.x, at.y);
  await page.eval("nd.reset()");
  for (let i = 0; i < 6; i++) sh("xdotool", "click", "5");
  await Bun.sleep(900);
  const m = await metrics();
  check("wheelScroll", m.scrollY > 0, `scrollY=${m.scrollY} events=${m.events.join(",")}`);
  await page.eval("scrollTo(0,0)");
  noStray("wheelScroll");
}

if (hasApp) {
  // Tab past the page's last focusable element: the browser reports it is
  // giving focus up, and the app's own chrome has to be able to take it.
  const at = await pageToScreen("probe");
  pointerTo(at.x, at.y);
  click(1);
  await Bun.sleep(800);
  for (let i = 0; i < 8; i++) {
    key("Tab");
    await Bun.sleep(250);
  }
  await Bun.sleep(700);
  const focusedNow = await app.tree();
  let focusedType = "none";
  const walk = (n: { type?: string; focused?: boolean; children?: unknown[] }): void => {
    if (n.focused) focusedType = `${n.type}`;
    for (const child of (n.children ?? []) as never[]) walk(child);
  };
  walk((focusedNow as { root: never }).root);
  check("tabTraversalLeavesThePage", focusedType !== "none" && focusedType !== "WebView", `GTK focus widget is ${focusedType}`);
  noStray("tabTraversal");
} else {
  skip("tabTraversalLeavesThePage", "the app under test has one widget tree");
}

await resyncPage();
{
  const r = await rect("sel");
  const view = shownView()!;
  const dpr = (await metrics()).dpr;
  const y = Math.round(view.container.y + (r.y + r.h / 2) * dpr);
  pointerTo(Math.round(view.container.x + (r.x + 4) * dpr), y);
  await Bun.sleep(300);
  press(1);
  for (let i = 1; i <= 6; i++) {
    pointerTo(Math.round(view.container.x + (r.x + (r.w * i) / 6) * dpr), y);
    await Bun.sleep(150);
  }
  release(1);
  await Bun.sleep(500);
  const m = await metrics();
  check("textSelectionDrag", m.selection.trim().length > 0, `selected ${JSON.stringify(m.selection.slice(0, 40))}`);
  noStray("textSelectionDrag");
}

await resyncPage();
{
  // A <select> popup is an override-redirect Chromium window, the same shape
  // as the context menu, the tooltip and the autofill bubble: if one of them
  // is missing or mispositioned they all are.
  const at = await pageToScreen("picker");
  pointerTo(at.x, at.y);
  click(1);
  await Bun.sleep(1200);
  const popups = overrideRedirect();
  const near = popups.find((p) => Math.abs(p.x - at.x) < 400 && p.y > at.y - 400 && p.y < at.y + 400);
  capture(`${shots}/select-dropdown.png`);
  check("selectDropdownOpens", !!near, near ? `${near.id} ${near.w}x${near.h}+${near.x}+${near.y} for a control at ${at.x},${at.y}` : `${popups.length} override-redirect window(s), none near the control`);
  if (near) {
    // Third row of the list: the option under it is what must be selected.
    // Four options fill the popup, so the third row's centre is 5/8 down; the
    // pointer is parked there first so the row is highlighted before the click.
    const rowY = near.y + Math.round((near.h * 5) / 8);
    pointerTo(near.x + Math.round(near.w / 2), rowY);
    await Bun.sleep(400);
    click(1);
    await Bun.sleep(1200);
    const m = await metrics();
    check("selectDropdownPicks", m.picked !== "alpha", `value=${m.picked} popup ${near.w}x${near.h}+${near.x}+${near.y}, clicked y=${rowY}`);
  } else {
    check("selectDropdownPicks", false, "no dropdown to pick from");
    key("Escape");
  }
  noStray("selectDropdown");
}

await resyncPage();
{
  // Chromium shows a tooltip only for a browser it believes has the keyboard,
  // so the page is clicked back into focus first, and the pointer dwells on the
  // element with small real moves rather than one jump onto it.
  const focusFirst = await pageToScreen("title");
  pointerTo(focusFirst.x, focusFirst.y);
  click(1);
  await Bun.sleep(600);
  const at = await pageToScreen("tip");
  pointerTo(at.x - 20, at.y);
  await Bun.sleep(400);
  let tip: ({ id: string } & Geom) | undefined;
  const deadline = Date.now() + 6000;
  let nudge = 0;
  while (Date.now() < deadline) {
    const offsets: Array<[number, number]> = [[0, 0], [3, 1], [-2, 1], [1, -1]];
    const [dx, dy] = offsets[nudge % offsets.length]!;
    nudge += 1;
    pointerTo(at.x + dx, at.y + dy);
    await Bun.sleep(600);
    tip = overrideRedirect(40, 12).find((p) => p.y > at.y - 120 && p.y < at.y + 160 && p.h < 120);
    if (tip) break;
  }
  capture(`${shots}/tooltip.png`);
  check(
    "tooltip",
    !!tip,
    tip
      ? `${tip.id} ${tip.w}x${tip.h}+${tip.x}+${tip.y}`
      : `pointer at ${at.x},${at.y} for ${Math.round((Date.now() - (deadline - 6000)) / 1000)}s; override-redirect windows: ${overrideRedirect(1, 1).map((p) => `${p.w}x${p.h}+${p.x}+${p.y}`).join(", ") || "none"}`,
  );
  pointerTo(at.x, at.y + 300);
  await Bun.sleep(600);
  noStray("tooltip");
}

await resyncPage();
{
  const at = await pageToScreen("title");
  pointerTo(at.x, at.y);
  await page.eval("nd.reset()");
  click(3);
  await Bun.sleep(1500);
  const menu = overrideRedirect(120, 80).find((p) => Math.abs(p.x - at.x) < 600 && Math.abs(p.y - at.y) < 600);
  capture(`${shots}/context-menu.png`);
  const m = await metrics();
  check(
    "contextMenu",
    !!menu && m.events.includes("contextmenu"),
    menu ? `${menu.id} ${menu.w}x${menu.h}+${menu.x}+${menu.y}, page saw ${m.events.join(",")}` : `no menu window; page saw ${m.events.join(",")}`,
  );
  key("Escape");
  await Bun.sleep(1500);
  const left = overrideRedirect(120, 80);
  check(
    "contextMenuDismisses",
    left.length === 0,
    left.length === 0 ? "escape closed it" : left.map((p) => `${p.id} ${p.w}x${p.h}+${p.x}+${p.y}`).join(" | "),
  );
  noStray("contextMenu");
}

await resyncPage();
{
  const focusFirst = await pageToScreen("title");
  pointerTo(focusFirst.x, focusFirst.y);
  click(1);
  await Bun.sleep(600);
  const at = await pageToScreen("stage");
  pointerTo(at.x, at.y);
  click(1);
  await Bun.sleep(2500);
  const inFs = await metrics();
  const view = shownView();
  check(
    "html5Fullscreen",
    inFs.fullscreen === "stage",
    `fullscreenElement=${JSON.stringify(inFs.fullscreen)}, container ${view?.container.w}x${view?.container.h}`,
  );
  capture(`${shots}/html5-fullscreen.png`);
  key("Escape");
  await Bun.sleep(1500);
  const out = await metrics();
  check("html5FullscreenExits", out.fullscreen === "", `fullscreenElement=${JSON.stringify(out.fullscreen)}`);
  const s = await settled();
  check("afterFullscreenLayout", s.ok, s.detail);
  noStray("html5Fullscreen");
}

if (hasApp) {
  // A second toplevel of the app itself. On a tiling compositor the first
  // window is resized by the compositor when it opens and again when it goes,
  // which is the resize path no window-manager-less gate can reach.
  if (rig === "wlr") {
    swaymsg("floating", "disable");
    await Bun.sleep(1200);
    await settled(6000);
  }
  // The app's accelerators are GTK's, and they only fire while the native
  // chrome has the keyboard rather than the page, so the omnibox is clicked
  // first exactly as a user would have to.
  const chromeAt = await widgetToScreen("omnibox");
  if (chromeAt) {
    await focusNativeChrome(chromeAt);
    await Bun.sleep(700);
  }
  const before = shownView()!;
  key("ctrl+shift+p");
  await Bun.sleep(2500);
  const afterOpen = shownView();
  const openOk = !!afterOpen && (rig === "x11" || afterOpen.container.w !== before.container.w);
  const s1 = await settled(6000);
  check("secondWindowOpens", openOk && s1.ok, `container ${before.container.w}x${before.container.h} -> ${afterOpen?.container.w}x${afterOpen?.container.h}; ${s1.detail}`);
  const others = sh("xdotool", "search", "--classname", "nd-hello").split("\n").filter(Boolean)
    .map((d) => `0x${Number(d).toString(16)}`)
    .filter((id) => id !== top && (geom(id)?.mapped ?? false) && (geom(id)?.w ?? 0) > 200);
  for (const id of others) closeFocusedWindow(id);
  await Bun.sleep(2500);
  const s2 = await settled(6000);
  check("secondWindowCloses", s2.ok && toplevelId() === top, `back to ${s2.detail}`);
  noStray("secondWindow");
  await focusRouting("focusRoutingAfterSecondWindow");
  if (rig === "wlr") {
    resizeToplevel(top, 1280, 800);
    await Bun.sleep(900);
    await settled(6000);
  }
} else {
  skip("secondWindowOpens", "the app under test has one window");
  skip("secondWindowCloses", "the app under test has one window");
}

{
  // DevTools last: a Chrome-style browser that has had the inspector open dies
  // on the way out of the process (docs/webview.md), so the inspector is shut
  // again before the gate asks the host to quit.
  const at = await pageToScreen("title");
  pointerTo(at.x, at.y);
  click(1);
  await Bun.sleep(500);
  key("F12");
  // Chrome's inspector is a browser launch; on this rig the first one costs a
  // process and a SwiftShader fallback, so the wait is generous.
  let dt: Awaited<ReturnType<typeof targets>> = [];
  for (let i = 0; i < 40; i++) {
    dt = (await targets(port)).filter((t) => t.url.startsWith("devtools://"));
    if (dt.length > 0) break;
    await Bun.sleep(500);
  }
  const docked = shownView();
  // By id, not by name: the docked inspector's title propagates to the app's
  // own toplevel, so matching /DevTools/ against the census would report the
  // window the app has always had as a stray one.
  const onRoot = [...census().keys()].filter((id) => !baseline.has(id));
  check("devToolsDocks", dt.length > 0 && onRoot.length === 0, `${dt.length} devtools target(s), ${onRoot.length} extra top-level(s)`);
  if (dt.length > 0 && docked) {
    const split = docked.cef.w < docked.container.w;
    check("devToolsSplitsTheView", split, `page window ${docked.cef.w} inside a ${docked.container.w} container`);
    resizeToplevel(top, 1600, 1000);
    await Bun.sleep(1500);
    const after = shownView()!;
    const m = await metrics();
    const splitAfter = after.cef.w < after.container.w && Math.abs(Math.round(m.w * m.dpr) - after.cef.w) <= 1;
    capture(`${shots}/devtools-docked.png`);
    check("devToolsResize", splitAfter, `page window ${after.cef.w} of ${after.container.w}, page ${m.w}@${m.dpr}`);
  } else {
    check("devToolsSplitsTheView", false, "no docked inspector");
    check("devToolsResize", false, "no docked inspector");
  }
  // The pointer went to the inspector's half during the resize leg, and F12 is
  // a Chrome accelerator the browser only sees while the page has the
  // keyboard, so the page is clicked back into focus first.
  const back = await pageToScreen("title");
  pointerTo(back.x, back.y);
  click(1);
  await Bun.sleep(800);
  if (hasApp) await focusRouting("focusRoutingWithDevTools");
  key("F12");
  await Bun.sleep(4000);
  const leftOver = (await targets(port)).filter((t) => t.url.startsWith("devtools://"));
  check("devToolsCloses", leftOver.length === 0, `${leftOver.length} devtools target(s) left`);
  resizeToplevel(top, 1280, 800);
  await Bun.sleep(1000);
  const s = await settled(8000);
  check("afterDevToolsLayout", s.ok, s.detail);
  noStray("devTools");
}

capture(`${shots}/final.png`);


if (skipped.length > 0) console.log(`  ${skipped.length} leg(s) skipped`);
if (failures.length > 0) {
  console.error(`ND_APP_CHROME_FAIL(${rig}) ${failures.length} leg(s) failed:`);
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}
console.log(`ND_APP_CHROME_LEGS_OK(${rig})`);
process.exit(0);
