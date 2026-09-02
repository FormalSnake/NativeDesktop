import { render, useState } from "@nativedesktop/react";

// A prop that disappears from JSX has to leave the native widget too. Every
// widget here is rendered with a prop spread that goes empty when `dropped`
// flips, so one click removes `label`, `tooltip`, `enabled` and `checked`
// from the commit and the host has to reset each one to its schema default.
// Driven by scripts/propreset-drive.ts.

function App(): React.ReactNode {
  const [dropped, setDropped] = useState(false);

  const buttonProps = dropped ? {} : { label: "Subject", tooltip: "Subject hint", enabled: false };
  const checkProps = dropped ? {} : { checked: true, label: "Agree" };
  const styleProps = dropped ? {} : { style: { padding: 24, background: "#3584e4" } };

  return (
    <window title="ND Prop Reset" defaultWidth={420} defaultHeight={260}>
      <box orientation="vertical" spacing={8} style={{ padding: 16 }}>
        <button testID="drop-toggle" label="Drop props" onClick={() => setDropped(true)} />
        <label testID="state-label" text={dropped ? "dropped" : "set"} />
        <button testID="subject-button" {...buttonProps} />
        <checkbox testID="subject-check" {...checkProps} />
        <box testID="subject-box" orientation="vertical" {...styleProps}>
          <label testID="styled-label" text="Styled" />
        </box>
      </box>
    </window>
  );
}

render(<App />);
