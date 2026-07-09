import { render } from "@nativedesktop/react";
import { Suspense, use, useState, useTransition, useMemo, memo } from "react";

// Module-scoped (not useMemo'd): the uptime interval re-renders App every
// 500ms, and a concurrent render can discard an in-progress suspended fiber
// before it commits, resetting any hook-level cache (useMemo included) tied
// to that fiber. A promise created once at module load survives every
// discarded attempt, so `use()` keeps resolving against the same promise
// until it settles ~1s after the process starts.
const delayedBadgePromise = new Promise<string>((r) => setTimeout(() => r("ready:suspense-resolved"), 1000));

// memo: DelayedBadge takes no props, so its fiber need not be torn down by
// the unrelated uptime-interval re-renders of App every 500ms.
const DelayedBadge = memo(function DelayedBadge(): React.ReactNode {
  const text = use(delayedBadgePromise); // suspends until resolved -> fallback shown, then unhidden
  return <label text={text} />;
});

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
        <label testID="clicks-label" text={`Clicks: ${clicks}`} />
        <button testID="increment-button" label="Increment" onClick={onClick} />
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
