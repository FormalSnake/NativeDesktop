// Where a picture-in-picture window comes to rest after it is moved, on a
// window manager that lets a client place its own windows.
//
// The window is Chromium's and so is the drag: Chromium hands it to the window
// manager (_NET_WM_MOVERESIZE) from its frame or from the video. What the host
// adds is the landing the system's own floating video has. While a pointer
// button is down and the window moves, its position is sampled each frame;
// on release the momentum is projected forward and the window springs, at
// the release velocity, into the screen corner the projection lands nearest,
// or tucks past the left or right edge when the push carries its centre over
// that edge. A drag that starts during a flight takes the window from where it
// is.
//
// Nothing here fights the window manager. Under Hyprland the compositor owns
// placement and this does nothing at all; on X11 the moves are configure
// requests, and a manager that ignores them (one that tiles the window) is
// left alone for the rest of the window's life.
const std = @import("std");
const glib = @import("glib");
const gdk = @import("gdk");
const gio = @import("gio");
const x11 = @import("x11.zig");
const hyprland = @import("hyprland.zig");

const alloc = std.heap.c_allocator;

const margin: f64 = 16;
const tab: f64 = 28;
const tick_ms: c_uint = 8;

const Spring = struct {
    value: f64,
    velocity: f64,
    target: f64,
    response: f64 = 0.4,
    damping: f64 = 1,

    fn advance(self: *Spring, dt: f64) void {
        const stiffness = std.math.pow(f64, 2 * std.math.pi / self.response, 2);
        const friction = 4 * std.math.pi * self.damping / self.response;
        var left = dt;
        while (left > 0) {
            const step = @min(left, 1.0 / 480.0);
            const force = -stiffness * (self.value - self.target) - friction * self.velocity;
            self.velocity += force * step;
            self.value += self.velocity * step;
            left -= step;
        }
    }

    fn settled(self: Spring) bool {
        return @abs(self.value - self.target) < 0.5 and @abs(self.velocity) < 4;
    }
};

const Sample = struct { t: f64, x: f64, y: f64 };

const Window = struct {
    id: x11.Window,
    last: ?Sample = null,
    samples: [16]Sample = undefined,
    count: usize = 0,
    moving: bool = false,
    flight: ?struct { x: Spring, y: Spring, asked: [2]c_int, stuck: u8 = 0 } = null,
    /// Cleared the first time the window manager ignores a move.
    placeable: bool = true,
    /// Whether it may tuck past an edge. A document window may not: only
    /// Chromium's title strip drags it, and tucked left the part left showing
    /// is that strip's buttons, with nothing to take hold of.
    stashable: bool,
    /// Frame pacing of the current flight, for the trace.
    frames: u32 = 0,
    late: u32 = 0,
    last_tick: f64 = 0,
};

var windows: std.AutoHashMapUnmanaged(usize, Window) = .empty;
var timer: c_uint = 0;

fn trace() bool {
    const v = std.c.getenv("ND_PIP_TRACE") orelse return false;
    return std.mem.eql(u8, std.mem.span(v), "1");
}

fn now() f64 {
    return @as(f64, @floatFromInt(glib.getMonotonicTime())) / std.time.us_per_s;
}

/// Starts landing `window` after each move. A no-op under Hyprland, which
/// places XWayland windows itself.
pub fn track(window: x11.Window, stashable: bool) void {
    if (hyprland.running() or windows.contains(window)) return;
    windows.put(alloc, window, .{ .id = window, .stashable = stashable }) catch return;
    if (timer == 0) timer = glib.timeoutAdd(tick_ms, &onTick, null);
}

pub fn forget(window: x11.Window) void {
    _ = windows.remove(window);
}

fn onTick(_: ?*anyopaque) callconv(.c) c_int {
    if (windows.count() == 0) {
        timer = 0;
        return 0;
    }
    const buttons = x11.pointerButtonsDown();
    var it = windows.valueIterator();
    while (it.next()) |w| stepWindow(w, buttons);
    return 1;
}

fn stepWindow(w: *Window, buttons: bool) void {
    if (!w.placeable) return;
    const geo = x11.geometry(w.id) orelse return;
    const origin = x11.originOnRoot(w.id);
    const t = now();
    const here: Sample = .{ .t = t, .x = @floatFromInt(origin.x), .y = @floatFromInt(origin.y) };
    defer w.last = here;

    if (w.flight) |*f| {
        const off = @abs(here.x - @as(f64, @floatFromInt(f.asked[0]))) + @abs(here.y - @as(f64, @floatFromInt(f.asked[1])));
        const still = if (w.last) |l| l.x == here.x and l.y == here.y else false;
        if (buttons and off > 4) {
            // A move that did not come from this flight: the user took it.
            w.flight = null;
            report(w, "throw");
        } else {
            // Three frames standing still while asked to be elsewhere: the
            // manager does not take client moves for this window. One that
            // only keeps part of it on screen (openbox at an edge) still moves,
            // and is followed as far as it allows.
            if (off > 4 and still) {
                f.stuck += 1;
                if (f.stuck >= 3 and w.frames < 6) {
                    w.placeable = false;
                    w.flight = null;
                    if (trace()) std.debug.print("ND_PIP the window manager keeps window={x} where it is; no landing\n", .{w.id});
                    return;
                }
                if (f.stuck >= 3) {
                    w.flight = null;
                    report(w, "throw");
                    return;
                }
            } else f.stuck = 0;
            const dt = if (w.last_tick > 0) @min(t - w.last_tick, 1.0 / 30.0) else @as(f64, @floatFromInt(tick_ms)) / 1000;
            if (w.last_tick > 0 and t - w.last_tick > 1.5 / 60.0) w.late += 1;
            w.last_tick = t;
            w.frames += 1;
            f.x.advance(dt);
            f.y.advance(dt);
            f.asked = .{ @intFromFloat(@round(f.x.value)), @intFromFloat(@round(f.y.value)) };
            x11.moveWindow(w.id, f.asked[0], f.asked[1]);
            if (f.x.settled() and f.y.settled()) {
                x11.moveWindow(w.id, @intFromFloat(@round(f.x.target)), @intFromFloat(@round(f.y.target)));
                w.flight = null;
                report(w, "throw");
            }
            return;
        }
    }

    const moved = if (w.last) |l| (l.x != here.x or l.y != here.y) else false;
    if (moved and buttons) {
        if (!w.moving) {
            w.moving = true;
            w.count = 0;
        }
        w.samples[w.count % w.samples.len] = here;
        w.count += 1;
        return;
    }
    if (w.moving and !buttons) {
        w.moving = false;
        land(w, here, geo);
    }
}

fn land(w: *Window, at: Sample, geo: x11.Geometry) void {
    // Release velocity from the last ~80 ms of positions.
    var vx: f64 = 0;
    var vy: f64 = 0;
    const n = @min(w.count, w.samples.len);
    if (n >= 2) {
        const newest = w.samples[(w.count - 1) % w.samples.len];
        var oldest = newest;
        var i: usize = 1;
        while (i < n) : (i += 1) {
            const s = w.samples[(w.count - 1 - i) % w.samples.len];
            if (newest.t - s.t > 0.08) break;
            oldest = s;
        }
        const span = newest.t - oldest.t;
        if (span > 0.008) {
            vx = (newest.x - oldest.x) / span;
            vy = (newest.y - oldest.y) / span;
        }
    }
    const width: f64 = @floatFromInt(geo.w);
    const height: f64 = @floatFromInt(geo.h);
    const area = workAreaFor(at.x + width / 2, at.y + height / 2);
    const cx = at.x + width / 2;
    const cy = at.y + height / 2;
    const projected_x = cx + project(vx, 0.998);
    const projected_y = cy + project(vy, 0.998);
    const pushed_x = cx + project(vx, 0.99);
    const ax: f64 = @floatFromInt(area.x);
    const ay: f64 = @floatFromInt(area.y);
    const aw: f64 = @floatFromInt(area.w);
    const ah: f64 = @floatFromInt(area.h);
    var tx: f64 = undefined;
    var ty: f64 = undefined;
    var rest: []const u8 = undefined;
    if (w.stashable and (pushed_x < ax or pushed_x > ax + aw)) {
        const right = pushed_x > ax + aw;
        tx = if (right) ax + aw - tab else ax - width + tab;
        ty = std.math.clamp(at.y, ay + margin, ay + ah - height - margin);
        rest = if (right) "stashed right" else "stashed left";
    } else {
        const right = projected_x > ax + aw / 2;
        const bottom = projected_y > ay + ah / 2;
        tx = if (right) ax + aw - width - margin else ax + margin;
        ty = if (bottom) ay + ah - height - margin else ay + margin;
        rest = if (bottom) (if (right) "corner bottom right" else "corner bottom left") else (if (right) "corner top right" else "corner top left");
    }
    if (trace()) std.debug.print("ND_PIP settle throw v={d:.0},{d:.0} to={s} at={d:.0},{d:.0}\n", .{ vx, vy, rest, tx, ty });
    w.frames = 0;
    w.late = 0;
    w.last_tick = 0;
    w.flight = .{
        .x = .{ .value = at.x, .velocity = vx, .target = tx },
        .y = .{ .value = at.y, .velocity = vy, .target = ty },
        .asked = .{ @intFromFloat(at.x), @intFromFloat(at.y) },
    };
}

fn report(w: *Window, phase: []const u8) void {
    if (!trace()) return;
    std.debug.print("ND_PIP frames phase={s} frames={d} late={d} window={x}\n", .{ phase, w.frames, w.late, w.id });
}

/// Apple's momentum projection (Designing Fluid Interfaces).
fn project(velocity: f64, deceleration: f64) f64 {
    return (velocity / 1000) * deceleration / (1 - deceleration);
}

/// The monitor under the point, cut down to the manager's work area where it
/// publishes one.
fn workAreaFor(x: f64, y: f64) x11.Area {
    var best: x11.Area = .{ .x = 0, .y = 0, .w = 1920, .h = 1080 };
    if (gdk.Display.getDefault()) |display| {
        const monitors = gdk.Display.getMonitors(display);
        var i: c_uint = 0;
        while (i < gio.ListModel.getNItems(monitors)) : (i += 1) {
            const item = gio.ListModel.getItem(monitors, i) orelse continue;
            var r: gdk.Rectangle = .{ .f_x = 0, .f_y = 0, .f_width = 0, .f_height = 0 };
            gdk.Monitor.getGeometry(@ptrCast(@alignCast(item)), &r);
            const inside = x >= @as(f64, @floatFromInt(r.f_x)) and x < @as(f64, @floatFromInt(r.f_x + r.f_width)) and
                y >= @as(f64, @floatFromInt(r.f_y)) and y < @as(f64, @floatFromInt(r.f_y + r.f_height));
            if (i == 0 or inside) best = .{ .x = r.f_x, .y = r.f_y, .w = r.f_width, .h = r.f_height };
            if (inside) break;
        }
    }
    if (x11.workArea()) |wa| {
        const x0 = @max(best.x, wa.x);
        const y0 = @max(best.y, wa.y);
        const x1 = @min(best.x + best.w, wa.x + wa.w);
        const y1 = @min(best.y + best.h, wa.y + wa.h);
        if (x1 > x0 and y1 > y0) return .{ .x = x0, .y = y0, .w = x1 - x0, .h = y1 - y0 };
    }
    return best;
}
