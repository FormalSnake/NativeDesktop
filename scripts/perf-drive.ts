#!/usr/bin/env bun
// Resource numbers for a Chrome-style app under scripts/headless-app-chrome.sh
// (ND_ACCEPT_DRIVE=scripts/perf-drive.ts, one rig): CPU of every process in the
// host's session at idle with 2 and 10 tabs, PSS and RSS with 10 tabs, the GPU
// feature status, and CPU while an animation, a scroll and a VP9 video run.
// Run it with ND_ACCEPT_TABS=10.
// With ND_PERF_STOCK set to a Chromium binary, the same numbers for that
// browser on the same display follow, for comparison. Marker: ND_PERF_DONE.
import { mkdirSync, readFileSync, readdirSync, writeFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { connectApp } from "@nativedesktop/test";
import { Session, targets } from "./cdp.ts";

const port = Number(process.env.ND_CDP_PORT ?? "9555");
const fixture = process.env.ND_ACCEPT_FIXTURE ?? "http://127.0.0.1:9557/";
const hostPid = Number(process.env.ND_ACCEPT_HOST_PID ?? "0");
const work = process.env.ND_ACCEPT_SHOTS ?? "/tmp";
const settleMs = Number(process.env.ND_PERF_SETTLE_MS ?? "20000");
const sampleMs = Number(process.env.ND_PERF_SAMPLE_MS ?? "30000");
const hz = 100;
const extensionId = process.env.ND_ACCEPT_EXTENSION_ID ?? "";

type Proc = { pid: number; kind: string };

function stat(pid: number): string[] | null {
  try {
    const raw = readFileSync(`/proc/${pid}/stat`, "utf8");
    // comm can hold spaces and parens; the fields after it are fixed.
    return raw.slice(raw.lastIndexOf(")") + 2).split(" ");
  } catch {
    return null;
  }
}

function kindOf(pid: number): string {
  let cmd = "";
  try {
    cmd = readFileSync(`/proc/${pid}/cmdline`, "utf8").replaceAll("\0", " ");
  } catch {
    return "gone";
  }
  const type = /--type=([a-z-]+)/.exec(cmd)?.[1];
  if (!type) {
    if (/\bbun\b/.test(cmd.split(" ")[0] ?? "")) return "bun";
    return "browser";
  }
  if (type === "renderer" && cmd.includes("--extension-process")) return "renderer-ext";
  if (type === "utility") return `utility:${/--utility-sub-type=([\w.]+)/.exec(cmd)?.[1]?.split(".").pop() ?? "?"}`;
  if (type === "zygote") return "zygote";
  return type;
}

/** Every process in `sid`'s session (setsid puts the whole tree in one). */
function session(sid: number): Proc[] {
  const out: Proc[] = [];
  for (const d of readdirSync("/proc")) {
    const pid = Number(d);
    if (!pid) continue;
    const f = stat(pid);
    if (!f || Number(f[3]) !== sid) continue;
    out.push({ pid, kind: kindOf(pid) });
  }
  return out;
}

/** The process tree under `root`, root included. */
function tree(root: number): Proc[] {
  const kids = new Map<number, number[]>();
  for (const d of readdirSync("/proc")) {
    const pid = Number(d);
    if (!pid) continue;
    const f = stat(pid);
    if (!f) continue;
    const ppid = Number(f[1]);
    kids.set(ppid, [...(kids.get(ppid) ?? []), pid]);
  }
  const out: Proc[] = [];
  const walk = (p: number) => {
    out.push({ pid: p, kind: kindOf(p) });
    for (const c of kids.get(p) ?? []) walk(c);
  };
  walk(root);
  return out;
}

function ticks(pid: number): number {
  const f = stat(pid);
  return f ? Number(f[11]) + Number(f[12]) : 0;
}

function memKb(pid: number): { rss: number; pss: number } {
  try {
    const s = readFileSync(`/proc/${pid}/smaps_rollup`, "utf8");
    const rss = Number(/^Rss:\s+(\d+)/m.exec(s)?.[1] ?? 0);
    const pss = Number(/^Pss:\s+(\d+)/m.exec(s)?.[1] ?? 0);
    return { rss, pss };
  } catch {
    return { rss: 0, pss: 0 };
  }
}

/** Per-thread CPU of one process, by thread name, for the busiest threads. */
async function threads(label: string, pid: number, ms: number): Promise<void> {
  const read = () => {
    const m = new Map<string, number>();
    try {
      for (const tid of readdirSync(`/proc/${pid}/task`)) {
        const raw = readFileSync(`/proc/${pid}/task/${tid}/stat`, "utf8");
        const name = raw.slice(raw.indexOf("(") + 1, raw.lastIndexOf(")"));
        const f = raw.slice(raw.lastIndexOf(")") + 2).split(" ");
        m.set(`${name}#${tid}`, Number(f[11]) + Number(f[12]));
      }
    } catch {}
    return m;
  };
  const a = read();
  await Bun.sleep(ms);
  const b = read();
  const by = new Map<string, number>();
  for (const [k, v] of b) {
    const name = k.split("#")[0]!;
    by.set(name, (by.get(name) ?? 0) + v - (a.get(k) ?? 0));
  }
  const top = [...by.entries()].filter(([, t]) => t > 0).sort((x, y) => y[1] - x[1]).slice(0, 8);
  console.log(`  threads[${label}] ${top.map(([n, t]) => `${n}=${((t / hz) / (ms / 1000) * 100).toFixed(1)}`).join(" ")}`);
}

async function cpu(label: string, procs: () => Proc[], ms: number): Promise<number> {
  const before = new Map(procs().map((p) => [p.pid, ticks(p.pid)]));
  await Bun.sleep(ms);
  const by = new Map<string, number>();
  let total = 0;
  for (const p of procs()) {
    const d = ticks(p.pid) - (before.get(p.pid) ?? 0);
    by.set(p.kind, (by.get(p.kind) ?? 0) + d);
    total += d;
  }
  const pct = (t: number) => ((t / hz) / (ms / 1000) * 100).toFixed(1);
  const parts = [...by.entries()].filter(([, t]) => t > 0).sort((a, b) => b[1] - a[1]).map(([k, t]) => `${k}=${pct(t)}`);
  console.log(`  cpu[${label}] total=${pct(total)}% of one core over ${ms / 1000}s (${parts.join(" ")})`);
  return Number(pct(total));
}

function memory(label: string, procs: Proc[]): void {
  let rss = 0;
  let pss = 0;
  const by = new Map<string, { n: number; pss: number }>();
  for (const p of procs) {
    const m = memKb(p.pid);
    rss += m.rss;
    pss += m.pss;
    const e = by.get(p.kind) ?? { n: 0, pss: 0 };
    e.n++;
    e.pss += m.pss;
    by.set(p.kind, e);
  }
  const parts = [...by.entries()].sort((a, b) => b[1].pss - a[1].pss).map(([k, e]) => `${k}x${e.n}=${Math.round(e.pss / 1024)}`);
  console.log(`  mem[${label}] procs=${procs.length} pss=${Math.round(pss / 1024)}MB rss=${Math.round(rss / 1024)}MB (pss MB: ${parts.join(" ")})`);
}

async function browserSession(p: number): Promise<Session> {
  const v = await (await fetch(`http://127.0.0.1:${p}/json/version`)).json() as { webSocketDebuggerUrl: string };
  return Session.open(v.webSocketDebuggerUrl);
}

async function gpu(b: Session): Promise<void> {
  const info = await b.send("SystemInfo.getInfo") as {
    gpu?: { featureStatus?: Record<string, string>; devices?: { vendorString?: string; deviceString?: string; driverVendor?: string }[]; auxAttributes?: Record<string, unknown>; videoDecoding?: { profile: string }[] };
  };
  const fs = info.gpu?.featureStatus ?? {};
  const keys = ["gpu_compositing", "rasterization", "opengl", "webgl", "video_decode", "video_encode", "vulkan", "skia_graphite"];
  console.log(`  gpu featureStatus: ${keys.map((k) => `${k}=${fs[k] ?? "-"}`).join(" ")}`);
  const dev = info.gpu?.devices?.[0];
  const aux = info.gpu?.auxAttributes ?? {};
  console.log(`  gpu device: ${dev?.vendorString ?? ""} ${dev?.deviceString ?? ""} driver=${dev?.driverVendor ?? ""} glRenderer=${aux.glRenderer ?? "-"} glImpl=${aux.glImplementationParts ?? aux.glImplementation ?? "-"} vaapiProfiles=${info.gpu?.videoDecoding?.length ?? 0}`);
}

async function pages(p: number): Promise<number> {
  return (await targets(p)).filter((t) => t.type === "page" && !t.url.startsWith("devtools:")).length;
}

const animHtml = `<title>anim</title><style>div{width:200px;height:200px;background:linear-gradient(red,blue);animation:s 1s linear infinite}@keyframes s{to{transform:rotate(360deg)}}</style><div></div><canvas id=c width=800 height=600></canvas><script>const x=c.getContext('2d');let n=0;(function f(t){x.fillStyle='hsl('+(t/10%360)+',80%,50%)';x.fillRect(0,0,800,600);n++;requestAnimationFrame(f)})(0);window.frames_=()=>n</script>`;
const scrollHtml = `<title>scroll</title><body style="margin:0">${"<p style='font:20px sans-serif;padding:20px;border-bottom:1px solid #ccc;background:linear-gradient(90deg,#fee,#eef)'>Row of text that is long enough to wrap across the viewport and give the rasterizer some glyphs to draw.</p>".repeat(400)}</body>`;
const videoHtml = `<title>video</title><body style="margin:0;background:#000"><video src="/v.webm" muted loop autoplay style="width:100%"></video></body>`;
// Served over http: a data: URL navigated to over the protocol never reached
// the app's tab.
const pagePort = port + 11;
Bun.serve({
  port: pagePort,
  hostname: "127.0.0.1",
  fetch: (req) => {
    const path = new URL(req.url).pathname;
    if (path === "/v.webm" && process.env.ND_PERF_VIDEO) return new Response(Bun.file(process.env.ND_PERF_VIDEO), { headers: { "content-type": "video/webm" } });
    const body = path === "/anim" ? animHtml : path === "/scroll" ? scrollHtml : path === "/video" ? videoHtml : "";
    return new Response(body, { headers: { "content-type": "text/html; charset=utf-8" } });
  },
});
const anim = `http://127.0.0.1:${pagePort}/anim`;
const scroll = `http://127.0.0.1:${pagePort}/scroll`;

/**
 * Tab switch to first frame, once round the walk. Every fixture tab logs the
 * wall-clock time of each animation frame it gets; a switch's latency is the
 * first frame its new tab draws after the switch was asked for. A page kept
 * running in the background has frames all along and answers within one
 * frame, a page Chromium had hidden answers when it has resumed.
 */
async function switchLatency(p: number, next: () => Promise<void>, prev: () => Promise<void>): Promise<void> {
  const n = Number(process.env.ND_ACCEPT_TABS ?? "10");
  const sessions = new Map<string, Session>();
  for (const t of (await targets(p)).filter((t) => t.type === "page" && t.url.startsWith(fixture))) {
    const s = await Session.open(t.webSocketDebuggerUrl!).catch(() => null);
    if (!s) continue;
    sessions.set(t.url, s);
    await s.eval(`(() => { if (window.__fr) return; window.__fr = []; const f = () => { __fr.push(Date.now()); if (__fr.length > 600) __fr.shift(); requestAnimationFrame(f); }; requestAnimationFrame(f); })()`).catch(() => {});
  }
  // Long enough for every tab but the one on show to settle in the background.
  await Bun.sleep(Number(process.env.ND_PERF_SWITCH_IDLE_MS ?? "20000"));
  const ms: number[] = [];
  for (let i = 0; i < n; i++) {
    const url = i === n - 1 ? fixture : i === 0 ? `${fixture}?two` : `${fixture}?tab${i + 2}`;
    const t0 = Date.now();
    await next();
    await Bun.sleep(1500);
    const s = sessions.get(url);
    const first = s ? await s.eval<number>(`(__fr.find((t) => t >= ${t0}) ?? 0)`).catch(() => 0) : 0;
    ms.push(first ? first - t0 : -1);
    await Bun.sleep(Number(process.env.ND_PERF_SWITCH_GAP_MS ?? "3000"));
  }
  const report = (label: string, list: number[]) => {
    const ok = list.filter((x) => x >= 0).sort((a, b) => a - b);
    console.log(`  ${label} ms: ${list.join(" ")} (median ${ok[Math.floor(ok.length / 2)] ?? "?"}, max ${ok[ok.length - 1] ?? "?"})`);
  };
  report("switch to first frame, round all tabs", ms);
  // Back and forth between two tabs, the way a person checks one page against
  // another: each was on show seconds ago.
  const pair: number[] = [];
  for (let i = 0; i < 8; i++) {
    const url = i % 2 === 0 ? `${fixture}?two` : fixture;
    const t0 = Date.now();
    await (i % 2 === 0 ? next : prev)();
    await Bun.sleep(1500);
    const s = sessions.get(url);
    const first = s ? await s.eval<number>(`(__fr.find((t) => t >= ${t0}) ?? 0)`).catch(() => 0) : 0;
    pair.push(first ? first - t0 : -1);
    await Bun.sleep(Number(process.env.ND_PERF_SWITCH_GAP_MS ?? "3000"));
  }
  report("switch to first frame, between two tabs", pair);
  for (const s of sessions.values()) s.close();
}

/** Runs the measured sequence against a browser listening on `p`; `procs` names its processes. */
async function measure(
  name: string,
  p: number,
  procs: () => Proc[],
  open: (url: string) => Promise<void>,
  popup: (() => Promise<void>) | null,
  show: (url: string) => Promise<void> = open,
  warm: () => Promise<void> = async () => {},
  next: (() => Promise<void>) | null = null,
  prev: () => Promise<void> = async () => {},
): Promise<void> {
  console.log(`== ${name}`);
  await Bun.sleep(settleMs);
  console.log(`  tabs=${await pages(p)}`);
  await cpu(`${name} idle`, procs, sampleMs);
  const b = await browserSession(p);
  await gpu(b);
  for (let i = 0; i < 8; i++) await open(`${fixture}?perf${i}`);
  await warm();
  await Bun.sleep(settleMs);
  console.log(`  tabs=${await pages(p)}`);
  await cpu(`${name} idle 10 tabs`, procs, sampleMs);
  memory(`${name} 10 tabs`, procs());
  if (next) await switchLatency(p, next, prev);
  const main = procs().find((x) => x.kind === "browser");
  if (main) await threads(`${name} browser process`, main.pid, 15000);
  const vis: string[] = [];
  for (const t of (await targets(p)).filter((t) => t.type === "page" && t.url.startsWith(fixture))) {
    const s = await Session.open(t.webSocketDebuggerUrl!).catch(() => null);
    if (!s) continue;
    vis.push(await s.eval<string>("document.visibilityState").catch(() => "?"));
    s.close();
  }
  console.log(`  page visibility: ${vis.join(" ")}`);
  // Timers and frames in one background tab, over three seconds: a hidden page
  // gets no rAF and its 10 ms interval is throttled to about 1 Hz.
  // A tab the walk left long ago, well behind the ones shown most recently.
  const last = (await targets(p)).find((t) => t.type === "page" && t.url === `${fixture}?tab5`);
  const bg = last ? { url: last.url, s: await Session.open(last.webSocketDebuggerUrl!) } : null;
  if (bg) {
    const s = bg.s;
    const r = await s.eval<string>(`(async () => {
      let ticks = 0, frames = 0;
      const id = setInterval(() => ticks++, 10);
      const f = () => { frames++; requestAnimationFrame(f); };
      requestAnimationFrame(f);
      const t0 = performance.now();
      await new Promise((r) => setTimeout(r, 3000));
      clearInterval(id);
      return document.visibilityState + " ticks=" + ticks + " frames=" + frames + " ms=" + Math.round(performance.now() - t0);
    })()`).catch((e) => String(e));
    console.log(`  background tab ${bg.url.replace(fixture, "")}: ${r}`);
    s.close();
  }
  for (const x of procs().filter((q) => q.kind === "renderer").slice(0, 1)) {
    const cmd = readFileSync(`/proc/${x.pid}/cmdline`, "utf8").split("\0");
    console.log(`  renderer features: ${cmd.filter((a) => /^--(enable|disable)-features=/.test(a)).join(" ")}`);
  }
  const ui = (await targets(p)).filter((t) => t.type === "browser_ui");
  console.log(`  browser_ui targets: ${ui.length} ${[...new Set(ui.map((t) => t.url))].join(" ")}`);
  const ext = (await targets(p)).filter((t) => t.url.startsWith("chrome-extension://"));
  console.log(`  extension targets: ${ext.map((t) => `${t.type}:${t.url.replace(/^chrome-extension:\/\/([a-p]{6})[a-p]+/, "$1")}`).join(" ") || "none"}`);
  await show(anim);
  await Bun.sleep(3000);
  const animTarget = (await targets(p)).find((t) => t.url === anim);
  if (!animTarget) console.log(`  no ${anim} among ${(await targets(p)).filter((t) => t.type === "page").map((t) => t.url).join(" ")}`);
  const animPage = animTarget?.webSocketDebuggerUrl ? await Session.open(animTarget.webSocketDebuggerUrl) : null;
  const f0 = animPage ? await animPage.eval<number>("window.frames_()").catch(() => -1) : -1;
  await cpu(`${name} animation`, procs, 15000);
  const f1 = animPage ? await animPage.eval<number>("window.frames_()").catch(() => -1) : -1;
  console.log(`  animation fps=${f0 >= 0 && f1 >= 0 ? ((f1 - f0) / 15).toFixed(1) : "?"}`);
  animPage?.close();

  await show(scroll);
  await Bun.sleep(3000);
  const scrollTarget = (await targets(p)).find((t) => t.url === scroll);
  if (scrollTarget?.webSocketDebuggerUrl) {
    const s = await Session.open(scrollTarget.webSocketDebuggerUrl);
    const run = cpu(`${name} scroll`, procs, 10000);
    const t0 = Date.now();
    while (Date.now() - t0 < 9500) {
      await s.send("Input.synthesizeScrollGesture", { x: 300, y: 300, yDistance: -3000, speed: 3000, gestureSourceType: "mouse" }).catch(() => {});
      await s.send("Input.synthesizeScrollGesture", { x: 300, y: 300, yDistance: 3000, speed: 3000, gestureSourceType: "mouse" }).catch(() => {});
    }
    await run;
    console.log(`  scrollY after=${await s.eval<number>("scrollY").catch(() => -1)}`);
    s.close();
  }

  const video = process.env.ND_PERF_VIDEO;
  if (video && existsSync(video)) {
    const videoUrl = `http://127.0.0.1:${pagePort}/video`;
    await show(videoUrl);
    await Bun.sleep(4000);
    const vt = (await targets(p)).find((t) => t.url === videoUrl);
    let s: Session | null = null;
    if (vt?.webSocketDebuggerUrl) {
      s = await Session.open(vt.webSocketDebuggerUrl);
      await s.eval("(() => { const v = document.querySelector('video'); v.muted = true; v.loop = true; return v.play().then(() => true, () => false) })()", true).catch(() => false);
    }
    await Bun.sleep(2000);
    await cpu(`${name} video`, procs, 15000);
    if (s) {
      const q = await s.eval<string>("(() => { const v = document.querySelector('video'); const q = v.getVideoPlaybackQuality(); return v.videoWidth + 'x' + v.videoHeight + ' t=' + v.currentTime.toFixed(1) + ' frames=' + q.totalVideoFrames + ' dropped=' + q.droppedVideoFrames })()").catch((e) => String(e));
      console.log(`  video ${q}`);
      s.close();
    }
  }
  const hostLog = process.env.ND_ACCEPT_HOST_LOG;
  if (name === "app" && hostLog) {
    // Each registry or badge read is one evaluate on the host's trace.
    const log = readFileSync(hostLog, "utf8");
    const count = (needle: string) => log.split(needle).length - 1;
    console.log(`  registry reads=${count("listExtensions needs")} badge reads=${count("readExtensionAction needs")} change events=${count("extensionsChanged node=")}`);
  }
  b.close();
  if (popup && extensionId) {
    const times: number[] = [];
    for (let i = 0; i < 3; i++) {
      const isPopup = (t: { url: string }) => t.url.startsWith(`chrome-extension://${extensionId}/popup.html`);
      const before = (await targets(p)).filter(isPopup).length;
      const t0 = performance.now();
      await popup();
      let ms = -1;
      while (performance.now() - t0 < 10000) {
        if ((await targets(p)).filter(isPopup).length > before) {
          ms = performance.now() - t0;
          break;
        }
        await Bun.sleep(20);
      }
      times.push(Math.round(ms));
      await closePopup?.();
      await Bun.sleep(1500);
    }
    console.log(`  popup open ms: ${times.join(" ")}`);
  }

}

let closePopup: (() => Promise<void>) | null = null;

if (process.env.ND_PERF_SKIP_APP !== "1") {
  const app = await connectApp();
  closePopup = async () => {
    await app.keyboard.press("Escape").catch(() => {});
  };
  // What has to be on screen replaces the tab that is.
  let shownPage: Session | null = null;
  // The rig seeds the app's session with ten tabs (ND_ACCEPT_TABS).
  await measure("app", port, () => session(hostPid), async () => {}, async () => {
    await app.getByTestId(`ext-action-${extensionId}`).click();
  }, async (url) => {
    if (!shownPage) {
      for (const t of (await targets(port)).filter((t) => t.type === "page" && t.url.startsWith(fixture))) {
        const s = await Session.open(t.webSocketDebuggerUrl!).catch(() => null);
        if (!s) continue;
        if ((await s.eval<string>("document.visibilityState").catch(() => "")) === "visible") {
          shownPage = s;
          break;
        }
        s.close();
      }
    }
    const r = await shownPage?.send("Page.navigate", { url }).catch((e) => String(e));
    console.log(`  navigate ${url}: ${JSON.stringify(r)}`);
    await Bun.sleep(1500);
  }, async () => {
    // Every restored tab is loaded once it has been shown, so walk them all
    // and come back round to the first. Each extension that opens a welcome
    // tab on install adds one (ND_PERF_EXTRA_TABS).
    const n = Number(process.env.ND_ACCEPT_TABS ?? "10") + Number(process.env.ND_PERF_EXTRA_TABS ?? "0");
    for (let i = 0; i < n; i++) {
      await app.getByTestId("menu-next-tab").click().catch((e) => console.log(`  next tab: ${e}`));
      await Bun.sleep(1500);
    }
  }, process.env.ND_PERF_SWITCH === "1" ? async () => {
    await app.getByTestId("menu-next-tab").click().catch((e) => console.log(`  next tab: ${e}`));
  } : null, async () => {
    await app.getByTestId("menu-prev-tab").click().catch((e) => console.log(`  previous tab: ${e}`));
  });
}

const stock = process.env.ND_PERF_STOCK;
if (stock) {
  const stockPort = port + 7;
  const dir = join(work, "stock-profile");
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "First Run"), "");
  const args = [
    `--user-data-dir=${dir}`,
    `--remote-debugging-port=${stockPort}`,
    "--remote-allow-origins=*",
    "--no-first-run",
    "--no-default-browser-check",
    "--ozone-platform=x11",
    "--password-store=basic",
    "--enable-unsafe-extension-debugging",
    ...(process.env.ND_PERF_STOCK_EXTENSIONS ? [`--load-extension=${process.env.ND_PERF_STOCK_EXTENSIONS}`, `--disable-features=DisableLoadExtensionCommandLineSwitch`] : []),
    ...(process.env.ND_PERF_STOCK_ARGS ?? "").split(" ").filter(Boolean),
    `${fixture}`,
    `${fixture}?two`,
  ];
  const child = Bun.spawn([stock, ...args], { stdout: "ignore", stderr: Bun.file(join(work, "stock.log")) });
  for (let i = 0; i < 100; i++) {
    if (await fetch(`http://127.0.0.1:${stockPort}/json/version`).then(() => true, () => false)) break;
    await Bun.sleep(200);
  }
  closePopup = async () => {
    const b = await browserSession(stockPort);
    for (const t of await targets(stockPort)) {
      if (t.url.startsWith(`chrome-extension://${extensionId}/popup.html`)) await b.send("Target.closeTarget", { targetId: t.id }).catch(() => {});
    }
    b.close();
  };
  await measure("stock", stockPort, () => tree(child.pid), async (url) => {
    await fetch(`http://127.0.0.1:${stockPort}/json/new?${encodeURI(url)}`, { method: "PUT" }).catch(() => {});
    await Bun.sleep(1500);
  }, async () => {
    const b = await browserSession(stockPort);
    const all = await b.send("Target.getTargets") as { targetInfos: { targetId: string; type: string; url: string }[] };
    const tab = all.targetInfos.find((t) => t.type === "tab" && t.url.startsWith(fixture));
    await b.send("Extensions.triggerAction", { id: extensionId, targetId: tab?.targetId ?? "" }).catch((e) => console.log(`  triggerAction: ${e}`));
    b.close();
  });
  child.kill();
  await child.exited;
}
console.log("ND_PERF_DONE");
process.exit(0);
