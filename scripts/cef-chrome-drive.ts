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
import { Session, targets, waitForTarget } from "./cdp.ts";

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

// Its own pass because a Chrome-style browser that has had devtools open dies
// on the way out of the process, docked or not (CefBrowserInfo::RemoveFrame on
// a TabStripModel teardown; the same crash with CEF's own devtools window). The
// pass that runs it is killed rather than asked to quit, so the clean-quit
// assertion the other passes make stays strict.
if (pass === "devtools") {
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
  await leg("devToolsToggleOff", async () => {
    sh("xdotool", "key", "--clearmodifiers", "F12");
    await Bun.sleep(2500);
    return `${(await targets(port)).filter((t) => t.url.startsWith("devtools://")).length} devtools target(s) left`;
  });
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
