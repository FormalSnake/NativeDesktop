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
