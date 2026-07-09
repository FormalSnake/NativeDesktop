const std = @import("std");
const gtk = @import("gtk");
const glib = @import("glib");
const protocol = @import("protocol.zig");
const generated = @import("generated/widgets.zig");

pub const Widget = gtk.Widget;

var arena_state = std.heap.ArenaAllocator.init(std.heap.page_allocator);
const arena = arena_state.allocator();
var the_window: ?*gtk.Window = null;

pub const EventSink = *const fn (node_id: u32, name: []const u8, payload: protocol.EventPayload) void;

pub fn setEventSink(sink: EventSink) void {
    // Generated module owns all signal wiring + echo suppression (M5b-D5).
    // The arena outlives every widget; runtime installs the sink before the
    // NDP socket accepts, so this always precedes the first create.
    generated.initEvents(arena, sink);
}

pub fn connectEvents(widget: *gtk.Widget, kind: []const u8, node_id: u32) void {
    generated.connectEvents(widget, kind, node_id);
}

pub fn getWindow() ?*gtk.Window {
    return the_window;
}

fn dupeZ(s: []const u8) [:0]const u8 {
    return arena.dupeZ(u8, s) catch @panic("OOM in gtk_backend arena");
}

pub fn createWidget(app: *gtk.Application, kind: []const u8, props: ?std.json.Value) !*gtk.Widget {
    return generated.create(app, kind, props, &dupeZ, &the_window);
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
    generated.applyProps(widget, kind, props, &dupeZ);
}
