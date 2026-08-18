// Cairo-drawn surface for the <chart> widget. GTK ships no charting library,
// so GNOME apps draw their charts into a GtkDrawingArea; this is that, not a
// workaround for a missing widget.
//
// Data model: the whole `series` objectList is parsed into a module-owned
// Store per drawing area and redrawn from there. The app owns the data — the
// widget never sorts, filters or rewrites points (Table's rows contract).
//
// Theme: every colour is read at draw time (gtk_widget_get_color for text and
// axes, AdwStyleManager's accent for the first series), and one "notify"
// handler on the shared AdwStyleManager queues a redraw on every live chart,
// so an accent or light/dark switch repaints without an app re-render.
//
// Motion: `animated` grows the geometry from its baseline over ANIM_MS on
// every data change. gtk-enable-animations off outranks the prop, the rule
// <skeleton> set (AppKit peer: accessibilityDisplayShouldReduceMotion).
const std = @import("std");
const gtk = @import("gtk");
const gdk = @import("gdk");
const gobject = @import("gobject");
const adw = @import("adw");
const cairo = @import("cairo");
const pango = @import("pango");
const pangocairo = @import("pangocairo");
const protocol = @import("../protocol.zig");
const ndmotion = @import("motion.zig");

pub const EmitFn = *const fn (node_id: u32, name: []const u8, payload: protocol.EventPayload) void;

const alloc = std.heap.page_allocator;

var emit: ?EmitFn = null;
/// GtkDrawingArea ptr -> its parsed store (owned here; freed on destroy).
var stores: std.AutoHashMapUnmanaged(usize, *Store) = .empty;
/// One "notify" handler on the process-wide AdwStyleManager covers every
/// chart, so accent/dark changes repaint without a per-widget subscription.
var style_watch_connected = false;

const ANIM_MS: f64 = 420.0;
/// Click slop for the nearest-point search, in device pixels.
const HIT_RADIUS: f64 = 24.0;

const Kind = enum { line, area, bar, pie, scatter, candlestick };

fn parseKind(name: []const u8) Kind {
    if (std.mem.eql(u8, name, "area")) return .area;
    if (std.mem.eql(u8, name, "bar")) return .bar;
    if (std.mem.eql(u8, name, "pie")) return .pie;
    if (std.mem.eql(u8, name, "scatter")) return .scatter;
    if (std.mem.eql(u8, name, "candlestick")) return .candlestick;
    return .line;
}

const Point = struct {
    x: f64 = 0,
    y: f64 = 0,
    label: ?[:0]u8 = null,
    open: f64 = 0,
    high: f64 = 0,
    low: f64 = 0,
    close: f64 = 0,
    has_ohlc: bool = false,
};

const Series = struct {
    id: []u8,
    label: [:0]u8,
    color: ?gdk.RGBA = null,
    points: std.ArrayList(Point) = .empty,

    fn deinit(self: *Series) void {
        alloc.free(self.id);
        alloc.free(self.label);
        for (self.points.items) |*p| {
            if (p.label) |l| alloc.free(l);
        }
        self.points.deinit(alloc);
    }
};

const Rect = struct { x: f64 = 0, y: f64 = 0, w: f64 = 0, h: f64 = 0 };

const Store = struct {
    kind: Kind = .line,
    series: std.ArrayList(Series) = .empty,
    x_label: ?[:0]u8 = null,
    y_label: ?[:0]u8 = null,
    show_legend: bool = true,
    show_grid: bool = true,
    animated: bool = true,
    node_id: u32 = 0,
    tick: ?c_uint = null,
    started_us: i64 = 0,
    progress: f64 = 1.0,
    /// Last drawn plot rect + domain, so a click maps back to a datum without
    /// re-deriving the layout.
    plot: Rect = .{},
    x_min: f64 = 0,
    x_max: f64 = 1,
    y_min: f64 = 0,
    y_max: f64 = 1,

    fn deinitSeries(self: *Store) void {
        for (self.series.items) |*s| s.deinit();
        self.series.clearRetainingCapacity();
    }

    fn deinit(self: *Store) void {
        self.deinitSeries();
        self.series.deinit(alloc);
        if (self.x_label) |l| alloc.free(l);
        if (self.y_label) |l| alloc.free(l);
    }
};

// ---- props ----------------------------------------------------------------

fn propStr(props: ?std.json.Value, key: []const u8) ?[]const u8 {
    const v = props orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .string => field.string,
        else => null,
    };
}

fn propBool(props: ?std.json.Value, key: []const u8) ?bool {
    const v = props orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .bool => field.bool,
        else => null,
    };
}

fn propArray(props: ?std.json.Value, key: []const u8) ?std.json.Array {
    const v = props orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .array => field.array,
        else => null,
    };
}

fn objStr(obj: std.json.ObjectMap, key: []const u8) ?[]const u8 {
    const field = obj.get(key) orelse return null;
    return switch (field) {
        .string => field.string,
        else => null,
    };
}

fn objNum(obj: std.json.ObjectMap, key: []const u8) ?f64 {
    const field = obj.get(key) orelse return null;
    return switch (field) {
        .float => field.float,
        .integer => @floatFromInt(field.integer),
        else => null,
    };
}

fn asObject(ptr: anytype) *gobject.Object {
    return @ptrCast(@alignCast(ptr));
}

fn storeOf(widget: *gtk.Widget) ?*Store {
    return stores.get(@intFromPtr(widget));
}

// ---- colours --------------------------------------------------------------

/// GNOME's own palette, minus blue: slot 0 is the live system accent, so a
/// one-series chart is accent-coloured and a many-series chart never repeats
/// the accent hue next to itself.
const PALETTE = [_][3]f64{
    .{ 0.902, 0.380, 0.000 }, // orange 4
    .{ 0.200, 0.820, 0.478 }, // green 3
    .{ 0.569, 0.255, 0.675 }, // purple 3
    .{ 0.898, 0.647, 0.039 }, // yellow 5
    .{ 0.753, 0.110, 0.157 }, // red 4
    .{ 0.596, 0.416, 0.267 }, // brown 3
};

fn accentRgba(out: *gdk.RGBA) void {
    const rgba = adw.StyleManager.getAccentColorRgba(adw.StyleManager.getDefault());
    out.* = rgba.*;
    gdk.RGBA.free(rgba);
}

fn paletteAt(index: usize) gdk.RGBA {
    if (index == 0) {
        var accent: gdk.RGBA = undefined;
        accentRgba(&accent);
        return accent;
    }
    const slot = PALETTE[(index - 1) % PALETTE.len];
    return .{ .f_red = @floatCast(slot[0]), .f_green = @floatCast(slot[1]), .f_blue = @floatCast(slot[2]), .f_alpha = 1.0 };
}

fn seriesColor(s: Series, index: usize) gdk.RGBA {
    return s.color orelse paletteAt(index);
}

fn setSource(cr: *cairo.Context, c: gdk.RGBA, alpha_scale: f64) void {
    cairo.Context.setSourceRgba(cr, c.f_red, c.f_green, c.f_blue, c.f_alpha * alpha_scale);
}

// ---- parsing --------------------------------------------------------------

fn parseSeries(store: *Store, arr: std.json.Array) void {
    store.deinitSeries();
    for (arr.items) |item| {
        if (item != .object) continue;
        const obj = item.object;
        const id = alloc.dupe(u8, objStr(obj, "id") orelse "") catch continue;
        const label = alloc.dupeZ(u8, objStr(obj, "label") orelse "") catch {
            alloc.free(id);
            continue;
        };
        var s = Series{ .id = id, .label = label };
        if (objStr(obj, "color")) |spec| {
            var rgba: gdk.RGBA = undefined;
            const z = alloc.dupeZ(u8, spec) catch null;
            if (z) |zs| {
                defer alloc.free(zs);
                if (gdk.RGBA.parse(&rgba, zs.ptr) != 0) s.color = rgba;
            }
        }
        if (obj.get("points")) |raw| {
            if (raw == .array) {
                for (raw.array.items) |pit| {
                    if (pit != .object) continue;
                    const po = pit.object;
                    var p = Point{
                        .x = objNum(po, "x") orelse @floatFromInt(s.points.items.len),
                        .y = objNum(po, "y") orelse 0,
                    };
                    if (objStr(po, "label")) |l| p.label = alloc.dupeZ(u8, l) catch null;
                    const open = objNum(po, "open");
                    const close = objNum(po, "close");
                    if (open != null and close != null) {
                        p.open = open.?;
                        p.close = close.?;
                        p.high = objNum(po, "high") orelse @max(p.open, p.close);
                        p.low = objNum(po, "low") orelse @min(p.open, p.close);
                        p.has_ohlc = true;
                    }
                    s.points.append(alloc, p) catch {
                        if (p.label) |l| alloc.free(l);
                    };
                }
            }
        }
        store.series.append(alloc, s) catch s.deinit();
    }
}

// ---- animation ------------------------------------------------------------

fn restartAnimation(widget: *gtk.Widget, store: *Store) void {
    if (!store.animated or !ndmotion.animationsEnabled()) {
        store.progress = 1.0;
        if (store.tick) |id| {
            gtk.Widget.removeTickCallback(widget, id);
            store.tick = null;
        }
        gtk.Widget.queueDraw(widget);
        return;
    }
    store.progress = 0.0;
    store.started_us = 0;
    if (store.tick == null) store.tick = gtk.Widget.addTickCallback(widget, &cbTick, null, null);
    gtk.Widget.queueDraw(widget);
}

fn cbTick(widget: *gtk.Widget, clock: *gdk.FrameClock, _: ?*anyopaque) callconv(.c) c_int {
    const store = storeOf(widget) orelse return 0;
    const now = gdk.FrameClock.getFrameTime(clock);
    if (store.started_us == 0) store.started_us = now;
    const elapsed_ms = @as(f64, @floatFromInt(now - store.started_us)) / 1000.0;
    store.progress = @min(elapsed_ms / ANIM_MS, 1.0);
    gtk.Widget.queueDraw(widget);
    if (store.progress >= 1.0) {
        store.tick = null;
        return 0; // G_SOURCE_REMOVE
    }
    return 1; // G_SOURCE_CONTINUE
}

/// Eased 0..1, so a growing bar decelerates into place instead of stopping dead.
fn eased(p: f64) f64 {
    const inv = 1.0 - p;
    return 1.0 - inv * inv * inv;
}

// ---- text -----------------------------------------------------------------

/// A layout at `scale` of the widget's own font size, so tick labels stay
/// smaller than the legend without naming a font family.
fn makeLayout(widget: *gtk.Widget, text: [:0]const u8, scale: f64) *pango.Layout {
    const layout = gtk.Widget.createPangoLayout(widget, text.ptr);
    if (scale != 1.0) {
        const ctx = pango.Layout.getContext(layout);
        if (pango.Context.getFontDescription(ctx)) |base| {
            const desc = pango.FontDescription.copy(base) orelse return layout;
            defer pango.FontDescription.free(desc);
            const size = pango.FontDescription.getSize(desc);
            if (size > 0) pango.FontDescription.setSize(desc, @intFromFloat(@as(f64, @floatFromInt(size)) * scale));
            pango.Layout.setFontDescription(layout, desc);
        }
    }
    return layout;
}

fn layoutSize(layout: *pango.Layout, w: *f64, h: *f64) void {
    var pw: c_int = 0;
    var ph: c_int = 0;
    pango.Layout.getPixelSize(layout, &pw, &ph);
    w.* = @floatFromInt(pw);
    h.* = @floatFromInt(ph);
}

fn drawText(widget: *gtk.Widget, cr: *cairo.Context, text: [:0]const u8, x: f64, y: f64, scale: f64) void {
    const layout = makeLayout(widget, text, scale);
    defer gobject.Object.unref(asObject(layout));
    cairo.Context.moveTo(cr, x, y);
    pangocairo.showLayout(cr, layout);
}

/// Right-aligned around (x, y-centre) — the shape every y tick label wants.
fn drawTextRight(widget: *gtk.Widget, cr: *cairo.Context, text: [:0]const u8, x: f64, cy: f64, scale: f64) void {
    const layout = makeLayout(widget, text, scale);
    defer gobject.Object.unref(asObject(layout));
    var tw: f64 = 0;
    var th: f64 = 0;
    layoutSize(layout, &tw, &th);
    cairo.Context.moveTo(cr, x - tw, cy - th / 2.0);
    pangocairo.showLayout(cr, layout);
}

fn drawTextCentered(widget: *gtk.Widget, cr: *cairo.Context, text: [:0]const u8, cx: f64, y: f64, scale: f64) void {
    const layout = makeLayout(widget, text, scale);
    defer gobject.Object.unref(asObject(layout));
    var tw: f64 = 0;
    var th: f64 = 0;
    layoutSize(layout, &tw, &th);
    cairo.Context.moveTo(cr, cx - tw / 2.0, y);
    pangocairo.showLayout(cr, layout);
}

/// Axis ticks read as numbers, so they drop a trailing ".0" and keep at most
/// two decimals — 1000 and 0.25 both come out right.
fn formatTick(buf: []u8, value: f64) [:0]const u8 {
    const rounded = @round(value);
    if (@abs(value - rounded) < 0.005 and @abs(value) < 1e15) {
        return std.fmt.bufPrintZ(buf, "{d}", .{@as(i64, @intFromFloat(rounded))}) catch "";
    }
    return std.fmt.bufPrintZ(buf, "{d:.2}", .{value}) catch "";
}

// ---- geometry -------------------------------------------------------------

const Domain = struct { x_min: f64, x_max: f64, y_min: f64, y_max: f64 };

fn computeDomain(store: *Store) Domain {
    var d = Domain{ .x_min = 0, .x_max = 1, .y_min = 0, .y_max = 1 };
    var seen = false;
    for (store.series.items) |s| {
        for (s.points.items) |p| {
            const lo = if (p.has_ohlc) p.low else p.y;
            const hi = if (p.has_ohlc) p.high else p.y;
            if (!seen) {
                d = .{ .x_min = p.x, .x_max = p.x, .y_min = lo, .y_max = hi };
                seen = true;
                continue;
            }
            d.x_min = @min(d.x_min, p.x);
            d.x_max = @max(d.x_max, p.x);
            d.y_min = @min(d.y_min, lo);
            d.y_max = @max(d.y_max, hi);
        }
    }
    if (!seen) return d;
    // Bars and areas are read against zero, so the baseline has to be in frame.
    if (store.kind == .bar or store.kind == .area) {
        d.y_min = @min(d.y_min, 0);
        d.y_max = @max(d.y_max, 0);
    }
    if (d.x_max - d.x_min < 1e-9) {
        d.x_min -= 0.5;
        d.x_max += 0.5;
    }
    if (d.y_max - d.y_min < 1e-9) {
        d.y_min -= 0.5;
        d.y_max += 0.5;
    }
    // A tenth of headroom keeps the topmost point off the plot edge.
    const pad = (d.y_max - d.y_min) * 0.1;
    d.y_max += pad;
    if (d.y_min < 0) d.y_min -= pad;
    return d;
}

fn xToPx(store: *Store, x: f64) f64 {
    const span = store.x_max - store.x_min;
    if (span <= 0) return store.plot.x + store.plot.w / 2.0;
    return store.plot.x + (x - store.x_min) / span * store.plot.w;
}

fn yToPx(store: *Store, y: f64) f64 {
    const span = store.y_max - store.y_min;
    if (span <= 0) return store.plot.y + store.plot.h / 2.0;
    return store.plot.y + store.plot.h - (y - store.y_min) / span * store.plot.h;
}

// ---- draw -----------------------------------------------------------------

fn draw(area: *gtk.DrawingArea, cr: *cairo.Context, width: c_int, height: c_int, _: ?*anyopaque) callconv(.c) void {
    const widget = area.as(gtk.Widget);
    const store = storeOf(widget) orelse return;
    const w: f64 = @floatFromInt(width);
    const h: f64 = @floatFromInt(height);
    if (w <= 4 or h <= 4) return;

    var fg: gdk.RGBA = undefined;
    gtk.Widget.getColor(widget, &fg);

    const pad: f64 = 8.0;
    var top = pad;
    var bottom = h - pad;
    var left = pad;
    const right = w - pad;

    if (store.show_legend and store.series.items.len > 0) {
        top += drawLegend(widget, cr, store, fg, left, top, right - left);
    }
    if (store.x_label) |label| {
        if (label.len > 0) {
            const layout = makeLayout(widget, label, 0.9);
            var tw: f64 = 0;
            var th: f64 = 0;
            layoutSize(layout, &tw, &th);
            gobject.Object.unref(asObject(layout));
            bottom -= th + 2.0;
            setSource(cr, fg, 0.7);
            drawTextCentered(widget, cr, label, (left + right) / 2.0, bottom + 2.0, 0.9);
        }
    }
    if (store.y_label) |label| {
        if (label.len > 0) {
            const layout = makeLayout(widget, label, 0.9);
            var tw: f64 = 0;
            var th: f64 = 0;
            layoutSize(layout, &tw, &th);
            gobject.Object.unref(asObject(layout));
            setSource(cr, fg, 0.7);
            cairo.Context.save(cr);
            cairo.Context.translate(cr, left, (top + bottom) / 2.0 + tw / 2.0);
            cairo.Context.rotate(cr, -std.math.pi / 2.0);
            drawText(widget, cr, label, 0, 0, 0.9);
            cairo.Context.restore(cr);
            left += th + 4.0;
        }
    }

    if (store.kind == .pie) {
        drawPie(widget, cr, store, fg, .{ .x = left, .y = top, .w = right - left, .h = bottom - top });
        return;
    }

    // Tick gutters: measured from the widest y label so long numbers never
    // overrun the plot.
    const dom = computeDomain(store);
    store.x_min = dom.x_min;
    store.x_max = dom.x_max;
    store.y_min = dom.y_min;
    store.y_max = dom.y_max;

    const ticks: usize = 5;
    var gutter: f64 = 0;
    {
        var i: usize = 0;
        while (i < ticks) : (i += 1) {
            var buf: [32]u8 = undefined;
            const value = dom.y_min + (dom.y_max - dom.y_min) * (@as(f64, @floatFromInt(i)) / @as(f64, @floatFromInt(ticks - 1)));
            const layout = makeLayout(widget, formatTick(&buf, value), 0.8);
            var tw: f64 = 0;
            var th: f64 = 0;
            layoutSize(layout, &tw, &th);
            gobject.Object.unref(asObject(layout));
            gutter = @max(gutter, tw);
        }
    }
    var tick_h: f64 = 0;
    {
        var buf: [32]u8 = undefined;
        const layout = makeLayout(widget, formatTick(&buf, dom.x_max), 0.8);
        var tw: f64 = 0;
        layoutSize(layout, &tw, &tick_h);
        gobject.Object.unref(asObject(layout));
    }

    // `drawBars` centers each bar exactly on its x value, so the first bar
    // (at x_min, which maps to plot.x) has its own left half sitting to the
    // LEFT of the plot origin — straight into the tick-label gutter above.
    // Reserve that overlap too, sized from the same slot/group math
    // `drawBars` uses (against the width the label gutter alone would have
    // left), not a flat constant that would be wrong for a different point
    // count or series count.
    if (store.kind == .bar and store.series.items.len > 0) {
        const n: f64 = @floatFromInt(@max(store.series.items[0].points.items.len, 1));
        const group: f64 = @max(@as(f64, @floatFromInt(store.series.items.len)), 1.0);
        const approx_plot_w = @max(right - (left + gutter + 6.0), 1.0);
        const slot = approx_plot_w / n;
        const bar_w = @max((slot * 0.7) / group, 1.0);
        gutter += bar_w / 2.0;
    }

    store.plot = .{
        .x = left + gutter + 6.0,
        .y = top,
        .w = @max(right - (left + gutter + 6.0), 1.0),
        .h = @max(bottom - tick_h - 4.0 - top, 1.0),
    };

    drawAxes(widget, cr, store, fg, ticks);

    const p = if (store.progress >= 1.0) 1.0 else eased(store.progress);
    for (store.series.items, 0..) |s, si| {
        const color = seriesColor(s, si);
        switch (store.kind) {
            .line => drawLine(cr, store, s, color, p, false),
            .area => drawLine(cr, store, s, color, p, true),
            .scatter => drawScatter(cr, store, s, color, p),
            .bar => drawBars(cr, store, s, color, p, si),
            .candlestick => drawCandles(cr, store, s, p),
            .pie => unreachable, // handled above
        }
    }
}

/// Returns the height it consumed, so the plot starts below it.
fn drawLegend(widget: *gtk.Widget, cr: *cairo.Context, store: *Store, fg: gdk.RGBA, x: f64, y: f64, avail: f64) f64 {
    const swatch: f64 = 9.0;
    var cursor_x = x;
    var cursor_y = y;
    var row_h: f64 = 0;
    for (store.series.items, 0..) |s, i| {
        const layout = makeLayout(widget, s.label, 0.9);
        defer gobject.Object.unref(asObject(layout));
        var tw: f64 = 0;
        var th: f64 = 0;
        layoutSize(layout, &tw, &th);
        const entry_w = swatch + 5.0 + tw + 14.0;
        if (cursor_x > x and cursor_x + entry_w > x + avail) {
            cursor_x = x;
            cursor_y += th + 4.0;
        }
        row_h = th;
        setSource(cr, seriesColor(s, i), 1.0);
        cairo.Context.newPath(cr);
        cairo.Context.arc(cr, cursor_x + swatch / 2.0, cursor_y + th / 2.0, swatch / 2.0, 0.0, std.math.tau);
        cairo.Context.fill(cr);
        setSource(cr, fg, 0.85);
        cairo.Context.moveTo(cr, cursor_x + swatch + 5.0, cursor_y);
        pangocairo.showLayout(cr, layout);
        cursor_x += entry_w;
    }
    return (cursor_y - y) + row_h + 8.0;
}

fn drawAxes(widget: *gtk.Widget, cr: *cairo.Context, store: *Store, fg: gdk.RGBA, ticks: usize) void {
    const plot = store.plot;
    cairo.Context.setLineWidth(cr, 1.0);
    var i: usize = 0;
    while (i < ticks) : (i += 1) {
        const t = @as(f64, @floatFromInt(i)) / @as(f64, @floatFromInt(ticks - 1));
        const value = store.y_min + (store.y_max - store.y_min) * t;
        // Half-pixel offset keeps a 1px rule crisp instead of two grey rows.
        const py = @round(plot.y + plot.h - t * plot.h) + 0.5;
        if (store.show_grid) {
            setSource(cr, fg, 0.12);
            cairo.Context.newPath(cr);
            cairo.Context.moveTo(cr, plot.x, py);
            cairo.Context.lineTo(cr, plot.x + plot.w, py);
            cairo.Context.stroke(cr);
        }
        var buf: [32]u8 = undefined;
        setSource(cr, fg, 0.55);
        drawTextRight(widget, cr, formatTick(&buf, value), plot.x - 6.0, py, 0.8);
    }

    // x ticks: the ends plus the midpoint, which is as much as a small chart
    // can label without the numbers colliding.
    const xs = [_]f64{ 0.0, 0.5, 1.0 };
    for (xs) |t| {
        const px = @round(plot.x + t * plot.w) + 0.5;
        if (store.show_grid and t > 0.0 and t < 1.0) {
            setSource(cr, fg, 0.12);
            cairo.Context.newPath(cr);
            cairo.Context.moveTo(cr, px, plot.y);
            cairo.Context.lineTo(cr, px, plot.y + plot.h);
            cairo.Context.stroke(cr);
        }
        var buf: [32]u8 = undefined;
        const value = store.x_min + (store.x_max - store.x_min) * t;
        setSource(cr, fg, 0.55);
        drawTextCentered(widget, cr, formatTick(&buf, value), px, plot.y + plot.h + 3.0, 0.8);
    }

    setSource(cr, fg, 0.3);
    cairo.Context.newPath(cr);
    cairo.Context.moveTo(cr, plot.x + 0.5, plot.y);
    cairo.Context.lineTo(cr, plot.x + 0.5, @round(plot.y + plot.h) + 0.5);
    cairo.Context.lineTo(cr, plot.x + plot.w, @round(plot.y + plot.h) + 0.5);
    cairo.Context.stroke(cr);
}

fn drawLine(cr: *cairo.Context, store: *Store, s: Series, color: gdk.RGBA, p: f64, fill: bool) void {
    if (s.points.items.len == 0) return;
    const base = yToPx(store, @max(@min(0.0, store.y_max), store.y_min));
    cairo.Context.newPath(cr);
    for (s.points.items, 0..) |pt, i| {
        const px = xToPx(store, pt.x);
        const py = base + (yToPx(store, pt.y) - base) * p;
        if (i == 0) cairo.Context.moveTo(cr, px, py) else cairo.Context.lineTo(cr, px, py);
    }
    if (fill) {
        const last = s.points.items[s.points.items.len - 1];
        const first = s.points.items[0];
        cairo.Context.lineTo(cr, xToPx(store, last.x), base);
        cairo.Context.lineTo(cr, xToPx(store, first.x), base);
        cairo.Context.closePath(cr);
        setSource(cr, color, 0.22);
        cairo.Context.fillPreserve(cr);
        cairo.Context.newPath(cr);
        for (s.points.items, 0..) |pt, i| {
            const px = xToPx(store, pt.x);
            const py = base + (yToPx(store, pt.y) - base) * p;
            if (i == 0) cairo.Context.moveTo(cr, px, py) else cairo.Context.lineTo(cr, px, py);
        }
    }
    cairo.Context.setLineWidth(cr, 2.0);
    cairo.Context.setLineJoin(cr, .round);
    cairo.Context.setLineCap(cr, .round);
    setSource(cr, color, 1.0);
    cairo.Context.stroke(cr);
}

fn drawScatter(cr: *cairo.Context, store: *Store, s: Series, color: gdk.RGBA, p: f64) void {
    setSource(cr, color, 1.0);
    for (s.points.items) |pt| {
        cairo.Context.newPath(cr);
        cairo.Context.arc(cr, xToPx(store, pt.x), yToPx(store, pt.y), 3.5 * p, 0.0, std.math.tau);
        cairo.Context.fill(cr);
    }
}

fn drawBars(cr: *cairo.Context, store: *Store, s: Series, color: gdk.RGBA, p: f64, series_index: usize) void {
    if (s.points.items.len == 0) return;
    const n: f64 = @floatFromInt(s.points.items.len);
    const slot = store.plot.w / @max(n, 1.0);
    const group = @max(@as(f64, @floatFromInt(store.series.items.len)), 1.0);
    const bar_w = @max((slot * 0.7) / group, 1.0);
    const base = yToPx(store, @max(@min(0.0, store.y_max), store.y_min));
    setSource(cr, color, 0.9);
    for (s.points.items) |pt| {
        const center = xToPx(store, pt.x) - (group - 1.0) * bar_w / 2.0 + @as(f64, @floatFromInt(series_index)) * bar_w;
        const top = base + (yToPx(store, pt.y) - base) * p;
        cairo.Context.rectangle(cr, center - bar_w / 2.0, @min(top, base), bar_w, @abs(base - top));
        cairo.Context.fill(cr);
    }
}

fn drawCandles(cr: *cairo.Context, store: *Store, s: Series, p: f64) void {
    if (s.points.items.len == 0) return;
    const n: f64 = @floatFromInt(s.points.items.len);
    const body_w = @max((store.plot.w / @max(n, 1.0)) * 0.55, 1.0);
    var up = gdk.RGBA{ .f_red = 0.20, .f_green = 0.82, .f_blue = 0.478, .f_alpha = 1.0 };
    var down = gdk.RGBA{ .f_red = 0.753, .f_green = 0.110, .f_blue = 0.157, .f_alpha = 1.0 };
    // An explicit series colour wins over the up/down convention.
    if (s.color) |c| {
        up = c;
        down = c;
    }
    for (s.points.items) |pt| {
        if (!pt.has_ohlc) continue;
        const px = @round(xToPx(store, pt.x)) + 0.5;
        const rising = pt.close >= pt.open;
        const c = if (rising) up else down;
        const mid = (pt.high + pt.low) / 2.0;
        const high_y = yToPx(store, mid + (pt.high - mid) * p);
        const low_y = yToPx(store, mid + (pt.low - mid) * p);
        setSource(cr, c, 1.0);
        cairo.Context.setLineWidth(cr, 1.0);
        cairo.Context.newPath(cr);
        cairo.Context.moveTo(cr, px, high_y);
        cairo.Context.lineTo(cr, px, low_y);
        cairo.Context.stroke(cr);
        const body_mid = (pt.open + pt.close) / 2.0;
        const o = yToPx(store, body_mid + (pt.open - body_mid) * p);
        const cl = yToPx(store, body_mid + (pt.close - body_mid) * p);
        cairo.Context.rectangle(cr, px - body_w / 2.0, @min(o, cl), body_w, @max(@abs(cl - o), 1.0));
        cairo.Context.fill(cr);
    }
}

/// Pie reads the FIRST series only — a pie of two series is not a chart, it is
/// two charts (Swift Charts' SectorMark takes one angle scale for the same
/// reason). Slice magnitude is |y|; the labels come from the points.
fn drawPie(widget: *gtk.Widget, cr: *cairo.Context, store: *Store, fg: gdk.RGBA, box: Rect) void {
    if (store.series.items.len == 0) return;
    const s = store.series.items[0];
    var total: f64 = 0;
    for (s.points.items) |pt| total += @abs(pt.y);
    if (total <= 0) return;

    store.plot = box;
    const cx = box.x + box.w / 2.0;
    const cy = box.y + box.h / 2.0;
    const radius = @max(@min(box.w, box.h) / 2.0 - 8.0, 4.0);
    const p = if (store.progress >= 1.0) 1.0 else eased(store.progress);

    var angle: f64 = -std.math.pi / 2.0; // 12 o'clock, clockwise
    for (s.points.items, 0..) |pt, i| {
        const sweep = @abs(pt.y) / total * std.math.tau * p;
        setSource(cr, paletteAt(i), 1.0);
        cairo.Context.newPath(cr);
        cairo.Context.moveTo(cr, cx, cy);
        cairo.Context.arc(cr, cx, cy, radius, angle, angle + sweep);
        cairo.Context.closePath(cr);
        cairo.Context.fill(cr);
        if (pt.label) |label| {
            if (label.len > 0 and sweep > 0.25) {
                const mid = angle + sweep / 2.0;
                setSource(cr, fg, 0.9);
                drawTextCentered(widget, cr, label, cx + @cos(mid) * radius * 0.62, cy + @sin(mid) * radius * 0.62 - 8.0, 0.8);
            }
        }
        angle += sweep;
    }
}

// ---- create / applyProps --------------------------------------------------

/// `min_content_height` is the schema's Chart.minContentHeight, read by the
/// generated create arm. It is a floor, not a fixed height: a GtkDrawingArea
/// has no natural size, so without one a chart in a scroller measures zero
/// (TextArea/ScrollView take the same prop for the same reason).
pub fn create(props: ?std.json.Value, min_content_height: i64) *gtk.Widget {
    const area = gtk.DrawingArea.new();
    const widget = area.as(gtk.Widget);
    gtk.DrawingArea.setContentWidth(area, 240);
    if (min_content_height > 0) gtk.DrawingArea.setContentHeight(area, @intCast(min_content_height));
    gtk.Widget.setHexpand(widget, 1);
    gtk.Widget.setVexpand(widget, 1);

    const store = alloc.create(Store) catch return widget;
    store.* = .{};
    stores.put(alloc, @intFromPtr(widget), store) catch {
        alloc.destroy(store);
        return widget;
    };
    _ = gtk.Widget.signals.destroy.connect(widget, ?*anyopaque, &cbDestroyed, null, .{});

    ingest(store, props);
    gtk.DrawingArea.setDrawFunc(area, &draw, null, null);

    const click = gtk.GestureClick.new();
    gtk.GestureSingle.setButton(click.as(gtk.GestureSingle), 1);
    _ = gtk.GestureClick.signals.released.connect(click, *gtk.Widget, &cbClicked, widget, .{});
    gtk.Widget.addController(widget, click.as(gtk.EventController));

    watchStyleManager();
    restartAnimation(widget, store);
    return widget;
}

pub fn applyProps(widget: *gtk.Widget, props: ?std.json.Value) void {
    const store = storeOf(widget) orelse return;
    const had_data = propArray(props, "series") != null or propStr(props, "type") != null;
    ingest(store, props);
    if (had_data) restartAnimation(widget, store) else gtk.Widget.queueDraw(widget);
}

/// Every prop is read the same way on create and update — the widget holds no
/// native state a partial apply could desynchronize.
fn ingest(store: *Store, props: ?std.json.Value) void {
    if (propStr(props, "type")) |t| store.kind = parseKind(t);
    if (propArray(props, "series")) |arr| parseSeries(store, arr);
    if (propStr(props, "xLabel")) |l| {
        if (store.x_label) |old| alloc.free(old);
        store.x_label = alloc.dupeZ(u8, l) catch null;
    }
    if (propStr(props, "yLabel")) |l| {
        if (store.y_label) |old| alloc.free(old);
        store.y_label = alloc.dupeZ(u8, l) catch null;
    }
    if (propBool(props, "showLegend")) |v| store.show_legend = v;
    if (propBool(props, "showGrid")) |v| store.show_grid = v;
    if (propBool(props, "animated")) |v| store.animated = v;
}

fn cbDestroyed(widget: *gtk.Widget, _: ?*anyopaque) callconv(.c) void {
    const key = @intFromPtr(widget);
    if (stores.fetchRemove(key)) |kv| {
        kv.value.deinit();
        alloc.destroy(kv.value);
    }
}

// ---- theme ----------------------------------------------------------------

fn watchStyleManager() void {
    if (style_watch_connected) return;
    style_watch_connected = true;
    const mgr = adw.StyleManager.getDefault();
    _ = gobject.signalConnectData(asObject(mgr), "notify", @ptrCast(&cbStyleChanged), null, null, .{});
}

/// Any AdwStyleManager property change (dark, accent-color, high-contrast)
/// repaints every live chart: the colours are read inside the draw function,
/// so a queue_draw is the whole update.
fn cbStyleChanged(_: *gobject.Object, _: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
    var it = stores.keyIterator();
    while (it.next()) |key| {
        const widget: *gtk.Widget = @ptrFromInt(key.*);
        gtk.Widget.queueDraw(widget);
    }
}

// ---- events ---------------------------------------------------------------

pub fn connectEvents(widget: *gtk.Widget, node_id: u32, emit_fn: EmitFn) void {
    emit = emit_fn;
    const store = storeOf(widget) orelse return;
    store.node_id = node_id;
}

fn cbClicked(_: *gtk.GestureClick, _: c_int, x: f64, y: f64, widget: *gtk.Widget) callconv(.c) void {
    const store = storeOf(widget) orelse return;
    if (store.node_id == 0) return;
    const f = emit orelse return;
    const hit = hitTest(store, x, y) orelse return;
    const s = store.series.items[hit.series];
    const pt = s.points.items[hit.point];

    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    payload.put(alloc, "seriesId", .{ .string = s.id }) catch {};
    payload.put(alloc, "seriesIndex", .{ .integer = @intCast(hit.series) }) catch {};
    payload.put(alloc, "index", .{ .integer = @intCast(hit.point) }) catch {};
    payload.put(alloc, "x", .{ .float = pt.x }) catch {};
    payload.put(alloc, "y", .{ .float = pt.y }) catch {};
    f(store.node_id, "pointSelected", .{ .data = .{ .object = payload } });
}

const Hit = struct { series: usize, point: usize };

fn hitTest(store: *Store, x: f64, y: f64) ?Hit {
    if (store.series.items.len == 0) return null;
    if (store.kind == .pie) return pieHit(store, x, y);

    var best: ?Hit = null;
    var best_d: f64 = HIT_RADIUS * HIT_RADIUS;
    for (store.series.items, 0..) |s, si| {
        for (s.points.items, 0..) |pt, pi| {
            const px = xToPx(store, pt.x);
            const py = yToPx(store, if (pt.has_ohlc) pt.close else pt.y);
            const dx = px - x;
            const dy = py - y;
            const d = dx * dx + dy * dy;
            if (d <= best_d) {
                best_d = d;
                best = .{ .series = si, .point = pi };
            }
        }
    }
    return best;
}

fn pieHit(store: *Store, x: f64, y: f64) ?Hit {
    const s = store.series.items[0];
    var total: f64 = 0;
    for (s.points.items) |pt| total += @abs(pt.y);
    if (total <= 0) return null;

    const cx = store.plot.x + store.plot.w / 2.0;
    const cy = store.plot.y + store.plot.h / 2.0;
    const radius = @max(@min(store.plot.w, store.plot.h) / 2.0 - 8.0, 4.0);
    const dx = x - cx;
    const dy = y - cy;
    if (dx * dx + dy * dy > radius * radius) return null;

    // atan2 returns (-pi, pi]; the sectors start at -pi/2, so rebase into
    // [0, tau) measured clockwise from 12 o'clock.
    var theta = std.math.atan2(dy, dx) + std.math.pi / 2.0;
    if (theta < 0) theta += std.math.tau;
    var angle: f64 = 0;
    for (s.points.items, 0..) |pt, i| {
        const sweep = @abs(pt.y) / total * std.math.tau;
        if (theta >= angle and theta < angle + sweep) return .{ .series = 0, .point = i };
        angle += sweep;
    }
    return null;
}
