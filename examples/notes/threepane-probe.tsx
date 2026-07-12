import { render } from "@nativedesktop/react";

// examples/notes/threepane-probe.tsx — NOT imported by main.tsx. A minimal
// three-pane (folders / list / content) SplitView probe for M13 Feature B:
// exercises the new `list` slot end-to-end (schema + both backends) with a
// distinct testID per pane, driven headlessly by scripts/threepane-drive.ts.
// This does not replace the Feature C notes rework — it is a throwaway
// acceptance fixture proving the SplitView machinery before that rework
// consumes it.
function App(): React.ReactNode {
  return (
    <window title="ND Three-Pane Probe" defaultWidth={900} defaultHeight={600}>
      <splitview testID="probe-split" sidebarWidth={0.25} listWidth={0.3}>
        <toolbarview slot="sidebar" testID="probe-sidebar-toolbar">
          <headerbar testID="probe-sidebar-header" title="Folders" />
          <box testID="probe-sidebar-content" orientation="vertical" style={{ padding: 8 }}>
            <label testID="probe-sidebar-row" text="All Notes" />
          </box>
        </toolbarview>

        <toolbarview slot="list" testID="probe-list-toolbar">
          <headerbar testID="probe-list-header" title="Notes" />
          <box testID="probe-list-content" orientation="vertical" style={{ padding: 8 }}>
            <label testID="probe-list-row" text="Welcome note" />
          </box>
        </toolbarview>

        <toolbarview slot="content" testID="probe-content-toolbar">
          <headerbar testID="probe-content-header" title="Editor" />
          <box testID="probe-content-content" orientation="vertical" style={{ padding: 8 }}>
            <label testID="probe-content-row" text="Note body" />
          </box>
        </toolbarview>
      </splitview>
    </window>
  );
}

await render(<App />);
