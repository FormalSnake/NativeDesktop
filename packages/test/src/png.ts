// PNG dimension reader: parses the 8-byte signature + IHDR chunk (width/height,
// big-endian u32 at bytes 16/20) without pulling in an image-decoding
// dependency. Every drive script used to hand-roll this; this is the one copy.

export interface PngSize {
  width: number;
  height: number;
}

const SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

export async function pngSize(path: string): Promise<PngSize> {
  const bytes = new Uint8Array(await Bun.file(path).slice(0, 24).arrayBuffer());
  if (bytes.length < 24) throw new Error(`${path}: too short to be a PNG (${bytes.length} bytes)`);
  for (let i = 0; i < SIGNATURE.length; i++) {
    if (bytes[i] !== SIGNATURE[i]) throw new Error(`${path}: not a PNG (bad signature)`);
  }
  const chunkType = String.fromCharCode(bytes[12]!, bytes[13]!, bytes[14]!, bytes[15]!);
  if (chunkType !== "IHDR") throw new Error(`${path}: first chunk is "${chunkType}", want "IHDR"`);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  return { width: view.getUint32(16, false), height: view.getUint32(20, false) };
}
