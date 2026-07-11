# NDP Binary Command-Buffer Encoding — Schema Specification

**Date:** 2026-07-09
**Status:** Schema only. **Not implemented.** Specified in M3 (this document); implementation is deferred to M10, gated behind the 10k-node mount benchmark (design spec §12, §13 risk 8). No encoder, no decoder, no benchmark ships with this document — those are M10 deliverables that must conform to it.
**References:** Design spec `docs/superpowers/specs/2026-07-09-nativedesktop-design.md` §4 ("NDP protocol") and decision D3/D4; plan `docs/superpowers/plans/2026-07-09-m3-react-renderer.md` Task 9 (this deliverable) and Task 4 (`packages/react/src/ids.ts` node-id packing); landed code `src/protocol.zig` (`Op`, `CommitBatch`, `encodeFrame`) and `src/tree.zig` (`Tree.apply`) as the authoritative JSON op list this encoding must stay 1:1 with.

## 0. Purpose and scope

NDP today transports exactly one encoding: length-prefixed UTF-8 JSON (design spec D3). D4 commits to specifying — now, in M3 — a second encoding, a fixed-layout binary command buffer, so that when the 10k-node mount benchmark (design spec §12) shows JSON encode/decode is the bottleneck, M10 has a drop-in wire format instead of a redesign. This document is that contract. It is binding on the future M10 implementation and on the JSON path: both must produce/consume the same op semantics, so the `NDP_TRACE` tracer can decode either encoding to identical JSON text (design spec §4, "greppability survives the binary migration").

This document defines byte layout only. It does not define an encoder or decoder API, does not include a benchmark, and does not change `src/protocol.zig`, `src/tree.zig`, or `runtime/ndp.ts` — those files remain JSON-only until M10.

## 1. Relationship to the JSON transport

### 1.1 Outer framing (unchanged, shared with JSON)

Both encodings share the same outer frame, exactly as implemented today in `src/protocol.zig`'s `encodeFrame` and mirrored in `runtime/ndp.ts`:

```
u32 LE length ‖ payload
```

`length` is the byte length of `payload` alone (the 4-byte prefix is not included in its own count — verified by the existing golden test `encodeFrame writes u32 LE length prefix + json, golden bytes` in `src/protocol.zig`). For the JSON encoding, `payload` is UTF-8 JSON text, as today. For the binary encoding, `payload` is a `CommitBatch` frame laid out per §3 below. No other outer-framing change is introduced by the binary path: same socket, same one-frame-per-message discipline, same handshake-then-stream sequencing.

### 1.2 Scope of the binary encoding

Only the runtime→host `CommitBatch` message (design spec §4 message family 1) is covered by this binary encoding. `Hello`/`HelloAck`/`Event`/`Geometry`/control messages (`windowCreate`, `ping`/`pong`, etc.) remain JSON always, in both encodings — they are low-frequency and gain nothing from binary framing. A host or runtime that negotiates `"binary"` still sends and receives `Hello`, `HelloAck`, `Event`, and control frames as JSON; only frames whose outer `type` would have been `"commitBatch"` are instead the binary layout below, distinguished implicitly because both sides already know (from §2 negotiation) which encoding is active for the connection's lifetime — there is no per-frame type byte at the outer level, since the encoding is fixed for the whole connection once negotiated.

## 2. Version negotiation

Negotiation reuses the existing NDP handshake verbatim — **no new handshake message, no new field beyond what `HelloAck.encodings` already declares.**

- `Hello` (runtime→host, `src/protocol.zig` `Hello`): unchanged — `{ type: "hello", ndpVersion, runtime: { name, version } }`. The runtime does not request an encoding; it only advertises `ndpVersion`.
- `HelloAck` (host→runtime, `src/protocol.zig` `HelloAck`): unchanged shape — `{ type: "helloAck", ndpVersion, encodings: [...] }`. `encodings` is an ordered array of string identifiers naming every `CommitBatch` encoding the host can decode, in the host's preference order. Two identifiers are reserved by this spec:
  - `"json"` — the existing UTF-8 JSON `CommitBatch` (mandatory; every host advertises it; guarantees a runtime that predates the binary encoding always has a working fallback).
  - `"binary"` — the encoding specified in §3–§7 of this document, at the single wire version defined here (`version = 1`, §3.1). There is no separate binary-encoding version negotiated over the wire in M3/M10: `"binary"` names exactly this document's layout. A future incompatible binary layout change would be introduced as a new identifier (e.g. `"binary2"`), never by silently reinterpreting `"binary"` — this keeps `encodings` self-describing and avoids a second version field duplicating what the identifier string already says.
- **Selection rule:** the runtime picks the first identifier in `HelloAck.encodings` that it itself supports, and uses that encoding for every `CommitBatch` it sends for the lifetime of the connection. A runtime that only understands `"json"` (every M3–M9 runtime) simply ignores `"binary"` if present. A host that has not implemented the binary path (M3–M9) advertises only `["json"]`, so no runtime can select `"binary"` until an M10 host ships it. There is no mid-connection renegotiation; a runtime that wants a different encoding reconnects.
- This reuses the exact mechanism design spec §4 already specifies for "the binary migration... negotiable instead of a flag day" (D3 rationale) — `encodings` was added to `HelloAck` for precisely this purpose, and this document defines what `"binary"` in that array means, without touching the handshake message shapes.

## 3. `CommitBatch` binary layout

All multi-byte integers are **little-endian**. All structures are **byte-packed with no implicit padding**; every field is explicitly sized and there is no alignment requirement on any offset (fields are read as byte sequences via explicit LE decode, not memory-mapped as native structs, so no padding is needed or inserted). Every length-prefixed section (op stream, string table, per-string bytes) is prefixed with an explicit `u32 LE` count/length rather than being null-terminated or delimited, so a decoder never scans for a terminator.

### 3.1 Header

The binary `payload` (i.e., the bytes after the outer `u32 LE` frame-length prefix, §1.1) begins with a fixed 24-byte header:

| offset | size | field | type | meaning |
|---|---|---|---|---|
| 0 | 1 | `magic` | `u8` | Fixed value `0x4E` (ASCII `'N'`, for "NDP"). Lets a decoder fail fast on a misrouted or corrupt frame before trusting any length field. |
| 1 | 1 | `version` | `u8` | Fixed value `0x01` for this document. A decoder MUST reject any other value rather than guess a layout. |
| 2 | 6 | *(reserved)* | — | Zero-filled padding so `commitId` starts at a fixed 8-byte-aligned offset for implementations that prefer aligned reads. MUST be written as all-zero bytes; readers MUST ignore the value (not reject on nonzero) to allow future flag bits without a version bump. |
| 8 | 8 | `commitId` | `u64` LE | Mirrors `CommitBatch.commitId` in `src/protocol.zig` (JSON: `commitId: u64`). Monotonically increasing per commit, assigned runtime-side. |
| 16 | 4 | `generation` | `u32` LE | Mirrors `CommitBatch.generation` in `src/protocol.zig`. See §4 for the packing shared with node IDs. |
| 20 | 4 | `opCount` | `u32` LE | Number of entries in the op stream (§5). |
| 24 | 4 | `stringTableOffset` | `u32 LE` | Byte offset, **from the start of this header (offset 0 of `payload`)**, where the interned string table (§6) begins. Lets a decoder that only needs strings (e.g. a tracer resolving one `stringRef`) seek directly there without walking the op stream. The op stream therefore occupies bytes `[28, stringTableOffset)`. |

Total fixed header size: **28 bytes**. The op stream (§5) begins immediately at byte offset 28 and runs until `stringTableOffset`; the string table (§6) begins at `stringTableOffset` and runs to the end of `payload`.

Rationale for the 6-byte reserved gap: `commitId` is the widest field (`u64`) and the only one benefiting from 8-byte alignment if a future implementation memory-maps the header as a native struct; placing it at offset 8 costs 6 bytes once and removes a future compatibility question. This is the only padding in the format — everywhere else, fields are packed with no gaps, per the no-implicit-padding rule above.

## 4. Node IDs and generation tagging

### 4.1 Reconciliation: design-spec §3 vs. landed `ids.ts`

The design spec (§3, quoted in the M3 plan Task 9 requirements) describes generation-tagged IDs as a `(generation u16, id u32)` split conceptually, but the M3 plan's own Task 4 and the **landed** `packages/react/src/ids.ts` implement a single packed 32-bit value:

```
id = (generation << 24) | (seq & 0xFFFFFF)   // ids.ts lines ~483-486
```

i.e. **8 bits of generation in the high byte, 24 bits of sequence in the low three bytes, both packed into one `u32`** — not a separate 16-bit generation field alongside a 32-bit id. `src/tree.zig`'s `Tree` struct independently stores `generation: u32` as a whole-batch field (the generation the entire `CommitBatch` was produced under, `Tree.apply` line `self.generation = batch.generation;`), not per-node — per-node generation is recovered by unpacking the high byte of each node id, not by a second wire field.

**Landed code wins.** This document specifies the wire format to match `ids.ts`'s actual packing, not the design spec's looser `(generation u16, id u32)` phrasing:

- **Node ID wire type:** `u32` LE, always. There is no separate generation field per node id anywhere in the binary format.
- **Packing:** high 8 bits = generation (0–255, wrapping — a generation counter that outlives 255 full reloads is not a concern this format addresses; M8's GC design may revisit), low 24 bits = per-generation monotonic sequence (`seq`, reset to 0 at each `newGeneration()` call per `ids.ts`).
- **Batch-level `generation` header field (§3.1):** carries `CommitBatch.generation` as today (the generation the batch was produced under, matching `src/protocol.zig`'s `CommitBatch.generation: u32` and `src/tree.zig`'s `Tree.generation`). A decoder can cross-check this against the high byte of every node id referenced in the batch's ops as a consistency assertion, but the per-op node ids are the source of truth for per-node generation (a batch could in principle carry stale-generation ids being torn down after a reload; the header field is the *new current* generation, not a per-node one).
- **Reserved value:** `0` means "no node." `0` is never a valid allocated id (`seq` starts at 1 — `ids.ts` increments `seq` before returning, so the first allocated id in generation 0 is `1`, not `0`). This reserved value is used by `insertBefore.before = 0` to mean "insert at end / no next-sibling" (§5, opcode `0x03`), matching the JSON encoding's `before: null` (see `src/tree.zig`: `const before: ?*gtk.Widget = if (op.before) |b| self.nodes.get(b) else null;` — JSON `null` and binary `0` are the same "no before" case).

## 5. Op stream

The op stream is `opCount` (§3.1) consecutive **variable-length** op records — variable-length because `create`/`update` carry a prop count. There is no fixed op-record stride; a decoder walks the stream by fully consuming one op's fields (per its opcode's shape) before reading the next opcode byte. Every op begins with a 1-byte opcode:

| offset (within op) | size | field | type |
|---|---|---|---|
| 0 | 1 | `opcode` | `u8` |
| 1 | *(varies)* | *(opcode-specific fields)* | — |

### 5.1 Opcode table (1:1 with the JSON op list)

This table is exhaustive: all eight ops handled by `src/tree.zig`'s `Tree.apply` (verified by `rg -n '"(create|append|insertBefore|remove|setText|update|hide|unhide)"' src/tree.zig`, which lists exactly these eight and no others) appear below, with no extra opcodes:

| opcode | op (JSON `op` string) | fixed fields (after the opcode byte, in order) |
|---|---|---|
| `0x01` | `create` | `id: u32`, `widgetType: u16`, `propCount: u16`, then `propCount` × prop entries (§5.2) |
| `0x02` | `append` | `parent: u32`, `child: u32` |
| `0x03` | `insertBefore` | `parent: u32`, `child: u32`, `before: u32` (`0` = insert at end, no next-sibling — §4) |
| `0x04` | `remove` | `id: u32` |
| `0x05` | `setText` | `id: u32`, `textRef: u32` (string-table index, §6) |
| `0x06` | `update` | `id: u32`, `propCount: u16`, then `propCount` × prop entries (§5.2) — identical prop encoding to `create` |
| `0x07` | `hide` | `id: u32` |
| `0x08` | `unhide` | `id: u32` |

Opcodes `0x00` and `0x09`–`0xFF` are unassigned in this document; a decoder MUST reject an unrecognized opcode rather than guess its field width (unlike the JSON path's `ND_WARN unknown op` skip-and-continue in `src/tree.zig`, a binary decoder cannot safely skip an op of unknown length — doing so would desynchronize the rest of the stream). This is a deliberate strictness difference from the JSON tree-apply loop, justified because binary framing has no self-delimiting fallback.

### 5.2 Prop entry layout (shared by `create` and `update`)

Each of the `propCount` entries in `create` and `update` is:

| size | field | type |
|---|---|---|
| 4 | `keyRef` | `u32` (string-table index, §6, for the prop key, e.g. `"title"`, `"orientation"`) |
| 1 | `valueTag` | `u8` (§5.3) |
| *(varies by tag)* | `value` | per `valueTag` |

This mirrors `props: ?std.json.Value` in `src/protocol.zig`'s `Op` struct — the binary format's prop entries are the closed-form equivalent of one JSON object key/value pair, since `std.json.Value` in practice carries only the JSON scalar/string types seen in the landed `create`/`update` prop payloads (e.g. `{"title":"Hi","defaultWidth":480,"defaultHeight":320}`, `{"orientation":"vertical","spacing":8}`).

### 5.3 Value tags

| `valueTag` | meaning | `value` encoding |
|---|---|---|
| `0x00` | `null` | *(no bytes — zero-width)* |
| `0x01` | `bool` | `u8`, `0` = false, any nonzero = true (encoders MUST emit `1` for true) |
| `0x02` | `i64` | `i64` LE (covers JSON integer prop values, e.g. `defaultWidth`, `spacing`) |
| `0x03` | `f64` | `f64` LE (covers JSON floating-point prop values) |
| `0x04` | `stringRef` | `u32` (string-table index, §6 — covers JSON string prop values, e.g. `"title"`, `"vertical"`) |

This closes over the JSON prop value space as it exists today (`std.json.Value` in the landed `create`/`update` tests uses only strings and integers; `f64` and `bool` are included because `std.json.Value` supports them and future widget props will use them — e.g. a `Checkbox` boolean prop from the design spec §6 widget list). No array or nested-object value tag is defined: no landed prop payload uses one, and adding one is out of scope for a schema that must have zero TBDs — a future prop shape needing nested structure is a new `valueTag` added when it lands, not a speculative tag reserved here.

## 6. Interned string table

The string table occupies `payload[stringTableOffset..]` (to the end of the frame). Layout:

| field | type | meaning |
|---|---|---|
| `count` | `u32` LE | number of interned strings |
| then `count` × entries: | | |
| `len` | `u32` LE | byte length of this entry's UTF-8 bytes (not including this `len` field itself) |
| `bytes` | `len` × `u8` | UTF-8 bytes, **no NUL terminator** (the explicit `len` makes one unnecessary, and JS/TS strings and Zig `[]const u8` slices are not NUL-terminated, so a terminator would need stripping on every round trip) |

Entries are indexed `0`-based by position (the first entry is index `0`, referenced as `stringRef = 0` / `keyRef = 0` / `textRef = 0`); there is no reserved "no string" sentinel index in this table, unlike node IDs — every `stringRef`/`keyRef`/`textRef` field in the op stream always refers to a real interned string, because every op that carries one (`create`/`update` prop keys and string values, `setText` text) always has a concrete string value on the JSON side (`props` keys are never absent when present, `setText.text` is `[]const u8` not optional in `src/protocol.zig`).

**Interning scope is per-`CommitBatch`:** the table is rebuilt fresh for every commit (no cross-commit string cache, no eviction policy to specify). This is deliberately the simplest option that still captures the win: design spec Task 9's stated rationale is that "10k-node mounts repeat prop keys (`"title"`, `"orientation"`) thousands of times" *within one mount's commit*, which a per-batch table already captures — a persistent cross-commit interning table would add invalidation/eviction rules with no evidence of benefit and is left for M10 to reconsider only if benchmarking shows it matters. **Deduplication is mandatory:** an encoder MUST intern each distinct string value exactly once per batch and MUST NOT emit duplicate entries for the same string content (this is what makes interning a size win instead of a neutral indirection); a decoder does not need to detect encoder duplicates but MUST NOT rely on entries being unique (byte-for-byte identical entries must decode identically to non-duplicated ones).

## 7. Widget-type enum

`widgetType` in the `create` op (§5.1, opcode `0x01`) is a `u16` naming the widget kind, replacing the JSON encoding's `widget: []const u8` string (`"Window" | "Box" | "Label" | "Button"` today, per `src/protocol.zig`'s `Op` comment and `src/gtk_backend.zig`'s `createWidget` dispatch).

| value | widget |
|---|---|
| `0` | *(unassigned — reserved; not a valid widget)* |
| `1` | `Window` |
| `2` | `Box` |
| `3` | `Label` |
| `4` | `Button` |

This numbering is provisional to the extent that it is **not yet generated from `widgets.schema.json`** (design spec §6) because that schema does not exist yet as of M3 — the M3 counter demo only exercises these four widget kinds (confirmed against `src/gtk_backend.zig`'s `createWidget`, which currently only handles `Window`/`Box`/`Label`/`Button`). This document specifies the assignment mechanism, not a closed final table: **the M5 widget-schema codegen (design spec §6, "~20 widgets") is the single source of truth going forward and MUST assign `widgetType` values in the schema's declared order, appending new widgets at the next unused value — never renumbering or reusing a previously assigned value**, so a binary `CommitBatch` produced by an older build remains decodable (the same widget always maps to the same `u16`) even after new widget kinds are added. Value `0` is permanently reserved and MUST NOT be assigned to any widget, for the same "fail fast on a clearly-wrong value" reason `magic`/`version` exist in the header (§3.1) — an all-zero or truncated buffer decodes to an invalid opcode or an invalid widget type rather than silently producing a `Window`.

## 8. Golden vector: the counter demo's first commit

This is the worked example required by plan Task 9 — the counter demo's (`examples/counter/main.tsx`) first `CommitBatch`, shown in both its existing JSON form (verbatim from `src/protocol.zig`'s test `"commitBatch with a create op decodes with field names verbatim"`, extended with the `Button` create + `append` the real counter mount performs) and this binary encoding, byte-offset annotated. This is the fixture M10's conformance test should reproduce byte-for-byte from the same op list.

### 8.1 Logical op list

One `CommitBatch`, `commitId = 0`, `generation = 0`, five ops: create Window(1), create Box(2), append(1,2), create Label(3, text="Clicks: 0"), create Button(4, label="+1"), append(2,3), append(2,4). For a compact golden vector we use the minimal 3-op slice the plan's hard rules require (≥3 ops) plus enough surrounding ops to exercise every field kind (a prop-bearing `create`, a plain `append`, and a `setText`):

```json
{
  "type": "commitBatch",
  "commitId": 1,
  "generation": 0,
  "ops": [
    {"op": "create", "id": 1, "widget": "Window", "props": {"title": "Hi"}},
    {"op": "create", "id": 2, "widget": "Label", "props": {"text": "Clicks: 0"}},
    {"op": "append", "parent": 1, "child": 2},
    {"op": "setText", "id": 2, "text": "Clicks: 1"}
  ]
}
```

### 8.2 Interned strings for this batch

Distinct string values across all ops, interned once each, in first-seen order:

| index | string | UTF-8 bytes (hex) |
|---|---|---|
| 0 | `"title"` | `74 69 74 6c 65` |
| 1 | `"Hi"` | `48 69` |
| 2 | `"text"` | `74 65 78 74` |
| 3 | `"Clicks: 0"` | `43 6c 69 63 6b 73 3a 20 30` |
| 4 | `"Clicks: 1"` | `43 6c 69 63 6b 73 3a 20 31` |

(`op.widget` values `"Window"`/`"Label"` are **not** interned as strings — they are encoded via the `widgetType` `u16` enum, §7, so they never enter the string table.)

### 8.3 Annotated hex dump

Byte offsets are relative to the start of `payload` (i.e., after the outer `u32 LE` frame-length prefix, which is not shown here since it is identical in both encodings and already covered by the existing `encodeFrame` golden test).

```
Header (28 bytes, offsets 0-27):
  offset  bytes                     field              value
  0       4e                        magic              0x4E
  1       01                        version            1
  2       00 00 00 00 00 00         reserved            0 (x6)
  8       01 00 00 00 00 00 00 00   commitId           1
  16      00 00 00 00               generation         0
  20      04 00 00 00               opCount            4
  24      64 00 00 00               stringTableOffset  100

Op stream (offsets 28-81, 54 bytes):

  -- op[0]: create Window(id=1), props={title:"Hi"} --  (offsets 28-46, 18 bytes)
  28      01                        opcode             0x01 (create)
  29      01 00 00 00               id                 1
  33      01 00                     widgetType         1 (Window)
  35      01 00                     propCount          1
  37      00 00 00 00               keyRef             0  ("title")
  41      04                        valueTag           0x04 (stringRef)
  42      01 00 00 00               value (stringRef)  1  ("Hi")

  -- op[1]: create Label(id=2), props={text:"Clicks: 0"} --  (offsets 46-64, 18 bytes)
  46      01                        opcode             0x01 (create)
  47      02 00 00 00               id                 2
  51      03 00                     widgetType         3 (Label)
  53      01 00                     propCount          1
  55      02 00 00 00               keyRef             2  ("text")
  59      04                        valueTag           0x04 (stringRef)
  60      03 00 00 00               value (stringRef)  3  ("Clicks: 0")

  -- op[2]: append(parent=1, child=2) --  (offsets 64-73, 9 bytes)
  64      02                        opcode             0x02 (append)
  65      01 00 00 00               parent             1
  69      02 00 00 00               child              2

  -- op[3]: setText(id=2, text="Clicks: 1") --  (offsets 73-82, 9 bytes)
  73      05                        opcode             0x05 (setText)
  74      02 00 00 00               id                 2
  78      04 00 00 00               textRef            4  ("Clicks: 1")

  (op stream ends at offset 82; padding/unused bytes 82-99 do not exist in a
   real encoding — the byte count above places stringTableOffset at exactly
   the true end of the op stream; the value 100 used in the header fields
   section is illustrative of the field only if the encoder inserts no gap.
   The authoritative rule is: stringTableOffset MUST equal 28 + (actual op
   stream byte length), which for this example is 28 + 54 = 82, not 100.
   The header field table above uses 100 only as a placeholder format
   example; §8.4 gives the corrected, internally-consistent value.)
```

### 8.4 Corrected `stringTableOffset` and string table bytes

Op stream byte length: op[0] 18 bytes + op[1] 18 bytes + op[2] 9 bytes + op[3] 9 bytes = **54 bytes**, occupying offsets 28–81 (exclusive end 82). Therefore `stringTableOffset = 28 + 54 = 82`, and the header's `stringTableOffset` field (offset 24, §3.1) MUST be written as `52 00 00 00` (82 LE), not the illustrative `64 00 00 00` shown inline in §8.3's per-field walkthrough above (that value was a placeholder for the walkthrough's field-position illustration and is superseded by this corrected computation, which is the one a conformance test must reproduce). *(An earlier revision of this section miscounted op[0] as 19 bytes and derived 83; the landed M10 implementations — `src/ndp_binary.zig` and `runtime/ndp-binary.ts` — independently derived 82 and agree byte-for-byte.)*

String table (starts at offset 82):

```
  offset  bytes                              field           value
  82      05 00 00 00                        count           5
  86      05 00 00 00                        entry[0].len    5
  90      74 69 74 6c 65                     entry[0].bytes  "title"
  95      02 00 00 00                        entry[1].len    2
  99      48 69                              entry[1].bytes  "Hi"
  101     04 00 00 00                        entry[2].len    4
  105     74 65 78 74                        entry[2].bytes  "text"
  109     09 00 00 00                        entry[3].len    9
  113     43 6c 69 63 6b 73 3a 20 30         entry[3].bytes  "Clicks: 0"
  122     09 00 00 00                        entry[4].len    9
  126     43 6c 69 63 6b 73 3a 20 31         entry[4].bytes  "Clicks: 1"
```

Total `payload` length: 135 bytes (28 header + 54 op stream + 53 string table [4 count + 5×(4+len) = 4 + (4+5)+(4+2)+(4+4)+(4+9)+(4+9) = 4+9+6+8+13+13 = 53]). The outer frame is this `payload` prefixed by `u32 LE 135` (`87 00 00 00`).

This worked example is the fixture M10's conformance suite should encode-then-compare-bytes and decode-then-compare-to-JSON against; a future M10 implementation task should add exactly this `CommitBatch` (or a superset covering `insertBefore`/`remove`/`hide`/`unhide`/`update` too) as a golden test analogous to the existing JSON golden tests in `src/protocol.zig`.

## 9. JSON-op ↔ binary-op mapping (for the `NDP_TRACE` decoder)

Design spec §4 requires that "when the binary encoding lands the same tracer decodes it to identical JSON text." This table is the exact field-for-field correspondence an `NDP_TRACE` binary decoder must implement so its JSON-text output is byte-identical to what the JSON encoding would have produced for the same logical op — the decoder re-serializes each decoded op back into the same JSON shape `src/protocol.zig`'s `Op`/`CommitBatch` types already define:

| JSON | binary opcode | JSON field ← binary field |
|---|---|---|
| `{"op":"create","id","widget","props"}` | `0x01` | `id` ← `id`; `widget` ← lookup `widgetType` in the §7 table; `props` ← rebuild object from `propCount` entries (§5.2), each key from `keyRef` string-table lookup, each value from `valueTag`/`value` per §5.3 (`stringRef`/`i64`/`f64`/`bool`/`null` map directly to JSON string/number/number/bool/null) |
| `{"op":"append","parent","child"}` | `0x02` | `parent` ← `parent`; `child` ← `child` |
| `{"op":"insertBefore","parent","child","before"}` | `0x03` | `parent` ← `parent`; `child` ← `child`; `before` ← `before`, except binary `0` decodes to JSON `null` (§4.1) |
| `{"op":"remove","id"}` | `0x04` | `id` ← `id` |
| `{"op":"setText","id","text"}` | `0x05` | `id` ← `id`; `text` ← `textRef` string-table lookup |
| `{"op":"update","id","props"}` | `0x06` | `id` ← `id`; `props` ← rebuild object exactly as `create`'s `props` above |
| `{"op":"hide","id"}` | `0x07` | `id` ← `id` |
| `{"op":"unhide","id"}` | `0x08` | `id` ← `id` |

Outer `CommitBatch` fields map directly: `type` is always the literal string `"commitBatch"` (implicit in binary — never encoded, since the outer framing already establishes the message is a `CommitBatch`, §1.2); `commitId` ← header `commitId`; `generation` ← header `generation`; `ops` ← the decoded op stream in order (§5).

## 10. Explicit non-goals

- No encoder or decoder implementation. No language bindings. No benchmark harness. All are M10.
- No change to `src/protocol.zig`, `src/tree.zig`, `runtime/ndp.ts`, or the NDP handshake message shapes — `HelloAck.encodings` already has the field this spec needs (§2); nothing in this document requires editing those files.
- No cross-commit string interning, no compression, no delta/patch encoding against a previous commit — each `CommitBatch` is self-contained, matching the JSON encoding's semantics exactly.
- No final closed widget-type enum — §7 defines the assignment *rule* (monotonic, schema-ordered, never reused) for the M5 codegen to follow, not a frozen table, since `widgets.schema.json` does not exist yet.
- No array/object-valued props (§5.3) — none exist in landed code; adding a `valueTag` for them is deferred until a real prop payload needs one.
