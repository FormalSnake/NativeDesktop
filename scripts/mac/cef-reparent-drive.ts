#!/usr/bin/env bun
// Drives examples/webview-probe/cef-reparent.tsx: one live `<webview>` moved
// between two host windows with `moveNode`, which is what a tab dragged to
// another window has to do. The page must survive the move untouched, so every
// assertion is about continuity: the same CDP target, the same JS counter, the
// same scroll offset, no navigation, and a view that still paints, takes input
// and anchors Chromium's popups in the window it landed in.
import { connectApp } from "@nativedesktop/test";

import {
  anchors,
  assert,
  capture,
  censusHolds,
  colourDistance,
  hostAlive,
  pageEval,
  pageNumber,
  probePng,
  surfaces,
  until,
  type CensusWindow,
} from "./app-chrome-lib.ts";

const FILL: [number, number, number] = [0, 96, 208];
/// Alloy has no anchor window: CEF's own NSView is a child of the NDCefWebView
/// and travels with it, so the anchor assertions are Chrome style's alone.
const chromeStyle = (process.env.ND_CEF_STYLE ?? "alloy") === "chrome";
const app = await connectApp();
await until("the probe's windows", async () => (await app.windows()).windows.length, (n) => n === 2, 45000);

const failures: string[] = [];

/// Every leg names the slot it has to start in, and the runner puts the probe
/// there before running it: no inspector docked, the view in that slot. A leg
/// that fails halfway then costs one leg instead of every leg after it.
async function leg(name: string, startSlot: "a" | "b", run: () => Promise<void>): Promise<void> {
  try {
    await prepare(startSlot);
  } catch (error) {
    failures.push(`${name}: could not reach its starting state: ${String(error)}`);
    console.log(`ND_CEF_REPARENT_LEG ${name}: FAIL ${failures[failures.length - 1]}`);
    return;
  }
  try {
    await run();
    await censusHolds(app, name);
    console.log(`ND_CEF_REPARENT_LEG ${name}: ok`);
  } catch (error) {
    failures.push(`${name}: ${error instanceof Error ? error.message : String(error)}`);
    console.log(`ND_CEF_REPARENT_LEG ${name}: FAIL ${failures[failures.length - 1]}`);
  }
}

async function prepare(slot: "a" | "b"): Promise<void> {
  const windows = (await app.windows()).windows;
  if (windows.length === 0) throw new Error("the probe has no windows left");
  const a = await app.window(0);
  if (await inspectorIsDocked()) {
    await a.getByTestId("r-devtools").click();
    await until("the inspector closes", inspectorIsDocked, (open) => !open, 15000);
  }
  if (slot === "b" && windows.length < 2) throw new Error("window B is gone");
  if ((await slotOfView()) === slot) return;
  await a.getByTestId(slot === "a" ? "r-to-a" : "r-to-b").click();
  await until(`the view moves to slot ${slot}`, slotOfView, (where) => where === slot, 15000);
}

/// The page's own viewport against the view it sits in: a docked inspector is
/// the only thing that makes the two differ.
async function inspectorIsDocked(): Promise<boolean> {
  const slot = await slotOfView();
  if (slot === "none") return false;
  const windows = (await app.windows()).windows;
  const window = slot === "a" ? windows[0]?.ref : windows[1]?.ref;
  if (window === undefined) return false;
  const { box } = await viewBox(window);
  const width = await pageNumber(app, "r-view", "innerWidth");
  return width < box.width - 40;
}

const debugPort = process.env.ND_CEF_DEBUG_PORT ?? "9337";
async function pageTargetId(): Promise<string> {
  const response = await fetch(`http://127.0.0.1:${debugPort}/json/list`);
  const targets = (await response.json()) as { id: string; type: string; url: string }[];
  const page = targets.find((t) => t.type === "page" && t.url.startsWith("http://127.0.0.1:"));
  assert(page !== undefined, `no page target among ${targets.map((t) => t.type).join(",")}`);
  return page!.id;
}

/// Which slot the webview's node is under, read from the tree rather than from
/// what the app said it did. `getTree` answers with the whole app tree
/// whichever window it is asked about, so the slot is the discriminator, not
/// the window ref.
async function slotOfView(): Promise<string> {
  const tree = await app.tree();
  let slot = "none";
  const walk = (node: { testID: string | null; children: unknown[] }, under: string) => {
    const here = node.testID === "r-slot-a" ? "a" : node.testID === "r-slot-b" ? "b" : under;
    if (node.testID === "r-view") slot = here;
    for (const child of node.children) walk(child as never, here);
  };
  walk(tree.root as never, "none");
  return slot;
}

async function viewBox(window: number) {
  const factory = await app.window((await app.windows()).windows.findIndex((w) => w.ref === window));
  const box = await factory.getByTestId("r-view").boundingBox();
  assert(box !== null, "r-view has no bounding box");
  return { box: box!, factory };
}

async function windowRect(window: number): Promise<CensusWindow> {
  const info = (await app.windows()).windows.find((w) => w.ref === window)!;
  const frames = info.geometry!;
  return { number: 0, layer: 0, alpha: 1, title: "", x: frames.x, y: frames.y, width: frames.w, height: frames.h };
}

/// The anchor has to sit on the webview's rectangle in the window it is in now.
async function anchorTracks(window: number, what: string): Promise<void> {
  if (!chromeStyle) return;
  const [{ box }, rect] = await Promise.all([viewBox(window), windowRect(window)]);
  await until(
    `${what}: the anchor re-glues to the host window`,
    async () => (await anchors()).some(
      (a) =>
        Math.abs(a.x - (rect.x + box.x)) <= 2 && Math.abs(a.y - (rect.y + box.y)) <= 2 &&
        Math.abs(a.width - box.width) <= 2 && Math.abs(a.height - box.height) <= 2,
    ),
    (ok) => ok,
    12000,
  );
}

async function viewportMatchesView(window: number, what: string): Promise<void> {
  const { box } = await viewBox(window);
  await until(
    `${what}: the page viewport follows the view`,
    () => pageEval(app, "r-view", "innerWidth+'x'+innerHeight"),
    (value) => {
      const [w, h] = String(value).split("x").map(Number);
      return Math.abs(w - box.width) <= 2 && Math.abs(h - box.height) <= 2;
    },
    15000,
  );
}

let windowA = 0;
let windowB = 0;
let target = "";
let counter = 0;
let scroll = 0;

await leg("placedInWindowA", "a", async () => {
  await until("the page loads", () => pageEval(app, "r-view", "document.readyState"), (v) => v === "complete", 30000);
  assert((await slotOfView()) === "a", "the view did not start in window A's slot");
  const windows = (await app.windows()).windows;
  windowA = windows[0]!.ref;
  windowB = windows[1]!.ref;
  await viewportMatchesView(windowA, "in A");
  await anchorTracks(windowA, "in A");
  target = await pageTargetId();
});

await leg("movedToWindowB", "a", async () => {
  await pageEval(app, "r-view", "scrollTo(0, 900)");
  await until("the page scrolls", () => pageNumber(app, "r-view", "Math.round(scrollY)"), (y) => y > 800, 8000);
  counter = await pageNumber(app, "r-view", "window.__ndCounter");
  scroll = await pageNumber(app, "r-view", "Math.round(scrollY)");
  // pageshow fires for the first load too, so the baseline is what it reads
  // now, not zero.
  const navigations = await pageNumber(app, "r-view", "window.__ndNavigations");

  const a = await app.window((await app.windows()).windows.findIndex((w) => w.ref === windowA));
  await a.getByTestId("r-to-b").click();
  await until("the view lands in window B's slot", slotOfView, (slot) => slot === "b", 15000);

  assert(hostAlive(), "the host died during the move");
  assert((await pageTargetId()) === target, "the move created a new browser: the CDP target id changed");
  assert(
    (await pageNumber(app, "r-view", "window.__ndNavigations")) === navigations,
    "the page navigated during the move",
  );
  const moved = await pageNumber(app, "r-view", "window.__ndCounter");
  assert(moved >= counter, `the page restarted: counter went ${counter} -> ${moved}`);
  assert(
    Math.abs((await pageNumber(app, "r-view", "Math.round(scrollY)")) - scroll) <= 2,
    "the scroll offset was lost in the move",
  );
});

await leg("paintsInWindowB", "b", async () => {
  await viewportMatchesView(windowB, "in B");
  await anchorTracks(windowB, "in B");
  const frames = await pageNumber(app, "r-view", "window.__ndFrames");
  await until(
    "the page keeps painting in its new window",
    () => pageNumber(app, "r-view", "window.__ndFrames"),
    (n) => n > frames,
    10000,
  );
  const rect = await windowRect(windowB);
  const { box } = await viewBox(windowB);
  const shot = capture("reparent-b", (await onScreen(rect)).number);
  const report = probePng(shot);
  const scale = report.width / rect.width;
  const inside = probePng(shot, {
    x: Math.round(box.x * scale),
    y: Math.round((box.y + box.height * 0.4) * scale),
    w: Math.round(box.width * scale),
    h: Math.round(box.height * 0.5 * scale),
  });
  assert(
    colourDistance(inside.mean, FILL) < 90,
    `window B does not show the page after the move: mean ${JSON.stringify(inside.mean)}`,
  );
});

await leg("windowAIsEmptyAfterTheMove", "b", async () => {
  const rect = await windowRect(windowA);
  const shot = capture("reparent-a-empty", (await onScreen(rect)).number);
  const report = probePng(shot);
  assert(
    colourDistance(report.mean, FILL) > 90,
    `window A still shows the page it gave away: mean ${JSON.stringify(report.mean)}`,
  );
});

await leg("windowBResizeAndMove", "b", async () => {
  await app.setWindowFrame({ window: windowB, width: 900, height: 640 });
  await viewportMatchesView(windowB, "B resized");
  await anchorTracks(windowB, "B resized");
  const before = await windowRect(windowB);
  await app.setWindowFrame({ window: windowB, x: before.x + 40, y: before.y + 30 });
  await anchorTracks(windowB, "B moved");
});

await leg("inputReachesThePageInWindowB", "b", async () => {
  const { box, factory } = await viewBox(windowB);
  await pageEval(app, "r-view", "scrollTo(0,0); document.getElementById('text').value=''");
  const field = JSON.parse(
    (await pageEval(app, "r-view", "JSON.stringify(document.getElementById('text').getBoundingClientRect())")) ?? "{}",
  );
  await factory.mouse.click(box.x + field.x + 20, box.y + field.y + field.height / 2);
  await until(
    "the page in window B takes focus",
    () => pageEval(app, "r-view", "String(document.hasFocus())"),
    (v) => v === "true",
    10000,
  );
  await factory.keyboard.type("abc");
  await until(
    "the page in window B takes the keystrokes",
    () => pageEval(app, "r-view", "document.getElementById('text').value"),
    (v) => v === "abc",
    10000,
  );
});

await leg("chromiumSurfacesOpenInWindowB", "b", async () => {
  const { box, factory } = await viewBox(windowB);
  const select = JSON.parse(
    (await pageEval(app, "r-view", "JSON.stringify(document.getElementById('sel').getBoundingClientRect())")) ?? "{}",
  );
  const rect = await windowRect(windowB);
  await factory.mouse.click(box.x + select.x + select.width / 2, box.y + select.y + select.height / 2);
  const popup = await until(
    "the select popup opens in the new window",
    async () => (await surfaces(app)).filter((w) => w.alpha > 0),
    (windows) => windows.length > 0,
    10000,
  );
  const wanted = { x: rect.x + box.x + select.x, y: rect.y + box.y + select.y };
  assert(
    popup.some((w) => Math.abs(w.x - wanted.x) < 260 && Math.abs(w.y - wanted.y) < 300),
    `the popup opened at ${popup.map((w) => `${w.x},${w.y}`).join(" ")} rather than near ${wanted.x},${wanted.y}`,
  );
  await factory.keyboard.press("Escape");
});

await leg("dockedDevToolsSurvivesTheMove", "b", async () => {
  // Chrome style docks the inspector as a second BrowserView inside the same
  // Views window, so the move has to take it along with the page. Alloy gives
  // it a window of its own and has nothing to carry.
  if (!chromeStyle || process.env.ND_CEF_REPARENT_DEVTOOLS !== "1") return;
  const wide = await pageNumber(app, "r-view", "innerWidth");
  const a = await app.window((await app.windows()).windows.findIndex((w) => w.ref === windowA));
  await a.getByTestId("r-devtools").click();
  await until(
    "the dock takes a share of the viewport",
    () => pageNumber(app, "r-view", "innerWidth"),
    (w) => w < wide - 40,
    25000,
  );
  await a.getByTestId("r-to-a").click();
  await until("the view lands back in window A's slot", slotOfView, (slot) => slot === "a", 15000);
  const inA = (await viewBox(windowA)).box.width;
  await until(
    "the dock is still taking its share in the new window",
    () => pageNumber(app, "r-view", "innerWidth"),
    (w) => w > 0 && w < inA - 40,
    20000,
  );
  assert((await pageTargetId()) === target, "the move with the dock open created a new browser");
  await a.getByTestId("r-devtools").click();
  await until(
    "the dock closes and the page gets its width back",
    () => pageNumber(app, "r-view", "innerWidth"),
    (w) => w > inA - 40,
    20000,
  );
  await a.getByTestId("r-devtools").click();
  await until(
    "the dock reopens after being closed",
    () => pageNumber(app, "r-view", "innerWidth"),
    (w) => w < inA - 40,
    20000,
  );
  await a.getByTestId("r-devtools").click();
  await until(
    "the dock closes again",
    () => pageNumber(app, "r-view", "innerWidth"),
    (w) => w > inA - 40,
    20000,
  );
  await anchorTracks(windowA, "after the dock closed");
});

await leg("movedBackToWindowA", "b", async () => {
  const b = await app.window((await app.windows()).windows.findIndex((w) => w.ref === windowB));
  void b;
  const a = await app.window((await app.windows()).windows.findIndex((w) => w.ref === windowA));
  await a.getByTestId("r-to-a").click();
  await until("the view lands back in window A's slot", slotOfView, (slot) => slot === "a", 15000);
  assert((await pageTargetId()) === target, "moving back created a new browser");
  await viewportMatchesView(windowA, "back in A");
  await anchorTracks(windowA, "back in A");
});

await leg("closingTheOtherWindowLeavesTheBrowser", "a", async () => {
  const a = await app.window((await app.windows()).windows.findIndex((w) => w.ref === windowA));
  await a.getByTestId("r-close-b").click();
  await until("window B goes", async () => (await app.windows()).windows.length, (n) => n === 1, 15000);
  assert(hostAlive(), "closing the window the view had left took the host with it");
  assert((await pageTargetId()) === target, "closing the other window closed the browser");
  await viewportMatchesView(windowA, "after B closed");
  await anchorTracks(windowA, "after B closed");
});

/// The window server's record for a window the automation tree reported, so a
/// capture can name it.
async function onScreen(rect: CensusWindow): Promise<CensusWindow> {
  const { census } = await import("./app-chrome-lib.ts");
  const match = (await census()).find(
    (w) => w.alpha > 0 && Math.abs(w.x - rect.x) <= 2 && Math.abs(w.y - rect.y) <= 2,
  );
  assert(match !== undefined, `no on-screen window at ${rect.x},${rect.y}`);
  return match!;
}

if (failures.length > 0) {
  for (const failure of failures) console.error(`ND_CEF_REPARENT_FAIL ${failure}`);
  process.exit(1);
}
console.log("ND_CEF_REPARENT_OK");
process.exit(0);
