#!/usr/bin/env bun
// Drives examples/webview-probe/cef-m1.tsx over the automation socket and
// asserts the M1 contract for an embedded engine: url/title/loading/progress/
// canGoBack/canGoForward flow, a real page is laid out and composited at the
// embedded view's size, and window.open becomes a `newWindow` event with the
// app's window count unchanged. Marker: ND_CEF_M1_OK.
//
// Engine-agnostic on purpose: every assertion reads the schema's own events
// off the tree, so the same drive is the parity check between the system and
// chromium engines.
import { AutomationClient, findNode } from "@nativedesktop/test";
import type { GetTreeResult, JsonNode } from "@nativedesktop/test";

const client = await AutomationClient.connect();

function label(tree: JsonNode, testID: string): string {
  const node = findNode(tree, testID);
  if (!node) throw new Error(`${testID} not found in tree`);
  const text = node.text ?? "";
  return text.slice(text.indexOf("=") + 1);
}

// The page reports through the title once it has counted 30 animation frames,
// which is the paint signal: a browser that never got composited into a
// visible view is never asked for a frame.
await client.call("waitFor", { condition: { textContains: "title=painted" }, timeoutMs: 60000 });
await client.call("waitFor", { condition: { textContains: "popup=https://" }, timeoutMs: 30000 });
await client.call("waitFor", { condition: { textContains: "armed=http" }, timeoutMs: 30000 });

const root = ((await client.call("getTree")) as GetTreeResult).root;
const failures: string[] = [];
const check = (name: string, ok: boolean, detail: string) => {
  if (ok) console.log(`ND_CEF_M1_CHECK ${name}: ok (${detail})`);
  else failures.push(`${name}: ${detail}`);
};

const url = label(root, "m1-url");
check("navigate", url.startsWith("http"), url);

const title = label(root, "m1-title");
const painted = /^painted rAF=(\d+) h1=(\d+)x(\d+) vp=(\d+)x(\d+) dpr=([\d.]+)$/.exec(title);
check("titleChanged", painted !== null, title);
if (painted) {
  check("paint", Number(painted[1]) >= 30, `${painted[1]} animation frames`);
  check("layout", Number(painted[2]) > 0 && Number(painted[3]) > 0, `h1 ${painted[2]}x${painted[3]}`);
  check("embedded", Number(painted[4]) > 0 && Number(painted[5]) > 0, `viewport ${painted[4]}x${painted[5]} @${painted[6]}x`);
}

check("loadingChanged", label(root, "m1-loading") === "false", "settled after the load");
check("loadProgress", Number(label(root, "m1-progress")) === 1, label(root, "m1-progress"));
check("backForward", label(root, "m1-nav") === "false,false", `canGoBack,canGoForward = ${label(root, "m1-nav")}`);

const popup = label(root, "m1-popup");
check("newWindow", popup.startsWith("https://"), popup);

// A view mounted empty and given its address on a later commit. The engine
// used to park that address and never load it, which left every tab, popup and
// background page on about:blank.
const armed = label(root, "m1-armed");
check("armedAfterMount", armed.includes("armed=1"), armed);

// The invariant the spec calls hard: nothing the engine does may put a second
// top-level window on screen. The app's own census is the assertion, since the
// app is the only thing allowed to have opened one.
// The world-scoped eval has to land in the MAIN frame. The fixture injects
// into every frame, so the isolated world exists twice and only a cache that
// remembers which frame each context belongs to can tell them apart.
const inWorld = (await client.call("webviewEval", {
  testId: "m1-armed-view",
  code: "window.__ndFrameIsTop",
  world: "probe",
})) as { ok: boolean; value: string | null; error: string | null };
check("worldTargetsMainFrame", inWorld.ok && inWorld.value === "true", JSON.stringify(inWorld));

// A real right-click into the engine's own view, which is the only way to
// make on_before_context_menu run. The RPC dismisses the menu it opens.
await client.call("rightClick", { testId: "m1-view" });
await Bun.sleep(750);

const windows = (await client.call("windows")) as { windows: { title: string }[] };
check(
  "noStrayWindow",
  windows.windows.length === 1,
  `${windows.windows.length} window(s): ${windows.windows.map((w) => w.title).join(", ")}`,
);

// webviewInfo answers from whichever engine is rendering. On the chromium path
// the WKWebView underneath is never loaded, so this is what catches the
// automation surface reading the wrong engine's state.
const view = findNode(root, "m1-view");
if (!view) throw new Error("m1-view not found in tree");
const info = (await client.call("webviewInfo", { ref: view.ref })) as { url: string | null; title: string | null };
check("webviewInfo", info.url === url && info.title === title, JSON.stringify(info));

if (failures.length > 0) {
  for (const failure of failures) console.error(`ND_CEF_M1_FAIL ${failure}`);
  process.exit(1);
}
console.log("ND_CEF_M1_OK");
process.exit(0);
