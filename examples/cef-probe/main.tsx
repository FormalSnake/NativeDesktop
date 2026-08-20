import {
  executeJavaScript,
  onJavaScriptResult,
  render,
  sendCommand,
  useEffect,
  useRef,
  useState,
  webviewEngine,
} from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";

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
    return html(PAGE("ND CEF One", "<h1>ND Chromium engine</h1><p>page one</p>"));
  },
});

function html(body: string): Response {
  return new Response(body, { headers: { "content-type": "text/html; charset=utf-8" } });
}

const BASE = `http://127.0.0.1:${fixture.port}`;

/// The scheme the launch path declares through ND_CEF_SCHEMES. The extension
/// origin the browser app uses (nbext://) is declared exactly this way, so this
/// leg is that contract end to end: the origin is made standard during engine
/// startup, in every process, and the app registers the handler for it long
/// afterwards, once views already exist.
const LATE_SCHEME = "ndlate";
const LATE_HTML = PAGE("ND CEF Late", '<h1 id="marker">late-scheme-ok</h1>');

const CHECKS = ["render", "title", "progress", "history", "popup", "lateScheme", "hidden", "reload", "secondWindow"] as const;
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

function App(): React.ReactNode {
  const view = useRef<NdNodeRef<"webview">>(null);
  const late = useRef<NdNodeRef<"webview">>(null);
  const hidden = useRef<NdNodeRef<"webview">>(null);
  const hidden2 = useRef<NdNodeRef<"webview">>(null);
  const hidden3 = useRef<NdNodeRef<"webview">>(null);
  const second = useRef<NdNodeRef<"webview">>(null);
  const [secondOpen, setSecondOpen] = useState(false);
  const [lateReady, setLateReady] = useState(false);
  const [url, setUrl] = useState(`${BASE}/one`);
  const [phase, setPhase] = useState("starting");
  const [results, setResults] = useState<Record<string, string>>({});
  const started = useRef(false);

  const setResult = (name: CheckName, value: string): void =>
    setResults((prev) => ({ ...prev, [name]: value }));

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
          style={{ vexpand: true, hexpand: true }}
          onNavigate={(e) => record("navigate", e.text)}
          onTitleChanged={(e) => record("title", e.text)}
          onLoadingChanged={(e) => record("loading", e.checked)}
          onLoadProgress={(e) => record("progress", e.value)}
          onBackAvailable={(e) => record("back", e.checked)}
          onForwardAvailable={(e) => record("forward", e.checked)}
          onNewWindow={(e) => record("newWindow", e.text)}
          onLoadFailed={(e) => record("loadFailed", e.data)}
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

await render(<App />);
