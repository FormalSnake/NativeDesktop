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
  private buf: Uint8Array;
  private view: DataView;
  // Capacity mirrored in a plain number field: reading `buf.byteLength` on
  // every primitive write costs measurably more than a field load, and this
  // bound is checked a few hundred thousand times on a 10k-node mount.
  private cap: number;
  len = 0;
  constructor(capacity: number) {
    this.buf = new Uint8Array(capacity);
    this.view = new DataView(this.buf.buffer);
    this.cap = capacity;
  }
  private grow(n: number) {
    let cap = this.cap * 2;
    while (cap < this.len + n) cap *= 2;
    const next = new Uint8Array(cap);
    next.set(this.buf.subarray(0, this.len));
    this.buf = next;
    this.view = new DataView(next.buffer);
    this.cap = cap;
  }
  u8(v: number) {
    if (this.len + 1 > this.cap) this.grow(1);
    this.buf[this.len++] = v & 0xff;
  }
  u16(v: number) {
    if (this.len + 2 > this.cap) this.grow(2);
    this.view.setUint16(this.len, v, true);
    this.len += 2;
  }
  u32(v: number) {
    if (this.len + 4 > this.cap) this.grow(4);
    this.view.setUint32(this.len, v >>> 0, true);
    this.len += 4;
  }
  i64(v: number) {
    if (this.len + 8 > this.cap) this.grow(8);
    this.view.setBigInt64(this.len, BigInt(v), true);
    this.len += 8;
  }
  f64(v: number) {
    if (this.len + 8 > this.cap) this.grow(8);
    this.view.setFloat64(this.len, v, true);
    this.len += 8;
  }
  // Writes a u32 length prefix followed by the string's UTF-8 bytes, encoding
  // directly into this writer's own buffer (worst case 3 bytes/UTF-16 code
  // unit) instead of allocating an intermediate Uint8Array via `encode()`.
  // Widget prop names and most prop values are short ASCII, and for those a
  // charCodeAt loop beats `encodeInto` several times over, mostly because
  // `encodeInto` needs a `subarray` view per call. A byte over 127 abandons
  // the loop and re-encodes the whole string through `encodeInto`, so the
  // bytes written are the same either way.
  lenString(s: string) {
    const n = s.length;
    if (this.len + 4 + n * 3 > this.cap) this.grow(4 + n * 3);
    const lenPos = this.len;
    let p = lenPos + 4;
    let ascii = true;
    for (let i = 0; i < n; i++) {
      const c = s.charCodeAt(i);
      if (c > 127) {
        ascii = false;
        break;
      }
      this.buf[p++] = c;
    }
    if (!ascii) {
      const { written } = textEncoder.encodeInto(s, this.buf.subarray(lenPos + 4));
      p = lenPos + 4 + written;
    }
    this.view.setUint32(lenPos, p - lenPos - 4, true);
    this.len = p;
  }
  // Backpatch helpers for fields whose value is only known once the stretch
  // they measure has been written (header stringTableOffset, per-op prop
  // count). The slot must already have been reserved by a matching write.
  patchU16(pos: number, v: number) {
    this.view.setUint16(pos, v, true);
  }
  patchU32(pos: number, v: number) {
    this.view.setUint32(pos, v >>> 0, true);
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
  // Consecutive ops of the same widget carry the same prop names in the same
  // order, so the previous key's ref answers most key lookups by identity
  // compare instead of a Map hash. Values are deliberately not cached: they
  // are mostly distinct, so the check would never pay for itself.
  let lastKey = "";
  let lastKeyRef = 0;
  const internKey = (k: string): number => {
    if (k === lastKey) return lastKeyRef;
    lastKey = k;
    lastKeyRef = intern(k);
    return lastKeyRef;
  };

  // The op stream starts at the fixed 28-byte header's end, so writing it
  // straight into the output buffer behind a reserved header lands every byte
  // at its final offset the first time it is written. The header's two
  // length-dependent fields are backpatched once their stretch is closed.
  //
  // Sized up front from the op count. Growing from a small buffer costs a
  // realloc plus a copy of everything written so far at every doubling, and on
  // a 10k-node mount those copies were the single largest term in encode time.
  // 32 bytes/op covers the ops themselves and typical short string-table
  // entries; a batch that outgrows it still grows correctly, just slower.
  const out = new ByteWriter(32 * batch.ops.length + 4096);
  out.u8(0x4e);
  out.u8(0x01);
  for (let i = 0; i < 6; i++) out.u8(0);
  // commitId u64, written via setBigInt64 (an i64 view of the u64 field).
  // Documented deviation: JS has no native u64; for all real commit ids
  // (monotonically increasing, far below 2^63) the LE bytes are identical
  // to a u64 write, and the Zig side reads the same 8 LE bytes either way.
  out.i64(batch.commitId);
  out.u32(batch.generation);
  out.u32(batch.ops.length);
  const stringTableOffsetPos = out.len;
  out.u32(0);

  // Parity with the JSON path: props are sent verbatim (all keys, including
  // e.g. testID), so the binary encoder must not filter any prop key. `for in`
  // reports own enumerable string keys in the same order Object.keys does
  // (props objects are plain literals, with nothing enumerable on the
  // prototype chain) without allocating a key array per op, so the count it
  // would have supplied up front is backpatched instead.
  const writeProps = (props: Record<string, unknown>) => {
    const countPos = out.len;
    out.u16(0);
    let count = 0;
    for (const k in props) {
      count++;
      out.u32(internKey(k));
      const v = props[k];
      if (v === null || v === undefined) {
        out.u8(0x00);
      } else if (typeof v === "boolean") {
        out.u8(0x01);
        out.u8(v ? 1 : 0);
      } else if (typeof v === "number") {
        if (Number.isInteger(v)) {
          out.u8(0x02);
          out.i64(v);
        } else {
          out.u8(0x03);
          out.f64(v);
        }
      } else if (typeof v === "string") {
        out.u8(0x04);
        out.u32(intern(v));
      } else {
        throw new BinaryUnsupportedValue(k, v);
      }
    }
    out.patchU16(countPos, count);
  };

  for (const op of batch.ops) {
    switch (op.op) {
      case "create": {
        out.u8(0x01);
        out.u32(op.id);
        const wt = WIDGET_TYPE[op.widget];
        if (wt === undefined) throw new Error(`ndp-binary: unknown widget "${op.widget}"`);
        out.u16(wt);
        writeProps(op.props);
        break;
      }
      case "append":
        out.u8(0x02);
        out.u32(op.parent);
        out.u32(op.child);
        break;
      case "insertBefore":
        out.u8(0x03);
        out.u32(op.parent);
        out.u32(op.child);
        out.u32(op.before ?? 0);
        break;
      case "remove":
        out.u8(0x04);
        out.u32(op.id);
        break;
      case "setText":
        out.u8(0x05);
        out.u32(op.id);
        out.u32(intern(op.text));
        break;
      case "update":
        out.u8(0x06);
        out.u32(op.id);
        writeProps(op.props);
        break;
      case "hide":
        out.u8(0x07);
        out.u32(op.id);
        break;
      case "unhide":
        out.u8(0x08);
        out.u32(op.id);
        break;
    }
  }

  out.patchU32(stringTableOffsetPos, out.len);
  out.u32(strings.length);
  for (const s of strings) out.lenString(s);
  return out.slice();
}
