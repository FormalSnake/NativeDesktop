import { render, sendCommand, setContextMenuItems } from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";
import { createSignal, onSettled } from "solid-js";

// M1 gate for the Chromium engine on macOS. Everything it asserts arrives on
// the schema's own event surface, so the same file runs on either engine:
//
//   navigate / titleChanged / loadingChanged / loadProgress / back+forward
//   newWindow, with the app's window count unchanged after window.open
//
// The paint check rides the title channel because no pixel capture is
// available without a Screen Recording grant: the page counts 30 rAF frames
// before reporting, and a browser that is not composited into a visible view
// never gets them. It reports the h1's laid-out box and the viewport with it,
// which is also how the embedded view proves it is sized to its NSView.
const PAGE = `<!doctype html>
<html><head><meta charset="utf-8" /><title>ND CEF M1</title>
<style>body{font:28px system-ui;margin:0;padding:40px;background:#0b5cff;color:#fff}</style>
</head><body>
<h1 id="h">CEF renders here</h1>
<p>chromium engine, embedded in the AppKit host</p>
<iframe id="f" srcdoc="<p>inner</p>" style="width:200px;height:60px;border:0"></iframe>
<script>
  var frames = 0;
  function tick() {
    if (++frames < 30) return requestAnimationFrame(tick);
    var r = document.getElementById("h").getBoundingClientRect();
    document.title = "painted rAF=" + frames
      + " h1=" + Math.round(r.width) + "x" + Math.round(r.height)
      + " vp=" + innerWidth + "x" + innerHeight
      + " dpr=" + devicePixelRatio;
    setTimeout(function () { window.open("https://example.invalid/popup", "_blank"); }, 500);
  }
  requestAnimationFrame(tick);
</script>
</body></html>`;

const fixture = Bun.serve({
  port: 0,
  hostname: "127.0.0.1",
  fetch() {
    return new Response(PAGE, { headers: { "content-type": "text/html; charset=utf-8" } });
  },
});
const BASE = `http://127.0.0.1:${fixture.port}/`;

function App() {
  const [url, setUrl] = createSignal("");
  // A view mounted with no address and armed on a LATER commit. That is the
  // shape a tab, a background page and the framework's own hideOnceArmed
  // pattern all use, and the engine has to load the address even when it
  // arrives while the browser is still being created.
  const [armedUrl, setArmedUrl] = createSignal("");
  const [armed, setArmed] = createSignal("pending");
  const [view, setView] = createSignal<NdNodeRef<"webview">>();
  const [armedView, setArmedView] = createSignal<NdNodeRef<"webview">>();
  const [title, setTitle] = createSignal("");
  const [loading, setLoading] = createSignal(false);
  const [progress, setProgress] = createSignal(0);
  const [back, setBack] = createSignal(false);
  const [forward, setForward] = createSignal(false);
  const [popup, setPopup] = createSignal("none");
  onSettled(() => {
    // Injected into every frame, so the page and its iframe both report a
    // context for world "probe". A world cache keyed by name alone lets the
    // iframe's win, and every world-scoped eval then runs in the iframe.
    const armedNode = armedView();
    if (armedNode) {
      sendCommand(armedNode, "addUserScript", {
        id: "m1-world",
        source: 'window.__ndFrameIsTop = String(window.top === window.self);',
        injectionTime: "start",
        world: "probe",
        allFrames: true,
      });
    }
    setArmedUrl(`${BASE}?armed=1`);
  });

  // The engine menu leg: these have to reach Chromium's own model while
  // on_before_context_menu is still on the stack, so the drive right-clicks
  // the view and the host trace reports what the model received.
  onSettled(() => {
    const v = view();
    if (!v) return;
    setContextMenuItems(v, [
      { id: "m1-alpha", label: "Alpha", contexts: ["all"] },
      { id: "m1-beta", label: "Beta", contexts: ["all"] },
      { id: "m1-gamma", label: "Gamma", contexts: ["all"] },
    ]);
  });

  return (
    <window title="ND CEF M1" defaultWidth={900} defaultHeight={640}>
      <box orientation="vertical" spacing={4} style={{ padding: 12 }}>
        <label testID="m1-url" text={`url=${url()}`} />
        <label testID="m1-title" text={`title=${title()}`} />
        <label testID="m1-loading" text={`loading=${loading()}`} />
        <label testID="m1-progress" text={`progress=${progress()}`} />
        <label testID="m1-nav" text={`nav=${back()},${forward()}`} />
        <label testID="m1-popup" text={`popup=${popup()}`} />
        <label testID="m1-armed" text={`armed=${armed()}`} />
        <box orientation="horizontal" style={{ vexpand: true }}>
          <webview
            ref={setView}
            testID="m1-view"
            url={BASE}
            onNavigate={(e) => setUrl(e.text)}
            onTitleChanged={(e) => setTitle(e.text)}
            onLoadingChanged={(e) => setLoading(e.checked)}
            onLoadProgress={(e) => setProgress(e.value)}
            onBackAvailable={(e) => setBack(e.checked)}
            onForwardAvailable={(e) => setForward(e.checked)}
            onNewWindow={(e) => setPopup(e.text)}
          />
          <webview
            ref={setArmedView}
            testID="m1-armed-view"
            url={armedUrl()}
            onNavigate={(e) => setArmed(e.text)}
          />
        </box>
      </box>
    </window>
  );
}

await render(() => <App />);
