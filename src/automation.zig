const std = @import("std");
const protocol = @import("protocol.zig");
const Tree = @import("tree.zig").Tree;
const Widget = @import("backend.zig").impl.Widget;
const abi = @import("abi.zig");
const abi_backend = @import("abi_backend.zig");

fn paramStr(params: ?std.json.Value, key: []const u8) ?[]const u8 {
    const v = params orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .string => field.string,
        else => null,
    };
}

fn paramInt(params: ?std.json.Value, key: []const u8) ?i64 {
    const v = params orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .integer => field.integer,
        else => null,
    };
}

fn paramObj(params: ?std.json.Value, key: []const u8) ?std.json.Value {
    const v = params orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .object => field,
        else => null,
    };
}

fn paramAny(params: ?std.json.Value, key: []const u8) ?std.json.Value {
    const v = params orelse return null;
    if (v != .object) return null;
    return v.object.get(key);
}

fn paramFloat(params: ?std.json.Value, key: []const u8) ?f64 {
    const v = params orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .float => field.float,
        .integer => @floatFromInt(field.integer),
        else => null,
    };
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
        job.err_code = -32001;
        job.err_msg = "not actionable";
        job.err_data_json = std.fmt.allocPrint(job.gpa, "{{\"ref\":{d},\"reason\":\"unknown\"}}", .{job.ref}) catch null;
        return null;
    };
    if (!abi_backend.vtable.node_visible(abi_backend.ctx, widget)) {
        job.err_code = -32001;
        job.err_msg = "not actionable";
        job.err_data_json = std.fmt.allocPrint(job.gpa, "{{\"ref\":{d},\"reason\":\"invisible\"}}", .{job.ref}) catch null;
        return null;
    }
    var rect: abi.NdRect = undefined;
    const has_bounds = abi_backend.vtable.node_bounds(abi_backend.ctx, widget, &rect);
    if (!has_bounds or rect.w <= 0 or rect.h <= 0) {
        job.err_code = -32001;
        job.err_msg = "not actionable";
        job.err_data_json = std.fmt.allocPrint(job.gpa, "{{\"ref\":{d},\"reason\":\"offscreen\"}}", .{job.ref}) catch null;
        return null;
    }
    return widget;
}

/// Sets `job.err_code = -32602` with a `{ref}` data payload — the wrong-
/// widget-kind / bad-value-type error shape shared by setValue/type/scroll.
fn jobInvalidParams(job: *UiJob, msg: []const u8) void {
    job.err_code = -32602;
    job.err_msg = msg;
    job.err_data_json = std.fmt.allocPrint(job.gpa, "{{\"ref\":{d}}}", .{job.ref}) catch null;
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
    job.err_msg = "not actionable";
    if (err_out) |e| {
        job.err_data_json = job.gpa.dupe(u8, std.mem.span(e)) catch null;
        abi.nd_free(e);
    }
}

const ScreenshotResult = struct { path: []const u8, width: i32, height: i32 };

/// In-process render of the window to a PNG at `job.path`, via
/// `vtable.snapshot` (M6a-D3 — GTK fills this with today's WidgetPaintable
/// code verbatim; Mac supplies the fidelity-ladder solution in M6b). Width/
/// height are not reported by the vtable call itself (the embedder writes
/// the PNG directly) — 0 here is a placeholder the embedder-specific size
/// isn't threaded back across the ABI for v1; scripts assert file presence
/// and content, not these fields.
fn handleScreenshot(job: *UiJob) void {
    const path = job.path orelse {
        job.err_code = -32602;
        job.err_msg = "missing path";
        return;
    };
    if (!abi_backend.vtable.snapshot(abi_backend.ctx, path)) {
        job.err_code = -32603;
        job.err_msg = "failed to save png";
        return;
    }
    const result = ScreenshotResult{ .path = path, .width = 0, .height = 0 };
    job.result_json = std.json.Stringify.valueAlloc(job.gpa, result, .{}) catch null;
}

/// Owned tree-node JSON shape (matches the RPC surface contract exactly).
const Geometry = struct { x: i32, y: i32, w: i32, h: i32 };

const JsonNode = struct {
    ref: u32,
    type: []const u8,
    testID: ?[]const u8,
    text: ?[]const u8,
    visible: bool,
    geometry: ?Geometry,
    children: []JsonNode,
    /// ListView's row count (M5c-D4). Null for every widget that isn't
    /// data-driven; never derived from walking recycled row widgets.
    itemCount: ?u32 = null,
};

const GetTreeResult = struct {
    coordinateSpace: []const u8 = "logical-window-topleft",
    root: JsonNode,
};

/// Builds the nested snapshot on the UI thread. M6a Task 4: the child walk
/// is now core-owned, derived from `Tree.meta.parent` (portable — no native
/// widget traversal) instead of walking live GTK children. This is
/// actually simpler than the old walk: the meta map only ever contains
/// TRACKED nodes (GTK-internal wrappers like the auto-inserted GtkViewport
/// were never put into `tree.meta`), so there is no "untracked wrapper" to
/// skip-and-recurse-through anymore — every meta entry whose `parent`
/// matches becomes a direct child, in insertion order.
fn handleGetTree(job: *UiJob) void {
    const tree = job.tree;
    const root_id = tree.rootId() orelse {
        job.err_code = -32603;
        job.err_msg = "no root";
        return;
    };

    var arena_state = std.heap.ArenaAllocator.init(job.gpa);
    defer arena_state.deinit();
    const arena = arena_state.allocator();

    // id -> children ids, insertion order, built once per call from the
    // meta map's `parent` field (portable — replaces the old live-widget
    // getFirstChild/getNextSibling walk).
    var children_of: std.AutoHashMapUnmanaged(u32, std.ArrayList(u32)) = .empty;
    var meta_it = tree.meta.iterator();
    while (meta_it.next()) |entry| {
        const id = entry.key_ptr.*;
        if (id == root_id) continue;
        const parent = entry.value_ptr.parent;
        const gop = children_of.getOrPut(arena, parent) catch continue;
        if (!gop.found_existing) gop.value_ptr.* = .empty;
        gop.value_ptr.append(arena, id) catch {};
    }

    const root_widget = tree.get(root_id) orelse {
        job.err_code = -32603;
        job.err_msg = "root widget missing";
        return;
    };
    _ = root_widget;
    const root_node = buildNode(arena, tree, &children_of, root_id) catch {
        job.err_code = -32603;
        job.err_msg = "failed to build tree";
        return;
    };

    const result = GetTreeResult{ .root = root_node };
    job.result_json = std.json.Stringify.valueAlloc(job.gpa, result, .{}) catch null;
}

fn buildNode(
    arena: std.mem.Allocator,
    tree: *Tree,
    children_of: *std.AutoHashMapUnmanaged(u32, std.ArrayList(u32)),
    id: u32,
) !JsonNode {
    const meta = tree.metaGet(id);
    const widget_type = if (meta) |m| m.widget_type else "";
    const test_id = if (meta) |m| m.test_id else null;
    const text = if (meta) |m| m.text else null;
    const item_count = if (meta) |m| m.item_count else null;
    const widget = tree.get(id);

    const visible = if (widget) |w| abi_backend.vtable.node_visible(abi_backend.ctx, w) else false;

    var rect: abi.NdRect = undefined;
    const has_bounds = if (widget) |w| abi_backend.vtable.node_bounds(abi_backend.ctx, w, &rect) else false;
    const geometry: ?Geometry = if (has_bounds)
        .{ .x = rect.x, .y = rect.y, .w = rect.w, .h = rect.h }
    else
        null;

    var children: std.ArrayList(JsonNode) = .empty;
    if (children_of.get(id)) |kids| {
        for (kids.items) |child_id| {
            const child_node = try buildNode(arena, tree, children_of, child_id);
            try children.append(arena, child_node);
        }
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

    /// Parses `{jsonrpc,id,method,params}`, routes on `method`, and returns an
    /// already-serialized JSON-RPC response envelope (`gpa`-owned).
    fn dispatch(self: *Server, req_bytes: []const u8) ![]u8 {
        const Req = struct {
            id: std.json.Value = .null,
            method: []const u8,
            params: ?std.json.Value = null,
        };
        const parsed = std.json.parseFromSlice(Req, self.gpa, req_bytes, .{ .ignore_unknown_fields = true }) catch {
            return errorEnvelope(self.gpa, .null, -32700, "parse error", null);
        };
        defer parsed.deinit();
        const id = parsed.value.id;
        const method = parsed.value.method;
        std.debug.print("ND_RPC method={s} id={any}\n", .{ method, id });

        if (std.mem.eql(u8, method, "getTree")) {
            var job = UiJob{ .tree = self.tree, .kind = .get_tree, .gpa = self.gpa, .io = self.io };
            return self.runJobAndEnvelope(&job, id);
        }

        if (std.mem.eql(u8, method, "screenshot")) {
            const path = paramStr(parsed.value.params, "path") orelse {
                return errorEnvelope(self.gpa, id, -32602, "missing params.path", null);
            };
            if (paramInt(parsed.value.params, "window")) |window_ref| {
                const root_id = self.tree.rootId();
                if (root_id == null or @as(u32, @intCast(window_ref)) != root_id.?) {
                    return errorEnvelope(self.gpa, id, -32602, "unknown window ref", null);
                }
            }
            const path_z = self.gpa.dupeZ(u8, path) catch return error.OutOfMemory;
            defer self.gpa.free(path_z);
            var job = UiJob{ .tree = self.tree, .kind = .screenshot, .gpa = self.gpa, .io = self.io, .path = path_z };
            return self.runJobAndEnvelope(&job, id);
        }

        if (std.mem.eql(u8, method, "click")) {
            const ref = paramInt(parsed.value.params, "ref") orelse {
                return errorEnvelope(self.gpa, id, -32602, "missing params.ref", null);
            };
            var job = UiJob{ .tree = self.tree, .kind = .click, .gpa = self.gpa, .io = self.io, .ref = @intCast(ref) };
            return self.runJobAndEnvelope(&job, id);
        }

        if (std.mem.eql(u8, method, "waitFor")) {
            return self.dispatchWaitFor(id, parsed.value.params);
        }

        if (std.mem.eql(u8, method, "setValue")) {
            const ref = paramInt(parsed.value.params, "ref") orelse return errorEnvelope(self.gpa, id, -32602, "missing params.ref", null);
            const value = paramAny(parsed.value.params, "value") orelse return errorEnvelope(self.gpa, id, -32602, "missing params.value", null);
            var job = UiJob{ .tree = self.tree, .kind = .set_value, .gpa = self.gpa, .io = self.io, .ref = @intCast(ref), .value = value };
            return self.runJobAndEnvelope(&job, id);
        }

        if (std.mem.eql(u8, method, "type")) {
            const ref = paramInt(parsed.value.params, "ref") orelse return errorEnvelope(self.gpa, id, -32602, "missing params.ref", null);
            const text = paramStr(parsed.value.params, "text") orelse return errorEnvelope(self.gpa, id, -32602, "missing params.text", null);
            var job = UiJob{ .tree = self.tree, .kind = .type_text, .gpa = self.gpa, .io = self.io, .ref = @intCast(ref), .text = text };
            return self.runJobAndEnvelope(&job, id);
        }

        if (std.mem.eql(u8, method, "scroll")) {
            const ref = paramInt(parsed.value.params, "ref") orelse return errorEnvelope(self.gpa, id, -32602, "missing params.ref", null);
            var job = UiJob{ .tree = self.tree, .kind = .scroll, .gpa = self.gpa, .io = self.io, .ref = @intCast(ref), .dx = paramFloat(parsed.value.params, "dx"), .dy = paramFloat(parsed.value.params, "dy") };
            return self.runJobAndEnvelope(&job, id);
        }

        return errorEnvelope(self.gpa, id, -32601, "method not found", null);
    }

    /// Polls the tree on the UI thread at ~50ms until the condition holds or
    /// `timeoutMs` elapses. Each poll is a separate marshaled UI-thread read
    /// (`handleWaitPoll`); the sleep/deadline live here on the automation
    /// thread so tree access stays UI-thread-only.
    fn dispatchWaitFor(self: *Server, id: std.json.Value, params: ?std.json.Value) ![]u8 {
        const condition = paramObj(params, "condition") orelse {
            return errorEnvelope(self.gpa, id, -32602, "missing params.condition", null);
        };
        const timeout_ms: i64 = paramInt(params, "timeoutMs") orelse 2000;
        const text_contains = paramStr(condition, "textContains");
        const ref_visible = paramInt(condition, "refVisible");
        if (text_contains == null and ref_visible == null) {
            return errorEnvelope(self.gpa, id, -32602, "condition must have textContains or refVisible", null);
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
                .ref_visible = if (ref_visible) |r| @intCast(r) else null,
            };
            runOnUi(&job);
            if (job.matched) {
                return resultEnvelope(self.gpa, id, "{\"matched\":true}");
            }
            polls += 1;
            if (polls >= max_polls) {
                const data = try std.fmt.allocPrint(self.gpa, "{{\"timeoutMs\":{d}}}", .{timeout_ms});
                defer self.gpa.free(data);
                return errorEnvelope(self.gpa, id, -32002, "waitFor timeout", data);
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
        return errorEnvelope(self.gpa, id, job.err_code, job.err_msg orelse "internal error", job.err_data_json);
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
