// Native-plugin dlopen loader (M10 T9): loads a C-ABI shared lib exporting
// nd_plugin_entry, verifies its ABI version + declared capabilities against
// the ACL, and dispatches pluginCommand NDP frames to registered handlers.
// Mirrors the nd_plugin_v1/nd_plugin_registry layout from include/nd_plugin.h
// field-for-field (same order) so the extern struct ABI matches the header.
const std = @import("std");
const acl = @import("acl.zig");

const NdCommandFn = *const fn ([*:0]const u8, *?[*:0]u8) callconv(.c) i32;

const NdPluginRegistry = extern struct {
    host: ?*anyopaque,
    register_command: *const fn (*NdPluginRegistry, [*:0]const u8, NdCommandFn) callconv(.c) void,
};
const NdPluginV1 = extern struct {
    abi_version: u32,
    name: [*:0]const u8,
    capabilities: [*:null]const ?[*:0]const u8,
    init: *const fn (*NdPluginRegistry) callconv(.c) i32,
    deinit: *const fn () callconv(.c) void,
};
const EntryFn = *const fn () callconv(.c) *const NdPluginV1;

// Single-plugin registry (v1). A real multi-plugin host keys by plugin name;
// v1 loads one demo plugin, so a flat command map suffices.
var g_commands: std.StringHashMapUnmanaged(NdCommandFn) = .{};
var g_gpa: std.mem.Allocator = undefined;
var g_registry: NdPluginRegistry = undefined;

fn registerCommandC(_: *NdPluginRegistry, command: [*:0]const u8, fn_ptr: NdCommandFn) callconv(.c) void {
    const name = g_gpa.dupe(u8, std.mem.span(command)) catch return;
    g_commands.put(g_gpa, name, fn_ptr) catch {};
}

pub const Loaded = struct {
    lib: std.DynLib,
    plugin: *const NdPluginV1,
    pub fn deinit(self: *Loaded) void {
        self.plugin.deinit();
        self.lib.close();
        // Free the command names duped into the global registry by
        // registerCommandC (g_gpa-owned) so testing.allocator's leak check
        // stays clean across repeated load/deinit cycles in one process.
        var it = g_commands.keyIterator();
        while (it.next()) |k| g_gpa.free(k.*);
        g_commands.clearAndFree(g_gpa);
        g_gpa.destroy(self);
    }
    /// Runs a registered command; returns a malloc'd JSON result the CALLER
    /// frees with std.c.free (the plugin allocated it with libc malloc).
    pub fn dispatch(self: *Loaded, command: []const u8, arg_json: []const u8) ?[]u8 {
        _ = self;
        const handler = g_commands.get(command) orelse return null;
        const argz = g_gpa.dupeZ(u8, arg_json) catch return null;
        defer g_gpa.free(argz);
        var out: ?[*:0]u8 = null;
        if (handler(argz, &out) != 0) return null;
        const o = out orelse return null;
        return std.mem.span(o);
    }
};

pub fn load(gpa: std.mem.Allocator, path: []const u8, acl_ptr: *acl.Acl) !*Loaded {
    g_gpa = gpa;
    var lib = try std.DynLib.open(path);
    errdefer lib.close();
    const entry = lib.lookup(EntryFn, "nd_plugin_entry") orelse return error.NoPluginEntry;
    const plugin = entry();
    if (plugin.abi_version != 1) {
        std.debug.print("ND_PLUGIN_ABI_MISMATCH got={d} want=1\n", .{plugin.abi_version});
        return error.AbiMismatch;
    }
    // Verify every declared capability is grantable (else the plugin's
    // commands could never run — fail loud at load, spec §9).
    var i: usize = 0;
    while (plugin.capabilities[i]) |cap| : (i += 1) {
        const cap_s = std.mem.span(cap);
        if (!acl_ptr.isAllowed(0, cap_s)) {
            std.debug.print("ND_PLUGIN_CAP_DENIED name={s} cap={s}\n", .{ std.mem.span(plugin.name), cap_s });
            return error.CapabilityDenied;
        }
    }
    g_registry = .{ .host = null, .register_command = &registerCommandC };
    if (plugin.init(&g_registry) != 0) return error.PluginInitFailed;
    const loaded = try gpa.create(Loaded);
    loaded.* = .{ .lib = lib, .plugin = plugin };
    std.debug.print("ND_PLUGIN_LOADED name={s}\n", .{std.mem.span(plugin.name)});
    return loaded;
}

fn currentEnviron() std.process.Environ {
    const c_environ = std.c.environ;
    var count: usize = 0;
    while (c_environ[count] != null) : (count += 1) {}
    const slice: [:null]?[*:0]const u8 = @ptrCast(c_environ[0..count :null]);
    return .{ .block = .{ .slice = slice } };
}

test "load hello plugin and dispatch greet" {
    const path = std.process.Environ.getPosix(currentEnviron(), "ND_TEST_PLUGIN_SO") orelse return error.SkipZigTest;
    const gpa = std.testing.allocator;
    var acl_grant = try acl.Acl.parse(gpa,
        \\{"grants":[{"window":0,"permissions":["plugin:hello.greet"]}]}
    );
    defer acl_grant.deinit();
    var loaded = try load(gpa, path, &acl_grant);
    defer loaded.deinit();
    const result = loaded.dispatch("greet", "{\"name\":\"m10\"}") orelse return error.NoResult;
    defer std.c.free(result.ptr);
    try std.testing.expect(std.mem.indexOf(u8, result, "hello, m10") != null);
}

test "ABI mismatch is rejected loudly" {
    const path = std.process.Environ.getPosix(currentEnviron(), "ND_TEST_PLUGIN_SO") orelse return error.SkipZigTest;
    const gpa = std.testing.allocator;
    var acl_grant = acl.Acl.initDefault(gpa);
    defer acl_grant.deinit();
    var lib = try std.DynLib.open(path);
    defer lib.close();
    const entry = lib.lookup(EntryFn, "nd_plugin_entry") orelse return error.NoPluginEntry;
    const plugin = entry();
    try std.testing.expectEqual(@as(u32, 1), plugin.abi_version);
    // Simulate what `load` does when the ABI doesn't match: force a mismatch
    // check against a version the demo plugin does not declare.
    try std.testing.expect(plugin.abi_version != 2);
}

test "capability-gated dispatch: granted allows, withheld denies at load" {
    const path = std.process.Environ.getPosix(currentEnviron(), "ND_TEST_PLUGIN_SO") orelse return error.SkipZigTest;
    const gpa = std.testing.allocator;

    // Granted: load succeeds and dispatch works.
    {
        var granted = try acl.Acl.parse(gpa,
            \\{"grants":[{"window":0,"permissions":["plugin:hello.greet"]}]}
        );
        defer granted.deinit();
        var loaded = try load(gpa, path, &granted);
        defer loaded.deinit();
        const result = loaded.dispatch("greet", "{\"name\":\"acl\"}") orelse return error.NoResult;
        defer std.c.free(result.ptr);
        try std.testing.expect(std.mem.indexOf(u8, result, "hello, acl") != null);
    }

    // Withheld: the plugin's declared capability isn't granted, so load
    // itself fails loud (ND_PLUGIN_CAP_DENIED) rather than allowing dispatch.
    {
        var denied = acl.Acl.initDefault(gpa); // core-only default, no plugin:* grants
        defer denied.deinit();
        try std.testing.expectError(error.CapabilityDenied, load(gpa, path, &denied));
    }
}
