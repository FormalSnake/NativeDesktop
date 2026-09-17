#!/usr/bin/env bun
// Drives examples/webview-probe/cef-chrome.tsx and asserts what Chrome style
// has to hold on macOS: the page is embedded in the host's own view, Chromium's
// extension runtime is live, DevTools docks inside that view, and no window
// the engine created is ever on screen.
//
// The window census is the load-bearing assertion and it comes from the window
// server (scripts/mac/window-census.swift), not from the app: a Chromium window
// the app never hears about still shows up there. The rule is that every
// on-screen window of this process is either one of the app's own windows or an
// anchor at alpha 0.
import { connectApp } from "@nativedesktop/test";

const pid = process.env.ND_HOST_PID ?? "";
const debugPort = process.env.ND_CEF_DEBUG_PORT ?? "9334";
const app = await connectApp();

const failures: string[] = [];
const check = (name: string, ok: boolean, detail: string) => {
  if (ok) console.log(`ND_CEF_CHROME_CHECK ${name}: ok (${detail})`);
  else failures.push(`${name}: ${detail}`);
};

type Window = { alpha: number; width: number; height: number; x: number; y: number; layer: number };

async function census(): Promise<Window[]> {
  const proc = Bun.spawn(["swift", "scripts/mac/window-census.swift", pid], {
    stdout: "pipe",
    env: { ...process.env, SDKROOT: undefined },
  });
  const text = await new Response(proc.stdout).text();
  return text
    .split("\n")
    .filter((line) => line.trim().length > 0)
    .map((line) => JSON.parse(line) as Window);
}

/// Visible windows the app does not own. An anchor is invisible by construction
/// (alpha 0), so anything else at alpha > 0 beyond the app's own count is a
/// window the engine put on screen.
async function strays(leg: string): Promise<void> {
  const [windows, own] = await Promise.all([census(), app.windows()]);
  // Layer 0 is the ordinary window layer. A menu or a tooltip sits above it and
  // is not a browser window, which is what this is looking for.
  const visible = windows.filter((w) => w.alpha > 0 && w.layer === 0);
  const anchors = windows.filter((w) => w.alpha === 0);
  check(
    `census/${leg}`,
    visible.length === own.windows.length,
    `${visible.length} visible, app owns ${own.windows.length}, ${anchors.length} anchor(s)`,
  );
}

async function targets(): Promise<{ type: string; url: string }[]> {
  const response = await fetch(`http://127.0.0.1:${debugPort}/json/list`);
  return (await response.json()) as { type: string; url: string }[];
}

async function evaluate(code: string): Promise<string | null> {
  const result = await app.rpc.call("webviewEval", { testId: "c-view", code });
  return result.ok ? (result.value ?? null) : null;
}

await app.waitForText("title=painted", { timeoutMs: 60000 });
const painted = (await app.getByTestId("c-title").textContent()) ?? "";
check("paint", /vp=\d+x\d+/.test(painted), painted);
await strays("load");

// The whole reason Chrome style exists here: Alloy has no extension runtime, so
// neither the service worker target nor the content script's marker can appear.
const list = await targets();
const worker = list.find((t) => t.type === "service_worker" && t.url.startsWith("chrome-extension://"));
check("extensionServiceWorker", worker !== undefined, worker?.url ?? JSON.stringify(list.map((t) => t.type)));
check("extensionContentScript", (await evaluate("document.documentElement.dataset.ndExtension")) === "live", "content script marker");

// Clicking into the page must not take the host window out of its active look:
// the anchor is a window of its own and a key one would grey the title bar.
await app.getByTestId("c-view").click();
await Bun.sleep(400);
const after = await app.windows();
check("hostStaysKey", after.windows[0]?.key === true, JSON.stringify(after.windows[0]));
check("hostStaysMain", after.windows[0]?.main === true, JSON.stringify(after.windows[0]));

// A popup is the app's `newWindow` event and never a window.
await evaluate("document.getElementById('blank').click()");
await app.waitForText("popup=https://", { timeoutMs: 15000 });
await Bun.sleep(500);
await strays("popup");

// Chrome's own new-window and new-tab accelerators reach Chromium's command
// handling under Chrome style; cef_command_handler_t is what refuses them.
await app.keyboard.press("Meta+n");
await app.keyboard.press("Meta+t");
await app.keyboard.press("Meta+Shift+n");
await Bun.sleep(800);
await strays("chromeAccelerators");

// DevTools docks in the page's own window: the page's viewport shrinks by the
// dock's share and no devtools window appears. Skipped in the run that asserts
// a clean quit, because quitting with the dock open still crashes Chromium's
// shutdown (scripts/mac/cef-chrome-style.sh).
if (process.env.ND_CEF_CHROME_SKIP_DEVTOOLS !== "1") {
  const before = await evaluate("innerWidth");
  await app.getByTestId("c-devtools-open").click();
  await Bun.sleep(3000);
  const docked = await evaluate("innerWidth");
  check(
    "devtoolsDocked",
    before !== null && docked !== null && Number(docked) < Number(before),
    `viewport ${before} -> ${docked}`,
  );
  const devtoolsTarget = (await targets()).find((t) => t.url.startsWith("devtools://"));
  check("devtoolsTarget", devtoolsTarget !== undefined, devtoolsTarget?.url.slice(0, 60) ?? "none");
  await strays("devtools");
}

// A real click into the page followed by real keystrokes. The web contents is
// an ordinary subview now, so both travel the host window's own event and
// responder chain and never pass through a window of Chromium's.
await evaluate("document.getElementById('text').value = ''");
const field = JSON.parse(
  (await evaluate("JSON.stringify(document.getElementById('text').getBoundingClientRect())")) ?? "{}",
);
const viewBox = await app.getByTestId("c-view").boundingBox();
await app.mouse.click(
  (viewBox?.x ?? 0) + field.x + 20,
  (viewBox?.y ?? 0) + field.y + field.height / 2,
);
await Bun.sleep(300);
await app.keyboard.type("abc");
await Bun.sleep(600);
const typed = await evaluate("document.getElementById('text').value");
check("pointerAndKeyboard", typed === "abc", `field reads ${JSON.stringify(typed)}`);

// Chromium's own context menu, with the app's items merged into the model while
// on_before_context_menu is still on the stack. The RPC dismisses the menu.
await app.getByTestId("c-view").rightClick();
await Bun.sleep(900);
await strays("contextMenu");

// The host window resizing is the case that matters: the lifted subtree
// autoresizes with no work at all, and the anchor has to land on the webview's
// new rectangle because Chromium places its popups and bubbles against it.
await app.setWindowSize(820, 620);
await Bun.sleep(900);
const geometry = await app.getByTestId("c-view").boundingBox();
const anchors = (await census()).filter((w) => w.alpha === 0);
check(
  "anchorTracksResize",
  anchors.some((a) => Math.abs(a.width - (geometry?.width ?? -1)) <= 1),
  `webview ${geometry?.width}x${geometry?.height}, anchors ${anchors.map((a) => `${a.width}x${a.height}`).join(",")}`,
);
await strays("resize");

if (failures.length > 0) {
  for (const failure of failures) console.error(`ND_CEF_CHROME_FAIL ${failure}`);
  process.exit(1);
}
console.log("ND_CEF_CHROME_OK");
process.exit(0);
