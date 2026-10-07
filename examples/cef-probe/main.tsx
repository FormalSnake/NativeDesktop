import {
  executeJavaScript,
  getCookies,
  installExtension,
  listExtensionActions,
  listExtensions,
  readExtensionAction,
  triggerExtensionAction,
  onExtensionActions,
  onExtensionsChanged,
  onExtensionsList,
  setExtensionEnabled,
  uninstallExtension,
  watchExtensions,
  onJavaScriptResult,
  onCookiesResult,
  onSessionSaved,
  render,
  saveSession,
  sendCommand,
  webviewEngine,
} from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";
import { For, Show, createSignal, createStore, onCleanup, onSettled } from "solid-js";
import {
  answerPermission,
  dialogSurfacesRoute,
  permissionNavigated,
  permissionWithdrawn,
} from "../../scripts/fixtures/dialog-surfaces.ts";
import type { PermissionPayload } from "../../scripts/fixtures/dialog-surfaces.ts";

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

const CHECKS = ["render", "title", "progress", "history", "popup", "lateScheme", "hidden", "reload", "secondWindow", "closedBrowser", "removedNode", "extensions", "twoRegistryViews", "extensionsChanged", "runtimeActionState", "actionClick", "installExtensionError", "runtimeExtensions", "uninstallExtension", "uninstallSilent"] as const;

/// Chrome style is the only one with an extension registry to list, and the
/// launch path sets the same variable the host reads.
const CHROME_STYLE = (process.env.ND_CEF_STYLE ?? "") === "chrome";

/// The gate pass that runs the registry legs, which change the profile the
/// later passes reuse. Unset outside the gate, where there is one run.
const REGISTRY_PASS = (process.env.ND_CEF_PROBE_PASS ?? "first") === "first";

/// The pass that probes Chromium's own dialogs and bubbles. It renders one view
/// filling the window rather than the scripted legs above: a Views surface is
/// placed against the browser's own bounds, so a view sharing the window with
/// fifteen labels measures nothing the app would ever ship.
const DIALOGS_PASS = (process.env.ND_CEF_PROBE_PASS ?? "") === "dialogs";
/// The legs that mount extra views run once. GTK grows the window to fit them
/// and never shrinks it back, and the devtools pass sizes the window itself.
const FIRST_PASS = (process.env.ND_CEF_PROBE_PASS ?? "first") === "first";
type CheckName = (typeof CHECKS)[number];

const received: Record<string, unknown[]> = {};

const VIEWS = ["view", "late", "hidden", "hidden2", "hidden3", "second", "extensions", "actionPage", "closing", "remount", "registrySmall", "registryTab"] as const;
type ViewName = (typeof VIEWS)[number];

/// The live native node of each probe view, read by the scripted legs below.
/// Null while the view is unmounted: the closing and remount legs wait on it.
const views = Object.fromEntries(VIEWS.map((name) => [name, null])) as Record<ViewName, NdNodeRef<"webview"> | null>;

function bind(name: ViewName): (node: NdNodeRef<"webview">) => void {
  return (node) => {
    views[name] = node;
    onCleanup(() => {
      if (views[name] === node) views[name] = null;
    });
  };
}

/// Reported on whichever view a dialog was drawn over. uninstallExtension
/// must not raise Chrome's "Remove ...?" confirmation: the app asks instead.
const chromeDialogs: string[] = [];
const onChromeDialogSeen = (e: { data: unknown }): void => {
  const d = e.data as { x: number; y: number; width: number; height: number };
  chromeDialogs.push(`${d.x},${d.y} ${d.width}x${d.height}`);
};

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
function DialogsApp() {
  let view: NdNodeRef<"webview"> | undefined;
  const [events, setEvents] = createSignal<string[]>([]);
  const note = (line: string): void => {
    setEvents((prev) => [...prev, line].slice(-12));
  };

  return (
    <window title="ND CEF Dialogs" defaultWidth={1100} defaultHeight={820}>
      <box orientation="vertical" spacing={4} style={{ padding: 8 }}>
        <label testID="dialogs-events" text={`events=${events().join(" | ")}`} />
        <webview
          testID="wv"
          ref={(n) => (view = n)}
          engine="chromium"
          url={`${LOCAL_BASE}/dialogs`}
          style={{ vexpand: true, hexpand: true }}
          onNavigate={(e) => {
            if (view) permissionNavigated(view, e.text);
          }}
          onPermissionRequest={(e) => {
            const d = e.data as PermissionPayload;
            note(`permissionRequest ${d.types}`);
            answerPermission(view ?? null, d);
          }}
          onPermissionRequestDismissed={(e) => {
            const d = e.data as { id: string };
            note(`permissionDismissed ${d.id}`);
            permissionWithdrawn(view ?? null, d.id);
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

function App() {
  const [closingKey, setClosingKey] = createSignal(0);
  const [remountKey, setRemountKey] = createSignal(0);
  const [registryPair, setRegistryPair] = createSignal(false);
  const [secondOpen, setSecondOpen] = createSignal(false);
  const [lateReady, setLateReady] = createSignal(false);
  const [actionPopupUrl, setActionPopupUrl] = createSignal("");
  const [url, setUrl] = createSignal(`${BASE}/one`);
  const [phase, setPhase] = createSignal("starting");
  const [results, setResults] = createStore<Record<string, string>>({});

  const setResult = (name: CheckName, value: string): void =>
    setResults((draft) => {
      draft[name] = value;
    });

  onSettled(() => {
    void run({
      setClosingKey,
      setRemountKey,
      setRegistryPair,
      setActionPopupUrl,
      setUrl,
      setResult,
      setPhase,
      setLateReady,
      setSecondOpen,
    });
  });

  return (
    <>
    <Show when={secondOpen()}>
      <window title="ND CEF Second" testID="second-window" defaultWidth={420} defaultHeight={320}>
        <box orientation="vertical">
          <webview
            testID="wv-second"
            ref={bind("second")}
            engine="chromium"
            url={`${BASE}/one`}
            onNavigate={(e) => record("secondNavigate", e.text)}
            onJavaScriptResult={onJavaScriptResult}
          />
        </box>
      </window>
    </Show>
    <window title="ND CEF Probe" defaultWidth={1000} defaultHeight={700}>
      <box orientation="vertical" spacing={4} style={{ padding: 12 }}>
        <label testID="probe-phase" text={`phase=${phase()}`} />
        <label testID="probe-base" text={`base=${BASE}`} />
        <For each={CHECKS}>
          {(name) => <label testID={`chk-${name}`} text={`${name}=${results[name] ?? "pending"}`} />}
        </For>
        <webview
          testID="wv"
          ref={bind("view")}
          engine="chromium"
          url={url()}
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
          onExtensionActions={onExtensionActions}
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
            <Show when={closingKey() || undefined} keyed>
              {(_key) => (
                <webview
                  testID="wv-closing"
                  ref={bind("closing")}
                  engine="chromium"
                  url={`${BASE}/one`}
                  style={{ minHeight: 60 }}
                  onJavaScriptResult={onJavaScriptResult}
                />
              )}
            </Show>
            <Show when={remountKey() || undefined} keyed>
              {(_key) => (
                <webview
                  testID="wv-remount"
                  ref={bind("remount")}
                  engine="chromium"
                  url={`${BASE}/one`}
                  style={{ minHeight: 60 }}
                  onJavaScriptResult={onJavaScriptResult}
                  onCookiesResult={onCookiesResult}
                  onSessionSaved={onSessionSaved}
                />
              )}
            </Show>
            {/* The browser's own shape: a registry view kept at 2x2 on
                chrome://extensions and a tab the user opened on the same
                page. */}
            <Show when={registryPair()}>
              <box orientation="vertical">
                <webview
                  testID="wv-registry-small"
                  ref={bind("registrySmall")}
                  engine="chromium"
                  url="chrome://extensions"
                  style={{ minWidth: 2, minHeight: 2 }}
                  onJavaScriptResult={onJavaScriptResult}
                  onExtensionsList={onExtensionsList}
                />
                <webview
                  testID="wv-registry-tab"
                  ref={bind("registryTab")}
                  engine="chromium"
                  url="chrome://extensions"
                  style={{ minHeight: 200 }}
                  onJavaScriptResult={onJavaScriptResult}
                  onExtensionsList={onExtensionsList}
                />
              </box>
            </Show>
          </box>
          <box tabLabel="background" orientation="vertical">
            {/* Three at once, all with their address present in the very
                first commit: that is what restoring a session looks like, and
                the app keeps all but the active one hidden. */}
            <webview
              testID="wv-hidden"
              ref={bind("hidden")}
              engine="chromium"
              url={`${BASE}/two`}
              onNavigate={(e) => record("hiddenNavigate", e.text)}
              onTitleChanged={(e) => record("hiddenTitle", e.text)}
              onJavaScriptResult={onJavaScriptResult}
            />
            <webview
              testID="wv-hidden-2"
              ref={bind("hidden2")}
              engine="chromium"
              url={`${BASE}/one`}
              onJavaScriptResult={onJavaScriptResult}
            />
            <webview
              testID="wv-hidden-3"
              ref={bind("hidden3")}
              engine="chromium"
              url={`${BASE}/popup`}
              onJavaScriptResult={onJavaScriptResult}
            />
            {/* The extension registry lives on chrome://extensions and nowhere
                else, so listExtensions is sent to a view showing it. Created
                with that address rather than navigated to it: Chromium refuses
                a renderer-initiated navigation to a chrome:// page. */}
            {/* A page of the action fixture, which is the only context
                Chromium tells an action's runtime state to. Created with the
                address rather than navigated to it: Chromium refuses a
                renderer-initiated navigation to an extension page. */}
            <Show when={actionPopupUrl()}>
              <webview
                testID="wv-action"
                ref={bind("actionPage")}
                engine="chromium"
                url={actionPopupUrl()}
                onExtensionActions={onExtensionActions}
                onJavaScriptResult={onJavaScriptResult}
              />
            </Show>
            <webview
              testID="wv-extensions"
              ref={bind("extensions")}
              engine="chromium"
              url={CHROME_STYLE ? "chrome://extensions" : ""}
              onExtensionsList={onExtensionsList}
              onExtensionActions={onExtensionActions}
              onExtensionsChanged={onExtensionsChanged}
              onChromeDialog={onChromeDialogSeen}
            />
          </box>
        </tabview>
        <Show when={lateReady()}>
          <webview
            testID="wv-late"
            ref={bind("late")}
            engine="chromium"
            url={`${LATE_SCHEME}://probe/index.html`}
            style={{ vexpand: true, hexpand: true }}
            onJavaScriptResult={onJavaScriptResult}
            onSchemeRequest={(e) => {
              const request = e.data as { id: string };
              if (!views.late) return;
              sendCommand(views.late, "respondScheme", {
                id: request.id,
                base64: Buffer.from(LATE_HTML).toString("base64"),
                mime: "text/html",
                status: 200,
              });
            }}
          />
        </Show>
      </box>
    </window>
    </>
  );
}

async function run(ctx: {
  setClosingKey: (k: number) => void;
  setRemountKey: (k: number) => void;
  setRegistryPair: (on: boolean) => void;
  setActionPopupUrl: (u: string) => void;
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
    if (!views.view) throw new Error("no view ref");
    sendCommand(views.view, "goBack", undefined);
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
      if (views.late) break;
      await new Promise((r) => setTimeout(r, 50));
    }
    if (!views.late) throw new Error("the late-scheme view never mounted");
    const marker = await poll(
      () =>
        executeJavaScript(
          views.late!,
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
    if (!views.view) throw new Error("no view ref");
    sendCommand(views.view, "addUserScript", {
      id: "reload-world-mark",
      source: "window.__ndMark = (window.__ndMark || 0) + 1;",
      injectionTime: "start",
      world: "reloadworld",
    });
    sendCommand(views.view, "addUserScript", {
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
      () => executeJavaScript(views.view!, "String(window.__ndMark)", "reloadworld"),
      (v) => Number(v) >= 1,
      "the content script ran in its world",
    );
    // Which frame the world eval landed in. The page has an iframe, so a world
    // keyed by name alone answers from the subframe and this reads top=false.
    const where = await pollValue(
      () =>
        executeJavaScript(
          views.view!,
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
      () => executeJavaScript(views.view!, 'document.documentElement.getAttribute("data-nd")'),
      (v) => v === "marked",
      "the content script reached the page",
    );

    sendCommand(views.view, "reload");
    // A fresh document means a fresh world, so the counter is 1 again rather
    // than 2; reading 2 would mean the old world survived, and an error would
    // mean its context id did not.
    const after = await pollValue(
      () => executeJavaScript(views.view!, "String(window.__ndMark)", "reloadworld"),
      (v) => Number(v) >= 1,
      "the content script ran again after a reload",
    );
    await pollValue(
      () => executeJavaScript(views.view!, 'document.documentElement.getAttribute("data-nd")'),
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
    if (!views.hidden) throw new Error("the hidden view never mounted");
    // Every restored view has to answer, not just the first: they all attach
    // their devtools agent independently, and a queue that never drains on one
    // of them is the shape a restored session fails in.
    const wanted: Array<[ViewName, string]> = [
      ["hidden", "/two"],
      ["hidden2", "/one"],
      ["hidden3", "/popup"],
    ];
    const read: string[] = [];
    for (const [name, path] of wanted) {
      if (!views[name]) throw new Error(`a hidden view for ${path} never mounted`);
      const got = await pollValue(
        () => executeJavaScript(views[name]!, "location.pathname"),
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
    if (!views.view) throw new Error("no view ref");
    const alive = await pollValue(
      () => executeJavaScript(views.view!, "String(2 + 2)"),
      (v) => v === "4",
      "the host survives closing a window that held a webview",
    );
    return `ok (host alive after the window closed, eval ${alive})`;
  });

  // A browser Chromium closes on its own, here the page's own window.close()
  // on a tab with one history entry, while the app keeps sending it
  // DevTools calls. The view's host reference goes when the browser does, and
  // a call already on its way to the CEF UI thread with that host must still
  // land on a live object: this crashed the host at startup on x11 and wlr.
  await step("closedBrowser", async () => {
    if (!FIRST_PASS) return "skip: runs in the first pass";
    let settled = 0;
    for (let round = 1; round <= 5; round++) {
      ctx.setClosingKey(round);
      await until(() => views.closing !== null, `round ${round}: the view mounts`);
      const node = views.closing!;
      await pollValue(() => executeJavaScript(node, "location.pathname"), (v) => v === "/one", `round ${round}: the view loads`);
      const burst: Promise<unknown>[] = [];
      burst.push(executeJavaScript(node, "setTimeout(() => window.close(), 20); 'closing'"));
      const stopAt = Date.now() + 2500;
      while (Date.now() < stopAt) {
        for (let i = 0; i < 4; i++) burst.push(executeJavaScript(node, "document.title"));
        await new Promise((r) => setTimeout(r, 10));
      }
      const outcomes = await Promise.allSettled(burst.map((p) => withTimeout(p, 15000)));
      const unanswered = outcomes.filter((o) => o.status === "rejected" && String(o.reason).includes("no answer within")).length;
      if (unanswered > 0) throw new Error(`round ${round}: ${unanswered} of ${outcomes.length} calls never answered`);
      settled += outcomes.length;
    }
    ctx.setClosingKey(0);
    if (!views.view) throw new Error("no view ref");
    const alive = await pollValue(
      () => executeJavaScript(views.view!, "String(2 + 2)"),
      (v) => v === "4",
      "the host survives browsers closing under in-flight calls",
    );
    return `ok (${settled} calls settled over 5 closes, eval ${alive})`;
  });

  // A command to a view that is gone has to be answered, never dropped: the
  // app sent one to a node a keyed remount had just replaced and the host
  // stopped answering altogether. Two shapes, both with the three commands that
  // settle a promise: sent after the node was removed, and in flight while it
  // is being torn down.
  await step("removedNode", async () => {
    if (!FIRST_PASS) return "skip: runs in the first pass";
    ctx.setRemountKey(1);
    await until(() => views.remount !== null, "the remount view mounts");
    const first = views.remount!;
    await pollValue(() => executeJavaScript(first, "location.pathname"), (v) => v === "/one", "the remount view loads");
    ctx.setRemountKey(2);
    await until(() => views.remount !== null && views.remount.id !== first.id, "the keyed remount lands");
    const removed = await settleAll(first);
    const second = views.remount!;
    await pollValue(() => executeJavaScript(second, "location.pathname"), (v) => v === "/one", "the new view loads");
    const pending = [
      executeJavaScript(second, "new Promise((r) => setTimeout(() => r('late'), 2000))"),
      getCookies(second),
      saveSession(second),
    ];
    ctx.setRemountKey(0);
    const teardown = await settleWithin(pending, 10000);
    const silent = [...removed, ...teardown].filter((o) => o === "silent").length;
    if (silent > 0) throw new Error(`${silent} command(s) never answered (removed: ${removed.join(",")}; mid-teardown: ${teardown.join(",")})`);
    if (!views.view) throw new Error("no view ref");
    const alive = await executeJavaScript(views.view, "String(2 + 2)");
    return `ok (removed: ${removed.join(",")}; mid-teardown: ${teardown.join(",")}; eval ${alive})`;
  });

  // Chromium's own extension runtime, which only Chrome style has: the gate
  // launches with --load-extension, so the fixture has to come back named,
  // enabled and with the icon the manifest declares.
  await step("extensions", async () => {
    if (!CHROME_STYLE) return "skip: alloy style has no extension registry";
    if (!views.extensions) throw new Error("no extensions view ref");
    const list = await pollValue(
      () => listExtensions(views.extensions!),
      (l) => l.length > 0,
      "chrome://extensions reports at least one extension",
    );
    const named = list.map((e) => `${e.name} ${e.enabled ? "enabled" : "disabled"} ${e.iconUrl ? "icon" : "no-icon"}`);
    return `ok (${named.join("; ")})`;
  });

  // Two views on chrome://extensions, one of them the app's 2x2 registry view,
  // once took the host to where getTree stopped answering. Both have to load,
  // both have to answer, and the page the user is looking at has to as well.
  await step("twoRegistryViews", async () => {
    if (!CHROME_STYLE) return "skip: alloy style has no extension registry";
    if (!FIRST_PASS) return "skip: runs in the first pass";
    ctx.setRegistryPair(true);
    await until(() => views.registrySmall !== null && views.registryTab !== null, "both registry views mount");
    const small = views.registrySmall!;
    const tab = views.registryTab!;
    for (const [name, node] of [["small", small], ["tab", tab]] as const) {
      await pollValue(() => executeJavaScript(node, "location.href"), (v) => v.startsWith("chrome://extensions"), `the ${name} view loads`);
    }
    // Long enough for a focus fight to run away: each round trip moved X input
    // focus, so thousands pile up in seconds.
    await new Promise((r) => setTimeout(r, 4000));
    const [fromSmall, fromTab] = await settleValues([listExtensions(small), listExtensions(tab)], 10000);
    if (!views.view) throw new Error("no view ref");
    const [alive] = await settleValues([executeJavaScript(views.view, "String(2 + 2)")], 5000);
    ctx.setRegistryPair(false);
    const counts = [fromSmall, fromTab].map((v) => (Array.isArray(v) ? `${v.length} extension(s)` : String(v)));
    if (counts.some((c) => !c.endsWith("extension(s)")) || alive !== "4") {
      throw new Error(`small: ${counts[0]}; tab: ${counts[1]}; main view eval: ${String(alive)}`);
    }
    return `ok (small: ${counts[0]}; tab: ${counts[1]}; eval ${alive})`;
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
    if (!REGISTRY_PASS) return "skip: the registry legs run in the first pass";
    const view = views.extensions;
    if (!view) throw new Error("no extensions view ref");
    const sources = await watchExtensions(view, (change) => record("extensionsChanged", change.reason));
    if (sources.length === 0) throw new Error("watchExtensions attached to nothing");
    return `ok (${sources.join(", ")})`;
  });

  // What an action really is right now, which the manifest cannot say. The
  // fixture declares a popup and its worker turns it off, the way 1Password
  // does while no account is configured; an app that reads the manifest off
  // disk opens a popup the extension has switched off and shows a document it
  // never meant to be on screen.
  await step("runtimeActionState", async () => {
    if (!CHROME_STYLE) return "skip: alloy style has no extension registry";
    if (!REGISTRY_PASS) return "skip: the registry legs run in the first pass";
    const registry = views.extensions;
    if (!registry) throw new Error("no extensions view ref");

    const actions = await pollValue(
      () => listExtensionActions(registry),
      (list) => list.some((a) => a.name === "ND Action Extension"),
      "the action fixture is registered",
    );
    const manifest = actions.find((a) => a.name === "ND Action Extension")!;
    if (!manifest.popupUrl.endsWith("/popup.html")) throw new Error(`manifest popup ${manifest.popupUrl}`);
    if (manifest.title !== "ND Action Manifest") throw new Error(`manifest title ${manifest.title}`);

    ctx.setActionPopupUrl(manifest.popupUrl);
    const page = await pollValue(
      async () => views.actionPage,
      (ref) => ref !== null,
      "the action fixture's page is mounted",
    );

    // Off, though the manifest says otherwise. This is the whole leg: an app
    // must not open a popup for an action in this state.
    const cleared = await pollValue(
      () => readExtensionAction(page!),
      (state) => state.id === manifest.id,
      "the action page answers for its own extension",
    );
    if (cleared.popupUrl !== "") throw new Error(`runtime popup is ${JSON.stringify(cleared.popupUrl)}, want "" `);
    if (cleared.title !== "ND Action Runtime") throw new Error(`runtime title ${cleared.title}`);

    // Per tab, on the tab Chromium calls active: the badge is set for that one
    // and a different one is set as the default, so an answer carrying "T1"
    // can only have come from the per-tab value.
    await executeJavaScript(page!, `(async () => {
      const active = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
      if (!active.length) throw new Error("chromium reports no active tab");
      await chrome.action.setBadgeText({ text: "D" });
      await chrome.action.setBadgeText({ tabId: active[0].id, text: "T1" });
      await chrome.action.setPopup({ popup: "popup.html" });
      return String(active[0].id);
    })()`);

    const perTab = await pollValue(
      () => readExtensionAction(page!),
      (state) => state.badgeText === "T1" && state.popupUrl.endsWith("/popup.html"),
      "the badge and popup set at runtime are read back for the active tab",
    );
    if (perTab.tabId === 0) throw new Error("no active tab in the answer");
    return `ok (manifest ${manifest.popupUrl.slice(-11)}, runtime "" then popup.html, badge ${perTab.badgeText} on tab ${perTab.tabId})`;
  });

  // A click on an action with no popup, the way Chrome's toolbar button does
  // it: onClicked with this tab, and an activeTab grant on it. The fixture has
  // no host permissions, so its executeScript only reaches the page because
  // the click granted it.
  await step("actionClick", async () => {
    if (!CHROME_STYLE) return "skip: alloy style has no extension registry";
    if (!REGISTRY_PASS) return "skip: the registry legs run in the first pass";
    const registry = views.extensions;
    const page = views.view;
    if (!registry || !page) throw new Error("no view refs");
    const actions = await pollValue(
      () => listExtensionActions(registry),
      (list) => list.some((a) => a.name === "ND Click Extension"),
      "the click fixture is registered",
    );
    const click = actions.find((a) => a.name === "ND Click Extension")!;
    if (click.popupUrl !== "") throw new Error(`the click fixture declares a popup: ${click.popupUrl}`);
    try {
      await triggerExtensionAction(page, click.id);
    } catch (error) {
      const message = (error as Error).message;
      if (message.includes("no Chrome toolbar")) return `skip: ${message}`;
      throw error;
    }
    const mark = await pollValue(
      () => executeJavaScript(page, "document.documentElement.dataset.ndActionClicked || ''"),
      (text) => text.includes(BASE) || text.includes(LOCAL_BASE),
      "onClicked marks the page through activeTab",
    );
    return `ok (${mark})`;
  });

  // An install that cannot work has to answer. The command runs a promise on
  // chrome://extensions behind a directory chooser this engine answers itself,
  // and every step of that can stall; a promise left unsettled reads to the app
  // as the whole registry being dead.
  await step("installExtensionError", async () => {
    if (!CHROME_STYLE) return "skip: alloy style has no extension registry";
    if (!REGISTRY_PASS) return "skip: the registry legs run in the first pass";
    const view = views.extensions;
    if (!view) throw new Error("no extensions view ref");
    const started = Date.now();
    try {
      await installExtension(view, `${process.cwd()}/scripts/fixtures/not-an-extension`);
    } catch (error) {
      return `ok (rejected in ${Date.now() - started}ms: ${(error as Error).message.slice(0, 80)})`;
    }
    throw new Error("installExtension resolved for a directory that is not an extension");
  });

  await step("runtimeExtensions", async () => {
    const why = !CHROME_STYLE
      ? "skip: alloy style has no extension registry"
      : REGISTRY_PASS
        ? ""
        : "skip: the registry legs run in the first pass";
    if (why) {
      ctx.setResult("uninstallExtension", why);
      ctx.setResult("uninstallSilent", why);
      return why;
    }
    const view = views.extensions;
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

    const dialogsBefore = chromeDialogs.length;
    const left = await uninstallExtension(view, mine.id);
    ctx.setResult(
      "uninstallExtension",
      left.some((e) => e.id === mine.id) ? "fail: still installed" : `ok (${mine.id} removed)`,
    );
    await new Promise((r) => setTimeout(r, 1500));
    const dialogs = chromeDialogs.slice(dialogsBefore);
    ctx.setResult("uninstallSilent", dialogs.length === 0 ? `ok (no Chrome dialog; ${dialogsBefore} earlier in the run)` : `fail: Chrome dialog at ${dialogs.join(", ")}`);
    return `ok (installed ${mine.id}, change ${reason}, action ${action.title}, disabled, enabled, removed)`;
  });

  // The app's own items, which the engine appends to Chromium's model after a
  // separator. Left in place for the rest of the run so the context-menu gate
  // can right-click and read them back.
  sendCommand(views.view!, "setContextMenuItems", {
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

async function until(pred: () => boolean, what: string, timeoutMs = 15000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!pred()) {
    if (Date.now() > deadline) throw new Error(`${what} never held within ${timeoutMs}ms`);
    await new Promise((r) => setTimeout(r, 50));
  }
}

/// A call the host never answers fails the leg rather than hanging it.
function withTimeout<T>(p: Promise<T>, ms: number): Promise<T> {
  return Promise.race([
    p,
    new Promise<T>((_, reject) => setTimeout(() => reject(new Error(`no answer within ${ms}ms`)), ms)),
  ]);
}

function settleAll(node: NdNodeRef<"webview">): Promise<string[]> {
  return settleWithin([executeJavaScript(node, "1"), getCookies(node), saveSession(node)], 10000);
}

/// Each promise's value, or "rejected: ..." / "silent" when it failed or had
/// not settled by the deadline.
async function settleValues(promises: Promise<unknown>[], timeoutMs: number): Promise<unknown[]> {
  const deadline = new Promise<string>((r) => setTimeout(() => r("silent"), timeoutMs));
  return Promise.all(promises.map((p) => Promise.race([p.catch((e: unknown) => `rejected: ${String(e)}`), deadline])));
}

/// How each promise ended: "rejected", "resolved", or "silent" when it had not
/// settled by the deadline.
async function settleWithin(promises: Promise<unknown>[], timeoutMs: number): Promise<string[]> {
  const outcomes = promises.map((p) =>
    p.then(
      () => "resolved",
      () => "rejected",
    ),
  );
  const deadline = new Promise<string>((r) => setTimeout(() => r("silent"), timeoutMs));
  return Promise.all(outcomes.map((o) => Promise.race([o, deadline])));
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

await render(() => (DIALOGS_PASS ? <DialogsApp /> : <App />));
