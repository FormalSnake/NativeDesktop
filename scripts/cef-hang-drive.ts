#!/usr/bin/env bun
// scripts/cef-hang-drive.ts: drives examples/webview-probe/cef-hang.tsx under
// scripts/headless-cef-hang.sh. The engine asks "Page Unresponsive" about a
// page whose ping goes unanswered for 15 s; Chromium holds devtools messages
// back for as long as a main-frame navigation is in flight, so every leg that
// only waits runs past that 15 s and must end with no question asked.
//
//   leg 1  a same-site navigation to a server that sleeps 60 s
//   leg 2  a cross-site navigation to the same sleeping server (new renderer)
//   leg 3  http://neverssl.com/, slow or unanswered on some networks
//   leg 4  a cross-site navigation that commits
//   leg 5  a page waiting on its own alert()
//   leg 6  a page whose main thread spins: asked about within ~20 s, Wait
//          holds the question back 15 s, Exit page ends in renderProcessGone
//
// The window is kept as `cef-hang-<leg>.png` after each leg.
import { connectApp, findNode, poll } from "../packages/test/src/index.ts";

const log = process.env.ND_HOST_LOG ?? "";
const dir = process.env.XDG_RUNTIME_DIR ?? "/tmp";
const app = await connectApp();

const label = async (id: string) => findNode((await app.tree()).root, id)?.text ?? "";
const logText = async () => (log ? await Bun.file(log).text() : "");
const count = async (needle: string) => (await logText()).split(needle).length - 1;
const shown = () => count("pageUnresponsive shown");
const key = (chord: string) => {
  const r = Bun.spawnSync(["xdotool", "key", "--clearmodifiers", chord]);
  if (r.exitCode !== 0) throw new Error(`xdotool key ${chord}: ${r.stderr.toString().trim()}`);
};

async function capture(name: string) {
  const win = (await app.windows()).windows[0]?.geometry;
  const shot = `${dir}/cef-hang-root.png`;
  Bun.spawnSync(["import", "-window", "root", shot]);
  if (win) Bun.spawnSync(["magick", shot, "-crop", `${win.w}x${win.h}+${win.x}+${win.y}`, `${dir}/cef-hang-${name}.png`]);
}

let failed = false;
const fail = (msg: string) => {
  failed = true;
  console.log(`FAIL ${msg}`);
};

async function go(target: string) {
  await app.getByTestId(`h-${target}`).click();
}

async function home() {
  await go("home");
  await poll(() => label("h-title"), (v) => v === "title=home 127.0.0.1", { timeoutMs: 30000 });
}

/// Waits past the verdict (3 s tick + 15 s) and checks no question came up.
async function quiet(leg: string, ms = 24000) {
  const before = await shown();
  await Bun.sleep(ms);
  await capture(leg);
  const after = await shown();
  if (after !== before) fail(`${leg}: Page Unresponsive shown ${after - before}x`);
  else console.log(`ok ${leg}: no question in ${ms / 1000}s`);
}

await home();
await capture("0-home");

await go("slow");
await quiet("1-slow");

await home();
await go("crossSlow");
await quiet("2-cross-slow");

await home();
await go("neverssl");
await quiet("3-neverssl");

await home();
await go("cross");
await poll(() => label("h-title"), (v) => v === "title=home localhost", { timeoutMs: 30000 });
await quiet("4-cross", 20000);

await home();
const dialogsBefore = await count("pageDialog node=");
await go("alert");
await poll(() => count("pageDialog node="), (n) => n > dialogsBefore, { timeoutMs: 15000 }).catch(() => fail("5-alert: no page dialog"));
await quiet("5-alert");
key("Escape");

await home();
const t0 = Date.now();
const before = await shown();
await go("busy");
await poll(shown, (n) => n > before, { timeoutMs: 30000 })
  .then(() => console.log(`ok 6-busy: asked after ${((Date.now() - t0) / 1000).toFixed(1)}s`))
  .catch(() => fail("6-busy: never asked"));
await Bun.sleep(1000);
await capture("6-busy-asked");
// Escape is the dialog's close response, which is Wait.
const waits = await count("exit=false");
key("Escape");
await poll(() => count("exit=false"), (n) => n > waits, { timeoutMs: 5000 }).catch(() => fail("6-busy: Wait not answered"));
const waited = Date.now();
await poll(shown, (n) => n > before + 1, { timeoutMs: 40000 })
  .then(() => {
    const s = (Date.now() - waited) / 1000;
    if (s < 14) fail(`6-busy: asked again ${s.toFixed(1)}s after Wait`);
    else console.log(`ok 6-busy: asked again ${s.toFixed(1)}s after Wait`);
  })
  .catch(() => fail("6-busy: not asked again after Wait"));
await Bun.sleep(1000);
// Exit page is the other response: one Tab from the focused default.
key("shift+Tab");
key("Return");
await poll(() => count("exit=true"), (n) => n > 0, { timeoutMs: 5000 }).catch(() => fail("6-busy: Exit page not answered"));
await poll(() => label("h-gone"), (v) => v !== "gone=", { timeoutMs: 15000 })
  .then((v) => console.log(`ok 6-busy: ${v}`))
  .catch(() => fail("6-busy: no renderProcessGone after Exit page"));
await capture("6-busy-exited");

console.log(failed ? "ND_CEF_HANG_FAIL" : "ND_CEF_HANG_OK");
process.exit(failed ? 1 : 0);
