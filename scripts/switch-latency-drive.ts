#!/usr/bin/env bun
// Input to pixels for a tab switch, under scripts/headless-app-chrome.sh
// (ND_ACCEPT_DRIVE=scripts/switch-latency-drive.ts, one rig, ND_LAT_TRACE=1).
//
// Every fixture tab is painted one solid colour. A switch is a real XTEST
// event (Ctrl+Tab, or a click on the tab's row), and it is over when the X
// server holds the new tab's colour at a point inside the page: GetImage on the
// toplevel, polled from the moment the event is flushed. That is the frame the
// compositor presents next, so the only thing left out is the compositor's own
// frame. The host and the child print `ND_LAT` lines on the same clock
// (marker.zig, packages/react/src/lat.ts), and each switch is broken down by
// the first of each hop that follows its input.
//
// ND_SWITCH_STOCK=<chromium-like binary> runs the same key switches against
// that browser on the same display afterwards (Helium, Chrome), as an X11
// client like the app. Marker: ND_SWITCH_DONE.
import { readFileSync, mkdirSync, writeFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { CString, FFIType, JSCallback, dlopen, ptr, read } from "bun:ffi";
import { connectApp } from "@nativedesktop/test";
import { Session, targets } from "./cdp.ts";

const port = Number(process.env.ND_CDP_PORT ?? "9555");
const fixture = process.env.ND_ACCEPT_FIXTURE ?? "http://127.0.0.1:9557/";
const hostLog = process.env.ND_ACCEPT_HOST_LOG ?? "";
const hostClass = process.env.ND_ACCEPT_HOST_CLASS ?? "nd-hello";
const work = process.env.ND_ACCEPT_SHOTS ?? "/tmp";
const tabs = Number(process.env.ND_ACCEPT_TABS ?? "6");
const rounds = Number(process.env.ND_SWITCH_ROUNDS ?? "10");
const gapMs = Number(process.env.ND_SWITCH_GAP_MS ?? "1200");
const timeoutMs = 4000;

// ============================================================================
// Xlib through FFI: one connection for the input and the reads, so a switch's
// clock starts on the flush that sends its input.
// ============================================================================

function findLib(name: string): string {
  const dirs = `${process.env.ND_CEF_LD_LIBRARY_PATH ?? ""}:${process.env.LD_LIBRARY_PATH ?? ""}:/run/current-system/sw/lib`.split(":");
  for (const d of dirs) if (d && existsSync(join(d, name))) return join(d, name);
  return name;
}

const X = dlopen(findLib("libX11.so.6"), {
  XOpenDisplay: { args: [FFIType.ptr], returns: FFIType.ptr },
  XGetImage: { args: [FFIType.ptr, FFIType.u64, FFIType.i32, FFIType.i32, FFIType.u32, FFIType.u32, FFIType.u64, FFIType.i32], returns: FFIType.ptr },
  XFree: { args: [FFIType.ptr], returns: FFIType.i32 },
  XFlush: { args: [FFIType.ptr], returns: FFIType.i32 },
  XSync: { args: [FFIType.ptr, FFIType.i32], returns: FFIType.i32 },
  XKeysymToKeycode: { args: [FFIType.ptr, FFIType.u64], returns: FFIType.u8 },
  XSetErrorHandler: { args: [FFIType.ptr], returns: FFIType.ptr },
});
const T = dlopen(findLib("libXtst.so.6"), {
  XTestFakeKeyEvent: { args: [FFIType.ptr, FFIType.u32, FFIType.i32, FFIType.u64], returns: FFIType.i32 },
  XTestFakeButtonEvent: { args: [FFIType.ptr, FFIType.u32, FFIType.i32, FFIType.u64], returns: FFIType.i32 },
  XTestFakeMotionEvent: { args: [FFIType.ptr, FFIType.i32, FFIType.i32, FFIType.i32, FFIType.u64], returns: FFIType.i32 },
});
const C = dlopen("libc.so.6", {
  clock_gettime: { args: [FFIType.i32, FFIType.ptr], returns: FFIType.i32 },
});

// A window that is gone or unmapped answers GetImage with BadMatch, and Xlib's
// default handler exits the process.
const onXError = new JSCallback(() => 0, { args: [FFIType.ptr, FFIType.ptr], returns: FFIType.i32 });
X.symbols.XSetErrorHandler(onXError.ptr);
const dpy = X.symbols.XOpenDisplay(null);
if (!dpy) throw new Error(`no X display ${process.env.DISPLAY}`);

const tsBuf = new BigInt64Array(2);
/** CLOCK_MONOTONIC in µs: the clock of every ND_LAT line. */
function nowUs(): number {
  C.symbols.clock_gettime(1, ptr(tsBuf));
  return Number(tsBuf[0]) * 1e6 + Math.floor(Number(tsBuf[1]) / 1000);
}

const ZPixmap = 2;
const allPlanes = 0xffffffffffffffffn;
/** 0xRRGGBB at (x, y) of window `win`, or -1 when it cannot be read. */
function pixel(win: number, x: number, y: number): number {
  const img = X.symbols.XGetImage(dpy, BigInt(win), x, y, 1, 1, allPlanes, ZPixmap);
  if (!img) return -1;
  const data = read.ptr(img, 16);
  const v = data ? read.u32(data, 0) & 0xffffff : -1;
  if (data) X.symbols.XFree(data);
  X.symbols.XFree(img);
  return v;
}

const keysyms: Record<string, number> = { ctrl: 0xffe3, shift: 0xffe1, Tab: 0xff09 };
for (let d = 1; d <= 9; d++) keysyms[String(d)] = 0x30 + d;
function code(name: string): number {
  return X.symbols.XKeysymToKeycode(dpy, BigInt(keysyms[name] ?? 0));
}

/** Sends a chord ("ctrl+Tab") and returns the time of the flush that sent it. */
function sendChord(chord: string): number {
  const parts = chord.split("+").map(code);
  for (const k of parts) T.symbols.XTestFakeKeyEvent(dpy, k, 1, 0n);
  for (const k of [...parts].reverse()) T.symbols.XTestFakeKeyEvent(dpy, k, 0, 0n);
  const t = nowUs();
  X.symbols.XFlush(dpy);
  return t;
}

function moveTo(x: number, y: number): void {
  T.symbols.XTestFakeMotionEvent(dpy, -1, x, y, 0n);
  X.symbols.XSync(dpy, 0);
}

function sendClick(): number {
  T.symbols.XTestFakeButtonEvent(dpy, 1, 1, 0n);
  T.symbols.XTestFakeButtonEvent(dpy, 1, 0, 0n);
  const t = nowUs();
  X.symbols.XFlush(dpy);
  return t;
}

interface Probe { win: number; x: number; y: number }

/**
 * Polls until `probe` shows `want`. Returns the time it first did, and every
 * colour seen on the way with its time, which is how a blank or stale frame in
 * between shows up.
 */
function waitPixel(probe: Probe, want: number, t0: number, row?: { probe: Probe; was: number }): { at: number; rowAt: number; seen: string[] } {
  const seen: string[] = [];
  let last = -2;
  let at = -1;
  let rowAt = -1;
  while (nowUs() - t0 < timeoutMs * 1000) {
    if (at < 0) {
      const v = pixel(probe.win, probe.x, probe.y);
      const t = nowUs();
      if (v !== last) {
        seen.push(`${hex(v)}@${((t - t0) / 1000).toFixed(1)}`);
        last = v;
      }
      if (v === want) at = t;
    }
    if (row && rowAt < 0 && pixel(row.probe.win, row.probe.x, row.probe.y) !== row.was) rowAt = nowUs();
    if (at > 0 && (!row || rowAt > 0 || nowUs() - t0 > 1000_000)) break;
  }
  return { at, rowAt, seen };
}

const hex = (v: number) => (v < 0 ? "none" : `#${v.toString(16).padStart(6, "0")}`);

// ============================================================================
// Shell and the X window tree
// ============================================================================

function sh(...argv: string[]): string {
  const r = Bun.spawnSync(argv, { stdout: "pipe", stderr: "pipe" });
  return r.stdout.toString().trim();
}

function geom(id: string): { x: number; y: number; w: number; h: number; mapped: boolean } | null {
  const out = sh("xwininfo", "-id", id);
  const num = (label: string) => Number(out.match(new RegExp(`${label}:\\s+(-?\\d+)`))?.[1] ?? NaN);
  const x = num("Absolute upper-left X");
  if (Number.isNaN(x)) return null;
  return { x, y: num("Absolute upper-left Y"), w: num("Width"), h: num("Height"), mapped: out.includes("Map State: IsViewable") };
}

function biggest(ids: string[]): string {
  let best = "";
  let area = 0;
  for (const dec of ids) {
    const id = `0x${Number(dec).toString(16)}`;
    const g = geom(id);
    if (!g || !g.mapped) continue;
    if (g.w * g.h > area) {
      area = g.w * g.h;
      best = id;
    }
  }
  return best;
}

// ============================================================================
// Tab colours
// ============================================================================

// Far apart in every channel, so a half-blended frame never matches.
const palette = [0xd01010, 0x10a020, 0x1030d0, 0xd0c010, 0xc010c0, 0x10c0c0, 0x803010, 0x308050, 0x502090, 0x909010, 0x101060, 0x601010];

/** The fixture URL the rig seeds tab `n` (1-based) with. */
function urlOf(n: number): string {
  return n === 1 ? fixture : n === 2 ? `${fixture}?two` : `${fixture}?tab${n}`;
}

async function paintTabs(p: number, urls: string[]): Promise<number> {
  let painted = 0;
  const pages = (await targets(p)).filter((t) => t.type === "page");
  if (pages.filter((t) => urls.includes(t.url)).length < urls.length) console.log(`  page targets: ${pages.map((t) => t.url).join(" ")}`);
  for (const t of pages) {
    const i = urls.indexOf(t.url);
    if (i < 0) continue;
    const s = await Session.open(t.webSocketDebuggerUrl!).catch(() => null);
    if (!s) continue;
    const colour = hex(palette[i]!);
    await s.eval(`(() => { let d = document.getElementById("__nd_swatch"); if (!d) { d = document.createElement("div"); d.id = "__nd_swatch"; document.documentElement.appendChild(d); } d.style.cssText = "position:fixed;inset:0;z-index:2147483647;background:${colour}"; return true })()`).catch(() => false);
    s.close();
    painted++;
  }
  return painted;
}

// ============================================================================
// Hops
// ============================================================================

interface Lat { t: number; tag: string; rest: string }

function latLines(): Lat[] {
  if (!hostLog) return [];
  const out: Lat[] = [];
  for (const line of readFileSync(hostLog, "utf8").split("\n")) {
    const m = line.match(/ND_LAT (\d+) (\S+) ?(.*)$/);
    if (m) out.push({ t: Number(m[1]), tag: m[2]!, rest: m[3] ?? "" });
  }
  return out.sort((a, b) => a.t - b.t);
}

/// Events around a switch that are not the input itself.
const noise = /name=(\w*(enter|leave|motion|hover|focus|load|title|favicon|progress|security|navigate|available|zoom)\w*)/i;

/** The hops a switch is broken down into, in the order they happen. */
const hops = [
  "cef.accel",
  "cef.accelRun",
  "host.event",
  "js.event",
  "js.handled",
  "js.commitStart",
  "js.commitSend",
  "host.commitRecv",
  "host.applyStart",
  "cef.unmap",
  "cef.map",
  "cef.mapped",
  "host.applyEnd",
  "cef.layout",
] as const;

interface Switch { scenario: string; t0: number; px: number; row?: number; seen: string[] }

function hopOffsets(all: Lat[], s: Switch): Record<string, number> {
  const end = s.px > 0 ? s.px : s.t0 + timeoutMs * 1000;
  const out: Record<string, number> = {};
  for (const h of hops) {
    // The event that matters is the click or the menu activation, not the
    // pointer crossings and focus notes around it.
    const hit = all.find((l) => l.t >= s.t0 && l.t <= end && l.tag === h && !((h === "host.event" || h === "js.event") && noise.test(l.rest)));
    if (hit) out[h] = (hit.t - s.t0) / 1000;
  }
  if (s.row && s.row > 0) out.row = (s.row - s.t0) / 1000;
  if (s.px > 0) out.pixels = (s.px - s.t0) / 1000;
  return out;
}

function median(xs: number[]): number {
  const v = xs.filter((x) => x >= 0).sort((a, b) => a - b);
  return v.length ? v[Math.floor(v.length / 2)]! : NaN;
}

function report(label: string, list: Switch[], all: Lat[]): void {
  const totals = list.map((s) => (s.px > 0 ? (s.px - s.t0) / 1000 : -1));
  console.log(`  ${label} input->pixels ms: ${totals.map((x) => x.toFixed(1)).join(" ")} (median ${median(totals).toFixed(1)}, max ${Math.max(...totals).toFixed(1)})`);
  if (!all.length) return;
  const per = list.map((s) => hopOffsets(all, s));
  const cols = [...hops, "row", "pixels"].filter((h) => per.some((p) => p[h] !== undefined));
  console.log(`  ${label} hops (median ms after input): ${cols.map((h) => `${h}=${median(per.map((p) => p[h] ?? -1)).toFixed(1)}`).join(" ")}`);
  for (let i = 0; i < list.length; i++) {
    const s = list[i]!;
    const end = s.px > 0 ? s.px : s.t0 + timeoutMs * 1000;
    const events = all.filter((l) => l.t >= s.t0 && l.t <= end && l.tag === "host.event").map((l) => l.rest.replace(/^.*name=/, ""));
    const slow = all.filter((l) => l.t >= s.t0 && l.t <= end && l.tag === "host.slowOp").map((l) => l.rest.replace(/ props=$/, ""));
    console.log(`    #${i} ${cols.map((h) => `${h}=${per[i]![h]?.toFixed(1) ?? "-"}`).join(" ")} seen ${s.seen.join(" ")} events ${events.join(",")}${slow.length ? ` slow ${slow.join("; ")}` : ""}`);
  }
}

// ============================================================================
// The app
// ============================================================================

const urls = Array.from({ length: tabs }, (_, i) => urlOf(i + 1));
const all: Switch[] = [];

if (process.env.ND_SWITCH_SKIP_APP !== "1") {
  const app = await connectApp();
  console.log(`== app (${tabs} tabs, layout ${process.env.ND_ACCEPT_LAYOUT ?? "compact"})`);
  await Bun.sleep(Number(process.env.ND_SWITCH_SETTLE_MS ?? "15000"));
  // Every restored tab loads once it has been shown.
  for (let i = 0; i < tabs; i++) {
    await app.getByTestId("menu-next-tab").click().catch((e) => console.log(`  next tab: ${e}`));
    await Bun.sleep(1500);
  }
  console.log(`  painted ${await paintTabs(port, urls)} of ${tabs} tabs`);

  const win = Number(biggest(sh("xdotool", "search", "--classname", hostClass).split("\n").filter(Boolean)));
  const wg = geom(`0x${win.toString(16)}`)!;
  const slot = await app.getByTestId("view-slot").boundingBox().catch(() => null);
  const probe: Probe = slot
    ? { win, x: Math.round(slot.x + slot.width * 0.7), y: Math.round(slot.y + slot.height * 0.6) }
    : { win, x: Math.round(wg.w * 0.7), y: Math.round(wg.h * 0.6) };
  console.log(`  toplevel 0x${win.toString(16)} ${wg.w}x${wg.h}+${wg.x}+${wg.y}, page probe at ${probe.x},${probe.y}`);

  // Which tab is on show, by its colour.
  const onShow = (): number => palette.indexOf(pixel(probe.win, probe.x, probe.y));
  // A click into the page first: the keys then reach the page, the way they do
  // for someone reading it, and travel the engine's accelerator path.
  moveTo(wg.x + probe.x, wg.y + probe.y);
  await Bun.sleep(200);
  sendClick();
  await Bun.sleep(800);
  let cur = onShow();
  console.log(`  on show: tab ${cur + 1} (${hex(pixel(probe.win, probe.x, probe.y))})`);
  if (cur < 0) console.log("  the page probe reads no tab colour; check the capture");

  // A point in the left padding of the row (sidebar) or tab (compact) being
  // switched to, which changes when the app's chrome marks it as the one on
  // show.
  const rowProbe = async (n: number) => {
    const b = await app.getByTestId(`tab-slot-t${n}`).boundingBox().catch(() => null);
    if (!b) return undefined;
    const p: Probe = { win, x: Math.round(b.x + 3), y: Math.round(b.y + b.height / 2) };
    return { probe: p, was: pixel(win, p.x, p.y) };
  };
  const step = async (scenario: string, input: () => number, next: number) => {
    const row = await rowProbe(next + 1);
    const t0 = input();
    const r = waitPixel(probe, palette[next]!, t0, row);
    all.push({ scenario, t0, px: r.at, row: r.rowAt, seen: r.seen });
    cur = onShow();
    await Bun.sleep(gapMs);
  };

  // Between two tabs, each on show a second ago.
  for (let i = 0; i < rounds; i++) {
    const forward = i % 2 === 0;
    await step("app key pair", () => sendChord(forward ? "ctrl+Tab" : "ctrl+shift+Tab"), (cur + (forward ? 1 : tabs - 1)) % tabs);
  }
  // Round every tab: each was last on show `tabs` switches ago, beyond the
  // few kept running.
  for (let i = 0; i < tabs * 2; i++) await step("app key round", () => sendChord("ctrl+Tab"), (cur + 1) % tabs);

  // Clicks on the tab's own row (sidebar) or tab (compact).
  const rowOf = async (n: number) => {
    for (const id of [`tab-t${n}`, `tab-item-t${n}`]) {
      const b = await app.getByTestId(id).boundingBox().catch(() => null);
      if (b) return { x: Math.round(wg.x + b.x + b.width / 2), y: Math.round(wg.y + b.y + b.height / 2) };
    }
    return null;
  };
  for (let i = 0; i < rounds; i++) {
    const next = cur === 0 ? 1 : 0;
    const at = await rowOf(next + 1);
    if (!at) {
      console.log(`  no row for tab ${next + 1}`);
      break;
    }
    moveTo(at.x, at.y);
    await Bun.sleep(150);
    await step("app click pair", sendClick, next);
  }
  for (let i = 0; i < tabs; i++) {
    const next = (cur + 1) % tabs;
    const at = await rowOf(next + 1);
    if (!at) break;
    moveTo(at.x, at.y);
    await Bun.sleep(150);
    await step("app click round", sendClick, next);
  }

  const lat = latLines();
  for (const scenario of [...new Set(all.map((s) => s.scenario))]) report(scenario, all.filter((s) => s.scenario === scenario), lat);
}

// ============================================================================
// A stock browser on the same display
// ============================================================================

const stock = process.env.ND_SWITCH_STOCK;
if (stock) {
  const stockPort = port + 7;
  const dir = join(work, "stock-profile");
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "First Run"), "");
  const child = Bun.spawn([
    stock,
    `--user-data-dir=${dir}`,
    `--remote-debugging-port=${stockPort}`,
    "--remote-allow-origins=*",
    "--no-first-run",
    "--no-default-browser-check",
    "--ozone-platform=x11",
    "--password-store=basic",
    ...(process.env.ND_SWITCH_STOCK_ARGS ?? "").split(" ").filter(Boolean),
    ...urls,
  ], { stdout: "ignore", stderr: Bun.file(join(work, "stock.log")) });
  for (let i = 0; i < 100; i++) {
    if (await fetch(`http://127.0.0.1:${stockPort}/json/version`).then(() => true, () => false)) break;
    await Bun.sleep(200);
  }
  const name = stock.split("/").pop()!;
  console.log(`== ${name} (${tabs} tabs)`);
  await Bun.sleep(8000);
  const ids = sh("xdotool", "search", "--pid", String(child.pid)).split("\n").filter(Boolean);
  const win = Number(biggest(ids.length ? ids : sh("xdotool", "search", "--classname", name).split("\n").filter(Boolean)));
  const wg = geom(`0x${win.toString(16)}`);
  if (!win || !wg) {
    console.log(`  no ${name} window`);
  } else {
    // Walk the tabs once so each has drawn, the way the app's were.
    moveTo(wg.x + Math.round(wg.w * 0.7), wg.y + Math.round(wg.h * 0.6));
    await Bun.sleep(200);
    sendClick();
    await Bun.sleep(500);
    for (let i = 0; i < tabs; i++) {
      sendChord("ctrl+Tab");
      await Bun.sleep(1200);
    }
    console.log(`  painted ${await paintTabs(stockPort, urls)} of ${tabs} tabs`);
    await Bun.sleep(1500);
    const probe: Probe = { win, x: Math.round(wg.w * 0.7), y: Math.round(wg.h * 0.6) };
    console.log(`  toplevel 0x${win.toString(16)} ${wg.w}x${wg.h}+${wg.x}+${wg.y}, page probe at ${probe.x},${probe.y}`);
    const onShow = () => palette.indexOf(pixel(probe.win, probe.x, probe.y));
    let cur = onShow();
    console.log(`  on show: tab ${cur + 1}`);
    const list: Switch[] = [];
    const step = async (scenario: string, chord: string, next: number) => {
      const t0 = sendChord(chord);
      const r = waitPixel(probe, palette[next]!, t0);
      list.push({ scenario, t0, px: r.at, seen: r.seen });
      cur = onShow();
      await Bun.sleep(gapMs);
    };
    for (let i = 0; i < rounds; i++) {
      const forward = i % 2 === 0;
      await step(`${name} key pair`, forward ? "ctrl+Tab" : "ctrl+shift+Tab", (cur + (forward ? 1 : tabs - 1)) % tabs);
    }
    for (let i = 0; i < tabs * 2; i++) await step(`${name} key round`, "ctrl+Tab", (cur + 1) % tabs);
    for (let i = 0; i < Math.min(tabs, 8); i++) {
      const next = (cur + 1) % tabs;
      await step(`${name} ctrl+digit`, `ctrl+${next + 1}`, next);
    }
    for (const scenario of [...new Set(list.map((s) => s.scenario))]) report(scenario, list.filter((s) => s.scenario === scenario), []);
  }
  child.kill();
  await child.exited;
}

console.log("ND_SWITCH_DONE");
process.exit(0);
