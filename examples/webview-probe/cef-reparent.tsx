import { createPortal, moveNode, render, sendCommand, useEffect, useRef, useState } from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";

// Moving one live `<webview>` between two host windows, driven by
// scripts/mac/cef-reparent-drive.ts. The view is rendered into the reparent
// pool so React never unmounts it; `moveNode` relocates only the native widget,
// which is what a tab dragged to another window has to do without reloading the
// page it is showing.
const PAGE = `<!doctype html>
<html><head><meta charset="utf-8" /><title>ND CEF reparent</title>
<style>html,body{margin:0;height:100%}body{background:rgb(0,96,208);color:#fff;font:14px system-ui}
#tall{height:4000px}</style>
</head><body>
<p id="who">reparent fixture</p>
<p><select id="sel"><option value="a">alpha</option><option value="b">bravo</option></select></p>
<p><input id="text" value="" /></p>
<div id="tall"></div>
<script>
  window.__ndFrames = 0;
  (function count() { window.__ndFrames++; requestAnimationFrame(count); })();
  window.__ndCounter = 0;
  setInterval(function () { window.__ndCounter++; }, 50);
  window.__ndNavigations = 0;
  addEventListener("pageshow", function () { window.__ndNavigations++; });
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
  const slotA = useRef<NdNodeRef<"box"> | null>(null);
  const slotB = useRef<NdNodeRef<"box"> | null>(null);
  const [where, setWhere] = useState("a");
  const [secondOpen, setSecondOpen] = useState(true);

  // The portal holds the view outside every window until it is placed, so the
  // first placement is the mount.
  useEffect(() => {
    if (view.current && slotA.current) moveNode(view.current, slotA.current);
  }, []);

  const moveTo = (slot: "a" | "b") => {
    const target = slot === "a" ? slotA.current : slotB.current;
    if (!view.current || !target) return;
    moveNode(view.current, target);
    setWhere(slot);
  };

  return (
    <>
      <window testID="r-window-a" title="ND reparent A" defaultWidth={860} defaultHeight={620}>
        <box orientation="vertical" spacing={4} style={{ padding: 8 }}>
          <label testID="r-where" text={`where=${where}`} />
          <box orientation="horizontal" spacing={8}>
            <button testID="r-to-a" label="To A" onClick={() => moveTo("a")} />
            <button testID="r-to-b" label="To B" onClick={() => moveTo("b")} />
            <button
              testID="r-devtools"
              label="DevTools"
              onClick={() => view.current && sendCommand(view.current, "openDevTools", {})}
            />
            <button testID="r-close-b" label="Close B" onClick={() => setSecondOpen(false)} />
          </box>
          <box testID="r-slot-a" ref={slotA} orientation="vertical" style={{ hexpand: true, vexpand: true }} />
        </box>
      </window>
      {secondOpen && (
        <window testID="r-window-b" title="ND reparent B" defaultWidth={720} defaultHeight={520}>
          <box orientation="vertical" spacing={4} style={{ padding: 8 }}>
            <label testID="r-b-label" text="window B" />
            <box testID="r-slot-b" ref={slotB} orientation="vertical" style={{ hexpand: true, vexpand: true }} />
          </box>
        </window>
      )}
      {createPortal(
        <webview ref={view} testID="r-view" url={BASE} style={{ hexpand: true, vexpand: true }} />,
      )}
    </>
  );
}

await render(<App />);
