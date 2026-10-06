// Renderer-level benchmark: what React itself costs on top of the wire and the
// host, the number a renderer swap has to beat. Times mount, then ROUNDS
// updates of a single row (memoized siblings, the progress-bar shape) and
// ROUNDS updates that change every row. Each sample runs from setState to the
// layout effect, i.e. render + reconcile + commit ops handed to NDP. Prints one
// ND_BENCH_REACT line per phase with median and p95 in ms.
import { render, memo, useLayoutEffect, useState } from "@nativedesktop/react";

const N = Number(process.env.ND_BENCH_NODES ?? 2000);
const ROUNDS = Number(process.env.ND_BENCH_ROUNDS ?? 100);
const t0 = performance.now();

let committed: (() => void) | null = null;
let setState: (s: { tick: number; all: number }) => void = () => {};

const Row = memo(function Row({ i, tick, all }: { i: number; tick: number; all: number }) {
  return <label text={`row ${i} ${all}${i === 0 ? ` tick ${tick}` : ""}`} />;
});

function App(): React.ReactNode {
  const [s, set] = useState({ tick: 0, all: 0 });
  setState = set;
  useLayoutEffect(() => {
    committed?.();
  });
  const rows = [];
  for (let i = 0; i < N; i++) rows.push(<Row key={i} i={i} tick={i === 0 ? s.tick : 0} all={s.all} />);
  return (
    <window title="bench-react" defaultWidth={480} defaultHeight={640}>
      <scrollview>
        <box orientation="vertical">{rows}</box>
      </scrollview>
    </window>
  );
}

const next = (): Promise<number> => {
  const start = performance.now();
  return new Promise((resolve) => {
    committed = () => {
      committed = null;
      resolve(performance.now() - start);
    };
  });
};

const report = (phase: string, samples: number[]): void => {
  const sorted = [...samples].sort((a, b) => a - b);
  const at = (q: number): string => sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))]!.toFixed(2);
  console.error(`ND_BENCH_REACT phase=${phase} nodes=${N} rounds=${samples.length} median=${at(0.5)} p95=${at(0.95)}`);
};

// render() never resolves on first boot (it parks the process), so the bench
// runs alongside it instead of after it.
const mounted = next();
void render(<App />);
console.error(`ND_BENCH_REACT phase=mount nodes=${N} ms=${(await mounted).toFixed(2)} sinceStart=${(performance.now() - t0).toFixed(2)}`);

let state = { tick: 0, all: 0 };
const one: number[] = [];
for (let r = 0; r < ROUNDS; r++) {
  state = { ...state, tick: state.tick + 1 };
  const p = next();
  setState(state);
  one.push(await p);
}
report("one-row", one);

const all: number[] = [];
for (let r = 0; r < ROUNDS; r++) {
  state = { ...state, all: state.all + 1 };
  const p = next();
  setState(state);
  all.push(await p);
}
report("all-rows", all);
console.error(`ND_BENCH_REACT_DONE rss_mb=${(process.memoryUsage().rss / 1048576).toFixed(1)}`);
await new Promise((r) => setTimeout(r, Number(process.env.ND_BENCH_HOLD_MS ?? 3000)));
process.exit(0);
