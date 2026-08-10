// pngSize() reads only the 8-byte signature + IHDR chunk header (24 bytes
// total) — it never needs a full, CRC-valid PNG, so these fixtures only fill
// in that prefix.
import { test, expect } from "bun:test";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pngSize } from "../src/png.ts";

const SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

function ihdrPrefix(width: number, height: number): Uint8Array {
  const buf = new Uint8Array(24);
  buf.set(SIGNATURE, 0);
  const view = new DataView(buf.buffer);
  view.setUint32(8, 13, false); // IHDR chunk length
  buf.set([0x49, 0x48, 0x44, 0x52], 12); // "IHDR"
  view.setUint32(16, width, false);
  view.setUint32(20, height, false);
  return buf;
}

async function writeTemp(name: string, bytes: Uint8Array): Promise<string> {
  const path = join(tmpdir(), `nd-test-${name}-${Date.now()}-${Math.random().toString(36).slice(2)}.png`);
  await Bun.write(path, bytes);
  return path;
}

test("reads width/height from a well-formed IHDR prefix", async () => {
  const path = await writeTemp("ok", ihdrPrefix(1100, 700));
  await expect(pngSize(path)).resolves.toEqual({ width: 1100, height: 700 });
});

test("rejects a file with the wrong signature", async () => {
  const bytes = ihdrPrefix(100, 100);
  bytes[0] = 0x00;
  const path = await writeTemp("badsig", bytes);
  await expect(pngSize(path)).rejects.toThrow(/not a PNG/);
});

test("rejects a file whose first chunk isn't IHDR", async () => {
  const bytes = ihdrPrefix(100, 100);
  bytes.set([0x49, 0x44, 0x41, 0x54], 12); // "IDAT"
  const path = await writeTemp("badchunk", bytes);
  await expect(pngSize(path)).rejects.toThrow(/IHDR/);
});

test("rejects a truncated file", async () => {
  const path = await writeTemp("short", ihdrPrefix(100, 100).slice(0, 10));
  await expect(pngSize(path)).rejects.toThrow(/too short/);
});
