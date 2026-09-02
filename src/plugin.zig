// Context-owned native plugin manager. The C layouts mirror include/nd_plugin.h.
const std = @import("std");
const marker = @import("marker.zig");
const acl = @import("acl.zig");

const NdCommandFn = *const fn ([*:0]const u8, *?[*:0]u8) callconv(.c) i32;
const NdViewImpl = extern struct {
    create: *const fn ([*:0]const u8) callconv(.c) ?*anyopaque,
    apply_props: *const fn (?*anyopaque, [*:0]const u8) callconv(.c) void,
    command: *const fn (?*anyopaque, [*:0]const u8, [*:0]const u8) callconv(.c) void,
    destroy: *const fn (?*anyopaque) callconv(.c) void,
    // ABI v3, append-only. Read only for a v3 plugin.
    connect: ?*const fn (?*anyopaque, u32) callconv(.c) void,
};
const NdPluginRegistry = extern struct {
    host: ?*anyopaque,
    register_command: *const fn (*NdPluginRegistry, [*:0]const u8, NdCommandFn) callconv(.c) void,
    register_view: *const fn (*NdPluginRegistry, [*:0]const u8, *const NdViewImpl) callconv(.c) void,
    // ABI v3, append-only generic event callback.
    emit_event: *const fn (*NdPluginRegistry, u32, [*:0]const u8, [*:0]const u8) callconv(.c) void,
};
const NdPluginV1 = extern struct {
    abi_version: u32,
    name: [*:0]const u8,
    capabilities: [*:null]const ?[*:0]const u8,
    init: *const fn (*NdPluginRegistry) callconv(.c) i32,
    deinit: *const fn () callconv(.c) void,
};
const EntryFn = *const fn () callconv(.c) *const NdPluginV1;

const View = struct { impl: NdViewImpl, owner: *Loaded };
// Heap-allocated (address-stable): `registry.host` points back at it for the
// registration callbacks' lifetime.
const Loaded = struct {
    manager: *Manager,
    lib: std.DynLib,
    plugin: *const NdPluginV1,
    name: []u8,
    commands: std.StringHashMapUnmanaged(NdCommandFn) = .{},
    registry: NdPluginRegistry = undefined,
    loading: bool = true,
    failed: bool = false,
};

pub const Manager = struct {
    gpa: std.mem.Allocator,
    ctx: ?*anyopaque,
    emit: ?*const fn (?*anyopaque, u32, []const u8, []const u8) void,
    plugins: std.StringHashMapUnmanaged(*Loaded) = .{},
    views: std.StringHashMapUnmanaged(View) = .{},
    // Widget pointers actually returned by a view impl's create(). Plugin
    // callbacks route only for these — a backend placeholder (create()
    // returned NULL, or the kind was never registered) must never reach a
    // plugin that doesn't own it. Entries are removed on viewDestroy.
    live_views: std.AutoHashMapUnmanaged(usize, void) = .{},

    pub fn init(gpa: std.mem.Allocator, ctx: ?*anyopaque, emit: ?*const fn (?*anyopaque, u32, []const u8, []const u8) void) Manager {
        return .{ .gpa = gpa, .ctx = ctx, .emit = emit };
    }
    pub fn deinit(self: *Manager) void {
        var vit = self.views.keyIterator();
        while (vit.next()) |k| self.gpa.free(k.*);
        self.views.deinit(self.gpa);
        self.live_views.deinit(self.gpa);
        var it = self.plugins.iterator();
        while (it.next()) |e| destroyLoaded(self, e.value_ptr.*, true, true);
        self.plugins.deinit(self.gpa);
    }

    pub fn load(self: *Manager, path: []const u8, a: *acl.Acl) !void {
        var lib = try std.DynLib.open(path);
        errdefer lib.close();
        const entry = lib.lookup(EntryFn, "nd_plugin_entry") orelse return error.NoPluginEntry;
        try self.loadEntry(lib, entry(), a);
    }

    /// Validation + registration, split from `load` so the ABI/capability
    /// gates are testable in-process (no shared library needed). On error
    /// the caller still owns `lib`.
    fn loadEntry(self: *Manager, lib: std.DynLib, desc: *const NdPluginV1, a: *acl.Acl) !void {
        if (desc.abi_version < 1 or desc.abi_version > 3) {
            marker.print("ND_PLUGIN_ABI_MISMATCH got={d} want=1|2|3\n", .{desc.abi_version});
            return error.AbiMismatch;
        }
        const plugin_name = std.mem.span(desc.name);
        if (plugin_name.len == 0 or self.plugins.contains(plugin_name)) {
            marker.print("ND_PLUGIN_DUPLICATE name={s}\n", .{plugin_name});
            return error.DuplicatePlugin;
        }
        var i: usize = 0;
        while (desc.capabilities[i]) |cap| : (i += 1) {
            if (!a.isAllowed(0, std.mem.span(cap))) {
                marker.print("ND_PLUGIN_CAP_DENIED name={s} cap={s}\n", .{ plugin_name, std.mem.span(cap) });
                return error.CapabilityDenied;
            }
        }

        const loaded = try self.gpa.create(Loaded);
        loaded.* = .{ .manager = self, .lib = lib, .plugin = desc, .name = self.gpa.dupe(u8, plugin_name) catch |err| {
            self.gpa.destroy(loaded);
            return err;
        } };
        loaded.registry = .{ .host = loaded, .register_command = &registerCommandC, .register_view = &registerViewC, .emit_event = &emitEventC };
        // deinit() is owed only after init() returned 0 — a plugin whose
        // init failed has already cleaned up after itself (the pre-rewrite
        // loader's contract; calling deinit anyway risks a double-free).
        var init_ok = false;
        errdefer cleanupUncommitted(self, loaded, init_ok);
        init_ok = desc.init(&loaded.registry) == 0;
        if (!init_ok) {
            marker.print("ND_PLUGIN_INIT_FAILED name={s}\n", .{plugin_name});
            return error.PluginInitFailed;
        }
        if (loaded.failed) {
            marker.print("ND_PLUGIN_DUPLICATE_REGISTRATION name={s}\n", .{plugin_name});
            return error.DuplicateRegistration;
        }
        loaded.loading = false;
        try self.plugins.put(self.gpa, loaded.name, loaded);
        marker.print("ND_PLUGIN_LOADED name={s}\n", .{loaded.name});
    }

    pub fn dispatch(self: *Manager, plugin_name: []const u8, command: []const u8, arg_json: []const u8) ?[]u8 {
        const p = self.plugins.get(plugin_name) orelse return null;
        const handler = p.commands.get(command) orelse return null;
        const argz = self.gpa.dupeZ(u8, arg_json) catch return null;
        defer self.gpa.free(argz);
        var out: ?[*:0]u8 = null;
        if (handler(argz, &out) != 0) return null;
        return std.mem.span(out orelse return null);
    }
    pub fn viewCreate(self: *Manager, kind: []const u8, props: []const u8) ?*anyopaque {
        const v = self.views.get(kind) orelse return null;
        const z = self.gpa.dupeZ(u8, props) catch return null;
        defer self.gpa.free(z);
        const view = v.impl.create(z.ptr) orelse return null;
        self.live_views.put(self.gpa, @intFromPtr(view), {}) catch {
            // An untracked view would never receive apply/destroy — release
            // it now and let the backend fall back to its placeholder.
            v.impl.destroy(view);
            return null;
        };
        return view;
    }
    pub fn viewApplyProps(self: *Manager, kind: []const u8, view: *anyopaque, props: []const u8) void {
        const v = self.views.get(kind) orelse return;
        if (!self.live_views.contains(@intFromPtr(view))) return;
        const z = self.gpa.dupeZ(u8, props) catch return;
        defer self.gpa.free(z);
        v.impl.apply_props(view, z.ptr);
    }
    pub fn viewCommand(self: *Manager, kind: []const u8, view: *anyopaque, command: []const u8, arg: []const u8) void {
        const v = self.views.get(kind) orelse return;
        if (!self.live_views.contains(@intFromPtr(view))) return;
        const cz = self.gpa.dupeZ(u8, command) catch return;
        defer self.gpa.free(cz);
        const az = self.gpa.dupeZ(u8, arg) catch return;
        defer self.gpa.free(az);
        v.impl.command(view, cz.ptr, az.ptr);
    }
    pub fn viewConnect(self: *Manager, kind: []const u8, view: *anyopaque, node_id: u32) void {
        const v = self.views.get(kind) orelse return;
        if (!self.live_views.contains(@intFromPtr(view))) return;
        // registerViewC stores connect=null for pre-v3 plugins, so no
        // version re-check is needed here.
        if (v.impl.connect) |f| f(view, node_id);
    }
    pub fn viewDestroy(self: *Manager, kind: []const u8, view: *anyopaque) void {
        const v = self.views.get(kind) orelse return;
        if (self.live_views.fetchRemove(@intFromPtr(view)) == null) return;
        v.impl.destroy(view);
    }
};

fn hostLoaded(reg: *NdPluginRegistry) *Loaded {
    return @ptrCast(@alignCast(reg.host.?));
}
fn registerCommandC(reg: *NdPluginRegistry, name_z: [*:0]const u8, f: NdCommandFn) callconv(.c) void {
    const l = hostLoaded(reg);
    if (!l.loading) return;
    const name = std.mem.span(name_z);
    if (name.len == 0 or l.commands.contains(name)) {
        l.failed = true;
        return;
    }
    const copy = l.manager.gpa.dupe(u8, name) catch {
        l.failed = true;
        return;
    };
    l.commands.put(l.manager.gpa, copy, f) catch {
        l.manager.gpa.free(copy);
        l.failed = true;
    };
}
fn registerViewC(reg: *NdPluginRegistry, kind_z: [*:0]const u8, impl: *const NdViewImpl) callconv(.c) void {
    const l = hostLoaded(reg);
    if (!l.loading) return;
    const kind = std.mem.span(kind_z);
    if (kind.len == 0 or l.manager.views.contains(kind)) {
        l.failed = true;
        return;
    }
    const copy = l.manager.gpa.dupe(u8, kind) catch {
        l.failed = true;
        return;
    };
    // A v1/v2 struct has no appended connect field: never read it.
    var value: NdViewImpl = undefined;
    value.create = impl.create;
    value.apply_props = impl.apply_props;
    value.command = impl.command;
    value.destroy = impl.destroy;
    value.connect = if (l.plugin.abi_version >= 3) impl.connect else null;
    l.manager.views.put(l.manager.gpa, copy, .{ .impl = value, .owner = l }) catch {
        l.manager.gpa.free(copy);
        l.failed = true;
        return;
    };
    marker.print("ND_PLUGIN_VIEW_REGISTERED view_kind={s}\n", .{kind});
}
fn emitEventC(reg: *NdPluginRegistry, node_id: u32, name: [*:0]const u8, payload: [*:0]const u8) callconv(.c) void {
    const l = hostLoaded(reg);
    if (l.manager.emit) |f| f(l.manager.ctx, node_id, std.mem.span(name), std.mem.span(payload));
}
/// Shared per-Loaded teardown for `Manager.deinit` and `cleanupUncommitted`.
/// `deinit_plugin`: deinit() is owed only after a successful init().
/// `close_lib`: an uncommitted load's DynLib is still owned by `load`'s
/// errdefer, so only the committed (deinit) path closes it here.
fn destroyLoaded(m: *Manager, p: *Loaded, deinit_plugin: bool, close_lib: bool) void {
    if (deinit_plugin) p.plugin.deinit();
    var cit = p.commands.keyIterator();
    while (cit.next()) |k| m.gpa.free(k.*);
    p.commands.deinit(m.gpa);
    if (close_lib) p.lib.close();
    m.gpa.free(p.name);
    m.gpa.destroy(p);
}
fn cleanupUncommitted(m: *Manager, p: *Loaded, init_ok: bool) void {
    var doomed: std.ArrayListUnmanaged([]const u8) = .empty;
    defer doomed.deinit(m.gpa);
    var vit = m.views.iterator();
    while (vit.next()) |e| if (e.value_ptr.owner == p) doomed.append(m.gpa, e.key_ptr.*) catch {};
    for (doomed.items) |key| {
        const removed = m.views.fetchRemove(key).?;
        m.gpa.free(removed.key);
    }
    destroyLoaded(m, p, init_ok, false);
}

// Direct Zig backend compatibility: the generated GTK dispatcher's
// <nativeview> create/apply arms call these; connect/command/destroy flow
// through the retained tree via abi_backend instead.
fn current() *Manager {
    return &@import("abi_backend.zig").ctx.plugins;
}
pub fn viewCreate(k: []const u8, p: []const u8) ?*anyopaque {
    return current().viewCreate(k, p);
}
pub fn viewApplyProps(k: []const u8, v: *anyopaque, p: []const u8) void {
    current().viewApplyProps(k, v, p);
}

var test_connected: u32 = 0;
var test_applied: u32 = 0;
var test_commanded: u32 = 0;
var test_destroyed: u32 = 0;
var test_deinited: u32 = 0;
var test_event_node: u32 = 0;
var test_event_name: []const u8 = "";
var test_event_payload: []const u8 = "";

fn testCreate(_: [*:0]const u8) callconv(.c) ?*anyopaque {
    return @ptrFromInt(1);
}
fn testApply(_: ?*anyopaque, _: [*:0]const u8) callconv(.c) void {
    test_applied += 1;
}
fn testCommand(_: ?*anyopaque, _: [*:0]const u8, _: [*:0]const u8) callconv(.c) void {
    test_commanded += 1;
}
fn testDestroy(_: ?*anyopaque) callconv(.c) void {
    test_destroyed += 1;
}
fn testConnect(_: ?*anyopaque, node_id: u32) callconv(.c) void {
    test_connected = node_id;
}
fn testDeinit() callconv(.c) void {
    test_deinited += 1;
}
fn testInit(_: *NdPluginRegistry) callconv(.c) i32 {
    return 0;
}
fn testInitFail(_: *NdPluginRegistry) callconv(.c) i32 {
    return -1;
}
fn testEmit(_: ?*anyopaque, node_id: u32, name: []const u8, payload: []const u8) void {
    test_event_node = node_id;
    test_event_name = name;
    test_event_payload = payload;
}
const test_caps = [_:null]?[*:0]const u8{null};
const test_plugin_v3 = NdPluginV1{ .abi_version = 3, .name = "test", .capabilities = &test_caps, .init = &testInit, .deinit = &testDeinit };
const test_plugin_v2 = NdPluginV1{ .abi_version = 2, .name = "test-v2", .capabilities = &test_caps, .init = &testInit, .deinit = &testDeinit };

fn testLoaded(manager: *Manager, plugin_desc: *const NdPluginV1) !*Loaded {
    const loaded = try manager.gpa.create(Loaded);
    loaded.* = .{ .manager = manager, .lib = undefined, .plugin = plugin_desc, .name = try manager.gpa.dupe(u8, std.mem.span(plugin_desc.name)) };
    loaded.registry = .{ .host = loaded, .register_command = &registerCommandC, .register_view = &registerViewC, .emit_event = &emitEventC };
    return loaded;
}

test "v3 view registration connects, emits, and destroys exactly once per call" {
    const gpa = std.testing.allocator;
    var manager = Manager.init(gpa, null, &testEmit);
    defer manager.views.deinit(gpa);
    defer manager.live_views.deinit(gpa);
    const loaded = try testLoaded(&manager, &test_plugin_v3);
    defer cleanupUncommitted(&manager, loaded, false);
    var impl = NdViewImpl{ .create = &testCreate, .apply_props = &testApply, .command = &testCommand, .destroy = &testDestroy, .connect = &testConnect };
    registerViewC(&loaded.registry, "app.test", &impl);
    try std.testing.expect(!loaded.failed);

    const view = manager.viewCreate("app.test", "{}").?;
    test_connected = 0;
    manager.viewConnect("app.test", view, 42);
    try std.testing.expectEqual(@as(u32, 42), test_connected);
    emitEventC(&loaded.registry, 42, "pressed", "{\"ok\":true}");
    try std.testing.expectEqual(@as(u32, 42), test_event_node);
    try std.testing.expectEqualStrings("pressed", test_event_name);
    try std.testing.expectEqualStrings("{\"ok\":true}", test_event_payload);
    test_destroyed = 0;
    manager.viewDestroy("app.test", view);
    try std.testing.expectEqual(@as(u32, 1), test_destroyed);
}

test "duplicate view kinds fail without replacing the first owner" {
    const gpa = std.testing.allocator;
    var manager = Manager.init(gpa, null, null);
    defer manager.views.deinit(gpa);
    const first = try testLoaded(&manager, &test_plugin_v3);
    defer cleanupUncommitted(&manager, first, false);
    const second = try testLoaded(&manager, &test_plugin_v3);
    defer cleanupUncommitted(&manager, second, false);
    var impl = NdViewImpl{ .create = &testCreate, .apply_props = &testApply, .command = &testCommand, .destroy = &testDestroy, .connect = &testConnect };
    registerViewC(&first.registry, "app.duplicate", &impl);
    registerViewC(&second.registry, "app.duplicate", &impl);
    try std.testing.expect(!first.failed);
    try std.testing.expect(second.failed);
    try std.testing.expect(manager.views.get("app.duplicate").?.owner == first);
}

test "v2 view registration never enables the appended connect callback" {
    const LegacyViewImpl = extern struct {
        create: *const fn ([*:0]const u8) callconv(.c) ?*anyopaque,
        apply_props: *const fn (?*anyopaque, [*:0]const u8) callconv(.c) void,
        command: *const fn (?*anyopaque, [*:0]const u8, [*:0]const u8) callconv(.c) void,
        destroy: *const fn (?*anyopaque) callconv(.c) void,
    };
    const gpa = std.testing.allocator;
    var manager = Manager.init(gpa, null, null);
    defer manager.views.deinit(gpa);
    defer manager.live_views.deinit(gpa);
    const loaded = try testLoaded(&manager, &test_plugin_v2);
    defer cleanupUncommitted(&manager, loaded, false);
    const legacy = LegacyViewImpl{ .create = &testCreate, .apply_props = &testApply, .command = &testCommand, .destroy = &testDestroy };
    registerViewC(&loaded.registry, "app.legacy", @ptrCast(&legacy));
    const view = manager.viewCreate("app.legacy", "{}").?;
    test_connected = 0;
    manager.viewConnect("app.legacy", view, 99);
    try std.testing.expectEqual(@as(u32, 0), test_connected);
}

test "abi_version outside 1..3 is rejected at load" {
    const gpa = std.testing.allocator;
    var manager = Manager.init(gpa, null, null);
    defer manager.deinit();
    var a = acl.Acl.initDefault(gpa);
    defer a.deinit();
    const v0 = NdPluginV1{ .abi_version = 0, .name = "too-old", .capabilities = &test_caps, .init = &testInit, .deinit = &testDeinit };
    try std.testing.expectError(error.AbiMismatch, manager.loadEntry(undefined, &v0, &a));
    const v4 = NdPluginV1{ .abi_version = 4, .name = "too-new", .capabilities = &test_caps, .init = &testInit, .deinit = &testDeinit };
    try std.testing.expectError(error.AbiMismatch, manager.loadEntry(undefined, &v4, &a));
}

test "ungranted capability denies the load" {
    const gpa = std.testing.allocator;
    var manager = Manager.init(gpa, null, null);
    defer manager.deinit();
    var denied = acl.Acl.initDefault(gpa); // core-only default, no plugin:* grants
    defer denied.deinit();
    const caps = [_:null]?[*:0]const u8{"plugin:test.cap"};
    const desc = NdPluginV1{ .abi_version = 3, .name = "capful", .capabilities = &caps, .init = &testInit, .deinit = &testDeinit };
    try std.testing.expectError(error.CapabilityDenied, manager.loadEntry(undefined, &desc, &denied));
}

test "failed init never gets deinit" {
    const gpa = std.testing.allocator;
    var manager = Manager.init(gpa, null, null);
    defer manager.deinit();
    var a = acl.Acl.initDefault(gpa);
    defer a.deinit();
    const desc = NdPluginV1{ .abi_version = 3, .name = "failing", .capabilities = &test_caps, .init = &testInitFail, .deinit = &testDeinit };
    test_deinited = 0;
    try std.testing.expectError(error.PluginInitFailed, manager.loadEntry(undefined, &desc, &a));
    try std.testing.expectEqual(@as(u32, 0), test_deinited);
}

test "plugin callbacks no-op for pointers the plugin never created" {
    const gpa = std.testing.allocator;
    var manager = Manager.init(gpa, null, null);
    defer manager.views.deinit(gpa);
    defer manager.live_views.deinit(gpa);
    const loaded = try testLoaded(&manager, &test_plugin_v3);
    defer cleanupUncommitted(&manager, loaded, false);
    var impl = NdViewImpl{ .create = &testCreate, .apply_props = &testApply, .command = &testCommand, .destroy = &testDestroy, .connect = &testConnect };
    registerViewC(&loaded.registry, "app.track", &impl);

    // A placeholder the backend substituted (create() returned NULL /
    // unregistered kind) must never reach the plugin's callbacks.
    var foreign: u8 = 0;
    test_applied = 0;
    test_commanded = 0;
    test_connected = 0;
    test_destroyed = 0;
    manager.viewApplyProps("app.track", &foreign, "{}");
    manager.viewCommand("app.track", &foreign, "noop", "{}");
    manager.viewConnect("app.track", &foreign, 7);
    manager.viewDestroy("app.track", &foreign);
    try std.testing.expectEqual(@as(u32, 0), test_applied);
    try std.testing.expectEqual(@as(u32, 0), test_commanded);
    try std.testing.expectEqual(@as(u32, 0), test_connected);
    try std.testing.expectEqual(@as(u32, 0), test_destroyed);

    // A tracked view receives them; destroy untracks, so a second destroy
    // (e.g. remove op followed by a GC sweep) cannot double-free.
    const view = manager.viewCreate("app.track", "{}").?;
    manager.viewApplyProps("app.track", view, "{}");
    try std.testing.expectEqual(@as(u32, 1), test_applied);
    manager.viewDestroy("app.track", view);
    manager.viewDestroy("app.track", view);
    try std.testing.expectEqual(@as(u32, 1), test_destroyed);
}
