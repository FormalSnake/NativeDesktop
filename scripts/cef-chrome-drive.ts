#!/usr/bin/env bun
// scripts/cef-chrome-drive.ts: the Chrome-style half of the CEF gate, driven
// over the remote debugging port rather than the automation socket. Everything
// it checks lives inside Chromium: an extension's service worker, the browser
// paths that would put a window on screen, Chrome's own accelerators, and the
// docked inspector.
//
// Real X11 input, not CDP input: Chrome's accelerators are handled in the
// browser process from the native key event, and a debugger-injected one never
// reaches them.
import { Session, clickDevToolsClose, targets, waitForTarget } from "./cdp.ts";

const port = Number(process.env.ND_CDP_PORT ?? "9333");
const pass = process.env.ND_CHROME_PASS ?? "first";
const display = process.env.DISPLAY ?? ":96";
const shotPath = process.env.ND_CHROME_SHOT_PATH ?? "";
const token = process.env.ND_CHROME_TOKEN ?? "nd-gate-token";

const failures: string[] = [];
function check(name: string, ok: boolean, detail: string): void {
  console.log(`  ${name}: ${ok ? "ok" : "FAIL"} (${detail})`);
  if (!ok) failures.push(`${name}: ${detail}`);
}

function sh(...argv: string[]): string {
  return Bun.spawnSync(argv, { env: { ...process.env, DISPLAY: display } }).stdout.toString().trim();
}

interface Geometry {
  id: string;
  x: number;
  y: number;
  w: number;
  h: number;
  /// Position on the root, which is what a browser reports as `screenX`.
  rootX: number;
  rootY: number;
}

function geometryOf(id: string): Geometry | null {
  const out = sh("xwininfo", "-id", id);
  const w = out.match(/^\s*Width:\s+(\d+)/m);
  const h = out.match(/^\s*Height:\s+(\d+)/m);
  const x = out.match(/^\s*Relative upper-left X:\s+(-?\d+)/m);
  const y = out.match(/^\s*Relative upper-left Y:\s+(-?\d+)/m);
  const ax = out.match(/^\s*Absolute upper-left X:\s+(-?\d+)/m);
  const ay = out.match(/^\s*Absolute upper-left Y:\s+(-?\d+)/m);
  if (!w || !h || !x || !y || !ax || !ay) return null;
  return {
    id,
    x: Number(x[1]), y: Number(y[1]), w: Number(w[1]), h: Number(h[1]),
    rootX: Number(ax[1]), rootY: Number(ay[1]),
  };
}

function childrenOf(id: string): Geometry[] {
  const rows: Geometry[] = [];
  for (const line of sh("xwininfo", "-id", id, "-children").split("\n")) {
    const m = line.match(/^\s*(0x[0-9a-f]+).*?\s(\d+)x(\d+)\+(-?\d+)\+(-?\d+)\s+\+(-?\d+)\+(-?\d+)/);
    if (!m) continue;
    if (!sh("xwininfo", "-id", m[1]!).includes("Map State: IsViewable")) continue;
    rows.push({
      id: m[1]!, w: Number(m[2]), h: Number(m[3]), x: Number(m[4]), y: Number(m[5]),
      rootX: Number(m[6]), rootY: Number(m[7]),
    });
  }
  return rows;
}

/// The containers the engine embedded a `<webview>` into, newest first, from
/// the host's own trace. There is one per view in the app, and only the one
/// holding the inspector has a second mapped child.
async function embedContainers(): Promise<string[]> {
  const path = process.env.ND_HOST_LOG ?? "";
  if (!path) return [];
  const text = await Bun.file(path).text().catch(() => "");
  const ids: string[] = [];
  for (const m of text.matchAll(/ND_CEF embed node=\d+ parent=0x[0-9a-f]+ container=(0x[0-9a-f]+)/g)) {
    if (!ids.includes(m[1]!)) ids.push(m[1]!);
  }
  return ids;
}

/// The docked view's inner tiling as the X server holds it: the container the
/// engine embeds into, CEF's page window and the dock inside it, and the
/// inspector's own window inside the dock. The docked view is the one with two
/// mapped children; every other `<webview>` has only its page.
async function dockLayout(): Promise<{ view: Geometry; page: Geometry; dock: Geometry; inner: Geometry | null } | null> {
  for (const id of await embedContainers()) {
    const view = geometryOf(id);
    if (!view) continue;
    const kids = childrenOf(id);
    if (kids.length < 2) continue;
    const sorted = [...kids].sort((a, b) => a.x - b.x);
    const page = sorted[0]!;
    const dock = sorted[sorted.length - 1]!;
    return { view, page, dock, inner: childrenOf(dock.id)[0] ?? null };
  }
  return null;
}

/// The page session for one browser, found by where it sits on the root: the
/// app holds several `<webview>`s on the same origin, the unmapped ones are
/// parked off screen and keep Chromium's default 1024-wide viewport for ever,
/// and `screenX`/`screenY` is the only thing that joins a CDP target to the X
/// window the engine laid out.
async function pageAt(rootX: number, rootY: number): Promise<Session | null> {
  for (const target of await targets(port)) {
    if (target.type !== "page") continue;
    const session = await Session.open(target.webSocketDebuggerUrl!);
    const where = JSON.parse(await session.eval<string>("JSON.stringify([screenX, screenY])")) as number[];
    if (where[0] === rootX && where[1] === rootY) return session;
    session.close();
  }
  return null;
}

/// The three windows have to tile the view with no seam, and the page has to
/// be laid out at the width its window was given: a renderer still painting at
/// an older width leaves bare background between the page and the inspector.
async function checkTiling(phase: string): Promise<void> {
  const layout = await dockLayout();
  if (!layout) {
    check(`dockTiling/${phase}`, false, "no docked view: the container has no second mapped child");
    return;
  }
  const { view, page: paper, dock, inner } = layout;
  check(
    `dockTiling/${phase}`,
    paper.x === 0 && paper.x + paper.w === dock.x && dock.x + dock.w === view.w && paper.h === view.h && dock.h === view.h,
    `view ${view.w}x${view.h}, page ${paper.w}x${paper.h}+${paper.x}, dock ${dock.w}x${dock.h}+${dock.x}`,
  );
  check(
    `dockInner/${phase}`,
    inner !== null && inner.x === 0 && inner.w === dock.w && inner.h === dock.h,
    inner ? `inner ${inner.w}x${inner.h}+${inner.x} in dock ${dock.w}x${dock.h}` : "the dock has no mapped child",
  );
  const shown = await pageAt(paper.rootX, paper.rootY);
  const size = shown
    ? (JSON.parse(await shown.eval<string>("JSON.stringify([innerWidth, innerHeight])")) as number[])
    : [-1, -1];
  shown?.close();
  check(
    `dockPageViewport/${phase}`,
    size[0] === paper.w && size[1] === paper.h,
    `page lays out ${size[0]}x${size[1]} in a ${paper.w}x${paper.h} window`,
  );
}

/// The rows the open menu's keyboard walk stops on, from the host's own trace.
/// GTK skips separators and insensitive items, and the menu opens with the
/// first row focused, so a row's index is the number of Down presses to it.
async function menuRows(): Promise<string[]> {
  const path = process.env.ND_HOST_LOG ?? "";
  if (!path) return [];
  const text = await Bun.file(path).text().catch(() => "");
  const lines = text.split("\n").filter((line) => line.includes("menuShown "));
  const start = lines.findLastIndex((line) => line.includes("depth=0 index=0 "));
  const rows: string[] = [];
  for (const line of start < 0 ? lines : lines.slice(start)) {
    const m = /menuShown depth=(\d+) index=\d+ id=-?\d+ kind=(\w+) enabled=(\d) checked=\d (?:accel=\S* )?label=(.*)$/.exec(line);
    if (!m || m[1] !== "0" || m[2] === "separator" || m[3] !== "1") continue;
    rows.push(m[4]!);
  }
  return rows;
}

/// How many times the engine has reported the dock going away. The count, not
/// the presence: the same marker fires for every close, so only a fresh one
/// says the button that was just clicked is what closed it.
async function devToolsClosedMarkers(): Promise<number> {
  const path = process.env.ND_HOST_LOG ?? "";
  if (!path) return 0;
  const text = await Bun.file(path).text().catch(() => "");
  return text.split("\n").filter((line) => line.includes("ND_CEF devtoolsClosed")).length;
}

/// Windows a user could see: mapped, and at least 200x200. Chromium keeps a
/// handful of 1x1 and 10x10 utility windows on the root (clipboard owner, drag
/// proxy, the omnibox popup host) that are never presented, and counting those
/// would make the invariant unfalsifiable rather than strict.
function census(): string[] {
  const rows: string[] = [];
  for (const line of sh("xwininfo", "-root", "-children").split("\n")) {
    const m = line.match(/^\s*(0x[0-9a-f]+)\s+(".*?"|\(has no name\)).*?\s(\d+)x(\d+)\+/);
    if (!m) continue;
    const [, id, name, w, h] = m;
    if (Number(w) < 200 || Number(h) < 200) continue;
    if (!sh("xwininfo", "-id", id).includes("Map State: IsViewable")) continue;
    rows.push(`${id} ${name} ${w}x${h}`);
  }
  return rows;
}

/// The top-levels Chromium put up that are not browsers: its install prompt and
/// its post-install dialog, which the engine moves over the view. They sit
/// under the 200x200 floor `census` uses, and they carry no window name until
/// long after they are mapped, so they are found by size alone.
function chromeDialogs(): number[][] {
  const out: number[][] = [];
  for (const line of sh("xwininfo", "-root", "-children").split("\n")) {
    if (line.includes("ND CEF Probe")) continue;
    const m = line.match(/^\s*0x[0-9a-f]+.*?\s(\d+)x(\d+)\+(-?\d+)\+(-?\d+)/);
    if (!m) continue;
    const [, w, h, x, y] = m.map(Number);
    if (w < 200 || h < 80) continue;
    if (!sh("xwininfo", "-id", line.trim().split(/\s+/)[0]).includes("Map State: IsViewable")) continue;
    out.push([x, y, w, h]);
  }
  return out;
}

let baseline = census();

/// Runs one route that could open a window and reports what the X server says
/// afterwards. The names carry the window id, so a window that merely changed
/// size reads as a replacement rather than as an addition; that is deliberate,
/// since a resized Chromium top-level is as much of a failure as a new one.
async function leg(name: string, run: () => Promise<unknown>): Promise<void> {
  let detail = "";
  try {
    const value = await run();
    detail = value === undefined ? "" : String(value).slice(0, 80);
  } catch (error) {
    detail = `threw: ${(error as Error).message.slice(0, 80)}`;
  }
  await Bun.sleep(2500);
  const after = census();
  const added = after.filter((w) => !baseline.includes(w));
  baseline = after;
  check(name, added.length === 0, added.length ? `stray top-level ${added.join(" | ")}` : detail || "no new top-level");
}

const extensionsInfo = `new Promise((resolve) => {
  if (typeof chrome === "undefined" || !chrome.developerPrivate) { resolve("[]"); return; }
  chrome.developerPrivate.getExtensionsInfo({ includeDisabled: true, includeTerminated: true },
    (list) => resolve(JSON.stringify(list.map((e) => ({ id: e.id, name: e.name, state: e.state, icon: (e.iconUrl ?? "").slice(0, 12) })))));
})`;

const swTarget = await waitForTarget(port, (t) => t.type === "service_worker" && t.url.startsWith("chrome-extension://"), 60000);
const extId = swTarget.url.split("/")[2]!;
check("extensionServiceWorker", true, swTarget.url);

const pageTarget = await waitForTarget(port, (t) => t.type === "page" && t.url.startsWith("http://127.0.0.1"), 30000);
const page = await Session.open(pageTarget.webSocketDebuggerUrl!);
await page.send("Page.enable");
const sw = await Session.open(swTarget.webSocketDebuggerUrl!);

if (pass === "first") {
  await leg("windowOpen", () => page.eval("window.open('about:blank','_blank'); 'window.open'", true));
  await leg("targetBlank", () =>
    page.eval(
      "const a=document.createElement('a');a.href='about:blank';a.target='_blank';a.textContent='x';document.body.appendChild(a);a.click();'link click'",
      true,
    ));
  await leg("windowsCreate", () => sw.eval("ndOpenWindow().then(w=>'chrome.windows.create '+w.id).catch(e=>'rejected: '+e.message)"));
  await leg("tabsCreate", () => sw.eval("ndOpenTab().then(t=>'chrome.tabs.create '+t.id).catch(e=>'rejected: '+e.message)"));
  await leg("openOptionsPage", () => sw.eval("Promise.resolve(ndOpenOptions()).then(()=>'openOptionsPage').catch(e=>'rejected: '+e.message)"));
  // window.print blocks the renderer for as long as the preview is up, so it
  // is fired and never awaited.
  await leg("windowPrint", async () => {
    // Rejected whenever the preview or a later navigation takes the target
    // away, and nothing is waiting on it.
    page.send("Runtime.evaluate", { expression: "window.print()", userGesture: true }).catch(() => {});
    return "window.print";
  });
  await leg("viewSource", () => page.send("Page.navigate", { url: `view-source:${pageTarget.url}` }));

  // Chrome's accelerators reach the browser process only from a real key
  // event, so the pointer clicks into the view first to give it X input focus.
  sh("xdotool", "mousemove", "300", "400", "click", "1");
  await Bun.sleep(800);
  for (const key of ["ctrl+n", "ctrl+t", "ctrl+shift+n", "ctrl+u", "ctrl+p", "ctrl+shift+o", "ctrl+h", "ctrl+j"]) {
    await leg(`accelerator ${key}`, async () => {
      sh("xdotool", "key", "--clearmodifiers", key);
      return key;
    });
  }

  await sw.eval(`ndWrite(${JSON.stringify(token)}).then(()=>'written')`);
  check("storageWritten", true, token);
}

// Its own pass so the other two never navigate under an open inspector. It
// quits like they do, which is the assertion that the engine closes the
// devtools browser before the one it inspects.
if (pass === "devtools") {
  // The inspector draws its own toolbar only in a pane tall enough for it, and
  // this app's webview is one row among many at the window's default size. The
  // census is re-taken because the app's own window is in it.
  sh("xdotool", "search", "--name", "ND CEF Probe", "windowsize", "1240", "860");
  await Bun.sleep(2000);
  baseline = census();
  // The pointer clicks into the view first: a real X key event only reaches
  // Chrome's accelerators through the window that has X input focus.
  sh("xdotool", "mousemove", "300", "400", "click", "1");
  await Bun.sleep(800);
  await leg("devToolsDocked", async () => {
    sh("xdotool", "key", "--clearmodifiers", "F12");
    await Bun.sleep(4000);
    const devtools = (await targets(port)).filter((t) => t.url.startsWith("devtools://"));
    if (devtools.length === 0) throw new Error("no devtools:// target after F12");
    if (shotPath) sh("import", "-window", "root", shotPath);
    return `${devtools.length} devtools target(s), shot ${shotPath || "skipped"}`;
  });
  const onRoot = sh("xwininfo", "-root", "-children").split("\n").filter((l) => /DevTools/.test(l));
  check("devToolsInsideView", onRoot.length === 0, `no devtools window among the root's children (${onRoot.length} found)`);

  // The page and the inspector have to meet with no seam, at the open, across
  // a resize of the window they are in, and after the dock has been taken down
  // and put back. A strip of bare background between them is what a page
  // window that does not fill what it was given looks like.
  await checkTiling("open");
  // Growing is the direction that shows: a page whose window grew but whose
  // layout did not leaves bare background where the two used to meet, while
  // one that failed to shrink is merely clipped by its own window.
  // The last size is the one the close-button legs below run at: the inspector
  // draws its toolbar only in a pane tall enough for it.
  for (const [phase, w, h] of [["grown", 1420, 900], ["shrunk", 1120, 700], ["restored", 1240, 860]] as const) {
    sh("xdotool", "search", "--name", "ND CEF Probe", "windowsize", String(w), String(h));
    await Bun.sleep(3000);
    baseline = census();
    await checkTiling(phase);
  }
  sh("xdotool", "key", "--clearmodifiers", "F12");
  await Bun.sleep(2500);
  sh("xdotool", "key", "--clearmodifiers", "F12");
  await Bun.sleep(4000);
  await checkTiling("reopened");

  // The inspector's own close button. It is drawn only when the frontend was
  // told it can dock, and clicking it has to take the dock down through the
  // engine rather than leaving the toggle pointing at an inspector that is
  // already gone.
  const frontendTarget = await waitForTarget(port, (t) => t.url.startsWith("devtools://"));
  check("devToolsCanDock", frontendTarget.url.includes("can_dock=true"), frontendTarget.url.slice(-48));
  const closedBefore = await devToolsClosedMarkers();
  const frontend = await Session.open(frontendTarget.webSocketDebuggerUrl ?? "");
  await frontend.send("Runtime.enable");
  const closeBox = await clickDevToolsClose(frontend);
  check(
    "devToolsCloseButton",
    closeBox !== null,
    closeBox ? `${Math.round(closeBox.width)}x${Math.round(closeBox.height)} at ${Math.round(closeBox.x)}` : "no close control in the toolbar",
  );
  frontend.close();
  await Bun.sleep(3000);
  check(
    "devToolsClosedByButton",
    (await targets(port)).filter((t) => t.url.startsWith("devtools://")).length === 0,
    "no devtools:// target left",
  );
  const closedAfter = await devToolsClosedMarkers();
  check("devToolsCloseReported", closedAfter > closedBefore, `ND_CEF devtoolsClosed ${closedBefore} -> ${closedAfter}`);
  sh("xdotool", "key", "--clearmodifiers", "F12");
  await Bun.sleep(4000);
  check(
    "devToolsToggleNotStale",
    (await targets(port)).filter((t) => t.url.startsWith("devtools://")).length === 1,
    "F12 after the close button reopened the inspector",
  );

  await leg("devToolsToggleOff", async () => {
    sh("xdotool", "key", "--clearmodifiers", "F12");
    await Bun.sleep(2500);
    return `${(await targets(port)).filter((t) => t.url.startsWith("devtools://")).length} devtools target(s) left`;
  });

  // Chromium's own Inspect, picked from the engine's native menu. The engine
  // keeps that item because Chrome style docks the inspector, so the pick has
  // to land in the view rather than opening a DevTools window of its own.
  sh("xdotool", "mousemove", "300", "400", "click", "3");
  await Bun.sleep(1800);
  const rows = await menuRows();
  const inspectAt = rows.indexOf("Inspect");
  check("inspectInMenu", inspectAt >= 0, rows.join(" | ") || "no menu in the trace");
  if (inspectAt >= 0) {
    // Off the popover before keying: GTK moves focus to whatever the pointer
    // is over, and the click that opened the menu left it at its corner.
    sh("xdotool", "mousemove", "1200", "860");
    await Bun.sleep(400);
    for (let i = 0; i < inspectAt; i += 1) {
      sh("xdotool", "key", "--clearmodifiers", "Down");
      await Bun.sleep(150);
    }
    sh("xdotool", "key", "--clearmodifiers", "Return");
    await Bun.sleep(5000);
    const opened = (await targets(port)).filter((t) => t.url.startsWith("devtools://"));
    check("inspectOpensDevTools", opened.length === 1, `${opened.length} devtools target(s) after Inspect`);
    const onRootAfterInspect = sh("xwininfo", "-root", "-children").split("\n").filter((l) => /DevTools/.test(l));
    check("inspectStaysInside", onRootAfterInspect.length === 0, `${onRootAfterInspect.length} devtools window(s) on the root`);
    await checkTiling("inspect");
    if (opened[0]?.webSocketDebuggerUrl) {
      const frontend = await Session.open(opened[0].webSocketDebuggerUrl);
      const selected = await frontend.eval<string>(`(() => {
        const walk = (root) => {
          for (const el of root.querySelectorAll('li.selected, .elements-disclosure .selected')) {
            const text = (el.textContent || '').trim();
            if (text) return text.slice(0, 60);
          }
          for (const el of root.querySelectorAll('*')) {
            if (el.shadowRoot) { const hit = walk(el.shadowRoot); if (hit) return hit; }
          }
          return '';
        };
        return walk(document);
      })()`).catch(() => "");
      frontend.close();
      check("inspectSelectsElement", selected.length > 0, selected || "the Elements selection is not readable");
    }
    // Closed again, so the pass quits in the shape its clean-quit leg expects.
    sh("xdotool", "key", "--clearmodifiers", "F12");
    await Bun.sleep(2500);
  }
}

// chrome://extensions is the only page Chromium exposes its extension registry
// on, and a Chrome-style webview can be navigated straight to it.
if (pass !== "devtools") {
  await page.send("Page.navigate", { url: "chrome://extensions/" });
  await Bun.sleep(3000);
  const listed = JSON.parse(await page.eval<string>(extensionsInfo)) as Array<{ id: string; name: string; state: string; icon: string }>;
  const mine = listed.find((e) => e.id === extId);
  check("extensionRegistered", !!mine, mine ? `${mine.name} ${mine.state} icon=${mine.icon}` : `not among ${listed.length} extensions`);
  check("extensionEnabled", mine?.state === "ENABLED", mine?.state ?? "missing");
}

// The Chrome Web Store, opt-in because it needs the network and Google's
// consent interstitial. Chromium's own "Add <name>?" prompt is a Views window
// with no CEF callback: the engine moves it over the view and reports it, and
// the click that accepts it is a real X one.
if (pass === "store") {
  const item = process.env.ND_CEF_STORE_ITEM ?? "ddkjiahejlhfcafbddmgiahcphecmpfh";
  await page.send("Page.navigate", { url: `https://chromewebstore.google.com/detail/${item}` });
  await Bun.sleep(9000);
  const consent = await page.eval<string>(`(() => {
    const hit = [...document.querySelectorAll("button, [role=button]")].find((e) => /reject all|accept all/i.test(e.innerText || ""));
    if (!hit) return "none";
    hit.scrollIntoView();
    hit.click();
    return hit.innerText.trim();
  })()`);
  if (consent !== "none") await Bun.sleep(9000);
  check("storeDetailPage", (await page.eval<string>("location.href")).includes(item), consent);
  check("webstorePrivate", (await page.eval<string>("typeof chrome !== 'undefined' && typeof chrome.webstorePrivate")) === "object", "the store page's own install API");

  // What the store page's own install API answered, which is the only place an
  // install that goes wrong says so.
  await page.eval(`(() => {
    window.__ndStore = [];
    const wp = chrome.webstorePrivate;
    for (const name of Object.keys(wp)) {
      const original = wp[name];
      if (typeof original !== "function") continue;
      wp[name] = function (...args) {
        const cb = typeof args[args.length - 1] === "function" ? args.pop() : null;
        return original.call(wp, ...args, function (...answer) {
          window.__ndStore.push(name + " " + JSON.stringify(answer).slice(0, 120) +
            (chrome.runtime.lastError ? " lastError=" + chrome.runtime.lastError.message : ""));
          if (cb) cb(...answer);
        });
      };
    }
  })()`);
  const dialogsBefore = chromeDialogs().map(String);
  // The page's own button, clicked with a transient activation rather than a
  // synthetic pointer: the store's layout shifts while its images land, and a
  // click aimed at a rectangle read a moment earlier misses.
  const clicked = await page.eval<string>(`(() => {
    const b = [...document.querySelectorAll("button, [role=button]")].find((e) => /add to chrome/i.test(e.innerText || ""));
    if (!b) return "no Add to Chrome button";
    b.scrollIntoView({ block: "center" });
    b.click();
    return "clicked";
  })()`, true);
  check("storeAddToChrome", clicked === "clicked", clicked);

  // Chromium's own prompt, found on the X server: it is a Views widget, so no
  // CEF callback names it, and it carries no WM_NAME until well after it is up.
  // The engine has already moved it over the view by the time it is findable.
  let prompt: number[] | null = null;
  for (let i = 0; i < 40; i++) {
    prompt = chromeDialogs().find((d) => !dialogsBefore.includes(String(d))) ?? null;
    if (prompt) break;
    await Bun.sleep(500);
  }
  check("storeInstallPrompt", !!prompt, prompt ? `${prompt[2]}x${prompt[3]} at ${prompt[0]},${prompt[1]}` : "Chromium never raised the Add prompt");
  if (prompt) {
    const [x, y, w, h] = prompt;
    // The prompt was moved onto the view a moment ago, and Views takes a click
    // only once it has laid out at the new place.
    await Bun.sleep(4000);
    // "Add extension" is the wider of the prompt's two buttons.
    if (shotPath) sh("import", "-window", "root", shotPath);
    sh("xdotool", "mousemove", String(x + w - 80), String(y + h - 39), "click", "1");
  }

  await Bun.sleep(6000);
  const answers = (await page.eval<string[]>("window.__ndStore")) ?? [];
  check("storeInstallAnswered", answers.some((a) => a.startsWith("completeInstall")), answers.join(" | ") || "the store page's API said nothing");

  await page.send("Page.navigate", { url: "chrome://extensions/" });
  await Bun.sleep(3000);
  let installed: Array<{ id: string; name: string; state: string }> = [];
  for (let i = 0; i < 30; i++) {
    installed = JSON.parse(await page.eval<string>(extensionsInfo));
    if (installed.some((e) => e.id === item)) break;
    await Bun.sleep(1000);
  }
  const store = installed.find((e) => e.id === item);
  check("storeInstalled", store?.state === "ENABLED", store ? `${store.name} ${store.state}` : `not among ${installed.length} extensions`);
  // Chromium commits the profile on a timer, and the restart leg reads what
  // landed on disk rather than what this process still has in memory.
  await Bun.sleep(4000);
}

// The proof the install is real rather than a running-process artefact: same
// profile, a restart, and no --load-extension for it anywhere.
if (pass === "storeRestart") {
  const item = process.env.ND_CEF_STORE_ITEM ?? "ddkjiahejlhfcafbddmgiahcphecmpfh";
  const installed = JSON.parse(await page.eval<string>(extensionsInfo)) as Array<{ id: string; name: string; state: string }>;
  const store = installed.find((e) => e.id === item);
  check("storeSurvivedRestart", store?.state === "ENABLED", store ? `${store.name} ${store.state}` : `not among ${installed.length} extensions`);
  // An MV3 worker is only a target while it runs, and one that has nothing to
  // do after a restart is asleep, so it is woken the way Chrome's own
  // "service worker (inactive)" link does.
  await page.eval(`new Promise((r) => chrome.developerPrivate.openDevTools(
    { extensionId: ${JSON.stringify(item)}, renderViewId: -1, renderProcessId: -1, isServiceWorker: true }, () => r(0)))`);
  let worker;
  for (let i = 0; i < 40; i++) {
    worker = (await targets(port)).find((t) => t.type === "service_worker" && t.url.startsWith(`chrome-extension://${item}/`));
    if (worker) break;
    await Bun.sleep(500);
  }
  check("storeServiceWorker", !!worker, worker?.url ?? "no service worker target for the store extension");
}

if (pass === "second") {
  const stored = await sw.eval<string>("ndRead()");
  check("storageSurvivedRestart", stored === token, `read ${JSON.stringify(stored)}`);
}

const strays = census().filter((w) => !w.includes("ND CEF Probe"));
check("noStrayTopLevel", strays.length === 0, strays.length ? strays.join(" | ") : "only the host window");

page.close();
sw.close();

if (failures.length > 0) {
  console.error(`ND_CEF_CHROME_FAIL ${failures.length} check(s) failed on pass ${pass}:`);
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}
console.log(`ND_CEF_CHROME_LEGS_OK(${pass})`);
// Explicit: a debugger socket that Chromium tears down under us rejects after
// the last check, and an unhandled rejection is a non-zero exit on a run that
// passed.
process.exit(0);
