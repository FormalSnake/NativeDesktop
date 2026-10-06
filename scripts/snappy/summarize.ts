// Buckets a trace.sh host log by phase and prints, per phase, the UI thread's
// jobs and frames, the JS side's commits and event handlers, and the outside
// latency per measured step.
//   bun summarize.ts <host.log> <lat.txt>
const [logPath, latPath] = process.argv.slice(2) as [string, string];
const log = await Bun.file(logPath).text();
const lat = await Bun.file(latPath).text();

const phases: { name: string; begin: number; end: number }[] = [];
for (const m of log.matchAll(/^ND_PHASE (\S+) (begin|end) at=(\d+)/gm)) {
  if (m[2] === "begin") phases.push({ name: m[1]!, begin: Number(m[3]), end: Infinity });
  else phases.findLast((p) => p.name === m[1])!.end = Number(m[3]);
}
const phaseAt = (at: number) => phases.find((p) => at >= p.begin && at <= p.end)?.name;

type Series = Map<string, number[]>;
const series = new Map<string, Series>();
const add = (phase: string, key: string, v: number) => {
  const s = series.get(phase) ?? new Map();
  series.set(phase, s);
  (s.get(key) ?? s.set(key, []).get(key)!).push(v);
};
const kv = (line: string) => Object.fromEntries([...line.matchAll(/(\w+)=(\S+)/g)].map((m) => [m[1], m[2]]));

for (const line of log.split("\n")) {
  if (!line.startsWith("ND_PERF ")) continue;
  const f = kv(line);
  const phase = phaseAt(Number(f.at));
  if (!phase) continue;
  const kind = line.split(" ")[1];
  if (kind === "job") add(phase, "job_us", Number(f.us));
  else if (kind === "frame") {
    add(phase, "frame_us", Number(f.total_us));
    add(phase, "layout_us", Number(f.layout_us));
    add(phase, "paint_us", Number(f.paint_us));
  } else if (kind === "commit") {
    add(phase, "commit_ops", Number(f.ops));
    if (Number(f.since_us) >= 0) add(phase, `event_to_commit_us(${f.after})`, Number(f.since_us));
  } else if (kind === "event") add(phase, `handler_us(${line.split(" ")[2]})`, Number(f.handler_us));
  else if (kind === "emit") add(phase, `emit(${line.split(" ")[2]})`, 1);
}

// Frames closer together than this belong to one run: an animation, or a
// frame clock that never went idle.
const runGapUs = 40_000;
const frameAts = [...log.matchAll(/^ND_PERF frame .* at=(\d+)/gm)].map((m) => Number(m[1]));
const longestRun = new Map<string, number>();
for (let i = 0, start = 0; i < frameAts.length; i++) {
  if (i > 0 && frameAts[i]! - frameAts[i - 1]! > runGapUs) start = i;
  const phase = phaseAt(frameAts[i]!);
  if (phase) longestRun.set(phase, Math.max(longestRun.get(phase) ?? 0, frameAts[i]! - frameAts[start]!));
}

const pct = (v: number[], p: number) => [...v].sort((a, b) => a - b)[Math.min(v.length - 1, Math.floor(v.length * p))]!;
const sum = (v: number[]) => v.reduce((a, b) => a + b, 0);
for (const p of phases) {
  console.log(`== ${p.name} (${((p.end - p.begin) / 1e6).toFixed(1)} s, longest frame run ${((longestRun.get(p.name) ?? 0) / 1e6).toFixed(2)} s)`);
  const s = series.get(p.name);
  if (!s) continue;
  for (const [k, v] of [...s].sort()) {
    if (k.startsWith("emit(")) console.log(`  ${k} n=${v.length}`);
    else console.log(`  ${k} n=${v.length} sum=${sum(v)} p50=${pct(v, 0.5)} p95=${pct(v, 0.95)} max=${Math.max(...v)}`);
  }
}

const steps = new Map<string, number[]>();
let timeouts = 0;
for (const m of lat.matchAll(/^ND_LAT (\S+) (\S+)/gm)) {
  const key = m[1]!.replace(/_[a-z]$/, "");
  if (m[2] === "timeout") timeouts++;
  else (steps.get(key) ?? steps.set(key, []).get(key)!).push(Number(m[2]));
}
console.log("== latency ms (input to pixels)");
for (const [k, v] of steps) console.log(`  ${k} n=${v.length} mean=${(sum(v) / v.length).toFixed(1)} p95=${pct(v, 0.95).toFixed(1)} max=${Math.max(...v).toFixed(1)}`);
console.log(`  timeouts=${timeouts}`);
