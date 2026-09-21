#!/usr/bin/env bun
// scripts/cef-menu-drive.ts: the native context menu under Chrome style.
//
// Real X11 right-clicks, because a context menu is the one thing CDP cannot
// raise: Input.dispatchMouseEvent goes to the renderer and never reaches the
// browser process's menu path. What the menu ended up containing is read from
// the host's own ND_WEBVIEW_TRACE output (`menuItem` is the model Chromium
// handed the engine, `menuShown` what survived the filter and was drawn), and
// the X server is asked what it has on the root before and after.
import { Session, targets, waitForTarget } from "./cdp.ts";

const port = Number(process.env.ND_CDP_PORT ?? "9334");
const display = process.env.DISPLAY ?? ":96";
const hostLog = process.env.ND_HOST_LOG ?? "";
const shotPath = process.env.ND_MENU_SHOT_PATH ?? "";

const failures: string[] = [];
function check(name: string, ok: boolean, detail: string): void {
  console.log(`  ${name}: ${ok ? "ok" : "FAIL"} (${detail})`);
  if (!ok) failures.push(`${name}: ${detail}`);
}

function sh(...argv: string[]): string {
  return Bun.spawnSync(argv, { env: { ...process.env, DISPLAY: display } }).stdout.toString().trim();
}

/// Mapped windows a user could see, the same rule the chrome gate's census
/// uses. The menu adds exactly one of these while it is up.
function census(): string[] {
  const rows: string[] = [];
  for (const line of sh("xwininfo", "-root", "-children").split("\n")) {
    const m = line.match(/^\s*(0x[0-9a-f]+)\s+(".*?"|\(has no name\)).*?\s(\d+)x(\d+)\+/);
    if (!m) continue;
    const [, id, , w, h] = m;
    if (Number(w) < 60 || Number(h) < 60) continue;
    if (!sh("xwininfo", "-id", id).includes("Map State: IsViewable")) continue;
    rows.push(`${id} ${w}x${h}`);
  }
  return rows;
}

if (!hostLog) {
  console.error("ND_CEF_MENU_FAIL ND_HOST_LOG is not set; the rendered menu is only readable from the host trace");
  process.exit(1);
}

async function logText(): Promise<string> {
  return await Bun.file(hostLog).text();
}

interface Shown {
  depth: number;
  id: number;
  kind: string;
  enabled: boolean;
  checked: boolean;
  accel: string;
  label: string;
}

const shown_re = /menuShown depth=(\d+) index=\d+ id=(-?\d+) kind=(\w+) enabled=(\d) checked=(\d) accel=(\S*) label=(.*)$/;

/// The `menuShown` block of the most recent menu, plus the `menuItem` model it
/// was filtered from. Both are read from `tail`, the slice of the log written
/// since the right-click.
function parseMenu(tail: string): { shown: Shown[]; model: string[] } {
  const shown: Shown[] = [];
  const model: string[] = [];
  for (const line of tail.split("\n")) {
    const m = shown_re.exec(line);
    if (m) {
      shown.push({
        depth: Number(m[1]),
        id: Number(m[2]),
        kind: m[3]!,
        enabled: m[4] === "1",
        checked: m[5] === "1",
        accel: m[6] ?? "",
        label: m[7]!,
      });
      continue;
    }
    const mi = /menuItem depth=\d+ index=\d+ id=-?\d+ type=\d+ enabled=\d checked=\d label=(.*)$/.exec(line);
    if (mi) model.push(mi[1]!);
  }
  return { shown, model };
}

function labels(shown: Shown[]): string[] {
  return shown.filter((s) => s.kind !== "separator").map((s) => s.label);
}

/// Opens the menu at a page coordinate and returns what the host drew.
async function openMenu(px: number, py: number): Promise<{ shown: Shown[]; model: string[]; tail: string }> {
  const before = (await logText()).length;
  sh("xdotool", "mousemove", String(origin.x + px), String(origin.y + py), "click", "3");
  await Bun.sleep(1800);
  const tail = (await logText()).slice(before);
  return { ...parseMenu(tail), tail };
}

async function dismiss(): Promise<string> {
  const before = (await logText()).length;
  sh("xdotool", "key", "--clearmodifiers", "Escape");
  await Bun.sleep(1200);
  return (await logText()).slice(before);
}

// ------------------------------------------------------------------ setup ----

// The probe registers its setContextMenuItems tree as the last thing its run
// does, so that trace line is also "the app is up and its views have loaded".
const ready = Date.now() + 120000;
while (Date.now() < ready && !(await logText()).includes("setContextMenuItems")) {
  await Bun.sleep(500);
}
await waitForTarget(port, (t) => t.type === "page" && t.url.startsWith("http://127.0.0.1"), 60000);
const pages = (await targets(port)).filter((t) => t.type === "page" && t.url.startsWith("http://127.0.0.1"));
if (pages.length === 0) {
  console.error("ND_CEF_MENU_FAIL no fixture page target");
  process.exit(1);
}
const base = new URL(pages[0]!.url).origin;
// Every fixture view, because the visible one is not distinguishable over CDP
// and a hidden view showing the same page costs nothing.
const sessions: Session[] = [];
for (const target of pages) {
  const session = await Session.open(target.webSocketDebuggerUrl!);
  await session.send("Page.navigate", { url: `${base}/menu` });
  sessions.push(session);
}
await Bun.sleep(3000);
const page = sessions[sessions.length - 1]!;

const baseline = census();

// The view's origin on screen: right-click somewhere inside it and read back
// the view coordinate the engine reported for the click. Nothing else in the
// host knows where the X child window landed.
//
// Where to aim comes from the host's own embed trace rather than a fixed
// screen point: the app above the view decides how tall it is, and a probe
// that grows one label pushes the view out from under any point picked here.
const containers = [...(await logText()).matchAll(
  /ND_CEF embed node=\d+ parent=0x[0-9a-f]+ container=(0x[0-9a-f]+) bounds=\d+x\d+\+-?\d+\+-?\d+ mapped=true/g,
)].map((m) => m[1]!);
/// The centre of a container that is still on screen, newest first: the trace
/// also carries views from windows the probe has since closed, and those
/// containers are gone from the X server.
function aimAt(): { x: number; y: number } | null {
  for (const id of [...containers].reverse()) {
    const out = sh("xwininfo", "-id", id);
    if (!out.includes("Map State: IsViewable")) continue;
    const w = out.match(/^\s*Width:\s+(\d+)/m);
    const h = out.match(/^\s*Height:\s+(\d+)/m);
    const x = out.match(/^\s*Absolute upper-left X:\s+(-?\d+)/m);
    const y = out.match(/^\s*Absolute upper-left Y:\s+(-?\d+)/m);
    if (!w || !h || !x || !y) continue;
    if (Number(w[1]) < 40 || Number(h[1]) < 40) continue;
    return { x: Number(x[1]) + Math.round(Number(w[1]) / 2), y: Number(y[1]) + Math.round(Number(h[1]) / 2) };
  }
  return null;
}
const aim = aimAt();
if (!aim) {
  console.error("ND_CEF_MENU_FAIL the host never reported a mapped view to aim at");
  process.exit(1);
}
sh("xdotool", "mousemove", String(aim.x), String(aim.y), "click", "3");
await Bun.sleep(1800);
const calibration = [...(await logText()).matchAll(/menuOpen node=\d+ at=(-?\d+),(-?\d+)/g)].at(-1);
if (!calibration) {
  console.error(`ND_CEF_MENU_FAIL the engine drew no menu for the calibration click at ${aim.x},${aim.y}`);
  process.exit(1);
}
const origin = { x: aim.x - Number(calibration[1]), y: aim.y - Number(calibration[2]) };
const withMenu = census();
const added = withMenu.filter((w) => !baseline.includes(w));
check("menuIsOneSurface", added.length === 1, added.length ? added.join(" | ") : "the menu added no top-level of its own");
await dismiss();

// ------------------------------------------------------------------- legs ----

const pageMenu = await openMenu(700, 100);
const pageLabels = labels(pageMenu.shown);
check(
  "pageMenu",
  ["Back", "Forward", "Reload", "Save as…", "Inspect"].every((l) => pageLabels.includes(l)),
  pageLabels.join(", "),
);
check(
  "mnemonicsStripped",
  pageMenu.model.some((l) => l.includes("&")) && !pageLabels.some((l) => l.includes("&")),
  `model had ${pageMenu.model.filter((l) => l.includes("&")).length} ampersand labels, the menu none`,
);
check(
  "acceleratorsShown",
  pageMenu.shown.some((s) => s.label === "Reload" && s.accel === "<Control>r") &&
    pageMenu.shown.some((s) => s.label === "Back" && s.accel === "<Alt>Left"),
  pageMenu.shown.filter((s) => s.accel).map((s) => `${s.label}=${s.accel}`).join(" "),
);
check(
  "disabledKept",
  pageMenu.shown.some((s) => s.label === "Forward" && !s.enabled),
  "Forward is drawn insensitive with no history behind the view",
);
// Every one of these is in Chromium's model and is refused by the engine's
// command handler, so none of them may reach the menu.
const dead = ["Print…", "Cast…", "Create QR Code for this page", "View page source"];
const leaked = dead.filter((l) => pageLabels.includes(l));
const offered = dead.filter((l) => pageMenu.model.some((m) => m.replace(/&/g, "") === l));
check("deniedItemsDropped", leaked.length === 0 && offered.length > 0, `model offered ${offered.join(", ")}; menu drew none of them`);
if (shotPath) sh("import", "-window", "root", shotPath);
await dismiss();

const linkMenu = await openMenu(45, 26);
const linkLabels = labels(linkMenu.shown);
check(
  "linkMenu",
  ["Open link in new tab", "Copy link address", "Save link as…"].every((l) => linkLabels.includes(l)),
  linkLabels.join(", "),
);
await dismiss();

const imageMenu = await openMenu(174, 34);
const imageLabels = labels(imageMenu.shown);
check("imageMenu", ["Copy image", "Save image as…"].every((l) => imageLabels.includes(l)), imageLabels.join(", "));
await dismiss();

await page.eval("ndSelect()");
const selectionMenu = await openMenu(300, 26);
const selectionLabels = labels(selectionMenu.shown);
check("selectionMenu", selectionLabels.includes("Copy"), selectionLabels.join(", "));
await dismiss();

const editMenu = await openMenu(490, 22);
const editLabels = labels(editMenu.shown);
check(
  "editableMenu",
  ["Cut", "Copy", "Paste", "Select all"].every((l) => editLabels.includes(l)),
  editLabels.join(", "),
);
const spellRadios = editMenu.shown.filter((s) => s.kind === "radio");
check(
  "spellCheckSubmenu",
  editMenu.shown.some((s) => s.kind === "submenu" && s.label === "Spell check") && spellRadios.some((s) => s.checked),
  `${spellRadios.length} language radio(s), ${spellRadios.filter((s) => s.checked).length} checked`,
);
check(
  "checkItemsCarryState",
  editMenu.shown.some((s) => s.kind === "check" && s.checked),
  editMenu.shown.filter((s) => s.kind === "check").map((s) => `${s.label}=${s.checked}`).join(" "),
);
await dismiss();

// The app's own setContextMenuItems tree, appended by the engine after a
// separator and matched against the click the same way it is under Alloy.
check(
  "appItems",
  ["Probe Item", "Probe Submenu"].every((l) => pageLabels.includes(l)) && !pageLabels.includes("Probe Link Item"),
  pageLabels.filter((l) => l.startsWith("Probe")).join(", "),
);
check("appItemsOnLink", linkLabels.includes("Probe Link Item"), linkLabels.filter((l) => l.startsWith("Probe")).join(", "));
check(
  "appCheckboxState",
  pageMenu.shown.some((s) => s.label === "Probe Beta" && s.kind === "check" && s.checked),
  pageMenu.shown.filter((s) => s.label.startsWith("Probe ")).map((s) => `${s.label}:${s.kind}`).join(" "),
);

// The extension's own items, merged by Chromium into every one of those menus.
for (const [name, menu] of [["page", pageMenu], ["link", linkMenu], ["image", imageMenu], ["editable", editMenu]] as const) {
  const root = menu.shown.find((s) => s.label === "ND Gate Menu");
  const children = menu.shown.filter((s) => s.depth === 1 && s.label.startsWith("Gate Item"));
  check(`extensionItems ${name}`, !!root && root.kind === "submenu" && children.length === 2, root ? `${root.kind}, ${children.length} children` : "absent");
}

// ------------------------------------------------------------ activation ----

const swTarget = (await targets(port)).find((t) => t.type === "service_worker" && t.url.startsWith("chrome-extension://"));
if (!swTarget) {
  console.error("ND_CEF_MENU_FAIL the extension's service worker is not running");
  process.exit(1);
}
const sw = await Session.open(swTarget.webSocketDebuggerUrl!);
await sw.eval("ndClearMenu().then(()=>'cleared')");

const activation = await openMenu(700, 100);
// Rows the keyboard walks: the menu opens with the first one focused, and GTK
// skips separators and insensitive items.
const rows = activation.shown.filter((s) => s.depth === 0 && s.kind !== "separator" && s.enabled);
const target = rows.findIndex((s) => s.label === "ND Gate Menu");
// Off the popover before keying: GTK moves focus to whatever the pointer is
// over, and the click that opened the menu left it at its top corner.
sh("xdotool", "mousemove", "1200", "860");
await Bun.sleep(400);
for (let i = 0; i < target; i += 1) {
  sh("xdotool", "key", "--clearmodifiers", "Down");
  await Bun.sleep(150);
}
// Return on a submenu row slides the popover to that pane with its back button
// focused, so one more Down reaches the first child.
sh("xdotool", "key", "--clearmodifiers", "Return");
await Bun.sleep(800);
sh("xdotool", "key", "--clearmodifiers", "Down");
await Bun.sleep(300);
const beforeActivate = (await logText()).length;
sh("xdotool", "key", "--clearmodifiers", "Return");
await Bun.sleep(2000);
const activationTail = (await logText()).slice(beforeActivate);
const answered = /menuAnswer node=\d+ command=(-?\d+)/.exec(activationTail);
check("activationAnswered", !!answered && Number(answered[1]) > 0, answered ? `command ${answered[1]}` : "no answer in the trace");
const stored = await sw.eval<string>("ndReadMenu()");
check("extensionHandlerFired", stored === "nd-ext-one", `chrome.storage read ${JSON.stringify(stored)}`);

// -------------------------------------------------------------- dismissal ----

const escaped = await openMenu(700, 100);
check("menuReopened", escaped.shown.length > 0, `${escaped.shown.length} item(s)`);
const escapeTail = await dismiss();
check("escapeCancels", /menuAnswer node=\d+ command=0/.test(escapeTail), escapeTail.trim().split("\n").at(-1) ?? "nothing traced");

await openMenu(700, 100);
const beforeAway = (await logText()).length;
sh("xdotool", "mousemove", "1200", "860", "click", "1");
await Bun.sleep(1500);
check(
  "clickAwayCancels",
  /menuAnswer node=\d+ command=0/.test((await logText()).slice(beforeAway)),
  "a click outside the menu answers the callback",
);

await openMenu(700, 100);
const beforeNav = (await logText()).length;
await page.send("Page.navigate", { url: `${base}/one` });
await Bun.sleep(2500);
check(
  "navigationCancels",
  /menuAnswer node=\d+ command=0/.test((await logText()).slice(beforeNav)),
  "a navigation under an open menu answers the callback",
);

const after = census();
const strays = after.filter((w) => !baseline.includes(w));
check("censusRestored", strays.length === 0, strays.length ? strays.join(" | ") : `${after.length} top-level(s), unchanged`);

// Inspect, last: it docks the inspector in the right half of the view, which
// every check above would otherwise have to account for.
const inspect = await openMenu(700, 100);
const inspectRows = inspect.shown.filter((s) => s.depth === 0 && s.kind !== "separator" && s.enabled);
const inspectAt = inspectRows.findIndex((s) => s.label === "Inspect");
sh("xdotool", "mousemove", "1200", "860");
await Bun.sleep(400);
for (let i = 0; i < inspectAt; i += 1) {
  sh("xdotool", "key", "--clearmodifiers", "Down");
  await Bun.sleep(150);
}
sh("xdotool", "key", "--clearmodifiers", "Return");
await Bun.sleep(5000);
const devtools = (await targets(port)).filter((t) => t.url.startsWith("devtools://"));
check("inspectOpensDevTools", devtools.length > 0, `${devtools.length} devtools target(s)`);
const onRoot = sh("xwininfo", "-root", "-children").split("\n").filter((l) => /DevTools/.test(l));
check("devToolsStaysInside", onRoot.length === 0, `${onRoot.length} devtools window(s) on the root`);

for (const session of sessions) session.close();
sw.close();

if (failures.length > 0) {
  console.error(`ND_CEF_MENU_FAIL ${failures.length} check(s) failed:`);
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}
console.log("ND_CEF_MENU_OK the native menu carries Chromium's model, the extension's items and the app's own");
process.exit(0);
