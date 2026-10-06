import { createSignal } from "solid-js";
import { Panel } from "./Panel.desktop";

export function App() {
  const [clicks, setClicks] = createSignal(0);

  return (
    <window title="NativeDesktop App" defaultWidth={480} defaultHeight={320}>
      <box orientation="vertical" spacing={8}>
        <label testID="clicks-label" text={`Clicks: ${clicks()}`} />
        <button testID="increment-button" label="Increment" onClick={() => setClicks((c) => c + 1)} />
        <Panel />
      </box>
    </window>
  );
}
