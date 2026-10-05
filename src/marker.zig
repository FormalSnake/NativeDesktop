//! `ND_*` diagnostic markers: one line on stderr per host-visible event, the
//! shape the shell gates in `scripts/` grep for.
//!
//! Silent under `zig build test`. Zig 0.16's build runner dumps a run step's
//! captured stderr, plus a `failed command:` line naming the test binary,
//! whenever the step wrote anything at all — pass or fail. A marker printed
//! from a unit test therefore reads as six failing steps in a build that
//! exited 0.
const std = @import("std");
const builtin = @import("builtin");

pub fn print(comptime fmt: []const u8, args: anytype) void {
    if (builtin.is_test) return;
    std.debug.print(fmt, args);
}

var lat_on: ?bool = null;

/// ND_LAT_TRACE=1: one `ND_LAT <µs> <tag> ...` line per hop an input takes on
/// its way to pixels (native event, child, commit, apply, engine). The clock is
/// the one Bun's `process.hrtime` reads (CLOCK_MONOTONIC on Linux,
/// mach_absolute_time on macOS), so the child's lines and a drive's own
/// timestamps line up with these.
pub fn lat(comptime tag: []const u8, comptime fmt: []const u8, args: anytype) void {
    if (!latOn()) return;
    std.debug.print("ND_LAT {d} " ++ tag ++ " " ++ fmt ++ "\n", .{nowMicros()} ++ args);
}

pub fn latOn() bool {
    if (builtin.is_test) return false;
    return lat_on orelse blk: {
        const on = std.c.getenv("ND_LAT_TRACE") != null;
        lat_on = on;
        break :blk on;
    };
}

/// `lat` for a step that took at least `min_us` since `t0` (a `nowMicros`).
pub fn latSlow(comptime tag: []const u8, t0: i64, min_us: i64, comptime fmt: []const u8, args: anytype) void {
    if (t0 == 0) return;
    const took = nowMicros() - t0;
    if (took < min_us) return;
    std.debug.print("ND_LAT {d} " ++ tag ++ " us={d} " ++ fmt ++ "\n", .{ nowMicros(), took } ++ args);
}

pub fn nowMicros() i64 {
    var ts: std.c.timespec = undefined;
    const clock: std.c.clockid_t = if (builtin.os.tag.isDarwin()) .UPTIME_RAW else .MONOTONIC;
    _ = std.c.clock_gettime(clock, &ts);
    return @as(i64, ts.sec) * std.time.us_per_s + @divTrunc(@as(i64, ts.nsec), std.time.ns_per_us);
}
