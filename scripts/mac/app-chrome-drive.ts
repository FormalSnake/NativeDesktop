#!/usr/bin/env bun
// Drives the REAL browser app (the nativebrowser checkout) under
// ND_CEF_STYLE=chrome and asserts what Chrome style has to hold once an app
// with tabs, a sidebar, popovers and its own menus is on top of it. The probe
// gate (scripts/mac/cef-chrome-style.sh) covers one static view; this covers
// the window, tab and input paths that only a real app exercises.
//
// Every assertion reads from something outside the app's own React state: the
// page's own innerWidth through CDP-free `webviewEval`, the window server's
// census, the AppKit geometry in the automation tree, and ScreenCaptureKit
// window captures read back as colour.
//
// ND_APP_CHROME_LEGS=<comma separated> runs a subset.
import { connectApp, type AttachedApp, type LocatorFactory } from "@nativedesktop/test";

import {
  DEVICE_TOOLBAR,
  DEVTOOLS_CLOSE,
  FRONTEND_SPLITTER,
  Session,
  frontendBox,
  inspectedPageBounds,
  targets,
  waitForTarget,
  type Box,
} from "../cdp.ts";

import {
  FIXTURE_FILL,
  FIXTURE_ORIGIN,
  FIXTURE_PORT,
  HOST_PID,
  KEY_DOWN_ARROW,
  KEY_ESCAPE,
  KEY_RETURN,
  KEY_RIGHT_ARROW,
  LegFailure,
  LegSkipped,
  SHOTS,
  activateApp,
  anchors,
  appWindowRect,
  assert,
  obstructions,
  capture,
  census,
  hostAlive,
  surfaces,
  censusHolds,
  colourDistance,
  menuStopsTo,
  menuWindows,
  pageEval,
  pageNumber,
  probePng,
  shownMenu,
  trackedMenus,
  startFixtureServer,
  systemKey,
  until,
  type Leg,
  type ShownMenuItem,
} from "./app-chrome-lib.ts";

const app = await connectApp();
const fixtures = startFixtureServer();
await Bun.$`mkdir -p ${SHOTS}`.quiet();

// The socket opens before React's first commit, so a tree read taken straight
// after connecting answers -32603 with no root.
await until("the app's first window", async () => (await app.windows()).windows.length, (n) => n > 0, 45000);

/// Every leg is pinned to the app's FIRST window. `getTree` walks one window at
/// a time and a private window opened mid-run would otherwise become the one a
/// later leg reads, which shows up as "no visible WebView" in every leg after.
const mainWindow = (await app.windows()).windows[0]!.ref;
const main: LocatorFactory = await app.window(0);

const skipDevTools = process.env.ND_APP_CHROME_SKIP_DEVTOOLS === "1";
const DEBUG_PORT = Number(process.env.ND_CEF_DEBUG_PORT ?? "9436");

// MARK: - app vocabulary

/// The app's tab ids are sequential and its page nodes are `page-<id>`; the
/// active one is the only WebView the tree reports as visible.
async function activePage(): Promise<string> {
  const tree = await app.tree(mainWindow);
  let found: string | null = null;
  const walk = (node: { type: string; testID: string | null; visible: boolean; children: unknown[] }) => {
    if (node.type === "WebView" && node.visible && node.testID?.startsWith("page-")) found ??= node.testID;
    for (const child of node.children) walk(child as never);
  };
  walk(tree.root as never);
  assert(found !== null, "no visible WebView in the tree");
  return found!;
}

/// cmd+L, the address, return. The app's own address bar, so this is the same
/// path a user takes and it exercises native-field focus on the way.
///
/// The field's own state is not readable: a SearchInput inside a HeaderBar
/// reports `visible: false` with a placeholder geometry and never echoes what
/// was typed into it, so the address landing is read from where the page ends
/// up instead.
async function openAddress(url: string): Promise<void> {
  await main.keyboard.press("Meta+l");
  await Bun.sleep(500);
  // cmd+L focuses the field without selecting what is in it, so a second
  // address types itself onto the end of the first.
  await main.keyboard.press("Meta+a");
  await main.keyboard.type(url);
  await main.keyboard.press("Enter");
}

async function loadFixture(page = "index.html"): Promise<string> {
  const url = `${FIXTURE_ORIGIN}/${page}`;
  for (let attempt = 0; attempt < 3; attempt++) {
    await openAddress(url);
    const id = await until("a webview for the loaded page", activePage, () => true, 15000).catch(() => null);
    if (id === null) continue;
    const landed = await until(
      `${page} finishes loading`,
      async () => `${await pageEval(app, id, "location.href")}|${await pageEval(app, id, "document.readyState")}`,
      (v) => v.startsWith(url) && v.endsWith("|complete"),
      15000,
    ).catch(() => null);
    if (landed !== null) return id;
  }
  throw new LegFailure(`the address bar never took ${url}`);
}

async function viewBox(testId: string): Promise<{ x: number; y: number; width: number; height: number }> {
  const box = await main.getByTestId(testId).boundingBox();
  assert(box !== null, `${testId} has no bounding box`);
  return box!;
}

/// The page's viewport has to end up the size of the AppKit view it is lifted
/// into. Chromium lays the web contents out from the anchor window, so this is
/// the one number that proves the two halves agree.
async function viewportMatchesView(testId: string, what: string): Promise<void> {
  const box = await viewBox(testId);
  const seen = await until(
    `${what}: the page viewport follows the view`,
    async () => await pageEval(app, testId, "innerWidth+'x'+innerHeight"),
    (value) => {
      const [w, h] = String(value).split("x").map(Number);
      return Math.abs(w - box.width) <= 2 && Math.abs(h - box.height) <= 2;
    },
    15000,
  );
  void seen;
  const live = await anchors();
  assert(
    live.some((a) => Math.abs(a.width - box.width) <= 1 && Math.abs(a.height - box.height) <= 1),
    `${what}: no anchor on the webview rectangle ${box.width}x${box.height}` +
      ` (anchors ${live.map((a) => `${a.width}x${a.height}`).join(",") || "none"})`,
  );
}

/// AX through System Events, for the window operations the automation socket
/// has no method for (minimize, native fullscreen, the zoom button).
function osaWindow(script: string): string {
  const source = `tell application "System Events" to tell (first process whose unix id is ${HOST_PID}) to tell window 1 to ${script}`;
  const run = Bun.spawnSync(["osascript", "-e", source]);
  if (run.exitCode !== 0) throw new LegFailure(`osascript failed: ${run.stderr.toString().trim()}`);
  return run.stdout.toString().trim();
}

/// A click the WINDOW SERVER routes, not one posted into the app's own queue.
/// Chromium raises its context menu (and its tooltips) off the system's own
/// right-click handling, which an NSEvent posted with `NSApp.postEvent` never
/// reaches, so those legs need the real thing.
/// Puts the cursor somewhere without clicking and without asserting anything,
/// so an obstruction check can be taken with the pointer where the click will
/// land rather than wherever the leg before left it.
function warpPointer(x: number, y: number): void {
  Bun.spawnSync(
    ["swift", "scripts/mac/mac-click.swift", String(Math.round(x)), String(Math.round(y)), "move"],
    { env: { ...process.env, SDKROOT: undefined, DEVELOPER_DIR: undefined } },
  );
}

function realPointer(x: number, y: number, mode: "left" | "right" | "move"): void {
  // The window server routes by cursor, so the app has to be the window under
  // it; this machine has other windows, and one in front takes the click.
  activateApp();
  const blocked = obstructions(x, y);
  assert(
    blocked.length === 0,
    `a window in front of the app covers ${Math.round(x)},${Math.round(y)}: ${blocked.join(" ")}`,
  );
  const run = Bun.spawnSync(
    ["swift", "scripts/mac/mac-click.swift", String(x), String(y), mode],
    { env: { ...process.env, SDKROOT: undefined, DEVELOPER_DIR: undefined } },
  );
  if (run.exitCode !== 0) throw new LegFailure(`mac-click failed: ${run.stderr.toString().trim()}`);
}

/// The system pasteboard, read or written. A copy has no other readable side:
/// the page cannot read the clipboard without a permission it was never given.
function pasteboard(write?: string): string {
  if (write !== undefined) {
    Bun.spawnSync(["pbcopy"], { stdin: Buffer.from(write) });
    return write;
  }
  return Bun.spawnSync(["pbpaste"]).stdout.toString();
}

/// One AX query against the host process, for the Chromium menus nothing in
/// the automation tree can see.
function osaProcess(script: string): string {
  const run = Bun.spawnSync([
    "osascript", "-e",
    `tell application "System Events" to tell (first process whose unix id is ${HOST_PID}) to ${script}`,
  ]);
  if (run.exitCode !== 0) throw new LegFailure(`osascript failed: ${run.stderr.toString().trim()}`);
  return run.stdout.toString().trim();
}

function wheel(x: number, y: number, dy: number, momentum = false): void {
  const args = ["swift", "scripts/mac/mac-wheel.swift", String(HOST_PID), String(x), String(y), String(dy), "6"];
  void 0;
  if (momentum) args.push("momentum");
  const run = Bun.spawnSync(args, { env: { ...process.env, SDKROOT: undefined, DEVELOPER_DIR: undefined } });
  if (run.exitCode !== 0) throw new LegFailure(`mac-wheel failed: ${run.stderr.toString().trim()}`);
}

/// Real pointer moves in and out of the webview and back and forth across its
/// four edges. Every enter and exit runs AppKit's tracking-area routing over
/// whatever the lift left in the host window, which is where an owner that no
/// longer answers `mouseEntered:` takes the process down.
async function pointerSweep(testId: string, what: string, passes = 3): Promise<void> {
  const box = await viewBox(testId);
  const points: [number, number][] = [
    [box.x + box.width / 2, box.y + box.height / 2],
    [box.x - 12, box.y + box.height / 2],
    [box.x + 6, box.y + box.height / 2],
    [box.x + box.width / 2, box.y - 12],
    [box.x + box.width / 2, box.y + 6],
    [box.x + box.width - 6, box.y + box.height / 2],
    [box.x + box.width + 12, box.y + box.height / 2],
    [box.x + box.width / 2, box.y + box.height - 6],
    [box.x + 20, box.y + 20],
  ];
  for (let pass = 0; pass < passes; pass++) {
    for (const [x, y] of points) {
      await main.mouse.move(x, y);
      await Bun.sleep(35);
      assert(hostAlive(), `${what}: the host died on a pointer move to ${Math.round(x)},${Math.round(y)}`);
    }
  }
  // A live host that stopped answering is the same failure seen from the
  // socket side rather than from the process table.
  await app.windows();
}

/// A point inside the page, in the CoreGraphics global coordinates the wheel
/// helper posts at. The app window's frame is its content rectangle, so the
/// census origin and the tree's window-topleft space share an origin.
async function globalPoint(testId: string, dx: number, dy: number): Promise<{ x: number; y: number }> {
  const [box, win] = await Promise.all([viewBox(testId), appWindowRect()]);
  return { x: win.x + box.x + dx, y: win.y + box.y + dy };
}

/// The app's own tab count, from the sidebar's row list. A tab with no URL yet
/// has no WebView, so counting views cannot see one.
async function tabCount(): Promise<number> {
  const tree = await app.tree(mainWindow);
  let count = 0;
  const walk = (node: { testID: string | null; rows: unknown[] | null; children: unknown[] }) => {
    if (node.testID === "tab-list") count = node.rows?.length ?? 0;
    for (const child of node.children) walk(child as never);
  };
  walk(tree.root as never);
  return count;
}

/// cmd+T, retried. The first key event after the app is activated can be eaten
/// by the activation itself, and a cmd+L that follows a cmd+T which did nothing
/// re-navigates the tab that was already there instead of opening a new one.
async function newTab(): Promise<void> {
  const before = await tabCount();
  for (let attempt = 0; attempt < 3; attempt++) {
    await main.keyboard.press("Meta+t");
    const grew = await until("a new tab", tabCount, (n) => n > before, 4000).catch(() => null);
    if (grew === null) continue;
    // The app shows its new-tab page for the tab with no URL yet, and that is
    // the signal that the NEW tab is the active one. Without it the address
    // bar still aims at the tab that was active when cmd+T was pressed.
    await until(
      "the new tab becomes the active one",
      () => main.getByTestId("new-tab-page").isVisible(),
      (visible) => visible === true,
      8000,
    );
    return;
  }
  throw new LegFailure(`cmd+T opened no tab (the app still has ${before})`);
}

/// The webview's rectangle inside a window capture, in image pixels.
async function captureViewRect(name: string, testId: string) {
  const [box, win] = await Promise.all([viewBox(testId), appWindowRect()]);
  const panelEnd = Math.min(
    box.height - 8,
    (await pageNumber(app, testId, "document.getElementById('panel').getBoundingClientRect().bottom")) + 8,
  );
  // By window number, never by pid: the anchor is a window of this process too
  // and it carries no pixels, so a pid-matched capture can come back black.
  const path = capture(name, win.number);
  const report = probePng(path);
  const scale = report.width / win.width;
  return {
    path,
    report,
    rect: {
      x: Math.round(box.x * scale),
      y: Math.round(box.y * scale),
      w: Math.round(box.width * scale),
      h: Math.round(box.height * scale),
    },
    // The fixture's controls sit in a panel across the top, so the flat fill
    // that a stale band would show up against starts below it. The panel wraps
    // to more rows as the window narrows, so where it ends comes from the page
    // rather than from a fixed fraction of the view.
    fill: {
      x: Math.round(box.x * scale),
      y: Math.round((box.y + panelEnd) * scale),
      w: Math.round(box.width * scale),
      h: Math.round((box.height - panelEnd) * scale),
    },
  };
}


/// A real click goes to the topmost window under the cursor, and this machine
/// carries other windows, some of them at layers the app cannot be raised above
/// (a system alert sits at the modal-panel layer). The window is moved until the
/// rectangle the leg is about to click in is the app's.
async function clearForRealPointer(testId: string): Promise<void> {
  const origins = [null, { x: 1080, y: 120 }, { x: 120, y: 120 }, { x: 1080, y: 620 }, { x: 120, y: 620 }];
  let blocked: string[] = [];
  for (const origin of origins) {
    if (origin) {
      await app.setWindowFrame(origin);
      await Bun.sleep(600);
    }
    const [box, win] = await Promise.all([viewBox(testId), appWindowRect()]);
    const spots = [
      [box.width / 2, box.height / 2],
      [30, 30],
      [box.width - 30, box.height - 30],
    ].map(([dx, dy]) => [win.x + box.x + dx!, win.y + box.y + dy!] as [number, number]);
    // The cursor goes to each point before the check: the Dock only comes out
    // when the pointer reaches the screen edge, so a census taken with the
    // cursor elsewhere does not see the window that will take the click.
    for (const [x, y] of spots) warpPointer(x, y);
    await Bun.sleep(500);
    blocked = spots.flatMap(([x, y]) => obstructions(x, y));
    if (blocked.length === 0) return;
  }
  throw new LegFailure(
    `no position on this screen leaves the webview clickable: ${[...new Set(blocked)].join(" ")}`,
  );
}

/// Opens the engine's context menu over the page and leaves it tracking. The
/// menu blocks the host's main thread for as long as it is up, so everything
/// between this and the pick has to stay off the automation socket.
async function openPageMenu(page: string, atY?: number): Promise<ShownMenuItem[]> {
  await clearForRealPointer(page);
  const box = await viewBox(page);
  // Inside the PAGE, not the middle of the view: a docked inspector is most of
  // the view, and a right click that lands in the frontend gets the frontend's
  // own menu, or nothing.
  const inner = await pageEval(app, page, "innerWidth+'x'+innerHeight").catch(() => null);
  const [pageWidth, pageHeight] = (inner ?? `${box.width}x${box.height}`).split("x").map(Number);
  const spot = await globalPoint(
    page,
    Math.min(box.width, pageWidth!) / 2,
    Math.min(atY ?? box.height / 2, pageHeight! - 20),
  );
  const tracked = trackedMenus();
  realPointer(spot.x, spot.y, "right");
  await until(
    "the context menu opens",
    menuWindows,
    (windows) => windows.length > 0,
    15000,
  );
  // On screen is not yet taking keys: a Down sent before the menu tracks goes
  // to the page, and the Return after it then closes the menu on nothing.
  await until("the context menu takes keys", async () => trackedMenus(), (n) => n > tracked, 5000);
  const items = shownMenu();
  assert(items.length > 0, "the host drew a menu window but reported no items");
  return items;
}

/// Picks one top-level item of the open menu with real key events. AppKit
/// publishes no accessibility element for a contextual menu, so the keyboard is
/// the only way in, exactly as on the Linux gate.
async function pickMenuItem(items: ShownMenuItem[], label: string): Promise<void> {
  systemKey(KEY_DOWN_ARROW, menuStopsTo(items, label));
  systemKey(KEY_RETURN);
  await until("the menu closes", menuWindows, (windows) => windows.length === 0, 10000);
}

/// The app puts Inspect Element in the page context menu, and Chromium's own
/// accelerators for the inspector are refused by the command handler, so this is
/// the only way in.
async function openInspector(page: string): Promise<void> {
  const box = await viewBox(page);
  // Mid-page: the bottom of the view can be under the Dock, and a real right
  // click there never reaches the app.
  const items = await openPageMenu(page, box.height / 2);
  // The app's own item when it has one, Chromium's otherwise: the engine keeps
  // Inspect in the model precisely because Chrome style docks the inspector.
  const label = items.some((i) => i.depth === 0 && i.label === "Inspect Element") ? "Inspect Element" : "Inspect";
  await pickMenuItem(items, label);
}

/// From the frontend's own close button. Chromium's Inspect item on a page
/// that already has an inspector inspects again rather than closing it, and
/// the Chrome command handler refuses F12 and cmd+alt+I.
async function closeInspector(page: string): Promise<void> {
  await closeFrontend(page);
}

/// What the frontend reserved for the page, with the frontend's own viewport
/// beside it.
/// Runs `body` against the docked frontend's own protocol session.
async function onFrontend<T>(body: (session: Session) => Promise<T>): Promise<T> {
  const target = await waitForTarget(DEBUG_PORT, (t) => t.url.startsWith("devtools://"), 20000);
  const session = await Session.open(target.webSocketDebuggerUrl ?? "");
  try {
    await session.send("Runtime.enable");
    return await body(session);
  } finally {
    session.close();
  }
}

/// Where a control of the docked frontend is, as a point in the window: the
/// frontend fills the page's view, so its viewport origin is the view's.
/// Every inspector the host has docked is also in the host view by now. A
/// frontend answers over the protocol, button boxes and all, before its
/// subtree is lifted into the host view, and a press in that gap lands on
/// nothing.
async function inspectorShown(): Promise<void> {
  const path = process.env.ND_APP_HOST_LOG;
  if (!path) return;
  const count = (pattern: string) =>
    Number(Bun.spawnSync(["rg", "-c", pattern, path]).stdout.toString().trim() || "0");
  await until(
    "the inspector is in the host view",
    async () => count("chrome devtools lifted") >= count("chrome devtools docked"),
    (shown) => shown,
    10000,
  );
}

async function frontendPoint(page: string, finder: string): Promise<{ x: number; y: number } | null> {
  await inspectorShown();
  const box = await onFrontend((session) => frontendBox(session, finder));
  if (box === null) return null;
  const view = await viewBox(page);
  return { x: view.x + box.x + box.width / 2, y: view.y + box.y + box.height / 2 };
}

/// Where the frontend saw pointer presses since the last call, in its own CSS
/// pixels. A real click that changed nothing is told apart from one that
/// never reached the frontend, or landed off its target, by this.
async function frontendPresses(): Promise<string[]> {
  return await onFrontend((session) => session.eval<string[]>(`(() => {
    if (!window.__ndPresses) {
      window.__ndPresses = [];
      document.addEventListener('pointerdown', (e) => window.__ndPresses.push(Math.round(e.clientX) + ',' + Math.round(e.clientY)), true);
    }
    return window.__ndPresses.splice(0);
  })()`));
}

/// The same record on the inspected page, which sits in the frontend's hole
/// and is the other place a press near the splitter can go.
async function pagePresses(page: string): Promise<string> {
  return (await pageEval(app, page, `(() => {
    if (!window.__ndPresses) {
      window.__ndPresses = [];
      document.addEventListener('pointerdown', (e) => window.__ndPresses.push(Math.round(e.clientX) + ',' + Math.round(e.clientY)), true);
    }
    return window.__ndPresses.splice(0).join(' ');
  })()`)) ?? "";
}

/// Where the pointer is, and what AppKit says is under it, for a failure that
/// has to say where a real press went.
async function pressReport(page: string, at: { x: number; y: number }): Promise<string> {
  const win = await appWindowRect();
  const over = obstructions(win.x + at.x, win.y + at.y);
  return `pressed at ${at.x},${at.y}, frontend saw [${(await frontendPresses()).join(" ")}], `
    + `page saw [${await pagePresses(page)}], in front: [${over.join(", ")}]`;
}

/// The inspector's own close button, which is the one path off the frontend
/// that this app's menus do not have: its Inspect item is Chromium's, and that
/// one only ever opens an inspector.
async function closeFrontend(page: string): Promise<void> {
  // The frontend draws its close button only once it has been told it is
  // docked, which is a poll on the host side that can land after the dock.
  const at = await until(
    "the frontend's toolbar shows its close button",
    () => frontendPoint(page, DEVTOOLS_CLOSE),
    (point) => point !== null,
    10000,
  ).catch(() => null);
  assert(at !== null, "the frontend's toolbar has no close button");
  await app.cursor.click(at!);
  const gone = await until(
    "the inspector goes away",
    async () => (await targets(DEBUG_PORT)).filter((t) => t.url.startsWith("devtools://")).length,
    (n) => n === 0,
    20000,
  ).catch(() => null);
  if (gone === null) {
    const frontendSize = await onFrontend((session) => session.eval<string>("innerWidth+'x'+innerHeight")).catch(() => "?");
    const view = await viewBox(page);
    throw new LegFailure(
      `the inspector stayed after its close button; frontend ${frontendSize}, view ${view.width}x${view.height}@${view.x},${view.y}; `
        + await pressReport(page, at!),
    );
  }
}

async function frontendGeometry(): Promise<{ bounds: Box | null; width: number; height: number }> {
  const target = await waitForTarget(DEBUG_PORT, (t) => t.url.startsWith("devtools://"), 20000);
  const session = await Session.open(target.webSocketDebuggerUrl ?? "");
  try {
    return await inspectedPageBounds(session);
  } finally {
    session.close();
  }
}

/// Where the docked inspector leaves the page, against where the page actually
/// is. The two X/NS windows tiling the webview says nothing about this: told it
/// can dock, the frontend lays its panels out around an inspected-page
/// placeholder and expects the embedder to put the page in that rectangle. A
/// placeholder that is not the page's own rectangle is the empty strip the
/// owner sees, and it is also what device mode draws the phone into.
/// The rectangle the host last put the page's own view at, off its trace. In
/// device mode the page's document reports the emulated viewport rather than
/// the view it is drawn in, and this is the only handle on the view itself.
function appliedPageRect(): { x: number; y: number; w: number; h: number } | null {
  const path = process.env.ND_APP_HOST_LOG;
  if (!path) return null;
  const log = Bun.spawnSync(["rg", "-o", "chrome devtools page rect \\d+x\\d+@\\d+,\\d+", path]);
  const lines = log.stdout.toString().trim().split("\n").filter((line) => line.length > 0);
  const last = lines[lines.length - 1];
  const m = last?.match(/(\d+)x(\d+)@(\d+),(\d+)/);
  return m ? { w: Number(m[1]), h: Number(m[2]), x: Number(m[3]), y: Number(m[4]) } : null;
}

async function dockTiles(page: string, phase: string, emulated = false): Promise<void> {
  let detail = "never measured";
  for (let attempt = 0; attempt < 15; attempt++) {
    const box = await viewBox(page);
    const inner = (await pageEval(app, page, "innerWidth+'x'+innerHeight")) ?? "";
    const [pw, ph] = inner.split("x").map(Number);
    const tools = await frontendGeometry().catch(() => null);
    if (tools === null) {
      detail = `${phase}: no devtools target`;
    } else if (tools.bounds === null) {
      detail = `${phase}: the frontend announced no page bounds`;
    } else {
      const hole = tools.bounds;
      const applied = appliedPageRect();
      detail = `${phase}: page ${pw}x${ph}, view ${applied ? `${applied.w}x${applied.h}@${applied.x},${applied.y}` : "untraced"}`
        + `, hole ${hole.width}x${hole.height}@${hole.x},${hole.y}`
        + `, inspector ${tools.width} wide, webview ${Math.round(box.width)}`;
      // The view the page is drawn in has to BE the hole, origin included.
      // Sizes only for the document itself: Chromium's screenX for a
      // BrowserView reparented into this window is the popup's original
      // position and never moves, so the two documents are not in one space,
      // and under device emulation the page reports the device's viewport
      // rather than the view at all.
      const placed = applied !== null && Math.abs(applied.x - hole.x) <= 2 && Math.abs(applied.y - hole.y) <= 2
        && Math.abs(applied.w - hole.width) <= 2 && Math.abs(applied.h - hole.height) <= 2;
      const fits = emulated || (Math.abs(hole.width - pw!) <= 2 && Math.abs(hole.height - ph!) <= 2);
      const spans = Math.abs(tools.width - box.width) <= 2;
      if (placed && fits && spans) return;
    }
    await Bun.sleep(1000);
  }
  throw new LegFailure(detail);
}

// MARK: - legs

let mainPage = "";

const legs: Leg[] = [
  {
    name: "initialSize",
    run: async () => {
      mainPage = await loadFixture();
      await viewportMatchesView(mainPage, "initial");
    },
  },
  {
    name: "pointerAcrossTheWebviewEdges",
    run: async () => {
      await pointerSweep(mainPage, "on a freshly loaded page", 4);
      await loadFixture("page2.html");
      await pointerSweep(await activePage(), "after a navigation", 4);
      await loadFixture();
      mainPage = await activePage();
      await pointerSweep(mainPage, "after navigating back", 4);
    },
  },
  {
    name: "resizeLarger",
    run: async () => {
      await app.setWindowSize(1180, 900);
      await viewportMatchesView(mainPage, "larger");
    },
  },
  {
    name: "resizeSmaller",
    run: async () => {
      await app.setWindowSize(700, 520);
      await viewportMatchesView(mainPage, "smaller");
    },
  },
  {
    name: "resizeRandom",
    run: async () => {
      const relifts = () => countTrace("chrome relift");
      const before = relifts();
      for (let i = 0; i < 20; i++) {
        const w = 640 + Math.floor(Math.random() * 560);
        const h = 480 + Math.floor(Math.random() * 420);
        await app.setWindowSize(w, h);
        await Bun.sleep(60);
      }
      await app.setWindowSize(980, 760);
      await viewportMatchesView(mainPage, "after 20 random sizes");
      await pointerSweep(mainPage, "after 20 resizes");
      const storm = relifts() - before;
      assert(storm <= 4, `${storm} relifts across 20 resizes, which is a relift storm`);
    },
  },
  {
    name: "liveResizeVisual",
    run: async () => {
      // A capture taken between two size changes: the page has to fill the
      // view with its own colour, with no band the compositor left behind.
      await app.setWindowSize(900, 700);
      await viewportMatchesView(mainPage, "pre-live");
      await app.setWindowSize(1120, 860);
      const shot = await captureViewRect("live-resize", mainPage);
      const inside = probePng(shot.path, shot.fill);
      const stale = inside.bands.filter((band) => colourDistance(band, FIXTURE_FILL) > 90);
      assert(
        stale.length === 0,
        `${stale.length}/${inside.bands.length} bands off the page colour: ${JSON.stringify(inside.bands)}`,
      );
    },
  },
  {
    name: "windowMove",
    run: async () => {
      const before = await appWindowRect();
      await app.setWindowFrame({ x: before.x + 60, y: before.y + 40 });
      await until(
        "the anchor follows the window",
        async () => {
          const [box, win] = await Promise.all([viewBox(mainPage), appWindowRect()]);
          const live = await anchors();
          return live.some((a) => Math.abs(a.x - (win.x + box.x)) <= 1 && Math.abs(a.y - (win.y + box.y)) <= 1);
        },
        (ok) => ok,
        10000,
      );
    },
  },
  {
    name: "minimizeRestore",
    run: async () => {
      osaWindow("set value of attribute \"AXMinimized\" to true");
      await until("the anchor leaves the screen", anchors, (a) => a.length === 0, 10000);
      osaWindow("set value of attribute \"AXMinimized\" to false");
      await until("the anchor comes back", anchors, (a) => a.length === 1, 10000);
      await viewportMatchesView(mainPage, "restored");
    },
  },
  {
    name: "nativeFullscreen",
    run: async () => {
      await main.keyboard.press("Meta+Control+f");
      await until(
        "the window reaches the screen width",
        async () => (await appWindowRect()).width,
        (w) => w > 1600,
        30000,
      );
      await viewportMatchesView(mainPage, "fullscreen");
      await main.keyboard.press("Meta+Control+f");
      await until(
        "the window comes back out",
        async () => (await appWindowRect()).width,
        (w) => w < 1600,
        30000,
      );
      await viewportMatchesView(mainPage, "out of fullscreen");
      await pointerSweep(mainPage, "after fullscreen");
    },
  },
  {
    name: "windowZoom",
    run: async () => {
      // The green title-bar button enters fullscreen on a full-size content
      // window and publishes itself as AXFullScreenButton, so the window is
      // zoomed the other way a user reaches it: the Window menu's Zoom item,
      // which is `zoom:` down the responder chain.
      const buttons = osaWindow("get subrole of every button").toLowerCase();
      assert(
        buttons.includes("axfullscreenbutton") || buttons.includes("axzoombutton"),
        `the window publishes no green button at all (buttons: ${buttons})`,
      );
      const before = await appWindowRect();
      osaProcess('click menu item "Zoom" of menu 1 of menu bar item "Window" of menu bar 1');
      await until(
        "Zoom changes the frame",
        async () => (await appWindowRect()).height,
        (h) => Math.abs(h - before.height) > 8,
        10000,
      );
      await viewportMatchesView(mainPage, "zoomed");
      osaProcess('click menu item "Zoom" of menu 1 of menu bar item "Window" of menu bar 1');
      await until(
        "Zoom restores the frame",
        async () => (await appWindowRect()).height,
        (h) => Math.abs(h - before.height) <= 8,
        10000,
      );
      await viewportMatchesView(mainPage, "unzoomed");
    },
  },
  {
    name: "compactLayout",
    run: async () => {
      const wide = await viewBox(mainPage);
      await main.getByTestId("layout-toggle").click();
      await until(
        "the sidebar leaves and the view grows",
        async () => (await viewBox(mainPage)).width,
        (w) => w > wide.width + 100,
        10000,
      );
      await viewportMatchesView(mainPage, "compact");
      await main.getByTestId("layout-toggle").click();
      await until(
        "the sidebar comes back",
        async () => (await viewBox(mainPage)).width,
        (w) => Math.abs(w - wide.width) <= 2,
        10000,
      );
      await viewportMatchesView(mainPage, "sidebar");
    },
  },
  {
    name: "twoTabs",
    run: async () => {
      await newTab();
      const second = await loadFixture("page2.html");
      assert(second !== mainPage, `the new tab reused ${second}`);
      // The hidden tab's anchor has to be off screen, and exactly one anchor
      // is left for the tab on show.
      await until("one anchor for two tabs", anchors, (a) => a.length === 1, 10000);
      await viewportMatchesView(second, "second tab");
      const frames = await pageNumber(app, mainPage, "window.__ndFrames").catch(() => -1);
      assert(frames !== 0, "the hidden tab never ran a frame at all");
    },
  },
  {
    name: "fiveTabs",
    run: async () => {
      const ids = [mainPage, await activePage()];
      for (let i = 0; i < 3; i++) {
        await newTab();
        ids.push(await loadFixture(i % 2 === 0 ? "index.html" : "page2.html"));
      }
      assert(
        (await tabCount()) === 5,
        `expected five tabs, the app has ${await tabCount()} (${ids.join(",")})`,
      );
      await until("one anchor for five tabs", anchors, (a) => a.length === 1, 10000);
      await censusHolds(app, "fiveTabs");
    },
  },
  {
    name: "switchToTabResizedWhileHidden",
    run: async () => {
      // The case a hidden tab cannot see: the window changes size while its
      // view is off screen, so the size it wakes up at can only come from the
      // lift and the anchor sync, never from a resize it observed.
      const shown = await activePage();
      await app.setWindowSize(760, 560);
      await viewportMatchesView(shown, "resized while others hidden");
      await main.getByTestId("menu-prev-tab").click();
      const back = await until("a different tab on show", activePage, (id) => id !== shown, 10000);
      await viewportMatchesView(back, "tab woken at the new size");
      const framesBefore = await pageNumber(app, back, "window.__ndFrames");
      await until(
        "the woken tab paints",
        () => pageNumber(app, back, "window.__ndFrames"),
        (n) => n > framesBefore,
        10000,
      );
    },
  },
  {
    name: "closeTab",
    run: async () => {
      const before = (await app.tree(mainWindow)).root;
      const count = countWebViews(before as never);
      await main.keyboard.press("Meta+w");
      await until(
        "one tab fewer",
        async () => countWebViews((await app.tree(mainWindow)).root as never),
        (n) => n === count - 1,
        10000,
      );
      await until("still one anchor", anchors, (a) => a.length === 1, 10000);
      await pointerSweep(await activePage(), "after a tab close");
      await censusHolds(app, "closeTab");
    },
  },
  {
    name: "privateWindow",
    run: async () => {
      await main.getByTestId("menu-private-window").click();
      await until("a second app window", async () => (await app.windows()).windows.length, (n) => n === 2, 10000);
      await censusHolds(app, "privateWindow");
      const windows = (await app.windows()).windows;
      const priv = windows.find((w) => (w.title ?? "").length >= 0 && w.ref !== windows[0]!.ref)!;
      await app.rpc.call("click", { testId: "private-new-tab", window: priv.ref }).catch(() => undefined);
      await censusHolds(app, "privateWindowUsed");
    },
  },
  {
    name: "closeWindowWithTabs",
    run: async () => {
      // Its own second window: the runner's precondition closes every extra
      // window before each leg, so one left open by the leg before is gone.
      await main.getByTestId("menu-private-window").click();
      await until("a second app window", async () => (await app.windows()).windows.length, (n) => n === 2, 10000);
      const windows = (await app.windows()).windows;
      assert(windows.length >= 2, `the private window did not open, saw ${windows.length}`);
      // The app's cmd+W is one menu item acting on the main window's active
      // tab, so the private window goes out through its own close button.
      osaWindow('perform action "AXPress" of (first button whose subrole is "AXCloseButton")');
      await until("back to one app window", async () => (await app.windows()).windows.length, (n) => n === 1, 15000);
      await until("no orphan anchor", anchors, (a) => a.length === 1, 10000);
      await pointerSweep(await activePage(), "after the second window closed");
      await censusHolds(app, "closeWindowWithTabs");
    },
  },
  {
    name: "pageInputFocusAndTyping",
    run: async () => {
      const id = await activePage();
      if ((await pageEval(app, id, "!!document.getElementById('text')")) !== "true") {
        await loadFixture();
      }
      const page = await activePage();
      await pageEval(app, page, "document.getElementById('text').value=''");
      const field = JSON.parse(
        (await pageEval(app, page, "JSON.stringify(document.getElementById('text').getBoundingClientRect())")) ?? "{}",
      );
      const box = await viewBox(page);
      await main.mouse.click(box.x + field.x + 20, box.y + field.y + field.height / 2);
      await until(
        "the page takes focus",
        () => pageEval(app, page, "String(document.hasFocus())"),
        (v) => v === "true",
        10000,
      );
      await main.keyboard.type("abc");
      await until(
        "the page field takes the keystrokes",
        () => pageEval(app, page, "document.getElementById('text').value"),
        (v) => v === "abc",
        10000,
      );
    },
  },
  {
    name: "nativeFieldKeepsItsKeystrokes",
    run: async () => {
      // cmd+L moves focus out of the web contents and into the app's own
      // field. What proves it is the page NOT seeing the keystrokes, and the
      // address the field was given driving the next navigation.
      const page = await activePage();
      await pageEval(app, page, "document.getElementById('text').value='keep'");
      const target = `${FIXTURE_ORIGIN}/page2.html`;
      await main.keyboard.press("Meta+l");
      await Bun.sleep(500);
      await main.keyboard.type(target);
      const stillThere = await pageEval(app, page, "document.getElementById('text').value");
      assert(
        stillThere === "keep",
        `the page field read ${JSON.stringify(stillThere)}, so the address bar's keystrokes reached the page`,
      );
      await main.keyboard.press("Enter");
      const landed = await until(
        "the address the field was given loads",
        async () => await pageEval(app, await activePage(), "location.href"),
        (href) => String(href).startsWith(target),
        15000,
      );
      void landed;
      mainPage = await loadFixture();
    },
  },
  {
    name: "appAccelerators",
    run: async () => {
      // cmd+T, cmd+W and cmd+R have to reach the APP's menu, not Chromium's:
      // the Chrome command handler refuses its own and the app's key
      // equivalents are what run.
      // A new tab is the app's own page with no WebView in it, so the count of
      // WebViews says nothing about whether cmd+T has landed. What does is the
      // page going off screen, and cmd+W bringing it back.
      const tabs = countWebViews((await app.tree(mainWindow)).root as never);
      const before = await activePage();
      const shown = async () => await activePage().catch(() => "");
      await main.keyboard.press("Meta+t");
      await until("cmd+T opens an app tab", shown, (id) => id !== before, 10000);
      await main.keyboard.press("Meta+w");
      await until("cmd+W closes it again", shown, (id) => id === before, 10000);
      await until(
        "the tab count is back",
        async () => countWebViews((await app.tree(mainWindow)).root as never),
        (n) => n === tabs,
        10000,
      );
      const page = before;
      const frames = await pageNumber(app, page, "window.__ndFrames");
      await main.keyboard.press("Meta+r");
      await until(
        "cmd+R reloads the page",
        () => pageNumber(app, page, "window.__ndFrames"),
        (n) => n < frames,
        15000,
      );
      await censusHolds(app, "appAccelerators");
    },
  },
  {
    name: "editingKeysInThePage",
    run: async () => {
      const page = await activePage();
      await pageEval(app, page, "scrollTo(0,0); document.getElementById('text').value=''");
      const field = JSON.parse(
        (await pageEval(app, page, "JSON.stringify(document.getElementById('text').getBoundingClientRect())")) ?? "{}",
      );
      await clearForRealPointer(page);
      const spot = await globalPoint(page, field.x + 20, field.y + field.height / 2);
      // Retried, the way cmd+T is above: the leg before this one reloads the
      // page, and a click that lands in the second after a reload is swallowed
      // before the page sees it. One click that never arrived reads exactly
      // like a field that refuses focus.
      await until(
        "the page field takes focus",
        async () => {
          realPointer(spot.x, spot.y, "left");
          await Bun.sleep(700);
          return await pageEval(app, page, "String(document.activeElement.id)");
        },
        (v) => v === "text",
        12000,
      );
      await main.keyboard.type("copyme");

      await main.keyboard.press("Meta+a");
      await until(
        "cmd+A selects inside the page's field",
        () => pageEval(app, page, "String(document.getElementById('text').selectionEnd - document.getElementById('text').selectionStart)"),
        (v) => v === "6",
        10000,
      );

      pasteboard("nd-not-copied-yet");
      await main.keyboard.press("Meta+c");
      // The system pasteboard is the only readable side of a copy: the page
      // cannot read the clipboard without a permission it was never granted.
      await until(
        "cmd+C puts the page's selection on the pasteboard",
        async () => pasteboard(),
        (v) => v === "copyme",
        10000,
      );

      pasteboard("pasted-in");
      await main.keyboard.press("Meta+v");
      await until(
        "cmd+V pastes into the page's field",
        () => pageEval(app, page, "document.getElementById('text').value"),
        (v) => v === "pasted-in",
        10000,
      );

      await main.keyboard.press("Meta+z");
      await until(
        "cmd+Z undoes inside the page",
        () => pageEval(app, page, "document.getElementById('text').value"),
        (v) => v === "copyme",
        10000,
      );

      await main.keyboard.press("Meta+Shift+z");
      await until(
        "cmd+shift+Z redoes inside the page",
        () => pageEval(app, page, "document.getElementById('text').value"),
        (v) => v === "pasted-in",
        10000,
      );

      await main.keyboard.press("Meta+a");
      pasteboard("nd-not-cut-yet");
      await main.keyboard.press("Meta+x");
      await until(
        "cmd+X takes the page's selection to the pasteboard",
        async () => pasteboard(),
        (v) => v === "pasted-in",
        10000,
      );
      await until(
        "cmd+X empties the page's field",
        () => pageEval(app, page, "document.getElementById('text').value"),
        (v) => v === "",
        10000,
      );
    },
  },
  {
    name: "editingKeysInAContentEditable",
    run: async () => {
      // A contenteditable is a different editing path in Blink from a text
      // input, and the Edit menu has to reach it the same way.
      const page = await activePage();
      await pageEval(app, page, "scrollTo(0,0); document.getElementById('edit').textContent=''");
      const region = JSON.parse(
        (await pageEval(app, page, "JSON.stringify(document.getElementById('edit').getBoundingClientRect())")) ?? "{}",
      );
      const box = await viewBox(page);
      await main.mouse.click(box.x + region.x + 8, box.y + region.y + region.height / 2);
      await until(
        "the contenteditable takes focus",
        () => pageEval(app, page, "String(document.activeElement.id)"),
        (v) => v === "edit",
        10000,
      );
      await main.keyboard.type("richtext");

      await main.keyboard.press("Meta+a");
      pasteboard("nd-not-copied-yet");
      await main.keyboard.press("Meta+c");
      await until(
        "cmd+C copies out of the contenteditable",
        async () => pasteboard(),
        (v) => v.trim() === "richtext",
        10000,
      );

      pasteboard("replacement");
      await main.keyboard.press("Meta+a");
      await main.keyboard.press("Meta+v");
      await until(
        "cmd+V pastes into the contenteditable",
        () => pageEval(app, page, "document.getElementById('edit').textContent"),
        (v) => String(v).trim() === "replacement",
        10000,
      );

      await main.keyboard.press("Meta+z");
      await until(
        "cmd+Z undoes inside the contenteditable",
        () => pageEval(app, page, "document.getElementById('edit').textContent"),
        (v) => String(v).trim() === "richtext",
        10000,
      );
    },
  },
  {
    name: "editingKeysInTheAddressField",
    run: async () => {
      // The same chords with a NATIVE field focused have to act on the field,
      // not on the page. The field's own text is unreadable (a SearchInput in a
      // HeaderBar reports no geometry to automation), so the pasteboard and the
      // address that ends up loading are what prove it.
      const page = await activePage();
      const here = await pageEval(app, page, "location.href");
      await main.keyboard.press("Meta+l");
      await Bun.sleep(500);
      await main.keyboard.press("Meta+a");
      pasteboard("nd-not-copied-yet");
      await main.keyboard.press("Meta+c");
      const copied = await until(
        "cmd+C copies the address out of the app's own field",
        async () => pasteboard(),
        (v) => v.includes(`127.0.0.1:${FIXTURE_PORT}`),
        10000,
      );
      assert(
        copied !== "nd-not-copied-yet" && String(here).includes(copied.trim().replace(/^https?:\/\//, "").split("/")[0]!),
        `the field copied ${JSON.stringify(copied)}, which is not where the page is (${here})`,
      );

      pasteboard(`${FIXTURE_ORIGIN}/page2.html`);
      await main.keyboard.press("Meta+a");
      await main.keyboard.press("Meta+v");
      await main.keyboard.press("Enter");
      await until(
        "cmd+V pastes into the app's own field and that address loads",
        async () => await pageEval(app, await activePage(), "location.href"),
        (href) => String(href).startsWith(`${FIXTURE_ORIGIN}/page2.html`),
        15000,
      );
      mainPage = await loadFixture();
    },
  },
  {
    name: "tabKeyIntoAndOutOfThePage",
    run: async () => {
      const page = await activePage();
      // An AppKit button does not take first responder when it is clicked, so
      // this only clears page focus for a page that never had it. Starting from
      // the address bar instead does not work: with Full Keyboard Access off
      // (the default) Tab walks the chrome's text fields and never leaves them,
      // so a page that has held focus cannot be tabbed back into at all. That
      // is the open "tab into the page" work, not this leg's to paper over.
      await main.getByTestId("reload").click();
      await until(
        "focus starts in the app's own chrome",
        () => pageEval(app, page, "String(document.hasFocus())"),
        (v) => v === "false",
        10000,
      );
      // Tab walks the native chrome's focus ring before it reaches the web
      // contents; how many stops that takes is the app's business, so the leg
      // bounds it rather than assuming one.
      let entered = false;
      for (let stop = 0; stop < 8 && !entered; stop++) {
        await main.keyboard.press("Tab");
        await Bun.sleep(200);
        entered = (await pageEval(app, page, "String(document.hasFocus())")) === "true";
      }
      assert(entered, "eight tab stops never reached the web contents");
      await main.keyboard.press("Meta+l");
      await until(
        "the app's own chrome takes focus back",
        () => pageEval(app, page, "String(document.hasFocus())"),
        (v) => v === "false",
        10000,
      );
      await main.keyboard.press("Escape");
    },
  },
  {
    name: "wheelScroll",
    run: async () => {
      const page = await activePage();
      await pageEval(app, page, "scrollTo(0,0)");
      const point = await globalPoint(page, 200, 300);
      wheel(point.x, point.y, -600);
      await until(
        "the wheel scrolls the page",
        () => pageNumber(app, page, "Math.round(scrollY)"),
        (y) => y > 50,
        10000,
      );
    },
  },
  {
    name: "momentumScroll",
    run: async () => {
      const page = await activePage();
      await pageEval(app, page, "scrollTo(0,0)");
      const point = await globalPoint(page, 200, 300);
      wheel(point.x, point.y, -900, true);
      await until(
        "a momentum fling scrolls the page",
        () => pageNumber(app, page, "Math.round(scrollY)"),
        (y) => y > 50,
        10000,
      );
    },
  },
  {
    name: "textSelectionDrag",
    run: async () => {
      const page = await activePage();
      await pageEval(app, page, "scrollTo(0,0); getSelection().removeAllRanges()");
      const prose = JSON.parse(
        (await pageEval(app, page, "JSON.stringify(document.getElementById('prose').getBoundingClientRect())")) ?? "{}",
      );
      const box = await viewBox(page);
      await main.mouse.dragTo(
        { x: box.x + prose.x + 4, y: box.y + prose.y + prose.height / 2 },
        { x: box.x + prose.x + prose.width - 4, y: box.y + prose.y + prose.height / 2 },
        { steps: 16, durationMs: 220 },
      );
      await until(
        "the drag selects text in the page",
        () => pageEval(app, page, "String(getSelection()).length"),
        (v) => Number(v) > 5,
        10000,
      );
    },
  },
  {
    name: "selectDropdown",
    run: async () => {
      const page = await activePage();
      const select = JSON.parse(
        (await pageEval(app, page, "JSON.stringify(document.getElementById('sel').getBoundingClientRect())")) ?? "{}",
      );
      const box = await viewBox(page);
      const win = await appWindowRect();
      await main.mouse.click(box.x + select.x + select.width / 2, box.y + select.y + select.height / 2);
      // Chromium draws the popup against the anchor's screen bounds, so it has
      // to land over the control rather than at the screen origin.
      const popup = await until(
        "the select popup opens",
        async () => (await surfaces(app)).filter((w) => w.alpha > 0),
        (windows) => windows.length > 0,
        8000,
      );
      const wanted = { x: win.x + box.x + select.x, y: win.y + box.y + select.y };
      assert(
        popup.some((w) => Math.abs(w.x - wanted.x) < 220 && Math.abs(w.y - wanted.y) < 260),
        `the popup opened at ${popup.map((w) => `${w.x},${w.y}`).join(" ")} rather than near ${wanted.x},${wanted.y}`,
      );
      await main.keyboard.press("ArrowDown");
      await main.keyboard.press("Enter");
      await until(
        "the select commits a new value",
        () => pageEval(app, page, "document.getElementById('sel').value"),
        (v) => v === "b" || v === "c",
        10000,
      );
      await censusHolds(app, "selectDropdown");
    },
  },
  {
    name: "tooltip",
    run: async () => {
      const page = await activePage();
      await clearForRealPointer(page);
      const tip = JSON.parse(
        (await pageEval(app, page, "JSON.stringify(document.getElementById('tip').getBoundingClientRect())")) ?? "{}",
      );
      // A real pointer move: Chromium raises the `title` tooltip off the
      // window server's own tracking, not off an NSEvent in the app's queue.
      const spot = await globalPoint(page, tip.x + tip.width / 2, tip.y + tip.height / 2);
      realPointer(spot.x, spot.y, "move");
      const seen = await until(
        "a tooltip surface appears over the page",
        async () => (await surfaces(app)).filter((w) => w.alpha > 0).length,
        (n) => n > 0,
        15000,
      );
      void seen;
      const corner = await globalPoint(page, 8, 8);
      realPointer(corner.x, corner.y, "move");
      await censusHolds(app, "tooltip");
    },
  },
  {
    name: "contextMenuOnAPage",
    run: async () => {
      // Chromium's own page menu, drawn by the host. What has to be there is
      // what Chrome offers for a click on nothing in particular, and what has
      // to be gone is every command the engine refuses.
      const page = await activePage();
      const items = await openPageMenu(page);
      const labels = items.filter((i) => i.depth === 0).map((i) => i.label.toLowerCase());
      systemKey(KEY_ESCAPE);
      await until("the menu closes", menuWindows, (windows) => windows.length === 0, 10000);
      for (const wanted of ["back", "forward", "reload"]) {
        assert(labels.some((l) => l.startsWith(wanted)), `the page menu has no "${wanted}" (${labels.join(" | ")})`);
      }
      for (const refused of ["view page source", "print", "cast", "create qr code"]) {
        assert(!labels.some((l) => l.includes(refused)), `the page menu still offers "${refused}"`);
      }
      // Escape answers the callback, and a page that never got its answer
      // cannot open a second menu.
      const again = await openPageMenu(page);
      assert(again.length > 0, "a second right-click drew no menu, so the first was never answered");
      systemKey(KEY_ESCAPE);
      await until("the second menu closes", menuWindows, (windows) => windows.length === 0, 10000);
    },
  },
  {
    name: "contextMenuOnALinkImageSelectionAndEditable",
    run: async () => {
      const page = await activePage();
      await clearForRealPointer(page);
      const box = await viewBox(page);
      async function menuOver(selector: string, what: string, expected: string[]): Promise<void> {
        const rect = JSON.parse(
          (await pageEval(app, page, `JSON.stringify(document.querySelector('${selector}').getBoundingClientRect())`)) ??
            "{}",
        );
        const spot = await globalPoint(page, rect.x + rect.width / 2, rect.y + rect.height / 2);
        realPointer(spot.x, spot.y, "right");
        await until(`the ${what} menu opens`, menuWindows, (windows) => windows.length > 0, 15000);
        const labels = shownMenu().filter((i) => i.depth === 0).map((i) => i.label.toLowerCase());
        systemKey(KEY_ESCAPE);
        await until(`the ${what} menu closes`, menuWindows, (windows) => windows.length === 0, 10000);
        for (const wanted of expected) {
          assert(labels.some((l) => l.includes(wanted)), `the ${what} menu has no "${wanted}" (${labels.join(" | ")})`);
        }
      }
      void box;
      await menuOver("#blank", "link", ["open link in new tab", "copy link address"]);
      await menuOver("#logo", "image", ["copy image", "save image as"]);
      await pageEval(app, page, "const r=document.createRange();r.selectNodeContents(document.getElementById('prose'));getSelection().removeAllRanges();getSelection().addRange(r);1");
      await menuOver("#prose", "selection", ["copy"]);
      await menuOver("#text", "editable", ["paste"]);
    },
  },
  {
    name: "contextMenuWithExtensionItem",
    run: async () => {
      const page = await activePage();
      await pageEval(app, page, "delete document.documentElement.dataset.ndMenu");
      const items = await openPageMenu(page);
      const labels = items.filter((i) => i.depth === 0).map((i) => i.label.toLowerCase());
      // The app's own "Inspect Element" and the extension's chrome.contextMenus
      // entry, with its submenu, both have to be in the menu the user sees.
      assert(labels.includes("inspect element"), `the app's items are not in the menu (${labels.join(" | ")})`);
      assert(labels.includes("nd probe item"), `the extension's item is not in the menu (${labels.join(" | ")})`);
      // The extension declares a child under its item, so Chromium renders it
      // as a submenu and the handler only fires for the child.
      const submenu = items.filter((i) => i.depth > 0).map((i) => i.label);
      assert(
        submenu.includes("ND probe child"),
        `the extension's submenu is not in the menu (${submenu.join(" | ") || "nothing nested"})`,
      );
      systemKey(KEY_DOWN_ARROW, menuStopsTo(items, "ND probe item"));
      systemKey(KEY_RIGHT_ARROW);
      systemKey(KEY_DOWN_ARROW);
      systemKey(KEY_RETURN);
      await until("the menu closes", menuWindows, (windows) => windows.length === 0, 10000);
      await until(
        "the extension's handler runs for its own item",
        () => pageEval(app, page, "document.documentElement.dataset.ndMenu ?? ''"),
        (v) => v === "nd-probe-sub",
        15000,
      );
      await censusHolds(app, "contextMenu");
    },
  },
  {
    name: "appOverlayDrawsOverTheWebContents",
    run: async () => {
      const page = await activePage();
      await pageEval(app, page, "scrollTo(0,0)");
      const before = await captureViewRect("overlay-before", page);
      const clean = probePng(before.path, before.fill);
      assert(
        colourDistance(clean.mean, FIXTURE_FILL) < 90,
        `the page is not filling the view before the overlay: mean ${JSON.stringify(clean.mean)}`,
      );
      // The find bar is the app's own chrome, and opening it takes a strip the
      // web contents held a moment ago. The lift makes z-order ordinary AppKit
      // sibling order, so those pixels have to become the app's: Chromium's
      // layer drawing on regardless is what this catches. Both probes read the
      // SAME window rectangle, the one the webview filled before the bar.
      await main.keyboard.press("Meta+f");
      await until("the find bar opens", () => main.getByTestId("find-bar").isVisible(), (v) => v === true, 10000);
      const after = await captureViewRect("overlay-after", page);
      const over = probePng(after.path, before.rect);
      const clean2 = probePng(before.path, before.rect);
      assert(
        colourDistance(over.mean, clean2.mean) > 6,
        `the find bar changed nothing in the rectangle the web contents filled (${JSON.stringify(clean2.mean)}` +
          ` vs ${JSON.stringify(over.mean)}), so Chromium's layer is still drawing there`,
      );
      await main.keyboard.press("Escape");
    },
  },
  {
    name: "javascriptDialogs",
    run: async () => {
      const page = await activePage();
      const box = await viewBox(page);
      for (const [button, expect] of [
        ["alert", "dlg=alert"],
        ["confirm", "dlg=confirm:true"],
      ] as const) {
        const rect = JSON.parse(
          (await pageEval(app, page, `JSON.stringify(document.getElementById('${button}').getBoundingClientRect())`)) ??
            "{}",
        );
        await main.mouse.click(box.x + rect.x + rect.width / 2, box.y + rect.y + rect.height / 2);
        await Bun.sleep(600);
        await main.keyboard.press("Enter");
        await until(
          `the ${button} dialog resolves`,
          () => pageEval(app, page, "document.getElementById('dlg').textContent"),
          (v) => v === expect,
          12000,
        );
      }
      await censusHolds(app, "javascriptDialogs");
    },
  },
  {
    name: "windowOpenBecomesAnAppTab",
    run: async () => {
      const page = await activePage();
      const tabs = countWebViews((await app.tree(mainWindow)).root as never);
      const rect = JSON.parse(
        (await pageEval(app, page, "JSON.stringify(document.getElementById('blank').getBoundingClientRect())")) ?? "{}",
      );
      const box = await viewBox(page);
      await main.mouse.click(box.x + rect.x + rect.width / 2, box.y + rect.y + rect.height / 2);
      await until(
        "target=_blank opens an app tab",
        async () => countWebViews((await app.tree(mainWindow)).root as never),
        (n) => n > tabs,
        15000,
      );
      await censusHolds(app, "targetBlank");
      await until("still one anchor on show", anchors, (a) => a.length === 1, 10000);
      await main.keyboard.press("Meta+w");
    },
  },
  {
    name: "download",
    run: async () => {
      const page = await activePage();
      const rect = JSON.parse(
        (await pageEval(app, page, "JSON.stringify(document.getElementById('dl').getBoundingClientRect())")) ?? "{}",
      );
      const box = await viewBox(page);
      await main.mouse.click(box.x + rect.x + rect.width / 2, box.y + rect.y + rect.height / 2);
      await main.getByTestId("downloads-button").click();
      await until(
        "the download reaches the app's own list",
        async () => {
          const tree = await app.tree(mainWindow);
          let seen = false;
          const walk = (node: { testID: string | null; children: unknown[] }) => {
            if (node.testID?.startsWith("downloads-item-")) seen = true;
            for (const child of node.children) walk(child as never);
          };
          walk(tree.root as never);
          return seen;
        },
        (ok) => ok,
        20000,
      );
      await main.keyboard.press("Escape");
    },
  },
  {
    name: "htmlFullscreenVideo",
    run: async () => {
      const page = await activePage();
      const rect = JSON.parse(
        (await pageEval(app, page, "JSON.stringify(document.getElementById('fs').getBoundingClientRect())")) ?? "{}",
      );
      const box = await viewBox(page);
      await main.mouse.click(box.x + rect.x + rect.width / 2, box.y + rect.y + rect.height / 2);
      await until(
        "the element goes fullscreen",
        () => pageEval(app, page, "document.documentElement.dataset.ndFullscreen ?? ''"),
        (v) => v === "on",
        12000,
      );
      await censusHolds(app, "htmlFullscreenOn");
      await main.keyboard.press("Escape");
      await until(
        "the element leaves fullscreen",
        () => pageEval(app, page, "document.documentElement.dataset.ndFullscreen ?? ''"),
        (v) => v === "off",
        12000,
      );
      await viewportMatchesView(page, "after html fullscreen");
      await censusHolds(app, "htmlFullscreenOff");
    },
  },
  {
    name: "devToolsDockResizeClose",
    run: async () => {
      if (skipDevTools) return;
      const page = await activePage();
      const before = await pageNumber(app, page, "innerWidth");
      await openInspector(page);
      const docked = await until(
        "the dock takes a share of the viewport",
        () => pageNumber(app, page, "innerWidth"),
        (w) => w < before - 40,
        25000,
      );
      void docked;
      await app.setWindowSize(1140, 860);
      // The dock is a split INSIDE the webview rectangle, so what the page gets
      // back is the view's own width, not the window's: the app's sidebar is
      // between the two.
      const full = (await until(
        "the view follows the window",
        async () => (await viewBox(page)).width,
        (w) => w > 200,
        15000,
      )) as number;
      await until(
        "the dock survives a resize",
        () => pageNumber(app, page, "innerWidth"),
        (w) => w > 0 && w < full - 40,
        20000,
      );
      await pointerSweep(page, "with the dock open", 2);
      await closeInspector(page);
      await until(
        "closing the dock gives the viewport back",
        () => pageNumber(app, page, "innerWidth"),
        (w) => Math.abs(w - full) <= 6,
        20000,
      );
      await openInspector(page);
      await until(
        "the dock reopens",
        () => pageNumber(app, page, "innerWidth"),
        (w) => w < full - 40,
        25000,
      );
      await closeInspector(page);
      await until(
        "the dock closes again",
        () => pageNumber(app, page, "innerWidth"),
        (w) => Math.abs(w - full) <= 6,
        20000,
      );
      await pointerSweep(page, "after the dock closed", 2);
      await censusHolds(app, "devTools");
    },
  },
  {
    name: "dockTiling",
    run: async () => {
      if (skipDevTools) return;
      // Three tabs and the app's sidebar: the page is not at the window origin
      // and it is one of several live webviews, which is the shape the probe
      // gate's single static view does not have.
      //
      // Brought forward first: this machine has other windows, and the address
      // bar only takes what it is typed while the app is the active one.
      activateApp();
      await Bun.sleep(600);
      await loadFixture();
      await newTab();
      await loadFixture();
      await newTab();
      const page = await loadFixture();
      const full = (await viewBox(page)).width;
      // The address field still holds the keyboard from loadFixture, and the
      // app commits what it holds, as https, when it loses focus. The inspector
      // takes focus as it docks, and the page would renavigate to a scheme the
      // fixture does not serve. Escape hands the field back its URL first.
      await main.keyboard.press("Escape");
      await Bun.sleep(300);
      await main.getByTestId(page).focus();
      await Bun.sleep(500);
      await openInspector(page);
      await until(
        "the dock takes a share of the viewport",
        () => pageNumber(app, page, "innerWidth"),
        (w) => w > 0 && w < full - 40,
        25000,
      ).catch(() => null);
      await dockTiles(page, "open");
      capture("dock-open", (await appWindowRect()).number);

      // Device mode: the hole stops being a column and becomes the device's
      // rectangle, which is what puts the phone in the page area.
      const phone = await frontendPoint(page, DEVICE_TOOLBAR);
      assert(phone !== null, "the frontend has no device-toolbar toggle");
      await frontendPresses();
      await pagePresses(page);
      await app.cursor.click(phone!);
      await Bun.sleep(2000);
      // In device mode the hole is the device's rectangle, inset from the
      // column on every side; a hole still at the column's origin is a toggle
      // that never turned on.
      const device = (await frontendGeometry()).bounds;
      assert(
        device !== null && device.y > 0,
        `device mode is not on: the hole is ${JSON.stringify(device)}; ` + await pressReport(page, phone!),
      );
      await dockTiles(page, "deviceMode", true);
      capture("dock-device", (await appWindowRect()).number);
      await app.cursor.click(phone!);
      await Bun.sleep(2000);
      await dockTiles(page, "deviceModeOff");

      // The user drags the frontend's own splitter: the hole moves and the
      // page has to move with it.
      // A capture can bring a system window forward (the Screen Recording
      // prompt, its notification), and a real drag goes to whatever is on top.
      activateApp();
      await Bun.sleep(600);
      const column = (await frontendGeometry()).bounds;
      const splitter = await frontendPoint(page, FRONTEND_SPLITTER);
      assert(splitter !== null, "the frontend has no splitter to drag");
      await frontendPresses();
      await pagePresses(page);
      await app.cursor.drag(splitter!, { x: splitter!.x - 120, y: splitter!.y });
      await Bun.sleep(1200);
      const dragged = (await frontendGeometry()).bounds;
      assert(
        dragged !== null && column !== null && dragged.width < column.width - 60,
        `the splitter drag left the hole at ${dragged?.width} of ${column?.width}; `
          + await pressReport(page, splitter!),
      );
      await dockTiles(page, "splitterDragged");

      await app.setWindowSize(1180, 880);
      await Bun.sleep(1500);
      await dockTiles(page, "resized");

      // Device mode again at the narrowest width the app's window takes, with
      // the toggle found afresh: the frontend's toolbar reflows as it narrows.
      await app.setWindowSize(1000, 720);
      await Bun.sleep(1500);
      await dockTiles(page, "narrow");
      const narrowPhone = await frontendPoint(page, DEVICE_TOOLBAR);
      assert(narrowPhone !== null, "the narrow frontend has no device-toolbar toggle");
      await app.cursor.click(narrowPhone!);
      await Bun.sleep(2000);
      const narrowDevice = (await frontendGeometry()).bounds;
      assert(
        narrowDevice !== null && narrowDevice.y > 0,
        `device mode is not on at the narrow width: the hole is ${JSON.stringify(narrowDevice)}; `
          + await pressReport(page, narrowPhone!),
      );
      await dockTiles(page, "narrowDeviceMode", true);
      capture("dock-device-narrow", (await appWindowRect()).number);
      await app.cursor.click(narrowPhone!);
      await Bun.sleep(2000);
      await dockTiles(page, "narrowDeviceModeOff");

      // Back to the first tab and forward again: every other webview in the
      // overlay is hidden rather than gone, and the one that comes back has to
      // be laid out against the inspector that is still docked.
      // The tab the inspector belongs to goes off screen and comes back. What
      // the other tab holds does not matter, so this reads the id defensively:
      // a tab with no page of its own answers activePage with nothing.
      const shown = async () => await activePage().catch(() => "");
      await main.getByTestId("menu-prev-tab").click();
      await until("the inspected tab goes off screen", shown, (id) => id !== page, 15000);
      await main.getByTestId("menu-next-tab").click();
      await until("the inspected tab is back", shown, (id) => id === page, 15000);
      await dockTiles(page, "tabSwitch");

      // Closed from the frontend's own button: the app's Inspect item is
      // Chromium's, which opens an inspector rather than toggling one.
      await closeFrontend(page);
      const back = (await viewBox(page)).width;
      await until(
        "the page takes the view back",
        () => pageNumber(app, page, "innerWidth"),
        (w) => Math.abs(w - back) <= 2,
        20000,
      );
      await openInspector(page);
      await dockTiles(page, "reopened");
      await closeFrontend(page);
      await censusHolds(app, "dockTiling");
    },
  },
  {
    name: "navigateWithDevToolsDocked",
    run: async () => {
      if (skipDevTools) return;
      // An inspector left open by a leg that failed before its close.
      if ((await targets(DEBUG_PORT)).some((t) => t.url.startsWith("devtools://"))) {
        await closeFrontend(await activePage());
      }
      // The inspected page navigates while the inspector is docked: same
      // origin, then another origin, which is a new site instance for the
      // inspected contents.
      activateApp();
      await Bun.sleep(600);
      const page = await loadFixture();
      const before = await pageNumber(app, page, "innerWidth");
      // The field commits what it holds, as https, when the docking inspector
      // takes focus; Escape hands it back its URL first.
      await main.keyboard.press("Escape");
      await Bun.sleep(300);
      await main.getByTestId(page).focus();
      await Bun.sleep(500);
      await openInspector(page);
      await until(
        "the dock takes a share of the viewport",
        () => pageNumber(app, page, "innerWidth"),
        (w) => w > 0 && w < before - 40,
        25000,
      );
      // From the page itself rather than the address bar: the app's field
      // commits what it holds again on blur, which reloads the old address
      // over the new one. Either way it is a load of the inspected contents,
      // which is what used to trap.
      const other = FIXTURE_ORIGIN.replace("127.0.0.1", "localhost");
      for (const target of [`${FIXTURE_ORIGIN}/page2.html`, `${other}/index.html`]) {
        await pageEval(app, page, `location.assign(${JSON.stringify(target)})`).catch(() => null);
        await until(
          `${target} loads`,
          async () => `${await pageEval(app, page, "location.href")}|${await pageEval(app, page, "document.readyState")}`,
          (v) => v.startsWith(target) && v.endsWith("|complete"),
          15000,
        );
      }
      await main.keyboard.press("Escape");
      await Bun.sleep(300);
      await closeFrontend(page);
      await censusHolds(app, "navigateWithDevToolsDocked");
    },
  },
];

function countWebViews(node: { type: string; children: unknown[] }): number {
  let count = node.type === "WebView" ? 1 : 0;
  for (const child of node.children) count += countWebViews(child as never);
  return count;
}

/// The host's own trace is the only place a relift is reported, so the storm
/// check reads its log.
function countTrace(marker: string): number {
  const path = process.env.ND_APP_HOST_LOG;
  if (!path) return 0;
  const file = Bun.spawnSync(["rg", "-c", marker, path]);
  return Number(file.stdout.toString().trim()) || 0;
}

// MARK: - runner

// The quit legs live in the shell gate, which needs a host sitting on a loaded
// page (and, for two of the four, on a docked inspector) before it signals it.
const prep = process.env.ND_APP_CHROME_PREP ?? "";
if (prep) {
  // A quit leg's host reopens the session the legs left, so the fixture is
  // usually on show already. Typing its address again would be the app's own
  // "same address reloads" path, which its field then races on blur.
  const restored = await until(
    "the restored tab shows a fixture page",
    async () => {
      const id = await activePage().catch(() => "");
      const at = id ? await pageEval(app, id, "location.port+'|'+document.readyState").catch(() => null) : null;
      return at === `${FIXTURE_PORT}|complete` ? id : "";
    },
    (id) => id !== "",
    15000,
  ).catch(() => "");
  const page = restored || (await loadFixture());
  if (prep === "devtools") {
    const before = await pageNumber(app, page, "innerWidth");
    await openInspector(page);
    await until("the dock opens", () => pageNumber(app, page, "innerWidth"), (w) => w < before - 40, 25000);
  }
  fixtures.stop();
  console.log(`ND_APP_CHROME_PREPARED ${prep}`);
  process.exit(0);
}

const only = (process.env.ND_APP_CHROME_LEGS ?? "").split(",").filter((name) => name.length > 0);
const failures: string[] = [];
const skipped: string[] = [];
let passed = 0;

/// Each leg starts on a loaded fixture page in the app's first window. A leg
/// that failed halfway can leave the app on its new-tab page or with a second
/// window on top, and without this every later leg reports that instead of its
/// own result.
async function ensureFixturePage(): Promise<void> {
  // The app's cmd+W is one menu item acting on the main window's active tab,
  // so a second window closes through its own close button.
  for (let attempt = 0; attempt < 3 && (await app.windows()).windows.length > 1; attempt++) {
    const before = (await app.windows()).windows.length;
    try {
      osaWindow('perform action "AXPress" of (first button whose subrole is "AXCloseButton")');
    } catch {
      break;
    }
    const closed = await until(
      "the extra window closes",
      async () => (await app.windows()).windows.length,
      (n) => n < before,
      8000,
    ).catch(() => null);
    if (closed === null) break;
  }
  // The fixture page every leg is written against, not merely a page from the
  // fixture server: a leg that inherited page2.html from the leg before it
  // reports a missing element rather than its own result.
  const page = await activePage().catch(() => null);
  if (page !== null && (await pageEval(app, page, "location.href").catch(() => null)) === `${FIXTURE_ORIGIN}/index.html`) {
    // The page carries state no reload cleared, and Chromium's own menu is
    // built from it: a selection left by an earlier leg turns the page menu
    // into the selection menu, and a leg asserting Back reports that instead.
    await pageEval(
      app,
      page,
      "getSelection().removeAllRanges(); scrollTo(0, 0);" +
        " document.activeElement?.blur?.(); document.getElementById('text').value = '';" +
        " document.getElementById('edit').textContent = ''",
    ).catch(() => null);
    return;
  }
  mainPage = await loadFixture();
}

for (const leg of legs) {
  if (only.length > 0 && !only.includes(leg.name)) continue;
  const started = Date.now();
  // The precondition is best effort: a leg must fail on its own assertion, not
  // on the tidy-up the runner did before it.
  await ensureFixturePage().catch((error) => console.log(`ND_APP_CHROME_PREPARE ${leg.name}: ${String(error)}`));
  try {
    await leg.run(app);
    await censusHolds(app, leg.name);
    passed++;
    console.log(`ND_APP_CHROME_LEG ${leg.name}: ok (${Date.now() - started}ms)`);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (error instanceof LegSkipped) {
      skipped.push(`${leg.name}: ${message}`);
      console.log(`ND_APP_CHROME_LEG ${leg.name}: skip (${message})`);
      continue;
    }
    failures.push(`${leg.name}: ${message}`);
    console.log(`ND_APP_CHROME_LEG ${leg.name}: FAIL ${message}`);
  }
}

fixtures.stop();
console.log(`ND_APP_CHROME_LEGS ${passed} passed, ${failures.length} failed, ${skipped.length} skipped`);
if (failures.length > 0) {
  for (const failure of failures) console.error(`ND_APP_CHROME_FAIL ${failure}`);
  process.exit(1);
}
console.log("ND_APP_CHROME_DRIVE_OK");
process.exit(0);
