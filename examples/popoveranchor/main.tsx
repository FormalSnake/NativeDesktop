import { createPortal, render, useRef, useState } from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";

// A popover anchored by ref rather than by where it sits in the tree. The
// <popover> is rendered through a portal into the off-window pool, so it has
// no tree parent to anchor on at all: `anchorRef` is the only thing that can
// place it. Drop the ref and it has nowhere to open, which is what makes the
// difference observable. Driven by scripts/popover-anchor-drive.ts.

function App(): React.ReactNode {
  const trigger = useRef<NdNodeRef<"button">>(null);
  const [open, setOpen] = useState(false);
  const [anchored, setAnchored] = useState(true);

  return (
    <window title="ND Popover Anchor" defaultWidth={420} defaultHeight={280}>
      <box orientation="vertical" spacing={8} style={{ padding: 16 }}>
        <button ref={trigger} testID="anchor-button" label="Open" onClick={() => setOpen(true)} />
        <button testID="detach-button" label="Detach" onClick={() => setAnchored(false)} />
        <label testID="mode-label" text={anchored ? "anchored" : "detached"} />
        <label testID="open-label" text={open ? "open" : "shut"} />
        {createPortal(
          <popover
            testID="anchored-popover"
            anchorRef={anchored ? trigger : undefined}
            open={open}
            position="bottom"
            onClosed={() => setOpen(false)}
          >
            <box testID="popover-body" orientation="vertical" spacing={8}>
              <label testID="popover-label" text="Anchored content" />
            </box>
          </popover>,
        )}
      </box>
    </window>
  );
}

render(<App />);
