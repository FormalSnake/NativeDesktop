import { render } from "@nativedesktop/solid";
import type { TableColumn, TableRow } from "@nativedesktop/solid";
import { createSignal } from "solid-js";

// M16 gestures probe: every widget here exists to give the input-synthesis
// automation RPCs (pointer/drag/doubleClick/rightClick/hover/keys) and the
// a11y tree fields a deterministic, tab-free assertion target — the peer of
// examples/counter for scripts/gestures-drive.ts. No tabs: everything stays
// visible so actionability never depends on tab state.

const columns: TableColumn[] = [
  { id: "name", title: "Name", width: 160 },
  { id: "role", title: "Role", width: 160 },
];

const rows: TableRow[] = [
  { id: "ada", cells: ["Ada Lovelace", "Analyst"] },
  { id: "grace", cells: ["Grace Hopper", "Rear Admiral"] },
  { id: "margaret", cells: ["Margaret Hamilton", "Lead Engineer"] },
];

function App() {
  const [volume, setVolume] = createSignal(20);
  const [agreed, setAgreed] = createSignal(false);
  const [name, setName] = createSignal("");
  const [activatedRow, setActivatedRow] = createSignal(-1);
  const [selectedRow, setSelectedRow] = createSignal(-1);

  return (
    <window title="ND Gestures" defaultWidth={640} defaultHeight={560}>
      <box orientation="vertical" spacing={8} style={{ padding: 16 }}>
        <slider
          testID="volume-slider"
          min={0}
          max={100}
          step={1}
          value={volume()}
          onValueChanged={(e) => setVolume(Math.round(e.value))}
        />
        <label testID="volume-label" text={`Volume: ${volume()}`} />

        <checkbox
          testID="agree-check"
          label="I agree"
          checked={agreed()}
          onToggled={(e) => setAgreed(e.checked)}
        />
        <label testID="agree-label" text={`Agreed: ${agreed() ? "yes" : "no"}`} />

        <textinput
          testID="name-input"
          text={name()}
          placeholder="Type here"
          onChanged={(e) => setName(e.text)}
        />
        <label testID="echo-label" text={`Echo: ${name()}`} />

        <table
          testID="people-table"
          columns={columns}
          rows={rows}
          selectedIndex={selectedRow()}
          onSelectionChanged={(e) => setSelectedRow(e.index)}
          onRowActivated={(e) => setActivatedRow(e.index)}
          style={{ vexpand: true }}
        />
        <label testID="activated-label" text={`Activated: ${activatedRow()}`} />
        <label testID="hover-target" text="Hover / right-click target" />
      </box>
    </window>
  );
}

await render(() => <App />);
