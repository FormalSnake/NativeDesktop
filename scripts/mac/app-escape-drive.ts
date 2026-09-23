#!/usr/bin/env bun
// Drives the REAL browser app under ND_CEF_STYLE=chrome through every route a
// page, a key chord, a context menu, a chrome:// page or an extension has to
// Chromium's own browser UI, and asserts that none of them puts a Chromium
// surface on screen. Each route states what the app is supposed to see instead
// (a new app window, a new app tab, nothing), and that is asserted too.
//
// Observation per route, all from outside the app's React state: the window
// server's census diffed against the one taken before the route, the app's own
// window and tab counts, the page viewport (a Chromium toolbar or bookmark bar
// in the view takes rows from it) and the debugging port's page targets (a
// Chromium browser the census cannot see still has one).
//
// ND_ESCAPE_ROUTES=<comma separated> runs a subset; ND_ESCAPE_EXPLORE=1 prints
// every observation and never fails.
import { Cursor, connectApp, type LocatorFactory } from "@nativedesktop/test";

import { Session, targets } from "../cdp.ts";
import {
  FIXTURE_ORIGIN,
  HOST_PID,
  KEY_DOWN_ARROW,
  KEY_ESCAPE,
  KEY_RETURN,
  LegFailure,
  activateApp,
  assert,
  census,
  hostAlive,
  menuStopsTo,
  menuWindows,
  pageEval,
  shownMenu,
  startFixtureServer,
  systemKey,
  until,
  type CensusWindow,
  type ShownMenuItem,
} from "./app-chrome-lib.ts";

const DEBUG_PORT = Number(process.env.ND_CEF_DEBUG_PORT ?? "9436");
const explore = process.env.ND_ESCAPE_EXPLORE === "1";
const only = (process.env.ND_ESCAPE_ROUTES ?? "").split(",").filter((n) => n.length > 0);
const skip = new Set((process.env.ND_ESCAPE_SKIP ?? "").split(",").filter((n) => n.length > 0));
/// Route groups by name prefix (key, page, menu, webui, ext): one host holds the
/// machine-wide lock for as long as it runs, so the gate is run a group at a time.
const groups = (process.env.ND_ESCAPE_GROUPS ?? "").split(",").filter((n) => n.length > 0);
/// Seconds this drive may run before it stops and names what it did not reach.
const deadline = Date.now() + Number(process.env.ND_ESCAPE_BUDGET_S ?? "480") * 1000;

const app = await connectApp();
const fixtures = startFixtureServer();
await until("the app's first window", async () => (await app.windows()).windows.length, (n) => n > 0, 45000);
const mainWindow = (await app.windows()).windows[0]!.ref;
const main: LocatorFactory = await app.window(0);

/// HID-level pointer events through the host binary's `--nd-input` helper, so
/// Chromium's own hit testing, context menu and modifier-click routing see a
/// real mouse. Points are relative to the app's first window.
const cursor = new Cursor({
  binary: async () => process.env.ND_INPUT_BINARY ?? "swift/.build/release/NDShell",
  pid: HOST_PID,
  windows: () => app.windows(),
  window: mainWindow,
});

// MARK: - app vocabulary (the same reads scripts/mac/app-chrome-drive.ts uses)

type Node = { type: string; testID: string | null; visible: boolean; rows: unknown[] | null; children: Node[] };

async function activePage(): Promise<string> {
  const tree = await app.tree(mainWindow);
  let found: string | null = null;
  const walk = (node: Node) => {
    if (node.type === "WebView" && node.visible && node.testID?.startsWith("page-")) found ??= node.testID;
    for (const child of node.children) walk(child);
  };
  walk(tree.root as never);
  assert(found !== null, "no visible WebView in the tree");
  return found!;
}

/// The app's tab count: the sidebar's row list, or in the compact layout the
/// tab strip's items.
async function tabCount(): Promise<number> {
  const tree = await app.tree(mainWindow);
  let rows = -1;
  let items = 0;
  const walk = (node: Node) => {
    if (node.testID === "tab-list") rows = node.rows?.length ?? 0;
    if (node.testID?.startsWith("tab-item-")) items++;
    for (const child of node.children) walk(child);
  };
  walk(tree.root as never);
  return rows >= 0 ? rows : items;
}
/// Real keys into the new-tab page's search field. The header's own address
/// field is not always on show (the app hides it in some layouts), so a page
/// that already has a tab is navigated from inside instead, and a chrome://
/// address, which a web page may not navigate to, gets a new tab first.
async function openAddress(url: string): Promise<void> {
  const page = await activePage().catch(() => null);
  if (page && !url.startsWith("chrome://") && (await pageEval(app, page, "location.protocol").catch(() => null))?.startsWith("http")) {
    await pageEval(app, page, `location.href = ${JSON.stringify(url)}`);
    return;
  }
  if (!(await main.getByTestId("new-tab-search").isVisible().catch(() => false))) {
    chord("t", ["command"]);
    await until("the new-tab page", () => main.getByTestId("new-tab-search").isVisible(), (v) => v === true, 8000);
  }
  await cursor.click(main.getByTestId("new-tab-search"));
  await Bun.sleep(400);
  osa(`tell application "System Events" to keystroke ${JSON.stringify(url)}`);
  await Bun.sleep(200);
  chord("36", []);
}

async function load(url: string, ready = "complete"): Promise<string> {
  for (let attempt = 0; attempt < 3; attempt++) {
    await openAddress(url);
    const id = await until("a webview", activePage, () => true, 15000).catch(() => null);
    if (id === null) continue;
    const landed = await until(
      `${url} loads`,
      async () => `${await pageEval(app, id, "location.href")}|${await pageEval(app, id, "document.readyState")}`,
      (v) => v.startsWith(url) && v.endsWith(`|${ready}`),
      15000,
    ).catch(() => null);
    if (landed !== null) return id;
  }
  throw new LegFailure(`the address bar never took ${url}`);
}

const ESCAPE_URL = `${FIXTURE_ORIGIN}/escape.html`;

// MARK: - real input

function osa(script: string): void {
  const run = Bun.spawnSync(["osascript", "-e", script]);
  if (run.exitCode !== 0) throw new LegFailure(`osascript failed: ${run.stderr.toString().trim()}`);
}

/// A chord through the window server, with the app frontmost: the path a user's
/// keyboard takes, which an NSEvent posted into the app's own queue is not.
function chord(key: string, mods: string[]): void {
  activateApp();
  const using = mods.length ? ` using {${mods.map((m) => `${m} down`).join(", ")}}` : "";
  const press = /^\d+$/.test(key) ? `key code ${key}` : `keystroke "${key}"`;
  osa(`tell application "System Events" to ${press}${using}`);
}

type ClickMode = "left" | "right" | "middle" | "cmd" | "shift";

async function click(at: { x: number; y: number }, mode: ClickMode): Promise<void> {
  activateApp();
  const button = mode === "right" ? "right" : mode === "middle" ? "middle" : "left";
  const modifiers = mode === "cmd" ? ["command" as const] : mode === "shift" ? ["shift" as const] : undefined;
  await cursor.click(at, { button, modifiers });
}

/// A point in the page, relative to the app's first window. `where` is either
/// an element id in the page or a fraction of the view.
async function pagePoint(page: string, where: string | [number, number]): Promise<{ x: number; y: number }> {
  const box = await main.getByTestId(page).boundingBox();
  assert(box !== null, `${page} has no box`);
  if (Array.isArray(where)) {
    return { x: box!.x + box!.width * where[0], y: box!.y + box!.height * where[1] };
  }
  const rect = JSON.parse(
    (await pageEval(app, page, `JSON.stringify(document.getElementById(${JSON.stringify(where)}).getBoundingClientRect())`)) ??
      "null",
  ) as { x: number; y: number; width: number; height: number } | null;
  assert(rect !== null, `the page has no #${where}`);
  return { x: box!.x + rect!.x + rect!.width / 2, y: box!.y + rect!.y + rect!.height / 2 };
}

/// Gives the page the keyboard the way a user does: a click on a quiet part of it.
async function focusPage(page: string): Promise<void> {
  await click(await pagePoint(page, "quiet"), "left");
  await Bun.sleep(300);
}

const SHOTS = process.env.ND_ESCAPE_SHOTS ?? "/tmp/nd-escape-shots";
await Bun.$`mkdir -p ${SHOTS}`.quiet();
const NDSHOT = process.env.ND_NDSHOT ?? "tools/ndshot/bin/ndshot";

/// The screen under the app's window with whatever sits on top of it (menus,
/// sheets, panels, Chromium surfaces), without taking focus: a focus change
/// closes an open menu.
function regionShot(name: string, region = false): string {
  const out = `${SHOTS}/${name}.png`;
  // By window number: a pid match can pick the menu bar or an anchor.
  const listed = Bun.spawnSync(["swift", "scripts/mac/window-census.swift", String(HOST_PID)], {
    env: { ...process.env, SDKROOT: undefined, DEVELOPER_DIR: undefined },
  }).stdout.toString().split("\n").filter((l) => l.startsWith("{")).map((l) => JSON.parse(l) as CensusWindow);
  const main = listed.filter((w) => w.alpha > 0 && w.layer === 0).sort((a, b) => b.width * b.height - a.width * a.height)[0];
  const target = main ? ["--window-id", String(main.number)] : ["--pid", String(HOST_PID)];
  // --region only for surfaces that are windows of their own (a native menu):
  // it can hang, so every capture is bounded.
  const run = Bun.spawnSync([NDSHOT, "capture", ...target, ...(region ? ["--region"] : []), "--no-focus", "--out", out], { timeout: 30000 });
  return run.exitCode === 0 ? out : `(no capture: ndshot exit ${run.exitCode})`;
}

/// What a context menu must never show: the same label twice at one level,
/// two separators in a row, a separator at either end, an empty submenu, or an
/// item that runs a command this engine refuses.
function lintMenu(items: ShownMenuItem[]): string[] {
  const problems: string[] = [];
  const levels = new Map<string, ShownMenuItem[]>();
  let path: number[] = [];
  for (const item of items) {
    path = path.slice(0, item.depth);
    const key = path.join("/");
    if (!levels.has(key)) levels.set(key, []);
    levels.get(key)!.push(item);
    path[item.depth] = item.index;
  }
  for (const [key, level] of levels) {
    const where = key === "" ? "top level" : `submenu ${key}`;
    const labels = level.filter((i) => i.kind !== "separator").map((i) => i.label);
    const dupes = labels.filter((l, i) => labels.indexOf(l) !== i);
    if (dupes.length) problems.push(`${where}: duplicate ${[...new Set(dupes)].map((d) => JSON.stringify(d)).join(", ")}`);
    if (labels.length === 0) problems.push(`${where}: empty`);
    if (level[0]?.kind === "separator") problems.push(`${where}: leading separator`);
    if (level.at(-1)?.kind === "separator") problems.push(`${where}: trailing separator`);
    for (let i = 1; i < level.length; i++) {
      if (level[i]!.kind === "separator" && level[i - 1]!.kind === "separator") problems.push(`${where}: adjacent separators at ${i}`);
    }
    for (const item of level) {
      if (item.kind !== "separator" && item.label.trim() === "") problems.push(`${where}: unlabelled item ${item.id}`);
      if (/incognito|chrome web store|open link as|view (page|frame) source|^cast|send to your devices|qr code/i.test(item.label)) {
        problems.push(`${where}: offers ${JSON.stringify(item.label)}, which opens Chromium UI`);
      }
    }
  }
  return problems;
}

type MenuSeen = { items: ShownMenuItem[]; shot: string; lint: string[] };
let lastMenu: MenuSeen | null = null;
/// The menu the route opened, once: a route that opens none answers null.
function takeMenu(): MenuSeen | null {
  const menu = lastMenu;
  lastMenu = null;
  return menu;
}

async function openPageMenu(page: string, where: string | [number, number], name = "menu"): Promise<ShownMenuItem[]> {
  await click(await pagePoint(page, where), "right");
  await until("the context menu opens", menuWindows, (w) => w.length > 0, 15000);
  await Bun.sleep(300);
  const items = shownMenu();
  assert(items.length > 0, "the host drew a menu but reported no items");
  lastMenu = { items, shot: regionShot(name, true), lint: lintMenu(items) };
  return items;
}

async function closeMenu(): Promise<void> {
  systemKey(KEY_ESCAPE);
  await until("the menu closes", menuWindows, (w) => w.length === 0, 10000).catch(() => null);
}

/// Picks `label` if the menu has it, and answers what it picked. A menu that
/// does not offer the item at all is a route that is closed, not a failure.
async function pickIfOffered(items: ShownMenuItem[], label: RegExp | null): Promise<string | null> {
  const top = items.filter((i) => i.depth === 0);
  const hit = label ? top.find((i) => label.test(i.label)) : undefined;
  if (!hit) {
    await closeMenu();
    return null;
  }
  if (!hit.enabled) {
    await closeMenu();
    return `${hit.label} (disabled)`;
  }
  systemKey(KEY_DOWN_ARROW, menuStopsTo(items, hit.label));
  systemKey(KEY_RETURN);
  await until("the menu closes", menuWindows, (w) => w.length === 0, 10000).catch(() => null);
  return hit.label;
}

/// Finds a node in a WebUI page through every shadow root, by its text, and
/// answers its box in the page's coordinates.
async function webuiBox(page: string, text: string): Promise<{ x: number; y: number; width: number; height: number } | null> {
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
        if (own === want || label === want) {
          const r = el.getBoundingClientRect();
          if (r.width > 0 && r.height > 0) hit = { x: r.x, y: r.y, width: r.width, height: r.height };
        }
      }
    };
    walk(document);
    return JSON.stringify(hit);
  })()`;
  return JSON.parse((await pageEval(app, page, code)) ?? "null");
}

async function clickWebui(page: string, text: string): Promise<boolean> {
  const rect = await until(`"${text}" in ${page}`, () => webuiBox(page, text), (r) => r !== null, 10000).catch(() => null);
  if (!rect) return false;
  const box = await main.getByTestId(page).boundingBox();
  await click({ x: box!.x + rect.x + rect.width / 2, y: box!.y + rect.y + rect.height / 2 }, "left");
  return true;
}

/// The escape probe extension's worker, found by the marker its script sets.
async function extensionWorker(): Promise<Session> {
  const all = await targets(DEBUG_PORT);
  for (const t of all.filter((t) => t.type === "service_worker" && t.webSocketDebuggerUrl)) {
    const session = await Session.open(t.webSocketDebuggerUrl!);
    if ((await session.eval<boolean>("self.ndEscape === true").catch(() => false)) === true) return session;
    session.close();
  }
  throw new LegFailure(`the escape probe extension's worker is not running (${all.map((t) => `${t.type} ${t.url}`).join(", ")})`);
}

async function inWorker(code: string): Promise<string> {
  const session = await extensionWorker();
  try {
    return String(await session.eval(`(async () => { try { await ${code}; return "called"; } catch (e) { return "threw " + e; } })()`));
  } finally {
    session.close();
  }
}

// MARK: - observation

interface Snapshot {
  windows: number;
  tabs: number;
  census: CensusWindow[];
  pages: number;
  viewport: string;
}

async function snapshot(): Promise<Snapshot> {
  const [own, tabs, windows, list] = await Promise.all([
    app.windows(),
    tabCount().catch(() => -1),
    census(),
    targets(DEBUG_PORT).catch(() => []),
  ]);
  const page = await activePage().catch(() => null);
  const viewport = page ? ((await pageEval(app, page, "innerWidth+'x'+innerHeight").catch(() => null)) ?? "?") : "none";
  return {
    windows: own.windows.length,
    tabs,
    census: windows,
    pages: list.filter((t) => t.type === "page" && !t.url.startsWith("devtools://")).length,
    viewport,
  };
}

interface Seen {
  newWindows: number;
  newTabs: number;
  surfaces: CensusWindow[];
  appPopover: boolean;
  pages: number;
  viewport: string;
}

/// The app's own popovers on show, as the size of their content. On AppKit the
/// Popover node is the anchor handle, not the popover window, so its content is
/// what measures it (in the popover window's own space).
async function appPopovers(): Promise<{ w: number; h: number }[]> {
  type Sized = Node & { geometry?: { w: number; h: number } | null };
  const found: { w: number; h: number }[] = [];
  const largest = (node: Sized): { w: number; h: number } | null => {
    let best = node.visible && node.geometry ? node.geometry : null;
    for (const child of node.children) {
      const g = largest(child as never);
      if (g && (!best || g.w * g.h > best.w * best.h)) best = g;
    }
    return best;
  };
  for (const w of (await app.windows()).windows) {
    const tree = await app.tree(w.ref);
    const walk = (node: Sized) => {
      if (node.type === "Popover") {
        const g = largest(node);
        if (g) found.push(g);
        return;
      }
      for (const child of node.children) walk(child as never);
    };
    walk(tree.root as never);
  }
  return found;
}

/// What a route left behind. A census window that was not there before and is
/// not one of the app's own is a surface someone else drew, whatever its layer:
/// a Chromium toplevel, a bubble, a dialog, the app menu, or an OS panel.
async function diff(before: Snapshot): Promise<Seen> {
  await Bun.sleep(2500);
  const after = await snapshot();
  const own = (await app.windows()).windows.flatMap((w) => (w.geometry ? [w.geometry] : []));
  const known = new Set(before.census.map((w) => w.number));
  const added = after.census.filter(
    (w) =>
      w.alpha > 0 &&
      !known.has(w.number) &&
      !own.some((f) => Math.abs(f.x - w.x) <= 1 && Math.abs(f.y - w.y) <= 1 && Math.abs(f.w - w.width) <= 1 && Math.abs(f.h - w.height) <= 1),
  );
  // An NSPopover is a window of its own, a little larger than its content
  // (the arrow and the frame), and the app's popovers are not in app.windows().
  const popovers = added.length ? await appPopovers().catch(() => []) : [];
  const ownPopover = (w: CensusWindow) => popovers.some((p) => Math.abs(p.w - w.width) <= 60 && Math.abs(p.h - w.height) <= 60);
  return {
    newWindows: after.windows - before.windows,
    newTabs: after.tabs - before.tabs,
    surfaces: added.filter((w) => !ownPopover(w)),
    appPopover: added.some(ownPopover),
    pages: after.pages - before.pages,
    viewport: `${before.viewport}->${after.viewport}`,
  };
}

// MARK: - routes

/// `native` counts surfaces that are the operating system's own UI rather than
/// Chromium's (the print panel, the share sheet, a save panel, AppKit's full
/// screen toolbar window) and are allowed. `known` names where an escape this
/// route shows is owned, so it is reported but does not fail the gate.
type Expect = { windows?: number; tabs?: number; native?: number; known?: string };
interface Route {
  name: string;
  expect: Expect;
  /// Runs the route and answers a note for the report.
  run: (page: string) => Promise<string | void>;
}

const key = (name: string, k: string, mods: string[], expect: Expect = {}): Route => ({
  name,
  expect,
  run: async (page) => {
    await focusPage(page);
    chord(k, mods);
  },
});

const pageClick = (name: string, id: string, mode: ClickMode, expect: Expect): Route => ({
  name,
  expect,
  run: async (page) => {
    await click(await pagePoint(page, id), mode);
  },
});

/// Opens the context menu on `where` (an element id, `select:<id>` to select
/// the element's text first, or a fraction of the view), lints it, captures it,
/// and picks `label` if given and offered.
const menuItem = (name: string, where: string | [number, number], label: RegExp | null, expect: Expect): Route => ({
  name,
  expect,
  run: async (page) => {
    let target = where;
    if (typeof where === "string" && where.startsWith("select:")) {
      target = where.slice(7);
      await pageEval(app, page, `(() => { const el = document.getElementById(${JSON.stringify(target)}); if (el.select) { el.focus(); el.select(); return; } const r = document.createRange(); r.selectNodeContents(el); getSelection().removeAllRanges(); getSelection().addRange(r); })()`);
    }
    const items = await openPageMenu(page, target, name);
    const picked = await pickIfOffered(items, label);
    if (label === null) return "";
    return picked === null ? "not offered" : `picked ${picked}`;
  },
});

const webui = (name: string, url: string, text: string | null): Route => ({
  name,
  // The app opens a chrome:// address in a tab of its own; that tab is this
  // route's setup, so only windows and surfaces are asserted.
  expect: { windows: 0 },
  run: async () => {
    const page = await load(url);
    if (text === null) return;
    const ok = await clickWebui(page, text);
    return ok ? `clicked "${text}"` : `no "${text}" in ${url}`;
  },
});

const ext = (name: string, call: string, expect: Expect): Route => ({
  name,
  expect,
  run: async () => `worker ${await inWorker(call)}`,
});

/// A link to a scheme no browser draws. Mail.app is quit again afterwards
/// unless it was already running, since the hand-off launches it.
const outside = (name: string, id: string): Route => ({
  name,
  expect: { windows: 0, tabs: 0 },
  run: async (page) => {
    const mailWasRunning = Bun.spawnSync(["pgrep", "-x", "Mail"]).exitCode === 0;
    const href = (await pageEval(app, page, `document.getElementById(${JSON.stringify(id)}).href`)) ?? "";
    const before = countTrace("openOutside");
    await click(await pagePoint(page, id), "left");
    const handed = await until("the host hands the link to the system", async () => countTrace("openOutside"), (n) => n > before, 8000).catch(() => before);
    const stayed = await pageEval(app, page, "location.href");
    if (!mailWasRunning) Bun.spawnSync(["osascript", "-e", 'if application "Mail" is running then tell application "Mail" to quit']);
    assert(handed > before, `${href} was never handed to the system`);
    assert(stayed === ESCAPE_URL, `the page left for ${stayed}`);
    return `handed ${href} to the system`;
  },
});

function countTrace(marker: string): number {
  const path = process.env.ND_APP_HOST_LOG;
  if (!path) return 0;
  return Number(Bun.spawnSync(["rg", "-c", marker, path]).stdout.toString().trim()) || 0;
}

const NONE: Expect = { windows: 0, tabs: 0 };
const TAB: Expect = { windows: 0, tabs: 1 };
const WINDOW: Expect = { windows: 1, tabs: 0 };
const NATIVE: Expect = { windows: 0, tabs: 0, native: 1 };
const BUBBLE = (what: string): Expect => ({ windows: 0, tabs: 0, known: `${what}: bubbles branch` });

const routes: Route[] = [
  // Key chords with the page holding the keyboard. The app's own accelerators
  // (cmd+N, cmd+T, cmd+shift+T, cmd+comma) have to run the app's action; every
  // other Chrome accelerator has to do nothing visible, or reach the app as
  // `browserCommand`.
  key("key.cmdN", "n", ["command"], WINDOW),
  // The app's private window, through browserCommand newPrivateWindow.
  key("key.cmdShiftN", "n", ["command", "shift"], WINDOW),
  key("key.cmdT", "t", ["command"], TAB),
  key("key.cmdShiftT", "t", ["command", "shift"], { windows: 0 }),
  key("key.cmdShiftB", "b", ["command", "shift"], NONE),
  key("key.cmdY", "y", ["command"], NONE),
  key("key.cmdShiftJ", "j", ["command", "shift"], NONE),
  key("key.cmdOptL", "l", ["command", "option"], NONE),
  key("key.cmdOptB", "b", ["command", "option"], NONE),
  key("key.cmdShiftO", "o", ["command", "shift"], NONE),
  // The app's Settings is a window of its own.
  key("key.cmdComma", ",", ["command"], WINDOW),
  key("key.cmdShiftDelete", "51", ["command", "shift"], NONE),
  key("key.cmdShiftA", "a", ["command", "shift"], NONE),
  key("key.cmdShiftM", "m", ["command", "shift"], NONE),
  key("key.cmdD", "d", ["command"], NONE),
  key("key.cmdShiftD", "d", ["command", "shift"], NONE),
  key("key.cmdS", "s", ["command"], NONE),
  // The system print panel, as window.print() gives; cmd+opt+P is Chrome's
  // "print using the system dialog", the same panel.
  key("key.cmdP", "p", ["command"], NATIVE),
  key("key.cmdOptP", "p", ["command", "option"], NATIVE),
  key("key.cmdO", "o", ["command"], NONE),
  key("key.cmdOptU", "u", ["command", "option"], NONE),
  key("key.cmdOptI", "i", ["command", "option"], { windows: 0, tabs: 0 }),
  key("key.cmdOptJ", "j", ["command", "option"], { windows: 0, tabs: 0 }),
  key("key.cmdOptC", "c", ["command", "option"], { windows: 0, tabs: 0 }),
  key("key.cmdE", "e", ["command"], NONE),
  key("key.cmdShiftW", "w", ["command", "shift"], NONE),
  key("key.cmdOptShiftI", "i", ["command", "option", "shift"], NONE),
  key("key.F7", "98", [], NONE),
  key("key.F1", "122", [], NONE),
  key("key.ctrlF2", "120", ["control"], NONE),
  key("key.cmdShiftL", "l", ["command", "shift"], NONE),
  key("key.cmdOptN", "n", ["command", "option"], NONE),
  key("key.cmdShiftK", "k", ["command", "shift"], NONE),
  key("key.cmdShiftH", "h", ["command", "shift"], NONE),
  key("key.cmd1", "1", ["command"], NONE),
  key("key.cmd9", "9", ["command"], NONE),
  key("key.shiftEsc", "53", ["shift"], NONE),
  key("key.cmdOptRight", "124", ["command", "option"], NONE),
  {
    // AppKit's own Enter Full Screen. In full screen AppKit moves the title bar
    // and the header into a toolbar window across the top of the screen, which
    // is the app's own; anything else is not.
    name: "key.ctrlCmdF",
    expect: NATIVE,
    run: async (page) => {
      await focusPage(page);
      chord("f", ["command", "control"]);
      await Bun.sleep(2500);
      const shot = regionShot("key.ctrlCmdF-fullscreen");
      chord("f", ["command", "control"]);
      await Bun.sleep(2000);
      return `full screen capture ${shot}`;
    },
  },
  key("key.cmdShiftF", "f", ["command", "shift"], { windows: 0 }),
  key("key.cmdShiftS", "s", ["command", "shift"], { windows: 0, known: "save dialog: crash agent" }),

  // Pages asking for a window.
  pageClick("page.windowOpenGesture", "open-gesture", "left", TAB),
  pageClick("page.windowOpenPopup", "open-popup", "left", TAB),
  // The popup is refused before it has a destination (onBeforePopup in
  // src/cef/engine.zig), so the app hears about:blank and opens nothing.
  pageClick("page.windowOpenBlankThenNav", "open-blank-nav", "left", { windows: 0, known: "about:blank popups are refused by design" }),
  pageClick("page.windowOpenNoopener", "open-noopener", "left", TAB),
  {
    name: "page.windowOpenNoGesture",
    expect: { windows: 0 },
    run: async (page) => {
      await pageEval(app, page, "void window.open('/page2.html')");
    },
  },
  pageClick("page.targetBlank", "blank", "left", TAB),
  pageClick("page.middleClick", "plain", "middle", TAB),
  pageClick("page.cmdClick", "plain", "cmd", TAB),
  pageClick("page.shiftClick", "plain", "shift", TAB),
  // Both go to the scheme's own application (the host traces the hand-off),
  // never an app tab, and the page stays where it was.
  outside("page.mailto", "mailto"),
  outside("page.externalProtocol", "extproto"),
  pageClick("page.print", "print", "left", NATIVE),
  pageClick("page.documentPip", "pip-doc", "left", NONE),
  pageClick("page.share", "share", "left", NATIVE),
  pageClick("page.geolocation", "geo", "left", BUBBLE("permission prompt")),
  pageClick("page.notification", "notif", "left", BUBBLE("permission prompt")),
  pageClick("page.openChromeUrl", "open-chrome", "left", { windows: 0 }),
  pageClick("page.passwordSave", "login-submit", "left", { windows: 0 }),

  // Chromium's context menu items that open a window, a tab or a bubble, and
  // every other context opened, captured and linted.
  menuItem("menu.openLinkNewTab", "plain", /open link in new tab/i, TAB),
  menuItem("menu.openLinkNewWindow", "plain", /open link in new window/i, { windows: 0 }),
  menuItem("menu.openLinkIncognito", "plain", /incognito/i, NONE),
  menuItem("menu.openLinkOtherProfile", "plain", /open link as|in profile/i, NONE),
  menuItem("menu.searchFor", "select:field-text", /search .* for/i, { windows: 0, tabs: 1 }),
  menuItem("menu.page", [0.5, 0.8], null, NONE),
  menuItem("menu.saveAs", [0.5, 0.8], /save as/i, NATIVE),
  menuItem("menu.image", "img", null, NONE),
  menuItem("menu.linkImage", "img-link", null, NONE),
  menuItem("menu.selection", "select:prose", null, NONE),
  menuItem("menu.editableEmpty", "field-empty", null, NONE),
  menuItem("menu.editableText", "select:field-text", null, NONE),
  menuItem("menu.misspelled", "misspelled", null, NONE),
  menuItem("menu.video", "vid", null, NONE),
  menuItem("menu.audio", "aud", null, NONE),

  // chrome:// pages in an app tab, and the controls in them that open more.
  webui("webui.settings", "chrome://settings", null),
  webui("webui.settingsManageProfile", "chrome://settings/people", "Customize profile"),
  webui("webui.settingsAddProfile", "chrome://settings/manageProfile", null),
  webui("webui.profilePicker", "chrome://profile-picker", null),
  webui("webui.history", "chrome://history", null),
  webui("webui.historyClearData", "chrome://history", "Delete browsing data"),
  webui("webui.downloads", "chrome://downloads", null),
  webui("webui.extensionsWebStore", "chrome://extensions", "Chrome Web Store"),
  webui("webui.extensionsDetails", "chrome://extensions", "Details"),
  webui("webui.bookmarks", "chrome://bookmarks", null),
  webui("webui.signin", "chrome://signin-dice-web-intercept.top-chrome", null),

  // Extensions asking Chromium for a window or a tab.
  ext("ext.tabsCreate", "chrome.tabs.create({ url: 'https://example.invalid/tabs-create' })", TAB),
  ext("ext.windowsCreate", "chrome.windows.create({ url: 'https://example.invalid/windows-create' })", { windows: 0 }),
  ext("ext.windowsCreatePopup", "chrome.windows.create({ url: 'https://example.invalid/popup', type: 'popup', width: 300, height: 300 })", { windows: 0 }),
  ext("ext.windowsCreateEmpty", "chrome.windows.create({})", { windows: 0 }),
  ext("ext.windowsCreateIncognito", "chrome.windows.create({ incognito: true, url: 'https://example.invalid/incog' })", { windows: 0 }),
  ext("ext.openOptionsPage", "chrome.runtime.openOptionsPage()", { windows: 0 }),
  ext("ext.tabsCreateChromeUrl", "chrome.tabs.create({ url: 'chrome://settings' })", { windows: 0 }),
];

// MARK: - runner

async function reset(): Promise<string> {
  for (let i = 0; i < 2; i++) systemKey(KEY_ESCAPE);
  // A route that zoomed the page (a stray cmd+plus, Chromium's own zoom
  // bubble) would show up as a viewport change in every route after it.
  chord("0", ["command"]);
  // Extra app windows (cmd+N, a private window, Settings) close through their
  // own close button; the front one is the newest.
  for (let attempt = 0; attempt < 4 && (await app.windows()).windows.length > 1; attempt++) {
    try {
      osa(`tell application "System Events" to tell (first process whose unix id is ${HOST_PID}) to perform action "AXPress" of (first button whose subrole is "AXCloseButton") of window 1`);
    } catch {
      break;
    }
    await Bun.sleep(800);
  }
  // Extra tabs close with the app's own cmd+W until one is left.
  for (let attempt = 0; attempt < 6 && (await tabCount()) > 1; attempt++) {
    chord("w", ["command"]);
    await Bun.sleep(600);
  }
  const page = await activePage().catch(() => null);
  if (page && (await pageEval(app, page, "location.href").catch(() => null)) === ESCAPE_URL) {
    await pageEval(app, page, "location.reload()").catch(() => null);
    await until("the fixture reloads", async () => pageEval(app, page, "document.readyState"), (s) => s === "complete", 10000).catch(() => null);
    return page;
  }
  return load(ESCAPE_URL);
}

const failures: string[] = [];
let ran = 0;
for (const route of routes) {
  if (only.length > 0 && !only.includes(route.name)) continue;
  if (skip.has(route.name)) continue;
  if (groups.length > 0 && !groups.includes(route.name.split(".")[0]!)) continue;
  if (Date.now() > deadline) {
    console.log(`ND_ESCAPE_UNRUN ${route.name}`);
    continue;
  }
  let page: string;
  try {
    page = await reset();
  } catch (error) {
    console.log(`ND_ESCAPE_ROUTE ${route.name}: SETUP ${String(error)}`);
    failures.push(`${route.name}: setup ${String(error)}`);
    continue;
  }
  const before = await snapshot();
  let note = "";
  try {
    note = (await route.run(page)) ?? "";
  } catch (error) {
    note = `run error: ${error instanceof Error ? error.message : String(error)}`;
  }
  if (!hostAlive()) {
    console.log(`ND_ESCAPE_ROUTE ${route.name}: HOST DIED ${note}`);
    failures.push(`${route.name}: the host died`);
    break;
  }
  if (route.expect.native) {
    // An OS panel (print, share, save) runs a modal loop on the host's main
    // thread, and the automation socket cannot answer until it is dismissed.
    // It is seen through the window server, captured, and dismissed first.
    await Bun.sleep(2500);
    const known = new Set(before.census.map((w) => w.number));
    const panels = (await census()).filter((w) => w.alpha > 0 && !known.has(w.number));
    note += ` panel ${panels.map((w) => `L${w.layer} ${w.width}x${w.height}`).join(" | ") || "none"} capture ${regionShot(route.name, true)}`;
    for (let i = 0; i < 3 && (await census()).some((w) => w.alpha > 0 && !known.has(w.number)); i++) {
      systemKey(KEY_ESCAPE);
      await Bun.sleep(800);
    }
  }
  const seen = await diff(before);
  if (seen.appPopover) note += " app popover on show";
  // Chromium's link status bubble (the URL shown while the pointer is over a
  // link) is a 22 pt strip inside the view: a hover hint, reported, allowed.
  const frames = (await app.windows().catch(() => ({ windows: [] as { geometry?: { x: number; y: number; w: number; h: number } }[] }))).windows.flatMap((w) => (w.geometry ? [w.geometry] : []));
  const status = seen.surfaces.filter((w) => w.layer === 0 && w.height <= 26 && frames.some((f) => w.x >= f.x && w.y >= f.y && w.x + w.width <= f.x + f.w && w.y + w.height <= f.y + f.h));
  if (status.length) note += ` link status bubble ${status.map((w) => `${w.width}x${w.height}`).join(",")}`;
  seen.surfaces = seen.surfaces.filter((w) => !status.includes(w));
  const surfaces = seen.surfaces.map((w) => `L${w.layer} ${w.width}x${w.height}@${w.x},${w.y}${w.title ? ` "${w.title}"` : ""}`);
  const problems: string[] = [];
  if (note.startsWith("run error")) problems.push(note);
  if (seen.surfaces.length > 0) problems.push(`surface(s) not the app's: ${surfaces.join(" | ")}`);
  if (route.expect.windows !== undefined && seen.newWindows !== route.expect.windows) problems.push(`app windows ${seen.newWindows >= 0 ? "+" : ""}${seen.newWindows}, expected +${route.expect.windows}`);
  if (route.expect.tabs !== undefined && seen.newTabs !== route.expect.tabs) problems.push(`app tabs ${seen.newTabs >= 0 ? "+" : ""}${seen.newTabs}, expected +${route.expect.tabs}`);
  const [bw, bh] = seen.viewport.split("->")[0]!.split("x").map(Number);
  const [aw, ah] = (seen.viewport.split("->")[1] ?? "").split("x").map(Number);
  if (bw && aw && seen.newTabs === 0 && seen.newWindows === 0 && (Math.abs(bw - aw) > 2 || Math.abs(bh! - ah!) > 2)) {
    problems.push(`page viewport changed ${seen.viewport} (Chromium chrome inside the view, or a zoom?)`);
  }
  const menu = takeMenu();
  if (menu) {
    problems.push(...menu.lint.map((l) => `menu ${l}`));
    note += ` menu capture ${menu.shot}`;
    console.log(`ND_ESCAPE_MENU ${route.name}: ${menu.items.map((i) => `${"  ".repeat(i.depth)}${i.kind === "separator" ? "---" : i.label}${i.enabled ? "" : " (disabled)"}`).join(" | ")}`);
  }
  if (seen.surfaces.length > 0 || problems.some((p) => p.startsWith("page viewport"))) note += ` capture ${regionShot(route.name, seen.surfaces.length > 0)}`;
  const detail = `windows ${seen.newWindows >= 0 ? "+" : ""}${seen.newWindows} tabs ${seen.newTabs >= 0 ? "+" : ""}${seen.newTabs} pages ${seen.pages >= 0 ? "+" : ""}${seen.pages} viewport ${seen.viewport}${surfaces.length ? ` surfaces ${surfaces.join(" | ")}` : ""}${note.trim() ? ` [${note.trim()}]` : ""}`;
  ran++;
  if (problems.length === 0) {
    console.log(`ND_ESCAPE_ROUTE ${route.name}: ok (${detail})`);
  } else if (route.expect.known) {
    console.log(`ND_ESCAPE_ROUTE ${route.name}: KNOWN ${route.expect.known}: ${problems.join("; ")} (${detail})`);
  } else {
    console.log(`ND_ESCAPE_ROUTE ${route.name}: ESCAPE ${problems.join("; ")} (${detail})`);
    failures.push(`${route.name}: ${problems.join("; ")}`);
  }
  // A window on screen that is not the app's takes focus and clicks from every
  // route after it, so the run stops here and the shell relaunches the host.
  if (seen.surfaces.some((w) => w.layer === 0 && w.width > 300 && w.height > 200)) {
    console.log(`ND_ESCAPE_DIRTY ${route.name}`);
    fixtures.stop();
    cursor.close();
    process.exit(3);
  }
}

fixtures.stop();
cursor.close();
console.log(`ND_ESCAPE_ROUTES ${ran} ran, ${failures.length} escaped${Date.now() > deadline ? ", budget spent" : ""}`);
if (failures.length > 0 && !explore) {
  for (const f of failures) console.error(`ND_ESCAPE_FAIL ${f}`);
  process.exit(1);
}
console.log("ND_ESCAPE_DRIVE_DONE");
process.exit(0);
