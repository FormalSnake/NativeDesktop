// colorview — a THIRD-PARTY native-view module (plugin ABI v2). It is a C-ABI
// shared library that dlopen's into nd-hello and registers its OWN native
// widget under the view kind "colorview", exactly like the builtin <terminal>/
// <webview> — but with zero edits to the core schema or codegen. The generic
// <nativeView viewKind="colorview" props='{"color":"#rrggbb"}'/> widget routes
// to the factory registered here.
//
// The view is a GtkDrawingArea whose cairo draw func fills it with the color
// parsed from props — mirroring src/gtk/webview.zig / src/gtk/terminal.zig's
// GtkWidget construction + g_object_set_data state stashing. GTK-first: this
// module ships only a GTK impl (an AppKit impl would return an NSView instead).
const std = @import("std");
const gtk = @import("gtk");
const gobject = @import("gobject");
const cairo = @import("cairo");

// ---- plugin ABI structs, re-declared locally to match include/nd_plugin.h
// (v2) field-for-field. The core copies the nd_view_impl by value at
// register_view time. ----
const NdCommandFn = *const fn ([*:0]const u8, *?[*:0]u8) callconv(.c) i32;

const NdViewImpl = extern struct {
    create: *const fn ([*:0]const u8) callconv(.c) ?*anyopaque,
    apply_props: *const fn (?*anyopaque, [*:0]const u8) callconv(.c) void,
    command: *const fn (?*anyopaque, [*:0]const u8, [*:0]const u8) callconv(.c) void,
    destroy: *const fn (?*anyopaque) callconv(.c) void,
};

const NdPluginRegistry = extern struct {
    host: ?*anyopaque,
    register_command: *const fn (*NdPluginRegistry, [*:0]const u8, NdCommandFn) callconv(.c) void,
    register_view: *const fn (*NdPluginRegistry, [*:0]const u8, *const NdViewImpl) callconv(.c) void,
};

const NdPluginV1 = extern struct {
    abi_version: u32,
    name: [*:0]const u8,
    capabilities: [*:null]const ?[*:0]const u8,
    init: *const fn (*NdPluginRegistry) callconv(.c) i32,
    deinit: *const fn () callconv(.c) void,
};

const STATE_KEY: [*:0]const u8 = "nd-colorview-state";

// Per-widget state (c_allocator: this lives in a long-running shared lib, not
// tied to any arena). Stashed on the DrawingArea for apply_props/unrealize, and
// handed to the draw func as its closure.
const State = struct {
    rgb: [3]f64,
};

fn stateFrom(widget: *gtk.Widget) ?*State {
    const raw = gobject.Object.getData(widget.as(gobject.Object), STATE_KEY) orelse return null;
    return @ptrCast(@alignCast(raw));
}

fn parseHex(s: []const u8) [3]f64 {
    var hex = s;
    if (hex.len > 0 and hex[0] == '#') hex = hex[1..];
    if (hex.len < 6) return .{ 0.5, 0.5, 0.5 };
    const r = std.fmt.parseInt(u8, hex[0..2], 16) catch return .{ 0.5, 0.5, 0.5 };
    const g = std.fmt.parseInt(u8, hex[2..4], 16) catch return .{ 0.5, 0.5, 0.5 };
    const b = std.fmt.parseInt(u8, hex[4..6], 16) catch return .{ 0.5, 0.5, 0.5 };
    return .{
        @as(f64, @floatFromInt(r)) / 255.0,
        @as(f64, @floatFromInt(g)) / 255.0,
        @as(f64, @floatFromInt(b)) / 255.0,
    };
}

/// Parse {"color":"#rrggbb"} defensively; default to mid-gray. c_allocator, not
/// page_allocator: raw mmap/munmap corrupts glibc malloc's arena inside a
/// dlopen'd .so (see plugins/hello/plugin.zig).
fn colorFromProps(props_json: [*:0]const u8) [3]f64 {
    const arg = std.mem.span(props_json);
    const parsed = std.json.parseFromSlice(std.json.Value, std.heap.c_allocator, arg, .{}) catch return .{ 0.5, 0.5, 0.5 };
    defer parsed.deinit();
    if (parsed.value == .object) {
        if (parsed.value.object.get("color")) |c| {
            if (c == .string) return parseHex(c.string);
        }
    }
    return .{ 0.5, 0.5, 0.5 };
}

fn drawCb(_: *gtk.DrawingArea, cr: *cairo.Context, width: c_int, height: c_int, user: ?*anyopaque) callconv(.c) void {
    const state: *State = @ptrCast(@alignCast(user orelse return));
    cairo.Context.setSourceRgb(cr, state.rgb[0], state.rgb[1], state.rgb[2]);
    cairo.Context.rectangle(cr, 0, 0, @floatFromInt(width), @floatFromInt(height));
    cairo.Context.fill(cr);
}

fn onUnrealize(widget: *gtk.Widget, state: *State) callconv(.c) void {
    gobject.Object.setData(widget.as(gobject.Object), STATE_KEY, null);
    std.heap.c_allocator.destroy(state);
}

// ---- nd_view_impl ----

fn viewCreate(props_json: [*:0]const u8) callconv(.c) ?*anyopaque {
    const area = gtk.DrawingArea.new();
    const widget = area.as(gtk.Widget);

    const state = std.heap.c_allocator.create(State) catch return null;
    state.* = .{ .rgb = colorFromProps(props_json) };
    gobject.Object.setData(widget.as(gobject.Object), STATE_KEY, state);

    // A content surface: default intrinsic size + expand so it fills whatever
    // the parent gives it (a 0x0 DrawingArea would be invisible).
    gtk.DrawingArea.setContentWidth(area, 240);
    gtk.DrawingArea.setContentHeight(area, 160);
    gtk.Widget.setHexpand(widget, 1);
    gtk.Widget.setVexpand(widget, 1);
    gtk.DrawingArea.setDrawFunc(area, &drawCb, state, null);

    // GTK-driven teardown (unparent / window close). The nd_view_impl.destroy
    // hook is an unwired follow-up, so unrealize is the single free path.
    _ = gtk.Widget.signals.unrealize.connect(widget, *State, &onUnrealize, state, .{});

    std.debug.print("ND_COLORVIEW_CREATE rgb=({d:.2},{d:.2},{d:.2})\n", .{ state.rgb[0], state.rgb[1], state.rgb[2] });
    return widget;
}

fn viewApplyProps(view: ?*anyopaque, props_json: [*:0]const u8) callconv(.c) void {
    const widget: *gtk.Widget = @ptrCast(@alignCast(view orelse return));
    const state = stateFrom(widget) orelse return;
    state.rgb = colorFromProps(props_json);
    gtk.Widget.queueDraw(widget);
}

fn viewCommand(_: ?*anyopaque, _: [*:0]const u8, _: [*:0]const u8) callconv(.c) void {
    // Command passthrough is a follow-up (the schema declares no NativeView
    // commands yet).
}

fn viewDestroy(_: ?*anyopaque) callconv(.c) void {
    // No-op: teardown happens on the GtkWidget's unrealize signal (see
    // viewCreate). Wiring this hook into the remove path is a follow-up.
}

const impl = NdViewImpl{
    .create = &viewCreate,
    .apply_props = &viewApplyProps,
    .command = &viewCommand,
    .destroy = &viewDestroy,
};

const caps = [_:null]?[*:0]const u8{"plugin:colorview.view"};

fn init(registry: *NdPluginRegistry) callconv(.c) i32 {
    registry.register_view(registry, "colorview", &impl);
    return 0;
}
fn deinit() callconv(.c) void {}

const plugin = NdPluginV1{
    .abi_version = 2,
    .name = "colorview",
    .capabilities = &caps,
    .init = &init,
    .deinit = &deinit,
};

pub export fn nd_plugin_entry() callconv(.c) *const NdPluginV1 {
    return &plugin;
}
