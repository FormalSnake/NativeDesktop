//! Remote-terminal transport — the `ndremote` C ABI (include/ndremote.h).
//!
//! Connects to a Canary byte-plane daemon (docs/protocol.md §2), AUTHs, ATTACHes
//! sessions, and pumps their streamed PTY output into a VIRTUAL ndterm
//! (src/core/terminal.zig `ndterm_open_virtual`). Keystrokes and VT query
//! responses go back over the wire as INPUT frames. `src/core/root.zig`
//! force-retains the `ndrt_*` exports into `libnd.a`.
//!
//! GTK-free / AppKit-free, exactly like terminal.zig: `std` plus hand-declared
//! `extern "c"` sockets + libc. The framing below is a standalone Zig port of
//! `packages/protocol/src/frames.ts` — ND does NOT depend on the canary repo.
//!
//! Threading. One reader thread per Connection owns all socket reads + the
//! reconnect loop; it is the sole feeder of every channel's VT. Locks nest in a
//! single global order — registry_mutex -> state_mutex -> ndterm mutex ->
//! write_mutex — so there are no cycles:
//!   - registry_mutex guards the process-global host:port -> Connection map.
//!   - Connection.state_mutex guards the channel maps + auth/backoff state, and
//!     is held across a channel's VT feed so a concurrent ndrt_close cannot free
//!     a channel mid-feed.
//!   - the ndterm handle's own mutex serializes feed/reset (reader) against
//!     render_lock/write_input/resize (surface UI thread) — no extra lock needed
//!     for VT consistency.
//!   - Connection.write_mutex serializes socket writes (reader PONG/ACK vs
//!     surface INPUT/RESIZE/DETACH).
//! effect_cb/state_cb fire on the reader thread; emitting an NDP event there is
//! safe, but touching a GTK/AppKit widget is not — surfaces must marshal.

const std = @import("std");
const builtin = @import("builtin");
const ndterm = @import("terminal.zig");

const gpa = std.heap.c_allocator;

// ---------------------------------------------------------------------------
// C ABI callback types (include/ndremote.h + ndterm.h).
// ---------------------------------------------------------------------------

/// ndterm.h `nd_term_effect_cb`. kind: 0 title, 1 bell, 2 exit (code).
const EffectCb = *const fn (userdata: ?*anyopaque, kind: c_int, text: ?[*:0]const u8, code: c_int) callconv(.c) void;
/// ndremote.h `nd_rt_state_cb`.
const StateCb = *const fn (userdata: ?*anyopaque, state: c_int, detail: ?[*:0]const u8) callconv(.c) void;

// nd_rt_state values (include/ndremote.h).
const RT_CONNECTING: c_int = 0;
const RT_AUTHED: c_int = 1;
const RT_ATTACHED: c_int = 2;
const RT_RECONNECTING: c_int = 3;
const RT_FAILED: c_int = 4;
const RT_CLOSED: c_int = 5;

// ---------------------------------------------------------------------------
// Byte-plane framing (docs/protocol.md §2 / frames.ts).
// ---------------------------------------------------------------------------

pub const MAX_FRAME: usize = 1024 * 1024; // 1 MiB, §2

pub const FrameType = struct {
    pub const ping: u8 = 0x00;
    pub const pong: u8 = 0x01;
    pub const auth: u8 = 0x02;
    pub const auth_ok: u8 = 0x03;
    pub const auth_err: u8 = 0x04;
    pub const attach: u8 = 0x05;
    pub const attached: u8 = 0x06;
    pub const detach: u8 = 0x07;
    pub const attach_err: u8 = 0x08;
    pub const output: u8 = 0x10;
    pub const input: u8 = 0x11;
    pub const resize: u8 = 0x12;
    pub const resized: u8 = 0x13;
    pub const ack: u8 = 0x14;
    pub const exit: u8 = 0x15;
    pub const pause: u8 = 0x16;
    pub const resume_: u8 = 0x17;
};

// OUTPUT.flags bits (§2.1).
pub const FLAG_RESET: u8 = 0x01;
pub const FLAG_SNAPSHOT: u8 = 0x02;

pub const FrameError = error{BadFrame};

/// Wrap a body as `u32 payloadLen(LE) | u8 frameType | body`. Caller frees.
pub fn frameBytes(alloc: std.mem.Allocator, frame_type: u8, body: []const u8) ![]u8 {
    const payload_len = 1 + body.len;
    if (payload_len > MAX_FRAME) return FrameError.BadFrame;
    const out = try alloc.alloc(u8, 4 + payload_len);
    std.mem.writeInt(u32, out[0..4], @intCast(payload_len), .little);
    out[4] = frame_type;
    @memcpy(out[5..], body);
    return out;
}

pub const RawFrame = struct { frame_type: u8, body: []const u8 };

/// Stateful decoder for the TCP stream — a port of frames.ts `FrameDecoder`:
/// an amortized growable buffer with a read offset (no per-push full copy, no
/// per-frame remainder re-slice). `next` returns a view into the buffer that is
/// valid until the following `push`; the reader fully processes each frame
/// before pulling the next, so no push intervenes.
pub const FrameDecoder = struct {
    buf: []u8 = &.{},
    start: usize = 0,
    end: usize = 0,
    alloc: std.mem.Allocator,

    pub fn init(alloc: std.mem.Allocator) FrameDecoder {
        return .{ .alloc = alloc };
    }

    pub fn deinit(self: *FrameDecoder) void {
        if (self.buf.len != 0) self.alloc.free(self.buf);
        self.* = .{ .alloc = self.alloc };
    }

    pub fn push(self: *FrameDecoder, chunk: []const u8) !void {
        if (self.start == self.end) {
            self.start = 0;
            self.end = 0;
        }
        try self.reserve(chunk.len);
        @memcpy(self.buf[self.end..][0..chunk.len], chunk);
        self.end += chunk.len;
    }

    pub fn next(self: *FrameDecoder) !?RawFrame {
        const pending = self.end - self.start;
        if (pending < 4) return null;
        const payload_len = std.mem.readInt(u32, self.buf[self.start..][0..4], .little);
        if (payload_len < 1) return FrameError.BadFrame;
        if (payload_len > MAX_FRAME) return FrameError.BadFrame;
        const total = 4 + payload_len;
        if (pending < total) return null;
        const frame_type = self.buf[self.start + 4];
        const body = self.buf[self.start + 5 .. self.start + total];
        self.start += total;
        return .{ .frame_type = frame_type, .body = body };
    }

    fn reserve(self: *FrameDecoder, extra: usize) !void {
        if (self.end + extra <= self.buf.len) return;
        const pending = self.end - self.start;
        if (pending + extra <= self.buf.len) {
            std.mem.copyForwards(u8, self.buf[0..pending], self.buf[self.start..self.end]);
        } else {
            var cap: usize = if (self.buf.len == 0) 4096 else self.buf.len * 2;
            while (cap < pending + extra) cap *= 2;
            const next_buf = try self.alloc.alloc(u8, cap);
            @memcpy(next_buf[0..pending], self.buf[self.start..self.end]);
            if (self.buf.len != 0) self.alloc.free(self.buf);
            self.buf = next_buf;
        }
        self.end = pending;
        self.start = 0;
    }
};

// JSON body shapes (§2.1). Parsed with ignore_unknown_fields; defaults cover
// a lenient peer.
const AuthOk = struct { maxFrame: u64 = 0, ackWindowBytes: u64 = 0, heartbeatSec: u64 = 0 };
const AuthErr = struct { code: []const u8 = "", message: []const u8 = "" };
const Attached = struct {
    sessionId: []const u8 = "",
    channel: u32 = 0,
    seq: u64 = 0,
    cols: u16 = 0,
    rows: u16 = 0,
    mode: []const u8 = "",
};
const AttachErr = struct {
    sessionId: []const u8 = "",
    channel: ?u32 = null,
    code: []const u8 = "",
    message: []const u8 = "",
};

// ---------------------------------------------------------------------------
// Sockets / libc (hand-declared extern "c", per terminal.zig's approach).
// ---------------------------------------------------------------------------

const AF_UNSPEC: c_int = 0;
const SOCK_STREAM: c_int = 1;
const SHUT_RDWR: c_int = 2;

/// getaddrinfo's `struct addrinfo`. ai_canonname and ai_addr are SWAPPED
/// between Linux (glibc) and BSD/macOS — switch per target.
const addrinfo = switch (builtin.os.tag) {
    .linux => extern struct {
        flags: c_int = 0,
        family: c_int = 0,
        socktype: c_int = 0,
        protocol: c_int = 0,
        addrlen: u32 = 0,
        addr: ?*anyopaque = null,
        canonname: ?[*:0]u8 = null,
        next: ?*addrinfo = null,
    },
    else => extern struct {
        flags: c_int = 0,
        family: c_int = 0,
        socktype: c_int = 0,
        protocol: c_int = 0,
        addrlen: u32 = 0,
        canonname: ?[*:0]u8 = null,
        addr: ?*anyopaque = null,
        next: ?*addrinfo = null,
    },
};

const timespec = extern struct { sec: i64, nsec: i64 };

extern "c" fn getaddrinfo(node: ?[*:0]const u8, service: ?[*:0]const u8, hints: ?*const addrinfo, res: *?*addrinfo) c_int;
extern "c" fn freeaddrinfo(res: ?*addrinfo) void;
extern "c" fn socket(domain: c_int, socktype: c_int, protocol: c_int) c_int;
extern "c" fn connect(fd: c_int, addr: *const anyopaque, len: u32) c_int;
extern "c" fn shutdown(fd: c_int, how: c_int) c_int;
extern "c" fn read(fd: c_int, buf: [*]u8, nbyte: usize) isize;
extern "c" fn write(fd: c_int, buf: [*]const u8, nbyte: usize) isize;
extern "c" fn close(fd: c_int) c_int;
extern "c" fn nanosleep(req: *const timespec, rem: ?*timespec) c_int;

/// Blocking dial of host:port (numeric or DNS, IPv4/IPv6). Returns a connected
/// fd, or null on any failure. Tries each resolved address in turn.
fn dial(host: [:0]const u8, port: u16) ?c_int {
    var hints = std.mem.zeroes(addrinfo);
    hints.family = AF_UNSPEC;
    hints.socktype = SOCK_STREAM;

    var svc_buf: [8]u8 = undefined;
    const svc = std.fmt.bufPrintZ(&svc_buf, "{d}", .{port}) catch return null;

    var res: ?*addrinfo = null;
    if (getaddrinfo(host.ptr, svc.ptr, &hints, &res) != 0) return null;
    defer freeaddrinfo(res);

    var it = res;
    while (it) |ai| : (it = ai.next) {
        const fd = socket(ai.family, ai.socktype, ai.protocol);
        if (fd < 0) continue;
        if (ai.addr) |sa| {
            if (connect(fd, sa, ai.addrlen) == 0) return fd;
        }
        _ = close(fd);
    }
    return null;
}

fn sleepMs(ms: u64) void {
    const ts = timespec{ .sec = @intCast(ms / 1000), .nsec = @intCast((ms % 1000) * 1_000_000) };
    _ = nanosleep(&ts, null);
}

// ---------------------------------------------------------------------------
// Registry + connection/channel types.
// ---------------------------------------------------------------------------

var registry_mutex: std.c.pthread_mutex_t = .{};
var conns: std.StringHashMapUnmanaged(*Connection) = .empty;

const Channel = struct {
    conn: *Connection,
    session_id: [:0]u8, // owned
    channel: u32 = 0, // server-assigned (valid once attached_once)
    attached_once: bool = false,
    dead: std.atomic.Value(bool) = std.atomic.Value(bool).init(false),

    term: ?*align(8) anyopaque = null, // virtual ndterm handle (core Terminal is 8-aligned)
    cols: u16,
    rows: u16,
    last_seq: u64 = 0,
    unacked: u64 = 0,

    effect_cb: ?EffectCb,
    state_cb: ?StateCb,
    surface_ud: ?*anyopaque,

    fn notifyState(ch: *Channel, state: c_int, detail: ?[]const u8) void {
        const cb = ch.state_cb orelse return;
        if (detail) |d| {
            var buf: [256]u8 = undefined;
            const n = @min(d.len, buf.len - 1);
            @memcpy(buf[0..n], d[0..n]);
            buf[n] = 0;
            cb(ch.surface_ud, state, @ptrCast(&buf));
        } else {
            cb(ch.surface_ud, state, null);
        }
    }
};

const Connection = struct {
    host: [:0]u8, // owned host (for reconnect dial)
    port: u16,
    key: []u8, // owned "host:port" registry key
    ticket: [:0]u8, // owned

    fd: std.atomic.Value(c_int) = std.atomic.Value(c_int).init(-1),
    reader_thread: std.Thread = undefined,
    closing: std.atomic.Value(bool) = std.atomic.Value(bool).init(false),
    failed: std.atomic.Value(bool) = std.atomic.Value(bool).init(false),

    write_mutex: std.c.pthread_mutex_t = .{},
    state_mutex: std.c.pthread_mutex_t = .{},

    decoder: FrameDecoder,

    // Guarded by state_mutex.
    members: std.ArrayListUnmanaged(*Channel) = .empty,
    by_srv: std.AutoHashMapUnmanaged(u32, *Channel) = .empty,
    refcount: usize = 0,

    // From AUTH_OK.
    ack_window_bytes: u64 = 256 * 1024,

    rng: u64, // cheap LCG state for backoff jitter

    fn lockWrite(self: *Connection) void {
        _ = std.c.pthread_mutex_lock(&self.write_mutex);
    }
    fn unlockWrite(self: *Connection) void {
        _ = std.c.pthread_mutex_unlock(&self.write_mutex);
    }
    fn lockState(self: *Connection) void {
        _ = std.c.pthread_mutex_lock(&self.state_mutex);
    }
    fn unlockState(self: *Connection) void {
        _ = std.c.pthread_mutex_unlock(&self.state_mutex);
    }

    /// Encode `body` as a frame and write it whole under write_mutex.
    fn sendFrame(self: *Connection, frame_type: u8, body: []const u8) void {
        const bytes = frameBytes(gpa, frame_type, body) catch return;
        defer gpa.free(bytes);
        self.lockWrite();
        defer self.unlockWrite();
        const fd = self.fd.load(.seq_cst);
        if (fd < 0) return;
        var off: usize = 0;
        while (off < bytes.len) {
            const rc = write(fd, bytes.ptr + off, bytes.len - off);
            if (rc <= 0) return; // socket gone; the reader will observe EOF
            off += @intCast(rc);
        }
    }

    fn sendJson(self: *Connection, frame_type: u8, value: anytype) void {
        const json = std.json.Stringify.valueAlloc(gpa, value, .{ .emit_null_optional_fields = false }) catch return;
        defer gpa.free(json);
        self.sendFrame(frame_type, json);
    }

    fn nextJitter(self: *Connection, cap: u64) u64 {
        // xorshift64 — deterministic per-connection jitter, no time dependency.
        var x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        return x % (cap + 1);
    }
};

// ---------------------------------------------------------------------------
// ndterm trampolines (single userdata = the *Channel).
// ---------------------------------------------------------------------------

fn transportEffectTramp(userdata: ?*anyopaque, kind: c_int, text: ?[*:0]const u8, code: c_int) callconv(.c) void {
    const ch: *Channel = @ptrCast(@alignCast(userdata orelse return));
    if (ch.effect_cb) |cb| cb(ch.surface_ud, kind, text, code);
}

fn transportOutputTramp(userdata: ?*anyopaque, bytes: [*]const u8, len: usize) callconv(.c) void {
    const ch: *Channel = @ptrCast(@alignCast(userdata orelse return));
    if (!ch.attached_once) return; // no server channel yet; drop pre-attach output
    var body = gpa.alloc(u8, 4 + len) catch return;
    defer gpa.free(body);
    std.mem.writeInt(u32, body[0..4], ch.channel, .little);
    @memcpy(body[4..], bytes[0..len]);
    ch.conn.sendFrame(FrameType.input, body);
}

// ---------------------------------------------------------------------------
// Reader thread — sole socket reader + reconnect loop for one Connection.
// ---------------------------------------------------------------------------

fn readerMain(conn: *Connection) void {
    var backoff_ms: u64 = 250;
    while (!conn.closing.load(.seq_cst) and !conn.failed.load(.seq_cst)) {
        const fd = dial(conn.host, conn.port) orelse {
            markAllReconnecting(conn);
            const jitter = conn.nextJitter(backoff_ms / 2);
            sleepMs(backoff_ms + jitter);
            backoff_ms = @min(backoff_ms * 2, 30_000);
            continue;
        };
        conn.fd.store(fd, .seq_cst);
        backoff_ms = 250;
        conn.decoder.deinit(); // drop any partial frame from the prior epoch

        // AUTH; the server assigns channels in ATTACHED after AUTH_OK.
        markAllConnecting(conn);
        conn.sendJson(FrameType.auth, .{ .ticket = @as([]const u8, conn.ticket), .protocolVersion = @as(u32, 1) });

        serveEpoch(conn, fd);

        _ = close(fd);
        conn.fd.store(-1, .seq_cst);
        clearServerChannels(conn);
        if (conn.closing.load(.seq_cst) or conn.failed.load(.seq_cst)) break;
        markAllReconnecting(conn);
    }
}

/// Read + dispatch frames until the socket closes.
fn serveEpoch(conn: *Connection, fd: c_int) void {
    var buf: [65536]u8 = undefined;
    while (true) {
        const rc = read(fd, &buf, buf.len);
        if (rc <= 0) return; // EOF / error / woken by shutdown()
        const n: usize = @intCast(rc);
        conn.decoder.push(buf[0..n]) catch return; // OOM: drop the epoch
        while (true) {
            const raw = conn.decoder.next() catch return; // bad frame -> close (§2)
            const frame = raw orelse break;
            handleFrame(conn, frame.frame_type, frame.body);
            if (conn.closing.load(.seq_cst) or conn.failed.load(.seq_cst)) return;
        }
    }
}

fn handleFrame(conn: *Connection, frame_type: u8, body: []const u8) void {
    switch (frame_type) {
        FrameType.ping => {
            if (body.len >= 8) conn.sendFrame(FrameType.pong, body[0..8]);
        },
        FrameType.pong => {},
        FrameType.auth_ok => {
            const parsed = std.json.parseFromSlice(AuthOk, gpa, body, .{ .ignore_unknown_fields = true }) catch return;
            defer parsed.deinit();
            if (parsed.value.ackWindowBytes > 0) conn.ack_window_bytes = parsed.value.ackWindowBytes;
            sendAttachAll(conn);
        },
        FrameType.auth_err => {
            const parsed = std.json.parseFromSlice(AuthErr, gpa, body, .{ .ignore_unknown_fields = true }) catch {
                failConnection(conn, "auth error");
                return;
            };
            defer parsed.deinit();
            failConnection(conn, parsed.value.message);
        },
        FrameType.attached => {
            const parsed = std.json.parseFromSlice(Attached, gpa, body, .{ .ignore_unknown_fields = true }) catch return;
            defer parsed.deinit();
            bindAttached(conn, parsed.value);
        },
        FrameType.attach_err => {
            const parsed = std.json.parseFromSlice(AttachErr, gpa, body, .{ .ignore_unknown_fields = true }) catch return;
            defer parsed.deinit();
            reportAttachErr(conn, parsed.value);
        },
        FrameType.output => handleOutput(conn, body),
        FrameType.resized => {
            if (body.len < 8) return;
            const srv = std.mem.readInt(u32, body[0..4], .little);
            const cols = std.mem.readInt(u16, body[4..6], .little);
            const rows = std.mem.readInt(u16, body[6..8], .little);
            conn.lockState();
            defer conn.unlockState();
            if (conn.by_srv.get(srv)) |ch| ndterm.ndterm_resize(@ptrCast(ch.term), cols, rows);
        },
        FrameType.exit => {
            if (body.len < 8) return;
            const srv = std.mem.readInt(u32, body[0..4], .little);
            const code = std.mem.readInt(i32, body[4..8], .little);
            conn.lockState();
            const ch_opt = conn.by_srv.get(srv);
            conn.unlockState();
            if (ch_opt) |ch| {
                if (ch.effect_cb) |cb| cb(ch.surface_ud, 2, null, code);
            }
        },
        FrameType.detach => {}, // server-initiated detach: nothing to feed
        else => {
            // §2: reject unknown binary frame types.
            failConnection(conn, "unknown frame type");
        },
    }
}

fn handleOutput(conn: *Connection, body: []const u8) void {
    if (body.len < 13) return;
    const srv = std.mem.readInt(u32, body[0..4], .little);
    const seq = std.mem.readInt(u64, body[4..12], .little);
    const flags = body[12];
    const data = body[13..];

    // Hold state_mutex across the feed: a concurrent ndrt_close takes it before
    // freeing the channel, so the channel cannot vanish mid-feed.
    conn.lockState();
    defer conn.unlockState();
    const ch = conn.by_srv.get(srv) orelse return;

    if (flags & FLAG_RESET != 0) ndterm.ndterm_reset(@ptrCast(ch.term));
    if (data.len != 0) ndterm.ndterm_feed(@ptrCast(ch.term), data.ptr, data.len);
    ch.last_seq = seq;
    ch.unacked += data.len;

    // §2.2 flow control: ACK every ackWindowBytes/2 so the daemon keeps sending.
    if (ch.unacked >= conn.ack_window_bytes / 2) {
        var ackbody: [12]u8 = undefined;
        std.mem.writeInt(u32, ackbody[0..4], ch.channel, .little);
        std.mem.writeInt(u64, ackbody[4..12], ch.last_seq, .little);
        conn.sendFrame(FrameType.ack, &ackbody);
        ch.unacked = 0;
    }
}

fn bindAttached(conn: *Connection, a: Attached) void {
    conn.lockState();
    defer conn.unlockState();
    // Match the pending channel by sessionId (server ids aren't known until now).
    for (conn.members.items) |ch| {
        if (std.mem.eql(u8, ch.session_id, a.sessionId)) {
            ch.channel = a.channel;
            ch.last_seq = a.seq;
            ch.attached_once = true;
            conn.by_srv.put(gpa, a.channel, ch) catch {};
            if (a.cols != 0 and a.rows != 0 and (a.cols != ch.cols or a.rows != ch.rows)) {
                ndterm.ndterm_resize(@ptrCast(ch.term), a.cols, a.rows);
                ch.cols = a.cols;
                ch.rows = a.rows;
            }
            ch.notifyState(RT_ATTACHED, a.mode);
            return;
        }
    }
}

fn reportAttachErr(conn: *Connection, e: AttachErr) void {
    conn.lockState();
    defer conn.unlockState();
    for (conn.members.items) |ch| {
        if (std.mem.eql(u8, ch.session_id, e.sessionId)) {
            ch.notifyState(RT_FAILED, e.message);
            return;
        }
    }
}

/// After AUTH_OK (or a reconnect), (re)ATTACH every member with its lastSeq.
fn sendAttachAll(conn: *Connection) void {
    conn.lockState();
    var buf: std.ArrayListUnmanaged(*Channel) = .empty;
    defer buf.deinit(gpa);
    for (conn.members.items) |ch| {
        ch.notifyState(RT_AUTHED, null);
        buf.append(gpa, ch) catch {};
    }
    conn.unlockState();

    for (buf.items) |ch| {
        const last: ?u64 = if (ch.attached_once) ch.last_seq else null;
        conn.sendJson(FrameType.attach, .{
            .sessionId = @as([]const u8, ch.session_id),
            .role = @as([]const u8, "controller"),
            .lastSeq = last,
            .cols = ch.cols,
            .rows = ch.rows,
        });
    }
}

fn markAllConnecting(conn: *Connection) void {
    conn.lockState();
    defer conn.unlockState();
    for (conn.members.items) |ch| ch.notifyState(RT_CONNECTING, null);
}

fn markAllReconnecting(conn: *Connection) void {
    conn.lockState();
    defer conn.unlockState();
    for (conn.members.items) |ch| ch.notifyState(RT_RECONNECTING, null);
}

fn clearServerChannels(conn: *Connection) void {
    conn.lockState();
    defer conn.unlockState();
    conn.by_srv.clearRetainingCapacity();
    for (conn.members.items) |ch| ch.attached_once = false;
}

fn failConnection(conn: *Connection, reason: []const u8) void {
    conn.failed.store(true, .seq_cst);
    conn.lockState();
    defer conn.unlockState();
    for (conn.members.items) |ch| ch.notifyState(RT_FAILED, reason);
}

// ---------------------------------------------------------------------------
// Registry helpers.
// ---------------------------------------------------------------------------

fn findOrCreateConnection(host: [:0]const u8, port: u16, ticket: [:0]const u8) ?*Connection {
    var key_buf: [512]u8 = undefined;
    const key = std.fmt.bufPrint(&key_buf, "{s}:{d}", .{ host, port }) catch return null;

    if (conns.get(key)) |c| return c;

    const conn = gpa.create(Connection) catch return null;
    const key_owned = gpa.dupe(u8, key) catch {
        gpa.destroy(conn);
        return null;
    };
    const host_owned = gpa.dupeZ(u8, host) catch {
        gpa.free(key_owned);
        gpa.destroy(conn);
        return null;
    };
    const ticket_owned = gpa.dupeZ(u8, ticket) catch {
        gpa.free(host_owned);
        gpa.free(key_owned);
        gpa.destroy(conn);
        return null;
    };
    conn.* = .{
        .host = host_owned,
        .port = port,
        .key = key_owned,
        .ticket = ticket_owned,
        .decoder = FrameDecoder.init(gpa),
        .rng = @intFromPtr(conn) | 1,
    };
    conns.put(gpa, key_owned, conn) catch {
        gpa.free(ticket_owned);
        gpa.free(host_owned);
        gpa.free(key_owned);
        gpa.destroy(conn);
        return null;
    };
    conn.reader_thread = std.Thread.spawn(.{}, readerMain, .{conn}) catch {
        _ = conns.remove(key_owned);
        gpa.free(ticket_owned);
        gpa.free(host_owned);
        gpa.free(key_owned);
        gpa.destroy(conn);
        return null;
    };
    return conn;
}

fn destroyConnection(conn: *Connection) void {
    // Reader has been joined by the caller; safe to free everything.
    conn.decoder.deinit();
    conn.members.deinit(gpa);
    conn.by_srv.deinit(gpa);
    gpa.free(conn.ticket);
    gpa.free(conn.host);
    gpa.free(conn.key);
    gpa.destroy(conn);
}

// ---------------------------------------------------------------------------
// ndremote.h exports.
// ---------------------------------------------------------------------------

pub export fn ndrt_open(
    host: ?[*:0]const u8,
    port: u16,
    session_id: ?[*:0]const u8,
    ticket: ?[*:0]const u8,
    cols: u16,
    rows: u16,
    effect_cb: ?EffectCb,
    state_cb: ?StateCb,
    userdata: ?*anyopaque,
) callconv(.c) ?*Channel {
    const host_s: [:0]const u8 = if (host) |h| std.mem.span(h) else "127.0.0.1";
    const sid_s: [:0]const u8 = if (session_id) |s| std.mem.span(s) else "";
    const ticket_s: [:0]const u8 = if (ticket) |t| std.mem.span(t) else "";

    const ch = gpa.create(Channel) catch return null;
    const sid_owned = gpa.dupeZ(u8, sid_s) catch {
        gpa.destroy(ch);
        return null;
    };
    ch.* = .{
        .conn = undefined,
        .session_id = sid_owned,
        .cols = @max(1, cols),
        .rows = @max(1, rows),
        .effect_cb = effect_cb,
        .state_cb = state_cb,
        .surface_ud = userdata,
    };

    const term = ndterm.ndterm_open_virtual(@max(1, cols), @max(1, rows), transportEffectTramp, transportOutputTramp, ch) orelse {
        gpa.free(sid_owned);
        gpa.destroy(ch);
        return null;
    };
    ch.term = @ptrCast(term);

    _ = std.c.pthread_mutex_lock(&registry_mutex);
    const conn = findOrCreateConnection(host_s, port, ticket_s) orelse {
        _ = std.c.pthread_mutex_unlock(&registry_mutex);
        ndterm.ndterm_close(@ptrCast(ch.term));
        gpa.free(sid_owned);
        gpa.destroy(ch);
        return null;
    };
    ch.conn = conn;
    conn.lockState();
    conn.members.append(gpa, ch) catch {};
    conn.refcount += 1;
    conn.unlockState();
    _ = std.c.pthread_mutex_unlock(&registry_mutex);

    ch.notifyState(RT_CONNECTING, null);
    // The reader thread issues the ATTACH once AUTH_OK arrives (or immediately
    // if it is already authed on a shared connection — it re-ATTACHes members
    // on every AUTH_OK, and a fresh member is picked up on the next reconnect;
    // for the common case the connection is brand new and AUTH is in flight).
    if (conn.fd.load(.seq_cst) >= 0) sendAttachAll(conn);
    return ch;
}

pub export fn ndrt_terminal(rt: ?*Channel) callconv(.c) ?*anyopaque {
    const ch = rt orelse return null;
    return ch.term;
}

pub export fn ndrt_write_input(rt: ?*Channel, bytes: [*]const u8, len: usize) callconv(.c) void {
    const ch = rt orelse return;
    ndterm.ndterm_write_input(@ptrCast(ch.term), bytes, len);
}

pub export fn ndrt_resize(rt: ?*Channel, cols: u16, rows: u16) callconv(.c) void {
    const ch = rt orelse return;
    ndterm.ndterm_resize(@ptrCast(ch.term), cols, rows);
    ch.cols = @max(1, cols);
    ch.rows = @max(1, rows);
    if (ch.attached_once) {
        var body: [8]u8 = undefined;
        std.mem.writeInt(u32, body[0..4], ch.channel, .little);
        std.mem.writeInt(u16, body[4..6], cols, .little);
        std.mem.writeInt(u16, body[6..8], rows, .little);
        ch.conn.sendFrame(FrameType.resize, &body);
    }
}

pub export fn ndrt_close(rt: ?*Channel) callconv(.c) void {
    const ch = rt orelse return;
    const conn = ch.conn;

    // DETACH is best-effort; the channel id is only valid if it attached.
    if (ch.attached_once) {
        var body: [4]u8 = undefined;
        std.mem.writeInt(u32, body[0..4], ch.channel, .little);
        conn.sendFrame(FrameType.detach, &body);
    }

    _ = std.c.pthread_mutex_lock(&registry_mutex);
    conn.lockState();
    ch.dead.store(true, .seq_cst);
    // Remove from maps so the reader will never feed it again. Holding
    // state_mutex here guarantees no feed is in progress on this channel.
    if (ch.attached_once) _ = conn.by_srv.remove(ch.channel);
    for (conn.members.items, 0..) |m, i| {
        if (m == ch) {
            _ = conn.members.swapRemove(i);
            break;
        }
    }
    conn.refcount -= 1;
    const last = conn.refcount == 0;
    if (last) {
        conn.closing.store(true, .seq_cst);
        _ = conns.remove(conn.key);
    }
    conn.unlockState();
    _ = std.c.pthread_mutex_unlock(&registry_mutex);

    // This channel is out of every map; free its VT + struct.
    ndterm.ndterm_close(@ptrCast(ch.term));
    ch.notifyState(RT_CLOSED, null);
    gpa.free(ch.session_id);
    gpa.destroy(ch);

    if (last) {
        // Wake the reader out of its blocking read/dial, join, then free.
        const fd = conn.fd.load(.seq_cst);
        if (fd >= 0) _ = shutdown(fd, SHUT_RDWR);
        conn.reader_thread.join();
        destroyConnection(conn);
    }
}

// ---------------------------------------------------------------------------
// Framing unit tests (WP6). Virtual-mode tests live in terminal.zig.
// ---------------------------------------------------------------------------

test "frameBytes round-trips a fixed-layout OUTPUT through the decoder" {
    const testing = std.testing;
    var body: [13 + 2]u8 = undefined;
    std.mem.writeInt(u32, body[0..4], 7, .little); // channel
    std.mem.writeInt(u64, body[4..12], 42, .little); // seq
    body[12] = FLAG_RESET;
    body[13] = 'h';
    body[14] = 'i';
    const bytes = try frameBytes(testing.allocator, FrameType.output, &body);
    defer testing.allocator.free(bytes);

    var dec = FrameDecoder.init(testing.allocator);
    defer dec.deinit();
    try dec.push(bytes);
    const raw = (try dec.next()).?;
    try testing.expectEqual(FrameType.output, raw.frame_type);
    try testing.expectEqual(@as(u32, 7), std.mem.readInt(u32, raw.body[0..4], .little));
    try testing.expectEqual(@as(u64, 42), std.mem.readInt(u64, raw.body[4..12], .little));
    try testing.expectEqual(FLAG_RESET, raw.body[12]);
    try testing.expectEqualSlices(u8, "hi", raw.body[13..]);
    try testing.expect((try dec.next()) == null);
}

test "decoder reassembles a frame split across chunk boundaries" {
    const testing = std.testing;
    var pbody: [8]u8 = undefined;
    std.mem.writeInt(u64, pbody[0..8], 0xdead_beef, .little);
    const bytes = try frameBytes(testing.allocator, FrameType.ping, &pbody);
    defer testing.allocator.free(bytes);

    var dec = FrameDecoder.init(testing.allocator);
    defer dec.deinit();
    // Split mid-frame: header partial in the first push.
    try dec.push(bytes[0..3]);
    try testing.expect((try dec.next()) == null);
    try dec.push(bytes[3..]);
    const raw = (try dec.next()).?;
    try testing.expectEqual(FrameType.ping, raw.frame_type);
    try testing.expectEqual(@as(u64, 0xdead_beef), std.mem.readInt(u64, raw.body[0..8], .little));
    try testing.expect((try dec.next()) == null);
}

test "decoder yields two frames from one chunk" {
    const testing = std.testing;
    var a: [4]u8 = undefined;
    std.mem.writeInt(u32, a[0..4], 1, .little);
    var b: [4]u8 = undefined;
    std.mem.writeInt(u32, b[0..4], 2, .little);
    const fa = try frameBytes(testing.allocator, FrameType.detach, &a);
    defer testing.allocator.free(fa);
    const fb = try frameBytes(testing.allocator, FrameType.detach, &b);
    defer testing.allocator.free(fb);
    const joined = try std.mem.concat(testing.allocator, u8, &.{ fa, fb });
    defer testing.allocator.free(joined);

    var dec = FrameDecoder.init(testing.allocator);
    defer dec.deinit();
    try dec.push(joined);
    const r1 = (try dec.next()).?;
    try testing.expectEqual(@as(u32, 1), std.mem.readInt(u32, r1.body[0..4], .little));
    const r2 = (try dec.next()).?;
    try testing.expectEqual(@as(u32, 2), std.mem.readInt(u32, r2.body[0..4], .little));
    try testing.expect((try dec.next()) == null);
}

test "decoder rejects an oversized payloadLen as EBADFRAME" {
    const testing = std.testing;
    var hdr: [4]u8 = undefined;
    std.mem.writeInt(u32, hdr[0..4], @intCast(MAX_FRAME + 1), .little);
    var dec = FrameDecoder.init(testing.allocator);
    defer dec.deinit();
    try dec.push(&hdr);
    try testing.expectError(FrameError.BadFrame, dec.next());
}

test "frameBytes rejects a body over the max frame size" {
    const testing = std.testing;
    const big = try testing.allocator.alloc(u8, MAX_FRAME);
    defer testing.allocator.free(big);
    try testing.expectError(FrameError.BadFrame, frameBytes(testing.allocator, FrameType.output, big));
}
