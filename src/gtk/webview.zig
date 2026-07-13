// GTK4 surface for the <webview> widget: a real WebKitGTK WebKitWebView when
// libwebkitgtk-6.0 is present at runtime, a placeholder GtkLabel otherwise.
// WebKitGTK is deliberately NOT a link-time dependency (M5b-D7: ~1GB closure,
// soname churn, no headless-CI story inside the weston harness) — the handful
// of C entry points this file needs are resolved once with std.DynLib, so the
// pinned flake and the mac GTK build (brew has no webkitgtk) stay untouched
// and degrade to the placeholder instead of failing to link.
const std = @import("std");
const gtk = @import("gtk");
const gobject = @import("gobject");
const protocol = @import("../protocol.zig");

/// Peer of the generated widgets.zig EmitFn (same shape, same protocol module
/// instance) — handed over once by the generated connectEvents WebView arm.
pub const EmitFn = *const fn (node_id: u32, name: []const u8, payload: protocol.EventPayload) void;

const MARKER_KEY = "nd-webview-real";

// WebKitLoadEvent (WebKitGTK 6.0): STARTED=0, REDIRECTED=1, COMMITTED=2, FINISHED=3.
const WEBKIT_LOAD_FINISHED: c_int = 3;

const Api = struct {
    web_view_new: *const fn () callconv(.c) *gtk.Widget,
    web_view_load_uri: *const fn (*anyopaque, [*:0]const u8) callconv(.c) void,
    web_view_get_uri: *const fn (*anyopaque) callconv(.c) ?[*:0]const u8,
    web_view_get_title: *const fn (*anyopaque) callconv(.c) ?[*:0]const u8,
    web_view_go_back: *const fn (*anyopaque) callconv(.c) void,
    web_view_go_forward: *const fn (*anyopaque) callconv(.c) void,
    web_view_reload: *const fn (*anyopaque) callconv(.c) void,
    web_view_stop_loading: *const fn (*anyopaque) callconv(.c) void,
    web_view_can_go_back: *const fn (*anyopaque) callconv(.c) c_int,
    web_view_can_go_forward: *const fn (*anyopaque) callconv(.c) c_int,
};

var api: ?Api = null;
var load_attempted = false;
// Held open for the process lifetime — WebKit spawns its own helper process
// tree and cannot be unloaded once a view exists.
var webkit_lib: std.DynLib = undefined;
var emit: ?EmitFn = null;

fn lookupAll(lib: *std.DynLib) ?Api {
    return .{
        .web_view_new = lib.lookup(@FieldType(Api, "web_view_new"), "webkit_web_view_new") orelse return null,
        .web_view_load_uri = lib.lookup(@FieldType(Api, "web_view_load_uri"), "webkit_web_view_load_uri") orelse return null,
        .web_view_get_uri = lib.lookup(@FieldType(Api, "web_view_get_uri"), "webkit_web_view_get_uri") orelse return null,
        .web_view_get_title = lib.lookup(@FieldType(Api, "web_view_get_title"), "webkit_web_view_get_title") orelse return null,
        .web_view_go_back = lib.lookup(@FieldType(Api, "web_view_go_back"), "webkit_web_view_go_back") orelse return null,
        .web_view_go_forward = lib.lookup(@FieldType(Api, "web_view_go_forward"), "webkit_web_view_go_forward") orelse return null,
        .web_view_reload = lib.lookup(@FieldType(Api, "web_view_reload"), "webkit_web_view_reload") orelse return null,
        .web_view_stop_loading = lib.lookup(@FieldType(Api, "web_view_stop_loading"), "webkit_web_view_stop_loading") orelse return null,
        .web_view_can_go_back = lib.lookup(@FieldType(Api, "web_view_can_go_back"), "webkit_web_view_can_go_back") orelse return null,
        .web_view_can_go_forward = lib.lookup(@FieldType(Api, "web_view_can_go_forward"), "webkit_web_view_can_go_forward") orelse return null,
    };
}

fn loadApi() ?*const Api {
    if (load_attempted) return if (api != null) &api.? else null;
    load_attempted = true;
    // The .4 soname is what distros actually ship; the bare .so only exists
    // with -dev packages installed. The dylib name is for a hypothetical mac
    // GTK stack that carries webkitgtk (brew's does not, today).
    const candidates = [_][]const u8{ "libwebkitgtk-6.0.so.4", "libwebkitgtk-6.0.so", "libwebkitgtk-6.0.dylib" };
    for (candidates) |name| {
        var lib = std.DynLib.open(name) catch continue;
        if (lookupAll(&lib)) |a| {
            webkit_lib = lib;
            api = a;
            std.debug.print("ND_WEBVIEW_ENGINE webkitgtk ({s})\n", .{name});
            return &api.?;
        }
        lib.close();
    }
    return null;
}

fn isReal(widget: *gtk.Widget) bool {
    return gobject.Object.getData(widget.as(gobject.Object), MARKER_KEY) != null;
}

pub fn create(url: ?[*:0]const u8) *gtk.Widget {
    const a = loadApi() orelse {
        std.debug.print("ND_WARN WebView unavailable (libwebkitgtk-6.0 not found); rendering placeholder label\n", .{});
        const label = gtk.Label.new("WebView unavailable (webkitgtk not installed)");
        return label.as(gtk.Widget);
    };
    const widget = a.web_view_new();
    gobject.Object.setData(widget.as(gobject.Object), MARKER_KEY, @ptrFromInt(1));
    // A webview is a content surface: expand into whatever the parent gives
    // it (a non-expanding WebKitWebView collapses to 0x0 inside a <box>).
    gtk.Widget.setHexpand(widget, 1);
    gtk.Widget.setVexpand(widget, 1);
    if (url) |u| {
        if (u[0] != 0) a.web_view_load_uri(@ptrCast(widget), u);
    }
    return widget;
}

/// createAndUpdate `url` prop: navigate iff it differs from the current URI
/// (the echo guard — onNavigate feeds the URL back into app state, which
/// re-applies the prop; without the compare every navigation would reload).
pub fn setUrl(widget: *gtk.Widget, url: [:0]const u8) void {
    const a = api orelse return;
    if (!isReal(widget)) return;
    if (url.len == 0) return;
    if (a.web_view_get_uri(@ptrCast(widget))) |cur| {
        if (std.mem.eql(u8, std.mem.span(cur), url)) return;
    }
    a.web_view_load_uri(@ptrCast(widget), url.ptr);
}

/// widgetCommand dispatch (generated widgets.zig WebView arm).
pub fn command(widget: *gtk.Widget, cmd: []const u8, arg: ?std.json.Value) void {
    _ = arg; // no WebView command takes an argument yet
    const a = api orelse return;
    if (!isReal(widget)) return;
    const v: *anyopaque = @ptrCast(widget);
    if (std.mem.eql(u8, cmd, "goBack")) {
        if (a.web_view_can_go_back(v) != 0) a.web_view_go_back(v);
    } else if (std.mem.eql(u8, cmd, "goForward")) {
        if (a.web_view_can_go_forward(v) != 0) a.web_view_go_forward(v);
    } else if (std.mem.eql(u8, cmd, "reload")) {
        a.web_view_reload(v);
    } else if (std.mem.eql(u8, cmd, "stop")) {
        a.web_view_stop_loading(v);
    } else {
        std.debug.print("ND_WARN unknown WebView command {s}\n", .{cmd});
    }
}

/// Generated connectEvents WebView arm: wires the WebKit signals that feed
/// the schema events. `emit_fn` is the generated module's emit sink (same
/// EmitFn shape, installed before any widget exists).
pub fn connectEvents(widget: *gtk.Widget, node_id: u32, emit_fn: EmitFn) void {
    if (api == null or !isReal(widget)) return;
    emit = emit_fn;
    const obj = widget.as(gobject.Object);
    const data: ?*anyopaque = @ptrFromInt(@as(usize, node_id));
    _ = gobject.signalConnectData(obj, "load-changed", @ptrCast(&cbLoadChanged), data, null, .{});
    _ = gobject.signalConnectData(obj, "notify::uri", @ptrCast(&cbNotifyUri), data, null, .{});
    _ = gobject.signalConnectData(obj, "notify::title", @ptrCast(&cbNotifyTitle), data, null, .{});
}

fn cbLoadChanged(obj: *gobject.Object, load_event: c_int, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const a = api orelse return;
    const v: *anyopaque = @ptrCast(obj);
    if (emit) |f| {
        f(node_id, "loadingChanged", .{ .checked = load_event != WEBKIT_LOAD_FINISHED });
        // History availability changes exactly on load transitions.
        f(node_id, "backAvailable", .{ .checked = a.web_view_can_go_back(v) != 0 });
        f(node_id, "forwardAvailable", .{ .checked = a.web_view_can_go_forward(v) != 0 });
    }
}

// notify:: handlers get (object, pspec, user_data).
fn cbNotifyUri(obj: *gobject.Object, _: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const a = api orelse return;
    const uri = a.web_view_get_uri(@ptrCast(obj)) orelse return;
    if (emit) |f| f(node_id, "navigate", .{ .text = std.mem.span(uri) });
}

fn cbNotifyTitle(obj: *gobject.Object, _: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const a = api orelse return;
    const title = a.web_view_get_title(@ptrCast(obj)) orelse return;
    if (emit) |f| f(node_id, "titleChanged", .{ .text = std.mem.span(title) });
}
