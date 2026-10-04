#!/usr/bin/env bun
// scripts/cef-zoom-drive.ts: drives examples/webview-probe/cef-zoom.tsx under
// Chrome style on either backend (scripts/headless-cef-zoom.sh on Linux,
// scripts/mac/cef-zoom.sh on macOS). Every way a page's zoom can change has to
// reach the app as `zoomChanged`, and none of them may put Chromium's zoom
// bubble on screen: with no location bar to anchor to, it used to land over
// the top centre of the page.
//
//   leg 1  setZoom from the app: the page zooms, `zoomChanged` says source=app,
//          no bubble.
//   leg 2  the zoom chords inside the page (IDC_ZOOM_PLUS / MINUS / NORMAL):
//          the engine serves them itself, source=page, no bubble.
//   leg 3  ctrl+wheel over the page, which Chromium turns into the same
//          commands (Linux; on macOS a pinch is page scale and changes no zoom
//          level).
//
// "No bubble" is asserted from the host's own trace (the engine names every
// bubble it closed, and a surface it placed instead is a failure), the window
// server's census of the host's visible windows and, on Linux, where Views
// paints the bubble inside the view, captures of the view's top right.
import { connectApp, findNode, poll } from "../packages/test/src/index.ts";
import { Session, waitForTarget } from "./cdp.ts";

const mac = process.platform === "darwin";
const pid = Number(process.env.ND_HOST_PID ?? 0);
const log = process.env.ND_HOST_LOG ?? "";
const port = Number(process.env.ND_CDP_PORT ?? process.env.ND_CEF_DEBUG_PORT ?? 9334);
const T = 15000;
const app = await connectApp();

const label = async (id: string) => findNode((await app.tree()).root, id)?.text ?? "";
const logText = async () => (log ? await Bun.file(log).text() : "");

type Win = { x: number; y: number; width: number; height: number };

/// Visible top-level windows of the host small enough to be the zoom bubble.
async function smallWindows(): Promise<Win[]> {
  if (mac) {
    const proc = Bun.spawn(["swift", "scripts/mac/window-census.swift", String(pid)], {
      stdout: "pipe",
      env: { ...process.env, SDKROOT: undefined, DEVELOPER_DIR: undefined },
    });
    const text = await new Response(proc.stdout).text();
    return text
      .split("\n")
      .filter((l) => l.trim().startsWith("{"))
      .map((l) => JSON.parse(l) as Win & { alpha: number; layer: number })
      .filter((w) => w.alpha > 0 && w.layer === 0 && w.height <= 120 && w.width <= 640);
  }
  const tree = Bun.spawnSync(["xwininfo", "-root", "-children"]).stdout.toString();
  const out: Win[] = [];
  for (const m of tree.matchAll(/^\s+(0x[0-9a-f]+) .*?(\d+)x(\d+)\+(-?\d+)\+(-?\d+)/gm)) {
    const [id, w, h, x, y] = [m[1]!, Number(m[2]), Number(m[3]), Number(m[4]), Number(m[5])];
    if (w < 60 || h < 30 || w > 640 || h > 120) continue;
    const info = Bun.spawnSync(["xwininfo", "-id", id]).stdout.toString();
    if (!info.includes("IsViewable")) continue;
    const owner = Bun.spawnSync(["xprop", "-id", id, "_NET_WM_PID"]).stdout.toString();
    if (!owner.includes(`= ${pid}`)) continue;
    out.push({ x, y, width: w, height: h });
  }
  return out;
}

/// Linux only: whether the bubble is painted inside the view. Views draws it
/// into the browser's own X window there, so the window list never shows it;
/// the top right of the (white) page going dark is the tell. The first sample
/// of each step is kept as `cef-zoom-<leg>.png` for the visual check.
async function bubbleInView(keep?: string): Promise<boolean> {
  const view = findNode((await app.tree()).root, "z-view")?.geometry;
  const win = (await app.windows()).windows[0]?.geometry;
  if (!view || !win) return false;
  const dir = process.env.XDG_RUNTIME_DIR ?? "/tmp";
  const shot = `${dir}/cef-zoom-strip.png`;
  Bun.spawnSync(["import", "-window", "root", shot]);
  if (keep) {
    const name = keep.replace(/\+/g, "-plus-").replace(/=/g, "equal").replace(/-$/, "minus").replace(/[^a-z0-9]+/gi, "-");
    Bun.spawnSync(["magick", shot, "-crop", `${win.w}x${win.h}+${win.x}+${win.y}`, `${dir}/cef-zoom-${name}.png`]);
  }
  // Inset from the view's edges so the dark frame around it never counts.
  const crop = `280x30+${win.x + view.x + view.w - 290}+${win.y + view.y + 8}`;
  const min = Bun.spawnSync(["magick", shot, "-crop", crop, "-colorspace", "Gray", "-format", "%[fx:minima]", "info:"]).stdout.toString();
  return Number(min) < 0.5;
}

/// Watches for a bubble for as long as Chrome keeps one up (1.5s), sampling
/// the window list and, on Linux, the view itself, and fails on the first one
/// seen.
async function noBubble(leg: string, since: number): Promise<void> {
  // The bubble is up within a frame or two of the change; the kept capture
  // is taken where it would be fully drawn.
  if (!mac) await Bun.sleep(250);
  const deadline = Date.now() + 1800;
  let first = true;
  while (Date.now() < deadline) {
    if (!mac) {
      if (await bubbleInView(first ? leg : undefined)) throw new Error(`${leg}: Chromium drew its zoom bubble inside the view`);
      first = false;
    }
    const seen = await smallWindows();
    if (seen.length > 0) throw new Error(`${leg}: a bubble-sized window is on screen: ${JSON.stringify(seen)}`);
    await Bun.sleep(100);
  }
  const fresh = (await logText()).slice(since);
  // A surface the window watch placed after a zoom is the bubble drawn over
  // the page, which is the bug.
  if (/chrome surface adopted|chromeDialog node=/.test(fresh)) {
    throw new Error(`${leg}: the host placed a Chromium surface after the zoom:\n${fresh.split("\n").filter((l) => /adopted|chromeDialog/.test(l)).join("\n")}`);
  }
}

async function zoomTo(expect: RegExp, leg: string): Promise<string> {
  return await poll(() => label("z-zoom"), (v) => expect.test(v), { timeoutMs: T }).catch(async () => {
    throw new Error(`${leg}: zoomChanged never reported ${expect} (label: ${await label("z-zoom")})`);
  }) as string;
}

const failures: string[] = [];
const leg = async (name: string, run: () => Promise<void>) => {
  try {
    await run();
  } catch (e) {
    failures.push((e as Error).message);
  }
};

await poll(() => label("z-title"), (v) => v.startsWith("title=dpr="), { timeoutMs: 60000 });
const baseDpr = Number((await label("z-title")).slice("title=dpr=".length));

await leg("setZoom", async () => {
  const since = (await logText()).length;
  await app.getByTestId("z-set150").click();
  const shown = await zoomTo(/^zoom=1\.50\/app changes=1$/, "setZoom");
  await poll(() => label("z-title"), (v) => Math.abs(Number(v.slice(10)) - baseDpr * 1.5) < 0.01, { timeoutMs: T });
  await noBubble("setZoom", since);
  console.log(`  ND_CEF_ZOOM_APP_OK ${shown}, the page scaled and no bubble window came up`);
});

const page = await waitForTarget(port, (t) => t.type === "page" && t.url.startsWith("http://127.0.0.1"));
const cdp = await Session.open(page.webSocketDebuggerUrl!);
const chord = async (key: string, code: string, keyCode: number) => {
  if (mac) {
    // A real keypress: Chromium's accelerator table on macOS answers the key
    // equivalent the window server delivers, not a debugger key event.
    await app.cursor.click(app.getByTestId("z-view"));
    await app.cursor.press(`Meta+${key}`);
    return;
  }
  // Chromium answers an unhandled ctrl+= from the page with IDC_ZOOM_PLUS,
  // exactly as for a real keypress.
  const base = { modifiers: 2, key, code, windowsVirtualKeyCode: keyCode, nativeVirtualKeyCode: keyCode };
  await cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...base });
  await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
};

if (mac) {
  // Chrome on macOS maps the zoom chords through its main menu, which this
  // embedding does not have: a real Cmd+= in the page reaches nothing, and
  // the app's own View menu accelerators are what zoom (nativebrowser
  // scripts/zoom-shots.ts drives that). What has to hold here is that the
  // chord raises nothing of Chromium's either.
  await leg("chords", async () => {
    const since = (await logText()).length;
    await app.cursor.click(app.getByTestId("z-view"));
    await app.cursor.press("Meta+=");
    await Bun.sleep(800);
    const shown = await label("z-zoom");
    if (!/changes=1$/.test(shown)) throw new Error(`Cmd+=: the page zoomed with no app accelerator (${shown})`);
    await noBubble("Cmd+=", since);
    console.log("  ND_CEF_ZOOM_CHORDS_OK a real Cmd+= in the page raises nothing of Chromium's; zoom chords are the app's menu");
  });
} else await leg("chords", async () => {
  let since = (await logText()).length;
  await chord("=", "Equal", 187);
  const up = await zoomTo(/^zoom=1\.75\/page changes=2$/, "ctrl+=");
  await noBubble("ctrl+=", since);
  since = (await logText()).length;
  await chord("-", "Minus", 189);
  await zoomTo(/^zoom=1\.50\/page changes=3$/, "ctrl+-");
  await noBubble("ctrl+-", since);
  since = (await logText()).length;
  await chord("0", "Digit0", 48);
  const reset = await zoomTo(/^zoom=1\.00\/page changes=4$/, "ctrl+0");
  await noBubble("ctrl+0", since);
  console.log(`  ND_CEF_ZOOM_CHORDS_OK ${up}, back down, then ${reset}; the engine served each chord and no bubble window came up`);
});

if (!mac) {
  await leg("wheel", async () => {
    const since = (await logText()).length;
    // A real X wheel click with ctrl held: Chromium turns it into
    // IDC_ZOOM_PLUS through the browser's own wheel handling, which a CDP
    // synthesized wheel does not reach.
    const view = findNode((await app.tree()).root, "z-view")?.geometry;
    const win = (await app.windows()).windows[0]?.geometry;
    if (!view || !win) throw new Error("ctrl+wheel: no geometry for the view");
    const at = [String(win.x + view.x + view.w / 2), String(win.y + view.y + view.h / 2)];
    Bun.spawnSync(["xdotool", "mousemove", ...at, "keydown", "ctrl", "click", "4", "keyup", "ctrl"]);
    const shown = await zoomTo(/\/page changes=5$/, "ctrl+wheel");
    await noBubble("ctrl+wheel", since);
    console.log(`  ND_CEF_ZOOM_WHEEL_OK ${shown} from ctrl+wheel, no bubble window`);
  });
}

await leg("extension fills", async () => {
  // Chrome's password manager is off (the prefs leg), and an extension still
  // fills a login form: that is the owner's setup with 1Password.
  const since = (await logText()).length;
  await cdp.send("Page.navigate", { url: page.url.replace(/\/?$/, "/login") });
  const value = await poll(async () => {
    const r = (await cdp.send("Runtime.evaluate", { expression: "document.querySelector('input[type=password]')?.value ?? ''", returnByValue: true })) as { result: { value: string } };
    return r.result.value;
  }, (v) => v === "filled-by-extension", { timeoutMs: T }).catch(() => {
    throw new Error("extension fills: the extension's content script never filled the password field");
  });
  await cdp.send("Runtime.evaluate", { expression: "document.getElementById('go').click()" });
  await Bun.sleep(1500);
  const fresh = (await logText()).slice(since);
  if (/chrome surface adopted|chromeDialog node=/.test(fresh)) throw new Error("extension fills: a Chromium surface came up after the sign-in");
  if (mac && (await smallWindows()).length > 0) throw new Error("extension fills: a bubble-sized window is on screen after the sign-in");
  console.log(`  ND_CEF_EXTENSION_FILL_OK the extension filled "${value}" and signing in raised no save-password bubble`);
});

await leg("prefs", async () => {
  // The password manager, autofill saving and translate raise their bubbles
  // from the page itself, so the engine switches them off in the profile.
  const names = ["credentials_enable_service", "credentials_enable_autosignin", "autofill.profile_enabled", "autofill.credit_card_enabled", "translate.enabled"];
  const text = await logText();
  const off = names.filter((n) => text.includes(`pref ${n}=false`)).length;
  if (off < names.length) throw new Error(`prefs: only ${off} of the 5 bubble preferences were switched off`);
  console.log(`  ND_CEF_BUBBLE_PREFS_OK the password manager, autofill saving and translate are off`);
});

const closed = (await logText()).split("\n").filter((l) => /zoom bubble closed/.test(l)).length;
if (mac) console.log(`  engine closed ${closed} zoom bubble window(s) before they were placed`);
await app.close();
if (failures.length > 0) {
  console.log(`FAIL: ${failures.join("\n")}`);
  process.exit(1);
}
console.log("ND_CEF_ZOOM_OK zoom reaches the app from every source and Chromium's bubble never comes up");
// The debugger socket would keep the process alive.
process.exit(0);
