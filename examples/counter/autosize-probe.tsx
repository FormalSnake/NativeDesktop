import { render } from "@nativedesktop/react";

// A <window> with no defaultWidth/defaultHeight: the acceptance fixture for
// sizing a window from its content. Both boxes carry an explicit minWidth /
// minHeight, so the natural size the window has to open at is a number this
// file states rather than one measured off the platform's font metrics.
//
// Root natural: 520 wide (the widest child), 240 + 120 + 8 spacing tall.
// Root minimum: the same numbers, since a minWidth/minHeight is a floor.
function App(): React.ReactNode {
  return (
    <window title="ND Autosize Probe">
      <box orientation="vertical" spacing={8} testID="autosize-root">
        <box testID="autosize-wide" orientation="vertical" style={{ minWidth: 520, minHeight: 240 }}>
          <label testID="autosize-label" text="Sized from content" />
        </box>
        <box testID="autosize-short" orientation="vertical" style={{ minWidth: 200, minHeight: 120 }}>
          <label text="Second block" />
        </box>
      </box>
    </window>
  );
}

await render(<App />);
