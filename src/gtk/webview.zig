// GTK4 surface for the <webview> widget: a real WebKitGTK WebKitWebView when
// libwebkitgtk-6.0 is present at runtime, a placeholder GtkLabel otherwise.
// WebKitGTK is deliberately NOT a link-time dependency (a ~1GB closure,
// soname churn, no headless-CI story inside the weston harness) — the handful
// of C entry points this file needs are resolved once with std.DynLib, so the
// pinned flake and the mac GTK build (brew has no webkitgtk) stay untouched
// and degrade to the placeholder instead of failing to link.
const std = @import("std");
const gtk = @import("gtk");
const gobject = @import("gobject");
const glib = @import("glib");
const protocol = @import("../protocol.zig");

/// Peer of the generated widgets.zig EmitFn (same shape, same protocol module
/// instance) — handed over once by the generated connectEvents WebView arm.
pub const EmitFn = *const fn (node_id: u32, name: []const u8, payload: protocol.EventPayload) void;

const MARKER_KEY = "nd-webview-real";
const NODE_ID_KEY = "nd-webview-node-id";
const PROGRESS_KEY = "nd-webview-last-progress";

// WebKitLoadEvent (WebKitGTK 6.0): STARTED=0, REDIRECTED=1, COMMITTED=2, FINISHED=3.
const WEBKIT_LOAD_FINISHED: c_int = 3;
// WebKitNetworkError (WebKitGTK 6.0, webkit2/WebKitError.h) — the codes we
// filter loadFailed on, so cancelling a navigation doesn't spam apps.
const WEBKIT_NETWORK_ERROR_CANCELLED: c_int = 302;
// WebKitPolicyError.FRAME_LOAD_INTERRUPTED_BY_POLICY_CHANGE — the tail of a
// navigation cancelled by policy (e.g. a response turned into a download); it
// carries the previous page's URI, so surfacing it would fire a bogus
// loadFailed right after downloadRequested. Peer of the AppKit backend
// filtering WebKitErrorDomain 102 in NDWebView.swift.
const WEBKIT_POLICY_ERROR_FRAME_LOAD_INTERRUPTED_BY_POLICY_CHANGE: c_int = 102;

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

// ---- Browser-grade extension surface -------------------------------------
// Every symbol below is looked up independently: a missing one degrades only
// the one feature that needs it (a single ND_WARN at load time), never the
// base surface above. `jsc_value_to_string` lives in libjavascriptcoregtk,
// not libwebkitgtk, so it gets its own fallback dlopen.
const FnGetF64 = *const fn (*anyopaque) callconv(.c) f64;
const FnGetPtr = *const fn (*anyopaque) callconv(.c) ?*anyopaque;
const FnGetCStr = *const fn (*anyopaque) callconv(.c) ?[*:0]const u8;
const FnVoidOnPtr = *const fn (*anyopaque) callconv(.c) void;
const FnSetF64 = *const fn (*anyopaque, f64) callconv(.c) void;
const FnSetCStrOpt = *const fn (*anyopaque, ?[*:0]const u8) callconv(.c) void;
const FnSetBool = *const fn (*anyopaque, c_int) callconv(.c) void;
const FnQuark = *const fn () callconv(.c) glib.Quark;
const FnEvalJsReady = *const fn (?*anyopaque, ?*anyopaque, ?*anyopaque) callconv(.c) void;
const FnEvalJs = *const fn (*anyopaque, [*:0]const u8, isize, ?[*:0]const u8, ?[*:0]const u8, ?*anyopaque, ?FnEvalJsReady, ?*anyopaque) callconv(.c) void;
const FnEvalJsFinish = *const fn (*anyopaque, ?*anyopaque, ?*?*glib.Error) callconv(.c) ?*anyopaque;
const FnJscToString = *const fn (*anyopaque) callconv(.c) ?[*:0]u8;

const ExtApi = struct {
    get_estimated_load_progress: ?FnGetF64 = null,
    navigation_action_get_request: ?FnGetPtr = null,
    uri_request_get_uri: ?FnGetCStr = null,
    get_network_session: ?FnGetPtr = null,
    download_get_request: ?FnGetPtr = null,
    download_cancel: ?FnVoidOnPtr = null,
    network_error_quark: ?FnQuark = null,
    policy_error_quark: ?FnQuark = null,
    evaluate_javascript: ?FnEvalJs = null,
    evaluate_javascript_finish: ?FnEvalJsFinish = null,
    jsc_value_to_string: ?FnJscToString = null,
    set_zoom_level: ?FnSetF64 = null,
    get_settings: ?FnGetPtr = null,
    settings_set_user_agent: ?FnSetCStrOpt = null,
    settings_set_enable_developer_extras: ?FnSetBool = null,
    get_inspector: ?FnGetPtr = null,
    web_inspector_show: ?FnVoidOnPtr = null,
};

var ext: ExtApi = .{};
var ext_loaded = false;
// Secondary lib for jsc_value_to_string when it isn't resolvable through the
// webkitgtk handle; held open for the process lifetime like `webkit_lib`.
var jsc_lib: std.DynLib = undefined;

fn lookupWarn(comptime T: type, lib: *std.DynLib, symbol: [:0]const u8, feature: []const u8) ?T {
    return lib.lookup(T, symbol) orelse {
        std.debug.print("ND_WARN WebView {s} unavailable (missing symbol {s})\n", .{ feature, symbol });
        return null;
    };
}

fn loadJscValueToString() ?FnJscToString {
    const candidates = [_][]const u8{ "libjavascriptcoregtk-6.0.so.1", "libjavascriptcoregtk-6.0.so" };
    for (candidates) |name| {
        var lib = std.DynLib.open(name) catch continue;
        if (lib.lookup(FnJscToString, "jsc_value_to_string")) |f| {
            jsc_lib = lib;
            return f;
        }
        lib.close();
    }
    std.debug.print("ND_WARN WebView executeJavaScript result stringification unavailable (missing symbol jsc_value_to_string)\n", .{});
    return null;
}

fn loadExt(lib: *std.DynLib) void {
    if (ext_loaded) return;
    ext_loaded = true;
    ext.get_estimated_load_progress = lookupWarn(FnGetF64, lib, "webkit_web_view_get_estimated_load_progress", "loadProgress");
    ext.navigation_action_get_request = lookupWarn(FnGetPtr, lib, "webkit_navigation_action_get_request", "newWindow");
    ext.uri_request_get_uri = lookupWarn(FnGetCStr, lib, "webkit_uri_request_get_uri", "newWindow/downloadRequested");
    ext.get_network_session = lookupWarn(FnGetPtr, lib, "webkit_web_view_get_network_session", "downloadRequested");
    ext.download_get_request = lookupWarn(FnGetPtr, lib, "webkit_download_get_request", "downloadRequested");
    ext.download_cancel = lookupWarn(FnVoidOnPtr, lib, "webkit_download_cancel", "downloadRequested");
    ext.network_error_quark = lookupWarn(FnQuark, lib, "webkit_network_error_quark", "loadFailed cancellation filter");
    ext.policy_error_quark = lookupWarn(FnQuark, lib, "webkit_policy_error_quark", "loadFailed policy-interruption filter");
    ext.evaluate_javascript = lookupWarn(FnEvalJs, lib, "webkit_web_view_evaluate_javascript", "executeJavaScript");
    ext.evaluate_javascript_finish = lookupWarn(FnEvalJsFinish, lib, "webkit_web_view_evaluate_javascript_finish", "executeJavaScript");
    ext.jsc_value_to_string = lib.lookup(FnJscToString, "jsc_value_to_string") orelse loadJscValueToString();
    ext.set_zoom_level = lookupWarn(FnSetF64, lib, "webkit_web_view_set_zoom_level", "setZoom");
    ext.get_settings = lookupWarn(FnGetPtr, lib, "webkit_web_view_get_settings", "setUserAgent/openDevTools");
    ext.settings_set_user_agent = lookupWarn(FnSetCStrOpt, lib, "webkit_settings_set_user_agent", "setUserAgent");
    ext.settings_set_enable_developer_extras = lookupWarn(FnSetBool, lib, "webkit_settings_set_enable_developer_extras", "openDevTools");
    ext.get_inspector = lookupWarn(FnGetPtr, lib, "webkit_web_view_get_inspector", "openDevTools");
    ext.web_inspector_show = lookupWarn(FnVoidOnPtr, lib, "webkit_web_inspector_show", "openDevTools");
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
            loadExt(&webkit_lib);
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

fn widgetNodeId(widget: *gtk.Widget) ?u32 {
    const raw = gobject.Object.getData(widget.as(gobject.Object), NODE_ID_KEY) orelse return null;
    return @intCast(@intFromPtr(raw));
}

fn objPutStr(obj: *std.json.ObjectMap, key: []const u8, val: []const u8) void {
    obj.put(std.heap.page_allocator, key, .{ .string = val }) catch {};
}

fn objPutBool(obj: *std.json.ObjectMap, key: []const u8, val: bool) void {
    obj.put(std.heap.page_allocator, key, .{ .bool = val }) catch {};
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

fn cmdSetZoom(v: *anyopaque, arg: ?std.json.Value) void {
    const f = ext.set_zoom_level orelse return;
    const zoom: f64 = switch (arg orelse return) {
        .float => |x| x,
        .integer => |x| @floatFromInt(x),
        else => {
            std.debug.print("ND_WARN WebView setZoom: malformed arg (expected number)\n", .{});
            return;
        },
    };
    f(v, zoom);
}

fn cmdSetUserAgent(v: *anyopaque, arg: ?std.json.Value) void {
    const set_ua = ext.settings_set_user_agent orelse return;
    const get_settings = ext.get_settings orelse return;
    const s: []const u8 = switch (arg orelse return) {
        .string => |x| x,
        else => {
            std.debug.print("ND_WARN WebView setUserAgent: malformed arg (expected string)\n", .{});
            return;
        },
    };
    const settings = get_settings(v) orelse return;
    if (s.len == 0) {
        set_ua(settings, null); // empty string -> restore WebKit's default UA
        return;
    }
    const z = std.heap.page_allocator.dupeZ(u8, s) catch return;
    defer std.heap.page_allocator.free(z);
    set_ua(settings, z.ptr);
}

fn cmdOpenDevTools(v: *anyopaque) void {
    const set_extras = ext.settings_set_enable_developer_extras orelse return;
    const get_settings = ext.get_settings orelse return;
    const get_inspector = ext.get_inspector orelse return;
    const show = ext.web_inspector_show orelse return;
    const settings = get_settings(v) orelse return;
    set_extras(settings, 1);
    const inspector = get_inspector(v) orelse return;
    show(inspector);
}

/// Async completion context for executeJavaScript: the request/response
/// pair is correlated by `id` across the NDP round trip, so it has to
/// survive past the synchronous command call — heap-allocated here, freed
/// in `cbJsEvalReady`.
const JsEvalCtx = struct {
    node_id: u32,
    id: []u8,
};

fn cmdExecuteJavaScript(widget: *gtk.Widget, v: *anyopaque, arg: ?std.json.Value) void {
    const eval = ext.evaluate_javascript orelse return;
    const obj = argObject(arg) orelse {
        std.debug.print("ND_WARN WebView executeJavaScript: malformed arg (expected {{id, code}})\n", .{});
        return;
    };
    const id = objStr(obj, "id") orelse {
        std.debug.print("ND_WARN WebView executeJavaScript: malformed arg (expected {{id, code}})\n", .{});
        return;
    };
    const code = objStr(obj, "code") orelse {
        std.debug.print("ND_WARN WebView executeJavaScript: malformed arg (expected {{id, code}})\n", .{});
        return;
    };
    const node_id = widgetNodeId(widget) orelse return;
    const ctx = std.heap.page_allocator.create(JsEvalCtx) catch return;
    ctx.id = std.heap.page_allocator.dupe(u8, id) catch {
        std.heap.page_allocator.destroy(ctx);
        return;
    };
    ctx.node_id = node_id;
    const code_z = std.heap.page_allocator.dupeZ(u8, code) catch {
        std.heap.page_allocator.free(ctx.id);
        std.heap.page_allocator.destroy(ctx);
        return;
    };
    defer std.heap.page_allocator.free(code_z); // WebKit copies the script synchronously before queuing the eval
    eval(v, code_z.ptr, -1, null, null, null, &cbJsEvalReady, ctx);
}

fn cbJsEvalReady(source: ?*anyopaque, res: ?*anyopaque, user_data: ?*anyopaque) callconv(.c) void {
    const ctx: *JsEvalCtx = @ptrCast(@alignCast(user_data orelse return));
    defer {
        std.heap.page_allocator.free(ctx.id);
        std.heap.page_allocator.destroy(ctx);
    }
    const finish = ext.evaluate_javascript_finish orelse return;
    const f = emit orelse return;

    var err: ?*glib.Error = null;
    const value = finish(source orelse return, res, &err);

    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(std.heap.page_allocator);
    objPutStr(&payload, "id", ctx.id);

    if (value) |jsc_value| {
        defer gobject.Object.unref(@ptrCast(@alignCast(jsc_value)));
        objPutBool(&payload, "ok", true);
        if (ext.jsc_value_to_string) |to_str| {
            if (to_str(jsc_value)) |cstr| {
                defer glib.free(cstr);
                objPutStr(&payload, "value", std.mem.span(cstr));
            }
        }
        f(ctx.node_id, "javaScriptResult", .{ .data = .{ .object = payload } });
        return;
    }

    objPutBool(&payload, "ok", false);
    if (err) |e| {
        defer e.free();
        const msg: []const u8 = if (e.f_message) |m| std.mem.span(m) else "unknown error";
        objPutStr(&payload, "error", msg);
    } else {
        objPutStr(&payload, "error", "unknown error");
    }
    f(ctx.node_id, "javaScriptResult", .{ .data = .{ .object = payload } });
}

/// widgetCommand dispatch (generated widgets.zig WebView arm).
pub fn command(widget: *gtk.Widget, cmd: []const u8, arg: ?std.json.Value) void {
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
    } else if (std.mem.eql(u8, cmd, "setZoom")) {
        cmdSetZoom(v, arg);
    } else if (std.mem.eql(u8, cmd, "setUserAgent")) {
        cmdSetUserAgent(v, arg);
    } else if (std.mem.eql(u8, cmd, "openDevTools")) {
        cmdOpenDevTools(v);
    } else if (std.mem.eql(u8, cmd, "executeJavaScript")) {
        cmdExecuteJavaScript(widget, v, arg);
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
    gobject.Object.setData(obj, NODE_ID_KEY, data); // retrieved by widgetCommand, which gets no node_id of its own
    _ = gobject.signalConnectData(obj, "load-changed", @ptrCast(&cbLoadChanged), data, null, .{});
    _ = gobject.signalConnectData(obj, "notify::uri", @ptrCast(&cbNotifyUri), data, null, .{});
    _ = gobject.signalConnectData(obj, "notify::title", @ptrCast(&cbNotifyTitle), data, null, .{});
    _ = gobject.signalConnectData(obj, "load-failed", @ptrCast(&cbLoadFailed), data, null, .{});
    if (ext.get_estimated_load_progress != null) {
        _ = gobject.signalConnectData(obj, "notify::estimated-load-progress", @ptrCast(&cbNotifyProgress), data, null, .{});
    }
    if (ext.navigation_action_get_request != null and ext.uri_request_get_uri != null) {
        _ = gobject.signalConnectData(obj, "create", @ptrCast(&cbCreate), data, null, .{});
    }
    if (ext.get_network_session != null and ext.download_get_request != null and ext.uri_request_get_uri != null and ext.download_cancel != null) {
        if (ext.get_network_session.?(@ptrCast(widget))) |session| {
            _ = gobject.signalConnectData(@ptrCast(@alignCast(session)), "download-started", @ptrCast(&cbDownloadStarted), data, null, .{});
        }
    }
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

/// Progress is stored bitcast into the widget's GObject data (same idiom as
/// MARKER_KEY) so the handler can skip re-emitting an unchanged value; -1.0
/// (never a real progress value) marks "nothing emitted yet". 64-bit targets
/// only — f64 and usize/pointer are both 8 bytes, which is all this project
/// builds for (macOS/Linux, no 32-bit backend).
fn cbNotifyProgress(obj: *gobject.Object, _: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const get_progress = ext.get_estimated_load_progress orelse return;
    const progress = get_progress(@ptrCast(obj));
    const prev: f64 = if (gobject.Object.getData(obj, PROGRESS_KEY)) |p| @bitCast(@intFromPtr(p)) else -1.0;
    if (progress == prev) return;
    gobject.Object.setData(obj, PROGRESS_KEY, @ptrFromInt(@as(usize, @bitCast(progress))));
    if (emit) |f| f(node_id, "loadProgress", .{ .value = progress });
}

/// `load-failed`: returning FALSE keeps WebKit's built-in error page (apps
/// get the event for their own handling — e.g. a toast — without losing the
/// safety net of a page that isn't just blank).
fn cbLoadFailed(_: *gobject.Object, _: c_int, failing_uri: ?[*:0]const u8, err: ?*glib.Error, data: ?*anyopaque) callconv(.c) c_int {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const e = err orelse return 0;
    if (ext.network_error_quark) |quark_fn| {
        if (e.f_domain == quark_fn() and e.f_code == WEBKIT_NETWORK_ERROR_CANCELLED) return 0;
    }
    if (ext.policy_error_quark) |quark_fn| {
        if (e.f_domain == quark_fn() and e.f_code == WEBKIT_POLICY_ERROR_FRAME_LOAD_INTERRUPTED_BY_POLICY_CHANGE) return 0;
    }
    const f = emit orelse return 0;
    const url: []const u8 = if (failing_uri) |u| std.mem.span(u) else "";
    const msg: []const u8 = if (e.f_message) |m| std.mem.span(m) else "";
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(std.heap.page_allocator);
    objPutStr(&payload, "url", url);
    objPutStr(&payload, "error", msg);
    f(node_id, "loadFailed", .{ .data = .{ .object = payload } });
    return 0;
}

/// `create`: fired for window.open/target=_blank. Returning NULL means no
/// native window is created — the app is expected to open a native tab
/// itself off the emitted URL.
fn cbCreate(_: *gobject.Object, navigation_action: ?*anyopaque, data: ?*anyopaque) callconv(.c) ?*gtk.Widget {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const get_request = ext.navigation_action_get_request orelse return null;
    const get_uri = ext.uri_request_get_uri orelse return null;
    const nav = navigation_action orelse return null;
    const request = get_request(nav) orelse return null;
    const uri = get_uri(request) orelse return null;
    if (emit) |f| f(node_id, "newWindow", .{ .text = std.mem.span(uri) });
    return null;
}

/// `download-started` on the WebKitNetworkSession (WebKitGTK 6 moved
/// downloads off WebKitWebContext): the engine-side download is always
/// cancelled — the Bun app process performs the actual download itself.
fn cbDownloadStarted(_: *gobject.Object, download: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const dl = download orelse return;
    const get_request = ext.download_get_request orelse return;
    const get_uri = ext.uri_request_get_uri orelse return;
    const cancel = ext.download_cancel orelse return;
    if (get_request(dl)) |request| {
        if (get_uri(request)) |uri| {
            if (emit) |f| {
                var payload: std.json.ObjectMap = .empty;
                defer payload.deinit(std.heap.page_allocator);
                objPutStr(&payload, "url", std.mem.span(uri));
                f(node_id, "downloadRequested", .{ .data = .{ .object = payload } });
            }
        }
    }
    cancel(dl);
}
