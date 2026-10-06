import { render } from "@nativedesktop/solid";
import { Loading, createMemo, createSignal, onCleanup } from "solid-js";

// Its own component so a hot edit to it leaves App, and the clicks signal App
// owns, mounted (scripts/counter-solid-hmr-drive.ts).
function ClicksLabel(props: { clicks: number }) {
  return <label testID="clicks-label" text={`Clicks: ${props.clicks}`} />;
}

function App() {
  const [clicks, setClicks] = createSignal(0);
  const [uptime, setUptime] = createSignal(0);
  // Mirrors examples/counter's suspended badge. Created in here rather than at
  // module scope: `bun --hot` re-runs module scope on every edit, and a fresh
  // promise there would count as a changed dependency and remount App.
  const badge = createMemo(() => new Promise<string>((r) => setTimeout(() => r("ready:loading-resolved"), 1000)));

  // Keeps ND_COMMIT_APPLIED flowing under headless CI (no input synthesis).
  const timer = setInterval(() => setUptime((s) => s + 1), 500);
  onCleanup(() => clearInterval(timer));

  return (
    <window title="NativeDesktop Solid Counter" defaultWidth={480} defaultHeight={320}>
      <box orientation="vertical" spacing={8}>
        <ClicksLabel clicks={clicks()} />
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
