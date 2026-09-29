//! One band of a page's X shape as the rectangle XShape takes. XRectangle
//! carries 16-bit fields, and a band comes from floating-point layout that can
//! run past them: a window resized far beyond the screen (xdotool, a tiling WM
//! mid-animation) put a span 100 000 px wide into `@intCast` and aborted the
//! host. X11 draws nothing outside 0..32767 anyway, so the band is clamped to
//! that range rather than wrapped or cast.
const std = @import("std");

pub const Rect = extern struct { x: c_short, y: c_short, width: c_ushort, height: c_ushort };

const coord_max: i64 = std.math.maxInt(c_short);

/// The band from x `lo` to `hi` and rows `top` to `bottom` (device pixels,
/// half-open), or null when nothing of it is left on the drawable range.
pub fn bandRect(lo: i32, hi: i32, top: usize, bottom: usize) ?Rect {
    const x0 = std.math.clamp(@as(i64, lo), 0, coord_max);
    const x1 = std.math.clamp(@as(i64, hi), 0, coord_max);
    const y0: i64 = @intCast(@min(top, coord_max));
    const y1: i64 = @intCast(@min(bottom, coord_max));
    if (x1 <= x0 or y1 <= y0) return null;
    return .{
        .x = @intCast(x0),
        .y = @intCast(y0),
        .width = @intCast(x1 - x0),
        .height = @intCast(y1 - y0),
    };
}

test "a band inside the drawable range is kept as it is" {
    const r = bandRect(10, 250, 4, 90).?;
    try std.testing.expectEqual(Rect{ .x = 10, .y = 4, .width = 240, .height = 86 }, r);
}

test "a band wider than X11 can address is clamped, not cast" {
    const r = bandRect(0, 100_320, 0, 800).?;
    try std.testing.expectEqual(@as(c_short, 0), r.x);
    try std.testing.expectEqual(@as(c_ushort, 32767), r.width);
    const far = bandRect(40_000, 90_000, 0, 10);
    try std.testing.expect(far == null);
}

test "a band left of the page starts at its edge" {
    const r = bandRect(-30, 20, 0, 1).?;
    try std.testing.expectEqual(Rect{ .x = 0, .y = 0, .width = 20, .height = 1 }, r);
}

test "rows past the drawable range are clamped and an empty band is dropped" {
    const r = bandRect(0, 10, 32_000, 70_000).?;
    try std.testing.expectEqual(@as(c_short, 32_000), r.y);
    try std.testing.expectEqual(@as(c_ushort, 767), r.height);
    try std.testing.expect(bandRect(5, 5, 0, 10) == null);
    try std.testing.expect(bandRect(0, 10, 40_000, 50_000) == null);
}
