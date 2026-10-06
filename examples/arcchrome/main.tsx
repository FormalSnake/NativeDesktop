import { render, sendCommand } from "@nativedesktop/solid";
import type { NdNodeRef } from "@nativedesktop/solid";
import { createSignal } from "solid-js";

// The Arc window shape: no toolbar, a full-height sidebar whose first row
// holds the window's own controls, the page in a rounded card beside it, and
// a hidden sidebar that slides back in over the page from the leading edge.
// Driven by scripts/arcchrome-drive.ts on both backends.

const PAGE =
  "data:text/html," +
  encodeURIComponent(
    "<!doctype html><title>Card page</title><style>html,body{margin:0;height:100%;background:#2f6bff}" +
      "h1{margin:0;padding:24px;color:#fff;font:600 28px system-ui}</style><h1>Page edge to edge</h1>",
  );

function App() {
  const [hidden, setHidden] = createSignal(false);
  const [revealed, setRevealed] = createSignal(false);
  const [clicks, setClicks] = createSignal(0);
  let split: NdNodeRef<"splitview"> | undefined;

  return (
    <window title="ND Arc Chrome" defaultWidth={1100} defaultHeight={700}>
      <splitview
        ref={split}
        testID="split"
        sidebarWidth={0.24}
        collapsed={hidden()}
        edgeReveal
        contentStyle="card"
        onRevealChanged={(e) => setRevealed(e.checked)}
      >
        <box slot="sidebar" testID="sidebar" orientation="vertical" spacing={8} style={{ vexpand: true, padding: 8 }}>
          <box testID="controls-row" orientation="horizontal" spacing={4} windowHandle style={{ minHeight: 32 }}>
            <windowcontrols testID="controls-start" side="start" style={{ valign: "center" }} />
            <box testID="controls-gap" orientation="horizontal" style={{ hexpand: true }} />
            <button
              testID="toggle"
              iconName="sidebar-show-symbolic"
              tooltip={hidden() ? "Show Sidebar" : "Hide Sidebar"}
              cssClasses={["flat"]}
              style={{ valign: "center" }}
              onClick={() => setHidden((h) => !h)}
            />
            <button testID="back" iconName="go-previous-symbolic" tooltip="Back" cssClasses={["flat"]} style={{ valign: "center" }} />
            <button
              testID="reload"
              iconName="view-refresh-symbolic"
              tooltip="Reload"
              cssClasses={["flat"]}
              style={{ valign: "center" }}
              onClick={() => setClicks((n) => n + 1)}
            />
            <windowcontrols testID="controls-end" side="end" style={{ valign: "center" }} />
          </box>
          <label testID="state" text={`hidden=${hidden()} revealed=${revealed()} clicks=${clicks()}`} ellipsize />
        </box>
        <box slot="content" testID="card" orientation="vertical" style={{ hexpand: true, vexpand: true }}>
          {/* The command path to the reveal, for a backend with no pointer
              synthesis (GTK4); the pointer path is the leading edge. */}
          <box testID="card-row" orientation="horizontal" spacing={4} style={{ padding: 4 }}>
            <button
              testID="reveal"
              label="Reveal"
              cssClasses={["flat"]}
              onClick={() => split && sendCommand(split, "revealSidebar")}
            />
            <button
              testID="conceal"
              label="Conceal"
              cssClasses={["flat"]}
              onClick={() => split && sendCommand(split, "concealSidebar")}
            />
            <button testID="show" label="Show Sidebar" cssClasses={["flat"]} onClick={() => setHidden(false)} />
            <label testID="card-state" text={`hidden=${hidden()} revealed=${revealed()}`} />
          </box>
          <webview testID="page" url={PAGE} style={{ hexpand: true, vexpand: true }} />
        </box>
      </splitview>
    </window>
  );
}

await render(() => <App />);
