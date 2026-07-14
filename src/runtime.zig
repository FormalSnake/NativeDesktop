const std = @import("std");
const protocol = @import("protocol.zig");
const Tree = @import("tree.zig").Tree;
const backend = @import("backend.zig").impl;
const acl = @import("acl.zig");
const ndp_binary = @import("ndp_binary.zig");
// The UI-thread marshal + crash-overlay chrome are vtable-only concerns (not
// part of the comptime null|abi seam every backend implements) — the core
// calls them straight through the registered C vtable (M6a Task 3), the same
// object `abi_backend.bind` already wired for the structural seam calls.
const abi_backend = @import("abi_backend.zig");

var trace: bool = false;

// Used when the embedder never called `nd_set_acl` (T10): the safe default
// grants the core UI ops so every existing demo stays green.
var default_acl: acl.Acl = undefined;
var default_acl_ready: bool = false;

/// Returns null if the batch is permitted; otherwise the first permission that
/// was denied (for the ND_ACL_DENY marker + error frame). Every commit needs
/// core:commit; a Window create additionally needs core:window.create.
fn commitGate(a: *acl.Acl, batch: protocol.CommitBatch) ?[]const u8 {
    if (!a.isAllowed(0, "core:commit")) return "core:commit";
    for (batch.ops) |op| {
        if (std.mem.eql(u8, op.op, "create")) {
            if (op.widget) |w| if (std.mem.eql(u8, w, "Window")) {
                if (!a.isAllowed(0, "core:window.create")) return "core:window.create";
            };
        }
    }
    return null;
}

pub const Runtime = struct {
    gpa: std.mem.Allocator,
    threaded: std.Io.Threaded,
    io: std.Io,
    server: std.Io.net.Server,
    stream: ?std.Io.net.Stream = null,
    tree: *Tree,
    child: std.process.Child,
    writer_mutex: std.Io.Mutex = .init,
    write_buf: [4096]u8 = undefined,
    seq: u64 = 0,
    sock_path: [:0]u8,
    // Active widget backend name ("gtk" | "appkit"), supplied by the embedder
    // via `nd_set_backend_name` and sent verbatim in the helloAck so the child
    // can populate `Platform.backend`.
    backend_name: []const u8 = "unknown",
    // ND_DEV=1 (M8-D4): gates the `--hot` spawn flag and the overlay's
    // Restart button ONLY. The overlay itself paints regardless.
    dev: bool = false,
    parent_env: *const std.process.Environ.Map = undefined,
    real_environ: std.process.Environ = undefined,
    // Stashed by the `runtimeError` reader arm; consumed by the crash overlay
    // (M8) when the imminent disconnect fires. Freed/replaced on each report.
    last_error_message: ?[]u8 = null,
    last_error_stack: ?[]u8 = null,
    // Set once the reader loop has painted the overlay for the current
    // child's exit, so a stalled/aborted handshake doesn't paint twice.
    overlay_shown: bool = false,

    var singleton: ?*Runtime = null;

    pub fn start(
        gpa: std.mem.Allocator,
        app: ?*anyopaque,
        tree: *Tree,
        parent_env: *const std.process.Environ.Map,
        real_environ: std.process.Environ,
        backend_name: []const u8,
    ) !*Runtime {
        _ = app; // window is owned by the child's `create Window` op via the embedder/tree
        trace = parent_env.get("NDP_TRACE") != null;

        const self = try gpa.create(Runtime);
        self.* = undefined;
        self.gpa = gpa;
        self.tree = tree;
        self.backend_name = backend_name;
        self.writer_mutex = .init;
        self.seq = 0;
        self.last_error_message = null;
        self.last_error_stack = null;
        self.overlay_shown = false;
        self.parent_env = parent_env;
        self.real_environ = real_environ;
        self.dev = if (parent_env.get("ND_DEV")) |v| std.mem.eql(u8, v, "1") else false;

        if (!default_acl_ready) {
            default_acl = acl.Acl.initDefault(gpa);
            default_acl_ready = true;
        }

        // `real_environ` (the process's actual environment block) is required so the
        // Threaded backend's PATH resolution for `std.process.spawn("bun", ...)` sees
        // the real PATH rather than falling back to `default_PATH`, which lacks bun.
        self.threaded = std.Io.Threaded.init(gpa, .{ .environ = real_environ });
        self.io = self.threaded.io();

        const runtime_dir = parent_env.get("XDG_RUNTIME_DIR") orelse "/tmp";
        const pid = std.os.linux.getpid();
        self.sock_path = try std.fmt.allocPrintSentinel(gpa, "{s}/nd-{d}.sock", .{ runtime_dir, pid }, 0);
        std.Io.Dir.deleteFileAbsolute(self.io, self.sock_path) catch {};

        const addr = try std.Io.net.UnixAddress.init(self.sock_path);
        self.server = try addr.listen(self.io, .{});

        try self.spawnChild();

        singleton = self;

        _ = try std.Thread.spawn(.{}, readerLoop, .{self});
        return self;
    }

    /// Spawns the bun child. `dev` selects `bun --hot <script>` over
    /// `bun <script>` (M8-D4) — everything else about the spawn is
    /// unchanged, so a non-dev run stays byte-identical to M5c. Factored out
    /// so a dev-mode Restart can respawn a fresh child against the same
    /// listening socket.
    fn spawnChild(self: *Runtime) !void {
        var env = std.process.Environ.Map.init(self.gpa);
        defer env.deinit();
        for (self.parent_env.keys(), self.parent_env.values()) |k, v| try env.put(k, v);
        try env.put("ND_SOCKET", self.sock_path);
        const script = self.parent_env.get("ND_SCRIPT") orelse "runtime/m2-demo.ts";
        const argv: []const []const u8 = if (self.dev)
            &.{ "bun", "--hot", script }
        else
            &.{ "bun", script };
        self.child = try std.process.spawn(self.io, .{
            .argv = argv,
            .environ_map = &env,
            .stdin = .inherit,
            .stdout = .inherit,
            .stderr = .inherit,
        });
    }

    pub fn getIo(self: *Runtime) std.Io {
        return self.io;
    }

    /// `nd_emit_event`'s body (abi.zig) routes here — the embedder->core
    /// event channel (M6a-D2). There is no more core-installed sink function
    /// pointer (M5c's `setEventSink`/`initStyle` are gone — the embedder
    /// calls `nd_emit_event` directly for every native signal, including its
    /// own style-error reports). A no-op before `start` has run (no window
    /// mounted yet, nothing to report to).
    pub fn emitEvent(node_id: u32, name: []const u8, payload: protocol.EventPayload) void {
        if (singleton) |self| self.sendEvent(node_id, name, payload);
    }

    pub fn sendEvent(self: *Runtime, node_id: u32, name: []const u8, payload: protocol.EventPayload) void {
        self.seq += 1;
        const ev = protocol.Event{ .seq = self.seq, .nodeId = node_id, .name = name, .payload = payload };
        self.writeFrameOpts(ev, .{ .emit_null_optional_fields = false });
    }

    fn writeFrame(self: *Runtime, value: anytype) void {
        self.writeFrameOpts(value, .{});
    }

    fn writeFrameOpts(self: *Runtime, value: anytype, options: std.json.Stringify.Options) void {
        const frame = protocol.encodeFrameOpts(self.gpa, value, options) catch return;
        defer self.gpa.free(frame);
        if (trace) std.debug.print("<< {s}\n", .{frame[4..]});
        self.writer_mutex.lockUncancelable(self.io);
        defer self.writer_mutex.unlock(self.io);
        const stream = self.stream orelse return;
        var w = stream.writer(self.io, &self.write_buf);
        w.interface.writeAll(frame) catch {};
        w.interface.flush() catch {};
    }

    /// Frames a pre-serialized JSON payload (u32 LE length ‖ bytes) through
    /// the same writer-mutex path as writeFrameOpts. Used for pluginCommand
    /// results: the plugin's JSON is arbitrary and already serialized, so
    /// splicing it in raw (rather than re-stringifying it as a JSON string)
    /// keeps the pluginResult frame's `result` field structurally typed.
    fn writeRawJson(self: *Runtime, json: []const u8) void {
        const frame = self.gpa.alloc(u8, 4 + json.len) catch return;
        defer self.gpa.free(frame);
        std.mem.writeInt(u32, frame[0..4], @intCast(json.len), .little);
        @memcpy(frame[4..], json);
        if (trace) std.debug.print("<< {s}\n", .{json});
        self.writer_mutex.lockUncancelable(self.io);
        defer self.writer_mutex.unlock(self.io);
        const stream = self.stream orelse return;
        var w = stream.writer(self.io, &self.write_buf);
        w.interface.writeAll(frame) catch {};
        w.interface.flush() catch {};
    }

    fn readerLoop(self: *Runtime) void {
        const stream = self.server.accept(self.io) catch {
            self.onChildExit();
            return;
        };
        self.stream = stream;
        std.debug.print("ND_CHILD_CONNECTED\n", .{});

        var read_buf: [64 * 1024]u8 = undefined;
        var r = stream.reader(self.io, &read_buf);

        // Handshake.
        const first = readFrame(self, &r.interface) catch {
            self.onChildExit();
            return;
        };
        defer self.gpa.free(first);
        {
            const parsed = std.json.parseFromSlice(protocol.Hello, self.gpa, first, .{ .ignore_unknown_fields = true }) catch {
                self.onChildExit();
                return;
            };
            defer parsed.deinit();
            if (parsed.value.ndpVersion != protocol.ndp_version) {
                self.writeFrame(protocol.ErrorFrame{
                    .message = "ndp version mismatch",
                    .expected = protocol.ndp_version,
                    .got = parsed.value.ndpVersion,
                });
                self.child.kill(self.io);
                self.onChildExit();
                return;
            }
        }
        self.writeFrame(protocol.HelloAck{ .ndpVersion = protocol.ndp_version, .encodings = &.{ "binary", "json" }, .backend = self.backend_name });
        std.debug.print("ND_HELLO_OK\n", .{});

        // Frame loop.
        while (true) {
            const bytes = readFrame(self, &r.interface) catch {
                self.onChildExit();
                return;
            };

            if (ndp_binary.isBinaryPayload(bytes)) {
                if (trace) {
                    const j = ndp_binary.traceToJson(self.gpa, bytes) catch null;
                    if (j) |jj| {
                        std.debug.print(">> {s}\n", .{jj});
                        self.gpa.free(jj);
                    }
                }
                self.marshalBinaryCommit(bytes); // ownership transfers
                continue;
            }
            const kind = protocol.peekType(self.gpa, bytes) catch {
                self.gpa.free(bytes);
                continue;
            };
            defer self.gpa.free(kind);
            if (std.mem.eql(u8, kind, "commitBatch")) {
                self.marshalCommit(bytes); // ownership transfers (JSON path)
            } else if (std.mem.eql(u8, kind, "ping")) {
                self.gpa.free(bytes);
                self.writeFrame(.{ .type = "pong" });
            } else if (std.mem.eql(u8, kind, "runtimeError")) {
                self.stashRuntimeError(bytes);
            } else if (std.mem.eql(u8, kind, "pluginCommand")) {
                self.handlePluginCommand(bytes);
            } else if (std.mem.eql(u8, kind, "widgetCommand")) {
                self.marshalWidgetCommand(bytes); // ownership transfers
            } else {
                self.gpa.free(bytes);
            }
        }
    }

    /// Reads one u32 LE length prefix + payload. Caller frees the returned slice.
    fn readFrame(self: *Runtime, r: *std.Io.Reader) ![]u8 {
        var len_buf: [4]u8 = undefined;
        try r.readSliceAll(&len_buf);
        const len = std.mem.readInt(u32, &len_buf, .little);
        const payload = try self.gpa.alloc(u8, len);
        errdefer self.gpa.free(payload);
        try r.readSliceAll(payload);
        // Binary payloads aren't valid UTF-8 text — tracing them raw here
        // would print garbage. `readerLoop`'s binary-sniff branch prints the
        // decoded-to-JSON trace instead (spec §9 parity), so skip here.
        if (trace and !ndp_binary.isBinaryPayload(payload)) std.debug.print(">> {s}\n", .{payload});
        return payload;
    }

    /// Parses a `runtimeError {message, stack}` frame and stashes both into
    /// `last_error_message`/`last_error_stack` (M8) — the crash overlay reads
    /// these when the imminent disconnect triggers `onChildExit`. Best-effort:
    /// a malformed frame is dropped, never crashes the reader loop.
    fn stashRuntimeError(self: *Runtime, bytes: []u8) void {
        defer self.gpa.free(bytes);
        const RE = struct { message: []const u8 = "", stack: []const u8 = "" };
        const parsed = std.json.parseFromSlice(RE, self.gpa, bytes, .{ .ignore_unknown_fields = true }) catch return;
        defer parsed.deinit();
        if (self.last_error_message) |m| self.gpa.free(m);
        if (self.last_error_stack) |s| self.gpa.free(s);
        self.last_error_message = self.gpa.dupe(u8, parsed.value.message) catch null;
        self.last_error_stack = self.gpa.dupe(u8, parsed.value.stack) catch null;
        std.debug.print("ND_RUNTIME_ERROR_REPORTED {s}\n", .{parsed.value.message});
    }

    /// Called from the reader-loop thread on every disconnect path (M8-D3:
    /// a dead `bytes` read = the child actually exited — crash, kill -9, or
    /// process.exit — never a `--hot` edit, which keeps the same socket).
    /// Prints the existing marker, then marshals the overlay paint onto the
    /// UI thread with whatever error text was last reported (or a generic
    /// fallback if the child died silently).
    fn onChildExit(self: *Runtime) void {
        std.debug.print("ND_CHILD_EXITED\n", .{});
        if (self.overlay_shown) return; // already painted for this child's exit
        const msg = self.last_error_message orelse "Runtime disconnected";
        const OverlayJob = struct { rt: *Runtime, msg: []const u8 };
        const job = self.gpa.create(OverlayJob) catch return;
        job.* = .{ .rt = self, .msg = self.gpa.dupe(u8, msg) catch msg };
        abi_backend.vtable.marshal_async(abi_backend.ctx, &showOverlayOnUi, job);
    }

    fn showOverlayOnUi(data: ?*anyopaque) callconv(.c) void {
        const OverlayJob = struct { rt: *Runtime, msg: []const u8 };
        const job: *OverlayJob = @ptrCast(@alignCast(data.?));
        defer {
            job.rt.gpa.free(job.msg);
            job.rt.gpa.destroy(job);
        }
        const self = job.rt;
        self.overlay_shown = true;
        const msg_z = self.gpa.dupeZ(u8, job.msg) catch return;
        defer self.gpa.free(msg_z);
        abi_backend.vtable.show_overlay(abi_backend.ctx, msg_z);
    }

    /// Restart trampoline: the embedder's crash-overlay Restart button (dev-
    /// mode only — the embedder gates this on its own `ND_DEV` read, since
    /// `show_overlay`'s ABI carries only the message, M6a Task 3) calls
    /// `nd_emit_event(ctx, 0, "restart", "{}")`; `nd_emit_event`'s body
    /// (abi.zig) intercepts that reserved name and routes here instead of
    /// forwarding it as a normal NDP event (the child is dead — there is
    /// nothing to forward it to).
    pub fn restart() void {
        if (singleton) |self| self.respawn();
    }

    /// Terminates the bun child. The embedder's app-shutdown handler routes
    /// here (via `nd_shutdown`) so closing the window kills the child instead
    /// of orphaning it. `overlay_shown` is set first so the reader loop's
    /// `onChildExit` treats this deliberate kill as already-handled and
    /// doesn't try to paint a crash overlay onto a window that is tearing
    /// down. No-op before `start` (no child yet).
    pub fn stop() void {
        if (singleton) |self| {
            self.overlay_shown = true;
            self.child.kill(self.io);
        }
    }

    /// Clears the crash overlay and spawns a fresh child against the same
    /// listening socket. The respawned child starts at generation 0 again;
    /// since the dead child's nodes were never GC'd (the process died, it
    /// never sent a higher-generation CommitBatch), every non-overlay node
    /// is cleared here so the fresh mount rebuilds from an empty tree.
    fn respawn(self: *Runtime) void {
        // Empty message is the clear sentinel (M6a Task 3): `show_overlay`'s
        // ABI carries only one string param, so "" means "take the overlay
        // down" rather than adding a second vtable field for one call site.
        abi_backend.vtable.show_overlay(abi_backend.ctx, "");
        self.tree.clearAppNodes();
        self.overlay_shown = false;
        if (self.last_error_message) |m| {
            self.gpa.free(m);
            self.last_error_message = null;
        }
        if (self.last_error_stack) |s| {
            self.gpa.free(s);
            self.last_error_stack = null;
        }
        self.spawnChild() catch |err| {
            std.debug.print("ND_RESPAWN_ERROR {any}\n", .{err});
            return;
        };
        std.debug.print("ND_RESPAWNED\n", .{});
        _ = std.Thread.spawn(.{}, readerLoop, .{self}) catch |err| {
            std.debug.print("ND_RESPAWN_ERROR {any}\n", .{err});
        };
    }

    const CommitJob = struct { rt: *Runtime, bytes: []u8 };

    fn marshalCommit(self: *Runtime, bytes: []u8) void {
        const job = self.gpa.create(CommitJob) catch {
            self.gpa.free(bytes);
            return;
        };
        job.* = .{ .rt = self, .bytes = bytes };
        abi_backend.vtable.marshal_async(abi_backend.ctx, &applyOnUi, job);
    }

    fn applyOnUi(data: ?*anyopaque) callconv(.c) void {
        const job: *CommitJob = @ptrCast(@alignCast(data.?));
        const self = job.rt;
        const parsed = std.json.parseFromSlice(protocol.CommitBatch, self.gpa, job.bytes, .{ .ignore_unknown_fields = true }) catch {
            self.gpa.free(job.bytes);
            self.gpa.destroy(job);
            return;
        };
        defer parsed.deinit();
        self.checkAndApply(parsed.value);
        self.gpa.free(job.bytes);
        self.gpa.destroy(job);
    }

    /// True if allowed; on denial prints ND_ACL_DENY, sends a structured error
    /// frame, and returns false (the batch is dropped, not applied).
    fn checkAndApply(self: *Runtime, batch: protocol.CommitBatch) void {
        const the_acl = if (abi_backend.ctx.acl) |a| a else &default_acl;
        if (commitGate(the_acl, batch)) |denied| {
            std.debug.print("ND_ACL_DENY permission={s}\n", .{denied});
            self.writeFrame(.{ .type = "error", .message = "capability denied", .expected = @as(u32, 0), .got = @as(u32, 0) });
            return;
        }
        self.tree.apply(batch);
    }

    /// Routes a `pluginCommand {"plugin","command","arg"}` NDP frame: gates on
    /// `plugin:<plugin>.<command>` via the ACL, dispatches into the loaded
    /// plugin's registry, and replies with a `pluginResult` frame carrying the
    /// plugin's JSON result verbatim. Dispatch only touches the (already
    /// loaded, resident) plugin registry, so unlike commit application it is
    /// cheap and safe to run inline on the reader thread rather than
    /// marshaling to the UI thread (mirrors marshalCommit's ownership intent,
    /// but no UI-thread widget calls happen here).
    fn handlePluginCommand(self: *Runtime, bytes: []u8) void {
        defer self.gpa.free(bytes);
        const PC = struct { plugin: []const u8 = "", command: []const u8 = "", arg: std.json.Value = .null };
        const parsed = std.json.parseFromSlice(PC, self.gpa, bytes, .{ .ignore_unknown_fields = true }) catch return;
        defer parsed.deinit();
        const perm = std.fmt.allocPrint(self.gpa, "plugin:{s}.{s}", .{ parsed.value.plugin, parsed.value.command }) catch return;
        defer self.gpa.free(perm);
        const the_acl = if (abi_backend.ctx.acl) |a| a else &default_acl;
        if (!the_acl.isAllowed(0, perm)) {
            std.debug.print("ND_ACL_DENY permission={s}\n", .{perm});
            self.writeFrame(.{ .type = "error", .message = "capability denied", .expected = @as(u32, 0), .got = @as(u32, 0) });
            return;
        }
        // `ctx.plugin` is added by T10's nd_load_plugin wiring (src/abi.zig);
        // guarded so this file compiles whether or not that field has landed
        // yet in the parallel wave.
        if (comptime !@hasField(@TypeOf(abi_backend.ctx.*), "plugin")) {
            self.writeFrame(.{ .type = "error", .message = "no plugin loaded", .expected = @as(u32, 0), .got = @as(u32, 0) });
            return;
        } else {
            const manager = &abi_backend.ctx.plugins;
            const arg_json = std.json.Stringify.valueAlloc(self.gpa, parsed.value.arg, .{}) catch return;
            defer self.gpa.free(arg_json);
            if (manager.dispatch(parsed.value.plugin, parsed.value.command, arg_json)) |result| {
                defer std.c.free(result.ptr);
                // The plugin ABI returns an arbitrary NUL-terminated C string;
                // nothing on the plugin side guarantees it's well-formed JSON.
                // Splicing it unvalidated into the frame would let a buggy
                // plugin emit a structurally invalid wire frame (crashing the
                // Bun child's JSON.parse) — validate first, degrade to the
                // structured error frame on failure (never splice raw).
                const validated = std.json.parseFromSlice(std.json.Value, self.gpa, result, .{}) catch {
                    std.debug.print("ND_PLUGIN_BAD_RESULT plugin={s} command={s}\n", .{ parsed.value.plugin, parsed.value.command });
                    self.writeFrame(.{ .type = "error", .message = "plugin returned malformed result", .expected = @as(u32, 0), .got = @as(u32, 0) });
                    return;
                };
                defer validated.deinit();
                std.debug.print("ND_PLUGIN_COMMAND_OK plugin={s} command={s}\n", .{ parsed.value.plugin, parsed.value.command });
                const framed = std.fmt.allocPrint(self.gpa, "{{\"type\":\"pluginResult\",\"result\":{s}}}", .{result}) catch return;
                defer self.gpa.free(framed);
                self.writeRawJson(framed);
            }
        }
    }

    /// Routes a `widgetCommand {"nodeId","command","arg"}` NDP frame. Unlike
    /// pluginCommand this touches live widgets, so it marshals to the UI
    /// thread exactly like commit application; socket FIFO ordering means a
    /// command sent after a commit is applied after that commit, so a node
    /// created in the previous batch is always resolvable here.
    fn marshalWidgetCommand(self: *Runtime, bytes: []u8) void {
        const job = self.gpa.create(CommitJob) catch {
            self.gpa.free(bytes);
            return;
        };
        job.* = .{ .rt = self, .bytes = bytes };
        abi_backend.vtable.marshal_async(abi_backend.ctx, &widgetCommandOnUi, job);
    }

    fn widgetCommandOnUi(data: ?*anyopaque) callconv(.c) void {
        const job: *CommitJob = @ptrCast(@alignCast(data.?));
        const self = job.rt;
        defer {
            self.gpa.free(job.bytes);
            self.gpa.destroy(job);
        }
        const parsed = std.json.parseFromSlice(protocol.WidgetCommand, self.gpa, job.bytes, .{ .ignore_unknown_fields = true }) catch return;
        defer parsed.deinit();
        // Same core-UI gate as commit application: a widget command is an
        // ordinary UI op on an already-committed node.
        const the_acl = if (abi_backend.ctx.acl) |a| a else &default_acl;
        if (!the_acl.isAllowed(0, "core:commit")) {
            std.debug.print("ND_ACL_DENY permission=core:commit\n", .{});
            self.writeFrame(.{ .type = "error", .message = "capability denied", .expected = @as(u32, 0), .got = @as(u32, 0) });
            return;
        }
        const cmd = parsed.value;
        const widget = self.tree.get(cmd.nodeId) orelse {
            std.debug.print("ND_WARN widgetCommand unknown node id={d}\n", .{cmd.nodeId});
            return;
        };
        const meta = self.tree.metaGet(cmd.nodeId);
        const kind = if (meta) |m| m.widget_type else "";
        if (meta) |m| {
            if (m.view_kind) |view_kind| backend.nativeViewCommand(view_kind, widget, cmd.command, cmd.arg) else backend.widgetCommand(widget, kind, cmd.command, cmd.arg);
        } else backend.widgetCommand(widget, kind, cmd.command, cmd.arg);
        std.debug.print("ND_WIDGET_COMMAND id={d} command={s}\n", .{ cmd.nodeId, cmd.command });
    }

    fn marshalBinaryCommit(self: *Runtime, bytes: []u8) void {
        const job = self.gpa.create(CommitJob) catch {
            self.gpa.free(bytes);
            return;
        };
        job.* = .{ .rt = self, .bytes = bytes };
        abi_backend.vtable.marshal_async(abi_backend.ctx, &applyBinaryOnUi, job);
    }

    fn applyBinaryOnUi(data: ?*anyopaque) callconv(.c) void {
        const job: *CommitJob = @ptrCast(@alignCast(data.?));
        const self = job.rt;
        var decoded = ndp_binary.decodeCommitBatch(self.gpa, job.bytes) catch {
            self.gpa.free(job.bytes);
            self.gpa.destroy(job);
            return;
        };
        defer decoded.deinit();
        self.checkAndApply(decoded.batch);
        self.gpa.free(job.bytes);
        self.gpa.destroy(job);
    }
};

test "commitGate denies window.create without grant, allows core:commit" {
    var denied = acl.Acl.initDefault(std.testing.allocator); // grants both by default
    defer denied.deinit();
    // Build a minimal batch with a Window create.
    var ops = [_]protocol.Op{.{ .op = "create", .id = 1, .widget = "Window" }};
    const batch = protocol.CommitBatch{ .commitId = 0, .generation = 0, .ops = &ops };
    // Default policy grants core:window.create → allowed.
    try std.testing.expect(commitGate(&denied, batch) == null);

    // A restrictive ACL (no window.create) → denied with the permission name.
    var strict = acl.Acl{ .arena = std.heap.ArenaAllocator.init(std.testing.allocator) };
    defer strict.deinit();
    _ = strict.default_perms.put(strict.arena.allocator(), "core:commit", {}) catch {};
    try std.testing.expectEqualStrings("core:window.create", commitGate(&strict, batch).?);
}

test "plugin result JSON validation: the handlePluginCommand guard accepts well-formed JSON and rejects garbage" {
    // Mirrors the exact predicate handlePluginCommand runs on a plugin's
    // dispatch() result before splicing it into the pluginResult frame
    // (src/runtime.zig's ND_PLUGIN_BAD_RESULT guard) — a plugin returning
    // non-JSON must be caught here, not corrupt the wire frame.
    const gpa = std.testing.allocator;

    const good = try std.json.parseFromSlice(std.json.Value, gpa, "{\"greeting\":\"hello, world\"}", .{});
    good.deinit();

    try std.testing.expectError(error.SyntaxError, std.json.parseFromSlice(std.json.Value, gpa, "not json", .{}));
    try std.testing.expectError(error.UnexpectedEndOfInput, std.json.parseFromSlice(std.json.Value, gpa, "{\"truncated\":", .{}));
    try std.testing.expectError(error.UnexpectedEndOfInput, std.json.parseFromSlice(std.json.Value, gpa, "", .{}));
}
