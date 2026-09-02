import { render, createPortal, moveNode, useEffect, useRef, useState } from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";

// Drag-a-tab-between-windows, without reloading the page. The <webview> lives in
// the off-window POOL (createPortal), so its React fiber never changes parent —
// React therefore never unmounts it, and its loaded page / scroll / JS state are
// never torn down. Only the LIVE native widget is relocated, by moveNode(), into
// whichever window's content <box> ("slot") should show it. A plain re-parent in
// the React tree would unmount+remount the subtree (the host turns that into
// remove+create → the page reloads); that is exactly what the pool + moveNode
// avoid.
//
// Set ND_DEMO_AUTOMOVE=1 to have the tab ping-pong A→B→A on its own (used by the
// runtime proof); otherwise use the per-window "Bring tab here" button.
const AUTO = typeof process !== "undefined" && process.env?.ND_DEMO_AUTOMOVE === "1";

function App(): React.ReactNode {
  const tab = useRef<NdNodeRef<"webview">>(null);
  const slotA = useRef<NdNodeRef<"box">>(null);
  const slotB = useRef<NdNodeRef<"box">>(null);
  const [host, setHost] = useState<"A" | "B">("A");

  function show(slot: NdNodeRef<"box"> | null, name: "A" | "B") {
    if (tab.current && slot) {
      moveNode(tab.current, slot);
      setHost(name);
    }
  }

  // effect:audited — one-time external sync on mount: the tab starts life in the
  // pool (shown in no window), so place it into a window once the native nodes
  // exist. Optionally ping-pong for the proof run.
  useEffect(() => {
    show(slotA.current, "A");
    if (!AUTO) return;
    const t1 = setTimeout(() => show(slotB.current, "B"), 1200);
    const t2 = setTimeout(() => show(slotA.current, "A"), 2400);
    return () => { clearTimeout(t1); clearTimeout(t2); };
  }, []);

  return (
    <>
      {/* The tab, pinned in the pool. Its React position never changes, so it is
          never unmounted when it moves between windows. */}
      {createPortal(
        <webview ref={tab} url="https://formalsnake.dev/" testID="tab" style={{ hexpand: true, vexpand: true }} />,
      )}

      <window title="Window A" defaultWidth={560} defaultHeight={380}>
        <box ref={slotA} orientation="vertical" spacing={8}>
          <button label="Bring tab here" onClick={() => show(slotA.current, "A")} />
          {host !== "A" && <label text="(tab is in Window B)" />}
        </box>
      </window>

      <window title="Window B" defaultWidth={560} defaultHeight={380}>
        <box ref={slotB} orientation="vertical" spacing={8}>
          <button label="Bring tab here" onClick={() => show(slotB.current, "B")} />
          {host !== "B" && <label text="(tab is in Window A)" />}
        </box>
      </window>
    </>
  );
}

await render(<App />);
