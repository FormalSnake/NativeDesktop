const std = @import("std");

// Mirrors include/nd.h exactly. Layout asserts below catch header/Zig drift
// at `zig build test` time — this is the compile-time contract Task 1 pins
// before any code moves onto the ABI.
pub const NdRect = extern struct { x: i32, y: i32, w: i32, h: i32 };

pub const NdBackend = extern struct {
    create: *const fn (*NdContext, [*:0]const u8, [*:0]const u8) callconv(.c) ?*anyopaque,
    apply_props: *const fn (*NdContext, ?*anyopaque, [*:0]const u8, [*:0]const u8) callconv(.c) void,
    append_child: *const fn (*NdContext, ?*anyopaque, [*:0]const u8, ?*anyopaque, [*:0]const u8) callconv(.c) void,
    insert_before: *const fn (*NdContext, ?*anyopaque, [*:0]const u8, ?*anyopaque, ?*anyopaque, [*:0]const u8) callconv(.c) void,
    remove_child: *const fn (*NdContext, ?*anyopaque, [*:0]const u8, ?*anyopaque) callconv(.c) void,
    set_text: *const fn (*NdContext, ?*anyopaque, [*:0]const u8) callconv(.c) void,
    set_visible: *const fn (*NdContext, ?*anyopaque, bool) callconv(.c) void,
    apply_style: *const fn (*NdContext, ?*anyopaque, u32, [*:0]const u8) callconv(.c) void,
    connect_events: *const fn (*NdContext, ?*anyopaque, [*:0]const u8, u32) callconv(.c) void,
    has_parent: *const fn (*NdContext, ?*anyopaque) callconv(.c) bool,
    unparent: *const fn (*NdContext, ?*anyopaque) callconv(.c) void,
    get_window: *const fn (*NdContext) callconv(.c) ?*anyopaque,

    node_visible: *const fn (*NdContext, ?*anyopaque) callconv(.c) bool,
    node_bounds: *const fn (*NdContext, ?*anyopaque, *NdRect) callconv(.c) bool,
    snapshot: *const fn (*NdContext, [*:0]const u8) callconv(.c) bool,
    semantic_action: *const fn (*NdContext, ?*anyopaque, u32, [*:0]const u8, [*:0]const u8, *?[*:0]u8, *?[*:0]u8) callconv(.c) i32,
};

pub const NdContext = opaque {};

comptime {
    // 16 function pointers + no padding on a 64-bit target.
    std.debug.assert(@sizeOf(NdBackend) == 16 * @sizeOf(usize));
    std.debug.assert(@alignOf(NdBackend) == @alignOf(usize));
    std.debug.assert(@sizeOf(NdRect) == 16);
}

// Export stubs so the symbols exist for a header-conformance link check.
// Bodies are filled by later tasks; here they @panic("unimplemented") — the
// point of Task 1 is the SHAPE compiles and the layout is pinned.
export fn nd_init() callconv(.c) ?*NdContext {
    @panic("M6a Task 2");
}
export fn nd_register_backend(_: *NdContext, _: *const NdBackend) callconv(.c) void {
    @panic("M6a Task 2");
}
export fn nd_start_runtime(_: *NdContext) callconv(.c) i32 {
    @panic("M6a Task 3");
}
export fn nd_start_automation(_: *NdContext) callconv(.c) i32 {
    @panic("M6a Task 4");
}
export fn nd_emit_event(_: *NdContext, _: u32, _: [*:0]const u8, _: [*:0]const u8) callconv(.c) void {
    @panic("M6a Task 3");
}
export fn nd_free(_: ?*anyopaque) callconv(.c) void {}
