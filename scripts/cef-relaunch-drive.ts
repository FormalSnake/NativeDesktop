#!/usr/bin/env bun
// The relaunch leg of the Chrome-style gates, on macOS and on X11. A host that
// was killed mid-session comes back on the same cache directory, and a second
// host is started against that cache while the first holds it. Neither may put
// a Chromium window of its own on screen: Chrome answers both with its startup
// browser (a "New Tab" window, plus "Restore pages?" when the profile's last
// exit was a crash) unless the engine stops it.
//
// ND_RELAUNCH_STEP picks what to do:
//   navigate   before the kill: move the page off its first URL, so a restored
//              session would be told apart from the app's own page
//   relaunch   after the relaunch on the same cache
//   collision  after a second host was started against the running one
import { connectApp } from "@nativedesktop/test";
import { Session } from "./cdp.ts";

const step = process.env.ND_RELAUNCH_STEP ?? "relaunch";
const pid = Number(process.env.ND_HOST_PID ?? "0");
const port = process.env.ND_CEF_DEBUG_PORT ?? process.env.ND_CDP_PORT ?? "9334";
const hostLog = process.env.ND_HOST_LOG ?? "";
const marker = "before-kill";

const failures: string[] = [];
const check = (name: string, ok: boolean, detail: string) => {
  if (ok) console.log(`ND_CEF_RELAUNCH_CHECK ${step}/${name}: ok (${detail})`);
  else {
    console.log(`ND_CEF_RELAUNCH_CHECK ${step}/${name}: FAIL (${detail})`);
    failures.push(`${name}: ${detail}`);
  }
};

type Target = { targetId: string; type: string; url: string; title: string };

async function getTargets(): Promise<Target[]> {
  const version = (await (await fetch(`http://127.0.0.1:${port}/json/version`)).json()) as {
    webSocketDebuggerUrl: string;
  };
  const browser = await Session.open(version.webSocketDebuggerUrl);
  try {
    const result = await browser.send("Target.getTargets");
    return (result.targetInfos as Target[]) ?? [];
  } finally {
    browser.close();
  }
}

const appPage = (t: Target) => t.type === "page" && /^https?:\/\/(127\.0\.0\.1|localhost)[:/]/.test(t.url);

async function waitForAppPage(timeoutMs = 30000): Promise<Target | undefined> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const hit = (await getTargets().catch(() => [] as Target[])).find(appPage);
    if (hit) return hit;
    await Bun.sleep(250);
  }
  return undefined;
}

if (step === "navigate") {
  const page = await waitForAppPage();
  if (!page) {
    console.log("ND_CEF_RELAUNCH_FAIL navigate: the app's page never came up");
    process.exit(1);
  }
  const list = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()) as {
    id: string;
    webSocketDebuggerUrl: string;
  }[];
  const socket = list.find((t) => t.id === page.targetId)?.webSocketDebuggerUrl ?? "";
  const session = await Session.open(socket);
  const next = `${page.url.split("?")[0]}?${marker}`;
  await session.send("Page.navigate", { url: next });
  const deadline = Date.now() + 10000;
  let now = "";
  while (Date.now() < deadline) {
    now = (await getTargets()).find((t) => t.targetId === page.targetId)?.url ?? "";
    if (now.includes(marker)) break;
    await Bun.sleep(200);
  }
  session.close();
  check("navigated", now.includes(marker), now);
  // Chrome writes the session and the exit type on a timer; what matters is
  // that the kill lands on a profile that has recorded a live session.
  await Bun.sleep(3000);
  finish();
}

/// Windows of the host on the screen that the app does not own. On macOS the
/// window server's list against the app's own window list; on X11 every
/// mapped top-level carrying the host's pid, against the app's own count.
async function census(): Promise<{ own: number; visible: string[] }> {
  const app = await connectApp();
  const own = (await app.windows()).windows.length;
  const visible: string[] = [];
  if (process.platform === "darwin") {
    const proc = Bun.spawn(["swift", "scripts/mac/window-census.swift", String(pid)], {
      stdout: "pipe",
      env: { ...process.env, SDKROOT: undefined },
    });
    for (const line of (await new Response(proc.stdout).text()).split("\n")) {
      if (!line.trim()) continue;
      const w = JSON.parse(line) as { alpha: number; layer: number; width: number; height: number; title: string };
      // Layer 0 and drawn: an anchor is alpha 0 and a menu or tooltip sits
      // above the window layer, and neither is a browser window.
      if (w.alpha > 0 && w.layer === 0) visible.push(`${w.width}x${w.height}${w.title ? ` "${w.title}"` : ""}`);
    }
  } else {
    const sh = (...argv: string[]) => {
      const r = Bun.spawnSync(argv, { stdout: "pipe", stderr: "ignore" });
      return new TextDecoder().decode(r.stdout);
    };
    for (const line of sh("xwininfo", "-root", "-children").split("\n")) {
      const m = line.match(/^\s*(0x[0-9a-f]+)\s+(".*?"|\(has no name\)).*?\s(\d+)x(\d+)\+/);
      if (!m) continue;
      const [, id, name, w, h] = m;
      // Chromium's 1x1 and 10x10 utility windows are never presented.
      if (Number(w) < 40 || Number(h) < 40) continue;
      if (!sh("xwininfo", "-id", id).includes("Map State: IsViewable")) continue;
      if (Number(sh("xdotool", "getwindowpid", id).trim()) !== pid) continue;
      visible.push(`${id} ${name} ${w}x${h}`);
    }
  }
  // What the screen showed, with sheets and panels composited on macOS
  // (ND_AUTOMATION_CAPTURE=region on the host), for whoever reads a red run.
  const shots = process.env.ND_RELAUNCH_SHOT_DIR;
  if (shots) {
    const r = await app.screenshot(`${shots}/${step}.png`).catch((e: unknown) => String(e));
    console.log(`ND_CEF_RELAUNCH_SHOT ${step}: ${typeof r === "string" ? r : `${shots}/${step}.png ${r.width}x${r.height}`}`);
  }
  await app.close();
  return { own, visible };
}

async function logLines(pattern: RegExp): Promise<string[]> {
  if (!hostLog) return [];
  const text = await Bun.file(hostLog).text().catch(() => "");
  return text.split("\n").filter((line) => pattern.test(line));
}

// A relaunch is answered on the running host's UI thread a moment after the
// second process connects, and a Chrome window takes a moment more to map.
await Bun.sleep(3000);

const page = await waitForAppPage();
check("appPage", page !== undefined, page ? page.url : "no app page among the targets");

const targets = await getTargets();
// The killed session's page, anywhere: in a browser Chrome restored for itself
// as much as in the app's own view.
const restored = targets.filter((t) => t.url.includes(marker));
check(
  "notRestored",
  restored.length === 0,
  restored.length ? `the killed session came back: ${restored.map((t) => `${t.type} ${t.url}`).join(" | ")}` : "no target carries the killed session's page",
);
const chromeOwn = targets.filter(
  (t) => /^chrome:\/\/(newtab|new-tab-page|restore|welcome|whats-new|history|settings)/.test(t.url) ||
    /^chrome-search:/.test(t.url),
);
check(
  "targets",
  chromeOwn.length === 0,
  chromeOwn.length
    ? `Chrome's own pages are open: ${chromeOwn.map((t) => `${t.type} ${t.url}`).join(" | ")}`
    : `${targets.length} target(s), none of Chrome's own`,
);

const { own, visible } = await census();
check(
  "census",
  visible.length === own,
  `${visible.length} window(s) on screen for pid ${pid}, the app owns ${own}${visible.length ? `: ${visible.join(" | ")}` : ""}`,
);

// What the engine did with a Chrome-created browser or a Views surface: the
// X11 sink and window watcher, the AppKit surface adoption.
const engineSaw = await logLines(/ND_CEF (sinkCreated|sinkNewWindow|chromeDialog)|surface adopted|newWindow chrome:/);
check(
  "engineTrace",
  engineSaw.length === 0,
  engineSaw.length ? engineSaw.slice(0, 3).join(" | ") : "no Chrome-created browser, no adopted surface",
);

finish();

function finish(): never {
  if (failures.length) {
    console.log(`ND_CEF_RELAUNCH_FAIL ${step}: ${failures.join("; ")}`);
    process.exit(1);
  }
  console.log(`ND_CEF_RELAUNCH_OK(${step})`);
  process.exit(0);
}
