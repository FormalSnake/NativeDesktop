// NDP binary CommitBatch encoder (docs/superpowers/specs/2026-07-09-ndp-binary-encoding.md).
// Pure function: builds the 28-byte header + op stream + string table
// `payload` described by the spec. Callers are responsible for the outer
// u32 LE frame-length prefix (shared with the JSON path, spec §1.1) — this
// module never touches the socket or the outbox.

import { WIDGET_TYPE } from "../packages/react/src/generated/widget-types";

type Op =
  | { op: "create"; id: number; widget: string; props: Record<string, unknown> }
  | { op: "append"; parent: number; child: number }
  | { op: "insertBefore"; parent: number; child: number; before: number | null }
  | { op: "remove"; id: number }
  | { op: "setText"; id: number; text: string }
  | { op: "update"; id: number; props: Record<string, unknown> }
  | { op: "hide"; id: number }
  | { op: "unhide"; id: number };
type Batch = { commitId: number; generation: number; ops: readonly Op[] };

// Thrown when a prop value has no binary value tag (spec §5.3: stringRef/i64/
// f64/bool/null only — no array/object tag). Callers (runtime/ndp.ts) catch
// this specifically to fall back to sending the offending batch as JSON;
// any other error out of this module is a genuine bug/corruption and should
// propagate uncaught.
export class BinaryUnsupportedValue extends Error {
  constructor(key: string, value: unknown) {
    super(`ndp-binary: unsupported prop value for "${key}" (${typeof value}) — spec §5.3 has no array/object tag`);
    this.name = "BinaryUnsupportedValue";
  }
}

// Reused across every string write in a single encode call (module-level:
// TextEncoder has no per-call state, so one instance amortizes across the
// whole process's lifetime instead of being re-allocated per encode).
const textEncoder = new TextEncoder();

// Single growable ArrayBuffer with one DataView + Uint8Array cached over it,
// both rebuilt only when the buffer itself grows (doubling) — this avoids
// the previous per-primitive-write `new DataView(...)` allocation, which
// dominated encode time on large op streams (see M10 task4 diagnosis).
class ByteWriter {
  private ab = new ArrayBuffer(256);
  private buf = new Uint8Array(this.ab);
  private view = new DataView(this.ab);
  len = 0;
  private ensure(n: number) {
    if (this.len + n <= this.ab.byteLength) return;
    let cap = this.ab.byteLength * 2;
    while (cap < this.len + n) cap *= 2;
    const nextAb = new ArrayBuffer(cap);
    new Uint8Array(nextAb).set(this.buf.subarray(0, this.len));
    this.ab = nextAb;
    this.buf = new Uint8Array(nextAb);
    this.view = new DataView(nextAb);
  }
  u8(v: number) {
    this.ensure(1);
    this.buf[this.len++] = v & 0xff;
  }
  u16(v: number) {
    this.ensure(2);
    this.view.setUint16(this.len, v, true);
    this.len += 2;
  }
  u32(v: number) {
    this.ensure(4);
    this.view.setUint32(this.len, v >>> 0, true);
    this.len += 4;
  }
  i64(v: number) {
    this.ensure(8);
    this.view.setBigInt64(this.len, BigInt(v), true);
    this.len += 8;
  }
  f64(v: number) {
    this.ensure(8);
    this.view.setFloat64(this.len, v, true);
    this.len += 8;
  }
  bytes(b: Uint8Array) {
    this.ensure(b.length);
    this.buf.set(b, this.len);
    this.len += b.length;
  }
  // Writes a u32 length prefix followed by the string's UTF-8 bytes, encoding
  // directly into this writer's own buffer (worst case 3 bytes/UTF-16 code
  // unit) instead of allocating an intermediate Uint8Array via `encode()`.
  lenString(s: string) {
    this.ensure(4 + s.length * 3);
    const lenPos = this.len;
    const { written } = textEncoder.encodeInto(s, this.buf.subarray(this.len + 4));
    this.view.setUint32(lenPos, written, true);
    this.len += 4 + written;
  }
  slice(): Uint8Array {
    return this.buf.slice(0, this.len);
  }
}

export function encodeCommitBatchBinary(batch: Batch): Uint8Array {
  const strings: string[] = [];
  const index = new Map<string, number>();
  const intern = (s: string): number => {
    const hit = index.get(s);
    if (hit !== undefined) return hit;
    const i = strings.length;
    strings.push(s);
    index.set(s, i);
    return i;
  };

  const ops = new ByteWriter();
  // Parity with the JSON path: props are sent verbatim (all keys, including
  // e.g. testID), so the binary encoder must not filter any prop key.
  const writeProps = (props: Record<string, unknown>) => {
    const keys = Object.keys(props);
    ops.u16(keys.length);
    for (const k of keys) {
      ops.u32(intern(k));
      const v = props[k];
      if (v === null || v === undefined) {
        ops.u8(0x00);
      } else if (typeof v === "boolean") {
        ops.u8(0x01);
        ops.u8(v ? 1 : 0);
      } else if (typeof v === "number") {
        if (Number.isInteger(v)) {
          ops.u8(0x02);
          ops.i64(v);
        } else {
          ops.u8(0x03);
          ops.f64(v);
        }
      } else if (typeof v === "string") {
        ops.u8(0x04);
        ops.u32(intern(v));
      } else {
        throw new BinaryUnsupportedValue(k, v);
      }
    }
  };

  for (const op of batch.ops) {
    switch (op.op) {
      case "create": {
        ops.u8(0x01);
        ops.u32(op.id);
        const wt = WIDGET_TYPE[op.widget];
        if (wt === undefined) throw new Error(`ndp-binary: unknown widget "${op.widget}"`);
        ops.u16(wt);
        writeProps(op.props);
        break;
      }
      case "append":
        ops.u8(0x02);
        ops.u32(op.parent);
        ops.u32(op.child);
        break;
      case "insertBefore":
        ops.u8(0x03);
        ops.u32(op.parent);
        ops.u32(op.child);
        ops.u32(op.before ?? 0);
        break;
      case "remove":
        ops.u8(0x04);
        ops.u32(op.id);
        break;
      case "setText":
        ops.u8(0x05);
        ops.u32(op.id);
        ops.u32(intern(op.text));
        break;
      case "update":
        ops.u8(0x06);
        ops.u32(op.id);
        writeProps(op.props);
        break;
      case "hide":
        ops.u8(0x07);
        ops.u32(op.id);
        break;
      case "unhide":
        ops.u8(0x08);
        ops.u32(op.id);
        break;
    }
  }

  const opBytes = ops.slice();
  const stringTableOffset = 28 + opBytes.length;

  const out = new ByteWriter();
  out.u8(0x4e);
  out.u8(0x01);
  for (let i = 0; i < 6; i++) out.u8(0);
  // commitId u64 — written via setBigInt64 (i64 view of the u64 field).
  // Documented deviation: JS has no native u64; for all real commit ids
  // (monotonically increasing, far below 2^63) the LE bytes are identical
  // to a u64 write, and the Zig side reads the same 8 LE bytes either way.
  out.i64(batch.commitId);
  out.u32(batch.generation);
  out.u32(batch.ops.length);
  out.u32(stringTableOffset);
  out.bytes(opBytes);
  out.u32(strings.length);
  for (const s of strings) out.lenString(s);
  return out.slice();
}
