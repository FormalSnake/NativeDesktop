import { Portal, moveNode, render, sendCommand } from "@nativedesktop/solid";
import type { NdNodeRef } from "@nativedesktop/solid";
import { Show, createSignal, onSettled } from "solid-js";

// Moving one live `<webview>` between two host windows, driven by
// scripts/mac/cef-reparent-drive.ts. The view is rendered through a Portal into the
// reparent pool so it is never unmounted; `moveNode` relocates only the native widget,
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
  const [view, setView] = createSignal<NdNodeRef<"webview">>();
  const [slotA, setSlotA] = createSignal<NdNodeRef<"box">>();
  const [slotB, setSlotB] = createSignal<NdNodeRef<"box">>();
  const [where, setWhere] = createSignal("a");
  const [secondOpen, setSecondOpen] = createSignal(true);

  // The portal holds the view outside every window until it is placed, so the
  // first placement is the mount.
  onSettled(() => {
    const v = view();
    const a = slotA();
    if (v && a) moveNode(v, a);
  });

  const moveTo = (slot: "a" | "b") => {
    const target = slot === "a" ? slotA() : slotB();
    const v = view();
    if (!v || !target) return;
    moveNode(v, target);
    setWhere(slot);
  };

  return (
    <>
      <window testID="r-window-a" title="ND reparent A" defaultWidth={860} defaultHeight={620}>
        <box orientation="vertical" spacing={4} style={{ padding: 8 }}>
          <label testID="r-where" text={`where=${where()}`} />
          <box orientation="horizontal" spacing={8}>
            <button testID="r-to-a" label="To A" onClick={() => moveTo("a")} />
            <button testID="r-to-b" label="To B" onClick={() => moveTo("b")} />
            <button
              testID="r-devtools"
              label="DevTools"
              onClick={() => {
                const v = view();
                if (v) sendCommand(v, "openDevTools", {});
              }}
            />
            <button testID="r-close-b" label="Close B" onClick={() => setSecondOpen(false)} />
          </box>
          <box testID="r-slot-a" ref={setSlotA} orientation="vertical" style={{ hexpand: true, vexpand: true }} />
        </box>
      </window>
      <Show when={secondOpen()}>
        <window testID="r-window-b" title="ND reparent B" defaultWidth={720} defaultHeight={520}>
          <box orientation="vertical" spacing={4} style={{ padding: 8 }}>
            <label testID="r-b-label" text="window B" />
            <box testID="r-slot-b" ref={setSlotB} orientation="vertical" style={{ hexpand: true, vexpand: true }} />
          </box>
        </window>
      </Show>
      <Portal>
        <webview ref={setView} testID="r-view" url={BASE} style={{ hexpand: true, vexpand: true }} />
      </Portal>
    </>
  );
}

await render(() => <App />);
