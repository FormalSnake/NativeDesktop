// Widget-level drag and drop for the GTK backend: the `draggable` /
// `dragPayload` / `dropTarget` props and the dragStarted / dragEnded /
// dragOver / dropped events.
//
// These are universal props, so this module is driven from ONE arm above the
// generated kind dispatch (tools/codegen.ts, UNIVERSAL_EVENTS) rather than
// from 69 per-widget templates. GTK4 models both halves as event controllers
// you attach to any GtkWidget, which is exactly the shape a universal arm
// needs.
//
// The payload is a plain string the app defines. It travels as a
// GdkContentProvider carrying a G_TYPE_STRING GValue, so a drag also
// interoperates with anything else on the desktop that accepts text.
//
// Ordering: the props arm runs at create time, before the core calls
// connectEvents, so a controller exists before its node id does. The
// callbacks therefore read the id back off the widget at fire time (the
// ndTagConnect idiom) instead of capturing it at connect time.
const std = @import("std");
const gtk = @import("gtk");
const gdk = @import("gdk");
const gobject = @import("gobject");
const protocol = @import("../protocol.zig");

pub const EmitFn = *const fn (node_id: u32, name: []const u8, payload: protocol.EventPayload) void;

const alloc = std.heap.page_allocator;

var emit: ?EmitFn = null;

// g_value_init needs the fundamental GType id; string is 16 << G_TYPE_FUNDAMENTAL_SHIFT
// (gobject/gtype.h, shift = 2). Same constant, same reason, as src/gtk/audio.zig.
const G_TYPE_STRING: usize = 16 << 2;

const NODE_ID_KEY = "nd-dnd-node-id";
const PAYLOAD_KEY = "nd-dnd-payload";
const SOURCE_KEY = "nd-dnd-source";
const TARGET_KEY = "nd-dnd-target";

fn asObject(widget: *gtk.Widget) *gobject.Object {
    return @ptrCast(@alignCast(widget));
}

fn propStr(props: ?std.json.Value, key: []const u8) ?[]const u8 {
    const v = props orelse return null;
    if (v != .object) return null;
    return switch (v.object.get(key) orelse return null) {
        .string => |s| s,
        else => null,
    };
}

fn propBool(props: ?std.json.Value, key: []const u8) ?bool {
    const v = props orelse return null;
    if (v != .object) return null;
    return switch (v.object.get(key) orelse return null) {
        .bool => |b| b,
        else => null,
    };
}

fn nodeIdOf(widget: *gtk.Widget) u32 {
    const raw = gobject.Object.getData(asObject(widget), NODE_ID_KEY) orelse return 0;
    return @intCast(@intFromPtr(raw));
}

/// The app's `dragPayload`, or "" when it set none. Owned by the backend
/// arena (dupeZ), so the pointer outlives every drag.
fn payloadOf(widget: *gtk.Widget) []const u8 {
    const raw = gobject.Object.getData(asObject(widget), PAYLOAD_KEY) orelse return "";
    const ptr: [*:0]const u8 = @ptrCast(@alignCast(raw));
    return std.mem.span(ptr);
}

/// Widget a controller is attached to. Null between `remove_controller` and
/// the controller's own finalize, which is when a stray late callback would
/// otherwise dereference a dead node.
fn widgetOf(controller: *gtk.EventController) ?*gtk.Widget {
    return gtk.EventController.getWidget(controller);
}

// ============================================================================
// Drag source
// ============================================================================

/// The GdkContentProvider is built per drag rather than once at attach time:
/// `dragPayload` can change on any update, and `prepare` is the only place
/// GTK guarantees to read it fresh.
fn cbPrepare(src: *gtk.DragSource, _: f64, _: f64, _: ?*anyopaque) callconv(.c) ?*gdk.ContentProvider {
    const widget = widgetOf(src.as(gtk.EventController)) orelse return null;
    const payload = payloadOf(widget);
    var value: gobject.Value = std.mem.zeroes(gobject.Value);
    _ = gobject.Value.init(&value, G_TYPE_STRING);
    defer gobject.Value.unset(&value);
    // The GValue copies the string, so the arena pointer need not outlive this.
    var buf: [1024]u8 = undefined;
    const len = @min(payload.len, buf.len - 1);
    @memcpy(buf[0..len], payload[0..len]);
    buf[len] = 0;
    gobject.Value.setString(&value, @ptrCast(&buf));
    return gdk.ContentProvider.newForValue(&value);
}

fn cbDragBegin(src: *gtk.DragSource, _: *gdk.Drag, _: ?*anyopaque) callconv(.c) void {
    const widget = widgetOf(src.as(gtk.EventController)) orelse return;
    const node_id = nodeIdOf(widget);
    if (node_id == 0) return;
    if (emit) |f| f(node_id, "dragStarted", .{ .text = payloadOf(widget) });
}

fn cbDragEnd(src: *gtk.DragSource, _: *gdk.Drag, _: c_int, _: ?*anyopaque) callconv(.c) void {
    const widget = widgetOf(src.as(gtk.EventController)) orelse return;
    const node_id = nodeIdOf(widget);
    if (node_id == 0) return;
    if (emit) |f| f(node_id, "dragEnded", .{});
}

fn ensureSource(widget: *gtk.Widget) void {
    if (gobject.Object.getData(asObject(widget), SOURCE_KEY) != null) return;
    const src = gtk.DragSource.new();
    gtk.DragSource.setActions(src, .{ .copy = true, .move = true });
    _ = gtk.DragSource.signals.prepare.connect(src, ?*anyopaque, &cbPrepare, null, .{});
    _ = gtk.DragSource.signals.drag_begin.connect(src, ?*anyopaque, &cbDragBegin, null, .{});
    _ = gtk.DragSource.signals.drag_end.connect(src, ?*anyopaque, &cbDragEnd, null, .{});
    gtk.Widget.addController(widget, src.as(gtk.EventController));
    gobject.Object.setData(asObject(widget), SOURCE_KEY, src);
}

fn removeSource(widget: *gtk.Widget) void {
    const raw = gobject.Object.getData(asObject(widget), SOURCE_KEY) orelse return;
    const src: *gtk.DragSource = @ptrCast(@alignCast(raw));
    gobject.Object.setData(asObject(widget), SOURCE_KEY, null);
    // add_controller took the only ref; removing it is what frees the controller.
    gtk.Widget.removeController(widget, src.as(gtk.EventController));
}

// ============================================================================
// Drop target
// ============================================================================

/// x/y arrive in the target widget's own coordinate space, which is what the
/// app needs to hit-test a drop zone against its own bounds — so they go on
/// the wire unchanged.
fn emitPoint(widget: *gtk.Widget, name: []const u8, text: []const u8, x: f64, y: f64) void {
    const node_id = nodeIdOf(widget);
    if (node_id == 0) return;
    const f = emit orelse return;
    var data: std.json.ObjectMap = .empty;
    defer data.deinit(alloc);
    data.put(alloc, "x", .{ .float = x }) catch {};
    data.put(alloc, "y", .{ .float = y }) catch {};
    f(node_id, name, .{ .text = text, .data = .{ .object = data } });
}

fn valueString(value: ?*const gobject.Value) []const u8 {
    const v = value orelse return "";
    const raw = gobject.Value.getString(v) orelse return "";
    return std.mem.span(raw);
}

fn cbMotion(target: *gtk.DropTarget, x: f64, y: f64, _: ?*anyopaque) callconv(.c) gdk.DragAction {
    const widget = widgetOf(target.as(gtk.EventController)) orelse return .{};
    // preload is on (see ensureTarget), so the dragged string is already
    // readable here rather than only at drop time.
    emitPoint(widget, "dragOver", valueString(gtk.DropTarget.getValue(target)), x, y);
    return .{ .copy = true };
}

fn cbDrop(target: *gtk.DropTarget, value: *gobject.Value, x: f64, y: f64, _: ?*anyopaque) callconv(.c) c_int {
    const widget = widgetOf(target.as(gtk.EventController)) orelse return 0;
    emitPoint(widget, "dropped", valueString(value), x, y);
    return 1;
}

fn ensureTarget(widget: *gtk.Widget) void {
    if (gobject.Object.getData(asObject(widget), TARGET_KEY) != null) return;
    const target = gtk.DropTarget.new(G_TYPE_STRING, .{ .copy = true, .move = true });
    // Without preload, `motion` sees no value and the app could only hit-test
    // on drop — too late to highlight a dock zone under the cursor.
    gtk.DropTarget.setPreload(target, 1);
    _ = gtk.DropTarget.signals.motion.connect(target, ?*anyopaque, &cbMotion, null, .{});
    _ = gtk.DropTarget.signals.drop.connect(target, ?*anyopaque, &cbDrop, null, .{});
    gtk.Widget.addController(widget, target.as(gtk.EventController));
    gobject.Object.setData(asObject(widget), TARGET_KEY, target);
}

fn removeTarget(widget: *gtk.Widget) void {
    const raw = gobject.Object.getData(asObject(widget), TARGET_KEY) orelse return;
    const target: *gtk.DropTarget = @ptrCast(@alignCast(raw));
    gobject.Object.setData(asObject(widget), TARGET_KEY, null);
    gtk.Widget.removeController(widget, target.as(gtk.EventController));
}

// ============================================================================
// Generated-dispatcher seam
// ============================================================================

/// Universal props arm, called from both `create` and `applyProps`. Menu
/// nodes hand back GMenu objects rather than widgets, hence the type guard
/// the tooltip arm carries for the same reason.
pub fn applyProps(widget: *gtk.Widget, props: ?std.json.Value, dupeZ: *const fn ([]const u8) [:0]const u8) void {
    if (!gobject.ext.isA(widget, gtk.Widget)) return;
    if (propStr(props, "dragPayload")) |p| {
        const owned = dupeZ(p);
        gobject.Object.setData(asObject(widget), PAYLOAD_KEY, @constCast(owned.ptr));
    }
    if (propBool(props, "draggable")) |on| {
        if (on) ensureSource(widget) else removeSource(widget);
    }
    if (propBool(props, "dropTarget")) |on| {
        if (on) ensureTarget(widget) else removeTarget(widget);
    }
}

/// Universal connect arm. Records the node id the controllers report against
/// and installs the emit sink once.
pub fn connectEvents(widget: *gtk.Widget, node_id: u32, emit_fn: EmitFn) void {
    emit = emit_fn;
    if (!gobject.ext.isA(widget, gtk.Widget)) return;
    gobject.Object.setData(asObject(widget), NODE_ID_KEY, @ptrFromInt(@as(usize, node_id)));
}
