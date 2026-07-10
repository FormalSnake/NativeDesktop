const std = @import("std");
const gtk = @import("gtk");
const glib = @import("glib");
const protocol = @import("protocol.zig");
const generated = @import("generated/widgets.zig");
const style = @import("style.zig");

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

pub fn appendChild(parent: *gtk.Widget, parent_kind: []const u8, child: *gtk.Widget, attached: protocol.Attached) void {
    generated.appendChild(parent, parent_kind, child, attached, &dupeZ);
}

pub fn setText(widget: *gtk.Widget, text: []const u8) void {
    const label: *gtk.Label = @ptrCast(@alignCast(widget));
    gtk.Label.setText(label, dupeZ(text));
}

pub fn removeChild(parent: *gtk.Widget, parent_kind: []const u8, child: *gtk.Widget) void {
    generated.removeChild(parent, parent_kind, child);
}

pub fn insertBefore(parent: *gtk.Widget, parent_kind: []const u8, child: *gtk.Widget, before: ?*gtk.Widget, attached: protocol.Attached) void {
    generated.insertBefore(parent, parent_kind, child, before, attached, &dupeZ);
}

pub fn setVisible(widget: *gtk.Widget, visible: bool) void {
    gtk.Widget.setVisible(widget, @intFromBool(visible));
}

pub fn applyProps(widget: *gtk.Widget, kind: []const u8, props: ?std.json.Value) void {
    generated.applyProps(widget, kind, props, &dupeZ);
}

pub fn initStyle(sink_err: style.StyleErrorFn) void {
    style.init(arena, sink_err);
}

pub fn applyStyle(widget: *gtk.Widget, node_id: u32, style_value: std.json.Value) void {
    style.applyStyle(widget, node_id, style_value);
}

/// Generation GC helpers (M8-D9): detach a swept widget from its parent
/// without destroying the parent or siblings.
pub fn hasParent(widget: *gtk.Widget) bool {
    return gtk.Widget.getParent(widget) != null;
}

pub fn unparentWidget(widget: *gtk.Widget) void {
    gtk.Widget.unparent(widget);
}
