import { render, sendCommand, useRef, useState } from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";

// Zoom gate (ND_CEF_STYLE=chrome), driven by scripts/cef-zoom-drive.ts on both
// backends. Every way a page's zoom can change has to reach the app as
// `zoomChanged`, and Chromium's own zoom bubble must not be placed over the
// page: this browser has no location bar for it to anchor to.
const PAGE = `<!doctype html>
<html><head><meta charset="utf-8" /><title>ND CEF zoom</title>
<style>body{font:20px system-ui;margin:0;padding:24px;background:#fff;color:#111}</style>
</head><body>
<h1>zoom</h1>
<p id="dpr"></p>
<script>
  function report() { document.title = "dpr=" + devicePixelRatio.toFixed(3); }
  addEventListener("resize", report);
  report();
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
  const [zoom, setZoom] = useState("none");
  const [changes, setChanges] = useState(0);
  const send = (name: string, arg?: unknown) => {
    if (view.current) sendCommand(view.current, name, arg);
  };
  return (
    <window title="ND CEF zoom" defaultWidth={900} defaultHeight={640}>
      <box orientation="vertical" spacing={4} style={{ padding: 12 }}>
        <label testID="z-title" text={`title=${title}`} />
        <label testID="z-zoom" text={`zoom=${zoom} changes=${changes}`} />
        <box orientation="horizontal" spacing={8}>
          <button testID="z-set150" label="150%" onClick={() => send("setZoom", 1.5)} />
          <button testID="z-set100" label="100%" onClick={() => send("setZoom", 1)} />
        </box>
        <webview
          ref={view}
          testID="z-view"
          url={BASE}
          style={{ vexpand: true, hexpand: true }}
          onTitleChanged={(e) => setTitle(String(e.text ?? ""))}
          onZoomChanged={(e) => {
            const data = e.data as { factor: number; source: string };
            setZoom(`${data.factor.toFixed(2)}/${data.source}`);
            setChanges((n) => n + 1);
          }}
        />
      </box>
    </window>
  );
}

await render(<App />);
