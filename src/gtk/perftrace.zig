//! ND_PERF_TRACE=1: UI-thread timings on stderr for latency attribution.
//! `ND_PERF job us=<n> at=<t>` for each job the core hands the UI thread (a
//! commit apply, mostly), and `ND_PERF frame layout_us=<n> paint_us=<n> at=<t>`
//! for every frame of each app window that laid out or painted (the rest are
//! counted in `ND_PERF empty_frames`). The hooks are connected after GTK's own
//! frame-clock handlers, so they run once GTK's work for that phase is done:
//! layout is the update phase (tick callbacks), style validation, size
//! allocation and the CEF views' bounds sync, paint the snapshot and render.
//! `at` is wall-clock microseconds, as on the Bun side's `ND_PERF` lines.
const std = @import("std");
const glib = @import("glib");
const gobject = @import("gobject");
const gtk = @import("gtk");
const gdk = @import("gdk");

var enabled: ?bool = null;

pub fn on() bool {
    if (enabled) |e| return e;
    const v = std.c.getenv("ND_PERF_TRACE");
    const e = if (v) |val| std.mem.eql(u8, std.mem.span(val), "1") else false;
    enabled = e;
    return e;
}

pub fn now() i64 {
    return glib.getMonotonicTime();
}

pub fn wall() i64 {
    return glib.getRealTime();
}

pub fn job(start: i64) void {
    const end = glib.getMonotonicTime();
    std.debug.print("ND_PERF job us={d} at={d}\n", .{ end - start, wall() });
}

const Phases = struct { begin: i64 = 0, layout: i64 = 0, paint: i64 = 0 };
const HOOK_KEY = "nd-perf-trace-clock";

/// Hooks the frame clock of every app window not hooked yet. Cheap enough to
/// call after each job: a window shows up once, then this is a data lookup.
pub fn hookWindows(app: *gtk.Application) void {
    var node: ?*glib.List = gtk.Application.getWindows(app);
    while (node) |n| : (node = n.f_next) {
        const w: *gtk.Widget = @ptrCast(@alignCast(n.f_data orelse continue));
        if (gobject.Object.getData(w.as(gobject.Object), HOOK_KEY) != null) continue;
        const clock = gtk.Widget.getFrameClock(w) orelse continue;
        const phases = std.heap.smp_allocator.create(Phases) catch continue;
        phases.* = .{};
        gobject.Object.setData(w.as(gobject.Object), HOOK_KEY, phases);
        _ = gdk.FrameClock.signals.before_paint.connect(clock, *Phases, &onBegin, phases, .{});
        _ = gdk.FrameClock.signals.layout.connect(clock, *Phases, &onLayout, phases, .{});
        _ = gdk.FrameClock.signals.paint.connect(clock, *Phases, &onPaint, phases, .{});
        _ = gdk.FrameClock.signals.after_paint.connect(clock, *Phases, &onAfter, phases, .{});
    }
}

fn onBegin(_: *gdk.FrameClock, p: *Phases) callconv(.c) void {
    p.begin = glib.getMonotonicTime();
}
fn onLayout(_: *gdk.FrameClock, p: *Phases) callconv(.c) void {
    p.layout = glib.getMonotonicTime();
}
fn onPaint(_: *gdk.FrameClock, p: *Phases) callconv(.c) void {
    p.paint = glib.getMonotonicTime();
}
/// Frames that ran no layout and no paint (a tick callback or an animation
/// clock asked for them) are only counted, one line per hundred.
var empty_frames: u32 = 0;

fn onAfter(_: *gdk.FrameClock, p: *Phases) callconv(.c) void {
    const end = glib.getMonotonicTime();
    if (p.paint < p.begin and p.layout < p.begin) {
        empty_frames += 1;
        if (empty_frames % 100 == 0) std.debug.print("ND_PERF empty_frames n={d} at={d}\n", .{ empty_frames, wall() });
        return;
    }
    const laid_out = p.layout >= p.begin;
    const layout = if (laid_out) p.layout - p.begin else 0;
    const paint = if (p.paint >= p.begin) p.paint - (if (laid_out) p.layout else p.begin) else 0;
    std.debug.print("ND_PERF frame layout_us={d} paint_us={d} total_us={d} at={d}\n", .{ layout, paint, end - p.begin, wall() });
}
