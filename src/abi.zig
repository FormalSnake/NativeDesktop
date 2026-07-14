const std = @import("std");
const abi_backend = @import("abi_backend.zig");
const protocol = @import("protocol.zig");
const Tree = @import("tree.zig").Tree;
const Runtime = @import("runtime.zig").Runtime;
const automation = @import("automation.zig");
const acl = @import("acl.zig");
const plugin = @import("plugin.zig");

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

    // Embedder UI-thread marshal + host chrome (M6a Task 3). `show_overlay`
    // with an empty message is the clear sentinel (see runtime.zig's
    // `respawn`) rather than a dedicated clear-overlay vtable field.
    marshal_async: *const fn (*NdContext, *const fn (?*anyopaque) callconv(.c) void, ?*anyopaque) callconv(.c) void,
    show_overlay: *const fn (*NdContext, [*:0]const u8) callconv(.c) void,

    node_visible: *const fn (*NdContext, ?*anyopaque) callconv(.c) bool,
    node_bounds: *const fn (*NdContext, ?*anyopaque, *NdRect) callconv(.c) bool,
    snapshot: *const fn (*NdContext, [*:0]const u8) callconv(.c) bool,
    semantic_action: *const fn (*NdContext, ?*anyopaque, u32, [*:0]const u8, [*:0]const u8, *?[*:0]u8, *?[*:0]u8) callconv(.c) i32,

    // App -> widget imperative command (M14, widgetCommand NDP frame). Arrives
    // on the UI thread (runtime.zig marshals before calling).
    widget_command: *const fn (*NdContext, ?*anyopaque, [*:0]const u8, [*:0]const u8, [*:0]const u8) callconv(.c) void,
};

// The core instance: owns the Tree and the Runtime (once nd_start_runtime
// succeeds), and the automation Server (once nd_start_automation succeeds),
// plus the registered vtable. `app` is the opaque embedder-app handle
// (GtkApplication* / NSApplication) threaded straight to `Tree.init` — the
// core never dereferences it (M6a Task 3).
pub const NdContext = struct {
    gpa: std.mem.Allocator,
    vtable: *const NdBackend,
    tree: Tree = undefined,
    runtime: ?*Runtime = null,
    automation_server: ?*automation.Server = null,
    // Set by T10's `nd_set_acl` (pending) before `nd_start_runtime`; null
    // means the embedder never called it, so `runtime.zig`'s commit gate
    // falls back to its own module-level default ACL.
    acl: ?*@import("acl.zig").Acl = null,
    // Context-owned multi-plugin registry.
    plugins: plugin.Manager = undefined,
    shutting_down: bool = false,
};

/// Builds an `Environ`/`Environ.Map` pair from the process's own `environ`
/// global (libc), matching exactly how Zig's own non-WASI process startup
/// populates `std.process.Init` (see std/start.zig's `mainWithoutEnv`) —
/// `nd_init(void)` takes no parameters (the frozen ABI shape), so the core
/// reads its own environment directly rather than threading it in from the
/// embedder.
fn currentEnviron() std.process.Environ {
    const c_environ = std.c.environ;
    var count: usize = 0;
    while (c_environ[count] != null) : (count += 1) {}
    const slice: [:null]?[*:0]const u8 = @ptrCast(c_environ[0..count :null]);
    return .{ .block = .{ .slice = slice } };
}

comptime {
    // 19 function pointers (16 from Task 1 + marshal_async/show_overlay
    // added in Task 3 + widget_command added in M14) + no padding on a
    // 64-bit target.
    std.debug.assert(@sizeOf(NdBackend) == 19 * @sizeOf(usize));
    std.debug.assert(@alignOf(NdBackend) == @alignOf(usize));
    std.debug.assert(@sizeOf(NdRect) == 16);
}

pub export fn nd_init() callconv(.c) ?*NdContext {
    const gpa = std.heap.page_allocator;
    const self = gpa.create(NdContext) catch return null;
    self.* = .{ .gpa = gpa, .vtable = undefined };
    self.plugins = plugin.Manager.init(gpa, self, &pluginEmit);
    return self;
}
pub export fn nd_register_backend(self: *NdContext, vt: *const NdBackend) callconv(.c) void {
    self.vtable = vt;
    abi_backend.bind(self.gpa, self, vt);
}

/// Opens the NDP socket + spawns the bun child (lifted from the old
/// `Runtime.start`, minus the unused GTK `app` param). `nd_init(void)` takes
/// no parameters (the frozen ABI shape), so the environment the child spawn
/// needs (`ND_SCRIPT`/`ND_DEV`/`XDG_RUNTIME_DIR`/PATH for `bun` lookup) comes
/// from the core reading its own process environment via `currentEnviron`,
/// exactly as a plain `main(std.process.Init)` would have received it.
pub export fn nd_start_runtime(self: *NdContext) callconv(.c) i32 {
    const real_environ = currentEnviron();
    // Heap-allocated (not a stack-local `defer`'d at function return):
    // `Runtime.start` stashes this pointer in `self.parent_env` for the
    // Runtime's whole lifetime (`spawnChild`'s dev-mode respawn reads it long
    // after this call returns), so a stack-local here was a dangling-pointer
    // bug — benign until something else's stack/heap churn clobbered the
    // freed frame before a crash-overlay Restart's `respawn()` walked it
    // (`self.parent_env.keys()` panicking with "incorrect alignment" on
    // corrupted MultiArrayList bytes). Never freed: lives for the process.
    const parent_env = self.gpa.create(std.process.Environ.Map) catch return -1;
    parent_env.* = std.process.Environ.createMap(real_environ, self.gpa) catch return -1;

    // `Tree.app` rides opaquely to `backend.createWidget`'s first argument;
    // the abi backend discards it entirely (the embedder's create vtable
    // call carries no app handle — M6a-D2's structural ops are widget/kind/
    // props only), so any non-null placeholder satisfies the `orelse
    // continue` early-exit in `Tree.apply`.
    self.tree = Tree.init(self.gpa, self);

    const rt = Runtime.start(self.gpa, self, &self.tree, parent_env, real_environ) catch return -1;
    self.runtime = rt;
    return 0;
}

/// Opens the automation socket + thread (lifted from `automation.Server.start`).
pub export fn nd_start_automation(self: *NdContext) callconv(.c) i32 {
    const rt = self.runtime orelse return -1;
    const real_environ = currentEnviron();
    var parent_env = std.process.Environ.createMap(real_environ, self.gpa) catch return -1;
    defer parent_env.deinit();
    const runtime_dir = parent_env.get("XDG_RUNTIME_DIR") orelse "/tmp";
    const server = automation.Server.start(self.gpa, rt.getIo(), &self.tree, runtime_dir) catch return -1;
    self.automation_server = server;
    return 0;
}

/// Installs a per-window capability grants manifest (D12). Call before
/// `nd_start_runtime` so `runtime.zig`'s commit gate sees it from the first
/// dispatch. A malformed manifest falls back to the safe default (core UI
/// ops granted, plugin ops denied) rather than failing the call.
pub export fn nd_set_acl(self: *NdContext, grants_json: [*:0]const u8) callconv(.c) void {
    const json = std.mem.span(grants_json);
    const a = self.gpa.create(acl.Acl) catch return;
    a.* = acl.Acl.parse(self.gpa, json) catch acl.Acl.initDefault(self.gpa);
    if (self.acl) |old| {
        old.deinit();
        self.gpa.destroy(old);
    }
    self.acl = a;
}

/// Loads a native `nd_plugin_v1` shared library (opt-in, D12). Lazily
/// installs the default-deny ACL if the embedder never called `nd_set_acl`,
/// so a plugin loaded with no manifest still gets capability-checked rather
/// than running unchecked.
pub export fn nd_load_plugin(self: *NdContext, path: [*:0]const u8) callconv(.c) i32 {
    const p = std.mem.span(path);
    const acl_ptr = self.acl orelse blk: {
        const a = self.gpa.create(acl.Acl) catch return -1;
        a.* = acl.Acl.initDefault(self.gpa);
        self.acl = a;
        break :blk a;
    };
    self.plugins.load(p, acl_ptr) catch return -1;
    return 0;
}

/// Embedder -> core event channel (M6a-D2). `name == "restart"` is a
/// reserved sentinel (M6a Task 3): the crash-overlay Restart button calls
/// this with `node_id=0` instead of a normal NDP event (the child is dead —
/// there is nothing to forward a real event to), so it routes to
/// `Runtime.restart` instead of `Runtime.emitEvent`.
fn pluginEmit(ctx: ?*anyopaque, node_id: u32, name: []const u8, payload_json: []const u8) void {
    const self: *NdContext = @ptrCast(@alignCast(ctx orelse return));
    const parsed = std.json.parseFromSlice(std.json.Value, self.gpa, payload_json, .{}) catch return;
    defer parsed.deinit();
    Runtime.emitEvent(node_id, "nativeEvent", .{ .nativeName = name, .data = parsed.value });
}

pub export fn nd_emit_event(_: *NdContext, node_id: u32, name: [*:0]const u8, payload_json: [*:0]const u8) callconv(.c) void {
    const name_s = std.mem.span(name);
    if (std.mem.eql(u8, name_s, "restart")) {
        Runtime.restart();
        return;
    }
    const payload = parseEventPayload(payload_json);
    Runtime.emitEvent(node_id, name_s, payload);
}

/// Best-effort JSON -> `EventPayload` decode for the embedder->core channel;
/// a malformed/empty payload degrades to the zero payload rather than
/// dropping the event (mirrors the rest of the core's defensive parsing).
fn parseEventPayload(payload_json: [*:0]const u8) protocol.EventPayload {
    const json = std.mem.span(payload_json);
    const parsed = std.json.parseFromSlice(protocol.EventPayload, std.heap.page_allocator, json, .{ .ignore_unknown_fields = true }) catch return .{};
    defer parsed.deinit();
    return parsed.value;
}

/// Terminates the bun child. The embedder calls this from its app-shutdown
/// path (last window closed) so the child dies with the parent rather than
/// being orphaned. Safe to call before `nd_start_runtime` (no-op — no child
/// spawned yet).
pub export fn nd_shutdown(self: *NdContext) callconv(.c) void {
    if (self.shutting_down) return;
    self.shutting_down = true;
    Runtime.stop();
    if (self.runtime != null) self.tree.clearAppNodes();
    self.plugins.deinit();
}

/// Frees a string the embedder allocated and handed back across the ABI
/// (e.g. `semantic_action`'s `result_json_out`/`err_json_out`, M6a Task 4).
/// The convention is the portable-C one: the embedder allocates with
/// `malloc`/`strdup`, the core frees with `free` — this is what makes
/// `nd_free` callable uniformly from a Zig, C, or Swift embedder.
pub export fn nd_plugin_view_create(self: *NdContext, kind: [*:0]const u8, props: [*:0]const u8) callconv(.c) ?*anyopaque {
    return self.plugins.viewCreate(std.mem.span(kind), std.mem.span(props));
}
pub export fn nd_plugin_view_apply_props(self: *NdContext, kind: [*:0]const u8, view: ?*anyopaque, props: [*:0]const u8) callconv(.c) void {
    self.plugins.viewApplyProps(std.mem.span(kind), view orelse return, std.mem.span(props));
}
pub export fn nd_plugin_view_connect(self: *NdContext, kind: [*:0]const u8, view: ?*anyopaque, node_id: u32) callconv(.c) void {
    self.plugins.viewConnect(std.mem.span(kind), view orelse return, node_id);
}
pub export fn nd_plugin_view_command(self: *NdContext, kind: [*:0]const u8, view: ?*anyopaque, command: [*:0]const u8, arg: [*:0]const u8) callconv(.c) void {
    self.plugins.viewCommand(std.mem.span(kind), view orelse return, std.mem.span(command), std.mem.span(arg));
}
pub export fn nd_plugin_view_destroy(self: *NdContext, kind: [*:0]const u8, view: ?*anyopaque) callconv(.c) void {
    self.plugins.viewDestroy(std.mem.span(kind), view orelse return);
}

pub export fn nd_free(p: ?*anyopaque) callconv(.c) void {
    std.c.free(p);
}

test "nd_set_acl parses grants into the context" {
    const self = nd_init().?;
    defer std.heap.page_allocator.destroy(self);
    nd_set_acl(self, "{\"grants\":[{\"window\":0,\"permissions\":[\"plugin:hello.greet\"]}]}");
    try std.testing.expect(self.acl != null);
    try std.testing.expect(self.acl.?.isAllowed(0, "plugin:hello.greet"));
}
