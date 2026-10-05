#!/usr/bin/env bun
// Input-to-pixels latency for any X11 client, measured from outside it: one
// XTEST event, then the window's pixels (or a new top-level window, for a
// popup) polled until they change. Works the same for the app and for a stock
// Chromium run with --ozone-platform=x11, so the two are comparable.
//
//   bun scripts/latency-x11.ts <pid-or-0> <steps...>
//
// A step is one of:
//   key:<keysym>[+<keysym>...]   e.g. key:Control_L+l, key:a, key:Escape
//   move:<x>,<y>                 pointer to window-relative x,y
//   click:<x>,<y> / rclick:<x>,<y>
//   wait:<ms>
//   region:<x>,<y>,<w>,<h>       where the next measured steps look for change
//   measure:<label>              the following input step is timed under label
//   startup:<x>,<y>,<w>,<h>      wait for the window to map and draw (times from
//                                ND_LAT_START_EPOCH_MS)
//   until:<label>:<x>,<y>,<rrggbb> wait for that window pixel to take the colour
//                                (times from ND_LAT_START_EPOCH_MS)
// Each measured step prints `ND_LAT <label> <ms>` (or `timeout`). Marker at the
// end: ND_LAT_DONE.
import { existsSync } from "node:fs";
import { dlopen, FFIType, JSCallback, ptr, toArrayBuffer, type Pointer } from "bun:ffi";

// NixOS has no global library path: look along the dev shell's (and nix-ld's).
function lib(name: string): string {
  const dirs = [process.env.ND_CEF_LD_LIBRARY_PATH, process.env.LD_LIBRARY_PATH, process.env.NIX_LD_LIBRARY_PATH]
    .flatMap((v) => (v ?? "").split(":"))
    .filter(Boolean);
  for (const d of dirs) if (existsSync(`${d}/${name}`)) return `${d}/${name}`;
  return name;
}

const X = dlopen(lib("libX11.so.6"), {
  XOpenDisplay: { args: [FFIType.ptr], returns: FFIType.ptr },
  XDefaultRootWindow: { args: [FFIType.ptr], returns: FFIType.u64 },
  XGetImage: {
    args: [FFIType.ptr, FFIType.u64, FFIType.i32, FFIType.i32, FFIType.u32, FFIType.u32, FFIType.u64, FFIType.i32],
    returns: FFIType.ptr,
  },
  XFree: { args: [FFIType.ptr], returns: FFIType.i32 },
  XSync: { args: [FFIType.ptr, FFIType.i32], returns: FFIType.i32 },
  XFlush: { args: [FFIType.ptr], returns: FFIType.i32 },
  XStringToKeysym: { args: [FFIType.cstring], returns: FFIType.u64 },
  XKeysymToKeycode: { args: [FFIType.ptr, FFIType.u64], returns: FFIType.u8 },
  XQueryTree: { args: [FFIType.ptr, FFIType.u64, FFIType.ptr, FFIType.ptr, FFIType.ptr, FFIType.ptr], returns: FFIType.i32 },
  XGetWindowAttributes: { args: [FFIType.ptr, FFIType.u64, FFIType.ptr], returns: FFIType.i32 },
  XTranslateCoordinates: {
    args: [FFIType.ptr, FFIType.u64, FFIType.u64, FFIType.i32, FFIType.i32, FFIType.ptr, FFIType.ptr, FFIType.ptr],
    returns: FFIType.i32,
  },
  XInternAtom: { args: [FFIType.ptr, FFIType.cstring, FFIType.i32], returns: FFIType.u64 },
  XGetWindowProperty: {
    args: [FFIType.ptr, FFIType.u64, FFIType.u64, FFIType.i64, FFIType.i64, FFIType.i32, FFIType.u64, FFIType.ptr, FFIType.ptr, FFIType.ptr, FFIType.ptr, FFIType.ptr],
    returns: FFIType.i32,
  },
  XSetErrorHandler: { args: [FFIType.ptr], returns: FFIType.ptr },
});
const T = dlopen(lib("libXtst.so.6"), {
  XTestFakeKeyEvent: { args: [FFIType.ptr, FFIType.u32, FFIType.i32, FFIType.u64], returns: FFIType.i32 },
  XTestFakeMotionEvent: { args: [FFIType.ptr, FFIType.i32, FFIType.i32, FFIType.i32, FFIType.u64], returns: FFIType.i32 },
  XTestFakeButtonEvent: { args: [FFIType.ptr, FFIType.u32, FFIType.i32, FFIType.u64], returns: FFIType.i32 },
});
const x = X.symbols;
const t = T.symbols;
const dpy = x.XOpenDisplay(null);
if (!dpy) throw new Error("no X display");
const root = x.XDefaultRootWindow(dpy);
// A window that unmaps between the query and the grab answers BadMatch, and
// Xlib's default handler exits the process for it.
const onXError = new JSCallback(() => 0, { args: [FFIType.ptr, FFIType.ptr], returns: FFIType.i32 });
x.XSetErrorHandler(onXError.ptr);
const cstr = (s: string) => Buffer.from(`${s}\0`);
const ZPixmap = 2;
const allPlanes = 0xffffffffffffffffn;
const netPid = x.XInternAtom(dpy, cstr("_NET_WM_PID"), 0);

function children(w: bigint): bigint[] {
  const r = new BigUint64Array(1);
  const p = new BigUint64Array(1);
  const list = new BigUint64Array(1);
  const n = new Uint32Array(1);
  if (!x.XQueryTree(dpy, w, ptr(r), ptr(p), ptr(list), ptr(n))) return [];
  if (n[0] === 0 || list[0] === 0n) return [];
  const ids = new BigUint64Array(toArrayBuffer(Number(list[0]) as unknown as Pointer, 0, n[0]! * 8)).slice();
  x.XFree(Number(list[0]) as unknown as Pointer);
  return [...ids];
}

// XWindowAttributes (LP64): x, y, width, height at 0..15; map_state at 92.
function attrs(w: bigint): { x: number; y: number; w: number; h: number; viewable: boolean } | null {
  const buf = new ArrayBuffer(136);
  if (!x.XGetWindowAttributes(dpy, w, ptr(buf))) return null;
  const v = new DataView(buf);
  return { x: v.getInt32(0, true), y: v.getInt32(4, true), w: v.getInt32(8, true), h: v.getInt32(12, true), viewable: v.getInt32(92, true) === 2 };
}

function pidOf(w: bigint): number {
  const type = new BigUint64Array(1);
  const fmt = new Int32Array(1);
  const n = new BigUint64Array(1);
  const after = new BigUint64Array(1);
  const prop = new BigUint64Array(1);
  if (x.XGetWindowProperty(dpy, w, netPid, 0n, 1n, 0, 0n, ptr(type), ptr(fmt), ptr(n), ptr(after), ptr(prop)) !== 0) return 0;
  if (prop[0] === 0n || n[0] === 0n) return 0;
  const pid = Number(new BigUint64Array(toArrayBuffer(Number(prop[0]) as unknown as Pointer, 0, 8))[0]);
  x.XFree(Number(prop[0]) as unknown as Pointer);
  return pid;
}

/** Viewable top-level clients: a reparenting window manager's frames are
 *  looked through to the client window inside. */
function viewableTops(): Map<bigint, { x: number; y: number; w: number; h: number }> {
  const out = new Map<bigint, { x: number; y: number; w: number; h: number }>();
  for (const w of children(root)) {
    const a = attrs(w);
    if (!a?.viewable || a.w <= 1 || a.h <= 1) continue;
    if (pidOf(w)) {
      out.set(w, a);
      continue;
    }
    const inner = children(w).find((c) => pidOf(c) && attrs(c)?.viewable);
    out.set(inner ?? w, inner ? attrs(inner)! : a);
  }
  return out;
}

const wantPid = Number(process.argv[2] ?? "0");
function mainWindow(): bigint | null {
  let best: bigint | null = null;
  let area = 0;
  for (const [w, a] of viewableTops()) {
    if (wantPid && pidOf(w) !== wantPid) continue;
    if (a.w * a.h > area) {
      area = a.w * a.h;
      best = w;
    }
  }
  return best;
}

// XImage: data pointer at 16, bytes_per_line at 44.
function grab(w: bigint, rx: number, ry: number, rw: number, rh: number): bigint | null {
  const img = x.XGetImage(dpy, w, rx, ry, rw, rh, allPlanes, ZPixmap);
  if (!img) return null;
  const head = new DataView(toArrayBuffer(img, 0, 64));
  const data = Number(head.getBigUint64(16, true));
  const bpl = head.getInt32(44, true);
  const bytes = new Uint8Array(toArrayBuffer(data as unknown as Pointer, 0, bpl * rh));
  const h = Bun.hash(bytes);
  x.XFree(data as unknown as Pointer);
  x.XFree(img);
  return BigInt(h);
}

/** Whether a fresh window has drawn anything other than one flat colour. */
function drawn(w: bigint, a: { w: number; h: number }): boolean {
  const rw = Math.min(a.w, 400);
  const rh = Math.min(a.h, 300);
  const img = x.XGetImage(dpy, w, 0, 0, rw, rh, allPlanes, ZPixmap);
  if (!img) return false;
  const head = new DataView(toArrayBuffer(img, 0, 64));
  const data = Number(head.getBigUint64(16, true));
  const bpl = head.getInt32(44, true);
  const px = new Uint32Array(toArrayBuffer(data as unknown as Pointer, 0, bpl * rh));
  let varied = false;
  for (let i = 1; i < px.length; i += 7) if (px[i] !== px[0]) { varied = true; break; }
  x.XFree(data as unknown as Pointer);
  x.XFree(img);
  return varied;
}

/** One pixel as 0xrrggbb. */
function pixel(w: bigint, px: number, py: number): number {
  const img = x.XGetImage(dpy, w, px, py, 1, 1, allPlanes, ZPixmap);
  if (!img) return -1;
  const head = new DataView(toArrayBuffer(img, 0, 64));
  const data = Number(head.getBigUint64(16, true));
  const v = new Uint32Array(toArrayBuffer(data as unknown as Pointer, 0, 4))[0]! & 0xffffff;
  x.XFree(data as unknown as Pointer);
  x.XFree(img);
  return v;
}

function keys(spec: string, press: boolean): void {
  const syms = spec.split("+");
  const codes = syms.map((s) => x.XKeysymToKeycode(dpy, x.XStringToKeysym(cstr(s))));
  if (press) for (const c of codes) t.XTestFakeKeyEvent(dpy, c, 1, 0n);
  else for (const c of codes.reverse()) t.XTestFakeKeyEvent(dpy, c, 0, 0n);
}

function abs(w: bigint, wx: number, wy: number): [number, number] {
  const dx = new Int32Array(1);
  const dy = new Int32Array(1);
  const child = new BigUint64Array(1);
  x.XTranslateCoordinates(dpy, w, root, wx, wy, ptr(dx), ptr(dy), ptr(child));
  return [dx[0]!, dy[0]!];
}

const steps = process.argv.slice(3);
let win: bigint | null = null;
let region = [0, 0, 200, 50];
let label = "";
const timeoutMs = Number(process.env.ND_LAT_TIMEOUT_MS ?? "3000");
const t0Start = Number(process.env.ND_LAT_START_EPOCH_MS ?? "0");

async function timed(fire: () => void): Promise<number | null> {
  const w = win!;
  x.XSync(dpy, 0);
  const before = grab(w, region[0]!, region[1]!, region[2]!, region[3]!);
  const tops = new Set(viewableTops().keys());
  const start = performance.now();
  fire();
  x.XFlush(dpy);
  let popup: { w: bigint; a: { w: number; h: number } } | null = null;
  while (performance.now() - start < timeoutMs) {
    if (popup) {
      if (drawn(popup.w, popup.a)) return performance.now() - start;
    } else {
      const now = grab(w, region[0]!, region[1]!, region[2]!, region[3]!);
      if (now !== null && now !== before) return performance.now() - start;
      for (const [id, a] of viewableTops()) if (!tops.has(id) && id !== w) popup = { w: id, a };
    }
    await Bun.sleep(1);
  }
  return null;
}

for (const step of steps) {
  const [kind, arg = ""] = step.split(/:(.*)/s);
  if (kind === "startup") {
    const r = arg.split(",").map(Number);
    let mapped = 0;
    while (!(win = mainWindow())) await Bun.sleep(5);
    mapped = Date.now();
    region = r;
    // First paint of the region with something other than a flat colour.
    const a = attrs(win)!;
    while (!drawn(win, a)) await Bun.sleep(5);
    const painted = Date.now();
    if (t0Start) console.log(`ND_LAT window_mapped ${mapped - t0Start}\nND_LAT window_drawn ${painted - t0Start}`);
    continue;
  }
  win ??= mainWindow();
  if (!win) throw new Error("no window");
  if (kind === "until") {
    const [what, at, hex] = arg.split(":") as [string, string, string];
    const [px, py] = at.split(",").map(Number) as [number, number];
    const want = Number.parseInt(hex, 16);
    const deadline = Date.now() + 60000;
    let hit = false;
    while (Date.now() < deadline) {
      if (pixel(win, px, py) === want) {
        hit = true;
        break;
      }
      await Bun.sleep(5);
    }
    console.log(`ND_LAT ${what} ${hit ? Date.now() - t0Start : "timeout"}`);
    continue;
  }
  if (kind === "wait") {
    await Bun.sleep(Number(arg));
    continue;
  }
  if (kind === "region") {
    region = arg.split(",").map(Number);
    continue;
  }
  if (kind === "measure") {
    label = arg;
    continue;
  }
  let fire: () => void;
  let after: () => void = () => {};
  if (kind === "key") {
    fire = () => keys(arg, true);
    after = () => {
      keys(arg, false);
      x.XFlush(dpy);
    };
  } else if (kind === "move" || kind === "click" || kind === "rclick") {
    const [wx, wy] = arg.split(",").map(Number) as [number, number];
    const [ax, ay] = abs(win, wx, wy);
    if (kind === "move") fire = () => t.XTestFakeMotionEvent(dpy, -1, ax, ay, 0n);
    else {
      const button = kind === "click" ? 1 : 3;
      t.XTestFakeMotionEvent(dpy, -1, ax, ay, 0n);
      x.XFlush(dpy);
      await Bun.sleep(150);
      fire = () => t.XTestFakeButtonEvent(dpy, button, 1, 0n);
      after = () => {
        t.XTestFakeButtonEvent(dpy, button, 0, 0n);
        x.XFlush(dpy);
      };
    }
  } else throw new Error(`unknown step ${step}`);
  if (label) {
    const ms = await timed(fire);
    console.log(`ND_LAT ${label} ${ms === null ? "timeout" : ms.toFixed(1)}`);
    label = "";
  } else {
    fire();
    x.XFlush(dpy);
  }
  after();
  await Bun.sleep(30);
}
console.log("ND_LAT_DONE");
