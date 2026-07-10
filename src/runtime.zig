const std = @import("std");
const glib = @import("glib");
const gio = @import("gio");
const gtk = @import("gtk");
const protocol = @import("protocol.zig");
const Tree = @import("tree.zig").Tree;
const backend = @import("gtk_backend.zig");
const overlay = @import("overlay.zig");

const G_SOURCE_REMOVE: c_int = 0;

var trace: bool = false;

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
        app: *gtk.Application,
        tree: *Tree,
        parent_env: *const std.process.Environ.Map,
        real_environ: std.process.Environ,
    ) !*Runtime {
        _ = app; // window is owned by the child's `create Window` op via gtk_backend/tree
        trace = parent_env.get("NDP_TRACE") != null;

        const self = try gpa.create(Runtime);
        self.* = undefined;
        self.gpa = gpa;
        self.tree = tree;
        self.writer_mutex = .init;
        self.seq = 0;
        self.last_error_message = null;
        self.last_error_stack = null;
        self.overlay_shown = false;
        self.parent_env = parent_env;
        self.real_environ = real_environ;
        self.dev = if (parent_env.get("ND_DEV")) |v| std.mem.eql(u8, v, "1") else false;

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
        backend.setEventSink(&sendEventStatic);
        backend.initStyle(&sendStyleErrorStatic);

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

    fn sendEventStatic(node_id: u32, name: []const u8, payload: protocol.EventPayload) void {
        if (singleton) |self| self.sendEvent(node_id, name, payload);
    }

    /// Host-side defensive layer (M5c-D7): an unknown style key never crashes
    /// or drops silently — it fires a structured `styleError` event alongside
    /// the ND_WARN stderr line style.zig already printed.
    fn sendStyleErrorStatic(node_id: u32, key: []const u8) void {
        if (singleton) |self| self.sendEvent(node_id, "styleError", .{ .key = key });
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
        self.writeFrame(protocol.HelloAck{ .ndpVersion = protocol.ndp_version, .encodings = &.{"json"} });
        std.debug.print("ND_HELLO_OK\n", .{});

        // Frame loop.
        while (true) {
            const bytes = readFrame(self, &r.interface) catch {
                self.onChildExit();
                return;
            };
            const kind = protocol.peekType(self.gpa, bytes) catch {
                self.gpa.free(bytes);
                continue;
            };
            defer self.gpa.free(kind);

            if (std.mem.eql(u8, kind, "commitBatch")) {
                // Ownership of `bytes` transfers to the marshaled closure.
                self.marshalCommit(bytes);
            } else if (std.mem.eql(u8, kind, "ping")) {
                self.gpa.free(bytes);
                self.writeFrame(.{ .type = "pong" });
            } else if (std.mem.eql(u8, kind, "runtimeError")) {
                self.stashRuntimeError(bytes);
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
        if (trace) std.debug.print(">> {s}\n", .{payload});
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
        _ = glib.MainContext.default().invokeFull(glib.PRIORITY_DEFAULT, &showOverlayOnUi, job, null);
    }

    fn showOverlayOnUi(data: ?*anyopaque) callconv(.c) c_int {
        const OverlayJob = struct { rt: *Runtime, msg: []const u8 };
        const job: *OverlayJob = @ptrCast(@alignCast(data.?));
        defer {
            job.rt.gpa.free(job.msg);
            job.rt.gpa.destroy(job);
        }
        const self = job.rt;
        const window = backend.getWindow() orelse return G_SOURCE_REMOVE; // no window yet (child never mounted)
        self.overlay_shown = true;
        overlay.show(self.tree, window, job.msg, self.dev, &respawnStatic);
        return G_SOURCE_REMOVE;
    }

    /// Restart button trampoline (dev-mode only — overlay.show only wires
    /// this when `dev` is true). Runs via glib.idleAdd from the click
    /// handler (overlay.zig) to avoid re-entrancy into the GTK signal
    /// dispatch that is still on the stack.
    fn respawnStatic() void {
        if (singleton) |self| self.respawn();
    }

    /// Clears the crash overlay and spawns a fresh child against the same
    /// listening socket. The respawned child starts at generation 0 again;
    /// since the dead child's nodes were never GC'd (the process died, it
    /// never sent a higher-generation CommitBatch), every non-overlay node
    /// is cleared here so the fresh mount rebuilds from an empty tree.
    fn respawn(self: *Runtime) void {
        if (backend.getWindow()) |window| overlay.clear(self.tree, window);
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
        _ = glib.MainContext.default().invokeFull(glib.PRIORITY_DEFAULT, &applyOnUi, job, null);
    }

    fn applyOnUi(data: ?*anyopaque) callconv(.c) c_int {
        const job: *CommitJob = @ptrCast(@alignCast(data.?));
        const self = job.rt;
        const parsed = std.json.parseFromSlice(protocol.CommitBatch, self.gpa, job.bytes, .{ .ignore_unknown_fields = true }) catch {
            self.gpa.free(job.bytes);
            self.gpa.destroy(job);
            return G_SOURCE_REMOVE;
        };
        defer parsed.deinit();
        self.tree.apply(parsed.value);
        self.gpa.free(job.bytes);
        self.gpa.destroy(job);
        return G_SOURCE_REMOVE;
    }
};
