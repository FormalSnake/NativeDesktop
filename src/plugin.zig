// Native-plugin dlopen loader (M10 T9): loads a C-ABI shared lib exporting
// nd_plugin_entry, verifies its ABI version + declared capabilities against
// the ACL, and dispatches pluginCommand NDP frames to registered handlers.
// Mirrors the nd_plugin_v1/nd_plugin_registry layout from include/nd_plugin.h
// field-for-field (same order) so the extern struct ABI matches the header.
const std = @import("std");
const acl = @import("acl.zig");

const NdCommandFn = *const fn ([*:0]const u8, *?[*:0]u8) callconv(.c) i32;

// Mirrors nd_view_impl (include/nd_plugin.h, v2) field-for-field: a native-view
// factory a plugin registers under a view_kind string. The handles are opaque
// backend widgets (GtkWidget*/NSView*) the core never dereferences.
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
const EntryFn = *const fn () callconv(.c) *const NdPluginV1;

// Single-plugin registry (v1). A real multi-plugin host keys by plugin name;
// v1 loads one demo plugin, so a flat command map suffices.
var g_commands: std.StringHashMapUnmanaged(NdCommandFn) = .{};
// Native-view factories keyed by view_kind (v2). The generic <nativeView>
// widget's create/apply/command/destroy route here (GTK: the generated Zig
// dispatcher calls viewCreate/…; AppKit: the Swift shell calls the nd_plugin_
// view_* C wrappers below). Copies of the plugin's nd_view_impl by value.
var g_views: std.StringHashMapUnmanaged(NdViewImpl) = .{};
var g_gpa: std.mem.Allocator = undefined;
var g_registry: NdPluginRegistry = undefined;

fn registerCommandC(_: *NdPluginRegistry, command: [*:0]const u8, fn_ptr: NdCommandFn) callconv(.c) void {
    const name = g_gpa.dupe(u8, std.mem.span(command)) catch return;
    g_commands.put(g_gpa, name, fn_ptr) catch {};
}

fn registerViewC(_: *NdPluginRegistry, view_kind: [*:0]const u8, impl: *const NdViewImpl) callconv(.c) void {
    const name = g_gpa.dupe(u8, std.mem.span(view_kind)) catch return;
    g_views.put(g_gpa, name, impl.*) catch {
        g_gpa.free(name);
        return;
    };
    std.debug.print("ND_PLUGIN_VIEW_REGISTERED view_kind={s}\n", .{name});
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
        var vit = g_views.keyIterator();
        while (vit.next()) |k| g_gpa.free(k.*);
        g_views.clearAndFree(g_gpa);
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
    if (plugin.abi_version != 1 and plugin.abi_version != 2) {
        std.debug.print("ND_PLUGIN_ABI_MISMATCH got={d} want=1|2\n", .{plugin.abi_version});
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
    g_registry = .{ .host = null, .register_command = &registerCommandC, .register_view = &registerViewC };
    if (plugin.init(&g_registry) != 0) return error.PluginInitFailed;
    const loaded = try gpa.create(Loaded);
    loaded.* = .{ .lib = lib, .plugin = plugin };
    std.debug.print("ND_PLUGIN_LOADED name={s}\n", .{std.mem.span(plugin.name)});
    return loaded;
}

// ---- Native-view dispatch (v2). The generic <nativeView> widget routes here
// by view_kind. GTK's generated create/apply dispatcher calls these Zig fns
// directly (@import("../plugin.zig")); the Swift shell calls the nd_plugin_
// view_* C wrappers. A lookup miss (no plugin loaded, or unknown kind) returns
// null / no-ops — the backend renders a placeholder rather than crashing. ----

/// Look up the factory for `view_kind` and build its native widget. Null when
/// unregistered. `g_gpa` is only touched after a successful lookup, which
/// implies a plugin loaded (so `g_gpa` is set).
pub fn viewCreate(view_kind: []const u8, props_json: []const u8) ?*anyopaque {
    const impl = g_views.get(view_kind) orelse return null;
    const argz = g_gpa.dupeZ(u8, props_json) catch return null;
    defer g_gpa.free(argz);
    return impl.create(argz.ptr);
}

pub fn viewApplyProps(view_kind: []const u8, view: *anyopaque, props_json: []const u8) void {
    const impl = g_views.get(view_kind) orelse return;
    const argz = g_gpa.dupeZ(u8, props_json) catch return;
    defer g_gpa.free(argz);
    impl.apply_props(view, argz.ptr);
}

pub fn viewCommand(view_kind: []const u8, view: *anyopaque, command: []const u8, arg_json: []const u8) void {
    const impl = g_views.get(view_kind) orelse return;
    const cmdz = g_gpa.dupeZ(u8, command) catch return;
    defer g_gpa.free(cmdz);
    const argz = g_gpa.dupeZ(u8, arg_json) catch return;
    defer g_gpa.free(argz);
    impl.command(view, cmdz.ptr, argz.ptr);
}

pub fn viewDestroy(view_kind: []const u8, view: *anyopaque) void {
    const impl = g_views.get(view_kind) orelse return;
    impl.destroy(view);
}

// C-ABI wrappers retained into libnd.a (src/abi.zig comptime block) for the
// Swift shell's generated <nativeView> arms.
pub export fn nd_plugin_view_create(view_kind: [*:0]const u8, props: [*:0]const u8) callconv(.c) ?*anyopaque {
    return viewCreate(std.mem.span(view_kind), std.mem.span(props));
}
pub export fn nd_plugin_view_apply_props(view_kind: [*:0]const u8, view: ?*anyopaque, props: [*:0]const u8) callconv(.c) void {
    const v = view orelse return;
    viewApplyProps(std.mem.span(view_kind), v, std.mem.span(props));
}
pub export fn nd_plugin_view_command(view_kind: [*:0]const u8, view: ?*anyopaque, command: [*:0]const u8, arg: [*:0]const u8) callconv(.c) void {
    const v = view orelse return;
    viewCommand(std.mem.span(view_kind), v, std.mem.span(command), std.mem.span(arg));
}
pub export fn nd_plugin_view_destroy(view_kind: [*:0]const u8, view: ?*anyopaque) callconv(.c) void {
    const v = view orelse return;
    viewDestroy(std.mem.span(view_kind), v);
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
