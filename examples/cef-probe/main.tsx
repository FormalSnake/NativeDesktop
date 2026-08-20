import {
  executeJavaScript,
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
      return html(PAGE("ND CEF Two", "<h1>page two</h1>"));
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

const CHECKS = ["render", "title", "progress", "history", "popup", "lateScheme"] as const;
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
    void run({ view, late, setUrl, setResult, setPhase, setLateReady });
  }, []);

  return (
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
        />
        {lateReady ? (
          <webview
            testID="wv-late"
            ref={late}
            engine="chromium"
            url={`${LATE_SCHEME}://probe/index.html`}
            style={{ vexpand: true, hexpand: true }}
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
  );
}

async function run(ctx: {
  view: React.RefObject<NdNodeRef<"webview"> | null>;
  late: React.RefObject<NdNodeRef<"webview"> | null>;
  setUrl: (u: string) => void;
  setResult: (name: CheckName, value: string) => void;
  setPhase: (p: string) => void;
  setLateReady: (v: boolean) => void;
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

  ctx.setPhase("done");
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
