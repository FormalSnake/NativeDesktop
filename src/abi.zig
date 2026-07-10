const std = @import("std");
const abi_backend = @import("abi_backend.zig");
const protocol = @import("protocol.zig");
const Tree = @import("tree.zig").Tree;
const Runtime = @import("runtime.zig").Runtime;
// NOTE: no `automation.zig` import here yet — it still imports gtk directly
// until Task 4 severs it; deferring this import keeps `abi.zig`'s
// standalone (no-gtk) test module compiling at Task 3. `nd_start_automation`
// stays a stub until Task 4 wires it.

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
};

// The core instance: owns the Tree and the Runtime (once nd_start_runtime
// succeeds), plus the registered vtable. `app` is the opaque embedder-app
// handle (GtkApplication* / NSApplication) threaded straight to `Tree.init`
// — the core never dereferences it (M6a Task 3). The automation Server
// pointer is added in Task 4, once `automation.zig` is GTK-free.
pub const NdContext = struct {
    gpa: std.mem.Allocator,
    vtable: *const NdBackend,
    tree: Tree = undefined,
    runtime: ?*Runtime = null,
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
    // 18 function pointers (16 from Task 1 + marshal_async/show_overlay
    // added in Task 3) + no padding on a 64-bit target.
    std.debug.assert(@sizeOf(NdBackend) == 18 * @sizeOf(usize));
    std.debug.assert(@alignOf(NdBackend) == @alignOf(usize));
    std.debug.assert(@sizeOf(NdRect) == 16);
}

export fn nd_init() callconv(.c) ?*NdContext {
    const gpa = std.heap.page_allocator;
    const self = gpa.create(NdContext) catch return null;
    self.* = .{ .gpa = gpa, .vtable = undefined };
    return self;
}
export fn nd_register_backend(self: *NdContext, vt: *const NdBackend) callconv(.c) void {
    self.vtable = vt;
    abi_backend.bind(self.gpa, self, vt);
}

/// Opens the NDP socket + spawns the bun child (lifted from the old
/// `Runtime.start`, minus the unused GTK `app` param). `nd_init(void)` takes
/// no parameters (the frozen ABI shape), so the environment the child spawn
/// needs (`ND_SCRIPT`/`ND_DEV`/`XDG_RUNTIME_DIR`/PATH for `bun` lookup) comes
/// from the core reading its own process environment via `currentEnviron`,
/// exactly as a plain `main(std.process.Init)` would have received it.
export fn nd_start_runtime(self: *NdContext) callconv(.c) i32 {
    const real_environ = currentEnviron();
    var parent_env = std.process.Environ.createMap(real_environ, self.gpa) catch return -1;
    defer parent_env.deinit();

    // `Tree.app` rides opaquely to `backend.createWidget`'s first argument;
    // the abi backend discards it entirely (the embedder's create vtable
    // call carries no app handle — M6a-D2's structural ops are widget/kind/
    // props only), so any non-null placeholder satisfies the `orelse
    // continue` early-exit in `Tree.apply`.
    self.tree = Tree.init(self.gpa, self);

    const rt = Runtime.start(self.gpa, self, &self.tree, &parent_env, real_environ) catch return -1;
    self.runtime = rt;
    return 0;
}

export fn nd_start_automation(_: *NdContext) callconv(.c) i32 {
    @panic("M6a Task 4");
}

/// Embedder -> core event channel (M6a-D2). `name == "restart"` is a
/// reserved sentinel (M6a Task 3): the crash-overlay Restart button calls
/// this with `node_id=0` instead of a normal NDP event (the child is dead —
/// there is nothing to forward a real event to), so it routes to
/// `Runtime.restart` instead of `Runtime.emitEvent`.
export fn nd_emit_event(_: *NdContext, node_id: u32, name: [*:0]const u8, payload_json: [*:0]const u8) callconv(.c) void {
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

export fn nd_free(_: ?*anyopaque) callconv(.c) void {}
