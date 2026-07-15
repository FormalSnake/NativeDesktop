import { render, useState } from "@nativedesktop/react";

// NOT imported by main.tsx. A minimal acceptance fixture for the
// <menubar>/<menu>/<menuitem> machinery, driven headlessly by
// scripts/menubar-drive.ts on BOTH backends. Exercises: a File menu with a
// custom accelerated+iconed item whose onSelect mutates a visible label; a
// Probe menu with a disabled custom item, a separator, and an `about` role
// item; and a plain counter label. This is a throwaway probe, not the real
// notes app in main.tsx.
function App(): React.ReactNode {
  const [count, setCount] = useState(0);
  const [thing, setThing] = useState("no thing yet");
  return (
    <window title="ND Menubar Probe" defaultWidth={640} defaultHeight={420}>
      <menubar testID="probe-menubar">
        <menu label="File" testID="probe-file-menu">
          <menuitem
            testID="probe-new-thing"
            label="New Thing"
            accelerator="primary+n"
            iconName="document-new"
            onSelect={() => {
              setCount((c) => c + 1);
              setThing("made a thing");
            }}
          />
        </menu>
        <menu label="Probe" testID="probe-menu">
          <menuitem
            testID="probe-disabled"
            label="Disabled Item"
            enabled={false}
            onSelect={() => setCount((c) => c + 100)}
          />
          <menuitem role="separator" testID="probe-sep" />
          <menuitem role="about" label="About This Probe" testID="probe-about" />
        </menu>
      </menubar>
      <box orientation="vertical" style={{ padding: 16 }}>
        <label testID="probe-counter" text={`count=${count}`} />
        <label testID="probe-thing" text={thing} />
      </box>
    </window>
  );
}

await render(<App />);
