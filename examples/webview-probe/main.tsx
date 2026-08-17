import {
  Platform,
  executeJavaScript,
  getCookies,
  onCookiesResult,
  onJavaScriptResult,
  onSessionSaved,
  render,
  saveSession,
  sendCommand,
  setContextMenuItems,
  useEffect,
  useRef,
  useState,
  webviewEngine,
} from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";

// Assertion target for the webview browser/extension surface: user scripts,
// script messages, isolated worlds, custom URI schemes, cookies, per-view
// profiles, favicons, find-in-page, TLS state, link hover, context menus,
// audio and session state. Every check writes its outcome into a label the
// automation socket can read (scripts/webview-drive.ts), so the drive script
// never has to cooperate with app internals beyond reading the tree.
//
// The app hosts its own HTTP fixture (Bun.serve) so the probe has a stable
// origin with a favicon, findable text and a link, and it answers the custom
// scheme itself — the whole round trip stays in one process.

const FAVICON_PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  "base64",
);

const FIXTURE_HTML = `<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <title>ND Probe Fixture</title>
    <link rel="icon" type="image/png" sizes="32x32" href="/favicon.png" />
  </head>
  <body>
    <h1>ND webview probe</h1>
    <p id="p">needle one needle two needle three</p>
    <a id="lnk" href="https://example.invalid/target">hover target</a>
    <audio id="a"></audio>
  </body>
</html>`;

const SCHEME_HTML = `<!doctype html>
<html><head><meta charset="utf-8" /><title>ND Probe Scheme</title></head>
<body><div id="marker">scheme-ok</div></body></html>`;

const fixture = Bun.serve({
  port: 0,
  hostname: "127.0.0.1",
  fetch(request) {
    const path = new URL(request.url).pathname;
    if (path === "/favicon.png") {
      return new Response(FAVICON_PNG, { headers: { "content-type": "image/png" } });
    }
    if (path === "/download") {
      return new Response("probe payload", {
        headers: {
          "content-type": "application/octet-stream",
          "content-disposition": 'attachment; filename="probe-download.bin"',
        },
      });
    }
    return new Response(FIXTURE_HTML, { headers: { "content-type": "text/html; charset=utf-8" } });
  },
});
const BASE = `http://127.0.0.1:${fixture.port}`;

// --- event bus ---------------------------------------------------------------
// Events land on props and are appended here; the probe sequence polls for the
// first entry matching a predicate, so an event that fired before the step
// started (favicon, securityChanged) is never missed.

const received: Record<string, unknown[]> = {};

function record(kind: string, value: unknown): void {
  (received[kind] ??= []).push(value);
}

async function waitForEvent<T>(kind: string, pred: (v: T) => boolean, timeoutMs = 10000): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const hit = (received[kind] ?? []).find((v) => pred(v as T)) as T | undefined;
    if (hit !== undefined) return hit;
    if (Date.now() > deadline) throw new Error(`no ${kind} event within ${timeoutMs}ms`);
    await new Promise((r) => setTimeout(r, 50));
  }
}

async function poll<T>(fn: () => Promise<T>, pred: (v: T) => boolean, what: string, timeoutMs = 10000): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  let last: unknown = "(never ran)";
  for (;;) {
    try {
      const value = await fn();
      last = value;
      if (pred(value)) return value;
    } catch (error) {
      last = String(error);
    }
    if (Date.now() > deadline) throw new Error(`${what} never held within ${timeoutMs}ms (last: ${JSON.stringify(last)})`);
    await new Promise((r) => setTimeout(r, 100));
  }
}

const CHECKS = [
  "userScriptStart",
  "worldIsolation",
  "removeUserScript",
  "scriptMessagePage",
  "scriptMessageWorld",
  "scheme",
  "cookies",
  "profileIsolation",
  "favicon",
  "find",
  "security",
  "audio",
  "session",
  "download",
  "focus",
  "linkHover",
  "contextMenu",
  "contextMenuItems",
] as const;
type CheckName = (typeof CHECKS)[number];

function App(): React.ReactNode {
  const main = useRef<NdNodeRef<"webview">>(null);
  const priv = useRef<NdNodeRef<"webview">>(null);
  const scheme = useRef<NdNodeRef<"webview">>(null);
  const started = useRef(false);
  const [schemeReady, setSchemeReady] = useState(false);
  const [phase, setPhase] = useState("starting");
  const [results, setResults] = useState<Record<string, string>>({});

  const setResult = (name: CheckName, value: string): void =>
    setResults((prev) => ({ ...prev, [name]: value }));

  // Scheme registration has to land before the first <webview> mounts, so the
  // views stay unrendered until it resolves.
  useEffect(() => {
    webviewEngine
      .registerScheme("ndprobe")
      .then(() => setSchemeReady(true))
      .catch((error: Error) => {
        setResult("scheme", `fail: ${error.message}`);
        setSchemeReady(true);
      });
  }, []);

  useEffect(() => {
    if (!schemeReady || started.current) return;
    started.current = true;
    void runProbe({ main, priv, scheme, setResult, setPhase });
  }, [schemeReady]);

  return (
    <window title="ND WebView Probe" defaultWidth={900} defaultHeight={640}>
      <box orientation="vertical" spacing={4} style={{ padding: 12 }}>
        <label testID="probe-phase" text={`phase=${phase}`} />
        <label testID="probe-base" text={`base=${BASE}`} />
        {CHECKS.map((name) => (
          <label key={name} testID={`chk-${name}`} text={`${name}=${results[name] ?? "pending"}`} />
        ))}
        {schemeReady ? (
          <box orientation="horizontal" spacing={8} style={{ vexpand: true }}>
            <webview
              ref={main}
              testID="wv-main"
              url=""
              contextMenuMode="suppress"
              onJavaScriptResult={onJavaScriptResult}
              onCookiesResult={onCookiesResult}
              onSessionSaved={onSessionSaved}
              onScriptMessage={(e) => record("scriptMessage", e.data)}
              onFaviconChanged={(e) => record("faviconChanged", e.data)}
              onFindResult={(e) => record("findResult", e.data)}
              onSecurityChanged={(e) => record("securityChanged", e.data)}
              onAudioStateChanged={(e) => record("audioStateChanged", e.data)}
              onLinkHover={(e) => record("linkHover", e.text)}
              onContextMenu={(e) => record("contextMenu", e.data)}
              onContextMenuItemClicked={(e) => record("contextMenuItemClicked", e.data)}
              onCookiesChanged={() => record("cookiesChanged", true)}
              onDownloadRequested={(e) => record("downloadRequested", { view: "main", ...(e.data as object) })}
            />
            <webview
              ref={priv}
              testID="wv-private"
              profile="private"
              url={`${BASE}/`}
              onJavaScriptResult={onJavaScriptResult}
              onCookiesResult={onCookiesResult}
            />
            <webview
              ref={scheme}
              testID="wv-scheme"
              url="ndprobe://probe/index.html"
              onJavaScriptResult={onJavaScriptResult}
              // The download check needs a SECOND view on the same (default)
              // network session as wv-main: the signal lives on the session,
              // so a per-view connection would report this view's copy too.
              onDownloadRequested={(e) => record("downloadRequested", { view: "scheme", ...(e.data as object) })}
              onSchemeRequest={(e) => {
                const request = e.data as { id: string; url: string; scheme: string };
                record("schemeRequest", request);
                if (!scheme.current) return;
                sendCommand(scheme.current, "respondScheme", {
                  id: request.id,
                  base64: Buffer.from(SCHEME_HTML).toString("base64"),
                  mime: "text/html",
                  status: 200,
                });
              }}
            />
          </box>
        ) : null}
      </box>
    </window>
  );
}

interface ProbeArgs {
  main: React.RefObject<NdNodeRef<"webview"> | null>;
  priv: React.RefObject<NdNodeRef<"webview"> | null>;
  scheme: React.RefObject<NdNodeRef<"webview"> | null>;
  setResult: (name: CheckName, value: string) => void;
  setPhase: (value: string) => void;
}

async function runProbe({ main, priv, scheme, setResult, setPhase }: ProbeArgs): Promise<void> {
  const gtk = Platform.backend === "gtk";
  const wv = await poll(async () => main.current, (v) => v != null, "webview ref");
  const wvScheme = await poll(async () => scheme.current, (v) => v != null, "scheme webview ref");
  const wvPrivate = await poll(async () => priv.current, (v) => v != null, "private webview ref");

  const step = async (name: CheckName, fn: () => Promise<string>): Promise<void> => {
    setPhase(name);
    try {
      setResult(name, await fn());
    } catch (error) {
      setResult(name, `fail: ${(error as Error).message}`);
    }
  };

  // Everything that must run before the first byte of the page: user scripts
  // and message-handler registration.
  sendCommand(wv!, "addUserScript", {
    id: "early",
    // readyState is "loading" only at document_start — a document_end script
    // would record "interactive", so this value IS the ordering assertion.
    source: 'window.__ndEarly = document.readyState;',
    injectionTime: "start",
  });
  sendCommand(wv!, "addUserScript", {
    id: "isolated",
    source: 'window.__ndWorld = "world-ok";',
    injectionTime: "start",
    world: "probe",
  });
  sendCommand(wv!, "addUserScript", {
    id: "removable",
    source: 'window.__ndRemovable = "yes";',
    injectionTime: "end",
  });
  sendCommand(wv!, "addUserScript", {
    id: "poster-page",
    source: 'window.webkit.messageHandlers.ndprobe.postMessage({ from: "page", n: 42 });',
    injectionTime: "end",
  });
  sendCommand(wv!, "addUserScript", {
    id: "poster-world",
    source: 'window.webkit.messageHandlers.ndprobeiso.postMessage({ from: "world", n: 43 });',
    injectionTime: "end",
    world: "probe",
  });
  sendCommand(wv!, "registerScriptMessage", { name: "ndprobe" });
  sendCommand(wv!, "registerScriptMessage", { name: "ndprobeiso", world: "probe" });

  setPhase("loading");
  sendCommand(wv!, "stop");
  // Navigating from JS rather than the url prop keeps the setup commands
  // strictly ahead of the first load.
  await executeJavaScript(wv!, `location.href = ${JSON.stringify(`${BASE}/`)}`).catch(() => "");
  await poll(() => executeJavaScript(wv!, "document.title"), (t) => t === "ND Probe Fixture", "fixture page load");

  await step("userScriptStart", async () => {
    const value = await executeJavaScript(wv!, "window.__ndEarly");
    return value === "loading" ? "ok" : `fail: readyState at injection was ${JSON.stringify(value)}`;
  });

  await step("worldIsolation", async () => {
    const inPage = await executeJavaScript(wv!, "typeof window.__ndWorld");
    const inWorld = await executeJavaScript(wv!, "window.__ndWorld", "probe");
    if (inPage !== "undefined") return `fail: isolated variable leaked into the page world (${inPage})`;
    return inWorld === "world-ok" ? "ok" : `fail: world eval returned ${JSON.stringify(inWorld)}`;
  });

  await step("scriptMessagePage", async () => {
    const message = await waitForEvent<{ name: string; world: string; body: { from?: string; n?: number } }>(
      "scriptMessage",
      (m) => m.name === "ndprobe",
    );
    if (message.world !== "") return `fail: page handler reported world ${JSON.stringify(message.world)}`;
    return message.body?.from === "page" && message.body?.n === 42
      ? "ok"
      : `fail: body was ${JSON.stringify(message.body)}`;
  });

  await step("scriptMessageWorld", async () => {
    const message = await waitForEvent<{ name: string; world: string; body: { from?: string } }>(
      "scriptMessage",
      (m) => m.name === "ndprobeiso",
    );
    if (message.world !== "probe") return `fail: isolated handler reported world ${JSON.stringify(message.world)}`;
    return message.body?.from === "world" ? "ok" : `fail: body was ${JSON.stringify(message.body)}`;
  });

  await step("removeUserScript", async () => {
    const before = await executeJavaScript(wv!, "window.__ndRemovable");
    if (before !== "yes") return `fail: script never ran (${JSON.stringify(before)})`;
    sendCommand(wv!, "removeUserScript", { id: "removable" });
    await executeJavaScript(wv!, `location.href = ${JSON.stringify(`${BASE}/?v=2`)}`).catch(() => "");
    await poll(() => executeJavaScript(wv!, "location.search"), (s) => s === "?v=2", "second navigation");
    const after = await executeJavaScript(wv!, "typeof window.__ndRemovable");
    return after === "undefined" ? "ok" : `fail: removed script still ran (${after})`;
  });

  await step("scheme", async () => {
    await poll(
      () => executeJavaScript(wvScheme!, "document.getElementById('marker') ? document.getElementById('marker').textContent : ''"),
      (t) => t === "scheme-ok",
      "custom-scheme page render",
    );
    return "ok";
  });

  await step("cookies", async () => {
    sendCommand(wv!, "setCookie", { name: "ndprobe", value: "v1", domain: "127.0.0.1", path: "/" });
    await poll(
      () => getCookies(wv!, `${BASE}/`),
      (list) => list.some((c) => c.name === "ndprobe" && c.value === "v1"),
      "cookie visible after setCookie",
    );
    sendCommand(wv!, "deleteCookie", { name: "ndprobe", domain: "127.0.0.1", path: "/" });
    await poll(
      () => getCookies(wv!, `${BASE}/`),
      (list) => !list.some((c) => c.name === "ndprobe"),
      "cookie gone after deleteCookie",
    );
    return "ok";
  });

  await step("profileIsolation", async () => {
    sendCommand(wv!, "setCookie", { name: "ndshared", value: "v2", domain: "127.0.0.1", path: "/" });
    await poll(
      () => getCookies(wv!, `${BASE}/`),
      (list) => list.some((c) => c.name === "ndshared"),
      "cookie visible in the default profile",
    );
    const isolated = await getCookies(wvPrivate!, `${BASE}/`);
    sendCommand(wv!, "deleteCookie", { name: "ndshared", domain: "127.0.0.1", path: "/" });
    return isolated.some((c) => c.name === "ndshared")
      ? "fail: the ephemeral profile saw the default profile's cookie"
      : "ok";
  });

  await step("favicon", async () => {
    const icon = await waitForEvent<{ dataUrl?: string; iconUrl?: string }>(
      "faviconChanged",
      (e) => Boolean(e.dataUrl || e.iconUrl),
    );
    return icon.dataUrl ? "ok" : `ok (iconUrl ${icon.iconUrl})`;
  });

  await step("find", async () => {
    sendCommand(wv!, "findStart", { text: "needle", caseSensitive: false, wrap: true });
    const result = await waitForEvent<{ matchFound: boolean; matchCount?: number; done: boolean }>(
      "findResult",
      (e) => e.done,
    );
    if (!result.matchFound) return "fail: findStart reported no match for a word that is on the page";
    const counted = (received.findResult ?? []).find(
      (e) => typeof (e as { matchCount?: number }).matchCount === "number" && (e as { matchCount: number }).matchCount > 0,
    ) as { matchCount: number } | undefined;
    sendCommand(wv!, "findNext");
    sendCommand(wv!, "findStop");
    return counted ? `ok (${counted.matchCount} matches)` : "ok (no count on this engine)";
  });

  await step("security", async () => {
    const state = await waitForEvent<{ secure: boolean; insecureContent: boolean }>(
      "securityChanged",
      () => true,
    );
    return state.secure === false ? "ok" : "fail: plain http reported as secure";
  });

  await step("audio", async () => {
    sendCommand(wv!, "setMuted", true);
    const muted = await waitForEvent<{ playing: boolean; muted: boolean }>("audioStateChanged", (e) => e.muted);
    sendCommand(wv!, "setMuted", false);
    return muted.muted ? "ok" : "fail: setMuted did not report a muted state";
  });

  await step("session", async () => {
    const state = await saveSession(wv!);
    if (!state) return "fail: saveSession returned an empty state";
    sendCommand(wv!, "restoreSession", { state });
    return `ok (${state.length} base64 chars)`;
  });

  // Regression guard for the multi-view download crash: `download-started`
  // lives on the NETWORK SESSION, and wv-main and wv-scheme share the default
  // one. Connecting per view made a single download emit once per live view
  // and cancel the same WebKitDownload once per handler, which segfaulted the
  // host in DownloadProxy::cancel. Exactly one event, from the view that
  // asked, is the assertion.
  await step("download", async () => {
    const before = (received.downloadRequested ?? []).length;
    await executeJavaScript(wv!, `location.href = ${JSON.stringify(`${BASE}/download`)}`).catch(() => "");
    await waitForEvent<unknown>("downloadRequested", () => true);
    // Let every other live view's handler run before counting; a per-view
    // connection would have delivered its duplicate well inside this window.
    await new Promise((r) => setTimeout(r, 1500));
    const events = (received.downloadRequested ?? []).slice(before) as {
      view?: string;
      url?: string;
      suggestedFilename?: string;
    }[];
    if (events.length !== 1) {
      return `fail: ${events.length} downloadRequested events for one download (${JSON.stringify(events.map((e) => e.view))})`;
    }
    const [event] = events;
    if (event.view !== "main") return `fail: event routed to the ${event.view} view`;
    if (!event.url?.endsWith("/download")) return `fail: url was ${JSON.stringify(event.url)}`;
    if (event.suggestedFilename !== "probe-download.bin") {
      return `fail: suggestedFilename was ${JSON.stringify(event.suggestedFilename)}`;
    }
    return "ok (1 event, named, from the requesting view)";
  });

  // `focus` is the cross-cutting widget command: no synthetic input on either
  // backend, so this is the only way a drive can put the keyboard somewhere.
  // The app sends it; the automation socket's a11y probe reports the result,
  // which scripts/webview-drive.ts asserts independently.
  await step("focus", async () => {
    sendCommand(wv!, "focus");
    return "ok (sent; the drive asserts the a11y focused flag)";
  });

  // Hover and context menu are engine-native on GTK (mouse-target-changed /
  // context-menu), which need a real pointer — GTK4 removed app-constructible
  // events, so the automation socket answers -32003 and there is nothing to
  // synthesize. On AppKit both ride this framework's own page-side agent, so a
  // JS-dispatched event exercises the whole path end to end.
  await step("linkHover", async () => {
    if (gtk) return "skip: GTK hover needs a real pointer (no synthetic input on GTK4)";
    await executeJavaScript(
      wv!,
      "document.getElementById('lnk').dispatchEvent(new MouseEvent('mouseover', { bubbles: true })), 1",
    );
    const url = await waitForEvent<string>("linkHover", (u) => u.includes("example.invalid"));
    return url.includes("example.invalid") ? "ok" : `fail: hovered url was ${url}`;
  });

  await step("contextMenu", async () => {
    if (gtk) return "skip: GTK context menu needs a real right-click (no synthetic input on GTK4)";
    await executeJavaScript(
      wv!,
      "document.getElementById('lnk').dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 11, clientY: 22 })), 1",
    );
    const menu = await waitForEvent<{ link?: string; editable: boolean }>(
      "contextMenu",
      (e) => Boolean(e.link),
    );
    return menu.link?.includes("example.invalid") ? "ok" : `fail: context menu link was ${menu.link}`;
  });

  // Neither backend can be made to OPEN a menu from a script: GTK4 synthesises
  // no pointer input, and WKWebView's menu comes from a real right-click, so
  // what is provable here is the command round trip: the host takes the tree,
  // parses it, and stores it against this view. `ND_WEBVIEW_TRACE` prints the
  // stored count and scripts/headless-webview.sh asserts that line.
  await step("contextMenuItems", async () => {
    setContextMenuItems(wv!, [
      { id: "probe-link", label: "Open link in new tab", contexts: ["link"] },
      { id: "probe-image", label: "Save image", contexts: ["image"], targetUrlGlobs: ["*.png"] },
      { type: "separator" },
      {
        id: "probe-group",
        label: "Probe",
        contexts: ["all"],
        children: [
          { id: "probe-a", label: "Alpha" },
          { id: "probe-b", label: "Beta", type: "checkbox", checked: true },
        ],
      },
    ]);
    // The command is one-way; give the host a beat to apply it before the probe
    // declares itself done and the drive tears the run down.
    await new Promise((r) => setTimeout(r, 200));
    return "ok: 4 items sent (the host-side trace asserts they were stored)";
  });

  setPhase("done");
}

await render(<App />);
