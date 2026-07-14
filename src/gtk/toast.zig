// AdwToastOverlay surface for the <toastoverlay> widget (M15): showToast /
// dismissToast commands with caller-supplied id correlation — the same
// widgetCommand → same-node result-event pattern as webview.zig's
// executeJavaScript. AdwToastOverlay's built-in queue (one visible toast,
// HIGH priority interrupts) sets the cross-platform queue contract.
const std = @import("std");
const gtk = @import("gtk");
const gobject = @import("gobject");
const glib = @import("glib");
const adw = @import("adw");
const protocol = @import("../protocol.zig");

pub const EmitFn = *const fn (node_id: u32, name: []const u8, payload: protocol.EventPayload) void;

const NODE_ID_KEY = "nd-toast-node-id";
const TOAST_ID_KEY = "nd-toast-id"; // NUL-terminated id string, owned by `toasts`' key

var emit: ?EmitFn = null;
/// Live toasts by caller id — entries evicted (and the extra ref dropped) in
/// cbToastDismissed. Ids are assumed unique per app while live (the React
/// wrapper generates them, same contract as executeJavaScript ids).
var toasts: std.StringHashMapUnmanaged(*adw.Toast) = .empty;

/// Generated connectEvents ToastOverlay arm.
pub fn connectEvents(widget: *gtk.Widget, node_id: u32, emit_fn: EmitFn) void {
    emit = emit_fn;
    gobject.Object.setData(widget.as(gobject.Object), NODE_ID_KEY, @ptrFromInt(@as(usize, node_id)));
}

fn widgetNodeId(widget: *gtk.Widget) ?u32 {
    const raw = gobject.Object.getData(widget.as(gobject.Object), NODE_ID_KEY) orelse return null;
    return @intCast(@intFromPtr(raw));
}

fn argObject(arg: ?std.json.Value) ?std.json.ObjectMap {
    return switch (arg orelse return null) {
        .object => |o| o,
        else => null,
    };
}

fn objStr(obj: std.json.ObjectMap, key: []const u8) ?[]const u8 {
    return switch (obj.get(key) orelse return null) {
        .string => |s| s,
        else => null,
    };
}

fn objInt(obj: std.json.ObjectMap, key: []const u8) ?i64 {
    return switch (obj.get(key) orelse return null) {
        .integer => |n| n,
        else => null,
    };
}

fn emitWithId(node_id: u32, name: []const u8, toast_id: []const u8) void {
    const f = emit orelse return;
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(std.heap.page_allocator);
    payload.put(std.heap.page_allocator, "id", .{ .string = toast_id }) catch {};
    f(node_id, name, .{ .data = .{ .object = payload } });
}

fn toastId(toast: *adw.Toast) ?[]const u8 {
    const raw = gobject.Object.getData(@ptrCast(@alignCast(toast)), TOAST_ID_KEY) orelse return null;
    return std.mem.span(@as([*:0]const u8, @ptrCast(raw)));
}

/// widgetCommand dispatch (generated widgets.zig ToastOverlay arm).
pub fn command(widget: *gtk.Widget, cmd: []const u8, arg: ?std.json.Value) void {
    if (std.mem.eql(u8, cmd, "showToast")) {
        cmdShowToast(widget, arg);
    } else if (std.mem.eql(u8, cmd, "dismissToast")) {
        cmdDismissToast(arg);
    } else {
        std.debug.print("ND_WARN unknown ToastOverlay command {s}\n", .{cmd});
    }
}

// arg: { id, title, buttonLabel?, timeoutSeconds?, priority?: "normal"|"high" }
fn cmdShowToast(widget: *gtk.Widget, arg: ?std.json.Value) void {
    const node_id = widgetNodeId(widget) orelse return;
    const obj = argObject(arg) orelse {
        std.debug.print("ND_WARN ToastOverlay showToast: malformed arg (expected {{id, title}})\n", .{});
        return;
    };
    const id = objStr(obj, "id") orelse {
        std.debug.print("ND_WARN ToastOverlay showToast: missing id\n", .{});
        return;
    };
    const title = objStr(obj, "title") orelse "";
    const alloc = std.heap.page_allocator;

    // AdwToast copies title/button-label; the id key is owned by `toasts` and
    // doubles (via its NUL terminator) as the toast's GObject-data id string.
    const title_z = alloc.dupeZ(u8, title) catch return;
    defer alloc.free(title_z);
    const id_z = alloc.dupeZ(u8, id) catch return;

    const toast = adw.Toast.new(title_z);
    if (objStr(obj, "buttonLabel")) |bl| {
        const bl_z = alloc.dupeZ(u8, bl) catch null;
        if (bl_z) |z| {
            defer alloc.free(z);
            adw.Toast.setButtonLabel(toast, z);
        }
    }
    if (objInt(obj, "timeoutSeconds")) |t| {
        if (t >= 0) adw.Toast.setTimeout(toast, @intCast(t)); // 0 = persist until dismissed
    }
    if (objStr(obj, "priority")) |p| {
        adw.Toast.setPriority(toast, if (std.mem.eql(u8, p, "high")) .high else .normal);
    }
    // Keep our own ref past add_toast's transfer-full so the `toasts` pointer
    // stays valid until the dismissed callback drops it. On an OOM put the
    // toast still shows, but without id bookkeeping no events correlate.
    _ = gobject.Object.ref(@ptrCast(@alignCast(toast)));
    toasts.put(alloc, id_z, toast) catch {
        alloc.free(id_z);
        gobject.Object.unref(@ptrCast(@alignCast(toast)));
        const overlay_oom: *adw.ToastOverlay = @ptrCast(@alignCast(widget));
        adw.ToastOverlay.addToast(overlay_oom, toast);
        return;
    };
    gobject.Object.setData(@ptrCast(@alignCast(toast)), TOAST_ID_KEY, @ptrCast(@constCast(id_z.ptr)));

    const data: ?*anyopaque = @ptrFromInt(@as(usize, node_id));
    _ = gobject.signalConnectData(@ptrCast(@alignCast(toast)), "button-clicked", @ptrCast(&cbToastButtonClicked), data, null, .{});
    _ = gobject.signalConnectData(@ptrCast(@alignCast(toast)), "dismissed", @ptrCast(&cbToastDismissed), data, null, .{});

    const overlay: *adw.ToastOverlay = @ptrCast(@alignCast(widget));
    adw.ToastOverlay.addToast(overlay, toast);
}

// arg: { id } (a bare string id is accepted too)
fn cmdDismissToast(arg: ?std.json.Value) void {
    const id: []const u8 = blk: {
        if (argObject(arg)) |obj| {
            if (objStr(obj, "id")) |i| break :blk i;
        }
        if (arg) |a| {
            if (a == .string) break :blk a.string;
        }
        std.debug.print("ND_WARN ToastOverlay dismissToast: malformed arg (expected {{id}})\n", .{});
        return;
    };
    const toast = toasts.get(id) orelse return; // already dismissed: no-op
    adw.Toast.dismiss(toast); // fires "dismissed" -> eviction below
}

fn cbToastButtonClicked(obj: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const toast: *adw.Toast = @ptrCast(@alignCast(obj));
    const id = toastId(toast) orelse return;
    emitWithId(node_id, "toastButtonClicked", id);
}

fn cbToastDismissed(obj: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const toast: *adw.Toast = @ptrCast(@alignCast(obj));
    const id = toastId(toast) orelse return;
    emitWithId(node_id, "toastDismissed", id);
    if (toasts.fetchRemove(id)) |kv| {
        std.heap.page_allocator.free(@as([*:0]const u8, @ptrCast(kv.key.ptr))[0 .. kv.key.len :0]);
        gobject.Object.unref(@ptrCast(@alignCast(kv.value)));
    }
}
