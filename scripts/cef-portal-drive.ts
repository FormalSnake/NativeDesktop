#!/usr/bin/env bun
// A tab dragged into another window, driven against examples/multiwindow under
// ND_CEF_STYLE=chrome. The view lives in a `createPortal` pool and moves
// between the two windows' slots with `moveNode`, so React never unmounts it;
// what this asserts is that the LIVE browser goes with it. Same debugger
// target, same JS state, same scroll offset, no navigation, and the X child
// the browser renders into reparented under the window that now shows it.
//
// scripts/headless-app-chrome.sh runs it (ND_ACCEPT_DRIVE), so it inherits the
// same two rigs and the same watchdog contract.
import { readFileSync } from "node:fs";
import { connectApp } from "@nativedesktop/test";
import { Session, targets, waitForTarget } from "./cdp.ts";

const rig = (process.env.ND_ACCEPT_RIG ?? "x11") as "x11" | "wlr";
const port = Number(process.env.ND_CDP_PORT ?? "9555");
const fixture = process.env.ND_ACCEPT_FIXTURE ?? "http://127.0.0.1:9557/";
const shots = process.env.ND_ACCEPT_SHOTS ?? "/tmp";
const hostLog = process.env.ND_ACCEPT_HOST_LOG ?? "";
const hostPid = Number(process.env.ND_ACCEPT_HOST_PID ?? "0");
const legBudgetMs = Number(process.env.ND_ACCEPT_LEG_BUDGET_MS ?? "180000");

const failures: string[] = [];
let lastProgress = Date.now();
let lastLeg = "startup";

function check(name: string, ok: boolean, detail: string): void {
  console.log(`  ${name}: ${ok ? "ok" : "FAIL"} (${detail})`);
  if (!ok) failures.push(`${name}: ${detail}`);
  lastLeg = name;
  lastProgress = Date.now();
}

function die(why: string): never {
  console.error(`ND_CEF_PORTAL_FAIL(${rig}) ${why}`);
  console.error(`  last leg to report: ${lastLeg}`);
  if (hostLog) {
    try {
      console.error(readFileSync(hostLog, "utf8").split("\n").slice(-25).join("\n"));
    } catch {
      // The log is the host's; a run that never got one says so by its absence.
    }
  }
  process.exit(1);
}

setInterval(() => {
  if (hostPid > 0) {
    try {
      process.kill(hostPid, 0);
    } catch {
      die(`the host process ${hostPid} is gone`);
    }
  }
  if (Date.now() - lastProgress > legBudgetMs) die(`no leg reported for ${Math.round((Date.now() - lastProgress) / 1000)}s`);
}, 5000);

function sh(...argv: string[]): string {
  return Bun.spawnSync(argv, { env: process.env as Record<string, string> }).stdout.toString().trim();
}

const capture = (path: string) => (rig === "x11" ? sh("import", "-window", "root", path) : sh("grim", path));

interface Geom { x: number; y: number; w: number; h: number; mapped: boolean }

function geom(id: string): Geom | null {
  const out = sh("xwininfo", "-id", id);
  const num = (label: string) => Number(out.match(new RegExp(`${label}:\\s+(-?\\d+)`))?.[1] ?? NaN);
  const x = num("Absolute upper-left X");
  if (Number.isNaN(x)) return null;
  return { x, y: num("Absolute upper-left Y"), w: num("Width"), h: num("Height"), mapped: out.includes("Map State: IsViewable") };
}

function childrenOf(id: string): string[] {
  return sh("xwininfo", "-id", id, "-children")
    .split("\n")
    .map((line) => line.match(/^\s*(0x[0-9a-f]+)\s/)?.[1])
    .filter((v): v is string => !!v);
}

/// The view's container and the browser's own window, from the host's trace.
function embedded(): { container: string; cef: string } | null {
  if (!hostLog) return null;
  let text = "";
  try {
    text = readFileSync(hostLog, "utf8");
  } catch {
    return null;
  }
  const container = [...text.matchAll(/embed node=\d+ parent=\S+ container=(0x[0-9a-f]+)/g)].pop()?.[1];
  const cef = [...text.matchAll(/created node=\d+ cefWindow=(0x[0-9a-f]+)/g)].pop()?.[1];
  return container && cef && cef !== "0x0" ? { container, cef } : null;
}

/// The host's toplevels, largest first: the example makes two.
/// Every mapped override-redirect window big enough to be a menu or a popup.
function popups(minW = 40, minH = 20): string[] {
  return sh("xwininfo", "-root", "-children")
    .split("\n")
    .map((line) => line.match(/^\s*(0x[0-9a-f]+)\s/)?.[1])
    .filter((v): v is string => !!v)
    .filter((id) => {
      const info = sh("xwininfo", "-id", id);
      const g = geom(id);
      return info.includes("Override Redirect State: yes") && info.includes("Map State: IsViewable")
        && !!g && g.w >= minW && g.h >= minH && g.x > -100 && g.y > -100;
    });
}

function toplevels(): string[] {
  return sh("xdotool", "search", "--classname", "nd-hello")
    .split("\n")
    .filter(Boolean)
    .map((dec) => `0x${Number(dec).toString(16)}`)
    .filter((id) => {
      const g = geom(id);
      return !!g && g.mapped && g.w >= 200 && g.h >= 200;
    });
}

const app = await connectApp();
const pageTarget = await waitForTarget(port, (t) => t.type === "page" && t.url.startsWith(fixture), 120000);
const targetId = pageTarget.id;
const page = await Session.open(pageTarget.webSocketDebuggerUrl!);
await page.send("Runtime.enable");

/// Which toplevel the view's container is a child of right now.
function hostWindow(): string | null {
  const view = embedded();
  if (!view) return null;
  for (const top of toplevels()) {
    if (childrenOf(top).includes(view.container)) return top;
  }
  return null;
}

async function settled(expect: string, timeoutMs = 8000): Promise<{ ok: boolean; detail: string }> {
  const deadline = Date.now() + timeoutMs;
  let detail = "no view";
  while (Date.now() < deadline) {
    const view = embedded();
    const under = hostWindow();
    if (view && under === expect) {
      const container = geom(view.container);
      const cef = geom(view.cef);
      const m = await page.eval<string>("JSON.stringify({w:innerWidth,h:innerHeight,dpr:devicePixelRatio})").catch(() => "");
      if (container && cef && m) {
        const size = JSON.parse(m) as { w: number; h: number; dpr: number };
        detail = `container ${container.w}x${container.h} under ${under}, cef ${cef.w}x${cef.h}, page ${size.w}x${size.h}`;
        // The browser's window is the container's width, or the page half of
        // it while the inspector is docked beside it; either way the page has
        // to have laid out for whatever the browser's window now is.
        const fits = cef.h === container.h && cef.w <= container.w && Math.abs(Math.round(size.w * size.dpr) - cef.w) <= 1;
        if (fits) return { ok: true, detail };
      }
    } else {
      detail = `container under ${under ?? "nothing"}, wanted ${expect}`;
    }
    await Bun.sleep(150);
  }
  return { ok: false, detail };
}

const windows = toplevels();
check("twoWindows", windows.length === 2, `${windows.length} toplevel(s): ${windows.join(" ")}`);
const [first, second] = windows;
const startedUnder = hostWindow();
check("startsInOneWindow", startedUnder !== null, `container under ${startedUnder ?? "nothing"}`);
const windowA = startedUnder ?? first!;
const windowB = windows.find((w) => w !== windowA) ?? second!;

// State the move has to preserve: a JS global, a scroll offset and the
// document's identity. A reload would clear all three.
await page.eval("window.__portalMark = 'mark-' + Date.now(); scrollTo(0, 900); document.getElementById('probe').value = 'kept'; '1'");
await Bun.sleep(400);
const before = JSON.parse(await page.eval<string>("JSON.stringify({mark:window.__portalMark,scroll:Math.round(scrollY),probe:document.getElementById('probe').value,href:location.href})"));

await app.getByTestId("bring-b").click();
await Bun.sleep(1500);
const movedOk = await settled(windowB);
check("movedToSecondWindow", movedOk.ok, movedOk.detail);

{
  const after = JSON.parse(await page.eval<string>("JSON.stringify({mark:window.__portalMark,scroll:Math.round(scrollY),probe:document.getElementById('probe').value,href:location.href})"));
  const live = (await targets(port)).some((t) => t.id === targetId);
  check("sameBrowser", live, `debugger target ${targetId} ${live ? "still listed" : "gone"}`);
  check(
    "stateSurvivedTheMove",
    after.mark === before.mark && after.scroll === before.scroll && after.probe === before.probe && after.href === before.href,
    `mark ${after.mark === before.mark}, scroll ${before.scroll}->${after.scroll}, field ${JSON.stringify(after.probe)}, url ${after.href === before.href}`,
  );
}

{
  // It has to be interactive where it landed, with the same real input the
  // other gate uses. Back to the top of the page first: the state legs left it
  // scrolled, and an element's rect is viewport-relative.
  await page.eval("scrollTo(0, 0); '1'");
  await settled(windowB);
  await Bun.sleep(500);
  const view = embedded()!;
  const container = geom(view.container)!;
  const r = JSON.parse(await page.eval<string>("JSON.stringify(nd.rect('probe'))")) as { x: number; y: number; w: number; h: number };
  const m = JSON.parse(await page.eval<string>("JSON.stringify(nd.metrics())")) as { dpr: number; scrollY: number };
  const at = { x: Math.round(container.x + (r.x + r.w / 2) * m.dpr), y: Math.round(container.y + (r.y + r.h / 2) * m.dpr) };
  sh("xdotool", "mousemove", "--sync", String(at.x), String(at.y));
  sh("xdotool", "click", "1");
  await Bun.sleep(1200);
  sh("xdotool", "type", "--delay", "30", "MOVED");
  await Bun.sleep(900);
  const typed = await page.eval<string>("document.getElementById('probe').value");
  check("inputWorksAfterTheMove", typed.includes("MOVED"), `field is ${JSON.stringify(typed)}`);

  sh("xdotool", "click", "3");
  await Bun.sleep(2000);
  const popup = popups(120, 80);
  capture(`${shots}/portal-context-menu.png`);
  const g = popup.length > 0 ? geom(popup[0]!) : null;
  check("contextMenuAfterTheMove", popup.length > 0, g ? `${popup[0]} ${g.w}x${g.h}+${g.x}+${g.y}` : "no menu window");
  let left = popups(120, 80);
  for (let i = 0; i < 4 && left.length > 0; i++) {
    sh("xdotool", "key", "--clearmodifiers", "Escape");
    await Bun.sleep(700);
    left = popups(120, 80);
  }
  check("contextMenuDismissedAfterTheMove", left.length === 0, `${left.length} popup window(s) left`);
}

{
  const view = embedded()!;
  const container = geom(view.container)!;
  const r = JSON.parse(await page.eval<string>("JSON.stringify(nd.rect('picker'))")) as { x: number; y: number; w: number; h: number };
  const m = JSON.parse(await page.eval<string>("JSON.stringify(nd.metrics())")) as { dpr: number };
  sh("xdotool", "mousemove", "--sync",
    String(Math.round(container.x + (r.x + r.w / 2) * m.dpr)),
    String(Math.round(container.y + (r.y + r.h / 2) * m.dpr)));
  sh("xdotool", "click", "1");
  await Bun.sleep(1500);
  const dropdown = popups(30, 40);
  capture(`${shots}/portal-select.png`);
  check("selectAfterTheMove", dropdown.length > 0, dropdown.length > 0 ? `${dropdown[0]}` : "no dropdown window");
  sh("xdotool", "key", "--clearmodifiers", "Escape");
  await Bun.sleep(700);
}

{
  // The inspector docks in whichever window shows the view, survives a move
  // back, and closes again.
  sh("xdotool", "key", "--clearmodifiers", "F12");
  let docked: Awaited<ReturnType<typeof targets>> = [];
  for (let i = 0; i < 40; i++) {
    docked = (await targets(port)).filter((t) => t.url.startsWith("devtools://"));
    if (docked.length > 0) break;
    await Bun.sleep(500);
  }
  check("devToolsDocksAfterTheMove", docked.length > 0, `${docked.length} devtools target(s)`);
  await app.getByTestId("bring-a").click();
  await Bun.sleep(1800);
  const backOk = await settled(windowA);
  const stillDocked = (await targets(port)).filter((t) => t.url.startsWith("devtools://"));
  check("devToolsSurvivesTheMoveBack", stillDocked.length === docked.length, `${stillDocked.length} devtools target(s) after moving back`);
  check("movedBack", backOk.ok, backOk.detail);
  // F12 is a Chrome accelerator, so the page has to hold the keyboard for it.
  const back = embedded();
  if (back) {
    const c = geom(back.container)!;
    sh("xdotool", "mousemove", "--sync", String(c.x + Math.round(c.w / 4)), String(c.y + Math.round(c.h / 2)));
    sh("xdotool", "click", "1");
    await Bun.sleep(1200);
  }
  sh("xdotool", "key", "--clearmodifiers", "F12");
  await Bun.sleep(4000);
  const remaining = (await targets(port)).filter((t) => t.url.startsWith("devtools://")).length;
  check("devToolsClosesAfterTheMove", remaining === 0, `${remaining} devtools target(s) left`);
}

{
  const after = JSON.parse(await page.eval<string>("JSON.stringify({mark:window.__portalMark,scroll:Math.round(scrollY),href:location.href})"));
  check("stateSurvivedTheMoveBack", after.mark === before.mark && after.href === before.href, `mark ${after.mark === before.mark}, url ${after.href === before.href}`);
}

{
  // The window the tab came from goes away while the tab is somewhere else.
  if (rig === "x11") sh("wmctrl", "-i", "-c", windowB);
  else sh("swaymsg", "-t", "command", "--", "[title=\"Window B\"]", "kill");
  await Bun.sleep(2500);
  const left = toplevels();
  const live = (await targets(port)).some((t) => t.id === targetId);
  const alive = await page.eval<number>("1 + 1").catch(() => 0);
  check(
    "originWindowClosedWithoutKillingTheBrowser",
    live && alive === 2 && left.length === 1,
    `${left.length} toplevel(s) left, target ${live ? "listed" : "gone"}, page answers ${alive}`,
  );
  const stillOk = await settled(windowA);
  check("layoutAfterTheOriginWindowClosed", stillOk.ok, stillOk.detail);
}

capture(`${shots}/portal-final.png`);

if (failures.length > 0) {
  console.error(`ND_CEF_PORTAL_FAIL(${rig}) ${failures.length} leg(s) failed:`);
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}
console.log(`ND_APP_CHROME_LEGS_OK(${rig})`);
process.exit(0);
