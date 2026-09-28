// <box tileMinWidth tileMaxColumns tileAspect>, GTK half. AppKit peer:
// NDBoxView's tile grid (NDShell/Layout.swift).
//
// A box with `tileMinWidth` above 0 lays its children out as a grid of equal
// cells that always spans its width: as many columns as fit at the minimum
// width (or a child's own minimum, whichever is wider), at most
// `tileMaxColumns`, share the width after the spacing. A short last row keeps
// the column pitch. With `tileAspect` a cell is that fraction of its width
// tall, so the box asks for height-for-width. Children's margins still apply
// (GTK adds them inside the allocation); expand and align flags do not.
//
// The GtkBoxLayout is swapped for a GtkCustomLayout while the grid is on, and
// GtkBox reads and writes its spacing through its layout manager, so the
// spacing lives here then (`setSpacing`).
const std = @import("std");
const gtk = @import("gtk");
const gobject = @import("gobject");

const K_STATE = "nd-tile-grid";

const State = struct {
    min_width: c_int = 0,
    max_columns: c_int = 0,
    aspect: f64 = 0,
    spacing: c_int = 0,
    orientation: gtk.Orientation = .horizontal,
};

fn asObject(p: anytype) *gobject.Object {
    return @ptrCast(@alignCast(p));
}

fn stateOf(w: *gtk.Widget) ?*State {
    const raw = gobject.Object.getData(asObject(w), K_STATE) orelse return null;
    return @ptrCast(@alignCast(raw));
}

fn freeState(data: ?*anyopaque) callconv(.c) void {
    const s: *State = @ptrCast(@alignCast(data.?));
    std.heap.c_allocator.destroy(s);
}

fn on(w: *gtk.Widget) ?*State {
    const s = stateOf(w) orelse return null;
    return if (s.min_width > 0) s else null;
}

pub fn setSpacing(w: *gtk.Widget, spacing: c_int) void {
    if (on(w)) |s| {
        if (s.spacing == spacing) return;
        s.spacing = spacing;
        gtk.Widget.queueResize(w);
        return;
    }
    gtk.Box.setSpacing(@ptrCast(@alignCast(w)), spacing);
}

pub fn applyProps(w: *gtk.Widget, min_width: ?i64, max_columns: ?i64, aspect: ?f64) void {
    if (min_width == null and max_columns == null and aspect == null) return;
    const s = stateOf(w) orelse blk: {
        const fresh = std.heap.c_allocator.create(State) catch return;
        fresh.* = .{};
        gobject.Object.setDataFull(asObject(w), K_STATE, fresh, &freeState);
        break :blk fresh;
    };
    const was_on = s.min_width > 0;
    if (min_width) |v| s.min_width = @intCast(std.math.clamp(v, 0, 1 << 16));
    if (max_columns) |v| s.max_columns = @intCast(std.math.clamp(v, 0, 1 << 16));
    if (aspect) |v| s.aspect = @max(v, 0);
    const now_on = s.min_width > 0;
    if (now_on and !was_on) {
        const box: *gtk.Box = @ptrCast(@alignCast(w));
        s.spacing = gtk.Box.getSpacing(box);
        s.orientation = gtk.Orientable.getOrientation(@ptrCast(box));
        gtk.Widget.setLayoutManager(w, gtk.CustomLayout.new(&requestMode, &measure, &allocate).as(gtk.LayoutManager));
    } else if (was_on and !now_on) {
        const layout = gtk.BoxLayout.new(s.orientation);
        gtk.BoxLayout.setSpacing(layout, @intCast(s.spacing));
        gtk.Widget.setLayoutManager(w, layout.as(gtk.LayoutManager));
    }
    gtk.Widget.queueResize(w);
}

const Grid = struct {
    columns: c_int,
    rows: c_int,
    cell_width: f64,
    cell_height: c_int,
};

const Children = struct {
    count: c_int = 0,
    min_width: c_int = 0,
    min_height: c_int = 0,
    natural_height: c_int = 0,
};

fn children(w: *gtk.Widget) Children {
    var out: Children = .{};
    var child = gtk.Widget.getFirstChild(w);
    while (child) |c| : (child = gtk.Widget.getNextSibling(c)) {
        if (gtk.Widget.shouldLayout(c) == 0) continue;
        var min: c_int = 0;
        var nat: c_int = 0;
        gtk.Widget.measure(c, .horizontal, -1, &min, null, null, null);
        out.min_width = @max(out.min_width, min);
        gtk.Widget.measure(c, .vertical, -1, &min, &nat, null, null);
        out.min_height = @max(out.min_height, min);
        out.natural_height = @max(out.natural_height, nat);
        out.count += 1;
    }
    return out;
}

/// The narrowest a column may be: the box's minimum, a child's minimum
/// width, and, with an aspect, the width at which a child's minimum height
/// is still the aspect's height (a cell is never stretched taller).
fn unit(s: *const State, kids: Children) c_int {
    var u = @max(s.min_width, kids.min_width);
    if (s.aspect > 0) u = @max(u, @as(c_int, @intFromFloat(@ceil(@as(f64, @floatFromInt(kids.min_height)) / s.aspect))));
    return u;
}

fn grid(w: *gtk.Widget, s: *const State, kids: Children, width: c_int) Grid {
    const floor: f64 = @floatFromInt(unit(s, kids));
    const gap: f64 = @floatFromInt(s.spacing);
    const avail: f64 = @floatFromInt(@max(width, 0));
    var columns: c_int = @max(1, @as(c_int, @intFromFloat(@floor((avail + gap) / (floor + gap)))));
    if (s.max_columns > 0) columns = @min(columns, s.max_columns);
    const cols: f64 = @floatFromInt(columns);
    const cell_width = @max(0, (avail - gap * (cols - 1)) / cols);
    var cell_height: c_int = if (s.aspect > 0) @intFromFloat(@round(cell_width * s.aspect)) else kids.natural_height;
    // Never under what a child needs at the cell's width.
    var child = gtk.Widget.getFirstChild(w);
    while (child) |c| : (child = gtk.Widget.getNextSibling(c)) {
        if (gtk.Widget.shouldLayout(c) == 0) continue;
        var min: c_int = 0;
        gtk.Widget.measure(c, .vertical, @intFromFloat(@floor(cell_width)), &min, null, null, null);
        cell_height = @max(cell_height, min);
    }
    const rows = @divFloor(kids.count + columns - 1, columns);
    return .{ .columns = columns, .rows = rows, .cell_width = cell_width, .cell_height = cell_height };
}

fn span(g: Grid, gap: c_int) c_int {
    if (g.rows == 0) return 0;
    return g.rows * g.cell_height + gap * (g.rows - 1);
}

fn requestMode(_: *gtk.Widget) callconv(.c) gtk.SizeRequestMode {
    return .height_for_width;
}

fn measure(
    w: *gtk.Widget,
    orientation: gtk.Orientation,
    for_size: c_int,
    minimum: *c_int,
    natural: *c_int,
    minimum_baseline: *c_int,
    natural_baseline: *c_int,
) callconv(.c) void {
    minimum_baseline.* = -1;
    natural_baseline.* = -1;
    const s = stateOf(w) orelse {
        minimum.* = 0;
        natural.* = 0;
        return;
    };
    const kids = children(w);
    if (kids.count == 0) {
        minimum.* = 0;
        natural.* = 0;
        return;
    }
    const u = unit(s, kids);
    // Natural: the columns the children fill at the minimum width; minimum:
    // one column.
    const cap = if (s.max_columns > 0) @min(kids.count, s.max_columns) else kids.count;
    const natural_width = cap * u + s.spacing * (cap - 1);
    if (orientation == .horizontal) {
        minimum.* = u;
        natural.* = natural_width;
        return;
    }
    const h = span(grid(w, s, kids, if (for_size >= 0) for_size else natural_width), s.spacing);
    minimum.* = h;
    natural.* = h;
}

fn allocate(w: *gtk.Widget, width: c_int, _: c_int, _: c_int) callconv(.c) void {
    const s = stateOf(w) orelse return;
    const kids = children(w);
    if (kids.count == 0) return;
    const g = grid(w, s, kids, width);
    const pitch = g.cell_width + @as(f64, @floatFromInt(s.spacing));
    const rtl = gtk.Widget.getDirection(w) == .rtl;
    var i: c_int = 0;
    var child = gtk.Widget.getFirstChild(w);
    while (child) |c| : (child = gtk.Widget.getNextSibling(c)) {
        if (gtk.Widget.shouldLayout(c) == 0) continue;
        const row = @divFloor(i, g.columns);
        var column = @mod(i, g.columns);
        if (rtl) column = g.columns - 1 - column;
        // Both edges to the nearest pixel: cells differ by at most one and
        // the last column ends on the box's trailing edge.
        const left = @as(f64, @floatFromInt(column)) * pitch;
        const x: c_int = @intFromFloat(@round(left));
        const right: c_int = @intFromFloat(@round(left + g.cell_width));
        var rect: gtk.Allocation = .{
            .f_x = x,
            .f_y = row * (g.cell_height + s.spacing),
            .f_width = right - x,
            .f_height = g.cell_height,
        };
        gtk.Widget.sizeAllocate(c, &rect, -1);
        i += 1;
    }
}
