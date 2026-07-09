const std = @import("std");
const gtk = @import("gtk");
const glib = @import("glib");

var event_sink: ?EventSink = null;
var arena_state = std.heap.ArenaAllocator.init(std.heap.page_allocator);
const arena = arena_state.allocator();
var the_window: ?*gtk.Window = null;

pub const EventSink = *const fn (node_id: u32) void;

pub fn setEventSink(sink: EventSink) void {
    event_sink = sink;
}

fn dupeZ(s: []const u8) [:0]const u8 {
    return arena.dupeZ(u8, s) catch @panic("OOM in gtk_backend arena");
}

fn propStr(props: ?std.json.Value, key: []const u8) ?[]const u8 {
    const v = props orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .string => field.string,
        else => null,
    };
}

fn propInt(props: ?std.json.Value, key: []const u8) ?i64 {
    const v = props orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .integer => field.integer,
        else => null,
    };
}

pub fn createWidget(app: *gtk.Application, kind: []const u8, props: ?std.json.Value) !*gtk.Widget {
    if (std.mem.eql(u8, kind, "Window")) {
        const window = gtk.ApplicationWindow.new(app);
        const win = window.as(gtk.Window);
        the_window = win;
        if (propStr(props, "title")) |t| gtk.Window.setTitle(win, dupeZ(t));
        const w: c_int = @intCast(propInt(props, "defaultWidth") orelse 480);
        const h: c_int = @intCast(propInt(props, "defaultHeight") orelse 320);
        gtk.Window.setDefaultSize(win, w, h);
        gtk.Window.present(win);
        return window.as(gtk.Widget);
    } else if (std.mem.eql(u8, kind, "Box")) {
        const vertical = if (propStr(props, "orientation")) |o| std.mem.eql(u8, o, "vertical") else true;
        const orientation: gtk.Orientation = if (vertical) .vertical else .horizontal;
        const spacing: c_int = @intCast(propInt(props, "spacing") orelse 0);
        const box = gtk.Box.new(orientation, spacing);
        return box.as(gtk.Widget);
    } else if (std.mem.eql(u8, kind, "Label")) {
        const text = propStr(props, "text") orelse "";
        const label = gtk.Label.new(dupeZ(text));
        return label.as(gtk.Widget);
    } else if (std.mem.eql(u8, kind, "Button")) {
        const lbl = propStr(props, "label") orelse "Button";
        const button = gtk.Button.newWithLabel(dupeZ(lbl));
        return button.as(gtk.Widget);
    }
    std.debug.print("ND_WARN unknown widget kind={s}\n", .{kind});
    return error.UnknownWidget;
}

/// The clicked-signal user-data is the node id, packed into the pointer slot.
pub fn connectButtonClick(button: *gtk.Button, node_id: u32) void {
    const data: ?*anyopaque = @ptrFromInt(@as(usize, node_id));
    _ = gtk.Button.signals.clicked.connect(button, ?*anyopaque, &onClicked, data, .{});
}

fn onClicked(_: *gtk.Button, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    if (event_sink) |sink| sink(node_id);
}

pub fn appendChild(parent: *gtk.Widget, child: *gtk.Widget) void {
    if (the_window) |win| {
        if (parent == win.as(gtk.Widget)) {
            gtk.Window.setChild(win, child);
            return;
        }
    }
    const box: *gtk.Box = @ptrCast(@alignCast(parent));
    gtk.Box.append(box, child);
}

pub fn setText(widget: *gtk.Widget, text: []const u8) void {
    const label: *gtk.Label = @ptrCast(@alignCast(widget));
    gtk.Label.setText(label, dupeZ(text));
}

pub fn removeChild(parent: *gtk.Widget, child: *gtk.Widget) void {
    if (the_window) |win| {
        if (parent == win.as(gtk.Widget)) {
            gtk.Window.setChild(win, null);
            return;
        }
    }
    const box: *gtk.Box = @ptrCast(@alignCast(parent));
    gtk.Box.remove(box, child);
}

pub fn insertBefore(parent: *gtk.Widget, child: *gtk.Widget, before: ?*gtk.Widget) void {
    if (the_window) |win| {
        if (parent == win.as(gtk.Widget)) {
            gtk.Window.setChild(win, child);
            return;
        }
    }
    const box: *gtk.Box = @ptrCast(@alignCast(parent));
    if (before) |b| {
        const prev = gtk.Widget.getPrevSibling(b);
        gtk.Box.insertChildAfter(box, child, prev); // prev == null => head
    } else {
        gtk.Box.append(box, child);
    }
}

pub fn setVisible(widget: *gtk.Widget, visible: bool) void {
    gtk.Widget.setVisible(widget, @intFromBool(visible));
}

pub fn applyProps(widget: *gtk.Widget, kind: []const u8, props: ?std.json.Value) void {
    if (std.mem.eql(u8, kind, "Box")) {
        if (propInt(props, "spacing")) |s| {
            const box: *gtk.Box = @ptrCast(@alignCast(widget));
            gtk.Box.setSpacing(box, @intCast(s));
        }
    } else if (std.mem.eql(u8, kind, "Window")) {
        if (propStr(props, "title")) |t| {
            const win: *gtk.Window = @ptrCast(@alignCast(widget));
            gtk.Window.setTitle(win, dupeZ(t));
        }
    }
}
