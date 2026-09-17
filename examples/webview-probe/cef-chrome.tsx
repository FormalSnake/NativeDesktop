import { render, sendCommand, setContextMenuItems, useEffect, useRef, useState } from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";

// Chrome style gate (ND_CEF_STYLE=chrome), driven by scripts/cef-chrome-drive.ts.
// Chrome style is the only style that carries Chromium's extension runtime, and
// it can only be created in a CEF Views window, so every assertion here is
// about that window never becoming something the user can see: the buttons are
// the paths that would otherwise open one.
const PAGE = `<!doctype html>
<html><head><meta charset="utf-8" /><title>ND CEF chrome style</title>
<style>body{font:20px system-ui;margin:0;padding:24px;background:#123;color:#fff}
a{color:#9cf}</style>
</head><body>
<h1 id="h">chrome style</h1>
<p><a id="blank" href="https://example.invalid/blank" target="_blank">target=_blank</a></p>
<p><select id="sel"><option>one</option><option>two</option></select></p>
<p><input id="text" value="typed:" /></p>
<script>
  var frames = 0;
  function tick() {
    if (++frames < 30) return requestAnimationFrame(tick);
    document.title = "painted vp=" + innerWidth + "x" + innerHeight;
  }
  requestAnimationFrame(tick);
  window.__ndFrames = 0;
  (function count() { window.__ndFrames++; requestAnimationFrame(count); })();
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
  const view = useRef<NdNodeRef<"webview"> | null>(null);
  const [title, setTitle] = useState("");
  const [popup, setPopup] = useState("none");
  const [wide, setWide] = useState(false);
  const [devtools, setDevtools] = useState("closed");
  useEffect(() => {
    if (!view.current) return;
    // Merged into Chromium's own menu; the host trace reports what the model
    // received and whether it received it while the callback was still running.
    setContextMenuItems(view.current, [
      { id: "c-alpha", label: "Alpha", contexts: ["all"] },
      { id: "c-beta", label: "Beta", contexts: ["all"] },
      { id: "c-gamma", label: "Gamma", contexts: ["all"] },
    ]);
  }, []);
  return (
    <window title="ND CEF chrome" defaultWidth={1000} defaultHeight={700}>
      <box orientation="vertical" spacing={4} style={{ padding: 12 }}>
        <label testID="c-title" text={`title=${title}`} />
        <label testID="c-popup" text={`popup=${popup}`} />
        <label testID="c-devtools" text={`devtools=${devtools}`} />
        <box orientation="horizontal" spacing={8}>
          <button
            testID="c-devtools-open"
            label="DevTools"
            onClick={() => {
              if (!view.current) return;
              sendCommand(view.current, "openDevTools", {});
              setDevtools("requested");
            }}
          />
          <button testID="c-resize" label="Resize" onClick={() => setWide((w) => !w)} />
          <textinput testID="c-field" value="native" />
        </box>
        <box orientation="horizontal" style={{ vexpand: true }}>
          <webview
            ref={view}
            testID="c-view"
            url={BASE}
            style={{ hexpand: true, minWidth: wide ? 700 : 400 }}
            onTitleChanged={(e) => setTitle(e.text)}
            onNewWindow={(e) => setPopup(e.text)}
          />
        </box>
      </box>
    </window>
  );
}

await render(<App />);
