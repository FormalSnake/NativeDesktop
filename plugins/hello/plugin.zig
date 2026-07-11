const std = @import("std");

const NdPluginRegistry = extern struct {
    host: ?*anyopaque,
    register_command: *const fn (*NdPluginRegistry, [*:0]const u8, NdCommandFn) callconv(.c) void,
};
const NdCommandFn = *const fn ([*:0]const u8, *?[*:0]u8) callconv(.c) i32;

const NdPluginV1 = extern struct {
    abi_version: u32,
    name: [*:0]const u8,
    capabilities: [*:null]const ?[*:0]const u8,
    init: *const fn (*NdPluginRegistry) callconv(.c) i32,
    deinit: *const fn () callconv(.c) void,
};

const caps = [_:null]?[*:0]const u8{"plugin:hello.greet"};

fn greet(arg_json: [*:0]const u8, result_out: *?[*:0]u8) callconv(.c) i32 {
    const arg = std.mem.span(arg_json);
    // Parse {"name":"..."} defensively; default to "world".
    var name: []const u8 = "world";
    // Use c_allocator (not page_allocator) for every allocation in this shared
    // lib: page_allocator's raw mmap/munmap corrupts glibc malloc's arena when
    // both run inside a dlopen'd .so's smaller address-space carve-out — this
    // segfaulted allocPrintSentinel's c_allocator-backed buffer growth below.
    const parsed = std.json.parseFromSlice(std.json.Value, std.heap.c_allocator, arg, .{}) catch null;
    if (parsed) |p| { defer p.deinit(); if (p.value == .object) if (p.value.object.get("name")) |n| if (n == .string) { name = n.string; }; }
    const out = std.fmt.allocPrintSentinel(std.heap.c_allocator, "{{\"greeting\":\"hello, {s}\"}}", .{name}, 0) catch return -32603;
    result_out.* = out.ptr;
    return 0;
}

fn init(registry: *NdPluginRegistry) callconv(.c) i32 {
    registry.register_command(registry, "greet", &greet);
    return 0;
}
fn deinit() callconv(.c) void {}

const plugin = NdPluginV1{
    .abi_version = 1,
    .name = "hello",
    .capabilities = &caps,
    .init = &init,
    .deinit = &deinit,
};

pub export fn nd_plugin_entry() callconv(.c) *const NdPluginV1 {
    return &plugin;
}
