import { render, useEffect, useState } from "@nativedesktop/react";

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
  const [url, setUrl] = useState("");
  // A view mounted with no address and armed on a LATER commit. That is the
  // shape a tab, a background page and the framework's own hideOnceArmed
  // pattern all use, and the engine has to load the address even when it
  // arrives while the browser is still being created.
  const [armedUrl, setArmedUrl] = useState("");
  const [armed, setArmed] = useState("pending");
  const [title, setTitle] = useState("");
  const [loading, setLoading] = useState(false);
  const [progress, setProgress] = useState(0);
  const [back, setBack] = useState(false);
  const [forward, setForward] = useState(false);
  const [popup, setPopup] = useState("none");
  useEffect(() => {
    setArmedUrl(`${BASE}?armed=1`);
  }, []);

  return (
    <window title="ND CEF M1" defaultWidth={900} defaultHeight={640}>
      <box orientation="vertical" spacing={4} style={{ padding: 12 }}>
        <label testID="m1-url" text={`url=${url}`} />
        <label testID="m1-title" text={`title=${title}`} />
        <label testID="m1-loading" text={`loading=${loading}`} />
        <label testID="m1-progress" text={`progress=${progress}`} />
        <label testID="m1-nav" text={`nav=${back},${forward}`} />
        <label testID="m1-popup" text={`popup=${popup}`} />
        <label testID="m1-armed" text={`armed=${armed}`} />
        <box orientation="horizontal" style={{ vexpand: true }}>
          <webview
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
            testID="m1-armed-view"
            url={armedUrl}
            onNavigate={(e) => setArmed(e.text)}
          />
        </box>
      </box>
    </window>
  );
}

await render(<App />);
