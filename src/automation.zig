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

/// A request/response handoff between the automation thread and the GTK main
/// thread. The mutex/condition here guard only this struct, never the tree —
/// the tree is read exclusively on the UI thread (see `runOnUi`).
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
/// `err_code`/`err_msg`/`err_data_json` on failure.
fn handleOnUi(job: *UiJob) void {
    switch (job.kind) {
        .get_tree => handleGetTree(job),
        .screenshot => handleScreenshot(job),
        .click => handleClick(job),
        .wait_poll => handleWaitPoll(job),
        .set_value => handleSetValue(job),
        .type_text => handleType(job),
        .scroll => handleScroll(job),
    }
}

/// Evaluates one `waitFor` condition against the live tree. Called once per
/// poll from `runOnUi` (each poll is a separate marshaled UI-thread read);
/// the sleep/deadline bookkeeping lives on the automation thread (Task 6
/// `dispatchWaitFor`), keeping all tree access UI-thread-only.
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
        job.matched = gtk.Widget.getVisible(widget) != 0;
        return;
    }
    job.matched = false;
}

const ClickResult = struct { ref: u32, dispatched: bool };

/// Actionability hit-test (exists ∧ visible ∧ mapped ∧ non-degenerate bounds),
/// shared by `click` and the setValue/type/scroll automation actions — never
/// act on what a user couldn't reach (research gotcha; full z-order/overlap
/// testing is deferred). Fills `job.err_*` (-32001) and returns null on
/// failure; same codes/reason strings as the original `handleClick` gates.
fn checkActionable(job: *UiJob) ?*gtk.Widget {
    const widget = job.tree.get(job.ref) orelse {
        job.err_code = -32001;
        job.err_msg = "not actionable";
        job.err_data_json = std.fmt.allocPrint(job.gpa, "{{\"ref\":{d},\"reason\":\"unknown\"}}", .{job.ref}) catch null;
        return null;
    };
    if (gtk.Widget.getVisible(widget) == 0) {
        job.err_code = -32001;
        job.err_msg = "not actionable";
        job.err_data_json = std.fmt.allocPrint(job.gpa, "{{\"ref\":{d},\"reason\":\"invisible\"}}", .{job.ref}) catch null;
        return null;
    }
    if (gtk.Widget.getMapped(widget) == 0) {
        job.err_code = -32001;
        job.err_msg = "not actionable";
        job.err_data_json = std.fmt.allocPrint(job.gpa, "{{\"ref\":{d},\"reason\":\"unmapped\"}}", .{job.ref}) catch null;
        return null;
    }
    const win = backend.getWindow().?.as(gtk.Widget);
    var rect: graphene.Rect = undefined;
    const has_bounds = gtk.Widget.computeBounds(widget, win, &rect) != 0;
    if (!has_bounds or rect.f_size.f_width <= 0 or rect.f_size.f_height <= 0) {
        job.err_code = -32001;
        job.err_msg = "not actionable";
        job.err_data_json = std.fmt.allocPrint(job.gpa, "{{\"ref\":{d},\"reason\":\"offscreen\"}}", .{job.ref}) catch null;
        return null;
    }
    return widget;
}

/// Sets `job.err_code = -32602` with a `{ref,type}` data payload — the wrong-
/// widget-kind / bad-value-type error shape shared by setValue/type/scroll.
fn jobInvalidParams(job: *UiJob, msg: []const u8) void {
    job.err_code = -32602;
    job.err_msg = msg;
    job.err_data_json = std.fmt.allocPrint(job.gpa, "{{\"ref\":{d}}}", .{job.ref}) catch null;
}

/// Actionability hit-test (exists ∧ visible ∧ mapped ∧ non-degenerate bounds)
/// before a semantic `clicked` dispatch — never click what a user couldn't
/// reach (research gotcha; full z-order/overlap testing is deferred).
fn handleClick(job: *UiJob) void {
    const widget = checkActionable(job) orelse return;

    // Semantic dispatch: emit `clicked` directly. `gtk.Widget.activate` was
    // tried first but only re-emits `clicked` on the first call per main-loop
    // settle under weston headless — rapid successive activate() calls (the
    // exact pattern the Task 9 driver uses) silently drop clicks 2+. Emitting
    // the signal directly bypasses GTK's activate/gesture state machine and
    // fires reliably on every call (verified: three rapid clicks -> Clicks: 3).
    gobject.signalEmitByName(@ptrCast(@alignCast(widget)), "clicked");

    const result = ClickResult{ .ref = job.ref, .dispatched = true };
    job.result_json = std.json.Stringify.valueAlloc(job.gpa, result, .{}) catch null;
}

const SetValueResult = struct { ref: u32, applied: bool };

/// setValue {ref,value}: kind-dispatched on the tracked widget_type. Sets the
/// native value through the same widget-owning object the generated
/// applyProps path uses, but WITHOUT the M5b-D2 echo-suppression wrapper —
/// automation actions must flow to React exactly like real user input
/// (plan judgment M5b-D2).
fn handleSetValue(job: *UiJob) void {
    const widget = checkActionable(job) orelse return;
    const meta = job.tree.metaGet(job.ref) orelse return; // gate 1 in checkActionable already proved existence
    const kind = meta.widget_type;
    const v = job.value orelse return jobInvalidParams(job, "missing params.value");

    if (std.mem.eql(u8, kind, "TextInput")) {
        if (v != .string) return jobInvalidParams(job, "value must be a string for TextInput");
        const z = job.gpa.dupeZ(u8, v.string) catch return;
        defer job.gpa.free(z);
        const editable = @as(*gtk.Entry, @ptrCast(@alignCast(widget))).as(gtk.Editable);
        gtk.Editable.setText(editable, z); // fires "changed" -> Event -> React (by design)
    } else if (std.mem.eql(u8, kind, "TextArea")) {
        if (v != .string) return jobInvalidParams(job, "value must be a string for TextArea");
        const z = job.gpa.dupeZ(u8, v.string) catch return;
        defer job.gpa.free(z);
        const view: *gtk.TextView = @ptrCast(@alignCast(widget));
        gtk.TextBuffer.setText(gtk.TextView.getBuffer(view), z, -1);
    } else if (std.mem.eql(u8, kind, "Checkbox") or std.mem.eql(u8, kind, "Radio")) {
        if (v != .bool) return jobInvalidParams(job, "value must be a bool for Checkbox/Radio");
        gtk.CheckButton.setActive(@ptrCast(@alignCast(widget)), @intFromBool(v.bool));
    } else if (std.mem.eql(u8, kind, "Slider")) {
        const num: f64 = switch (v) {
            .float => v.float,
            .integer => @floatFromInt(v.integer),
            else => return jobInvalidParams(job, "value must be a number for Slider"),
        };
        // GtkAdjustment-backed: Range.setValue drives the Scale's adjustment
        // and clamps to [min, max]; fires "value-changed".
        const range = @as(*gtk.Scale, @ptrCast(@alignCast(widget))).as(gtk.Range);
        gtk.Range.setValue(range, num);
    } else if (std.mem.eql(u8, kind, "Select")) {
        if (v != .integer) return jobInvalidParams(job, "value must be an integer index for Select");
        gtk.DropDown.setSelected(@ptrCast(@alignCast(widget)), @intCast(v.integer)); // fires notify::selected
    } else {
        return jobInvalidParams(job, "setValue unsupported for this widget type"); // data carries {ref}
    }
    const result = SetValueResult{ .ref = job.ref, .applied = true };
    job.result_json = std.json.Stringify.valueAlloc(job.gpa, result, .{}) catch null;
}

const TypeResult = struct { ref: u32, text: []const u8 };

/// type {ref,text}: TextInput only, semantic append through GtkEditable
/// (never keysyms — spec §8). Appends at the end of the current text.
fn handleType(job: *UiJob) void {
    const widget = checkActionable(job) orelse return;
    const meta = job.tree.metaGet(job.ref) orelse return;
    if (!std.mem.eql(u8, meta.widget_type, "TextInput"))
        return jobInvalidParams(job, "type supported only for TextInput");
    const text = job.text orelse return jobInvalidParams(job, "missing params.text");
    const editable = @as(*gtk.Entry, @ptrCast(@alignCast(widget))).as(gtk.Editable);
    const cur = std.mem.span(gtk.Editable.getText(editable));
    // insertText position is in CHARACTERS; append = current codepoint count.
    var pos: c_int = @intCast(std.unicode.utf8CountCodepoints(cur) catch cur.len);
    const z = job.gpa.dupeZ(u8, text) catch return;
    defer job.gpa.free(z);
    // Length param: bytes of `text` to insert. -1 (NUL-terminated) was
    // confirmed against a live GtkEntry (throwaway drive script, M5b Task 7
    // verification) to insert the full string correctly; kept as the
    // primary idiom per the plan's verify-then-fallback note.
    gtk.Editable.insertText(editable, z, -1, &pos); // fires "changed"
    const full = std.mem.span(gtk.Editable.getText(editable));
    const result = TypeResult{ .ref = job.ref, .text = full };
    job.result_json = std.json.Stringify.valueAlloc(job.gpa, result, .{}) catch null;
}

const ScrollResult = struct { ref: u32, x: f64, y: f64 };

/// scroll {ref,dx?,dy?}: ScrollView only, via its GtkAdjustments (spec §8
/// semantic-scroll model). Deltas are added to the current adjustment value;
/// `Adjustment.setValue` clamps to [lower, upper-page].
fn handleScroll(job: *UiJob) void {
    const widget = checkActionable(job) orelse return;
    const meta = job.tree.metaGet(job.ref) orelse return;
    if (!std.mem.eql(u8, meta.widget_type, "ScrollView"))
        return jobInvalidParams(job, "scroll supported only for ScrollView");
    const sw: *gtk.ScrolledWindow = @ptrCast(@alignCast(widget));
    if (job.dx) |dx| {
        const h = gtk.ScrolledWindow.getHadjustment(sw);
        gtk.Adjustment.setValue(h, gtk.Adjustment.getValue(h) + dx);
    }
    if (job.dy) |dy| {
        const v = gtk.ScrolledWindow.getVadjustment(sw);
        gtk.Adjustment.setValue(v, gtk.Adjustment.getValue(v) + dy);
    }
    const result = ScrollResult{
        .ref = job.ref,
        .x = gtk.Adjustment.getValue(gtk.ScrolledWindow.getHadjustment(sw)),
        .y = gtk.Adjustment.getValue(gtk.ScrolledWindow.getVadjustment(sw)),
    };
    job.result_json = std.json.Stringify.valueAlloc(job.gpa, result, .{}) catch null;
}

const ScreenshotResult = struct { path: []const u8, width: i32, height: i32 };

/// In-process GTK->GSK render of the window to a PNG at `job.path`. Primary
/// route: `WidgetPaintable` (captures the whole widget directly, no
/// parent/child juggling) snapshotted through the window's live renderer.
fn handleScreenshot(job: *UiJob) void {
    const path = job.path orelse {
        job.err_code = -32602;
        job.err_msg = "missing path";
        return;
    };
    const win = backend.getWindow() orelse {
        job.err_code = -32603;
        job.err_msg = "no window";
        return;
    };
    const win_widget = win.as(gtk.Widget);

    const native = gtk.Widget.getNative(win_widget) orelse {
        job.err_code = -32603;
        job.err_msg = "widget has no native";
        return;
    };
    var owned_renderer: ?*gsk.CairoRenderer = null;
    const renderer: *gsk.Renderer = gtk.Native.getRenderer(native) orelse blk: {
        // Fallback: realize a standalone cairo renderer against the native's surface.
        const cairo_renderer = gsk.CairoRenderer.new();
        owned_renderer = cairo_renderer;
        const surface = gtk.Native.getSurface(native);
        _ = gsk.Renderer.realize(cairo_renderer.as(gsk.Renderer), surface, null);
        break :blk cairo_renderer.as(gsk.Renderer);
    };
    defer if (owned_renderer) |r| {
        gsk.Renderer.unrealize(r.as(gsk.Renderer));
        r.as(gsk.Renderer).unref();
    };

    const paintable = gtk.WidgetPaintable.new(win_widget);
    defer paintable.unref();

    const snapshot = gtk.Snapshot.new();
    const width = gtk.Widget.getWidth(win_widget);
    const height = gtk.Widget.getHeight(win_widget);
    gdk.Paintable.snapshot(paintable.as(gdk.Paintable), snapshot.as(gdk.Snapshot), @floatFromInt(width), @floatFromInt(height));

    const node = gtk.Snapshot.freeToNode(snapshot) orelse {
        job.err_code = -32603;
        job.err_msg = "empty snapshot";
        return;
    };
    defer gsk.RenderNode.unref(node);

    const texture = gsk.Renderer.renderTexture(renderer, node, null);
    defer texture.unref();

    const path_z = job.gpa.dupeZ(u8, path) catch {
        job.err_code = -32603;
        job.err_msg = "oom";
        return;
    };
    defer job.gpa.free(path_z);
    if (gdk.Texture.saveToPng(texture, path_z) == 0) {
        job.err_code = -32603;
        job.err_msg = "failed to save png";
        return;
    }

    const result = ScreenshotResult{ .path = path, .width = gdk.Texture.getWidth(texture), .height = gdk.Texture.getHeight(texture) };
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
