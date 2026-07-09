const std = @import("std");
const glib = @import("glib");
const gio = @import("gio");
const gtk = @import("gtk");
const protocol = @import("protocol.zig");
const Tree = @import("tree.zig").Tree;
const backend = @import("gtk_backend.zig");

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

        // Build the child's environment: inherit the parent's, then overlay ND_SOCKET.
        var env = std.process.Environ.Map.init(gpa);
        for (parent_env.keys(), parent_env.values()) |k, v| try env.put(k, v);
        try env.put("ND_SOCKET", self.sock_path);
        const script = parent_env.get("ND_SCRIPT") orelse "runtime/m2-demo.ts";
        self.child = try std.process.spawn(self.io, .{
            .argv = &.{ "bun", script },
            .environ_map = &env,
            .stdin = .inherit,
            .stdout = .inherit,
            .stderr = .inherit,
        });
        env.deinit();

        singleton = self;
        backend.setEventSink(&sendEventStatic);

        _ = try std.Thread.spawn(.{}, readerLoop, .{self});
        return self;
    }

    pub fn getIo(self: *Runtime) std.Io {
        return self.io;
    }

    fn sendEventStatic(node_id: u32) void {
        if (singleton) |self| self.sendEvent(node_id);
    }

    pub fn sendEvent(self: *Runtime, node_id: u32) void {
        self.seq += 1;
        const ev = protocol.Event{ .seq = self.seq, .nodeId = node_id, .name = "clicked" };
        self.writeFrame(ev);
    }

    fn writeFrame(self: *Runtime, value: anytype) void {
        const frame = protocol.encodeFrame(self.gpa, value) catch return;
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
            std.debug.print("ND_CHILD_EXITED\n", .{});
            return;
        };
        self.stream = stream;
        std.debug.print("ND_CHILD_CONNECTED\n", .{});

        var read_buf: [64 * 1024]u8 = undefined;
        var r = stream.reader(self.io, &read_buf);

        // Handshake.
        const first = readFrame(self, &r.interface) catch {
            std.debug.print("ND_CHILD_EXITED\n", .{});
            return;
        };
        defer self.gpa.free(first);
        {
            const parsed = std.json.parseFromSlice(protocol.Hello, self.gpa, first, .{ .ignore_unknown_fields = true }) catch {
                std.debug.print("ND_CHILD_EXITED\n", .{});
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
                std.debug.print("ND_CHILD_EXITED\n", .{});
                return;
            }
        }
        self.writeFrame(protocol.HelloAck{ .ndpVersion = protocol.ndp_version, .encodings = &.{"json"} });
        std.debug.print("ND_HELLO_OK\n", .{});

        // Frame loop.
        while (true) {
            const bytes = readFrame(self, &r.interface) catch {
                std.debug.print("ND_CHILD_EXITED\n", .{});
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
