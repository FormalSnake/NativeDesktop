const std = @import("std");
const protocol = @import("protocol.zig");
const Tree = @import("tree.zig").Tree;
const backend = @import("backend.zig").impl;
const acl = @import("acl.zig");
const plugin = @import("plugin.zig");
const ndp_binary = @import("ndp_binary.zig");
// The UI-thread marshal + crash-overlay chrome are vtable-only concerns (not
// part of the comptime null|abi seam every backend implements) — the core
// calls them straight through the registered C vtable, the same object
// `abi_backend.bind` already wired for the structural seam calls.
const abi_backend = @import("abi_backend.zig");

var trace: bool = false;

// Used when the embedder never called `nd_set_acl`: the safe default
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
                // Multi-window: gate each Window create against ITS OWN target
                // window id (the node id being created), not a hardcoded 0, so a
                // grants manifest can scope window.create per window. window-0
                // grants still apply everywhere (Acl.isAllowed treats 0 as
                // "all windows"), so the default policy is unchanged.
                const wid = op.id orelse 0;
                if (!a.isAllowed(wid, "core:window.create")) return "core:window.create";
            };
        }
    }
    return null;
}

/// Maps a systemRequest `method` to the `core:<group>` capability that gates
/// it, or null for an unknown method (the caller answers ok=false rather than
/// dispatching). Coarse per-group gating — one capability per capability
/// family, not per method — matching how the ACL grants are written.
fn systemMethodCapability(method: []const u8) ?[]const u8 {
    if (std.mem.startsWith(u8, method, "dialog.")) return "core:dialog";
    if (std.mem.eql(u8, method, "clipboard.readText")) return "core:clipboard.read";
    if (std.mem.eql(u8, method, "clipboard.readImage")) return "core:clipboard.read.image";
    if (std.mem.eql(u8, method, "clipboard.writeText")) return "core:clipboard.write";
    if (std.mem.startsWith(u8, method, "notification.")) return "core:notification";
    if (std.mem.startsWith(u8, method, "recent.")) return "core:recent";
    if (std.mem.startsWith(u8, method, "credentials.")) return "core:credentials";
    if (std.mem.startsWith(u8, method, "audio.")) return "core:audio";
    if (std.mem.eql(u8, method, "system.getAppearance")) return "core:system";
    return null;
}

/// Renders the reply frame JSON for an ACL-allowed pluginCommand (caller
/// frees; null only on OOM): a `pluginResult` splicing the plugin's validated
/// JSON, or an `error` frame when dispatch found no handler (unknown plugin/
/// command or the handler failed) or the result was malformed. protocol.ts
/// documents pluginResult as pluginCommand's reply, so every path must
/// produce a frame — a silent drop strands the child's request.
fn pluginCommandReplyJson(gpa: std.mem.Allocator, manager: *plugin.Manager, plugin_name: []const u8, command: []const u8, arg_json: []const u8) ?[]u8 {
    const result = manager.dispatch(plugin_name, command, arg_json) orelse {
        std.debug.print("ND_PLUGIN_NO_HANDLER plugin={s} command={s}\n", .{ plugin_name, command });
        const msg = std.fmt.allocPrint(gpa, "no handler for plugin command {s}.{s}", .{ plugin_name, command }) catch return null;
        defer gpa.free(msg);
        return std.json.Stringify.valueAlloc(gpa, .{ .type = "error", .message = msg, .expected = @as(u32, 0), .got = @as(u32, 0) }, .{}) catch null;
    };
    defer std.c.free(result.ptr);
    // The plugin ABI returns an arbitrary NUL-terminated C string; nothing on
    // the plugin side guarantees it's well-formed JSON. Splicing it
    // unvalidated into the frame would let a buggy plugin emit a structurally
    // invalid wire frame (crashing the Bun child's JSON.parse) — validate
    // first, degrade to the structured error frame on failure (never splice
    // raw).
    const validated = std.json.parseFromSlice(std.json.Value, gpa, result, .{}) catch {
        std.debug.print("ND_PLUGIN_BAD_RESULT plugin={s} command={s}\n", .{ plugin_name, command });
        return std.json.Stringify.valueAlloc(gpa, .{ .type = "error", .message = "plugin returned malformed result", .expected = @as(u32, 0), .got = @as(u32, 0) }, .{}) catch null;
    };
    defer validated.deinit();
    std.debug.print("ND_PLUGIN_COMMAND_OK plugin={s} command={s}\n", .{ plugin_name, command });
    return std.fmt.allocPrint(gpa, "{{\"type\":\"pluginResult\",\"result\":{s}}}", .{result}) catch null;
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
    // ND_DEV=1: gates the `--hot` spawn flag and the overlay's
    // Restart button ONLY. The overlay itself paints regardless.
    dev: bool = false,
    parent_env: *const std.process.Environ.Map = undefined,
    real_environ: std.process.Environ = undefined,
    // Stashed by the `runtimeError` reader arm; consumed by the crash overlay
    // when the imminent disconnect fires. Freed/replaced on each report.
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
    /// `bun <script>`. Factored out so a dev-mode Restart can respawn a
    /// fresh child against the same listening socket.
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
    /// event channel. There is no core-installed sink function pointer;
    /// the embedder calls `nd_emit_event` directly for every native signal,
    /// including its own style-error reports. A no-op before `start` has run
    /// (no window mounted yet, nothing to report to).
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
            } else if (std.mem.eql(u8, kind, "systemRequest")) {
                self.handleSystemRequest(bytes); // frees bytes; marshals a dup'd job
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

    /// Parses a `runtimeError {message, stack, fatal}` frame. fatal=true:
    /// stashes both into `last_error_message`/`last_error_stack`, which the
    /// crash overlay reads when the imminent disconnect triggers
    /// `onChildExit`. fatal=false: the child survived; log only, never stash
    /// (a stale non-fatal message must not become overlay text on a later
    /// crash). Best-effort: a malformed frame is dropped, never crashes the
    /// reader loop. `fatal` defaults true so a frame missing the field takes
    /// the conservative (overlay) path.
    fn stashRuntimeError(self: *Runtime, bytes: []u8) void {
        defer self.gpa.free(bytes);
        const RE = struct { message: []const u8 = "", stack: []const u8 = "", fatal: bool = true };
        const parsed = std.json.parseFromSlice(RE, self.gpa, bytes, .{ .ignore_unknown_fields = true }) catch return;
        defer parsed.deinit();
        if (!parsed.value.fatal) {
            std.debug.print("ND_RUNTIME_ERROR_NONFATAL {s}\n", .{parsed.value.message});
            return;
        }
        if (self.last_error_message) |m| self.gpa.free(m);
        if (self.last_error_stack) |s| self.gpa.free(s);
        self.last_error_message = self.gpa.dupe(u8, parsed.value.message) catch null;
        self.last_error_stack = self.gpa.dupe(u8, parsed.value.stack) catch null;
        std.debug.print("ND_RUNTIME_ERROR_REPORTED {s}\n", .{parsed.value.message});
    }

    /// Called from the reader-loop thread on every disconnect path (a dead
    /// `bytes` read means the child actually exited: crash, kill -9, or
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
    /// `show_overlay`'s ABI carries only the message) calls
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
        // Empty message is the clear sentinel: `show_overlay`'s
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
        const arg_json = std.json.Stringify.valueAlloc(self.gpa, parsed.value.arg, .{}) catch return;
        defer self.gpa.free(arg_json);
        // Direct field access (no @hasField gate): a `plugins` rename must
        // break the build here, not silently compile dispatch out.
        const reply = pluginCommandReplyJson(self.gpa, &abi_backend.ctx.plugins, parsed.value.plugin, parsed.value.command, arg_json) orelse return;
        defer self.gpa.free(reply);
        self.writeRawJson(reply);
    }

    const SystemRequestJob = struct { rt: *Runtime, id: u32, method: [:0]u8, params: [:0]u8 };

    /// Routes a `systemRequest {"id","method","params"}` NDP frame: resolves the
    /// method's `core:<group>` capability, gates it on the same ACL the commit
    /// gate uses, then marshals the (method, params) pair to the UI thread where
    /// the backend's `system_request` vtable op runs it (dialogs, clipboard, …
    /// touch native state, so unlike pluginCommand this can't run on the reader
    /// thread). An unknown method or a denied capability answers immediately
    /// with an ok=false systemResponse rather than dispatching — every request
    /// gets exactly one reply (the backend later calls `nd_system_response` for
    /// the dispatched ones).
    fn handleSystemRequest(self: *Runtime, bytes: []u8) void {
        defer self.gpa.free(bytes);
        const parsed = std.json.parseFromSlice(protocol.SystemRequest, self.gpa, bytes, .{ .ignore_unknown_fields = true }) catch return;
        defer parsed.deinit();
        const req = parsed.value;
        const cap = systemMethodCapability(req.method) orelse {
            sendSystemResponse(req.id, false, "unknown method");
            return;
        };
        const the_acl = if (abi_backend.ctx.acl) |a| a else &default_acl;
        if (!the_acl.isAllowed(0, cap)) {
            std.debug.print("ND_ACL_DENY permission={s}\n", .{cap});
            sendSystemResponse(req.id, false, "capability denied");
            return;
        }
        // `req.method`/`req.params` borrow from `bytes` (freed on return) —
        // dupe both into a heap job before marshaling; `params` re-serializes
        // the parsed value back to the JSON string the backend receives.
        const params_json = std.json.Stringify.valueAlloc(self.gpa, req.params, .{}) catch return;
        defer self.gpa.free(params_json);
        const method_z = self.gpa.dupeZ(u8, req.method) catch return;
        const params_z = self.gpa.dupeZ(u8, params_json) catch {
            self.gpa.free(method_z);
            return;
        };
        const job = self.gpa.create(SystemRequestJob) catch {
            self.gpa.free(method_z);
            self.gpa.free(params_z);
            return;
        };
        job.* = .{ .rt = self, .id = req.id, .method = method_z, .params = params_z };
        abi_backend.vtable.marshal_async(abi_backend.ctx, &systemRequestOnUi, job);
    }

    fn systemRequestOnUi(data: ?*anyopaque) callconv(.c) void {
        const job: *SystemRequestJob = @ptrCast(@alignCast(data.?));
        const self = job.rt;
        defer {
            self.gpa.free(job.method);
            self.gpa.free(job.params);
            self.gpa.destroy(job);
        }
        abi_backend.vtable.system_request(abi_backend.ctx, job.id, job.method.ptr, job.params.ptr);
    }

    /// backend -> core result channel for a dispatched systemRequest
    /// (`nd_system_response`, abi.zig routes here), correlated by `id`. ok=true
    /// splices the backend's JSON `result` verbatim (structurally typed, like
    /// pluginResult) after validating it parses — a malformed result degrades
    /// to an ok=false "backend returned malformed result" reply rather than a
    /// broken wire frame. ok=false carries `json` as the `errorMessage` string
    /// (the failure field is `errorMessage`, not `error`, since the generated
    /// host struct can't name a Zig keyword).
    pub fn sendSystemResponse(id: u32, ok: bool, json: []const u8) void {
        const self = singleton orelse return;
        if (ok) {
            const validated = std.json.parseFromSlice(std.json.Value, self.gpa, json, .{}) catch {
                std.debug.print("ND_SYSTEM_BAD_RESULT id={d}\n", .{id});
                return sendSystemResponse(id, false, "backend returned malformed result");
            };
            validated.deinit();
            const frame = std.fmt.allocPrint(self.gpa, "{{\"type\":\"systemResponse\",\"id\":{d},\"ok\":true,\"result\":{s}}}", .{ id, json }) catch return;
            defer self.gpa.free(frame);
            self.writeRawJson(frame);
        } else {
            self.writeFrameOpts(protocol.SystemResponse{ .id = id, .ok = false, .errorMessage = json }, .{ .emit_null_optional_fields = false });
        }
    }

    /// backend -> core app-level event channel (`nd_system_event`, abi.zig
    /// routes here): validates `data_json` parses, JSON-escapes `channel`, and
    /// splices `{"type":"systemEvent","channel":<chan>,"data":<data>}` onto the
    /// wire. A malformed payload is dropped with a diagnostic rather than
    /// corrupting the frame.
    pub fn sendSystemEvent(channel: []const u8, data_json: []const u8) void {
        const self = singleton orelse return;
        const validated = std.json.parseFromSlice(std.json.Value, self.gpa, data_json, .{}) catch {
            std.debug.print("ND_SYSTEM_BAD_EVENT channel={s}\n", .{channel});
            return;
        };
        validated.deinit();
        const chan = std.json.Stringify.valueAlloc(self.gpa, channel, .{}) catch return;
        defer self.gpa.free(chan);
        const frame = std.fmt.allocPrint(self.gpa, "{{\"type\":\"systemEvent\",\"channel\":{s},\"data\":{s}}}", .{ chan, data_json }) catch return;
        defer self.gpa.free(frame);
        self.writeRawJson(frame);
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
        // Reserved sentinel command (peer of nd_emit_event's "restart"): the
        // app's `moveNode()` rides the widgetCommand frame to request a
        // widget-preserving cross-window move rather than a per-widget schema
        // command, so no protocol/schema change is needed. `arg` carries the
        // target `{parent, before?}` node ids.
        if (std.mem.eql(u8, cmd.command, "__ndReparent")) {
            handleReparent(self.tree, cmd.nodeId, cmd.arg);
            return;
        }
        const widget = self.tree.get(cmd.nodeId) orelse {
            std.debug.print("ND_WARN widgetCommand unknown node id={d}\n", .{cmd.nodeId});
            return;
        };
        const meta = self.tree.metaGet(cmd.nodeId);
        const kind = if (meta) |m| m.widget_type else "";
        const view_kind = if (meta) |m| m.view_kind else null;
        if (view_kind) |vk| backend.nativeViewCommand(vk, widget, cmd.command, cmd.arg) else backend.widgetCommand(widget, kind, cmd.command, cmd.arg);
        std.debug.print("ND_WIDGET_COMMAND id={d} command={s}\n", .{ cmd.nodeId, cmd.command });
    }

    /// Decodes the `{parent, before?}` arg of a `"__ndReparent"` widgetCommand
    /// and drives the widget-preserving cross-window move (`Tree.reparent`). A
    /// missing/malformed parent id drops the move rather than crashing the UI
    /// thread — same defensive posture as the rest of the command path.
    fn handleReparent(tree: *Tree, child_id: u32, arg: std.json.Value) void {
        if (arg != .object) return;
        const parent_v = arg.object.get("parent") orelse return;
        const parent_id: u32 = switch (parent_v) {
            .integer => std.math.cast(u32, parent_v.integer) orelse return,
            else => return,
        };
        const before_id: ?u32 = if (arg.object.get("before")) |b| switch (b) {
            .integer => std.math.cast(u32, b.integer),
            else => null,
        } else null;
        tree.reparent(child_id, parent_id, before_id);
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

test "commitGate keys window.create by the target window id (per-window grant)" {
    // A restrictive ACL that grants window.create to window 5 ONLY (not in the
    // global default — the default policy would otherwise grant it everywhere).
    // Creating window 5 is allowed; creating window 7 is denied. Proves the gate
    // keys on each Window create op's own node id, not a hardcoded 0.
    var acl_scoped = acl.Acl{ .arena = std.heap.ArenaAllocator.init(std.testing.allocator) };
    defer acl_scoped.deinit();
    const a = acl_scoped.arena.allocator();
    _ = acl_scoped.default_perms.put(a, "core:commit", {}) catch {};
    const gop = acl_scoped.per_window.getOrPut(a, 5) catch unreachable;
    if (!gop.found_existing) gop.value_ptr.* = .{};
    _ = gop.value_ptr.put(a, "core:window.create", {}) catch {};

    var ops5 = [_]protocol.Op{.{ .op = "create", .id = 5, .widget = "Window" }};
    const batch5 = protocol.CommitBatch{ .commitId = 0, .generation = 0, .ops = &ops5 };
    try std.testing.expect(commitGate(&acl_scoped, batch5) == null); // window 5 granted

    var ops7 = [_]protocol.Op{.{ .op = "create", .id = 7, .widget = "Window" }};
    const batch7 = protocol.CommitBatch{ .commitId = 0, .generation = 0, .ops = &ops7 };
    try std.testing.expectEqualStrings("core:window.create", commitGate(&acl_scoped, batch7).?); // window 7 denied
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

test "pluginCommand dispatch miss still answers with an error frame naming the command" {
    const gpa = std.testing.allocator;
    var manager = plugin.Manager.init(gpa, null, null);
    defer manager.deinit();
    // Nothing loaded (same null dispatch as a typo'd plugin/command name):
    // the child must still get a reply frame, never silence.
    const reply = pluginCommandReplyJson(gpa, &manager, "hello", "greet", "{}").?;
    defer gpa.free(reply);
    try std.testing.expect(std.mem.indexOf(u8, reply, "\"type\":\"error\"") != null);
    try std.testing.expect(std.mem.indexOf(u8, reply, "hello.greet") != null);
}

test "systemMethodCapability maps each capability group and denies unknown methods" {
    try std.testing.expectEqualStrings("core:dialog", systemMethodCapability("dialog.openFile").?);
    try std.testing.expectEqualStrings("core:clipboard.read", systemMethodCapability("clipboard.readText").?);
    try std.testing.expectEqualStrings("core:clipboard.write", systemMethodCapability("clipboard.writeText").?);
    try std.testing.expectEqualStrings("core:notification", systemMethodCapability("notification.show").?);
    try std.testing.expectEqualStrings("core:recent", systemMethodCapability("recent.add").?);
    try std.testing.expectEqualStrings("core:credentials", systemMethodCapability("credentials.get").?);
    try std.testing.expectEqualStrings("core:audio", systemMethodCapability("audio.play").?);
    try std.testing.expectEqualStrings("core:system", systemMethodCapability("system.getAppearance").?);
    // The clipboard split is per-method: only the two named methods map.
    try std.testing.expect(systemMethodCapability("clipboard.clear") == null);
    try std.testing.expect(systemMethodCapability("unknown.method") == null);
    try std.testing.expect(systemMethodCapability("") == null);
}
