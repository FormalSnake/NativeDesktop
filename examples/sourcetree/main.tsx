// Target app for scripts/sourcetree-drive.ts: a single window (no tabs,
// unlike the gallery's SourceTree tab, so every surface is visible for
// screenshots) holding a `toolbar` strip, a
// <sourcetree> with sections + a 3-level chain + captions/badges + two
// actions, and readout labels the drive asserts through. Row order puts the
// actionable project row FIRST so the AppKit pointer leg can hit its
// trailing action button at a predictable y. actionVisibility "always" for
// the same reason (hover can't be a precondition for a coordinate click).
import { render, useState, useMountEffect, app, hasCommand, hasWidget } from "@nativedesktop/react";
import type { SourceTreeAction, SourceTreeNode } from "@nativedesktop/react";

const actions: SourceTreeAction[] = [
  { id: "new-run", iconName: "list-add-symbolic", label: "New Run" },
  { id: "close-run", iconName: "window-close-symbolic", tooltip: "Close run", destructive: true },
];

// A 16x16 solid magenta PNG: `iconData` takes raw image bytes rather than a
// freedesktop icon name, which is the only shape a favicon comes in. Loud on
// purpose — a capture makes it obvious whether the bytes reached the row, and
// the third toolbar button below takes the same prop.
const ICON_DATA =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAGklEQVR42mP4z/D/PyWYYdSAUQNGDRguBgAAx4/9H5ua3FcAAAAASUVORK5CYII=";

const nodeMeta: Omit<SourceTreeNode, "expanded">[] = [
  { id: "proj-nd", title: "NativeDesktop", hasChildren: true, actionIds: ["new-run"], testID: "st-proj-nd" },
  { id: "run-1", parentId: "proj-nd", title: "fix sidebar", caption: "running · 2m", badge: "3",
    actionIds: ["close-run"], testID: "st-run-1" },
  { id: "run-2", parentId: "proj-nd", title: "docs pass", caption: "idle", iconData: ICON_DATA, testID: "st-run-2" },
  { id: "sec-hosts", title: "Hosts", section: true, hasChildren: true, testID: "st-sec-hosts" },
  { id: "host-mac", parentId: "sec-hosts", title: "macbook", caption: "connected", iconName: "computer-symbolic",
    captionIconName: "network-transmit-receive-symbolic", hasChildren: true, testID: "st-host-mac" },
  { id: "proj-two", parentId: "host-mac", title: "Docs", hasChildren: true, testID: "st-proj-two" },
  { id: "run-3", parentId: "proj-two", title: "deep run", caption: "level three", testID: "st-run-3" },
  { id: "sec-settled", title: "Settled", section: true, hasChildren: true, testID: "st-sec-settled" },
  { id: "run-old", parentId: "sec-settled", title: "old run", caption: "settled yesterday", testID: "st-run-old" },
];

// ---- row-geometry probe (ND_ST_GEOMETRY=flat|deep|hover|always) ------------
// One tree, alone in a window whose layout does not change between variants,
// so two captures of two variants are directly comparable pixel for pixel.
// The variants differ in exactly ONE property each: flat vs deep answers "does
// a list with nothing expandable still reserve the disclosure gutter?", hover
// vs always answers "does an appearing action button change the allocation?".
// Probe rows carry no icon and no caption, so the leftmost ink in a row's band
// IS its title; the row count fills the viewport so the widget's centre (all
// the hover RPC can aim at) lands on a row.
const PROBE_ROWS = 20;
const probeRow = (i: number): SourceTreeNode => ({ id: `g-${i}`, title: `Alpha ${i}`, expanded: false });
const probeNodes = (variant: string): SourceTreeNode[] => {
  const rows = Array.from({ length: PROBE_ROWS }, (_, i) => probeRow(i));
  if (variant === "deep") {
    // The one expandable node sits LAST: the rule under test is about the
    // whole tree, so row 0 must keep the gutter for a branch it cannot see.
    rows.push({ id: "g-parent", title: "Parent", hasChildren: true, expanded: false });
  }
  if (variant === "hover" || variant === "always") {
    // A badge plus a trailing action: the badge is the row content a
    // collapsing action slot drags rightwards, and it stays left of the slot
    // in both states, which is what makes the shift measurable off a capture.
    return rows.map((n) => ({ ...n, badge: "12", actionIds: ["close-run"] }));
  }
  return rows;
};

function GeometryProbe({ variant }: { variant: string }): React.ReactNode {
  return (
    <window title="SourceTree Geometry" defaultWidth={480} defaultHeight={520}>
      <box orientation="vertical" spacing={0}>
        <sourcetree
          testID="st-geo"
          nodes={probeNodes(variant)}
          actions={actions}
          actionVisibility={variant === "hover" ? "hover" : "always"}
          style={{ vexpand: true }}
        />
      </box>
    </window>
  );
}

function App(): React.ReactNode {
  const [expanded, setExpanded] = useState<Set<string>>(
    new Set(["proj-nd", "sec-hosts", "host-mac", "proj-two"]),
  );
  const [selectedId, setSelectedId] = useState("");
  const [lastActivated, setLastActivated] = useState("");
  const [lastAction, setLastAction] = useState("");
  const [lastExpandEvent, setLastExpandEvent] = useState("");
  const [lastToolbar, setLastToolbar] = useState("");
  const nodes: SourceTreeNode[] = nodeMeta.map((n) => ({ ...n, expanded: expanded.has(n.id) }));

  // Activation transitions re-render the readouts below; the drive frontmosts
  // the process and waits for the active label to flip.
  const [, setActivationTick] = useState(0);
  useMountEffect(() => {
    const offActivate = app.onActivate(() => setActivationTick((t) => t + 1));
    const offDeactivate = app.onDeactivate(() => setActivationTick((t) => t + 1));
    return () => {
      offActivate();
      offDeactivate();
    };
  });

  // Render-time (no await): hasCommand/hasWidget answer from the handshake
  // manifest; app.isActive() from the host's replayed activation state.
  const capsText = `caps present=${hasCommand("window", "present")} nope=${hasCommand("window", "nope")} sourcetree=${hasWidget("sourcetree")}`;
  const activeText = `active ${app.isActive()} replay=${globalThis.__nd_app_active !== undefined ? "yes" : "no"}`;

  return (
    <window title="SourceTree Drive" defaultWidth={480} defaultHeight={760}>
      <box orientation="vertical" spacing={8}>
        <box orientation="horizontal" spacing={6} cssClasses={["toolbar"]} testID="st-toolbar">
          <button testID="st-toolbar-refresh" iconName="view-refresh-symbolic" cssClasses={["flat"]}
            onClick={() => setLastToolbar("refresh")} />
          <button testID="st-toolbar-add" iconName="list-add-symbolic" cssClasses={["flat"]} />
          <button testID="st-toolbar-site" iconData={ICON_DATA} tooltip="Current site" cssClasses={["flat"]} />
          <button testID="st-toolbar-labelled" label="Site" iconData={ICON_DATA} cssClasses={["flat"]} />
        </box>
        <sourcetree
          testID="st-tree"
          nodes={nodes}
          actions={actions}
          selectedId={selectedId}
          actionVisibility="always"
          onSelectionChanged={(e) => setSelectedId((e.data as { nodeId: string | null }).nodeId ?? "")}
          onRowActivated={(e) => setLastActivated((e.data as { nodeId: string }).nodeId)}
          onNodeExpanded={(e) => {
            const { nodeId } = e.data as { nodeId: string };
            setLastExpandEvent(`expanded:${nodeId}`);
            setExpanded((prev) => new Set(prev).add(nodeId));
          }}
          onNodeCollapsed={(e) => {
            const { nodeId } = e.data as { nodeId: string };
            setLastExpandEvent(`collapsed:${nodeId}`);
            setExpanded((prev) => {
              const next = new Set(prev);
              next.delete(nodeId);
              return next;
            });
          }}
          onActionClicked={(e) => {
            const { nodeId, actionId } = e.data as { nodeId: string; actionId: string };
            setLastAction(`${actionId}@${nodeId}`);
          }}
          style={{ vexpand: true }}
        />
        <checkbox testID="st-settled-toggle" label="Show settled" checked={expanded.has("sec-settled")}
          onToggled={(e) => setExpanded((prev) => {
            const next = new Set(prev);
            if (e.checked) next.add("sec-settled");
            else next.delete("sec-settled");
            return next;
          })} />
        <label testID="st-selected-readout" text={`sel ${selectedId || "(none)"}`} />
        <label testID="st-activated-readout" text={`act ${lastActivated || "(none)"}`} />
        <label testID="st-action-readout" text={`action ${lastAction || "(none)"}`} />
        <label testID="st-expand-readout" text={`expand ${lastExpandEvent || "(none)"}`} />
        <label testID="st-toolbar-readout" text={`toolbar ${lastToolbar || "(none)"}`} />
        <label testID="st-caps-readout" text={capsText} />
        <label testID="st-active-readout" text={activeText} />
      </box>
    </window>
  );
}

const geometryVariant = process.env.ND_ST_GEOMETRY;
await render(geometryVariant ? <GeometryProbe variant={geometryVariant} /> : <App />);
