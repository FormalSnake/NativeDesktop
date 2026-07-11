import { test, expect } from "bun:test";
import { encodeCommitBatchBinary, BinaryUnsupportedValue } from "./ndp-binary";

// ndp-binary spec §8 golden vector.
const batch = {
  commitId: 1,
  generation: 0,
  ops: [
    { op: "create", id: 1, widget: "Window", props: { title: "Hi" } },
    { op: "create", id: 2, widget: "Label", props: { text: "Clicks: 0" } },
    { op: "append", parent: 1, child: 2 },
    { op: "setText", id: 2, text: "Clicks: 1" },
  ],
} as const;

const expected = new Uint8Array([
  0x4e, 0x01, 0, 0, 0, 0, 0, 0,
  1, 0, 0, 0, 0, 0, 0, 0, // commitId=1
  0, 0, 0, 0, // generation
  4, 0, 0, 0, // opCount
  // stringTableOffset = 28 (header) + 54 (op stream below) = 82.
  // Spec §8.3/§8.4 prose says 55/83, but that contradicts its own byte-offset
  // table in §8.3 (op[0] 28..46 = 18B, op[1] 46..64 = 18B, op[2] 64..73 = 9B,
  // op[3] 73..82 = 9B => 54B total) and its own restated rule "stringTableOffset
  // MUST equal 28 + actual op stream byte length". 82 is the value consistent
  // with the op-stream bytes actually written below (and with §3.1's rule).
  // Cross-checked byte-for-byte against the Zig decoder's golden fixture
  // (src/ndp_binary.zig, `golden_payload`, commit 8b818f7), which independently
  // reaches the same 82 and documents the identical deviation.
  82, 0, 0, 0, // stringTableOffset
  0x01, 1, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0x04, 1, 0, 0, 0, // create Window
  0x01, 2, 0, 0, 0, 3, 0, 1, 0, 2, 0, 0, 0, 0x04, 3, 0, 0, 0, // create Label
  0x02, 1, 0, 0, 0, 2, 0, 0, 0, // append
  0x05, 2, 0, 0, 0, 4, 0, 0, 0, // setText
  5, 0, 0, 0,
  5, 0, 0, 0, 0x74, 0x69, 0x74, 0x6c, 0x65,
  2, 0, 0, 0, 0x48, 0x69,
  4, 0, 0, 0, 0x74, 0x65, 0x78, 0x74,
  9, 0, 0, 0, 0x43, 0x6c, 0x69, 0x63, 0x6b, 0x73, 0x3a, 0x20, 0x30,
  9, 0, 0, 0, 0x43, 0x6c, 0x69, 0x63, 0x6b, 0x73, 0x3a, 0x20, 0x31,
]);

test("binary encode matches spec §8 golden bytes", () => {
  const got = encodeCommitBatchBinary(batch);
  expect(Array.from(got)).toEqual(Array.from(expected));
});

test("array-valued prop throws BinaryUnsupportedValue (spec §5.3 has no array tag)", () => {
  const arrayBatch = {
    commitId: 1,
    generation: 0,
    ops: [{ op: "create", id: 1, widget: "Select", props: { options: ["a", "b", "c"] } }],
  } as const;
  expect(() => encodeCommitBatchBinary(arrayBatch)).toThrow(BinaryUnsupportedValue);
});

test("object-valued prop throws BinaryUnsupportedValue (spec §5.3 has no object tag)", () => {
  const objectBatch = {
    commitId: 1,
    generation: 0,
    ops: [{ op: "create", id: 1, widget: "Box", props: { style: { color: "red" } } }],
  } as const;
  expect(() => encodeCommitBatchBinary(objectBatch)).toThrow(BinaryUnsupportedValue);
});
