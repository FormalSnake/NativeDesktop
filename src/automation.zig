const std = @import("std");
const protocol = @import("protocol.zig");
// Method names, params/result shapes, and error codes are GENERATED from
// schema/rpc.json (the single source of truth shared with the TS mirror,
// packages/react/src/generated/rpc.ts) — a method/param/result change there
// regenerates both sides, so drift is a compile error, not a silent break.
const rpc = @import("generated/rpc.zig");
const widget_types = @import("generated/widget_types.zig");
const Tree = @import("tree.zig").Tree;
const Widget = @import("backend.zig").impl.Widget;
const abi = @import("abi.zig");
const abi_backend = @import("abi_backend.zig");

/// A typed view over a request's `params` object plus the `std.json.Parsed`
/// arena that owns its strings/values (kept alive until `deinit`, i.e. for
/// the whole synchronous dispatch of the method).
fn ParsedParams(comptime T: type) type {
    return struct {
        value: T,
        parsed: ?std.json.Parsed(T),
        fn deinit(self: @This()) void {
            if (self.parsed) |p| p.deinit();
        }
    };
}

/// Decodes `params` into the generated schema/rpc.json param struct. Absent
/// `params` decodes as the struct's defaults (all-null), so dispatch can
/// answer each missing required param with the exact "missing params.<x>"
/// message; a type-mismatched param fails the parse (the caller maps that to
/// a generic invalid-params error).
fn parseParams(comptime T: type, gpa: std.mem.Allocator, params: ?std.json.Value) !ParsedParams(T) {
    const v = params orelse return .{ .value = .{}, .parsed = null };
    const parsed = try std.json.parseFromValue(T, gpa, v, .{ .ignore_unknown_fields = true });
    return .{ .value = parsed.value, .parsed = parsed };
}

/// The kinds of work a `UiJob` can carry. `window_action` targets a Window
/// node's handle (pointer/drag/keys input synthesis); `probe_rect`
/// reads one widget's bounds + owning window on the UI thread so the
/// automation thread can resolve drag endpoints.
const JobKind = enum { get_tree, screenshot, click, wait_poll, set_value, type_text, scroll, double_click, right_click, hover, window_action, probe_rect };

/// A request/response handoff between the automation thread and the
/// embedder's UI thread. The mutex/condition here guard only this struct,
/// never the tree — the tree is read exclusively on the UI thread (see
/// `runOnUi`).
const UiJob = struct {
    tree: *Tree,
    kind: JobKind,

    // input (tagged by kind)
    ref: u32 = 0,
    path: ?[:0]const u8 = null,
    text_contains: ?[]const u8 = null, // wait_poll: {"textContains":...}
    ref_visible: ?u32 = null, // wait_poll: {"refVisible":...}
    value: ?std.json.Value = null, // set_value: params.value (string|bool|number per widget kind)
    text: ?[]const u8 = null, // type_text: params.text
    dx: ?f64 = null, // scroll: params.dx
    dy: ?f64 = null, // scroll: params.dy
    window: ?u32 = null, // screenshot/getTree/window_action: target Window node id (null = root/first)
    action: ?[]const u8 = null, // window_action: the semantic_action string ("pointer"|"drag"|"keys")
    args_json: ?[:0]const u8 = null, // window_action: pre-serialized arg_json (owned by dispatch)

    // output (filled on the UI thread by `handleOnUi`)
    result_json: ?[]u8 = null, // owned by gpa; the automation thread frees
    matched: bool = false, // wait_poll only
    rect: abi.NdRect = .{ .x = 0, .y = 0, .w = 0, .h = 0 }, // probe_rect only
    rect_ok: bool = false, // probe_rect only
    window_of: u32 = 0, // probe_rect only: the ref's owning Window node id (0 = not found)
    err_code: i32 = 0,
    err_msg: ?[]const u8 = null,
    err_data_json: ?[]u8 = null, // pre-serialized `data` object, owned by gpa

    gpa: std.mem.Allocator,
    io: std.Io,
    mutex: std.Io.Mutex = .init,
    done: std.Io.Condition = .init,
    finished: bool = false,
};

/// Marshals `job` onto the embedder's UI thread via the registered vtable
/// and blocks the automation thread until the embedder signals completion.
/// This is the sole place automation touches the UI thread; the SLO
/// guarantee (SIGSTOP-the-child still answers) holds because this call never
/// crosses into the Bun child, only into the (fast, main-thread-marshaled)
/// backend vtable.
fn runOnUi(job: *UiJob) void {
    abi_backend.vtable.marshal_async(abi_backend.ctx, &uiCallback, job);
    job.mutex.lockUncancelable(job.io);
    defer job.mutex.unlock(job.io);
    while (!job.finished) job.done.waitUncancelable(job.io, &job.mutex);
}

fn uiCallback(data: ?*anyopaque) callconv(.c) void {
    const job: *UiJob = @ptrCast(@alignCast(data.?));
    handleOnUi(job);
    job.mutex.lockUncancelable(job.io);
    job.finished = true;
    job.done.signal(job.io);
    job.mutex.unlock(job.io);
}

/// Runs on the embedder's UI thread. Fills `result_json` on success, or
/// `err_code`/`err_msg`/`err_data_json` on failure.
fn handleOnUi(job: *UiJob) void {
    switch (job.kind) {
        .get_tree => handleGetTree(job),
        .screenshot => handleScreenshot(job),
        .click => handleSemanticAction(job, "click"),
        .wait_poll => handleWaitPoll(job),
        .set_value => handleSemanticAction(job, "setValue"),
        .type_text => handleSemanticAction(job, "type"),
        .scroll => handleSemanticAction(job, "scroll"),
        .double_click => handleSemanticAction(job, "doubleClick"),
        .right_click => handleSemanticAction(job, "rightClick"),
        .hover => handleSemanticAction(job, "hover"),
        .window_action => handleWindowAction(job),
        .probe_rect => handleProbeRect(job),
    }
}

/// Evaluates one `waitFor` condition against the live tree. Called once per
/// poll from `runOnUi` (each poll is a separate marshaled UI-thread read);
/// the sleep/deadline bookkeeping lives on the automation thread
/// (`dispatchWaitFor`), keeping all tree access UI-thread-only. `visible` is
/// a vtable call — the core never walks a native widget itself.
fn handleWaitPoll(job: *UiJob) void {
    if (job.text_contains) |needle| {
        var it = job.tree.meta.valueIterator();
        while (it.next()) |m| {
            if (m.text) |t| {
                if (std.mem.indexOf(u8, t, needle) != null) {
                    job.matched = true;
                    return;
                }
            }
        }
        job.matched = false;
        return;
    }
    if (job.ref_visible) |ref| {
        const widget = job.tree.get(ref) orelse {
            job.matched = false;
            return;
        };
        job.matched = abi_backend.vtable.node_visible(abi_backend.ctx, widget);
        return;
    }
    job.matched = false;
}

/// Actionability hit-test (exists ∧ visible ∧ non-degenerate bounds), shared
/// by click and the setValue/type/scroll automation actions — never act on
/// what a user couldn't reach (full z-order/overlap testing is deferred).
/// Fills `job.err_*` (-32001) and returns null on failure. "mapped" folds
/// into `node_visible`'s contract: the embedder reports false there for an
/// unmapped widget too.
fn checkActionable(job: *UiJob) ?*Widget {
    const widget = job.tree.get(job.ref) orelse {
        job.err_code = rpc.code_not_actionable;
        job.err_msg = rpc.msg_not_actionable;
        job.err_data_json = std.fmt.allocPrint(job.gpa, "{{\"ref\":{d},\"reason\":\"unknown\"}}", .{job.ref}) catch null;
        return null;
    };
    if (!abi_backend.vtable.node_visible(abi_backend.ctx, widget)) {
        job.err_code = rpc.code_not_actionable;
        job.err_msg = rpc.msg_not_actionable;
        job.err_data_json = std.fmt.allocPrint(job.gpa, "{{\"ref\":{d},\"reason\":\"invisible\"}}", .{job.ref}) catch null;
        return null;
    }
    var rect: abi.NdRect = undefined;
    const has_bounds = abi_backend.vtable.node_bounds(abi_backend.ctx, widget, &rect);
    if (!has_bounds or rect.w <= 0 or rect.h <= 0) {
        job.err_code = rpc.code_not_actionable;
        job.err_msg = rpc.msg_not_actionable;
        job.err_data_json = std.fmt.allocPrint(job.gpa, "{{\"ref\":{d},\"reason\":\"offscreen\"}}", .{job.ref}) catch null;
        return null;
    }
    return widget;
}

/// `std.json.Stringify.valueAlloc` returns a plain `[]u8` (no sentinel);
/// dupe it with a NUL appended for the ABI's `[*:0]const u8` params (mirrors
/// `abi_backend.zig`'s `allocZFromValue`).
fn allocZFromValue(gpa: std.mem.Allocator, v: anytype) [:0]const u8 {
    const json = std.json.Stringify.valueAlloc(gpa, v, .{}) catch return gpa.dupeZ(u8, "{}") catch @panic("OOM in automation allocZFromValue");
    defer gpa.free(json);
    return gpa.dupeZ(u8, json) catch @panic("OOM in automation allocZFromValue");
}

/// Builds the `arg_json` for `vtable.semantic_action` from the job's kind-
/// tagged fields (params cross the ABI as JSON). Caller frees.
fn buildActionArgs(job: *UiJob) [:0]const u8 {
    return switch (job.kind) {
        .set_value => allocZFromValue(job.gpa, .{ .value = job.value orelse .null }),
        .type_text => allocZFromValue(job.gpa, .{ .text = job.text orelse "" }),
        .scroll => allocZFromValue(job.gpa, .{ .dx = job.dx, .dy = job.dy }),
        else => job.gpa.dupeZ(u8, "{}") catch @panic("OOM in automation buildActionArgs"),
    };
}

/// Dispatches click/setValue/type/scroll through `vtable.semantic_action`.
/// Never suppresses the resulting native event — automation actions must
/// flow to React exactly like real user input.
fn handleSemanticAction(job: *UiJob, action: []const u8) void {
    const widget = checkActionable(job) orelse return;
    const action_z = job.gpa.dupeZ(u8, action) catch return;
    defer job.gpa.free(action_z);
    const args_z = buildActionArgs(job);
    defer job.gpa.free(args_z);
    dispatchToBackend(job, widget, job.ref, action_z, args_z);
}

/// Calls `vtable.semantic_action` and maps the outcome into the job —
/// shared by ref-targeted actions (`handleSemanticAction`) and
/// window-targeted input synthesis (`handleWindowAction`).
fn dispatchToBackend(job: *UiJob, widget: *Widget, node_id: u32, action_z: [:0]const u8, args_z: [:0]const u8) void {
    var result_out: ?[*:0]u8 = null;
    var err_out: ?[*:0]u8 = null;
    const code = abi_backend.vtable.semantic_action(abi_backend.ctx, widget, node_id, action_z, args_z, &result_out, &err_out);

    if (code == 0) {
        if (result_out) |r| {
            job.result_json = job.gpa.dupe(u8, std.mem.span(r)) catch null;
            abi.nd_free(r);
        }
        return;
    }
    job.err_code = code;
    job.err_msg = switch (code) {
        rpc.code_input_unsupported => rpc.msg_input_unsupported,
        rpc.code_invalid_params => rpc.msg_invalid_params,
        rpc.code_method_not_found => rpc.msg_method_not_found,
        else => rpc.msg_not_actionable,
    };
    if (err_out) |e| {
        job.err_data_json = job.gpa.dupe(u8, std.mem.span(e)) catch null;
        abi.nd_free(e);
    }
}

/// Input synthesis (pointer/drag/keys) targets a WINDOW node's handle, not a
/// widget ref, and skips `checkActionable` on purpose: a native-chrome
/// window's create-time handle is orphaned once a SplitView takes over
/// (Backend registry resolves it), and the backend hit-tests the coordinates
/// itself exactly like real input would.
fn handleWindowAction(job: *UiJob) void {
    const target_id = job.window orelse job.tree.rootId() orelse {
        job.err_code = rpc.code_internal_error;
        job.err_msg = "no root";
        return;
    };
    const widget = job.tree.get(target_id) orelse {
        job.err_code = rpc.code_invalid_params;
        job.err_msg = "unknown window ref";
        return;
    };
    const action_z = job.gpa.dupeZ(u8, job.action.?) catch return;
    defer job.gpa.free(action_z);
    dispatchToBackend(job, widget, target_id, action_z, job.args_json orelse "{}");
}

/// Reads one actionable widget's bounds and owning Window node id on the UI
/// thread (drag endpoint resolution — the automation thread never touches
/// the tree directly).
fn handleProbeRect(job: *UiJob) void {
    const widget = checkActionable(job) orelse return;
    job.rect_ok = abi_backend.vtable.node_bounds(abi_backend.ctx, widget, &job.rect);
    var cur: u32 = job.ref;
    while (job.tree.metaGet(cur)) |m| {
        if (std.mem.eql(u8, m.widget_type, "Window")) {
            job.window_of = cur;
            break;
        }
        if (m.parent == 0 or m.parent == cur) break;
        cur = m.parent;
    }
}

/// Reads width/height straight from the PNG's IHDR chunk (portable, no GTK):
/// an 8-byte signature, then a 4-byte chunk length, 4-byte "IHDR" tag, then
/// width/height as big-endian u32 — a fixed layout every PNG encoder
/// (including GTK's `gdk.Texture.saveToPng` and any future AppKit encoder)
/// produces identically. `vtable.snapshot` only returns success/failure;
/// the core reads the file it just asked the embedder to write rather than
/// growing the ABI for width/height.
fn readPngDimensions(io: std.Io, path: [:0]const u8) ?struct { w: i32, h: i32 } {
    const file = std.Io.Dir.openFileAbsolute(io, path, .{}) catch return null;
    defer file.close(io);
    var read_buf: [24]u8 = undefined;
    var r = file.reader(io, &read_buf);
    var header: [24]u8 = undefined;
    r.interface.readSliceAll(&header) catch return null;
    if (!std.mem.eql(u8, header[0..8], &.{ 0x89, 'P', 'N', 'G', '\r', '\n', 0x1a, '\n' })) return null;
    if (!std.mem.eql(u8, header[12..16], "IHDR")) return null;
    const w = std.mem.readInt(u32, header[16..20], .big);
    const h = std.mem.readInt(u32, header[20..24], .big);
    return .{ .w = @intCast(w), .h = @intCast(h) };
}

/// Selects which window the following `snapshot` renders (multi-window). The
/// `snapshot` ABI op carries no window handle, so the target is chosen out-of-
/// band here: resolving the target Window node's handle through
/// `resolve_window` (which every backend also uses for reconstruction) records
/// it as the backend's current snapshot target — GTK stashes the window,
/// AppKit caches its live content view. `job.window` picks a specific window;
/// its absence falls back to the root/first window (`rootId`) so a plain
/// screenshot keeps rendering the primary window rather than whichever the
/// single-window global last pointed at. Runs on the UI thread inside the same
/// synchronous marshaled callback as the snapshot call below, so no other
/// window resolution can interleave between selection and render.
fn selectSnapshotWindow(job: *UiJob) void {
    const target_id = job.window orelse job.tree.rootId() orelse return;
    const handle = job.tree.get(target_id) orelse return;
    _ = abi_backend.resolveWindow(handle);
}

/// In-process render of the window to a PNG at `job.path`, via
/// `vtable.snapshot`.
fn handleScreenshot(job: *UiJob) void {
    const path = job.path orelse {
        job.err_code = rpc.code_invalid_params;
        job.err_msg = "missing path";
        return;
    };
    selectSnapshotWindow(job);
    if (!abi_backend.vtable.snapshot(abi_backend.ctx, path)) {
        job.err_code = rpc.code_internal_error;
        job.err_msg = "failed to save png";
        return;
    }
    const dims = readPngDimensions(job.io, path) orelse {
        job.err_code = rpc.code_internal_error;
        job.err_msg = "failed to read png dimensions";
        return;
    };
    const result = rpc.ScreenshotResult{ .path = path, .width = dims.w, .height = dims.h };
    job.result_json = std.json.Stringify.valueAlloc(job.gpa, result, .{}) catch null;
}

/// Builds the nested snapshot on the UI thread. Child order comes from
/// `Tree.childrenOf` (the ordered per-parent sibling list maintained by
/// `apply`'s append/insertBefore/remove handlers), never from grouping over
/// `tree.meta`'s hashmap iteration — that is bucket layout, not insertion
/// order, and silently scrambles sibling order under
/// `insertBefore`/reorders.
fn handleGetTree(job: *UiJob) void {
    const tree = job.tree;
    // params.window scopes the snapshot to that Window node's subtree
    // (validated as a Window kind in dispatch); absent, the root/first window.
    const root_id = job.window orelse tree.rootId() orelse {
        job.err_code = rpc.code_internal_error;
        job.err_msg = "no root";
        return;
    };

    var arena_state = std.heap.ArenaAllocator.init(job.gpa);
    defer arena_state.deinit();
    const arena = arena_state.allocator();

    // Nodes never reached by the ordered `children` lists (host-created
    // overlay chrome: `registerOverlayNode` in gtk/overlay.zig only
    // calls `putMeta` with `parent == 0` — it never records into `Tree`'s
    // ordered list) still need to surface in getTree, matching the
    // live-GTK walk's behaviour (the crash panel IS a real descendant of the
    // window widget there). Collect every id already placed by some
    // parent's ordered list, then attach the rest under root, sorted by id
    // (overlay ids are allocated from a monotonically increasing sequence,
    // so this preserves their creation order).
    var placed: std.AutoHashMapUnmanaged(u32, void) = .empty;
    var children_it = tree.children.iterator();
    while (children_it.next()) |entry| {
        for (entry.value_ptr.items) |child_id| placed.put(arena, child_id, {}) catch {};
    }
    var orphans: std.ArrayList(u32) = .empty;
    // Orphan chrome attaches under the PRIMARY window only — a snapshot
    // explicitly scoped to another window (params.window) stays that
    // window's own subtree.
    if (root_id == tree.rootId()) {
        var meta_it = tree.meta.iterator();
        while (meta_it.next()) |entry| {
            const id = entry.key_ptr.*;
            if (id == root_id) continue;
            if (placed.contains(id)) continue;
            orphans.append(arena, id) catch {};
        }
        std.mem.sort(u32, orphans.items, {}, std.sort.asc(u32));
    }

    if (tree.get(root_id) == null) {
        job.err_code = rpc.code_internal_error;
        job.err_msg = "root widget missing";
        return;
    }
    const root_node = buildNode(arena, tree, id: {
        // Orphans (nodes never recorded into any parent's ordered list,
        // e.g. overlay chrome) attach under root only, appended after the
        // root's own ordered children.
        var root_children: std.ArrayList(u32) = .empty;
        root_children.appendSlice(arena, tree.childrenOf(root_id)) catch {};
        root_children.appendSlice(arena, orphans.items) catch {};
        break :id root_children.items;
    }, root_id) catch {
        job.err_code = rpc.code_internal_error;
        job.err_msg = "failed to build tree";
        return;
    };

    const result = rpc.GetTreeResult{ .root = root_node };
    job.result_json = std.json.Stringify.valueAlloc(job.gpa, result, .{}) catch null;
}

fn buildNode(
    arena: std.mem.Allocator,
    tree: *Tree,
    ordered_children: []const u32,
    id: u32,
) !rpc.JsonNode {
    const meta = tree.metaGet(id);
    const widget_type = if (meta) |m| m.widget_type else "";
    const test_id = if (meta) |m| m.test_id else null;
    const text = if (meta) |m| m.text else null;
    const item_count = if (meta) |m| m.item_count else null;
    const rows: ?[]rpc.RowJson = if (meta) |m| blk: {
        const r = m.rows orelse break :blk null;
        const out = try arena.alloc(rpc.RowJson, r.len);
        for (r, 0..) |row, i| {
            out[i] = .{ .title = row.title, .badge = row.badge, .iconName = row.icon_name };
        }
        break :blk out;
    } else null;
    const widget = tree.get(id);

    const visible = if (widget) |w| abi_backend.vtable.node_visible(abi_backend.ctx, w) else false;

    var rect: abi.NdRect = undefined;
    const has_bounds = if (widget) |w| abi_backend.vtable.node_bounds(abi_backend.ctx, w, &rect) else false;
    const geometry: ?rpc.Geometry = if (has_bounds)
        .{ .x = rect.x, .y = rect.y, .w = rect.w, .h = rect.h }
    else
        null;

    // Accessibility state: the schema-declared role plus a live
    // per-node "a11y" probe through `semantic_action`. Backends without the
    // probe (or non-widget handles like menu nodes) answer nonzero and the
    // fields keep their defaults — the tree never fails over a11y.
    const role = if (widget_type.len > 0) widget_types.roleOf(widget_type) else null;
    var enabled = true;
    var focused = false;
    var value: ?std.json.Value = null;
    if (widget) |w| {
        var res: ?[*:0]u8 = null;
        var err: ?[*:0]u8 = null;
        const code = abi_backend.vtable.semantic_action(abi_backend.ctx, w, id, "a11y", "{}", &res, &err);
        if (err) |e| abi.nd_free(e);
        if (res) |r| {
            defer abi.nd_free(r);
            if (code == 0) {
                // alloc_always: the Value must not alias `r`, which is freed
                // at scope exit while the Value lives until serialization.
                if (std.json.parseFromSliceLeaky(std.json.Value, arena, std.mem.span(r), .{ .allocate = .alloc_always }) catch null) |p| {
                    if (p == .object) {
                        if (p.object.get("enabled")) |v| {
                            if (v == .bool) enabled = v.bool;
                        }
                        if (p.object.get("focused")) |v| {
                            if (v == .bool) focused = v.bool;
                        }
                        if (p.object.get("value")) |v| {
                            if (v != .null) value = v;
                        }
                    }
                }
            }
        }
    }

    var children: std.ArrayList(rpc.JsonNode) = .empty;
    for (ordered_children) |child_id| {
        const child_node = try buildNode(arena, tree, tree.childrenOf(child_id), child_id);
        try children.append(arena, child_node);
    }

    return .{
        .ref = id,
        .type = widget_type,
        .testID = test_id,
        .text = text,
        .visible = visible,
        .geometry = geometry,
        .children = children.items,
        .itemCount = item_count,
        .rows = rows,
        .role = role,
        .enabled = enabled,
        .focused = focused,
        .value = value,
    };
}

pub const Server = struct {
    gpa: std.mem.Allocator,
    io: std.Io,
    tree: *Tree,
    server: std.Io.net.Server,
    sock_path: [:0]u8,

    pub fn start(gpa: std.mem.Allocator, io: std.Io, tree: *Tree, runtime_dir: []const u8) !*Server {
        const self = try gpa.create(Server);
        self.gpa = gpa;
        self.io = io;
        self.tree = tree;

        const pid = std.os.linux.getpid();
        self.sock_path = try std.fmt.allocPrintSentinel(gpa, "{s}/nd-automation-{d}.sock", .{ runtime_dir, pid }, 0);
        std.Io.Dir.deleteFileAbsolute(io, self.sock_path) catch {};

        const addr = try std.Io.net.UnixAddress.init(self.sock_path);
        self.server = try addr.listen(io, .{});

        std.debug.print("ND_AUTOMATION_LISTENING path={s}\n", .{self.sock_path});

        _ = try std.Thread.spawn(.{}, listenLoop, .{self});
        return self;
    }

    fn listenLoop(self: *Server) void {
        while (true) {
            const stream = self.server.accept(self.io) catch break;
            std.debug.print("ND_AUTOMATION_CONNECTED\n", .{});
            serveClient(self, stream);
            std.debug.print("ND_AUTOMATION_DISCONNECTED\n", .{});
        }
    }

    fn serveClient(self: *Server, stream: std.Io.net.Stream) void {
        var read_buf: [64 * 1024]u8 = undefined;
        var r = stream.reader(self.io, &read_buf);
        var write_buf: [64 * 1024]u8 = undefined;
        var w = stream.writer(self.io, &write_buf);

        while (true) {
            const bytes = readFrame(self.gpa, &r.interface) catch return;
            defer self.gpa.free(bytes);
            const response = dispatch(self, bytes) catch |err| blk: {
                std.debug.print("ND_RPC_INTERNAL_ERROR {any}\n", .{err});
                break :blk self.gpa.dupe(u8, "{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32603,\"message\":\"internal error\"}}") catch return;
            };
            defer self.gpa.free(response);
            const frame = frameFromJson(self.gpa, response) catch return;
            defer self.gpa.free(frame);
            w.interface.writeAll(frame) catch return;
            w.interface.flush() catch return;
        }
    }

    /// Parses `{jsonrpc,id,method,params}`, routes on the generated
    /// `rpc.Method` enum with generated typed params, and returns an
    /// already-serialized JSON-RPC response envelope (`gpa`-owned).
    fn dispatch(self: *Server, req_bytes: []const u8) ![]u8 {
        const Req = struct {
            id: std.json.Value = .null,
            method: []const u8,
            params: ?std.json.Value = null,
        };
        const parsed = std.json.parseFromSlice(Req, self.gpa, req_bytes, .{ .ignore_unknown_fields = true }) catch {
            return errorEnvelope(self.gpa, .null, rpc.code_parse_error, rpc.msg_parse_error, null);
        };
        defer parsed.deinit();
        const id = parsed.value.id;
        std.debug.print("ND_RPC method={s} id={any}\n", .{ parsed.value.method, id });
        const method = rpc.methodFromString(parsed.value.method) orelse {
            return errorEnvelope(self.gpa, id, rpc.code_method_not_found, rpc.msg_method_not_found, null);
        };

        switch (method) {
            .getTree => {
                const p = parseParams(rpc.GetTreeParams, self.gpa, parsed.value.params) catch {
                    return errorEnvelope(self.gpa, id, rpc.code_invalid_params, rpc.msg_invalid_params, null);
                };
                defer p.deinit();
                if (p.value.window) |window_ref| {
                    if (self.windowRefError(id, window_ref)) |env| return env;
                }
                var job = UiJob{ .tree = self.tree, .kind = .get_tree, .gpa = self.gpa, .io = self.io, .window = p.value.window };
                return self.runJobAndEnvelope(&job, id);
            },
            .screenshot => {
                const p = parseParams(rpc.ScreenshotParams, self.gpa, parsed.value.params) catch {
                    return errorEnvelope(self.gpa, id, rpc.code_invalid_params, rpc.msg_invalid_params, null);
                };
                defer p.deinit();
                const path = p.value.path orelse {
                    return errorEnvelope(self.gpa, id, rpc.code_invalid_params, "missing params.path", null);
                };
                if (p.value.window) |window_ref| {
                    if (self.windowRefError(id, window_ref)) |env| return env;
                }
                const path_z = self.gpa.dupeZ(u8, path) catch return error.OutOfMemory;
                defer self.gpa.free(path_z);
                var job = UiJob{ .tree = self.tree, .kind = .screenshot, .gpa = self.gpa, .io = self.io, .path = path_z, .window = p.value.window };
                return self.runJobAndEnvelope(&job, id);
            },
            .click => {
                const p = parseParams(rpc.ClickParams, self.gpa, parsed.value.params) catch {
                    return errorEnvelope(self.gpa, id, rpc.code_invalid_params, rpc.msg_invalid_params, null);
                };
                defer p.deinit();
                const ref = p.value.ref orelse {
                    return errorEnvelope(self.gpa, id, rpc.code_invalid_params, "missing params.ref", null);
                };
                var job = UiJob{ .tree = self.tree, .kind = .click, .gpa = self.gpa, .io = self.io, .ref = ref };
                return self.runJobAndEnvelope(&job, id);
            },
            .waitFor => return self.dispatchWaitFor(id, parsed.value.params),
            .setValue => {
                const p = parseParams(rpc.SetValueParams, self.gpa, parsed.value.params) catch {
                    return errorEnvelope(self.gpa, id, rpc.code_invalid_params, rpc.msg_invalid_params, null);
                };
                defer p.deinit();
                const ref = p.value.ref orelse return errorEnvelope(self.gpa, id, rpc.code_invalid_params, "missing params.ref", null);
                const value = p.value.value orelse return errorEnvelope(self.gpa, id, rpc.code_invalid_params, "missing params.value", null);
                var job = UiJob{ .tree = self.tree, .kind = .set_value, .gpa = self.gpa, .io = self.io, .ref = ref, .value = value };
                return self.runJobAndEnvelope(&job, id);
            },
            .@"type" => {
                const p = parseParams(rpc.TypeParams, self.gpa, parsed.value.params) catch {
                    return errorEnvelope(self.gpa, id, rpc.code_invalid_params, rpc.msg_invalid_params, null);
                };
                defer p.deinit();
                const ref = p.value.ref orelse return errorEnvelope(self.gpa, id, rpc.code_invalid_params, "missing params.ref", null);
                const text = p.value.text orelse return errorEnvelope(self.gpa, id, rpc.code_invalid_params, "missing params.text", null);
                var job = UiJob{ .tree = self.tree, .kind = .type_text, .gpa = self.gpa, .io = self.io, .ref = ref, .text = text };
                return self.runJobAndEnvelope(&job, id);
            },
            .scroll => {
                const p = parseParams(rpc.ScrollParams, self.gpa, parsed.value.params) catch {
                    return errorEnvelope(self.gpa, id, rpc.code_invalid_params, rpc.msg_invalid_params, null);
                };
                defer p.deinit();
                const ref = p.value.ref orelse return errorEnvelope(self.gpa, id, rpc.code_invalid_params, "missing params.ref", null);
                var job = UiJob{ .tree = self.tree, .kind = .scroll, .gpa = self.gpa, .io = self.io, .ref = ref, .dx = p.value.dx, .dy = p.value.dy };
                return self.runJobAndEnvelope(&job, id);
            },
            .doubleClick, .rightClick, .hover => {
                const p = parseParams(rpc.ClickParams, self.gpa, parsed.value.params) catch {
                    return errorEnvelope(self.gpa, id, rpc.code_invalid_params, rpc.msg_invalid_params, null);
                };
                defer p.deinit();
                const ref = p.value.ref orelse {
                    return errorEnvelope(self.gpa, id, rpc.code_invalid_params, "missing params.ref", null);
                };
                const kind: JobKind = switch (method) {
                    .doubleClick => .double_click,
                    .rightClick => .right_click,
                    else => .hover,
                };
                var job = UiJob{ .tree = self.tree, .kind = kind, .gpa = self.gpa, .io = self.io, .ref = ref };
                return self.runJobAndEnvelope(&job, id);
            },
            .pointer => {
                const p = parseParams(rpc.PointerParams, self.gpa, parsed.value.params) catch {
                    return errorEnvelope(self.gpa, id, rpc.code_invalid_params, rpc.msg_invalid_params, null);
                };
                defer p.deinit();
                const phase = p.value.phase orelse return errorEnvelope(self.gpa, id, rpc.code_invalid_params, "missing params.phase", null);
                const x = p.value.x orelse return errorEnvelope(self.gpa, id, rpc.code_invalid_params, "missing params.x", null);
                const y = p.value.y orelse return errorEnvelope(self.gpa, id, rpc.code_invalid_params, "missing params.y", null);
                if (!std.mem.eql(u8, phase, "down") and !std.mem.eql(u8, phase, "move") and !std.mem.eql(u8, phase, "up")) {
                    return errorEnvelope(self.gpa, id, rpc.code_invalid_params, "params.phase must be down|move|up", null);
                }
                if (p.value.window) |window_ref| {
                    if (self.windowRefError(id, window_ref)) |env| return env;
                }
                const args = allocZFromValue(self.gpa, .{ .phase = phase, .x = x, .y = y, .button = p.value.button orelse "left", .clickCount = p.value.clickCount orelse 1 });
                defer self.gpa.free(args);
                var job = UiJob{ .tree = self.tree, .kind = .window_action, .gpa = self.gpa, .io = self.io, .window = p.value.window, .action = "pointer", .args_json = args };
                return self.runJobAndEnvelope(&job, id);
            },
            .keys => {
                const p = parseParams(rpc.KeysParams, self.gpa, parsed.value.params) catch {
                    return errorEnvelope(self.gpa, id, rpc.code_invalid_params, rpc.msg_invalid_params, null);
                };
                defer p.deinit();
                const keys_spec = p.value.keys orelse return errorEnvelope(self.gpa, id, rpc.code_invalid_params, "missing params.keys", null);
                if (p.value.window) |window_ref| {
                    if (self.windowRefError(id, window_ref)) |env| return env;
                }
                const args = allocZFromValue(self.gpa, .{ .keys = keys_spec });
                defer self.gpa.free(args);
                var job = UiJob{ .tree = self.tree, .kind = .window_action, .gpa = self.gpa, .io = self.io, .window = p.value.window, .action = "keys", .args_json = args };
                return self.runJobAndEnvelope(&job, id);
            },
            .drag => return self.dispatchDrag(id, parsed.value.params),
        }
    }

    /// Validates a params.window ref as a live Window node; returns the
    /// error envelope to send, or null when the ref is valid. Any Window
    /// node is a valid target (multi-window), not just the first —
    /// `metaGet` is the tree's window registry.
    fn windowRefError(self: *Server, id: std.json.Value, window_ref: u32) ?[]u8 {
        const m = self.tree.metaGet(window_ref);
        if (m == null or !std.mem.eql(u8, m.?.widget_type, "Window")) {
            return errorEnvelope(self.gpa, id, rpc.code_invalid_params, "unknown window ref", null) catch null;
        }
        return null;
    }

    /// One resolved drag endpoint: window-topleft coordinates plus the
    /// owning Window node id when the endpoint came from a widget ref.
    const DragEndpoint = struct { x: f64, y: f64, window: ?u32 };

    /// Resolves fromRef/toRef (widget center via a UI-thread bounds probe)
    /// or explicit coordinates. Returns null after already writing the
    /// error envelope into `out_env`.
    fn resolveDragEndpoint(self: *Server, id: std.json.Value, ref: ?u32, x: ?f64, y: ?f64, which: []const u8, out_env: *?[]u8) ?DragEndpoint {
        if (ref) |r| {
            var probe = UiJob{ .tree = self.tree, .kind = .probe_rect, .gpa = self.gpa, .io = self.io, .ref = r };
            runOnUi(&probe);
            defer if (probe.err_data_json) |d| self.gpa.free(d);
            if (!probe.rect_ok) {
                out_env.* = errorEnvelope(self.gpa, id, probe.err_code, probe.err_msg orelse rpc.msg_not_actionable, probe.err_data_json) catch null;
                return null;
            }
            return .{
                .x = @as(f64, @floatFromInt(probe.rect.x)) + @as(f64, @floatFromInt(probe.rect.w)) / 2.0,
                .y = @as(f64, @floatFromInt(probe.rect.y)) + @as(f64, @floatFromInt(probe.rect.h)) / 2.0,
                .window = if (probe.window_of != 0) probe.window_of else null,
            };
        }
        if (x != null and y != null) return .{ .x = x.?, .y = y.?, .window = null };
        const msg = if (std.mem.eql(u8, which, "from")) "drag needs fromRef or fromX/fromY" else "drag needs toRef or toX/toY";
        out_env.* = errorEnvelope(self.gpa, id, rpc.code_invalid_params, msg, null) catch null;
        return null;
    }

    /// drag — resolves both endpoints, then hands the WHOLE press-move-
    /// release sequence to the backend as one `semantic_action("drag")` on
    /// the window handle. One batch, not per-phase marshals: AppKit controls
    /// run nested mouse-tracking loops inside `mouseDown` dispatch that
    /// block the main thread until the matching up-event arrives, so the
    /// full event sequence must already sit in the app's event queue.
    fn dispatchDrag(self: *Server, id: std.json.Value, params: ?std.json.Value) ![]u8 {
        const p = parseParams(rpc.DragParams, self.gpa, params) catch {
            return errorEnvelope(self.gpa, id, rpc.code_invalid_params, rpc.msg_invalid_params, null);
        };
        defer p.deinit();

        var env: ?[]u8 = null;
        const from = self.resolveDragEndpoint(id, p.value.fromRef, p.value.fromX, p.value.fromY, "from", &env) orelse
            return env orelse error.OutOfMemory;
        const to = self.resolveDragEndpoint(id, p.value.toRef, p.value.toX, p.value.toY, "to", &env) orelse
            return env orelse error.OutOfMemory;
        if (from.window != null and to.window != null and from.window.? != to.window.?) {
            return errorEnvelope(self.gpa, id, rpc.code_invalid_params, "fromRef and toRef must share a window", null);
        }
        var window_id: ?u32 = from.window orelse to.window orelse p.value.window;
        if (p.value.window) |window_ref| {
            if (from.window == null and to.window == null) {
                if (self.windowRefError(id, window_ref)) |e| return e;
                window_id = window_ref;
            }
        }

        const steps = @max(p.value.steps, 1);
        const args = allocZFromValue(self.gpa, .{
            .fromX = from.x,
            .fromY = from.y,
            .toX = to.x,
            .toY = to.y,
            .steps = steps,
            .durationMs = p.value.durationMs,
            .button = p.value.button orelse "left",
        });
        defer self.gpa.free(args);
        var job = UiJob{ .tree = self.tree, .kind = .window_action, .gpa = self.gpa, .io = self.io, .window = window_id, .action = "drag", .args_json = args };
        return self.runJobAndEnvelope(&job, id);
    }

    /// Polls the tree on the UI thread at ~50ms until the condition holds or
    /// `timeoutMs` elapses. Each poll is a separate marshaled UI-thread read
    /// (`handleWaitPoll`); the sleep/deadline live here on the automation
    /// thread so tree access stays UI-thread-only.
    fn dispatchWaitFor(self: *Server, id: std.json.Value, params: ?std.json.Value) ![]u8 {
        const p = parseParams(rpc.WaitForParams, self.gpa, params) catch {
            return errorEnvelope(self.gpa, id, rpc.code_invalid_params, rpc.msg_invalid_params, null);
        };
        defer p.deinit();
        const condition = p.value.condition orelse {
            return errorEnvelope(self.gpa, id, rpc.code_invalid_params, "missing params.condition", null);
        };
        const timeout_ms: i64 = p.value.timeoutMs;
        const text_contains = condition.textContains;
        const ref_visible = condition.refVisible;
        if (text_contains == null and ref_visible == null) {
            return errorEnvelope(self.gpa, id, rpc.code_invalid_params, "condition must have textContains or refVisible", null);
        }

        const poll_interval_ms = 50;
        const max_polls = @max(1, @divTrunc(timeout_ms, poll_interval_ms) + 1);
        var polls: i64 = 0;
        while (true) {
            var job = UiJob{
                .tree = self.tree,
                .kind = .wait_poll,
                .gpa = self.gpa,
                .io = self.io,
                .text_contains = text_contains,
                .ref_visible = ref_visible,
            };
            runOnUi(&job);
            if (job.matched) {
                return resultEnvelope(self.gpa, id, "{\"matched\":true}");
            }
            polls += 1;
            if (polls >= max_polls) {
                const data = try std.fmt.allocPrint(self.gpa, "{{\"timeoutMs\":{d}}}", .{timeout_ms});
                defer self.gpa.free(data);
                return errorEnvelope(self.gpa, id, rpc.code_wait_for_timeout, rpc.msg_wait_for_timeout, data);
            }
            // Sleeps the automation thread only (never the UI thread); `.awake`
            // is the monotonic clock, unaffected by wall-clock adjustments.
            std.Io.sleep(self.io, .fromMilliseconds(poll_interval_ms), .awake) catch {};
        }
    }

    /// Runs `job` on the UI thread and wraps the outcome as a JSON-RPC envelope.
    fn runJobAndEnvelope(self: *Server, job: *UiJob, id: std.json.Value) ![]u8 {
        runOnUi(job);
        defer if (job.result_json) |r| self.gpa.free(r);
        defer if (job.err_data_json) |d| self.gpa.free(d);
        if (job.result_json) |result| {
            return resultEnvelope(self.gpa, id, result);
        }
        return errorEnvelope(self.gpa, id, job.err_code, job.err_msg orelse rpc.msg_internal_error, job.err_data_json);
    }
};

/// Reads one u32 LE length prefix + payload (the NDP outer frame). Caller frees.
fn readFrame(gpa: std.mem.Allocator, r: *std.Io.Reader) ![]u8 {
    var len_buf: [4]u8 = undefined;
    try r.readSliceAll(&len_buf);
    const len = std.mem.readInt(u32, &len_buf, .little);
    const payload = try gpa.alloc(u8, len);
    errdefer gpa.free(payload);
    try r.readSliceAll(payload);
    return payload;
}

/// Wraps already-serialized JSON in the u32 LE length prefix frame.
fn frameFromJson(gpa: std.mem.Allocator, json: []const u8) ![]u8 {
    const frame = try gpa.alloc(u8, 4 + json.len);
    std.mem.writeInt(u32, frame[0..4], @intCast(json.len), .little);
    @memcpy(frame[4..], json);
    return frame;
}

/// Builds `{"jsonrpc":"2.0","id":id,"result":<result_json>}` by splicing the
/// already-serialized result rather than double-encoding it.
fn resultEnvelope(gpa: std.mem.Allocator, id: std.json.Value, result_json: []const u8) ![]u8 {
    const id_str = try std.json.Stringify.valueAlloc(gpa, id, .{});
    defer gpa.free(id_str);
    return std.fmt.allocPrint(gpa, "{{\"jsonrpc\":\"2.0\",\"id\":{s},\"result\":{s}}}", .{ id_str, result_json });
}

/// Builds the JSON-RPC error envelope. `data_json`, if present, is spliced
/// verbatim (already-serialized); otherwise `data` is omitted.
fn errorEnvelope(gpa: std.mem.Allocator, id: std.json.Value, code: i32, message: []const u8, data_json: ?[]const u8) ![]u8 {
    const id_str = try std.json.Stringify.valueAlloc(gpa, id, .{});
    defer gpa.free(id_str);
    const msg_str = try std.json.Stringify.valueAlloc(gpa, message, .{});
    defer gpa.free(msg_str);
    if (data_json) |d| {
        return std.fmt.allocPrint(gpa, "{{\"jsonrpc\":\"2.0\",\"id\":{s},\"error\":{{\"code\":{d},\"message\":{s},\"data\":{s}}}}}", .{ id_str, code, msg_str, d });
    }
    return std.fmt.allocPrint(gpa, "{{\"jsonrpc\":\"2.0\",\"id\":{s},\"error\":{{\"code\":{d},\"message\":{s}}}}}", .{ id_str, code, msg_str });
}
