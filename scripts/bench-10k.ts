// 10k-node mount benchmark driver (M10 gate). Builds a 10k-node tree in ONE
// CommitBatch and reports encode+send time. The host prints ND_COMMIT_APPLIED
// once tree.apply finishes; headless-m10.sh (the harness) bounds total wall
// time and compares the json vs binary legs by poll-count/marker, not by
// flaky wall-clock. Encoding is chosen by the host handshake (ND_FORCE_JSON=1
// forces the json leg). Props are scalar/string only (no arrays/objects) so
// the binary leg actually exercises binary encoding instead of silently
// falling back to per-batch JSON (ndp.ts's BinaryUnsupportedValue fallback).
import { Ndp } from "../runtime/ndp";

const N = 10000;
const encoding = process.env.ND_FORCE_JSON === "1" ? "json" : "binary";

const ndp = await Ndp.connect();
await ndp.handshake({ name: "bun", version: Bun.version });

const ops: any[] = [];
ops.push({ op: "create", id: 1, widget: "Window", props: { title: "bench" } });
ops.push({ op: "create", id: 2, widget: "Box", props: { orientation: "vertical", spacing: 0 } });
ops.push({ op: "append", parent: 1, child: 2 });
for (let i = 3; i <= N; i++) {
  ops.push({ op: "create", id: i, widget: "Label", props: { text: `row ${i}` } });
  ops.push({ op: "append", parent: 2, child: i });
}

const t0 = performance.now();
ndp.sendCommit({ commitId: 0, generation: 0, ops });
const ms = (performance.now() - t0).toFixed(1);
console.error(`ND_BENCH_MOUNT encoding=${encoding} nodes=${N} ms=${ms}`);

// Keep the process alive briefly so the host applies the commit before exit.
await Bun.sleep(2000);
console.error("ND_BENCH_DONE");
process.exit(0);
