#!/usr/bin/env bun
// Driven by scripts/mac/switch-latency.sh. Switches tabs with real key chords
// (⇧⌘] and ⇧⌘[ through app.cursor, the HID path a person's keyboard takes)
// and reports, per switch, the first animation frame the tab switched to runs
// after the chord was sent. A tab Chromium kept running answers within a
// frame; one it had hidden answers when it has resumed. The pixels themselves
// cannot be read at this rate here (one signed capture binary, a capture per
// call), so this is the frame, not the present. The same chords then go to
// ND_SWITCH_STOCK, a stock browser in front of the app, with the same pages.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { FFIType, dlopen, ptr } from "bun:ffi";
import { connectApp } from "@nativedesktop/test";
import { Session, targets } from "../cdp.ts";

const port = Number(process.env.ND_CDP_PORT ?? "9446");
const fixture = process.env.ND_ACCEPT_FIXTURE ?? "http://127.0.0.1:9733/";
const tabs = Number(process.env.ND_ACCEPT_TABS ?? "6");
const hostLog = process.env.ND_ACCEPT_HOST_LOG ?? "";
const work = process.env.ND_ACCEPT_SHOTS ?? "/tmp";
const rounds = Number(process.env.ND_SWITCH_ROUNDS ?? "10");

const libc = dlopen("/usr/lib/libSystem.B.dylib", { clock_gettime: { args: [FFIType.i32, FFIType.ptr], returns: FFIType.i32 } });
const ts = new BigInt64Array(2);
/** CLOCK_UPTIME_RAW in µs, the clock of every ND_LAT line on macOS. */
function nowUs(): number {
  libc.symbols.clock_gettime(8, ptr(ts));
  return Number(ts[0]) * 1e6 + Math.floor(Number(ts[1]) / 1000);
}

const urls = Array.from({ length: tabs }, (_, i) => (i === 0 ? fixture : i === 1 ? `${fixture}?two` : `${fixture}?tab${i + 1}`));

/** One CDP session per fixture tab, each logging the wall time of its frames. */
async function frameLoggers(p: number): Promise<Map<string, Session>> {
  const sessions = new Map<string, Session>();
  for (const t of (await targets(p)).filter((t) => t.type === "page" && urls.includes(t.url))) {
    const s = await Session.open(t.webSocketDebuggerUrl!).catch(() => null);
    if (!s) continue;
    sessions.set(t.url, s);
    await s.eval(`(() => { if (window.__fr) return; window.__fr = []; const f = () => { __fr.push(Date.now()); if (__fr.length > 600) __fr.shift(); requestAnimationFrame(f); }; requestAnimationFrame(f); })()`).catch(() => {});
  }
  return sessions;
}

async function onShow(sessions: Map<string, Session>): Promise<number> {
  for (const [url, s] of sessions) {
    if ((await s.eval<string>("document.visibilityState").catch(() => "")) === "visible") return urls.indexOf(url);
  }
  return -1;
}

interface Lat { t: number; tag: string }
function latLines(): Lat[] {
  if (!hostLog) return [];
  return readFileSync(hostLog, "utf8").split("\n").flatMap((l) => {
    const m = l.match(/ND_LAT (\d+) (\S+)/);
    return m ? [{ t: Number(m[1]), tag: m[2]! }] : [];
  });
}

interface Switch { label: string; t0w: number; t0u: number; ms: number }

function report(label: string, list: Switch[], lat: Lat[]): void {
  const ms = list.map((s) => s.ms);
  const ok = ms.filter((x) => x >= 0).sort((a, b) => a - b);
  console.log(`  ${label} input->first frame ms: ${ms.join(" ")} (median ${ok[Math.floor(ok.length / 2)] ?? "?"}, max ${ok.at(-1) ?? "?"})`);
  if (!lat.length) return;
  const hops = ["host.event", "js.event", "js.commitStart", "js.commitSend", "host.commitRecv", "host.applyStart", "host.applyEnd"];
  const med = hops.map((h) => {
    const v = list.map((s) => lat.find((l) => l.tag === h && l.t >= s.t0u && l.t < s.t0u + 1_000_000)).map((l, i) => (l ? (l.t - list[i]!.t0u) / 1000 : NaN)).filter((x) => !Number.isNaN(x)).sort((a, b) => a - b);
    return `${h}=${v.length ? v[Math.floor(v.length / 2)]!.toFixed(1) : "-"}`;
  });
  console.log(`  ${label} hops (median ms after input): ${med.join(" ")}`);
}

async function measure(name: string, p: number, press: (chord: string) => Promise<void>, withLat: boolean, aim?: (chord: string) => Promise<void>, keys = ["Meta+Shift+]", "Meta+Shift+["]): Promise<void> {
  const sessions = await frameLoggers(p);
  console.log(`  ${name}: ${sessions.size} of ${tabs} tabs logging frames`);
  await Bun.sleep(5000);
  const list: Switch[] = [];
  // The tab switched to is whichever is visible afterwards: a browser's tab
  // order need not be the order the pages were opened in.
  const step = async (label: string, chord: string) => {
    const before = await onShow(sessions);
    // The pointer is already over a row it is about to click.
    if (aim) await aim(chord);
    const t0w = Date.now();
    const t0u = nowUs();
    await press(chord);
    await Bun.sleep(1200);
    const now = await onShow(sessions);
    const s = now >= 0 && now !== before ? sessions.get(urls[now]!) : undefined;
    const first = s ? await s.eval<number>(`(__fr.find((t) => t >= ${t0w}) ?? 0)`).catch(() => 0) : 0;
    list.push({ label, t0w, t0u, ms: first ? first - t0w : -1 });
    if (!s) console.log(`  ${label}: on show ${before} -> ${now}`);
    await Bun.sleep(800);
  };
  for (let i = 0; i < rounds; i++) await step(`${name} key pair`, keys[i % 2]!);
  for (let i = 0; i < tabs * 2; i++) await step(`${name} key round`, keys[0]!);
  if (aim) {
    // A real click on the tab's row: "click:<n>" in place of a chord.
    for (let i = 0; i < rounds; i++) await step(`${name} click pair`, `click:${i % 2 === 0 ? 2 : 1}`);
    for (let i = 0; i < tabs * 2; i++) await step(`${name} click round`, `click:${(i % tabs) + 1}`);
  }
  const lat = withLat ? latLines() : [];
  for (const label of [...new Set(list.map((s) => s.label))]) report(label, list.filter((s) => s.label === label), lat);
  for (const s of sessions.values()) s.close();
}

const app = await connectApp();
console.log(`== app (${tabs} tabs)`);
await Bun.sleep(10000);
const slot = await app.getByTestId("view-slot").boundingBox().catch(() => null);
// A click into the page gives the window the keyboard, the way a person
// reading it has it.
if (slot) await app.cursor.click({ x: slot.x + slot.width * 0.7, y: slot.y + slot.height * 0.6 }).catch((e) => console.log(`  click: ${e}`));
for (let i = 0; i < tabs; i++) {
  await app.getByTestId("menu-next-tab").click().catch((e) => console.log(`  next tab: ${e}`));
  await Bun.sleep(1500);
}
const rowAt = async (n: number) => {
  const b = await app.getByTestId(`tab-t${n}`).boundingBox().catch(() => null);
  return b ? { x: b.x + b.width / 2, y: b.y + b.height / 2 } : null;
};
await measure("app", port, async (c) => {
  if (!c.startsWith("click:")) return app.cursor.press(c);
  await app.cursor.down();
  await app.cursor.up();
}, true, async (c) => {
  if (!c.startsWith("click:")) return;
  const at = await rowAt(Number(c.slice(6)));
  if (at) await app.cursor.move(at);
  await Bun.sleep(150);
});

const stock = process.env.ND_SWITCH_STOCK;
if (stock) {
  const stockPort = port + 7;
  const dir = join(work, "stock-profile");
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "First Run"), "");
  const child = Bun.spawn([stock, `--user-data-dir=${dir}`, `--remote-debugging-port=${stockPort}`, "--remote-allow-origins=*", "--no-first-run", "--no-default-browser-check", "--password-store=basic", "--use-mock-keychain", ...urls], { stdout: "ignore", stderr: Bun.file(join(work, "stock.log")) });
  for (let i = 0; i < 100; i++) {
    if (await fetch(`http://127.0.0.1:${stockPort}/json/version`).then(() => true, () => false)) break;
    await Bun.sleep(200);
  }
  await Bun.sleep(5000);
  const name = stock.split("/").pop()!;
  console.log(`== ${name} (${tabs} tabs)`);
  // Its window in front, so the chords reach it.
  Bun.spawnSync(["osascript", "-e", `tell application "System Events" to set frontmost of (first process whose unix id is ${child.pid}) to true`]);
  await Bun.sleep(800);
  for (let i = 0; i < tabs; i++) {
    await app.cursor.press("Control+Tab");
    await Bun.sleep(1200);
  }
  // Ctrl+Tab: a bracket chord depends on the keyboard layout, and on this
  // machine's it does not reach Chromium's tab commands.
  await measure(name, stockPort, (c) => app.cursor.press(c), false, undefined, ["Control+Tab", "Control+Shift+Tab"]);
  child.kill();
  await child.exited;
}
console.log("ND_SWITCH_DONE");
process.exit(0);
