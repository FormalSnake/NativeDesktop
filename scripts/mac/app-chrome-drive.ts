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
  FIXTURE_FILL,
  FIXTURE_ORIGIN,
  FIXTURE_PORT,
  HOST_PID,
  LegFailure,
  SHOTS,
  anchors,
  appWindowRect,
  assert,
  capture,
  census,
  hostAlive,
  surfaces,
  censusHolds,
  colourDistance,
  pageEval,
  pageNumber,
  probePng,
  startFixtureServer,
  until,
  type Leg,
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

/// cmd+T, waited out. The app mounts the new tab a commit later and the
/// address bar aims at whichever tab is active when it opens, so a cmd+L that
/// does not wait re-navigates the tab that was already there.
async function newTab(): Promise<void> {
  const before = (await activePage().catch(() => null)) ?? "";
  await main.keyboard.press("Meta+t");
  await until(
    "the new tab becomes the active one",
    async () => (await activePage().catch(() => null)) ?? "",
    (id) => id !== before,
    8000,
  ).catch(() => undefined);
}

/// The webview's rectangle inside a window capture, in image pixels.
async function captureViewRect(name: string, testId: string) {
  const [box, win] = await Promise.all([viewBox(testId), appWindowRect()]);
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
    // that a stale band would show up against starts below it.
    fill: {
      x: Math.round(box.x * scale),
      y: Math.round((box.y + box.height * 0.25) * scale),
      w: Math.round(box.width * scale),
      h: Math.round(box.height * 0.75 * scale),
    },
  };
}


/// The app puts Inspect Element last in the page context menu, and Chromium's
/// own accelerators for the inspector are refused by the command handler, so
/// this is the only way in. The menu is Chromium's, drawn by Views: nothing in
/// the automation tree can read it, so it is driven by arrow keys, with "up
/// from the top" landing on the last item whatever Chromium put above it.
async function openInspector(page: string): Promise<void> {
  const box = await viewBox(page);
  await main.mouse.click(box.x + box.width / 2, box.y + box.height - 60, { button: "right" });
  await until(
    "the context menu opens",
    async () => (await surfaces(app)).filter((w) => w.alpha > 0 && w.height > 60).length,
    (n) => n > 0,
    12000,
  );
  osaKeys("key code 126");
  osaKeys("key code 36");
}

async function closeInspector(page: string): Promise<void> {
  const box = await viewBox(page);
  // The inspector's own close is inside its front-end; F12 and cmd+alt+I are
  // refused by the Chrome command handler, so it goes out the way it came in.
  await main.mouse.click(box.x + 40, box.y + box.height - 60, { button: "right" });
  await until(
    "the context menu opens again",
    async () => (await surfaces(app)).filter((w) => w.alpha > 0 && w.height > 60).length,
    (n) => n > 0,
    12000,
  );
  osaKeys("key code 126");
  osaKeys("key code 36");
}

function osaKeys(script: string): void {
  const run = Bun.spawnSync([
    "osascript", "-e",
    `tell application "System Events" to tell (first process whose unix id is ${HOST_PID}) to ${script}`,
  ]);
  if (run.exitCode !== 0) throw new LegFailure(`osascript failed: ${run.stderr.toString().trim()}`);
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
    name: "zoomButton",
    run: async () => {
      // A window with a full-size content view publishes AXZoomButton as an
      // attribute rather than as one of its three title-bar buttons; the green
      // one is the fullscreen button and only an option-click zooms it.
      const zoom = osaWindow('get value of attribute "AXZoomButton"');
      assert(zoom.length > 0 && zoom !== "missing value", `the window publishes no zoom button (${zoom})`);
      const before = await appWindowRect();
      osaWindow('perform action "AXPress" of (value of attribute "AXZoomButton")');
      await until(
        "the zoom button changes the frame",
        async () => (await appWindowRect()).height,
        (h) => Math.abs(h - before.height) > 8,
        10000,
      );
      await viewportMatchesView(mainPage, "zoomed");
      osaWindow('perform action "AXPress" of (value of attribute "AXZoomButton")');
      await until(
        "the zoom button restores the frame",
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
      assert(new Set(ids).size === 5, `expected five tabs, the app has ${new Set(ids).size} (${ids.join(",")})`);
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
      const windows = (await app.windows()).windows;
      assert(windows.length >= 2, `expected the private window to still be open, saw ${windows.length}`);
      const second = await app.window(1);
      await second.keyboard.press("Meta+w");
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
      const tabs = countWebViews((await app.tree(mainWindow)).root as never);
      await main.keyboard.press("Meta+t");
      await until(
        "cmd+T opens an app tab",
        async () => countWebViews((await app.tree(mainWindow)).root as never) >= tabs,
        (ok) => ok,
        10000,
      );
      await main.keyboard.press("Meta+w");
      await until(
        "cmd+W closes it again",
        async () => countWebViews((await app.tree(mainWindow)).root as never),
        (n) => n === tabs,
        10000,
      );
      const page = await activePage();
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
      const field = JSON.parse(
        (await pageEval(app, page, "JSON.stringify(document.getElementById('text').getBoundingClientRect())")) ?? "{}",
      );
      const box = await viewBox(page);
      await pageEval(app, page, "document.getElementById('text').value=''");
      await main.mouse.click(box.x + field.x + 20, box.y + field.y + field.height / 2);
      await main.keyboard.type("copyme");
      await main.keyboard.press("Meta+a");
      await main.keyboard.press("Meta+c");
      await main.keyboard.press("Meta+v");
      await main.keyboard.press("Meta+v");
      await until(
        "cmd+A/C/V reach the page",
        () => pageEval(app, page, "document.getElementById('text').value"),
        (v) => v === "copymecopyme",
        10000,
      );
      await main.keyboard.press("Meta+z");
      await until(
        "cmd+Z undoes in the page",
        () => pageEval(app, page, "document.getElementById('text').value"),
        (v) => v !== "copymecopyme",
        10000,
      );
    },
  },
  {
    name: "tabKeyIntoAndOutOfThePage",
    run: async () => {
      const page = await activePage();
      await main.keyboard.press("Meta+l");
      await Bun.sleep(500);
      await until(
        "focus leaves the web contents for the app's own field",
        () => pageEval(app, page, "String(document.hasFocus())"),
        (v) => v === "false",
        10000,
      );
      await main.keyboard.press("Escape");
      await main.keyboard.press("Tab");
      await main.keyboard.press("Tab");
      await until(
        "tab out of the native chrome reaches the page",
        () => pageEval(app, page, "String(document.hasFocus())"),
        (v) => v === "true",
        10000,
      );
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
      const tip = JSON.parse(
        (await pageEval(app, page, "JSON.stringify(document.getElementById('tip').getBoundingClientRect())")) ?? "{}",
      );
      const box = await viewBox(page);
      await main.mouse.move(box.x + tip.x + tip.width / 2, box.y + tip.y + tip.height / 2);
      // A tooltip is a window of Chromium's own, anchored on the widget; the
      // census is what sees it, because nothing in the app's tree knows about
      // it.
      const seen = await until(
        "a tooltip surface appears over the page",
        async () => (await surfaces(app)).filter((w) => w.alpha > 0).length,
        (n) => n > 0,
        12000,
      );
      void seen;
      await main.mouse.move(box.x + 8, box.y + 8);
      await censusHolds(app, "tooltip");
    },
  },
  {
    name: "contextMenuWithExtensionItem",
    run: async () => {
      const page = await activePage();
      await pageEval(app, page, "delete document.documentElement.dataset.ndMenu");
      await main.getByTestId(page).rightClick();
      await until(
        "a context menu surface is on screen",
        async () => (await surfaces(app)).filter((w) => w.alpha > 0 && w.height > 60).length,
        (n) => n > 0,
        12000,
      );
      const shot = capture("context-menu", (await appWindowRect()).number);
      assert(probePng(shot).width > 0, "the context menu capture is empty");
      await main.keyboard.press("Escape");
      await until(
        "the menu closes",
        async () => (await surfaces(app)).filter((w) => w.alpha > 0 && w.height > 60).length,
        (n) => n === 0,
        8000,
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
        `the page is not filling the view before the popover: mean ${JSON.stringify(clean.mean)}`,
      );
      await main.getByTestId("downloads-button").click();
      await until(
        "the downloads popover opens",
        () => main.getByTestId("downloads-panel").isVisible(),
        (v) => v === true,
        10000,
      );
      const after = await captureViewRect("overlay-after", page);
      const over = probePng(after.path, after.fill);
      assert(
        colourDistance(over.mean, clean.mean) > 12,
        `the popover left the webview rectangle unchanged (${JSON.stringify(clean.mean)} vs ${JSON.stringify(over.mean)}),` +
          " so it is not drawing over the web contents",
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
      await until(
        "the dock survives a resize",
        () => pageNumber(app, page, "innerWidth"),
        (w) => w > 0 && w < 1140 - 40,
        20000,
      );
      await pointerSweep(page, "with the dock open", 2);
      await closeInspector(page);
      await until(
        "closing the dock gives the viewport back",
        () => pageNumber(app, page, "innerWidth"),
        (w) => w > 1140 - 80,
        20000,
      );
      await openInspector(page);
      await until(
        "the dock reopens",
        () => pageNumber(app, page, "innerWidth"),
        (w) => w < 1140 - 80,
        25000,
      );
      await closeInspector(page);
      await until(
        "the dock closes again",
        () => pageNumber(app, page, "innerWidth"),
        (w) => w > 1140 - 80,
        20000,
      );
      await pointerSweep(page, "after the dock closed", 2);
      await censusHolds(app, "devTools");
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
  const page = await loadFixture();
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
let passed = 0;

/// Each leg starts on a loaded fixture page in the app's first window. A leg
/// that failed halfway can leave the app on its new-tab page or with a second
/// window on top, and without this every later leg reports that instead of its
/// own result.
async function ensureFixturePage(): Promise<void> {
  // The app's cmd+W is one menu item acting on the main window's active tab,
  // so a second window closes through its own close button.
  while ((await app.windows()).windows.length > 1) {
    const before = (await app.windows()).windows.length;
    osaWindow('perform action "AXPress" of (value of attribute "AXCloseButton")');
    const closed = await until(
      "the extra window closes",
      async () => (await app.windows()).windows.length,
      (n) => n < before,
      8000,
    ).catch(() => null);
    if (closed === null) break;
  }
  const page = await activePage().catch(() => null);
  if (page !== null && (await pageEval(app, page, "location.host").catch(() => null)) === `127.0.0.1:${FIXTURE_PORT}`) {
    return;
  }
  mainPage = await loadFixture();
}

for (const leg of legs) {
  if (only.length > 0 && !only.includes(leg.name)) continue;
  const started = Date.now();
  try {
    await ensureFixturePage();
    await leg.run(app);
    await censusHolds(app, leg.name);
    passed++;
    console.log(`ND_APP_CHROME_LEG ${leg.name}: ok (${Date.now() - started}ms)`);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    failures.push(`${leg.name}: ${message}`);
    console.log(`ND_APP_CHROME_LEG ${leg.name}: FAIL ${message}`);
  }
}

fixtures.stop();
console.log(`ND_APP_CHROME_LEGS ${passed} passed, ${failures.length} failed`);
if (failures.length > 0) {
  for (const failure of failures) console.error(`ND_APP_CHROME_FAIL ${failure}`);
  process.exit(1);
}
console.log("ND_APP_CHROME_DRIVE_OK");
process.exit(0);
