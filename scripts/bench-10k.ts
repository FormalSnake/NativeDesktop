// Node-mount benchmark driver (M10 gate). Builds a tree in ONE CommitBatch and
// reports encode+send time plus the encoded frame size. The host prints
// ND_COMMIT_APPLIED once tree.apply finishes; headless-m10.sh (the harness)
// bounds total wall time and compares the json vs binary legs by
// poll-count/marker, not by flaky wall-clock. Encoding is chosen by the host
// handshake (ND_FORCE_JSON=1 forces the json leg). Props are scalar/string only
// (no arrays/objects) so the binary leg actually exercises binary encoding
// instead of silently falling back to per-batch JSON (ndp.ts's
// BinaryUnsupportedValue fallback). ND_BENCH_NODES picks the tree size;
// ND_BENCH_HOLD_MS how long to stay alive so the host finishes applying.
import { Ndp } from "../runtime/ndp";
import { encodeCommitBatchBinary } from "../runtime/ndp-binary";

const N = Number(process.env.ND_BENCH_NODES ?? 10000);
const holdMs = Number(process.env.ND_BENCH_HOLD_MS ?? 2000);
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

const batch = { commitId: 0, generation: 0, ops };
const t0 = performance.now();
ndp.sendCommit(batch);
const ms = (performance.now() - t0).toFixed(1);
// Re-encode off the clock purely to report the wire size; the timed send above
// is the number the gate compares.
const bytes =
  encoding === "binary"
    ? encodeCommitBatchBinary(batch as never).length
    : Buffer.byteLength(JSON.stringify({ type: "commitBatch", ...batch }));
console.error(`ND_BENCH_MOUNT encoding=${encoding} nodes=${N} ms=${ms} bytes=${bytes}`);

// Keep the process alive briefly so the host applies the commit before exit.
await Bun.sleep(holdMs);
console.error("ND_BENCH_DONE");
process.exit(0);
