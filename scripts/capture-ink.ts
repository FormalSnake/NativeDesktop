// A minimal reader for the 8-bit non-interlaced PNGs the `screenshot` RPC
// writes on both backends, plus the ink primitives the capture-measuring gates
// share. getTree carries geometry per WIDGET and never per row or per piece of
// chrome, so a capture is the only channel that can answer what a widget
// actually painted and where.
import { inflateSync } from "node:zlib";

export interface Png {
  w: number;
  h: number;
  channels: number;
  data: Uint8Array;
}
export function decodePng(bytes: Uint8Array): Png {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let off = 8, w = 0, h = 0, depth = 0, color = 0, interlace = 0;
  const idat: Uint8Array[] = [];
  while (off + 8 <= bytes.length) {
    const len = view.getUint32(off);
    const type = String.fromCharCode(bytes[off + 4]!, bytes[off + 5]!, bytes[off + 6]!, bytes[off + 7]!);
    if (type === "IHDR") {
      w = view.getUint32(off + 8);
      h = view.getUint32(off + 12);
      depth = bytes[off + 16]!;
      color = bytes[off + 17]!;
      interlace = bytes[off + 20]!;
    } else if (type === "IDAT") {
      idat.push(bytes.subarray(off + 8, off + 8 + len));
    } else if (type === "IEND") break;
    off += 12 + len;
  }
  const channels = ({ 0: 1, 2: 3, 4: 2, 6: 4 } as Record<number, number>)[color];
  if (depth !== 8 || interlace !== 0 || !channels) {
    throw new Error(`unsupported PNG (depth=${depth} colorType=${color} interlace=${interlace})`);
  }
  const packed = new Uint8Array(idat.reduce((a, b) => a + b.length, 0));
  let at = 0;
  for (const chunk of idat) { packed.set(chunk, at); at += chunk.length; }
  const raw = inflateSync(packed);
  const stride = w * channels;
  const out = new Uint8Array(h * stride);
  let ri = 0;
  for (let y = 0; y < h; y++) {
    const filter = raw[ri++]!;
    const cur = out.subarray(y * stride, (y + 1) * stride);
    const prev = y > 0 ? out.subarray((y - 1) * stride, y * stride) : null;
    for (let i = 0; i < stride; i++) {
      const a = i >= channels ? cur[i - channels]! : 0;
      const b = prev ? prev[i]! : 0;
      const c = prev && i >= channels ? prev[i - channels]! : 0;
      let v = raw[ri + i]!;
      if (filter === 1) v += a;
      else if (filter === 2) v += b;
      else if (filter === 3) v += (a + b) >> 1;
      else if (filter === 4) {
        const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
        v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      } else if (filter !== 0) throw new Error(`bad PNG filter ${filter}`);
      cur[i] = v & 0xff;
    }
    ri += stride;
  }
  return { w, h, channels, data: out };
}

export function luminance(img: Png, x: number, y: number): number {
  const i = (y * img.w + x) * img.channels;
  if (img.channels <= 2) return img.data[i]!;
  return (img.data[i]! * 299 + img.data[i + 1]! * 587 + img.data[i + 2]! * 114) / 1000;
}

export type Rect = { x: number; y: number; w: number; h: number };

/// The modal luminance of `rect`, which every profile below reads as the fill
/// its ink contrasts with, so a profile reads the same in dark appearance and
/// under a hover highlight.
export function modalFill(img: Png, x0: number, x1: number, y0: number, y1: number): number {
  const hist = new Map<number, number>();
  for (let y = y0; y < y1; y++) {
    for (let x = x0; x < x1; x++) {
      const l = luminance(img, x, y) & ~3;
      hist.set(l, (hist.get(l) ?? 0) + 1);
    }
  }
  let fill = 0, best = -1;
  for (const [l, count] of hist) if (count > best) { best = count; fill = l; }
  return fill;
}

/// The tallest contiguous ink band inside `rect`, in logical units. `fill` is
/// the caller's reference background and `contrast` how far from it a pixel has
/// to sit to count. Both are the caller's because a narrow column can be
/// dominated by the very chrome being measured: take the modal luminance inside
/// a button's own column and the button's fill IS the fill, which reads as no
/// ink at all.
export function inkBandHeight(img: Png, rect: Rect, scale: number, fill: number, contrast: number): number {
  const x0 = Math.round(rect.x * scale), x1 = Math.min(img.w, Math.round((rect.x + rect.w) * scale));
  const y0 = Math.round(rect.y * scale), y1 = Math.min(img.h, Math.round((rect.y + rect.h) * scale));
  let tallest = 0, run = 0;
  for (let y = y0; y < y1; y++) {
    let ink = false;
    for (let x = x0; x < x1; x++) if (Math.abs(luminance(img, x, y) - fill) > contrast) { ink = true; break; }
    run = ink ? run + 1 : 0;
    if (run > tallest) tallest = run;
  }
  return tallest / scale;
}

