import {
  executeJavaScript,
  installExtension,
  listExtensionActions,
  listExtensions,
  onExtensionActions,
  onExtensionsChanged,
  onExtensionsList,
  setExtensionEnabled,
  uninstallExtension,
  watchExtensions,
  onJavaScriptResult,
  render,
  sendCommand,
  useEffect,
  useRef,
  useState,
  webviewEngine,
} from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";
import { dialogSurfacesRoute } from "../../scripts/fixtures/dialog-surfaces.ts";

// M1 assertion target for the Chromium engine: one <webview engine="chromium">
// renders a real page inside the host's own window, the six create-time events
// flow, and window.open never produces a second top-level window.
//
// Deliberately smaller than examples/webview-probe: everything that probe
// asserts (user scripts, schemes, cookies, find, dialogs) runs on the CDP
// substrate the CEF backend gains in M2, and would report nothing but "not
// wired yet" here. Each check writes its outcome into a label, so
// scripts/cef-drive.ts only reads the tree.
//
// The app serves its own fixture so the gate needs no network.

const PAGE = (title: string, body: string): string =>
  `<!doctype html><html><head><meta charset="utf-8"><title>${title}</title></head>` +
  `<body style="font:16px sans-serif;background:#101014;color:#e8e8ef;margin:0">` +
  `<div style="padding:40px">${body}</div></body></html>`;

/// A 1x1 PNG, scaled by the tag: the context-menu gate needs an image that
/// really decoded, and it has no network.
const PIXEL =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

const fixture = Bun.serve({
  port: 0,
  hostname: "127.0.0.1",
  fetch(request) {
    const path = new URL(request.url).pathname;
    if (path === "/two") {
      // The iframe is the assertion, not decoration: an isolated world gets a
      // context PER FRAME, the child's is created second, and a world-scoped
      // eval that keys on the world name alone lands in it.
      return html(PAGE("ND CEF Two", '<h1>page two</h1><iframe src="/frame" width="80" height="40"></iframe>'));
    }
    if (path === "/frame") {
      return html(PAGE("ND CEF Frame", "<h1>inner frame</h1>"));
    }
    if (path === "/popup") {
      // Page-side JS, not executeJavaScript: the M1 backend has no eval yet,
      // and a popup has to come from the page for on_before_popup to fire.
      return html(
        PAGE(
          "ND CEF Popup",
          "<h1>popup source</h1><script>setTimeout(function(){window.open('/opened','_blank');},400)</script>",
        ),
      );
    }
    if (path === "/opened") {
      return html(PAGE("ND CEF Opened", "<h1>should never render here</h1>"));
    }
    if (path === "/menu") {
      // Absolutely positioned so the context-menu gate can aim at a link, an
      // image, a selection and an editable field by coordinate. The image is
      // an inline PNG: the gate serves no network and an image the renderer
      // failed to load is not an image context.
      return html(
        `<!doctype html><html><head><meta charset="utf-8"><title>ND CEF Menu</title></head>` +
          `<body style="font:16px sans-serif;background:#101014;color:#e8e8ef;margin:0;overflow:hidden">` +
          `<a id="lnk" href="/one" style="position:absolute;left:20px;top:16px">a link</a>` +
          `<img id="img" src="${PIXEL}" width="48" height="48" style="position:absolute;left:150px;top:10px">` +
          `<span id="sel" style="position:absolute;left:250px;top:16px">selected words</span>` +
          `<input id="inp" value="teh wrold" spellcheck="true" style="position:absolute;left:430px;top:12px;width:160px">` +
          `<script>` +
          `window.ndSelect=function(){var r=document.createRange();` +
          `r.selectNodeContents(document.getElementById('sel'));` +
          `var s=getSelection();s.removeAllRanges();s.addRange(r);return 'selected'};` +
          `ndSelect();` +
          `</script></body></html>`,
      );
    }
    const surface = dialogSurfacesRoute(path);
    if (surface) return surface;
    return html(PAGE("ND CEF One", "<h1>ND Chromium engine</h1><p>page one</p>"));
  },
});

function html(body: string): Response {
  return new Response(body, { headers: { "content-type": "text/html; charset=utf-8" } });
}

const BASE = `http://127.0.0.1:${fixture.port}`;

/// The same fixture through a name rather than a literal address. WebAuthn
/// refuses an IP-address origin outright ("relying party ID is not a
/// registrable domain"), so the passkey legs have to be driven from a host
/// name; `localhost` is the only one a gate with no network has.
const LOCAL_BASE = `http://localhost:${fixture.port}`;

/// The scheme the launch path declares through ND_CEF_SCHEMES. The extension
/// origin the browser app uses (nbext://) is declared exactly this way, so this
/// leg is that contract end to end: the origin is made standard during engine
/// startup, in every process, and the app registers the handler for it long
/// afterwards, once views already exist.
const LATE_SCHEME = "ndlate";
const LATE_HTML = PAGE("ND CEF Late", '<h1 id="marker">late-scheme-ok</h1>');

const CHECKS = ["render", "title", "progress", "history", "popup", "lateScheme", "hidden", "reload", "secondWindow", "extensions", "extensionsChanged", "runtimeExtensions", "uninstallExtension", "chromeDialog"] as const;

/// Chrome style is the only one with an extension registry to list, and the
/// launch path sets the same variable the host reads.
const CHROME_STYLE = (process.env.ND_CEF_STYLE ?? "") === "chrome";

/// The gate pass whose driver clicks Chrome's "Remove …?" confirmation. Unset
/// outside the gate, where there is one run and somebody is watching it.
const REGISTRY_PASS = (process.env.ND_CEF_PROBE_PASS ?? "first") === "first";

/// The pass that probes Chromium's own dialogs and bubbles. It renders one view
/// filling the window rather than the scripted legs above: a Views surface is
/// placed against the browser's own bounds, so a view sharing the window with
/// fifteen labels measures nothing the app would ever ship.
const DIALOGS_PASS = (process.env.ND_CEF_PROBE_PASS ?? "") === "dialogs";
type CheckName = (typeof CHECKS)[number];

const received: Record<string, unknown[]> = {};

function record(kind: string, value: unknown): void {
  (received[kind] ??= []).push(value);
}

async function waitFor<T>(kind: string, pred: (v: T) => boolean, what: string, timeoutMs = 20000): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const hit = (received[kind] ?? []).find((v) => pred(v as T)) as T | undefined;
    if (hit !== undefined) return hit;
    if (Date.now() > deadline) {
      throw new Error(`${what}: no matching ${kind} within ${timeoutMs}ms (saw ${JSON.stringify(received[kind] ?? [])})`);
    }
    await new Promise((r) => setTimeout(r, 50));
  }
}

/// One view, the whole window, on the page that asks for every Chromium-drawn
/// surface. What the app was told about each one is written into a label, so
/// the drive can read the app's side and the X server's side of the same event.
function DialogsApp(): React.ReactNode {
  const view = useRef<NdNodeRef<"webview">>(null);
  const [events, setEvents] = useState<string[]>([]);
  const note = (line: string): void => setEvents((prev) => [...prev, line].slice(-12));

  return (
    <window title="ND CEF Dialogs" defaultWidth={1100} defaultHeight={820}>
      <box orientation="vertical" spacing={4} style={{ padding: 8 }}>
        <label testID="dialogs-events" text={`events=${events.join(" | ")}`} />
        <webview
          testID="wv"
          ref={view}
          engine="chromium"
          url={`${LOCAL_BASE}/dialogs`}
          style={{ vexpand: true, hexpand: true }}
          onPermissionRequest={(e) => {
            const d = e.data as { id: string; origin: string; types: string };
            note(`permissionRequest ${d.types}`);
            // Denied, so the page's promise settles and the gate can read the
            // round trip back off it. A real app draws its own sheet here.
            if (view.current) sendCommand(view.current, "respondPermission", { id: d.id, allow: false });
          }}
          onChromeDialog={(e) => {
            const d = e.data as { x: number; y: number; width: number; height: number };
            note(`chromeDialog ${d.width}x${d.height}@${d.x},${d.y}`);
          }}
          onDownloadRequested={(e) => note(`downloadRequested ${JSON.stringify(e.data)}`)}
          onNewWindow={(e) => note(`newWindow ${e.text}`)}
          onJavaScriptResult={onJavaScriptResult}
        />
      </box>
    </window>
  );
}

function App(): React.ReactNode {
  const view = useRef<NdNodeRef<"webview">>(null);
  const late = useRef<NdNodeRef<"webview">>(null);
  const hidden = useRef<NdNodeRef<"webview">>(null);
  const hidden2 = useRef<NdNodeRef<"webview">>(null);
  const hidden3 = useRef<NdNodeRef<"webview">>(null);
  const second = useRef<NdNodeRef<"webview">>(null);
  const extensions = useRef<NdNodeRef<"webview">>(null);
  const [secondOpen, setSecondOpen] = useState(false);
  const [lateReady, setLateReady] = useState(false);
  const [url, setUrl] = useState(`${BASE}/one`);
  const [phase, setPhase] = useState("starting");
  const [results, setResults] = useState<Record<string, string>>({});
  const started = useRef(false);

  const setResult = (name: CheckName, value: string): void =>
    setResults((prev) => ({ ...prev, [name]: value }));

  // Reported on whichever view the dialog was drawn over, which is the one on
  // screen rather than the hidden registry view the command was sent to.
  const onChromeDialogSeen = (e: { data: unknown }): void => {
    const d = e.data as { x: number; y: number; width: number; height: number };
    setResult("chromeDialog", `ok (${d.x},${d.y} ${d.width}x${d.height})`);
  };

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    void run({
      view,
      late,
      hidden,
      hidden2,
      hidden3,
      second,
      extensions,
      setUrl,
      setResult,
      setPhase,
      setLateReady,
      setSecondOpen,
    });
  }, []);

  return (
    <>
    {secondOpen ? (
      <window title="ND CEF Second" testID="second-window" defaultWidth={420} defaultHeight={320}>
        <box orientation="vertical">
          <webview
            testID="wv-second"
            ref={second}
            engine="chromium"
            url={`${BASE}/one`}
            onNavigate={(e) => record("secondNavigate", e.text)}
            onJavaScriptResult={onJavaScriptResult}
          />
        </box>
      </window>
    ) : null}
    <window title="ND CEF Probe" defaultWidth={1000} defaultHeight={700}>
      <box orientation="vertical" spacing={4} style={{ padding: 12 }}>
        <label testID="probe-phase" text={`phase=${phase}`} />
        <label testID="probe-base" text={`base=${BASE}`} />
        {CHECKS.map((name) => (
          <label key={name} testID={`chk-${name}`} text={`${name}=${results[name] ?? "pending"}`} />
        ))}
        <webview
          testID="wv"
          ref={view}
          engine="chromium"
          url={url}
          // A floor, not decoration: the check labels above stack one row per
          // check, and the drives right-click at points inside this view. A
          // check added to the list used to squeeze the view until those points
          // fell outside it.
          style={{ vexpand: true, hexpand: true, minHeight: 200 }}
          onNavigate={(e) => record("navigate", e.text)}
          onTitleChanged={(e) => record("title", e.text)}
          onLoadingChanged={(e) => record("loading", e.checked)}
          onLoadProgress={(e) => record("progress", e.value)}
          onBackAvailable={(e) => record("back", e.checked)}
          onForwardAvailable={(e) => record("forward", e.checked)}
          onNewWindow={(e) => record("newWindow", e.text)}
          onLoadFailed={(e) => record("loadFailed", e.data)}
          onChromeDialog={onChromeDialogSeen}
          onJavaScriptResult={onJavaScriptResult}
        />
        {/* A background tab, which is the shape the bug was found in twice: an
            extension's background page and a tab opened by target=_blank are
            both <webview>s navigated on a page nobody is looking at, so they
            are never mapped and never get an allocation. They still have to
            load. The host trace asserts this one really was unmapped
            (`ND_CEF embed ... mapped=false`). */}
        <tabview selectedIndex={0}>
          <box tabLabel="front" orientation="vertical">
            <label text="front tab" />
          </box>
          <box tabLabel="background" orientation="vertical">
            {/* Three at once, all with their address present in the very
                first commit: that is what restoring a session looks like, and
                the app keeps all but the active one hidden. */}
            <webview
              testID="wv-hidden"
              ref={hidden}
              engine="chromium"
              url={`${BASE}/two`}
              onNavigate={(e) => record("hiddenNavigate", e.text)}
              onTitleChanged={(e) => record("hiddenTitle", e.text)}
              onJavaScriptResult={onJavaScriptResult}
            />
            <webview
              testID="wv-hidden-2"
              ref={hidden2}
              engine="chromium"
              url={`${BASE}/one`}
              onJavaScriptResult={onJavaScriptResult}
            />
            <webview
              testID="wv-hidden-3"
              ref={hidden3}
              engine="chromium"
              url={`${BASE}/popup`}
              onJavaScriptResult={onJavaScriptResult}
            />
            {/* The extension registry lives on chrome://extensions and nowhere
                else, so listExtensions is sent to a view showing it. Created
                with that address rather than navigated to it: Chromium refuses
                a renderer-initiated navigation to a chrome:// page. */}
            <webview
              testID="wv-extensions"
              ref={extensions}
              engine="chromium"
              url={CHROME_STYLE ? "chrome://extensions" : ""}
              onExtensionsList={onExtensionsList}
              onExtensionActions={onExtensionActions}
              onExtensionsChanged={onExtensionsChanged}
              onChromeDialog={onChromeDialogSeen}
            />
          </box>
        </tabview>
        {lateReady ? (
          <webview
            testID="wv-late"
            ref={late}
            engine="chromium"
            url={`${LATE_SCHEME}://probe/index.html`}
            style={{ vexpand: true, hexpand: true }}
            onJavaScriptResult={onJavaScriptResult}
            onSchemeRequest={(e) => {
              const request = e.data as { id: string };
              if (!late.current) return;
              sendCommand(late.current, "respondScheme", {
                id: request.id,
                base64: Buffer.from(LATE_HTML).toString("base64"),
                mime: "text/html",
                status: 200,
              });
            }}
          />
        ) : null}
      </box>
    </window>
    </>
  );
}

async function run(ctx: {
  view: React.RefObject<NdNodeRef<"webview"> | null>;
  late: React.RefObject<NdNodeRef<"webview"> | null>;
  hidden: React.RefObject<NdNodeRef<"webview"> | null>;
  hidden2: React.RefObject<NdNodeRef<"webview"> | null>;
  hidden3: React.RefObject<NdNodeRef<"webview"> | null>;
  second: React.RefObject<NdNodeRef<"webview"> | null>;
  extensions: React.RefObject<NdNodeRef<"webview"> | null>;
  setUrl: (u: string) => void;
  setResult: (name: CheckName, value: string) => void;
  setPhase: (p: string) => void;
  setLateReady: (v: boolean) => void;
  setSecondOpen: (v: boolean) => void;
}): Promise<void> {
  const step = async (name: CheckName, body: () => Promise<string>): Promise<void> => {
    ctx.setPhase(name);
    try {
      ctx.setResult(name, await body());
    } catch (error) {
      ctx.setResult(name, `fail: ${(error as Error).message}`);
    }
  };

  await step("render", async () => {
    const at = await waitFor<string>("navigate", (u) => u.endsWith("/one"), "first navigate");
    await waitFor<boolean>("loading", (v) => v === false, "first load settles");
    return `ok (${at})`;
  });

  await step("title", async () => {
    const t = await waitFor<string>("title", (v) => v === "ND CEF One", "the page title");
    return `ok (${t})`;
  });

  await step("progress", async () => {
    const seen = (received["progress"] ?? []) as number[];
    if (!seen.some((v) => v > 0)) throw new Error(`no positive loadProgress (saw ${JSON.stringify(seen)})`);
    return `ok (max ${Math.max(...seen)})`;
  });

  await step("history", async () => {
    ctx.setUrl(`${BASE}/two`);
    await waitFor<string>("navigate", (u) => u.endsWith("/two"), "second navigate");
    await waitFor<boolean>("back", (v) => v === true, "backAvailable turns on");
    if (!ctx.view.current) throw new Error("no view ref");
    sendCommand(ctx.view.current, "goBack", undefined);
    await waitFor<boolean>("forward", (v) => v === true, "forwardAvailable turns on after goBack");
    return "ok (back and forward availability tracked, goBack applied)";
  });

  await step("popup", async () => {
    ctx.setUrl(`${BASE}/popup`);
    const opened = await waitFor<string>("newWindow", (u) => u.endsWith("/opened"), "window.open routes to newWindow");
    return `ok (${opened})`;
  });

  await step("lateScheme", async () => {
    if ((process.env.ND_CEF_SCHEMES ?? "").split(",").indexOf(LATE_SCHEME) < 0) {
      return `skip: ND_CEF_SCHEMES does not declare ${LATE_SCHEME}`;
    }
    // Registered only now, with a browser already running: the origin came
    // from the launch environment, and this call is only asking for the
    // handler that serves it.
    await webviewEngine.registerScheme(LATE_SCHEME);
    ctx.setLateReady(true);
    for (let i = 0; i < 200; i += 1) {
      if (ctx.late.current) break;
      await new Promise((r) => setTimeout(r, 50));
    }
    if (!ctx.late.current) throw new Error("the late-scheme view never mounted");
    const marker = await poll(
      () =>
        executeJavaScript(
          ctx.late.current!,
          "document.getElementById('marker') ? document.getElementById('marker').textContent : ''",
        ),
      (t) => t === "late-scheme-ok",
      `${LATE_SCHEME}:// page render`,
      20000,
    );
    return `ok (${marker})`;
  });

  // A revisited view, which is a different thing from a fresh one: an isolated
  // world's execution context dies with the document, and a reload is the
  // cheapest way to make a page that already had content scripts get them
  // again. The counter is the assertion: a world that was re-made reads 1, a
  // world whose script never re-ran reads nothing, and a world whose context id
  // went stale answers with an error instead of a value.
  await step("reload", async () => {
    if (!ctx.view.current) throw new Error("no view ref");
    sendCommand(ctx.view.current, "addUserScript", {
      id: "reload-world-mark",
      source: "window.__ndMark = (window.__ndMark || 0) + 1;",
      injectionTime: "start",
      world: "reloadworld",
    });
    sendCommand(ctx.view.current, "addUserScript", {
      id: "reload-page-mark",
      source: 'document.documentElement.setAttribute("data-nd", "marked");',
      injectionTime: "end",
    });
    ctx.setUrl(`${BASE}/two`);
    await waitFor<string>("navigate", (u) => u.endsWith("/two"), "the marked page loads", 30000);

    // At least once, not exactly once: a script is registered with
    // runImmediately, so it can run in the document that is already open AND
    // again as a new-document script. The assertions that matter are that it
    // ran at all, that it ran again after the reload, and that it ran in the
    // main frame rather than the iframe.
    const before = await pollValue(
      () => executeJavaScript(ctx.view.current!, "String(window.__ndMark)", "reloadworld"),
      (v) => Number(v) >= 1,
      "the content script ran in its world",
    );
    // Which frame the world eval landed in. The page has an iframe, so a world
    // keyed by name alone answers from the subframe and this reads top=false.
    const where = await pollValue(
      () =>
        executeJavaScript(
          ctx.view.current!,
          'location.pathname + " top=" + (window.top === window)',
          "reloadworld",
        ),
      (v) => typeof v === "string" && v.includes("top=true"),
      "the world eval targets the main frame, not the iframe",
    );
    if (!String(where).startsWith("/two")) {
      return `fail: the world eval answered from ${JSON.stringify(where)}`;
    }
    await pollValue(
      () => executeJavaScript(ctx.view.current!, 'document.documentElement.getAttribute("data-nd")'),
      (v) => v === "marked",
      "the content script reached the page",
    );

    sendCommand(ctx.view.current, "reload");
    // A fresh document means a fresh world, so the counter is 1 again rather
    // than 2; reading 2 would mean the old world survived, and an error would
    // mean its context id did not.
    const after = await pollValue(
      () => executeJavaScript(ctx.view.current!, "String(window.__ndMark)", "reloadworld"),
      (v) => Number(v) >= 1,
      "the content script ran again after a reload",
    );
    await pollValue(
      () => executeJavaScript(ctx.view.current!, 'document.documentElement.getAttribute("data-nd")'),
      (v) => v === "marked",
      "the reloaded page carries the content script's mark",
    );
    return `ok (world mark ${before} before, ${after} after the reload; world eval on ${where})`;
  });

  await step("hidden", async () => {
    const at = await waitFor<string>(
      "hiddenNavigate",
      (u) => u.endsWith("/two"),
      "a never-shown view navigates",
      30000,
    );
    const title = await waitFor<string>(
      "hiddenTitle",
      (t) => t === "ND CEF Two",
      "a never-shown view reports its title",
      30000,
    );
    // The other half of the report: automation against a hidden view must
    // answer, not fail, which is what the extension drive asks of a
    // background page.
    if (!ctx.hidden.current) throw new Error("the hidden view never mounted");
    // Every restored view has to answer, not just the first: they all attach
    // their devtools agent independently, and a queue that never drains on one
    // of them is the shape a restored session fails in.
    const wanted: Array<[React.RefObject<NdNodeRef<"webview"> | null>, string]> = [
      [ctx.hidden, "/two"],
      [ctx.hidden2, "/one"],
      [ctx.hidden3, "/popup"],
    ];
    const read: string[] = [];
    for (const [ref, path] of wanted) {
      if (!ref.current) throw new Error(`a hidden view for ${path} never mounted`);
      const got = await pollValue(
        () => executeJavaScript(ref.current!, "location.pathname"),
        (v) => v === path,
        `eval against the hidden view on ${path}`,
      );
      read.push(String(got));
    }
    return `ok (${at}, ${title}, evals ${read.join(" ")})`;
  });

  // Closing a window destroys its toplevel surface, and the X server destroys
  // that window's whole child subtree with it, so the webview inside is torn
  // down against XIDs that are already gone. Untrapped, the BadWindow that
  // raises aborted the host: the assertion is that the host is still here
  // afterwards and the view in the other window still answers.
  await step("secondWindow", async () => {
    ctx.setSecondOpen(true);
    await waitFor<string>(
      "secondNavigate",
      (u) => u.endsWith("/one"),
      "the second window's view loads",
      30000,
    );
    ctx.setSecondOpen(false);
    // The host surviving is the point; an eval proves it is still serving.
    if (!ctx.view.current) throw new Error("no view ref");
    const alive = await pollValue(
      () => executeJavaScript(ctx.view.current!, "String(2 + 2)"),
      (v) => v === "4",
      "the host survives closing a window that held a webview",
    );
    return `ok (host alive after the window closed, eval ${alive})`;
  });

  // Chromium's own extension runtime, which only Chrome style has: the gate
  // launches with --load-extension, so the fixture has to come back named,
  // enabled and with the icon the manifest declares.
  await step("extensions", async () => {
    if (!CHROME_STYLE) return "skip: alloy style has no extension registry";
    if (!ctx.extensions.current) throw new Error("no extensions view ref");
    const list = await pollValue(
      () => listExtensions(ctx.extensions.current!),
      (l) => l.length > 0,
      "chrome://extensions reports at least one extension",
    );
    const named = list.map((e) => `${e.name} ${e.enabled ? "enabled" : "disabled"} ${e.iconUrl ? "icon" : "no-icon"}`);
    return `ok (${named.join("; ")})`;
  });

  // The registry is writable at runtime: an unpacked directory installs into
  // the live profile with no relaunch, its action is reported, it disables and
  // enables again, and it uninstalls.
  // The registry's own change events. An app that cannot hear them is left
  // polling for an install that happens entirely inside Chromium; the
  // subscription is made before the registry is touched so the install below
  // is what proves it fires.
  await step("extensionsChanged", async () => {
    if (!CHROME_STYLE) return "skip: alloy style has no extension registry";
    if (!REGISTRY_PASS) return "skip: the registry legs run in the pass that answers Chrome's confirmation";
    const view = ctx.extensions.current;
    if (!view) throw new Error("no extensions view ref");
    const sources = await watchExtensions(view, (change) => record("extensionsChanged", change.reason));
    if (sources.length === 0) throw new Error("watchExtensions attached to nothing");
    return `ok (${sources.join(", ")})`;
  });

  await step("runtimeExtensions", async () => {
    // The uninstall leg ends at Chrome's own confirmation, which only this
    // pass's driver answers; leaving it up for a later pass would put a dialog
    // over the view before that pass had done anything.
    const why = !CHROME_STYLE
      ? "skip: alloy style has no extension registry"
      : REGISTRY_PASS
        ? ""
        : "skip: the registry legs run in the pass that answers Chrome's confirmation";
    if (why) {
      ctx.setResult("uninstallExtension", why);
      ctx.setResult("chromeDialog", why);
      return why;
    }
    const view = ctx.extensions.current;
    if (!view) throw new Error("no extensions view ref");
    const path = `${process.cwd()}/scripts/fixtures/chrome-ext-runtime`;
    const installed = await installExtension(view, path);
    const mine = installed.find((e) => e.name === "ND Runtime Extension");
    if (!mine) throw new Error(`installExtension did not register it: ${installed.map((e) => e.name).join(", ")}`);
    // The subscription made above has to have heard the install that just
    // happened; an app hears a Web Store install the same way.
    const reason = await waitFor<string>("extensionsChanged", () => true, "the registry reports the install");

    const actions = await listExtensionActions(view);
    const action = actions.find((a) => a.id === mine.id);
    if (!action) throw new Error(`no action for ${mine.id} among ${actions.length}`);
    if (action.title !== "ND Runtime") throw new Error(`action title ${action.title}`);
    if (!action.popupUrl.endsWith("/popup.html")) throw new Error(`action popup ${action.popupUrl}`);
    if (!action.iconUrl.endsWith("/icon16.png")) throw new Error(`action icon ${action.iconUrl}`);

    const disabled = await setExtensionEnabled(view, mine.id, false);
    if (disabled.find((e) => e.id === mine.id)?.enabled !== false) throw new Error("setExtensionEnabled(false) did not take");
    const enabled = await setExtensionEnabled(view, mine.id, true);
    if (enabled.find((e) => e.id === mine.id)?.enabled !== true) throw new Error("setExtensionEnabled(true) did not take");

    // Not awaited: Chrome puts its own "Remove …?" confirmation up and the
    // promise settles when that is answered, which is somebody else's click.
    uninstallExtension(view, mine.id).then(
      (left) => ctx.setResult(
        "uninstallExtension",
        left.some((e) => e.id === mine.id) ? "fail: still installed" : `ok (${mine.id} removed)`,
      ),
      (error: Error) => ctx.setResult("uninstallExtension", `fail: ${error.message}`),
    );
    return `ok (installed ${mine.id}, change ${reason}, action ${action.title}, disabled, enabled, uninstall asked)`;
  });

  // The app's own items, which the engine appends to Chromium's model after a
  // separator. Left in place for the rest of the run so the context-menu gate
  // can right-click and read them back.
  sendCommand(ctx.view.current, "setContextMenuItems", {
    items: [
      { id: "probe-any", label: "Probe Item", contexts: ["all"] },
      { id: "probe-link", label: "Probe Link Item", contexts: ["link"] },
      {
        id: "probe-group",
        label: "Probe Submenu",
        contexts: ["all"],
        children: [
          { id: "probe-alpha", label: "Probe Alpha" },
          { id: "probe-beta", label: "Probe Beta", type: "checkbox", checked: true },
        ],
      },
    ],
  });

  ctx.setPhase("done");
}

/// Polls a value, treating a thrown eval (a document mid-navigation, a world
/// with no context yet) as "not yet" rather than as a failure.
async function pollValue<T>(
  fn: () => Promise<T>,
  pred: (v: T) => boolean,
  what: string,
  timeoutMs = 30000,
): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  let last: unknown = "(never ran)";
  for (;;) {
    try {
      const value = await fn();
      last = value;
      if (pred(value)) return value;
    } catch (error) {
      last = `threw ${String(error)}`;
    }
    if (Date.now() > deadline) {
      throw new Error(`${what} never held within ${timeoutMs}ms (last: ${JSON.stringify(last)})`);
    }
    await new Promise((r) => setTimeout(r, 150));
  }
}

async function poll<T>(
  fn: () => Promise<T>,
  pred: (v: T) => boolean,
  what: string,
  timeoutMs: number,
): Promise<T> {
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
    if (Date.now() > deadline) {
      throw new Error(`${what} never held within ${timeoutMs}ms (last: ${JSON.stringify(last)})`);
    }
    await new Promise((r) => setTimeout(r, 100));
  }
}

await render(DIALOGS_PASS ? <DialogsApp /> : <App />);
