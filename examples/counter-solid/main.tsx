import { render } from "@nativedesktop/solid";
import { Loading, createMemo, createSignal } from "solid-js";

// Mirrors examples/counter. Created once at module load so the badge resolves
// about 1s after the process starts, whatever re-renders around it.
const delayedBadge = new Promise<string>((r) => setTimeout(() => r("ready:loading-resolved"), 1000));

function App() {
  const [clicks, setClicks] = createSignal(0);
  const [uptime, setUptime] = createSignal(0);
  const badge = createMemo(() => delayedBadge);

  // Keeps ND_COMMIT_APPLIED flowing under headless CI (no input synthesis).
  setInterval(() => setUptime((s) => s + 1), 500);

  return (
    <window title="NativeDesktop Solid Counter" defaultWidth={480} defaultHeight={320}>
      <box orientation="vertical" spacing={8}>
        <label testID="clicks-label" text={`Clicks: ${clicks()}`} />
        <button testID="increment-button" label="Increment" onClick={() => setClicks((c) => c + 1)} />
        <label text={`Uptime: ${uptime()}s`} />
        <Loading fallback={<label text="loading..." />}>
          <label testID="badge-label" text={badge()} />
        </Loading>
      </box>
    </window>
  );
}

await render(() => <App />);
