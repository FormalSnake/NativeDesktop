import { render, useState } from "@nativedesktop/react";
import type { SourceTreeAction, SourceTreeNode } from "@nativedesktop/react";

// NOT imported by main.tsx. The window chrome a browser-style app builds: a
// <splitview> whose sidebar pane is a <toolbarview> wrapping a button over a
// <sourcetree>, a content pane that owns the sidebar toggle (so it stays
// clickable while the sidebar is gone), and a <popover> whose panel holds a
// row with a hexpanding column in it. Driven by scripts/chrome-drive.ts,
// which measures the sidebar fraction, the pane the toggle leaves behind,
// what a collapsed pane's descendants report, and whether the popover's frame
// holds its own content. ND_CHROME_COLLAPSED=1 starts the split collapsed.
const ACTIONS: SourceTreeAction[] = [
  { id: "close", iconName: "window-close-symbolic", tooltip: "Close" },
];

// Longer than the sidebar can hold, so the row's own truncation is what the
// drive measures rather than the string's length.
const LONG_TITLE = "Alpha Bravo Charlie Delta Echo Foxtrot Golf";
const NODES: SourceTreeNode[] = Array.from({ length: 8 }, (_, i) => ({
  id: `t-${i}`,
  title: `${LONG_TITLE} ${i}`,
  iconName: "web-browser-symbolic",
  actionIds: ["close"],
  expanded: false,
}));

const startCollapsed = process.env.ND_CHROME_COLLAPSED === "1";

// A window narrow enough that the declared fraction falls under the sidebar's
// 180pt floor, so the pane can only come up at the floor. The drive resizes
// the window afterwards and asks what the fraction did with the new width,
// which is the shape an app that carries its window size in a store hits: the
// split is laid out at one width and the window settles at another.
const startNarrow = process.env.ND_CHROME_NARROW === "1";

function App(): React.ReactNode {
  const [showSidebar, setShowSidebar] = useState(true);
  const [panelOpen, setPanelOpen] = useState(false);

  return (
    <window title="ND Chrome Probe" defaultWidth={startNarrow ? 560 : 900} defaultHeight={600}>
      <splitview testID="sp-split" sidebarWidth={0.3} collapsed={startCollapsed}>
        {showSidebar && (
          <toolbarview slot="sidebar" testID="sp-sidebar-toolbar">
            <headerbar testID="sp-sidebar-header" title="Sidebar" />
            <box
              testID="sp-sidebar"
              orientation="vertical"
              spacing={4}
              style={{ vexpand: true, padding: 6 }}
            >
              <button testID="sp-new" label="New Tab" labelAlign="start" cssClasses={["flat"]} style={{ hexpand: true }} />
              <sourcetree
                testID="sp-tree"
                nodes={NODES}
                actions={ACTIONS}
                actionVisibility="always"
                indentationPerLevel={0}
                style={{ vexpand: true }}
              />
            </box>
          </toolbarview>
        )}

        <toolbarview slot="content" testID="sp-content-toolbar">
          <headerbar testID="sp-content-header" title="Content" />
          <box testID="sp-content" orientation="vertical" spacing={8} style={{ padding: 8 }}>
            <button testID="sp-toggle" label="Toggle Sidebar" onClick={() => setShowSidebar((v) => !v)} />
            <label testID="sp-state" text={`sidebar ${showSidebar ? "on" : "off"}`} />
            {/* A title-bearing widget with no `text`/`label` prop: what the
                snapshot reports for it is the schema's declared text source. */}
            <settingsgroup testID="sp-group" title="Group Title">
              <row testID="sp-row" title="Row Title" subtitle="Row subtitle" />
            </settingsgroup>

            {/* A downloads-panel popover: an icon, a hexpanding column whose
                label ellipsizes, a trailing button, and a wide button under
                the lot. The hexpanding column is what makes the panel's
                minimum width smaller than the width it actually lays out at,
                which is the frame the popover has to hold. */}
            <box orientation="horizontal" spacing={8}>
              <button testID="sp-panel-trigger" label="Downloads" onClick={() => setPanelOpen(true)} />
              <popover testID="sp-panel" open={panelOpen} position="bottom" onClosed={() => setPanelOpen(false)}>
                <box testID="sp-panel-body" orientation="vertical" spacing={8} style={{ padding: 8 }}>
                  <label testID="sp-panel-heading" text="Downloads" cssClasses={["heading"]} style={{ halign: "start" }} />
                  <box testID="sp-panel-row" orientation="horizontal" spacing={8}>
                    <image iconName="folder-download-symbolic" symbolScale="small" cssClasses={["dimmed"]} />
                    <box testID="sp-panel-column" orientation="vertical" style={{ hexpand: true }}>
                      <label testID="sp-panel-name" text="quarterly-report-final-v3.pdf" ellipsize style={{ halign: "start" }} />
                      <label text="12.4 MB of 48 MB" cssClasses={["dimmed", "caption"]} style={{ halign: "start" }} />
                    </box>
                    <button testID="sp-panel-reveal" iconName="folder-symbolic" cssClasses={["flat"]} style={{ halign: "end", valign: "center" }} />
                  </box>
                  <button testID="sp-panel-folder" label="Open Downloads Folder" cssClasses={["flat"]} />
                </box>
              </popover>
            </box>
          </box>
        </toolbarview>
      </splitview>
    </window>
  );
}

await render(<App />);
