// Context-owned native plugin manager. The C layouts mirror include/nd_plugin.h.
const std = @import("std");
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

const View = struct { impl: NdViewImpl, abi_version: u32, owner: *Loaded };
const Loaded = struct {
    lib: std.DynLib,
    plugin: *const NdPluginV1,
    name: []u8,
    commands: std.StringHashMapUnmanaged(NdCommandFn) = .{},
    registry_host: ?*RegistryHost = null,
    registry: NdPluginRegistry = undefined,
};
const RegistryHost = struct {
    manager: *Manager,
    loaded: *Loaded,
    loading: bool = true,
    failed: bool = false,
};

pub const Manager = struct {
    gpa: std.mem.Allocator,
    ctx: ?*anyopaque,
    emit: ?*const fn (?*anyopaque, u32, []const u8, []const u8) void,
    plugins: std.StringHashMapUnmanaged(*Loaded) = .{},
    views: std.StringHashMapUnmanaged(View) = .{},

    pub fn init(gpa: std.mem.Allocator, ctx: ?*anyopaque, emit: ?*const fn (?*anyopaque, u32, []const u8, []const u8) void) Manager {
        return .{ .gpa = gpa, .ctx = ctx, .emit = emit };
    }
    pub fn deinit(self: *Manager) void {
        var vit = self.views.keyIterator();
        while (vit.next()) |k| self.gpa.free(k.*);
        self.views.deinit(self.gpa);
        var it = self.plugins.iterator();
        while (it.next()) |e| {
            const p = e.value_ptr.*;
            p.plugin.deinit();
            var cit = p.commands.keyIterator();
            while (cit.next()) |k| self.gpa.free(k.*);
            p.commands.deinit(self.gpa);
            p.lib.close();
            if (p.registry_host) |host| self.gpa.destroy(host);
            self.gpa.free(p.name);
            self.gpa.destroy(p);
        }
        self.plugins.deinit(self.gpa);
    }

    pub fn load(self: *Manager, path: []const u8, a: *acl.Acl) !void {
        var lib = try std.DynLib.open(path);
        errdefer lib.close();
        const entry = lib.lookup(EntryFn, "nd_plugin_entry") orelse return error.NoPluginEntry;
        const desc = entry();
        if (desc.abi_version < 1 or desc.abi_version > 3) return error.AbiMismatch;
        const plugin_name = std.mem.span(desc.name);
        if (plugin_name.len == 0 or self.plugins.contains(plugin_name)) return error.DuplicatePlugin;
        var i: usize = 0;
        while (desc.capabilities[i]) |cap| : (i += 1) if (!a.isAllowed(0, std.mem.span(cap))) return error.CapabilityDenied;

        const loaded = try self.gpa.create(Loaded);
        loaded.* = .{ .lib = lib, .plugin = desc, .name = self.gpa.dupe(u8, plugin_name) catch |err| {
            self.gpa.destroy(loaded);
            return err;
        } };
        const registry_host = self.gpa.create(RegistryHost) catch |err| {
            self.gpa.free(loaded.name);
            self.gpa.destroy(loaded);
            return err;
        };
        registry_host.* = .{ .manager = self, .loaded = loaded };
        loaded.registry_host = registry_host;
        loaded.registry = .{ .host = registry_host, .register_command = &registerCommandC, .register_view = &registerViewC, .emit_event = &emitEventC };
        var init_called = false;
        errdefer cleanupUncommitted(self, loaded, init_called);
        init_called = true;
        if (desc.init(&loaded.registry) != 0 or registry_host.failed) {
            if (registry_host.failed) return error.DuplicateRegistration;
            return error.PluginInitFailed;
        }
        registry_host.loading = false;
        try self.plugins.put(self.gpa, loaded.name, loaded);
        std.debug.print("ND_PLUGIN_LOADED name={s}\n", .{loaded.name});
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
        return v.impl.create(z.ptr);
    }
    pub fn viewApplyProps(self: *Manager, kind: []const u8, view: *anyopaque, props: []const u8) void {
        const v = self.views.get(kind) orelse return;
        const z = self.gpa.dupeZ(u8, props) catch return;
        defer self.gpa.free(z);
        v.impl.apply_props(view, z.ptr);
    }
    pub fn viewCommand(self: *Manager, kind: []const u8, view: *anyopaque, command: []const u8, arg: []const u8) void {
        const v = self.views.get(kind) orelse return;
        const cz = self.gpa.dupeZ(u8, command) catch return;
        defer self.gpa.free(cz);
        const az = self.gpa.dupeZ(u8, arg) catch return;
        defer self.gpa.free(az);
        v.impl.command(view, cz.ptr, az.ptr);
    }
    pub fn viewConnect(self: *Manager, kind: []const u8, view: *anyopaque, node_id: u32) void {
        const v = self.views.get(kind) orelse return;
        if (v.abi_version >= 3) if (v.impl.connect) |f| f(view, node_id);
    }
    pub fn viewDestroy(self: *Manager, kind: []const u8, view: *anyopaque) void {
        const v = self.views.get(kind) orelse return;
        v.impl.destroy(view);
    }
};

fn registryHost(reg: *NdPluginRegistry) *RegistryHost {
    return @ptrCast(@alignCast(reg.host.?));
}
fn registerCommandC(reg: *NdPluginRegistry, name_z: [*:0]const u8, f: NdCommandFn) callconv(.c) void {
    const s = registryHost(reg);
    if (!s.loading) return;
    const name = std.mem.span(name_z);
    if (name.len == 0 or s.loaded.commands.contains(name)) {
        s.failed = true;
        return;
    }
    const copy = s.manager.gpa.dupe(u8, name) catch {
        s.failed = true;
        return;
    };
    s.loaded.commands.put(s.manager.gpa, copy, f) catch {
        s.manager.gpa.free(copy);
        s.failed = true;
    };
}
fn registerViewC(reg: *NdPluginRegistry, kind_z: [*:0]const u8, impl: *const NdViewImpl) callconv(.c) void {
    const s = registryHost(reg);
    if (!s.loading) return;
    const kind = std.mem.span(kind_z);
    if (kind.len == 0 or s.manager.views.contains(kind)) {
        s.failed = true;
        return;
    }
    const copy = s.manager.gpa.dupe(u8, kind) catch {
        s.failed = true;
        return;
    };
    // A v1/v2 struct has no appended connect field: never read it.
    var value: NdViewImpl = undefined;
    value.create = impl.create;
    value.apply_props = impl.apply_props;
    value.command = impl.command;
    value.destroy = impl.destroy;
    value.connect = if (s.loaded.plugin.abi_version >= 3) impl.connect else null;
    s.manager.views.put(s.manager.gpa, copy, .{ .impl = value, .abi_version = s.loaded.plugin.abi_version, .owner = s.loaded }) catch {
        s.manager.gpa.free(copy);
        s.failed = true;
    };
}
fn emitEventC(reg: *NdPluginRegistry, node_id: u32, name: [*:0]const u8, payload: [*:0]const u8) callconv(.c) void {
    const s = registryHost(reg);
    if (s.manager.emit) |f| f(s.manager.ctx, node_id, std.mem.span(name), std.mem.span(payload));
}
fn cleanupUncommitted(m: *Manager, p: *Loaded, init_called: bool) void {
    var cit = p.commands.keyIterator();
    while (cit.next()) |k| m.gpa.free(k.*);
    p.commands.deinit(m.gpa);
    var doomed: std.ArrayListUnmanaged([]const u8) = .empty;
    defer doomed.deinit(m.gpa);
    var vit = m.views.iterator();
    while (vit.next()) |e| if (e.value_ptr.owner == p) doomed.append(m.gpa, e.key_ptr.*) catch {};
    for (doomed.items) |key| {
        const removed = m.views.fetchRemove(key).?;
        m.gpa.free(removed.key);
    }
    if (init_called) p.plugin.deinit();
    if (p.registry_host) |host| m.gpa.destroy(host);
    m.gpa.free(p.name);
    m.gpa.destroy(p);
}

// Direct Zig backend compatibility: context is the ABI backend's current context.
fn current() ?*Manager {
    const ab = @import("abi_backend.zig");
    return &ab.ctx.plugins;
}
pub fn viewCreate(k: []const u8, p: []const u8) ?*anyopaque {
    return (current() orelse return null).viewCreate(k, p);
}
pub fn viewApplyProps(k: []const u8, v: *anyopaque, p: []const u8) void {
    (current() orelse return).viewApplyProps(k, v, p);
}
pub fn viewConnect(k: []const u8, v: *anyopaque, node_id: u32) void {
    (current() orelse return).viewConnect(k, v, node_id);
}
pub fn viewCommand(k: []const u8, v: *anyopaque, c: []const u8, a: []const u8) void {
    (current() orelse return).viewCommand(k, v, c, a);
}
pub fn viewDestroy(k: []const u8, v: *anyopaque) void {
    (current() orelse return).viewDestroy(k, v);
}

var test_connected: u32 = 0;
var test_destroyed: u32 = 0;
var test_event_node: u32 = 0;
var test_event_name: []const u8 = "";
var test_event_payload: []const u8 = "";

fn testCreate(_: [*:0]const u8) callconv(.c) ?*anyopaque {
    return @ptrFromInt(1);
}
fn testApply(_: ?*anyopaque, _: [*:0]const u8) callconv(.c) void {}
fn testCommand(_: ?*anyopaque, _: [*:0]const u8, _: [*:0]const u8) callconv(.c) void {}
fn testDestroy(_: ?*anyopaque) callconv(.c) void {
    test_destroyed += 1;
}
fn testConnect(_: ?*anyopaque, node_id: u32) callconv(.c) void {
    test_connected = node_id;
}
fn testDeinit() callconv(.c) void {}
fn testInit(_: *NdPluginRegistry) callconv(.c) i32 {
    return 0;
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
    loaded.* = .{ .lib = undefined, .plugin = plugin_desc, .name = try manager.gpa.dupe(u8, std.mem.span(plugin_desc.name)) };
    const host = try manager.gpa.create(RegistryHost);
    host.* = .{ .manager = manager, .loaded = loaded };
    loaded.registry_host = host;
    loaded.registry = .{ .host = host, .register_command = &registerCommandC, .register_view = &registerViewC, .emit_event = &emitEventC };
    return loaded;
}

test "v3 view registration connects, emits, and destroys exactly once per call" {
    const gpa = std.testing.allocator;
    var manager = Manager.init(gpa, null, &testEmit);
    defer manager.views.deinit(gpa);
    const loaded = try testLoaded(&manager, &test_plugin_v3);
    defer cleanupUncommitted(&manager, loaded, false);
    var impl = NdViewImpl{ .create = &testCreate, .apply_props = &testApply, .command = &testCommand, .destroy = &testDestroy, .connect = &testConnect };
    registerViewC(&loaded.registry, "app.test", &impl);
    try std.testing.expect(!loaded.registry_host.?.failed);

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
    try std.testing.expect(!first.registry_host.?.failed);
    try std.testing.expect(second.registry_host.?.failed);
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
    const loaded = try testLoaded(&manager, &test_plugin_v2);
    defer cleanupUncommitted(&manager, loaded, false);
    const legacy = LegacyViewImpl{ .create = &testCreate, .apply_props = &testApply, .command = &testCommand, .destroy = &testDestroy };
    registerViewC(&loaded.registry, "app.legacy", @ptrCast(&legacy));
    test_connected = 0;
    manager.viewConnect("app.legacy", @ptrFromInt(1), 99);
    try std.testing.expectEqual(@as(u32, 0), test_connected);
}
