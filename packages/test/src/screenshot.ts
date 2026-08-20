// screenshot() with retries + floor checks — subsumes every drive script's
// hand-rolled screenshotRetrying loop (notes-drive, tabs-drive, m6-drive: a
// screenshot right after a mutation/animation can race frame invalidation
// and answer -32603 or a blank frame; poll rather than treat one bad shot as
// final). `via: "ndshot"` shells out to tools/ndshot (macOS/AppKit only) for
// the ScreenCaptureKit capture path documented in automation-socket.md, for
// callers hitting the offscreen-render TextInput/TextArea blanking bug.
import { existsSync } from "node:fs";
import { resolve as resolvePath } from "node:path";
import type { Backend } from "@nativedesktop/host";
import { RPC_ERRORS, type ScreenshotResult } from "@nativedesktop/react/rpc";
import { pngSize } from "./png.ts";
import { AutomationRpcError } from "./socket.ts";

export interface ScreenshotOptions {
  window?: number;
  retries?: number;
  minHeight?: number;
  minBytes?: number;
  via?: "rpc" | "ndshot";
}

export interface ScreenshotDeps {
  callScreenshot: (path: string, window?: number) => Promise<ScreenshotResult>;
  pid: number;
  backend: Backend;
}

const NDSHOT_BIN = resolvePath(import.meta.dir, "..", "..", "..", "tools", "ndshot", "bin", "ndshot");

export async function takeScreenshot(
  deps: ScreenshotDeps,
  path: string,
  opts: ScreenshotOptions = {},
): Promise<ScreenshotResult> {
  const retries = opts.retries ?? 5;
  let lastErr: Error | undefined;
  for (let attempt = 0; attempt < retries; attempt++) {
    try {
      const shot =
        opts.via === "ndshot" ? await captureViaNdshot(deps, path) : await deps.callScreenshot(path, opts.window);
      if (shot.width <= 0 || shot.height <= 0) {
        throw new Error(`screenshot has no dimensions (${shot.width}x${shot.height})`);
      }
      if (opts.minHeight !== undefined && shot.height < opts.minHeight) {
        throw new Error(`screenshot height ${shot.height} below floor ${opts.minHeight}`);
      }
      if (opts.minBytes !== undefined) {
        const size = Bun.file(path).size;
        if (size < opts.minBytes) throw new Error(`screenshot ${size} bytes below floor ${opts.minBytes}`);
      }
      return shot;
    } catch (e) {
      // A closed window is never coming back mid-run, so retrying just
      // burns the retry budget on a guaranteed repeat failure; fail fast
      // with the distinguishable error instead of the generic "N attempts"
      // wrapper the frame-invalidation races below are meant for.
      if (e instanceof AutomationRpcError && e.code === RPC_ERRORS.windowGone.code) throw e;
      lastErr = e as Error;
      if (attempt < retries - 1) await new Promise((r) => setTimeout(r, 150));
    }
  }
  throw new Error(`screenshot failed after ${retries} attempts: ${lastErr?.message}`);
}

async function captureViaNdshot(deps: ScreenshotDeps, path: string): Promise<ScreenshotResult> {
  if (deps.backend !== "appkit") throw new Error(`via: "ndshot" is macOS/AppKit-only (backend=${deps.backend})`);
  if (!existsSync(NDSHOT_BIN)) throw new Error(`ndshot binary missing at ${NDSHOT_BIN} — run tools/ndshot/build.sh`);
  // The bare --pid path captures the FIRST window matching the pid, which is
  // the app's menu-bar strip, not its content window. Resolve the real
  // window (largest on-screen) via `ndshot list` and capture by --window-id.
  const list = Bun.spawnSync([NDSHOT_BIN, "list"]);
  if (list.exitCode !== 0) throw new Error(`ndshot list failed: ${list.stderr.toString().trim()}`);
  const win = list.stdout
    .toString()
    .split("\n")
    .filter((l) => l.trim().startsWith("{"))
    .map((l) => JSON.parse(l) as { pid: number; windowID: number; width: number; height: number; onScreen: boolean })
    .filter((w) => w.pid === deps.pid && w.onScreen && w.height > 100)
    .sort((a, b) => b.width * b.height - a.width * a.height)[0];
  if (!win) throw new Error(`ndshot list found no capturable window for pid ${deps.pid}`);
  const proc = Bun.spawn([NDSHOT_BIN, "capture", "--out", path, "--window-id", String(win.windowID)], {
    stdout: "ignore",
    stderr: "pipe",
  });
  const status = await proc.exited;
  if (status !== 0) {
    const err = await new Response(proc.stderr).text();
    throw new Error(`ndshot capture failed (exit ${status}): ${err.trim()}`);
  }
  const { width, height } = await pngSize(path);
  return { path, width, height };
}
