// ND_LAT_TRACE=1: the child's half of the host's `ND_LAT` lines (marker.zig),
// on the host's clock (CLOCK_MONOTONIC, mach_absolute_time on macOS), so one log
// orders an input's hops across both processes. Bun's own hrtime counts from
// process start, so the clock is read through libc.
import { FFIType, dlopen, ptr } from "bun:ffi";

const on = process.env.ND_LAT_TRACE != null;
const mac = process.platform === "darwin";
const clock = on
  ? dlopen(mac ? "/usr/lib/libSystem.B.dylib" : "libc.so.6", {
      clock_gettime: { args: [FFIType.i32, FFIType.ptr], returns: FFIType.i32 },
    })
  : null;
const ts = new BigInt64Array(2);

export function lat(tag: string, detail = ""): void {
  if (!clock) return;
  // CLOCK_UPTIME_RAW on macOS, CLOCK_MONOTONIC on Linux.
  clock.symbols.clock_gettime(mac ? 8 : 1, ptr(ts));
  const us = ts[0]! * 1000000n + ts[1]! / 1000n;
  process.stderr.write(`ND_LAT ${us} ${tag} ${detail}\n`);
}
