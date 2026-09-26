// Shared by scripts/mac/app-chrome-drive.ts: the leg runner, the window
// census, page evaluation, window captures and the fixture server the real
// browser app is pointed at.
import type { AttachedApp } from "@nativedesktop/test";

export const HOST_PID = Number(process.env.ND_HOST_PID ?? "0");
export const FIXTURE_PORT = Number(process.env.ND_APP_FIXTURE_PORT ?? "9723");
export const FIXTURE_ORIGIN = `http://127.0.0.1:${FIXTURE_PORT}`;
export const SHOTS = process.env.ND_APP_SHOTS ?? "/tmp/nd-app-chrome-shots";

/// ScreenCaptureKit through the signed `ndshot` binary, which is the only
/// capture path an agent session can use: `screencapture` runs as the terminal
/// and the terminal cannot be granted Screen Recording. The default is this
/// checkout's own copy; ND_NDSHOT points at whichever copy holds the grant.
export const NDSHOT = process.env.ND_NDSHOT ?? "tools/ndshot/bin/ndshot";

export interface CensusWindow {
  number: number;
  layer: number;
  alpha: number;
  title: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Leg {
  name: string;
  run: (app: AttachedApp) => Promise<void>;
}

export class LegFailure extends Error {}

/// A leg the machine cannot run at all, as opposed to one that ran and was
/// wrong. Screen Recording is the only such gate here: it is granted to a
/// binary by the machine's owner through System Settings and no script can
/// obtain it. A leg that never ran is reported as skipped, never as passed.
export class LegSkipped extends Error {}

export function assert(ok: boolean, detail: string): void {
  if (!ok) throw new LegFailure(detail);
}

export async function census(): Promise<CensusWindow[]> {
  const proc = Bun.spawn(["swift", "scripts/mac/window-census.swift", String(HOST_PID)], {
    stdout: "pipe",
    stderr: "pipe",
    env: { ...process.env, SDKROOT: undefined, DEVELOPER_DIR: undefined },
  });
  const text = await new Response(proc.stdout).text();
  return text
    .split("\n")
    .filter((line) => line.trim().startsWith("{"))
    .map((line) => JSON.parse(line) as CensusWindow);
}

/// The invariant every leg ends on: each on-screen ordinary-layer window of
/// this process is one the app owns, and every window the engine made is an
/// anchor at alpha 0. Layers above 0 are menus and tooltips, not browser
/// windows.
export async function censusHolds(app: AttachedApp, leg: string): Promise<void> {
  const [windows, own] = await Promise.all([census(), app.windows()]);
  const visible = windows.filter((w) => w.alpha > 0 && w.layer === 0);
  const frames = own.windows.flatMap((w) => (w.geometry ? [w.geometry] : []));
  const stray = visible.filter((w) => !frames.some((f) => coversWindow(f, w)) && !isSurface(w, frames));
  assert(
    stray.length === 0,
    `census/${leg}: ${stray.length} window(s) the app does not own` +
      ` (${stray.map((w) => `${w.width}x${w.height}@${w.x},${w.y}`).join(" ")};` +
      ` app owns ${frames.map((f) => `${f.w}x${f.h}@${f.x},${f.y}`).join(" ")})`,
  );
}

function coversWindow(frame: { x: number; y: number; w: number; h: number }, w: CensusWindow): boolean {
  return Math.abs(frame.x - w.x) <= 1 && Math.abs(frame.y - w.y) <= 1
    && Math.abs(frame.w - w.width) <= 1 && Math.abs(frame.h - w.height) <= 1;
}

/// Chromium draws its menus, tooltips, link-status and download bubbles and
/// `<select>` popups as ordinary-layer windows of this process, anchored on the
/// widget. They are inside one of the app's own windows and small; a browser
/// window Chromium opened for itself is neither, which is what the census is
/// there to catch.
function isSurface(w: CensusWindow, frames: { x: number; y: number; w: number; h: number }[]): boolean {
  return frames.some(
    (f) =>
      w.x >= f.x - 8 && w.y >= f.y - 8 && w.x + w.width <= f.x + f.w + 8 && w.y + w.height <= f.y + f.h + 8 &&
      w.width * w.height < f.w * f.h * 0.6,
  );
}

/// Chromium's own transient surfaces on screen right now.
export async function surfaces(app: AttachedApp): Promise<CensusWindow[]> {
  const [windows, own] = await Promise.all([census(), app.windows()]);
  const frames = own.windows.flatMap((w) => (w.geometry ? [w.geometry] : []));
  return windows.filter((w) => w.alpha > 0 && w.layer >= 0 && !frames.some((f) => coversWindow(f, w)));
}

/// The engine's context menu as the window server sees it. Deliberately census
/// only: an NSMenu blocks the host's main thread for as long as it is tracking,
/// so every check taken while one is open has to stay out of the automation
/// socket, which cannot answer until the menu closes.
export async function menuWindows(): Promise<CensusWindow[]> {
  return (await census()).filter((w) => w.alpha > 0 && w.layer >= 100);
}

export interface ShownMenuItem {
  depth: number;
  index: number;
  id: number;
  kind: string;
  enabled: boolean;
  checked: boolean;
  label: string;
}

/// What the host drew for the most recent menu, read off its own trace. AppKit
/// publishes no accessibility element for a contextual menu, so this is the only
/// handle on the items; the peer of the Linux gate's `ND_CEF menuShown`.
/// How many menus the host has started tracking, from its own trace.
export function trackedMenus(): number {
  const path = process.env.ND_APP_HOST_LOG;
  if (!path) return 0;
  return Bun.spawnSync(["rg", "-c", "chrome menuTracking$", path]).stdout.toString().trim().split("\n")
    .reduce((sum, line) => sum + (Number(line) || 0), 0);
}

export function shownMenu(): ShownMenuItem[] {
  const path = process.env.ND_APP_HOST_LOG;
  if (!path) return [];
  const text = Bun.spawnSync(["rg", "-N", "chrome menuShown ", path]).stdout.toString();
  const lines = text.split("\n").filter((line) => line.includes("chrome menuShown "));
  // One open menu is traced as a run of lines; the last run is the live one,
  // which starts at the last depth=0 index=0.
  const start = lines.findLastIndex((line) => line.includes("depth=0 index=0 "));
  return (start < 0 ? lines : lines.slice(start)).map((line) => {
    const fields = /depth=(\d+) index=(\d+) id=(-?\d+) kind=(\w+) enabled=(\d) checked=(\d) label=(.*)$/.exec(line)!;
    return {
      depth: Number(fields[1]),
      index: Number(fields[2]),
      id: Number(fields[3]),
      kind: fields[4]!,
      enabled: fields[5] === "1",
      checked: fields[6] === "1",
      label: fields[7]!,
    };
  });
}

/// Real key events through the window server, which is what an NSMenu's own
/// tracking loop reads. An NSEvent posted into the app's queue is not.
export function systemKey(code: number, times = 1): void {
  for (let i = 0; i < times; i++) {
    const run = Bun.spawnSync(["osascript", "-e", `tell application "System Events" to key code ${code}`]);
    if (run.exitCode !== 0) throw new LegFailure(`key code ${code} failed: ${run.stderr.toString().trim()}`);
  }
}

export const KEY_DOWN_ARROW = 125;
export const KEY_RIGHT_ARROW = 124;
export const KEY_RETURN = 36;
export const KEY_ESCAPE = 53;

/// How many Down presses reach `label` from the top of the open menu. NSMenu
/// navigation skips separators and disabled items, so the count is over what it
/// would actually stop on.
export function menuStopsTo(items: ShownMenuItem[], label: string): number {
  const top = items.filter((item) => item.depth === 0);
  const target = top.findIndex((item) => item.label.toLowerCase() === label.toLowerCase());
  if (target < 0) {
    throw new LegFailure(`the menu has no "${label}" (${top.map((i) => i.label).join(" | ")})`);
  }
  return top.slice(0, target + 1).filter((item) => item.kind !== "separator" && item.enabled).length;
}

export async function anchors(): Promise<CensusWindow[]> {
  return (await census()).filter((w) => w.alpha === 0);
}

/// The app's own window as the window server sees it, largest first, so the
/// CoreGraphics coordinates the wheel helper needs can be derived from it.
export async function appWindowRect(): Promise<CensusWindow> {
  const visible = (await census())
    .filter((w) => w.alpha > 0 && w.layer === 0)
    .sort((a, b) => b.width * b.height - a.width * a.height);
  assert(visible.length > 0, "the app has no on-screen window");
  return visible[0]!;
}

/// Brings the host forward. A click the window server routes goes to whatever
/// window is topmost under the cursor, so every real-pointer leg needs the app
/// in front first; the machine this runs on has other windows on it.
export function activateApp(): void {
  Bun.spawnSync([
    "osascript", "-e",
    `tell application "System Events" to set frontmost of (first process whose unix id is ${HOST_PID}) to true`,
  ]);
}

/// Foreign windows sitting in front of the app's own at one point, front first.
/// A real click lands on the first of these rather than on the app, and a leg
/// that never sees its click has to name that instead of blaming the feature.
export function obstructions(x: number, y: number): string[] {
  const run = Bun.spawnSync(
    ["swift", "scripts/mac/window-stack.swift", String(Math.round(x)), String(Math.round(y))],
    { env: { ...process.env, SDKROOT: undefined, DEVELOPER_DIR: undefined } },
  );
  const stack = run.stdout
    .toString()
    .split("\n")
    .filter((line) => line.trim().startsWith("{"))
    .map((line) => JSON.parse(line) as { owner: string; pid: number; layer: number });
  const ours = stack.findIndex((w) => w.pid === HOST_PID);
  if (ours < 0) return stack.map((w) => `${w.owner}(layer ${w.layer})`);
  // The window server's own cursor and shield windows sit at the extreme
  // layers and take no clicks, and Notification Center's full-screen backdrop
  // is always in front and always click-through. What is left is the ordinary,
  // floating and modal-panel layers, which do take one.
  return stack
    .slice(0, ours)
    .filter((w) => w.pid !== HOST_PID && w.layer >= 0 && w.layer <= 101 && w.owner !== "Notification Center")
    .map((w) => `${w.owner}(layer ${w.layer})`);
}

/// The host process still being there. A tracking area whose owner does not
/// answer `mouseEntered:` raises out of the run loop and takes the process with
/// it, so a pointer leg that "passes" on a dead host is a leg that lied.
export function hostAlive(): boolean {
  try {
    process.kill(HOST_PID, 0);
    return true;
  } catch {
    return false;
  }
}

export async function pageEval(app: AttachedApp, testId: string, code: string): Promise<string | null> {
  const result = (await app.rpc.call("webviewEval", { testId, code, timeoutMs: 8000 })) as {
    ok: boolean;
    value?: string;
    error?: string;
  };
  if (!result.ok) throw new LegFailure(`webviewEval(${testId}) failed: ${result.error ?? "no reason"}`);
  return result.value ?? null;
}

export async function pageNumber(app: AttachedApp, testId: string, code: string): Promise<number> {
  const value = await pageEval(app, testId, code);
  const n = Number(value);
  assert(Number.isFinite(n), `${code} read ${JSON.stringify(value)}, not a number`);
  return n;
}

/// Polls until `check` passes, so no leg carries a fixed sleep as its timing
/// assumption. The last value seen is what the failure reports.
export async function until<T>(
  what: string,
  read: () => Promise<T>,
  check: (value: T) => boolean,
  timeoutMs = 10000,
): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  let last: T | undefined;
  let error: unknown;
  for (;;) {
    try {
      last = await read();
      error = undefined;
      if (check(last)) return last;
    } catch (e) {
      error = e;
    }
    if (Date.now() >= deadline) {
      const seen = error ? `error ${String(error)}` : JSON.stringify(last);
      throw new LegFailure(`${what}: still ${seen} after ${timeoutMs}ms`);
    }
    await Bun.sleep(100);
  }
}

export function capture(name: string, windowNumber?: number): string {
  const out = `${SHOTS}/${name}.png`;
  const args = [NDSHOT, "capture", "--out", out];
  if (windowNumber !== undefined) args.push("--window-id", String(windowNumber));
  else args.push("--pid", String(HOST_PID));
  // ndshot can hang inside ScreenCaptureKit; a leg that waits on it forever
  // takes the whole run and the machine's gate lock with it.
  const shot = Bun.spawnSync(args, { timeout: 30_000 });
  if (shot.exitCode === 0) return out;
  // ndshot's exit 2 is "no Screen Recording access for this binary", which is
  // the machine owner's to grant. `screencapture -l` is the one other path to a
  // window's pixels, and it needs the same grant for the calling terminal, so
  // it is tried before the leg gives up.
  if (shot.exitCode === 2) {
    if (windowNumber !== undefined) {
      const fallback = Bun.spawnSync(["screencapture", "-x", "-o", "-l", String(windowNumber), out]);
      if (fallback.exitCode === 0) return out;
    }
    throw new LegSkipped("no screen recording permission");
  }
  throw new LegFailure(`ndshot capture ${name} failed (${shot.exitCode}): ${shot.stderr.toString().trim()}`);
}

export interface PngReport {
  width: number;
  height: number;
  rect: { x: number; y: number; w: number; h: number };
  mean: [number, number, number];
  bands: [number, number, number][];
}

export function probePng(path: string, rect?: { x: number; y: number; w: number; h: number }): PngReport {
  const args = ["swift", "scripts/mac/png-probe.swift", path];
  if (rect) args.push(String(rect.x), String(rect.y), String(rect.w), String(rect.h));
  const proc = Bun.spawnSync(args, { env: { ...process.env, SDKROOT: undefined, DEVELOPER_DIR: undefined } });
  if (proc.exitCode !== 0) throw new LegFailure(`png-probe failed: ${proc.stderr.toString().trim()}`);
  return JSON.parse(proc.stdout.toString()) as PngReport;
}

/// How far a colour is from the fixture's fill, summed over the channels. The
/// capture goes through a display colour space, so an exact match is not
/// available and the threshold is on distance.
export function colourDistance(a: [number, number, number], b: [number, number, number]): number {
  return Math.abs(a[0] - b[0]) + Math.abs(a[1] - b[1]) + Math.abs(a[2] - b[2]);
}

export const FIXTURE_FILL: [number, number, number] = [0, 96, 208];

export function startFixtureServer(): { stop: () => void } {
  const root = `${import.meta.dir}/app-chrome-fixtures`;
  const server = Bun.serve({
    port: FIXTURE_PORT,
    hostname: "127.0.0.1",
    async fetch(request) {
      const path = new URL(request.url).pathname;
      if (path === "/download.bin") {
        return new Response("nd fixture download payload", {
          headers: {
            "content-type": "application/octet-stream",
            "content-disposition": 'attachment; filename="nd-fixture.bin"',
          },
        });
      }
      const name = path === "/" ? "/index.html" : path;
      const file = Bun.file(`${root}${name}`);
      if (!(await file.exists())) return new Response("not found", { status: 404 });
      return new Response(file);
    },
  });
  return { stop: () => server.stop(true) };
}
