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
  const shot = Bun.spawnSync(args);
  if (shot.exitCode !== 0) {
    throw new LegFailure(`ndshot capture ${name} failed (${shot.exitCode}): ${shot.stderr.toString().trim()}`);
  }
  return out;
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
