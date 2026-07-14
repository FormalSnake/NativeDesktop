const std = @import("std");
const protocol = @import("protocol.zig");
// Method names, params/result shapes, and error codes are GENERATED from
// schema/rpc.json (the single source of truth shared with the TS mirror,
// packages/react/src/generated/rpc.ts) — a method/param/result change there
// regenerates both sides, so drift is a compile error, not a silent break.
const rpc = @import("generated/rpc.zig");
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

/// The kinds of work a `UiJob` can carry.
const JobKind = enum { get_tree, screenshot, click, wait_poll, set_value, type_text, scroll };

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
    window: ?u32 = null, // screenshot: params.window (target Window node id; null = root/first)

    // output (filled on the UI thread by `handleOnUi`)
    result_json: ?[]u8 = null, // owned by gpa; the automation thread frees
    matched: bool = false, // wait_poll only
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
/// (M6a Task 4 — was `glib.MainContext.default().invokeFull` directly) and
/// blocks the automation thread until the embedder signals completion. This
/// is the sole place automation touches the UI thread; the SLO guarantee
/// (SIGSTOP-the-child still answers) holds because this call never crosses
/// into the Bun child, only into the (fast, main-thread-marshaled) backend
/// vtable — same as before the ABI existed, just one indirection further.
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
    }
}

/// Evaluates one `waitFor` condition against the live tree. Called once per
/// poll from `runOnUi` (each poll is a separate marshaled UI-thread read);
/// the sleep/deadline bookkeeping lives on the automation thread
/// (`dispatchWaitFor`), keeping all tree access UI-thread-only. `visible` is
/// a vtable call (M6a Task 4) — the core no longer walks a native widget.
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
/// what a user couldn't reach (research gotcha; full z-order/overlap
/// testing is deferred). Fills `job.err_*` (-32001) and returns null on
/// failure; same codes/reason strings as before the ABI (M6a Task 4:
/// "mapped" folds into `node_visible`'s contract per the plan's v1
/// decision — the embedder reports false there for an unmapped widget too).
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
/// tagged fields (M6a-D2: params cross the ABI as JSON). Caller frees.
fn buildActionArgs(job: *UiJob) [:0]const u8 {
    return switch (job.kind) {
        .set_value => allocZFromValue(job.gpa, .{ .value = job.value orelse .null }),
        .type_text => allocZFromValue(job.gpa, .{ .text = job.text orelse "" }),
        .scroll => allocZFromValue(job.gpa, .{ .dx = job.dx, .dy = job.dy }),
        else => job.gpa.dupeZ(u8, "{}") catch @panic("OOM in automation buildActionArgs"),
    };
}

/// Dispatches click/setValue/type/scroll through `vtable.semantic_action`
/// (M6a-D3: the GTK embedder fills this with today's exact signal-emit/
/// editable/adjustment code; Mac fills it with AppKit in M6b). Never
/// suppresses the resulting native event — automation actions must flow to
/// React exactly like real user input (plan judgment M5b-D2, preserved
/// through the ABI).
fn handleSemanticAction(job: *UiJob, action: []const u8) void {
    const widget = checkActionable(job) orelse return;
    const action_z = job.gpa.dupeZ(u8, action) catch return;
    defer job.gpa.free(action_z);
    const args_z = buildActionArgs(job);
    defer job.gpa.free(args_z);

    var result_out: ?[*:0]u8 = null;
    var err_out: ?[*:0]u8 = null;
    const code = abi_backend.vtable.semantic_action(abi_backend.ctx, widget, job.ref, action_z, args_z, &result_out, &err_out);

    if (code == 0) {
        if (result_out) |r| {
            job.result_json = job.gpa.dupe(u8, std.mem.span(r)) catch null;
            abi.nd_free(r);
        }
        return;
    }
    job.err_code = code;
    job.err_msg = rpc.msg_not_actionable;
    if (err_out) |e| {
        job.err_data_json = job.gpa.dupe(u8, std.mem.span(e)) catch null;
        abi.nd_free(e);
    }
}

/// Reads width/height straight from the PNG's IHDR chunk (portable, no GTK):
/// an 8-byte signature, then a 4-byte chunk length, 4-byte "IHDR" tag, then
/// width/height as big-endian u32 — a fixed layout every PNG encoder
/// (including GTK's `gdk.Texture.saveToPng` and any future AppKit encoder)
/// produces identically. `vtable.snapshot` only returns success/failure
/// (M6a-D3); the core reads the file it just asked the embedder to write
/// rather than growing the ABI for width/height.
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
/// `vtable.snapshot` (M6a-D3 — GTK fills this with today's WidgetPaintable
/// code verbatim; Mac supplies the fidelity-ladder solution in M6b).
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

/// Builds the nested snapshot on the UI thread. Task 3: child order now
/// comes from `Tree.childrenOf` — the ordered per-parent sibling list
/// maintained by `apply`'s append/insertBefore/remove handlers — instead of
/// grouping over `tree.meta`'s hashmap iteration (which is bucket layout,
/// not insertion order, and silently scrambled sibling order under
/// `insertBefore`/reorders).
fn handleGetTree(job: *UiJob) void {
    const tree = job.tree;
    const root_id = tree.rootId() orelse {
        job.err_code = rpc.code_internal_error;
        job.err_msg = "no root";
        return;
    };

    var arena_state = std.heap.ArenaAllocator.init(job.gpa);
    defer arena_state.deinit();
    const arena = arena_state.allocator();

    // Nodes never reached by the ordered `children` lists (host-created
    // overlay chrome, M8-D5: `registerOverlayNode` in gtk/overlay.zig only
    // calls `putMeta` with `parent == 0` — it never records into `Tree`'s
    // ordered list) still need to surface in getTree, matching the old
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
    var meta_it = tree.meta.iterator();
    while (meta_it.next()) |entry| {
        const id = entry.key_ptr.*;
        if (id == root_id) continue;
        if (placed.contains(id)) continue;
        orphans.append(arena, id) catch {};
    }
    std.mem.sort(u32, orphans.items, {}, std.sort.asc(u32));

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
                var job = UiJob{ .tree = self.tree, .kind = .get_tree, .gpa = self.gpa, .io = self.io };
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
                    // Any Window node is a valid target (multi-window), not just
                    // the first — `metaGet` is the tree's window registry.
                    const m = self.tree.metaGet(window_ref);
                    if (m == null or !std.mem.eql(u8, m.?.widget_type, "Window")) {
                        return errorEnvelope(self.gpa, id, rpc.code_invalid_params, "unknown window ref", null);
                    }
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
        }
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
