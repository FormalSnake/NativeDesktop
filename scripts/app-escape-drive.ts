#!/usr/bin/env bun
// The no-escape routes on Linux, run by scripts/headless-app-chrome.sh with
// ND_ACCEPT_DRIVE=scripts/app-escape-drive.ts against the real app under
// ND_CEF_STYLE=chrome. Every key chord, page API, context menu item, chrome://
// control and extension call that can ask Chromium for a window, a tab strip,
// the app menu or a bubble is driven with real X input, and none of them may
// put a Chromium surface on screen. Each route also states what the app does
// instead (a new app window, a new app tab, nothing), and that is asserted.
// The peer of scripts/mac/app-escape-drive.ts.
//
// Observation per route: the X server's root children diffed against the set
// before the route (a Chromium toplevel, a bubble or a print dialog is a new
// mapped window whatever its size), the app's own window and tab counts, the
// page viewport (a Chromium toolbar or bookmark bar in the view takes rows
// from it) and the debugging port's page targets.
//
// The escape extension (examples/webview-probe/escape-ext) has to be loaded:
// the rig passes ND_ACCEPT_EXTENSIONS to the host. ND_ESCAPE_ROUTES runs a
// subset; ND_ESCAPE_EXPLORE=1 reports without failing.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { connectApp } from "@nativedesktop/test";
import { Session, targets } from "./cdp.ts";

const rig = process.env.ND_ACCEPT_RIG ?? "x11";
const cdpPort = Number(process.env.ND_CDP_PORT ?? "9555");
const hostLog = process.env.ND_ACCEPT_HOST_LOG ?? "";
const hostPid = Number(process.env.ND_ACCEPT_HOST_PID ?? "0");
const explore = process.env.ND_ESCAPE_EXPLORE === "1";
const only = (process.env.ND_ESCAPE_ROUTES ?? "").split(",").filter((n) => n.length > 0);
const fixturePort = Number(process.env.ND_ESCAPE_FIXTURE_PORT ?? String(Number(process.env.ND_CDP_PORT ?? "9555") + 7));
const ORIGIN = `http://127.0.0.1:${fixturePort}`;
const ESCAPE_URL = `${ORIGIN}/escape.html`;

// The mac gate's fixture directory is the one copy of the escape page.
const fixtureDir = join(import.meta.dir, "mac", "app-chrome-fixtures");
const server = Bun.serve({
  port: fixturePort,
  hostname: "127.0.0.1",
  async fetch(request) {
    const path = new URL(request.url).pathname;
    const file = Bun.file(join(fixtureDir, path === "/" ? "escape.html" : path));
    if (!(await file.exists())) return new Response("not found", { status: 404 });
    return new Response(file);
  },
});

function sh(...argv: string[]): string {
  const out = Bun.spawnSync(argv, { env: process.env as Record<string, string> });
  return out.stdout.toString().trim();
}

const key = (chord: string) => sh("xdotool", "key", "--clearmodifiers", chord);
const typeText = (text: string) => sh("xdotool", "type", "--delay", "20", text);
function pointerTo(x: number, y: number): void {
  sh("xdotool", "mousemove", "--sync", String(Math.round(x)), String(Math.round(y)));
}

let lastProgress = Date.now();
setInterval(() => {
  if (hostPid > 0) {
    try {
      process.kill(hostPid, 0);
    } catch {
      console.log(`ND_ESCAPE_HOST_GONE (${rig})`);
      process.exit(1);
    }
  }
  if (Date.now() - lastProgress > 240000) {
    console.log(`ND_ESCAPE_STALLED (${rig})`);
    process.exit(1);
  }
}, 5000);

async function until<T>(read: () => Promise<T>, ok: (v: T) => boolean, timeoutMs: number): Promise<T | null> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const v = await read();
      if (ok(v)) return v;
    } catch {
      // Not there yet.
    }
    await Bun.sleep(150);
  }
  return null;
}

// ============================================================================
// The X server
// ============================================================================

interface Win { id: string; x: number; y: number; w: number; h: number; or: boolean; name: string }

function geom(id: string): { x: number; y: number; w: number; h: number; mapped: boolean; or: boolean } | null {
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
    or: out.includes("Override Redirect State: yes"),
  };
}

/// Every mapped child of the root bigger than a utility window, with the first
/// name found in its subtree (a window manager frame carries none itself).
function rootWindows(): Win[] {
  const found: Win[] = [];
  for (const line of sh("xwininfo", "-root", "-children").split("\n")) {
    const id = line.match(/^\s*(0x[0-9a-f]+)\s/)?.[1];
    if (!id) continue;
    const g = geom(id);
    if (!g || !g.mapped || g.w < 40 || g.h < 30) continue;
    if (g.x + g.w < 0 || g.y + g.h < 0) continue;
    const name = sh("xwininfo", "-tree", "-id", id).match(/0x[0-9a-f]+ "([^"]+)"/)?.[1] ?? "";
    found.push({ id, x: g.x, y: g.y, w: g.w, h: g.h, or: g.or, name });
  }
  return found;
}

/// The embedding container of the view on screen, from the host's own trace.
function shownContainer(): { x: number; y: number; w: number; h: number } | null {
  if (!hostLog) return null;
  let text = "";
  try {
    text = readFileSync(hostLog, "utf8");
  } catch {
    return null;
  }
  const containers = new Set<string>();
  for (const m of text.matchAll(/embed node=\d+ parent=\S+ container=(0x[0-9a-f]+)/g)) containers.add(m[1]!);
  for (const id of containers) {
    const g = geom(id);
    if (!g || !g.mapped || g.x < -1000 || g.w < 100) continue;
    return g;
  }
  return null;
}

function toplevelId(): string {
  const ids = sh("xdotool", "search", "--classname", "nd-hello").split("\n").filter(Boolean);
  let best = "";
  let area = 0;
  for (const dec of ids) {
    const id = `0x${Number(dec).toString(16)}`;
    const g = geom(id);
    if (!g || !g.mapped || g.or) continue;
    if (g.w * g.h > area) {
      area = g.w * g.h;
      best = id;
    }
  }
  return best;
}

// ============================================================================
// The app
// ============================================================================

const app = await connectApp();
await until(async () => (await app.windows()).windows.length, (n) => n > 0, 45000);
const mainRef = (await app.windows()).windows[0]!.ref;

type Node = { type: string; testID: string | null; visible: boolean; rows: unknown[] | null; children: Node[] };

async function activePage(): Promise<string> {
  const tree = await app.tree(mainRef);
  let found: string | null = null;
  const walk = (node: Node) => {
    if (node.type === "WebView" && node.visible && node.testID?.startsWith("page-")) found ??= node.testID;
    for (const child of node.children) walk(child);
  };
  walk(tree.root as never);
  if (!found) throw new Error("no visible WebView");
  return found;
}

async function tabCount(): Promise<number> {
  const tree = await app.tree(mainRef);
  let count = 0;
  const walk = (node: Node) => {
    if (node.testID === "tab-list") count = node.rows?.length ?? 0;
    for (const child of node.children) walk(child);
  };
  walk(tree.root as never);
  return count;
}

/// The app's own popovers on show. GTK draws a popover as an override-redirect
/// window carrying the app's class, exactly like a Chromium bubble would be, so
/// the tree is what tells the two apart.
async function appPopovers(): Promise<{ w: number; h: number }[]> {
  const found: { w: number; h: number }[] = [];
  for (const w of (await app.windows()).windows) {
    const tree = await app.tree(w.ref);
    const walk = (node: Node & { geometry?: { w: number; h: number } | null }) => {
      if (node.type === "Popover" && node.visible && node.geometry) found.push(node.geometry);
      for (const child of node.children) walk(child as never);
    };
    walk(tree.root as never);
  }
  return found;
}

async function pageEval(testId: string, code: string): Promise<string | null> {
  const result = (await app.rpc.call("webviewEval", { testId, code, timeoutMs: 8000 })) as { ok: boolean; value?: string; error?: string };
  if (!result.ok) throw new Error(`webviewEval(${testId}): ${result.error ?? "failed"}`);
  return result.value ?? null;
}

function focusApp(): void {
  const id = toplevelId();
  // Bounded: --sync waits for the window manager to report the activation,
  // and one that never does would park the drive here.
  if (id) sh("timeout", "3", "xdotool", "windowactivate", "--sync", id);
}

/// Navigates the active tab through CDP's Page.navigate on its own target,
/// which reaches chrome:// addresses too and needs no keyboard focus: under
/// XWayland (wlr, hypr) keys go to the child window under the pointer, so
/// typing into the address field is not a route every rig can take. Typing is
/// the fallback for a tab with no page yet.
async function navigateActive(url: string): Promise<boolean> {
  const id = await activePage().catch(() => null);
  if (!id) return false;
  const current = await pageEval(id, "location.href").catch(() => null);
  if (!current) return false;
  const target = (await targets(cdpPort).catch(() => [])).find((t) => t.type === "page" && t.url === current && t.webSocketDebuggerUrl);
  if (!target) return false;
  const session = await Session.open(target.webSocketDebuggerUrl!);
  try {
    await session.send("Page.navigate", { url });
  } finally {
    session.close();
  }
  return true;
}

async function load(url: string, lands = url): Promise<string> {
  let last = "";
  for (let attempt = 0; attempt < 3; attempt++) {
    if (!(await navigateActive(url))) {
      focusApp();
      key("ctrl+l");
      await Bun.sleep(400);
      key("ctrl+a");
      typeText(url);
      key("Return");
    }
    const landed = await until(
      async () => {
        const id = await activePage();
        last = `${id}|${await pageEval(id, "location.href")}|${await pageEval(id, "document.readyState")}`;
        return last;
      },
      (v) => v.split("|")[1]!.startsWith(lands) && v.endsWith("|complete"),
      15000,
    );
    if (landed) return landed.split("|")[0]!;
    key("ctrl+t");
    await Bun.sleep(800);
  }
  throw new Error(`the address field never took ${url} (last seen ${last.split("|").slice(1).join(" ") || "nothing"})`);
}

/// A point in the page in root pixels: an element id, or a fraction of the view.
async function pagePoint(page: string, where: string | [number, number]): Promise<{ x: number; y: number }> {
  const view = await until(async () => shownContainer(), (v) => v !== null, 6000);
  if (!view) throw new Error("no shown view container");
  const dpr = Number(await pageEval(page, "devicePixelRatio")) || 1;
  if (Array.isArray(where)) return { x: view.x + view.w * where[0], y: view.y + view.h * where[1] };
  // The first point inside the element that the element itself is hit at: a
  // media control can overhang its grid cell and take the centre of the next.
  const code = `(() => {
    const el = document.getElementById(${JSON.stringify(where)});
    if (!el) return "null";
    const r = el.getBoundingClientRect();
    for (const fy of [0.5, 0.25, 0.75, 0.1, 0.9]) for (const fx of [0.5, 0.25, 0.75, 0.1]) {
      const x = r.x + r.width * fx, y = r.y + r.height * fy;
      const hit = document.elementFromPoint(x, y);
      if (hit && (hit === el || el.contains(hit))) return JSON.stringify({ x, y });
    }
    return JSON.stringify({ x: r.x + r.width / 2, y: r.y + r.height / 2 });
  })()`;
  const r = JSON.parse((await pageEval(page, code)) ?? "null") as { x: number; y: number } | null;
  if (!r) throw new Error(`no #${where} in the page`);
  return { x: view.x + r.x * dpr, y: view.y + r.y * dpr };
}

async function clickAt(at: { x: number; y: number }, button = 1, mods: string[] = []): Promise<void> {
  pointerTo(at.x, at.y);
  for (const m of mods) sh("xdotool", "keydown", m);
  sh("xdotool", "mousedown", String(button));
  await Bun.sleep(button === 3 ? 250 : 40);
  sh("xdotool", "mouseup", String(button));
  for (const m of mods) sh("xdotool", "keyup", m);
}

async function focusPage(page: string): Promise<void> {
  focusApp();
  await clickAt(await pagePoint(page, "quiet"));
  await Bun.sleep(300);
}

// ============================================================================
// Context menu
// ============================================================================

interface Item { depth: number; kind: string; enabled: boolean; label: string }

function shownMenu(): Item[] {
  if (!hostLog) return [];
  const lines = readFileSync(hostLog, "utf8").split("\n").filter((l) => l.includes("menuShown "));
  const start = lines.findLastIndex((l) => /depth=0 index=0 /.test(l));
  return (start < 0 ? [] : lines.slice(start)).flatMap((line) => {
    const m = /menuShown depth=(\d+) index=\d+ id=-?\d+ kind=(\w+) enabled=(\d) checked=\d (?:accel=\S* )?label=(.*)$/.exec(line);
    return m ? [{ depth: Number(m[1]), kind: m[2]!, enabled: m[3] === "1", label: m[4]! }] : [];
  });
}

async function menuRoute(page: string, where: string | [number, number], label: RegExp): Promise<string> {
  const before = hostLog ? readFileSync(hostLog, "utf8").split("\n").filter((l) => l.includes("menuShown depth=0 index=0 ")).length : 0;
  await clickAt(await pagePoint(page, where), 3);
  const opened = await until(
    async () => readFileSync(hostLog, "utf8").split("\n").filter((l) => l.includes("menuShown depth=0 index=0 ")).length,
    (n) => n > before,
    8000,
  );
  if (opened === null) return "no menu opened";
  await Bun.sleep(600);
  const top = shownMenu().filter((i) => i.depth === 0);
  const hit = top.find((i) => label.test(i.label));
  if (!hit || !hit.enabled) {
    key("Escape");
    return `not offered (${top.map((i) => i.label).join(" | ")})`;
  }
  // GTK puts the focus on the first sensitive item as the menu opens.
  const stops = top.slice(0, top.indexOf(hit) + 1).filter((i) => i.kind !== "separator" && i.enabled).length;
  for (let i = 1; i < stops; i++) key("Down");
  key("Return");
  return `picked ${hit.label}`;
}

const shots = process.env.ND_ACCEPT_SHOTS ?? "/tmp";

/// Chromium's own UI an item may never open from a menu in this browser.
const bannedItems = /incognito|view (page |frame )?source|^cast|qr code|send to (your )?devices|web store|open link as|in profile/i;

interface TreeItem { kind: string; label: string; children: TreeItem[] }

function menuTree(): TreeItem[] {
  const lines = readFileSync(hostLog, "utf8").split("\n").filter((l) => l.includes("menuShown "));
  const start = lines.findLastIndex((l) => /depth=0 index=0 /.test(l));
  const root: TreeItem[] = [];
  const stack: TreeItem[][] = [root];
  for (const line of start < 0 ? [] : lines.slice(start)) {
    const m = /menuShown depth=(\d+) index=\d+ id=-?\d+ kind=(\w+) enabled=\d checked=\d (?:accel=\S* )?label=(.*)$/.exec(line);
    if (!m) continue;
    const depth = Number(m[1]);
    const item: TreeItem = { kind: m[2]!, label: m[3]!, children: [] };
    stack.length = depth + 1;
    stack[depth]!.push(item);
    stack[depth + 1] = item.children;
  }
  return root;
}

/// The structural rules every drawn menu has to meet, level by level.
function menuProblems(items: TreeItem[], path = "top"): string[] {
  const problems: string[] = [];
  const labels = new Map<string, number>();
  items.forEach((item, i) => {
    if (item.kind === "separator") {
      if (i === 0) problems.push(`${path}: leading separator`);
      if (i === items.length - 1) problems.push(`${path}: trailing separator`);
      if (items[i + 1]?.kind === "separator") problems.push(`${path}: adjacent separators at ${i}`);
      return;
    }
    const key = item.label.replace(/[_&]/g, "").toLowerCase();
    labels.set(key, (labels.get(key) ?? 0) + 1);
    if (bannedItems.test(item.label)) problems.push(`${path}: "${item.label}" opens Chromium UI`);
    if (item.label.trim() === "") problems.push(`${path}: empty label at ${i}`);
    if (item.kind === "submenu") {
      if (item.children.length === 0) problems.push(`${path}: empty submenu "${item.label}"`);
      problems.push(...menuProblems(item.children, `${path} > ${item.label}`));
    }
  });
  for (const [label, n] of labels) if (n > 1) problems.push(`${path}: "${label}" ${n} times`);
  return problems;
}

function menuCount(): number {
  return readFileSync(hostLog, "utf8").split("\n").filter((l) => l.includes("menuShown depth=0 index=0 ")).length;
}

/// Opens the engine's context menu over `at`, captures it, checks it and
/// dismisses it.
async function checkMenu(name: string, at: { x: number; y: number }): Promise<{ note: string; problems: string[] }> {
  const before = menuCount();
  await clickAt(at, 3);
  const opened = await until(async () => menuCount(), (n) => n > before, 8000);
  if (opened === null) {
    const shot = `${shots}/ctx-${name}.png`;
    sh("import", "-window", "root", shot);
    const drawn = rootWindows().filter((w) => w.or && w.w < 800);
    key("Escape");
    return { note: `${shot} untraced, ${drawn.length} popup window(s)`, problems: name === "devtools" ? [] : ["no menu opened"] };
  }
  await Bun.sleep(700);
  const shot = `${shots}/ctx-${name}.png`;
  sh("import", "-window", "root", shot);
  const tree = menuTree();
  const menu = rootWindows().filter((w) => w.or && w.w < 800).sort((a, b) => b.w * b.h - a.w * a.h)[0];
  const problems = menuProblems(tree);
  // Truncation: GTK sizes a menu to its widest label, so a window narrower than
  // that label at the font's average width is one that cut it off.
  const longest = Math.max(0, ...tree.map((i) => i.label.length));
  if (menu && menu.w < longest * 5 + 30) problems.push(`menu ${menu.w}px wide for a ${longest}-character label`);
  if (!menu) problems.push("no menu window on screen");
  key("Escape");
  await Bun.sleep(500);
  const labels = tree.map((i) => (i.kind === "separator" ? "|" : i.label)).join(", ");
  return { note: `${shot} [${labels}]`, problems };
}

/// A native widget's centre in root pixels, calibrated on the page's container
/// (the one widget the X server can place exactly).
async function nativePoint(page: string, testId: string): Promise<{ x: number; y: number } | null> {
  const view = shownContainer();
  const pageBox = await app.getByTestId(page).boundingBox().catch(() => null);
  const box = await app.getByTestId(testId).boundingBox().catch(() => null);
  if (!view || !pageBox || !box) return null;
  return { x: view.x - pageBox.x + box.x + box.width / 2, y: view.y - pageBox.y + box.y + box.height / 2 };
}

async function webuiClick(page: string, text: string): Promise<string> {
  const code = `(() => {
    const want = ${JSON.stringify(text.toLowerCase())};
    let hit = null;
    const walk = (root) => {
      for (const el of root.querySelectorAll('*')) {
        if (hit) return;
        if (el.shadowRoot) walk(el.shadowRoot);
        if (hit) return;
        const own = [...el.childNodes].filter((n) => n.nodeType === 3).map((n) => n.textContent).join('').trim().toLowerCase();
        const label = (el.getAttribute('aria-label') || '').toLowerCase();
        if (own.includes(want) || label.includes(want)) {
          const r = el.getBoundingClientRect();
          if (r.width > 0 && r.height > 0) hit = { x: r.x + r.width / 2, y: r.y + r.height / 2 };
        }
      }
    };
    walk(document);
    return JSON.stringify(hit);
  })()`;
  const at = await until(async () => JSON.parse((await pageEval(page, code)) ?? "null"), (v) => v !== null, 10000);
  if (!at) return `no "${text}"`;
  const view = shownContainer();
  if (!view) return "no view";
  const dpr = Number(await pageEval(page, "devicePixelRatio")) || 1;
  await clickAt({ x: view.x + at.x * dpr, y: view.y + at.y * dpr });
  return `clicked "${text}"`;
}

async function inWorker(code: string): Promise<string> {
  for (const t of (await targets(cdpPort)).filter((t) => t.type === "service_worker" && t.webSocketDebuggerUrl)) {
    const s = await Session.open(t.webSocketDebuggerUrl!);
    try {
      if ((await s.eval<boolean>("self.ndEscape === true").catch(() => false)) !== true) continue;
      await s.eval(code).catch((e) => console.log(`  worker eval: ${String(e)}`));
      return "called";
    } finally {
      s.close();
    }
  }
  return "no escape extension worker";
}

// ============================================================================
// Routes
// ============================================================================

/// `viewport: false` for a route whose app action legitimately resizes the page
/// (a palette that stands the page aside, a docked inspector).
type Expect = { windows?: number; tabs?: number; viewport?: boolean };
type Outcome = string | void | { note: string; problems: string[] };
interface Route { name: string; expect: Expect; run: (page: string) => Promise<Outcome>; after?: () => void }

const NONE: Expect = { windows: 0, tabs: 0 };
const TAB: Expect = { windows: 0, tabs: 1 };
const WINDOW: Expect = { windows: 1 };

const chord = (name: string, keys: string, expect: Expect = NONE, after?: () => void): Route => ({
  name,
  expect,
  after,
  run: async (page) => {
    await focusPage(page);
    key(keys);
  },
});
const pageClick = (name: string, id: string, expect: Expect, button = 1, mods: string[] = []): Route => ({
  name,
  expect,
  run: async (page) => {
    focusApp();
    await clickAt(await pagePoint(page, id), button, mods);
  },
});
const menuItem = (
  name: string,
  where: string | [number, number],
  label: RegExp,
  expect: Expect = {},
  prepare?: (page: string) => Promise<void>,
): Route => ({
  name,
  expect,
  // The app's Inspect docks the inspector; F12 takes it away for the next route.
  after: /inspect/.test(String(label)) ? () => key("F12") : undefined,
  run: async (page) => {
    focusApp();
    if (prepare) await prepare(page);
    return menuRoute(page, where, label);
  },
});
const webui = (name: string, url: string, text: string | null, expect: Expect = {}, lands = url): Route => ({
  name,
  expect,
  run: async () => {
    const page = await load(url, lands);
    if (text) return webuiClick(page, text);
  },
});
const outside = (name: string, id: string): Route => ({
  name,
  expect: NONE,
  run: async (page) => {
    const count = () => (hostLog ? readFileSync(hostLog, "utf8").split("\n").filter((l) => l.includes("externalScheme url=") && l.includes("gesture=true")).length : 0);
    const before = count();
    focusApp();
    await clickAt(await pagePoint(page, id), 1, []);
    const handed = await until(async () => count(), (n) => n > before, 8000);
    const stayed = await pageEval(page, "location.href");
    if (!handed) throw new Error(`#${id} was never handed to the desktop`);
    if (stayed !== ESCAPE_URL) throw new Error(`the page left for ${stayed}`);
    return "handed to the desktop";
  },
});
const ext = (name: string, code: string, expect: Expect = {}): Route => ({ name, expect, run: async () => inWorker(code) });

const routes: Route[] = [
  // App-declared chords run the app's own action; the rest do nothing visible.
  chord("key.ctrlN", "ctrl+n", WINDOW),
  // The app's private window, through browserCommand newPrivateWindow.
  chord("key.ctrlShiftN", "ctrl+shift+n", WINDOW),
  chord("key.ctrlT", "ctrl+t", TAB),
  chord("key.ctrlShiftT", "ctrl+shift+t", {}),
  chord("key.ctrlShiftB", "ctrl+shift+b"),
  // The app's history palette, through browserCommand history; it stands the
  // page aside like ctrl+K.
  chord("key.ctrlH", "ctrl+h", { viewport: false }),
  chord("key.ctrlJ", "ctrl+j"),
  chord("key.ctrlShiftO", "ctrl+shift+o"),
  chord("key.ctrlShiftDelete", "ctrl+shift+Delete"),
  chord("key.ctrlShiftA", "ctrl+shift+a"),
  chord("key.ctrlShiftM", "ctrl+shift+m"),
  chord("key.ctrlD", "ctrl+d"),
  chord("key.ctrlS", "ctrl+s"),
  chord("key.ctrlP", "ctrl+p"),
  chord("key.ctrlShiftP", "ctrl+shift+p", {}),
  chord("key.ctrlO", "ctrl+o"),
  chord("key.ctrlU", "ctrl+u"),
  chord("key.ctrlShiftW", "ctrl+shift+w", {}),
  chord("key.altF", "alt+f"),
  chord("key.altE", "alt+e"),
  chord("key.F10", "F10"),
  chord("key.F1", "F1"),
  chord("key.F7", "F7"),
  chord("key.F11", "F11", {}),
  chord("key.shiftEsc", "shift+Escape"),
  chord("key.altShiftI", "alt+shift+i"),
  chord("key.ctrlE", "ctrl+e"),
  // The app's command palette, which stands the page aside while it is up.
  chord("key.ctrlK", "ctrl+k", { viewport: false }),
  // Chromium's own inspect chord docks the inspector in the view, which is
  // this engine's devtools; F12 is the toggle that takes it away again.
  chord("key.ctrlShiftC", "ctrl+shift+c", { windows: 0, tabs: 0, viewport: false }, () => key("F12")),
  chord("key.altShiftT", "alt+shift+t"),
  chord("key.ctrl1", "ctrl+1", {}),
  chord("key.altHome", "alt+Home", {}),
  // Last of the chords: on a build that lets it through, it ends the browser.
  chord("key.ctrlShiftQ", "ctrl+shift+q"),

  pageClick("page.windowOpenGesture", "open-gesture", TAB),
  pageClick("page.windowOpenPopup", "open-popup", TAB),
  // Denied by design: the app hears about:blank and the opener gets null
  // (onBeforePopup in src/cef/engine.zig).
  pageClick("page.windowOpenBlankThenNav", "open-blank-nav", { windows: 0 }),
  pageClick("page.windowOpenNoopener", "open-noopener", TAB),
  { name: "page.windowOpenNoGesture", expect: {}, run: async (page) => void (await pageEval(page, "void window.open('/page2.html')")) },
  pageClick("page.targetBlank", "blank", TAB),
  pageClick("page.middleClick", "plain", TAB, 2),
  pageClick("page.ctrlClick", "plain", TAB, 1, ["ctrl"]),
  pageClick("page.shiftClick", "plain", TAB, 1, ["shift"]),
  // Both go to the desktop's handler for the scheme (the engine traces the
  // hand-off), never an app tab, and the page stays where it was.
  outside("page.mailto", "mailto"),
  outside("page.externalProtocol", "extproto"),
  pageClick("page.print", "print", NONE),
  pageClick("page.documentPip", "pip-doc", NONE),
  pageClick("page.geolocation", "geo", NONE),
  pageClick("page.notification", "notif", NONE),
  pageClick("page.openChromeUrl", "open-chrome", {}),
  pageClick("page.passwordSave", "login-submit", NONE),

  menuItem("menu.openLinkNewTab", "plain", /open link in new tab/i, TAB),
  menuItem("menu.openLinkNewWindow", "plain", /open link in new window/i),
  menuItem("menu.openLinkIncognito", "plain", /incognito/i),
  menuItem("menu.openLinkOtherProfile", "plain", /open link as|in profile/i),
  menuItem("menu.viewSource", [0.5, 0.8], /view page source/i, NONE),
  menuItem("menu.saveAs", [0.5, 0.8], /save as/i),
  menuItem("menu.print", [0.5, 0.8], /^print/i, NONE),
  menuItem("menu.cast", [0.5, 0.8], /cast/i, NONE),
  menuItem("menu.qrCode", [0.5, 0.8], /qr code/i, NONE),
  menuItem("menu.translate", [0.5, 0.8], /translate/i, NONE),
  menuItem("menu.readingMode", [0.5, 0.8], /reading mode/i, NONE),
  menuItem("menu.inspect", [0.5, 0.8], /inspect/i, { windows: 0, tabs: 0, viewport: false }),

  {
    // The inspector's "dock to separate window": the frontend asks its host to
    // undock, which for Chromium's own DevToolsWindow is a window of its own.
    name: "devtools.undock",
    expect: { windows: 0, tabs: 0, viewport: false },
    after: () => key("F12"),
    run: async (page) => {
      await focusPage(page);
      key("ctrl+shift+c");
      const frontend = await until(
        async () => (await targets(cdpPort)).find((t) => t.url.startsWith("devtools://") && t.webSocketDebuggerUrl),
        (t) => t !== undefined,
        15000,
      );
      if (!frontend) return "no inspector opened";
      const session = await Session.open(frontend.webSocketDebuggerUrl!);
      try {
        await Bun.sleep(1500);
        await session.eval("InspectorFrontendHost.setIsDocked(false, () => {})").catch((e) => console.log(`  undock: ${String(e)}`));
      } finally {
        session.close();
      }
      return "asked the frontend to undock";
    },
  },
  webui("webui.settings", "chrome://settings", null, NONE),
  webui("webui.settingsPeople", "chrome://settings/people", "Customize profile"),
  webui("webui.profilePicker", "chrome://profile-picker", null),
  webui("webui.history", "chrome://history", null, NONE),
  webui("webui.historyClearData", "chrome://history", "browsing data"),
  webui("webui.downloads", "chrome://downloads", null, NONE),
  webui("webui.extensionsWebStore", "chrome://extensions", "web store"),
  webui("webui.extensionsDetails", "chrome://extensions", "Details"),
  webui("webui.bookmarks", "chrome://bookmarks", null, NONE),
  // The app keeps chrome://newtab for its own new-tab page; Chrome rewrites it
  // to chrome://new-tab-page/.
  webui("webui.newTabPage", "chrome://newtab", null, { windows: 0, viewport: false }, "chrome://new-tab-page"),

  ext("ext.tabsCreate", "chrome.tabs.create({ url: 'https://example.invalid/tabs-create' })", TAB),
  ext("ext.windowsCreate", "chrome.windows.create({ url: 'https://example.invalid/windows-create' })"),
  ext("ext.windowsCreatePopup", "chrome.windows.create({ url: 'https://example.invalid/popup', type: 'popup', width: 300, height: 300 })"),
  ext("ext.windowsCreateEmpty", "chrome.windows.create({})"),
  ext("ext.windowsCreateIncognito", "chrome.windows.create({ incognito: true, url: 'https://example.invalid/incog' })"),
  ext("ext.openOptionsPage", "chrome.runtime.openOptionsPage()"),
  ext("ext.tabsCreateChromeUrl", "chrome.tabs.create({ url: 'chrome://settings' })"),
];

const ctx = (name: string, where: string | [number, number], prepare?: (page: string) => Promise<void>): Route => ({
  name: `ctx.${name}`,
  expect: { windows: 0, tabs: 0 },
  run: async (page) => {
    focusApp();
    if (prepare) await prepare(page);
    return checkMenu(name, await pagePoint(page, where));
  },
});

const selectNode = (id: string) => async (page: string) => {
  await pageEval(page, `(() => { const r = document.createRange(); r.selectNodeContents(document.getElementById(${JSON.stringify(id)})); getSelection().removeAllRanges(); getSelection().addRange(r); })()`);
};

routes.push(
  ctx("page", [0.5, 0.8]),
  ctx("link", "plain"),
  ctx("image", "img"),
  ctx("imageLink", "img-link"),
  ctx("selection", "prose", selectNode("prose")),
  ctx("editableEmpty", "field-empty"),
  ctx("editableText", "field-text", async (page) => {
    await clickAt(await pagePoint(page, "field-text"));
    await pageEval(page, "document.getElementById('field-text').select()");
  }),
  ctx("misspelled", "misspelled", async (page) => {
    await clickAt(await pagePoint(page, "misspelled"));
    await pageEval(page, "(() => { const t = document.getElementById('misspelled'); t.setSelectionRange(0, 0); })()");
    await Bun.sleep(1500);
  }),
  ctx("video", "vid"),
  ctx("audio", "aud"),
  menuItem("menu.importPasswords", "field-empty", /import passwords/i, NONE),
  menuItem("menu.enhancedSpellcheck", "misspelled", /enhanced spell check/i, NONE, async (page) => {
    await clickAt(await pagePoint(page, "misspelled"));
    await pageEval(page, "document.getElementById('misspelled').setSelectionRange(0, 0)");
    await Bun.sleep(2500);
  }),
  menuItem("menu.searchGoogle", "field-text", /search google/i, TAB, async (page) => {
    await clickAt(await pagePoint(page, "field-text"));
    await pageEval(page, "document.getElementById('field-text').select()");
  }),
  {
    name: "ctx.devtools",
    expect: { windows: 0, tabs: 0, viewport: false },
    after: () => key("F12"),
    run: async (page) => {
      await focusPage(page);
      key("ctrl+shift+c");
      await Bun.sleep(3000);
      const view = shownContainer();
      if (!view) return { note: "", problems: ["no view"] };
      return checkMenu("devtools", { x: view.x + view.w - 120, y: view.y + view.h / 2 });
    },
  },
  ...["tab-list", "omnibox"].map((testId): Route => ({
    // The app's own widgets: GTK's menus, which the engine does not trace, so
    // what is asserted is that one opened and that nothing of Chromium's did.
    name: `ctx.native.${testId}`,
    expect: { windows: 0, tabs: 0, viewport: false },
    run: async (page) => {
      focusApp();
      const found = await nativePoint(page, testId);
      if (!found) return `${testId} not on screen`;
      // The first row of the tab list, not the empty space under the rows.
      const box = await app.getByTestId(testId).boundingBox();
      const at = testId === "tab-list" && box ? { x: found.x, y: found.y - box.height / 2 + 14 } : found;
      const before = new Set(rootWindows().map((w) => w.id));
      await clickAt(at, 3);
      await Bun.sleep(1200);
      const shot = `${shots}/ctx-native-${testId}.png`;
      sh("import", "-window", "root", shot);
      const menus = rootWindows().filter((w) => w.or && !before.has(w.id));
      key("Escape");
      await Bun.sleep(500);
      return `${shot} ${menus.length ? `menu ${menus[0]!.w}x${menus[0]!.h}` : "no menu"}`;
    },
  })),
);

// ============================================================================
// Runner
// ============================================================================

interface Snapshot { windows: number; tabs: number; root: Win[]; pages: number; viewport: string }

async function snapshot(): Promise<Snapshot> {
  const [own, tabs, list] = await Promise.all([
    app.windows(),
    tabCount().catch(() => -1),
    targets(cdpPort).catch(() => []),
  ]);
  const page = await activePage().catch(() => null);
  const viewport = page ? ((await pageEval(page, "innerWidth+'x'+innerHeight").catch(() => null)) ?? "?") : "none";
  return {
    windows: own.windows.length,
    tabs,
    root: rootWindows(),
    pages: list.filter((t) => t.type === "page" && !t.url.startsWith("devtools://")).length,
    viewport,
  };
}

async function reset(): Promise<string> {
  key("Escape");
  key("Escape");
  // Extra app windows (ctrl+N, a private window) and anything Chromium left
  // on screen close through the window manager; the app's first window stays.
  const keep = toplevelId();
  for (let attempt = 0; attempt < 4 && (await app.windows()).windows.length > 1; attempt++) {
    const extra = sh("xdotool", "search", "--onlyvisible", "--classname", "nd-hello").split("\n").filter(Boolean)
      .map((d) => `0x${Number(d).toString(16)}`).filter((id) => id !== keep);
    for (const id of extra) sh("wmctrl", "-i", "-c", id);
    await Bun.sleep(800);
  }
  focusApp();
  for (let attempt = 0; attempt < 8 && (await tabCount()) > 1; attempt++) {
    key("ctrl+w");
    await Bun.sleep(500);
  }
  let page = await activePage().catch(() => null);
  // A route can leave the tab with no page on show (a mailto: the app keeps
  // for itself), and the address field then belongs to nothing; a fresh tab
  // is the way back.
  if (!page) {
    key("ctrl+t");
    await Bun.sleep(800);
    page = await activePage().catch(() => null);
  }
  if (page && (await pageEval(page, "location.href").catch(() => null)) === ESCAPE_URL) {
    await pageEval(page, "location.reload()").catch(() => null);
    await until(async () => pageEval(page, "document.readyState"), (s) => s === "complete", 10000);
    return page;
  }
  return load(ESCAPE_URL);
}

const failures: string[] = [];
let ran = 0;
for (const route of routes) {
  if (only.length > 0 && !only.includes(route.name)) continue;
  lastProgress = Date.now();
  let page: string;
  try {
    page = await reset();
  } catch (error) {
    console.log(`ND_ESCAPE_ROUTE ${route.name}: SETUP ${String(error)}`);
    failures.push(`${route.name}: setup`);
    continue;
  }
  const before = await snapshot();
  let note = "";
  const routeProblems: string[] = [];
  try {
    const outcome = await route.run(page);
    if (typeof outcome === "object" && outcome !== null) {
      note = outcome.note;
      routeProblems.push(...outcome.problems);
    } else note = outcome ?? "";
  } catch (error) {
    note = `run error: ${error instanceof Error ? error.message : String(error)}`;
    routeProblems.push(note);
  }
  await Bun.sleep(2500);
  try {
    process.kill(hostPid, 0);
  } catch {
    console.log(`ND_ESCAPE_ROUTE ${route.name}: ESCAPE the host exited [${note}]`);
    failures.push(`${route.name}: the host exited`);
    break;
  }
  const after = await snapshot();
  const known = new Set(before.root.map((w) => w.id));
  const added = after.root.filter((w) => !known.has(w.id));
  const newWindows = after.windows - before.windows;
  const newTabs = after.tabs - before.tabs;
  // App windows the route was supposed to open are the big non-override ones;
  // everything past that count is Chromium's.
  const toplevels = added.filter((w) => !w.or && w.w >= 300 && w.h >= 200);
  const popovers = await appPopovers().catch(() => []);
  const ownPopover = (w: Win) => w.or && popovers.some((p) => Math.abs(p.w - w.w) <= 60 && Math.abs(p.h - w.h) <= 60);
  const surfaces = [
    ...toplevels.slice(Math.max(0, newWindows)),
    ...added.filter((w) => (w.or || w.w < 300 || w.h < 200) && !ownPopover(w)),
  ];
  if (added.some(ownPopover)) note += `${note ? "; " : ""}app popover on show`;
  const problems: string[] = [...routeProblems];
  if (surfaces.length > 0) {
    problems.push(`Chromium surface(s): ${surfaces.map((w) => `${w.or ? "OR " : ""}${w.w}x${w.h}+${w.x}+${w.y}${w.name ? ` "${w.name}"` : ""}`).join(" | ")}`);
  }
  if (route.expect.windows !== undefined && newWindows !== route.expect.windows) problems.push(`app windows ${newWindows >= 0 ? "+" : ""}${newWindows}, expected +${route.expect.windows}`);
  if (route.expect.tabs !== undefined && newTabs !== route.expect.tabs) problems.push(`app tabs ${newTabs >= 0 ? "+" : ""}${newTabs}, expected +${route.expect.tabs}`);
  if (route.expect.viewport !== false && newTabs === 0 && newWindows === 0 && before.viewport !== after.viewport && /^\d/.test(before.viewport) && /^\d/.test(after.viewport)) {
    problems.push(`page viewport ${before.viewport}->${after.viewport}`);
  }
  const detail = `windows ${newWindows >= 0 ? "+" : ""}${newWindows} tabs ${newTabs >= 0 ? "+" : ""}${newTabs} pages ${after.pages - before.pages >= 0 ? "+" : ""}${after.pages - before.pages} viewport ${before.viewport}->${after.viewport}${note ? ` [${note}]` : ""}`;
  route.after?.();
  ran++;
  if (problems.length === 0) console.log(`ND_ESCAPE_ROUTE ${route.name}: ok (${detail})`);
  else {
    console.log(`ND_ESCAPE_ROUTE ${route.name}: ESCAPE ${problems.join("; ")} (${detail})`);
    failures.push(`${route.name}: ${problems.join("; ")}`);
    // A Chromium toplevel left up would take focus from every later route.
    for (const w of surfaces) if (!w.or) sh("wmctrl", "-i", "-c", w.id);
  }
}

server.stop(true);
console.log(`ND_ESCAPE_ROUTES(${rig}) ${ran} ran, ${failures.length} escaped`);
if (failures.length > 0 && !explore) {
  for (const f of failures) console.error(`  - ${f}`);
  console.error(`ND_APP_CHROME_FAIL(${rig})`);
  process.exit(1);
}
// The rig reads this marker for its verdict.
console.log(`ND_APP_CHROME_LEGS_OK(${rig})`);
process.exit(0);
