const std = @import("std");
const glib = @import("glib");
const gobject = @import("gobject");
const gtk = @import("gtk");
const gsk = @import("gsk");
const gdk = @import("gdk");
const graphene = @import("graphene");
const protocol = @import("protocol.zig");
const Tree = @import("tree.zig").Tree;
const backend = @import("gtk_backend.zig");

const G_SOURCE_REMOVE: c_int = 0;

/// The kinds of work a `UiJob` can carry. Real handlers land in Tasks 3-6;
/// Task 2 only scaffolds the marshal-and-block plumbing.
const JobKind = enum { get_tree, screenshot, click, wait_poll };

/// A request/response handoff between the automation thread and the GTK main
/// thread. The mutex/condition here guard only this struct, never the tree —
/// the tree is read exclusively on the UI thread (see `runOnUi`).
const UiJob = struct {
    tree: *Tree,
    kind: JobKind,

    // input (tagged by kind)
    ref: u32 = 0,
    path: ?[:0]const u8 = null,

    // output (filled on the UI thread by `handleOnUi`)
    result_json: ?[]u8 = null, // owned by gpa; the automation thread frees
    err_code: i32 = 0,
    err_msg: ?[]const u8 = null,
    err_data_json: ?[]u8 = null, // pre-serialized `data` object, owned by gpa

    gpa: std.mem.Allocator,
    io: std.Io,
    mutex: std.Io.Mutex = .init,
    done: std.Io.Condition = .init,
    finished: bool = false,
};

fn runOnUi(job: *UiJob) void {
    _ = glib.MainContext.default().invokeFull(glib.PRIORITY_DEFAULT, &uiCallback, job, null);
    job.mutex.lockUncancelable(job.io);
    defer job.mutex.unlock(job.io);
    while (!job.finished) job.done.waitUncancelable(job.io, &job.mutex);
}

fn uiCallback(data: ?*anyopaque) callconv(.c) c_int {
    const job: *UiJob = @ptrCast(@alignCast(data.?));
    handleOnUi(job);
    job.mutex.lockUncancelable(job.io);
    job.finished = true;
    job.done.signal(job.io);
    job.mutex.unlock(job.io);
    return G_SOURCE_REMOVE;
}

/// Runs on the GTK main thread. Fills `result_json` on success, or
/// `err_code`/`err_msg`/`err_data_json` on failure. Tasks 4-6 fill the
/// remaining arms; `.get_tree` is implemented here (Task 3).
fn handleOnUi(job: *UiJob) void {
    switch (job.kind) {
        .get_tree => handleGetTree(job),
        .screenshot, .click, .wait_poll => {
            job.result_json = job.gpa.dupe(u8, "{\"ok\":true}") catch null;
        },
    }
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
};

const GetTreeResult = struct {
    coordinateSpace: []const u8 = "logical-window-topleft",
    root: JsonNode,
};

/// Builds the nested snapshot on the UI thread: walks live GTK children
/// (`getFirstChild`/`getNextSibling`) for true visual order, mapping each
/// live widget back to its id via a reverse lookup built once per call.
fn handleGetTree(job: *UiJob) void {
    const tree = job.tree;
    const root_id = tree.rootId() orelse {
        job.err_code = -32603;
        job.err_msg = "no root";
        return;
    };
    const window_widget = backend.getWindow().?.as(gtk.Widget);

    var arena_state = std.heap.ArenaAllocator.init(job.gpa);
    defer arena_state.deinit();
    const arena = arena_state.allocator();

    // widget -> id reverse lookup, built once per call.
    var reverse = std.AutoHashMapUnmanaged(*gtk.Widget, u32){};
    var it = tree.nodes.iterator();
    while (it.next()) |entry| {
        reverse.put(arena, entry.value_ptr.*, entry.key_ptr.*) catch {};
    }

    const root_widget = tree.get(root_id) orelse {
        job.err_code = -32603;
        job.err_msg = "root widget missing";
        return;
    };
    const root_node = buildNode(arena, tree, &reverse, window_widget, root_widget, root_id) catch {
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
    reverse: *std.AutoHashMapUnmanaged(*gtk.Widget, u32),
    window_widget: *gtk.Widget,
    widget: *gtk.Widget,
    id: u32,
) !JsonNode {
    const meta = tree.metaGet(id);
    const widget_type = if (meta) |m| m.widget_type else "";
    const test_id = if (meta) |m| m.test_id else null;
    const text = if (meta) |m| m.text else null;
    const visible = gtk.Widget.getVisible(widget) != 0;

    var rect: graphene.Rect = undefined;
    const has_bounds = gtk.Widget.computeBounds(widget, window_widget, &rect) != 0;
    const geometry: ?Geometry = if (has_bounds)
        .{
            .x = @intFromFloat(rect.f_origin.f_x),
            .y = @intFromFloat(rect.f_origin.f_y),
            .w = @intFromFloat(rect.f_size.f_width),
            .h = @intFromFloat(rect.f_size.f_height),
        }
    else if (visible)
        .{ .x = 0, .y = 0, .w = gtk.Widget.getWidth(widget), .h = gtk.Widget.getHeight(widget) }
    else
        null;

    var children: std.ArrayList(JsonNode) = .empty;
    var maybe_child = gtk.Widget.getFirstChild(widget);
    while (maybe_child) |child| : (maybe_child = gtk.Widget.getNextSibling(child)) {
        const child_id = reverse.get(child) orelse continue;
        const child_node = try buildNode(arena, tree, reverse, window_widget, child, child_id);
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

        // Tasks 4-6 add screenshot/click/waitFor/stubs routing here.
        return errorEnvelope(self.gpa, id, -32601, "method not found", null);
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
