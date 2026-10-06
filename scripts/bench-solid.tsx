// Solid twin of scripts/bench-react.tsx: same tree, same phases, same marker
// format under ND_BENCH_SOLID. Each sample runs from the signal write to the
// CommitBatch handed to NDP (bench-react stops at the layout effect, which
// React runs after resetAfterCommit sends the batch). Solid JSX needs the
// transform registered first, so run it as
//   BUN_OPTIONS=--preload=./packages/solid/src/register.ts ND_SCRIPT=scripts/bench-solid.tsx <host>
import { render, nextCommit } from "@nativedesktop/solid";
import { createSignal, For } from "solid-js";

const N = Number(process.env.ND_BENCH_NODES ?? 2000);
const ROUNDS = Number(process.env.ND_BENCH_ROUNDS ?? 100);
const t0 = performance.now();

const [tick, setTick] = createSignal(0);
const [all, setAll] = createSignal(0);
const indices = Array.from({ length: N }, (_, i) => i);

function App() {
  return (
    <window title="bench-solid" defaultWidth={480} defaultHeight={640}>
      <scrollview>
        <box orientation="vertical">
          <For each={indices}>
            {(i) => <label text={`row ${i} ${all()}${i === 0 ? ` tick ${tick()}` : ""}`} />}
          </For>
        </box>
      </scrollview>
    </window>
  );
}

const sample = async (write: () => void): Promise<number> => {
  const start = performance.now();
  const done = nextCommit();
  write();
  await done;
  return performance.now() - start;
};

const report = (phase: string, samples: number[]): void => {
  const sorted = [...samples].sort((a, b) => a - b);
  const at = (q: number): string => sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))]!.toFixed(2);
  console.error(`ND_BENCH_SOLID phase=${phase} nodes=${N} rounds=${samples.length} median=${at(0.5)} p95=${at(0.95)}`);
};

// render() never resolves on first boot (it parks the process), so the bench
// runs alongside it instead of after it.
const mountStart = performance.now();
const mounted = nextCommit();
void render(() => <App />);
await mounted;
console.error(`ND_BENCH_SOLID phase=mount nodes=${N} ms=${(performance.now() - mountStart).toFixed(2)} sinceStart=${(performance.now() - t0).toFixed(2)}`);

const one: number[] = [];
for (let r = 0; r < ROUNDS; r++) one.push(await sample(() => setTick((t) => t + 1)));
report("one-row", one);

const every: number[] = [];
for (let r = 0; r < ROUNDS; r++) every.push(await sample(() => setAll((a) => a + 1)));
report("all-rows", every);
console.error(`ND_BENCH_SOLID_DONE rss_mb=${(process.memoryUsage().rss / 1048576).toFixed(1)}`);
await new Promise((r) => setTimeout(r, Number(process.env.ND_BENCH_HOLD_MS ?? 3000)));
process.exit(0);
