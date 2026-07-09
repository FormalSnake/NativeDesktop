import { render } from "@nativedesktop/react";
import { Suspense, use, useState, useTransition, useMemo } from "react";

// A promise that resolves after ~1s, memoized so it isn't recreated each render.
function useOneShot<T>(value: T, ms: number): Promise<T> {
  return useMemo(() => new Promise<T>((r) => setTimeout(() => r(value), ms)), [value, ms]);
}

function DelayedBadge(): React.ReactNode {
  const promise = useOneShot("ready:suspense-resolved", 1000);
  const text = use(promise); // suspends until resolved -> fallback shown, then unhidden
  return <label text={text} />;
}

function App(): React.ReactNode {
  const [clicks, setClicks] = useState(0);
  const [uptime, setUptime] = useState(0);
  const [slow, setSlow] = useState("idle");
  const [, startTransition] = useTransition();

  // Uptime interval keeps ND_COMMIT_APPLIED flowing under headless CI (no input synthesis).
  // useMemo runs the setup once; the interval drives state so commits continue.
  useMemo(() => {
    setInterval(() => setUptime((s) => s + 1), 500);
  }, []);

  const onClick = (): void => {
    setClicks((c) => c + 1);                       // discrete lane
    startTransition(() => setSlow(`transition:${Date.now()}`)); // transition lane
  };

  return (
    <window title="NativeDesktop M3 Counter" defaultWidth={480} defaultHeight={320}>
      <box orientation="vertical" spacing={8}>
        <label text={`Clicks: ${clicks}`} />
        <button label="Increment" onClick={onClick} />
        <label text={`Uptime: ${uptime}s`} />
        <label text={`Slow: ${slow}`} />
        <Suspense fallback={<label text="loading..." />}>
          <DelayedBadge />
        </Suspense>
      </box>
    </window>
  );
}

await render(<App />);
