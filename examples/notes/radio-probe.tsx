import { render, useState } from "@nativedesktop/react";

// Regression probe for the M13 radio-group use-after-free: a conditionally
// rendered radio group that unmounts and remounts under the SAME group name.
// Before the fix (radio_groups eviction on anchor destroy), the second mount
// joined a freed GtkCheckButton and segfaulted in _gtk_check_button_set_group.
// Driven by the scratch radio drive (click toggle x4, assert host alive and
// radios re-present); not imported by main.tsx.

function App(): React.ReactNode {
  const [show, setShow] = useState(true);
  const [cycles, setCycles] = useState(0);

  return (
    <window title="Radio Probe" defaultWidth={420} defaultHeight={300}>
      <box orientation="vertical" spacing={8} style={{ padding: 16 }}>
        <button
          testID="toggle"
          label="Toggle group"
          onClick={() => {
            setShow((s) => !s);
            setCycles((c) => c + 1);
          }}
        />
        <label testID="cycles" text={`cycles: ${cycles}`} />
        {show ? (
          <box orientation="vertical" spacing={4} testID="radio-box">
            <radio group="theme" label="System" checked />
            <radio group="theme" label="Dark" />
            <radio group="theme" label="Light" />
          </box>
        ) : (
          <label testID="hidden-marker" text="group unmounted" />
        )}
      </box>
    </window>
  );
}

await render(<App />);
