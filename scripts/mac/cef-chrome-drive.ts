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
import { Session, clickDevToolsClose } from "../cdp.ts";
import { KEY_DOWN_ARROW, KEY_ESCAPE, KEY_RETURN, activateApp, menuStopsTo, menuWindows, shownMenu, systemKey, until } from "./app-chrome-lib";

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

async function targets(): Promise<{ type: string; url: string; webSocketDebuggerUrl?: string }[]> {
  const response = await fetch(`http://127.0.0.1:${debugPort}/json/list`);
  return (await response.json()) as { type: string; url: string; webSocketDebuggerUrl?: string }[];
}

/// The page and the inspector share the webview: whatever each was laid out
/// at has to add up to it, and a pair that does not is the strip of bare
/// background a docked inspector leaves when one of the two is not filling its
/// half.
async function checkTiling(phase: string): Promise<void> {
  let detail = "never measured";
  // Polled: the box layout and the two documents settle on their own schedule
  // after a dock or a resize, so the assertion is about where they come to
  // rest rather than about the first frame after the event.
  for (let attempt = 0; attempt < 12; attempt++) {
    const box = await app.getByTestId("c-view").boundingBox();
    const pageWidth = Number(await evaluate("innerWidth"));
    const devtoolsTarget = (await targets()).find((t) => t.url.startsWith("devtools://"));
    let toolsWidth = -1;
    if (devtoolsTarget?.webSocketDebuggerUrl) {
      const session = await Session.open(devtoolsTarget.webSocketDebuggerUrl);
      toolsWidth = Number(await session.eval<number>("innerWidth"));
      session.close();
    }
    const view = Math.round(box?.width ?? -1);
    detail = `page ${pageWidth} + inspector ${toolsWidth} = ${pageWidth + toolsWidth}, webview ${view}`;
    if (pageWidth > 0 && toolsWidth > 0 && Math.abs(pageWidth + toolsWidth - view) <= 1) {
      check(`dockTiling/${phase}`, true, detail);
      return;
    }
    await Bun.sleep(1000);
  }
  check(`dockTiling/${phase}`, false, detail);
}

/// How many times the host has reported the dock going away. The count, not
/// the presence: the same trace line fires for every close, so only a fresh
/// one says the button that was just clicked is what closed it.
async function devToolsClosedTraces(): Promise<number> {
  const path = process.env.ND_HOST_LOG ?? "";
  if (!path) return 0;
  const text = await Bun.file(path).text().catch(() => "");
  return text.split("\n").filter((line) => line.includes("chrome devtools closed")).length;
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

  // The inspector's own close button. It is drawn only when the frontend was
  // told it can dock, and clicking it has to take the dock down through the
  // host rather than leaving the toggle pointing at an inspector that is
  // already gone.
  check("devtoolsCanDock", devtoolsTarget?.url.includes("can_dock=true") === true, devtoolsTarget?.url.slice(-48) ?? "none");

  // The page and the inspector have to tile the webview with no seam. Their
  // halves are two CEF views in one box layout here rather than two X windows,
  // so what each was actually laid out at is what each document reports, and
  // the two have to add up to the view they share. The resize case rides the
  // window resize at the end of this drive, which already runs with the dock
  // up; resizing here as well only churns Chromium's own widgets.
  await checkTiling("open");

  // Taken with the dock up, before the close below: Chromium shows its status
  // bubble for a few seconds after a docked inspector goes away, on the app's
  // own toggle as much as on the close button, and it is a window of its own.
  await strays("devtools");
  const closedBefore = await devToolsClosedTraces();
  const frontend = await Session.open(devtoolsTarget?.webSocketDebuggerUrl ?? "");
  await frontend.send("Runtime.enable");
  const closeBox = await clickDevToolsClose(frontend);
  check(
    "devtoolsCloseButton",
    closeBox !== null,
    closeBox ? `${Math.round(closeBox.width)}x${Math.round(closeBox.height)} at ${Math.round(closeBox.x)}` : "no close control in the toolbar",
  );
  frontend.close();
  await Bun.sleep(3000);
  check(
    "devtoolsClosedByButton",
    (await targets()).filter((t) => t.url.startsWith("devtools://")).length === 0,
    "no devtools:// target left",
  );
  check("devtoolsCloseReported", (await devToolsClosedTraces()) > closedBefore, `chrome devtools closed ${closedBefore} -> after`);

  // The dock goes back up, so the rest of the drive runs against the state the
  // legs below were written for, and the app's own toggle proves it did not go
  // stale when the frontend closed itself.
  await app.getByTestId("c-devtools-open").click();
  await Bun.sleep(4000);
  check(
    "devtoolsToggleNotStale",
    (await targets()).filter((t) => t.url.startsWith("devtools://")).length === 1,
    "the app's toggle reopened the inspector",
  );
  await checkTiling("reopened");
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
// on_before_context_menu is still on the stack. Under Chrome style the host
// draws it as an NSMenu, whose tracking loop owns the main thread, so nothing
// between the right click and the Escape may touch the automation socket: the
// window server is the only reader while it is up.
activateApp();
await Bun.sleep(400);
await app.getByTestId("c-view").rightClick();
const menus = await until("the context menu opens", menuWindows, (w) => w.length > 0, 15000).catch(() => []);
check("contextMenuOpens", menus.length > 0, `${menus.length} menu window(s)`);
systemKey(KEY_ESCAPE);
await until("the context menu closes", menuWindows, (w) => w.length === 0, 10000);
await Bun.sleep(400);
await strays("contextMenu");

// Chromium's own Inspect, picked from the native menu. The engine keeps that
// item because Chrome style docks the inspector, so the pick has to land in
// the view like the toggle does rather than opening a DevTools window.
if (process.env.ND_CEF_CHROME_SKIP_DEVTOOLS !== "1") {
  await app.getByTestId("c-devtools-open").click();
  await until(
    "the inspector closes before the Inspect pick",
    async () => (await targets()).filter((t) => t.url.startsWith("devtools://")).length,
    (n) => n === 0,
    15000,
  );
  activateApp();
  await Bun.sleep(400);
  await app.getByTestId("c-view").rightClick();
  await until("the context menu opens for Inspect", menuWindows, (w) => w.length > 0, 15000);
  const stops = menuStopsTo(shownMenu(), "Inspect");
  systemKey(KEY_DOWN_ARROW, stops);
  systemKey(KEY_RETURN);
  await until("the context menu closes after Inspect", menuWindows, (w) => w.length === 0, 10000);
  const docked = await until(
    "Inspect opens the inspector",
    async () => (await targets()).filter((t) => t.url.startsWith("devtools://")),
    (t) => t.length > 0,
    20000,
  ).catch(() => []);
  check("inspectOpensDevTools", docked.length === 1, `${docked.length} devtools target(s) after Inspect`);
  await Bun.sleep(2500);
  // Docked, not a window of its own: the two halves add up to the webview.
  await checkTiling("inspect");
  if (docked[0]?.webSocketDebuggerUrl) {
    const frontend = await Session.open(docked[0].webSocketDebuggerUrl);
    const selected = await frontend.eval<string>(`(() => {
      const walk = (root) => {
        for (const el of root.querySelectorAll('li.selected, .elements-disclosure .selected')) {
          const text = (el.textContent || '').trim();
          if (text) return text.slice(0, 60);
        }
        for (const el of root.querySelectorAll('*')) {
          if (el.shadowRoot) { const hit = walk(el.shadowRoot); if (hit) return hit; }
        }
        return '';
      };
      return walk(document);
    })()`).catch(() => "");
    frontend.close();
    check("inspectSelectsElement", selected.length > 0, selected || "the Elements selection is not readable");
  }
}

// A background tab: the page's own view is hidden by the tab view, so the
// lifted subtree goes with it, the browser is told it is hidden, and the anchor
// leaves the screen. Switching back has to bring animation frames with it.
const framesBefore = Number(await evaluate("window.__ndFrames"));
await app.getByTestId("c-background").click();
await Bun.sleep(1200);
const hiddenAnchors = (await census()).filter((w) => w.alpha === 0);
check("hiddenTabAnchorGone", hiddenAnchors.length === 0, `${hiddenAnchors.length} anchor(s) still on screen`);
await strays("backgroundTab");
await app.getByTestId("c-background").click();
await Bun.sleep(1200);
const framesAfter = Number(await evaluate("window.__ndFrames"));
check("foregroundTabRepaints", framesAfter > framesBefore, `rAF ${framesBefore} -> ${framesAfter}`);
await strays("foregroundTab");

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
// The dock is still up from the devtools leg, so the two halves have to have
// followed the window to its new width.
if (process.env.ND_CEF_CHROME_SKIP_DEVTOOLS !== "1") await checkTiling("resized");
await strays("resize");

// The surfaces Chromium draws itself rather than asking a CEF handler about:
// the WebAuthn sheet, the permission prompts, the JS dialogs, HTTP auth, a
// download and the save-password bubble. Chrome style anchors them to the
// browser's own widget, which here is a content view lifted out of an invisible
// anchor window, so the question each leg answers is whether the surface landed
// on the app's window or somewhere of its own.
// Only in the run that leaves DevTools alone: navigating this view while the
// inspector is docked trips a Chromium CHECK and takes the browser process
// down, which is the same shutdown-ordering ground as the quit crash and has
// nothing to do with the surfaces below.
const pageTarget = process.env.ND_CEF_CHROME_SKIP_DEVTOOLS !== "1"
  ? undefined
  : (await targets()).find((t) => t.type === "page" && t.url.startsWith("http://127.0.0.1"));
if (process.env.ND_CEF_CHROME_SKIP_DEVTOOLS !== "1") {
  // Nothing to assert in this run.
} else if (!pageTarget?.webSocketDebuggerUrl) {
  check("dialogSurfaces", false, "no page target to drive the dialog surfaces on");
} else {
  const page = await Session.open(pageTarget.webSocketDebuggerUrl);
  // Through a host name, not the literal address: WebAuthn refuses an
  // IP-address origin outright.
  const dialogs = pageTarget.url.replace("127.0.0.1", "localhost").replace(/\/$/, "") + "/dialogs";
  await page.send("Page.navigate", { url: dialogs });
  await Bun.sleep(2500);
  const ownWindows = (await app.windows()).windows.length;

  async function ndState(): Promise<Record<string, string>> {
    return JSON.parse(await page.eval<string>("JSON.stringify(window.ndState ?? {})").catch(() => "{}"));
  }

  /// Fires one surface and reports every window of this process the surface put
  /// on screen beyond the app's own. An anchor is invisible by construction, so
  /// anything at alpha > 0 on the ordinary window layer is a window the user can
  /// see. `dismiss` is when Escape goes in: "before" for a surface the framework
  /// answers with a sheet of the app's own, which is a window and has to be gone
  /// before the census means anything; "after" for a Chromium sheet, where
  /// ending the request the page is waiting on is the proof it took the app's
  /// key event.
  async function surface(
    name: string,
    call: string,
    settles: string,
    waitMs = 9000,
    dismiss: "none" | "before" | "after" = "none",
  ): Promise<void> {
    let fired = "";
    try {
      fired = String(await page.eval<string>(call, true));
    } catch (error) {
      fired = `threw: ${(error as Error).message.slice(0, 60)}`;
    }
    await Bun.sleep(waitMs);
    if (dismiss === "before") {
      // The framework answers a JS dialog with a sheet on the app's own window,
      // and a sheet is a window: it has to be answered before the census counts
      // anything. Return takes the default button, which Escape does not on an
      // alert whose only button is OK.
      activateApp();
      await Bun.sleep(400);
      systemKey(KEY_RETURN);
      await Bun.sleep(2000);
      systemKey(KEY_ESCAPE);
      await Bun.sleep(1500);
    }
    const visible = (await census()).filter((w) => w.alpha > 0 && w.layer === 0);
    check(
      `${name}NoStrayWindow`,
      visible.length <= ownWindows,
      visible.length > ownWindows
        ? `${visible.length} visible windows, the app owns ${ownWindows}: ${visible.map((w) => `${w.width}x${w.height}@${w.x},${w.y}`).join(" | ")}`
        : `fired ${fired}`,
    );
    if (dismiss === "after") {
      // Ended from the page rather than with a key event: a Chromium sheet here
      // is a window of its own that synthetic input never reaches, and one left
      // up would sit over every later leg.
      await page.eval("window.ndAbortPasskey()").catch(() => "");
      await Bun.sleep(2500);
    }
    if (settles) {
      const value = (await ndState())[settles] ?? "absent";
      check(`${name}Settles`, value !== "pending" && value !== "absent", `window.ndState.${settles} = ${value}`);
    }
  }

  await surface("passkeyGet", "window.ndPasskeyGet()", "passkeyGet", 9000, "after");
  await surface("passkeyCreate", "window.ndPasskeyCreate()", "passkeyCreate", 9000, "after");
  // Conditional mediation is passive by design: it draws nothing until the user
  // picks a credential out of the autofill list, so it has no outcome to settle.
  await surface("passkeyConditional", "window.ndPasskeyConditional()", "", 9000, "after");
  // Permission prompts reach the app through cef_permission_handler_t rather
  // than Chromium's own bubble, and the probe denies them.
  await surface("geolocation", "window.ndGeolocation()", "geolocation");
  await surface("notifications", "window.ndNotifications()", "notifications");
  // The framework answers a JS dialog with a sheet on the app's own window, so
  // the sheet is dismissed before the census counts windows.
  await surface("jsAlert", "window.ndAlert()", "", 4000, "before");
  await surface("httpAuth", "window.ndHttpAuth()", "", 6000, "before");
  await surface("download", "window.ndDownload()", "", 6000);
  await surface("passwordSubmit", "window.ndPasswordSubmit()", "", 8000);
  page.close();
}

if (failures.length > 0) {
  for (const failure of failures) console.error(`ND_CEF_CHROME_FAIL ${failure}`);
  process.exit(1);
}
console.log("ND_CEF_CHROME_OK");
process.exit(0);
