import { executeJavaScript, onJavaScriptResult, render, useEffect, useRef, useState, webviewEngine } from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// Content-blocking probe for the Chromium engine. A fixture page carries one
// case per rule kind; the page reports what survived through its title, and
// the probe walks three phases: blocking on, the site switched off, a user
// rule added. Each phase prints one ND_ADBLOCK_PHASE line; the drive script
// (scripts/adblock-drive.ts) asserts on them.

const child = Bun.serve({
  port: 0,
  hostname: "localhost",
  fetch(req) {
    const path = new URL(req.url).pathname;
    if (path === "/tracker.gif") {
      return new Response(Buffer.from("R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7", "base64"), { headers: { "content-type": "image/gif" } });
    }
    return new Response(
      `<!doctype html><div class="ad-banner">child ad</div><script>
      setTimeout(function () {
        var el = document.querySelector(".ad-banner");
        parent.postMessage({ child: getComputedStyle(el).display, childScriptlet: window.__ndScriptlet === 42 }, "*");
      }, 1200);
      </script>`,
      { headers: { "content-type": "text/html" } },
    );
  },
});

const PAGE = (childPort: number) => `<!doctype html><html><head><meta charset="utf-8"><title>loading</title>
<script>window.__early = window.__ndScriptlet;</script>
<script src="/ad-script.js"></script>
<script src="/redirected.js"></script>
</head><body style="font:16px system-ui">
<div class="ad-banner">generic class rule</div>
<div class="site-ad">site rule</div>
<div class="proc">Sponsored content (procedural)</div>
<div class="user-hidden">user rule</div>
<img id="px" src="http://localhost:${childPort}/tracker.gif">
<iframe src="http://localhost:${childPort}/frame"></iframe>
<script>
  var report = { child: "missing", worker: false };
  new Worker("/worker.js").onmessage = function () { report.worker = true; };
  addEventListener("message", function (e) { if (e.data && e.data.child) { report.child = e.data.child; report.childScriptlet = e.data.childScriptlet; } });
  function shown(sel) { return getComputedStyle(document.querySelector(sel)).display !== "none"; }
  setTimeout(function () {
    report.adScript = !!window.__adLoaded;
    report.redirectReal = !!window.__redirReal;
    report.redirectNoop = !!window.__ndNoop;
    report.scriptletEarly = window.__early === 42;
    report.pixel = document.getElementById("px").naturalWidth > 0;
    report.banner = shown(".ad-banner");
    report.siteAd = shown(".site-ad");
    report.proc = shown(".proc");
    report.user = shown(".user-hidden");
    document.title = "R" + JSON.stringify(report);
  }, 2000);
</script></body></html>`;

const top = Bun.serve({
  port: 0,
  hostname: "127.0.0.1",
  fetch(req) {
    const path = new URL(req.url).pathname;
    if (path === "/ad-script.js") return new Response("window.__adLoaded = 1", { headers: { "content-type": "text/javascript" } });
    if (path === "/worker.js") return new Response("postMessage(1)", { headers: { "content-type": "text/javascript" } });
    if (path === "/redirected.js") return new Response("window.__redirReal = 1", { headers: { "content-type": "text/javascript" } });
    return new Response(PAGE(child.port), { headers: { "content-type": "text/html" } });
  },
});

const dir = mkdtempSync(join(tmpdir(), "nd-adblock-probe-"));
const listPath = join(dir, "list.txt");
writeFileSync(
  listPath,
  [
    "||127.0.0.1^*/ad-script.js",
    "||localhost^*/tracker.gif$image",
    "||127.0.0.1^*/redirected.js$script,redirect=noop.js",
    "##.ad-banner",
    "127.0.0.1##.site-ad",
    "127.0.0.1##div.proc:has-text(Sponsored)",
    "127.0.0.1##+js(nd-test-set, __ndScriptlet, 42)",
  ].join("\n"),
);
const b64 = (s: string) => Buffer.from(s).toString("base64");
const resourcesPath = join(dir, "resources.json");
writeFileSync(
  resourcesPath,
  JSON.stringify([
    { name: "noop.js", aliases: [], kind: { mime: "application/javascript" }, content: b64("window.__ndNoop = 1;") },
    { name: "nd-test-set.js", aliases: [], kind: "template", content: b64("window['{{1}}'] = {{2}};") },
  ]),
);
const BASE = `http://127.0.0.1:${top.port}/`;
console.log(`ND_ADBLOCK_FIXTURE ${BASE}`);

type Phase = "on" | "siteOff" | "userRule";
const PHASES: Phase[] = ["on", "siteOff", "userRule"];

function App() {
  const view = useRef<NdNodeRef<"webview"> | null>(null);
  const [url, setUrl] = useState("about:blank");
  const [phase, setPhase] = useState(0);
  const [blocked, setBlocked] = useState(0);
  const [title, setTitle] = useState("");

  useEffect(() => {
    void (async () => {
      const loaded = await webviewEngine.contentBlocking.load({
        lists: [{ path: listPath }],
        resources: resourcesPath,
        cacheFile: join(dir, "engine.bin"),
        cacheKey: "probe-1",
      });
      console.log(`ND_ADBLOCK_LOAD ${JSON.stringify(loaded)}`);
      setUrl(`${BASE}?phase=on`);
    })();
  }, []);

  useEffect(() => {
    if (!title.startsWith("R")) return;
    const name = PHASES[phase];
    void (async () => {
      // An isolated world of the page (an extension's content scripts live in
      // one) must not get the page's scriptlets.
      const isolated = view.current ? await executeJavaScript(view.current, "String(window.__ndScriptlet)", "nd-probe-isolated").catch(String) : "";
      const report = { ...JSON.parse(title.slice(1)), isolatedScriptlet: isolated === "42" };
      console.log(`ND_ADBLOCK_PHASE ${name} blocked=${blocked} ${JSON.stringify(report)}`);
      const next = phase + 1;
      if (next >= PHASES.length) {
        console.log("ND_ADBLOCK_PROBE_DONE");
        return;
      }
      if (PHASES[next] === "siteOff") await webviewEngine.contentBlocking.configure({ disabledSites: ["127.0.0.1"] });
      if (PHASES[next] === "userRule") {
        await webviewEngine.contentBlocking.configure({ disabledSites: [], userRules: "127.0.0.1##.user-hidden" });
      }
      setTitle("");
      setPhase(next);
      setUrl(`${BASE}?phase=${PHASES[next]}`);
    })();
  }, [title]);

  return (
    <window title="adblock probe" defaultWidth={1000} defaultHeight={700}>
      <box orientation="vertical" style={{ vexpand: true }}>
        <label testID="ab-state" text={`phase=${PHASES[phase]} blocked=${blocked}`} />
        <webview
          ref={view}
          testID="ab-view"
          url={url}
          style={{ vexpand: true, hexpand: true }}
          onTitleChanged={(e) => setTitle(e.text)}
          onContentBlocked={(e) => setBlocked((e.data as { count: number }).count)}
          onJavaScriptResult={onJavaScriptResult}
        />
      </box>
    </window>
  );
}

render(<App />);
