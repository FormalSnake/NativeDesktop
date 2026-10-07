import { render, Portal, moveNode, executeJavaScript, onJavaScriptResult } from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";
import { Show, createSignal, onSettled } from "solid-js";

// One <webview> moves between two windows
// without reloading. It renders through <Portal> into the off-window pool, so
// its owner never moves and Solid never disposes it; moveNode() relocates the
// live native widget into whichever window's slot should show it. Inserting
// it under a different parent in the Solid tree would remove and recreate the
// widget instead, and the page would reload.
//
// The page counts its own loads and carries a mark set once it has loaded;
// the "check" button reads both back, which is how
// scripts/multiwindow-drive.ts tells a move from a reload.
const URL = process.env.ND_DEMO_URL || "https://formalsnake.dev/";

function App() {
  let tab: NdNodeRef<"webview"> | undefined;
  let slotA: NdNodeRef<"box"> | undefined;
  let slotB: NdNodeRef<"box"> | undefined;
  const [host, setHost] = createSignal<"A" | "B">("A");
  const [loads, setLoads] = createSignal(0);
  const [mark, setMark] = createSignal("unset");

  function show(slot: NdNodeRef<"box"> | undefined, name: "A" | "B") {
    if (!tab || !slot) return;
    moveNode(tab, slot);
    setHost(name);
  }

  async function check() {
    if (!tab) return;
    setMark(await executeJavaScript(tab, "String(window.__ndMark)"));
  }

  // The tab starts in the pool, shown in no window; place it once the native
  // nodes exist.
  onSettled(() => show(slotA, "A"));

  return (
    <>
      <Portal>
        <webview
          ref={(w) => (tab = w)}
          url={URL}
          testID="tab"
          style={{ hexpand: true, vexpand: true }}
          onJavaScriptResult={onJavaScriptResult}
          onLoadingChanged={(e) => {
            if (e.checked || !tab) return;
            // Marked on the first load only: after a reload the mark is gone.
            if (setLoads((n) => n + 1) === 1) void executeJavaScript(tab, "window.__ndMark = 'kept'; 'set'").then(setMark);
          }}
        />
      </Portal>

      <window title="Window A" testID="window-a" defaultWidth={760} defaultHeight={560}>
        <box ref={(b) => (slotA = b)} testID="slot-a" orientation="vertical" spacing={8}>
          <button testID="bring-a" label="Bring tab here" onClick={() => show(slotA, "A")} />
          <button testID="check" label="Check tab" onClick={() => void check()} />
          <label testID="tab-state" text={`loads: ${loads()} mark: ${mark()}`} />
          <Show when={host() !== "A"}>
            <label text="(tab is in Window B)" />
          </Show>
        </box>
      </window>

      <window title="Window B" testID="window-b" defaultWidth={760} defaultHeight={560}>
        <box ref={(b) => (slotB = b)} testID="slot-b" orientation="vertical" spacing={8}>
          <button testID="bring-b" label="Bring tab here" onClick={() => show(slotB, "B")} />
          <Show when={host() !== "B"}>
            <label text="(tab is in Window A)" />
          </Show>
        </box>
      </window>
    </>
  );
}

await render(() => <App />);
