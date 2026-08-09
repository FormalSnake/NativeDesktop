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

// fcntl/setsockopt/poll plumbing for the interruptible dial, TCP keepalive,
// and the heartbeat read timeout. Values differ between Linux and the
// BSD/macOS family; switch per target like `addrinfo` above.
const F_GETFL: c_int = 3;
const F_SETFL: c_int = 4;
const O_NONBLOCK: c_int = switch (builtin.os.tag) {
    .linux => 0o4000,
    else => 0x0004,
};
const POLLOUT: i16 = 0x0004;
const SOL_SOCKET: c_int = switch (builtin.os.tag) {
    .linux => 1,
    else => 0xffff,
};
const SO_KEEPALIVE: c_int = switch (builtin.os.tag) {
    .linux => 9,
    else => 0x0008,
};
const SO_ERROR: c_int = switch (builtin.os.tag) {
    .linux => 4,
    else => 0x1007,
};
const SO_RCVTIMEO: c_int = switch (builtin.os.tag) {
    .linux => 20,
    else => 0x1006,
};
const IPPROTO_TCP: c_int = 6;
// macOS spells the keepalive-idle option TCP_KEEPALIVE (0x10), Linux TCP_KEEPIDLE (4).
const TCP_KEEPIDLE: c_int = switch (builtin.os.tag) {
    .linux => 4,
    else => 0x10,
};
const TCP_KEEPINTVL: c_int = switch (builtin.os.tag) {
    .linux => 5,
    else => 0x101,
};
const TCP_KEEPCNT: c_int = switch (builtin.os.tag) {
    .linux => 6,
    else => 0x102,
};
const EINTR: c_int = 4;
const EINPROGRESS: c_int = switch (builtin.os.tag) {
    .linux => 115,
    else => 36,
};
const EWOULDBLOCK: c_int = switch (builtin.os.tag) {
    .linux => 11,
    else => 35,
};

const nfds_t = switch (builtin.os.tag) {
    .linux => c_ulong,
    else => c_uint,
};
const pollfd = extern struct { fd: c_int, events: i16, revents: i16 };
const timeval = switch (builtin.os.tag) {
    .linux => extern struct { sec: c_long, usec: c_long },
    else => extern struct { sec: c_long, usec: i32 },
};

extern "c" fn getaddrinfo(node: ?[*:0]const u8, service: ?[*:0]const u8, hints: ?*const addrinfo, res: *?*addrinfo) c_int;
extern "c" fn freeaddrinfo(res: ?*addrinfo) void;
extern "c" fn socket(domain: c_int, socktype: c_int, protocol: c_int) c_int;
extern "c" fn connect(fd: c_int, addr: *const anyopaque, len: u32) c_int;
extern "c" fn shutdown(fd: c_int, how: c_int) c_int;
extern "c" fn read(fd: c_int, buf: [*]u8, nbyte: usize) isize;
extern "c" fn write(fd: c_int, buf: [*]const u8, nbyte: usize) isize;
extern "c" fn close(fd: c_int) c_int;
extern "c" fn nanosleep(req: *const timespec, rem: ?*timespec) c_int;
extern "c" fn fcntl(fd: c_int, cmd: c_int, ...) c_int;
extern "c" fn poll(fds: [*]pollfd, nfds: nfds_t, timeout: c_int) c_int;
extern "c" fn setsockopt(fd: c_int, level: c_int, optname: c_int, optval: *const anyopaque, optlen: u32) c_int;
extern "c" fn getsockopt(fd: c_int, level: c_int, optname: c_int, optval: *anyopaque, optlen: *u32) c_int;

const DIAL_TIMEOUT_MS: u64 = 5_000;
/// AUTH_OK.heartbeatSec fallback when the daemon omits or zeroes the field.
const DEFAULT_HEARTBEAT_SEC: u64 = 30;
/// The read timeout is heartbeat/2, so this many consecutive idle wakes equals
/// 2x heartbeat of total inbound silence: the dead-link threshold (docs
/// protocol.md §2.2).
const DEAD_IDLE_WAKES: u32 = 4;

fn effectiveHeartbeatSec(from_auth_ok: u64) u64 {
    return if (from_auth_ok == 0) DEFAULT_HEARTBEAT_SEC else from_auth_ok;
}

/// SO_RCVTIMEO for serveEpoch's read loop: half the heartbeat interval, so
/// the reader wakes twice per interval to PING and to count silence.
fn readTimeoutFor(heartbeat_sec: u64) timeval {
    const half_ms = heartbeat_sec * 500;
    return .{ .sec = @intCast(half_ms / 1000), .usec = @intCast((half_ms % 1000) * 1000) };
}

/// Dial conn.host:port (numeric or DNS, IPv4/IPv6) with a bounded,
/// interruptible connect: O_NONBLOCK + poll with a DIAL_TIMEOUT_MS deadline.
/// The in-progress fd is published to conn.fd BEFORE the wait so ndrt_close
/// can shutdown() a hanging dial, and the wait polls in 100ms slices checking
/// conn.closing (macOS shutdown() on a still-connecting socket is ENOTCONN
/// and wakes nothing). On success the fd is blocking again with TCP keepalive
/// armed. Returns null on any failure; tries each resolved address in turn.
fn dial(conn: *Connection) ?c_int {
    var hints = std.mem.zeroes(addrinfo);
    hints.family = AF_UNSPEC;
    hints.socktype = SOCK_STREAM;

    var svc_buf: [8]u8 = undefined;
    const svc = std.fmt.bufPrintZ(&svc_buf, "{d}", .{conn.port}) catch return null;

    var res: ?*addrinfo = null;
    if (getaddrinfo(conn.host.ptr, svc.ptr, &hints, &res) != 0) return null;
    defer freeaddrinfo(res);

    var it = res;
    while (it) |ai| : (it = ai.next) {
        if (conn.closing.load(.seq_cst)) return null;
        const sa = ai.addr orelse continue;
        const fd = socket(ai.family, ai.socktype, ai.protocol);
        if (fd < 0) continue;
        if (connectDeadline(conn, fd, sa, ai.addrlen)) {
            armKeepalive(fd);
            return fd;
        }
        conn.fd.store(-1, .seq_cst);
        _ = close(fd);
    }
    return null;
}

fn connectDeadline(conn: *Connection, fd: c_int, sa: *const anyopaque, salen: u32) bool {
    const fl = fcntl(fd, F_GETFL, @as(c_int, 0));
    if (fl < 0) return false;
    if (fcntl(fd, F_SETFL, fl | O_NONBLOCK) < 0) return false;
    conn.fd.store(fd, .seq_cst); // published pre-wait: ndrt_close can shutdown() this fd
    if (connect(fd, sa, salen) != 0) {
        if (std.c._errno().* != EINPROGRESS) return false;
        var pfd = [1]pollfd{.{ .fd = fd, .events = POLLOUT, .revents = 0 }};
        var waited_ms: u64 = 0;
        var ready = false;
        while (waited_ms < DIAL_TIMEOUT_MS) {
            if (conn.closing.load(.seq_cst)) return false;
            const rc = poll(&pfd, 1, 100);
            if (rc > 0) {
                ready = true;
                break;
            }
            if (rc < 0 and std.c._errno().* != EINTR) return false;
            waited_ms += 100;
        }
        if (!ready) return false;
        var err: c_int = 0;
        var errlen: u32 = @sizeOf(c_int);
        if (getsockopt(fd, SOL_SOCKET, SO_ERROR, &err, &errlen) != 0 or err != 0) return false;
    }
    return fcntl(fd, F_SETFL, fl) >= 0;
}

/// SO_KEEPALIVE with aggressive probing (idle 20s, then 5s x 3 probes) so a
/// blackholed link also dies at the TCP layer, independent of the PING cycle.
fn armKeepalive(fd: c_int) void {
    var v: c_int = 1;
    _ = setsockopt(fd, SOL_SOCKET, SO_KEEPALIVE, &v, @sizeOf(c_int));
    v = 20;
    _ = setsockopt(fd, IPPROTO_TCP, TCP_KEEPIDLE, &v, @sizeOf(c_int));
    v = 5;
    _ = setsockopt(fd, IPPROTO_TCP, TCP_KEEPINTVL, &v, @sizeOf(c_int));
    v = 3;
    _ = setsockopt(fd, IPPROTO_TCP, TCP_KEEPCNT, &v, @sizeOf(c_int));
}

fn sleepMs(ms: u64) void {
    const ts = timespec{ .sec = @intCast(ms / 1000), .nsec = @intCast((ms % 1000) * 1_000_000) };
    _ = nanosleep(&ts, null);
}

/// Backoff sleep in 100ms slices watching conn.closing, so ndrt_close never
/// waits out a full reconnect backoff (up to 45s) to join the reader.
fn sleepInterruptible(conn: *Connection, total_ms: u64) void {
    var remaining = total_ms;
    while (remaining > 0) {
        if (conn.closing.load(.seq_cst)) return;
        const slice = @min(remaining, 100);
        sleepMs(slice);
        remaining -= slice;
    }
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
    /// Fresh ATTACHes (no lastSeq) request `replay: "history"` (§2.2): the
    /// daemon replays the session's retained ring history into this fresh VT
    /// instead of a screen-only snapshot. Set only via ndrt_open_history.
    replay_history: bool = false,
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

    // From AUTH_OK. heartbeat_sec is only touched on the reader thread
    // (serveEpoch reads it, handleFrame's AUTH_OK arm writes it).
    ack_window_bytes: u64 = 256 * 1024,
    heartbeat_sec: u64 = DEFAULT_HEARTBEAT_SEC,

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
        const fd = dial(conn) orelse {
            markAllReconnecting(conn);
            const jitter = conn.nextJitter(backoff_ms / 2);
            sleepInterruptible(conn, backoff_ms + jitter);
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

/// Read + dispatch frames until the socket closes or the link goes silent.
/// SO_RCVTIMEO (heartbeat/2, from AUTH_OK.heartbeatSec) wakes the blocking
/// read periodically; each idle wake sends a PING, and DEAD_IDLE_WAKES
/// consecutive idle wakes (2x heartbeat with no inbound bytes at all, PONGs
/// included) ends the epoch like an EOF so the normal redial/backoff path
/// runs instead of blocking forever on a blackholed peer.
fn serveEpoch(conn: *Connection, fd: c_int) void {
    var buf: [65536]u8 = undefined;
    var applied_hb: u64 = 0;
    var idle_wakes: u32 = 0;
    var ping_nonce: u64 = 0;
    while (true) {
        if (conn.heartbeat_sec != applied_hb) {
            applied_hb = conn.heartbeat_sec;
            var tv = readTimeoutFor(applied_hb);
            _ = setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, @sizeOf(timeval));
        }
        const rc = read(fd, &buf, buf.len);
        if (rc == 0) return; // EOF / woken by shutdown()
        if (rc < 0) {
            const e = std.c._errno().*;
            if (e == EINTR) continue;
            if (e != EWOULDBLOCK) return; // hard socket error
            idle_wakes += 1;
            if (idle_wakes >= DEAD_IDLE_WAKES) return; // silent link: reconnect
            ping_nonce += 1;
            var nonce: [8]u8 = undefined;
            std.mem.writeInt(u64, nonce[0..8], ping_nonce, .little);
            conn.sendFrame(FrameType.ping, &nonce);
            continue;
        }
        idle_wakes = 0;
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
            conn.heartbeat_sec = effectiveHeartbeatSec(parsed.value.heartbeatSec);
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
            // Hold state_mutex across the callback (like the OUTPUT/RESIZED
            // feeds): a concurrent ndrt_close takes it before freeing the
            // channel, so `ch` cannot vanish between the lookup and the call.
            // The surface effect cb only marshals onto its UI loop — it never
            // re-enters ndrt_*, so there is no lock cycle.
            conn.lockState();
            defer conn.unlockState();
            if (conn.by_srv.get(srv)) |ch| {
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
            // A re-ATTACH (opening another channel on this live connection
            // re-attaches every member, and the daemon assigns a fresh channel
            // id each time — bytelane/server.ts nextChannel++) rebinds this
            // member to a new id. Drop the prior mapping first, or by_srv keeps
            // a stale entry pointing at this Channel that ndrt_close's
            // remove(ch.channel) never clears — the reader would then feed a
            // freed VT via the orphaned key.
            if (ch.attached_once) _ = conn.by_srv.remove(ch.channel);
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
    // Hold state_mutex across the sends: a concurrent ndrt_close takes it
    // before removing a member from `members` and freeing it, so iterating and
    // reading ch.session_id/cols/rows here can never touch a freed Channel.
    // sendJson only takes write_mutex, which nests below state_mutex in the
    // documented lock order, so widening the scope introduces no cycle.
    conn.lockState();
    defer conn.unlockState();
    for (conn.members.items) |ch| {
        ch.notifyState(RT_AUTHED, null);
        const last: ?u64 = if (ch.attached_once) ch.last_seq else null;
        // §2.2: `replay` is only consulted when lastSeq is absent; a warm
        // re-ATTACH keeps the exact byte-gap semantics regardless of the flag.
        const replay: ?[]const u8 = if (ch.replay_history and last == null) "history" else null;
        conn.sendJson(FrameType.attach, .{
            .sessionId = @as([]const u8, ch.session_id),
            .role = @as([]const u8, "controller"),
            .lastSeq = last,
            .replay = replay,
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
    // Keyed by host:port:ticket, NOT host:port. A ticket names one server-side
    // grant (which sessions this connection may attach; typically single-use),
    // so terminals opened with the same ticket share a connection, while a
    // fresh ticket always gets a fresh connection and reader, even when a
    // failed connection for the same endpoint still has live members. Keying
    // by endpoint alone silently discarded every later ticket: an
    // endpoint-shared connection could only ever attach the FIRST ticket's
    // session scope, and after failConnection() the dead entry blocked the
    // endpoint until its last member closed.
    const key_owned = std.fmt.allocPrint(gpa, "{s}:{d}:{s}", .{ host, port, ticket }) catch return null;

    if (conns.get(key_owned)) |c| {
        gpa.free(key_owned);
        return c;
    }

    const conn = gpa.create(Connection) catch {
        gpa.free(key_owned);
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
    return openImpl(host, port, session_id, ticket, cols, rows, null, false, effect_cb, state_cb, userdata);
}

/// WP polish-1 deliverable 7: same as ndrt_open, plus an optional open-time
/// default fg/bg + 256-color palette applied to the underlying virtual ndterm
/// (see include/ndterm.h `nd_term_open_opts`). `opts == null` behaves exactly
/// like ndrt_open.
pub export fn ndrt_open_ex(
    host: ?[*:0]const u8,
    port: u16,
    session_id: ?[*:0]const u8,
    ticket: ?[*:0]const u8,
    cols: u16,
    rows: u16,
    opts: ?*const ndterm.nd_term_open_opts,
    effect_cb: ?EffectCb,
    state_cb: ?StateCb,
    userdata: ?*anyopaque,
) callconv(.c) ?*Channel {
    return openImpl(host, port, session_id, ticket, cols, rows, opts, false, effect_cb, state_cb, userdata);
}

/// Same as ndrt_open_ex, but the fresh ATTACH opts into the daemon's
/// retained-ring history replay (canary docs/protocol.md §2.2
/// `replay: "history"`): the daemon feeds the session's ring history
/// (screen *and* scrollback, bounded by ring capacity) into this fresh VT
/// instead of a screen-only snapshot. Falls back to the ordinary
/// snapshot/live attach when the daemon cannot serve history (older daemon,
/// empty ring), so callers need no special handling.
pub export fn ndrt_open_history(
    host: ?[*:0]const u8,
    port: u16,
    session_id: ?[*:0]const u8,
    ticket: ?[*:0]const u8,
    cols: u16,
    rows: u16,
    opts: ?*const ndterm.nd_term_open_opts,
    effect_cb: ?EffectCb,
    state_cb: ?StateCb,
    userdata: ?*anyopaque,
) callconv(.c) ?*Channel {
    return openImpl(host, port, session_id, ticket, cols, rows, opts, true, effect_cb, state_cb, userdata);
}

fn openImpl(
    host: ?[*:0]const u8,
    port: u16,
    session_id: ?[*:0]const u8,
    ticket: ?[*:0]const u8,
    cols: u16,
    rows: u16,
    opts: ?*const ndterm.nd_term_open_opts,
    replay_history: bool,
    effect_cb: ?EffectCb,
    state_cb: ?StateCb,
    userdata: ?*anyopaque,
) ?*Channel {
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
        .replay_history = replay_history,
        .cols = @max(1, cols),
        .rows = @max(1, rows),
        .effect_cb = effect_cb,
        .state_cb = state_cb,
        .surface_ud = userdata,
    };

    const term = ndterm.ndterm_open_virtual_ex(@max(1, cols), @max(1, rows), opts, transportEffectTramp, transportOutputTramp, ch) orelse {
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

test "heartbeat falls back to 30s when AUTH_OK omits or zeroes heartbeatSec" {
    const testing = std.testing;
    try testing.expectEqual(DEFAULT_HEARTBEAT_SEC, effectiveHeartbeatSec(0));
    try testing.expectEqual(@as(u64, 7), effectiveHeartbeatSec(7));
}

test "read timeout is half the heartbeat interval" {
    const testing = std.testing;
    const tv30 = readTimeoutFor(30);
    try testing.expect(tv30.sec == 15 and tv30.usec == 0);
    const tv1 = readTimeoutFor(1);
    try testing.expect(tv1.sec == 0 and tv1.usec == 500_000);
    // DEAD_IDLE_WAKES half-interval wakes must add up to the 2x-heartbeat
    // silence budget (docs/protocol.md §2.2).
    try testing.expectEqual(@as(u64, 2 * 30 * 1000), DEAD_IDLE_WAKES * 30 * 500);
}

// ---------------------------------------------------------------------------
// Close-while-streaming regression (the paned-restructure use-after-free).
//
// A scripted loopback daemon drives the real reader thread. Opening a second
// channel on the live shared connection re-ATTACHes the first, and the daemon
// assigns a fresh channel id per ATTACH (bytelane/server.ts nextChannel++), so
// the first member gets rebound to a new id. The daemon keeps streaming OUTPUT
// on the FIRST-assigned channel id. Before the fix, bindAttached left the
// stale id in by_srv, ndrt_close(A) never cleared it, and the reader fed the
// freed VT via the orphaned key — a GP fault. This exercises exactly that
// timing and asserts the process survives it.
// ---------------------------------------------------------------------------

// Loopback listener libc (Zig 0.16 has no std.net/std.posix sockets; mirror the
// file's hand-declared extern "c" approach). sockaddr_in has the BSD sin_len /
// sin_family split on Darwin — branch on the target like `addrinfo` above.
const AF_INET: c_int = 2;
extern "c" fn bind(fd: c_int, addr: *const anyopaque, len: u32) c_int;
extern "c" fn listen(fd: c_int, backlog: c_int) c_int;
extern "c" fn accept(fd: c_int, addr: ?*anyopaque, len: ?*u32) c_int;
extern "c" fn getsockname(fd: c_int, addr: *anyopaque, len: *u32) c_int;

const sockaddr_in = switch (builtin.os.tag) {
    .linux => extern struct { family: u16 = AF_INET, port: u16 = 0, addr: u32 = 0, zero: [8]u8 = @splat(0) },
    else => extern struct { len: u8 = 16, family: u8 = AF_INET, port: u16 = 0, addr: u32 = 0, zero: [8]u8 = @splat(0) },
};

/// Bind a fresh 127.0.0.1 listener on an ephemeral port; returns (fd, port).
fn loopbackListen() !struct { fd: c_int, port: u16 } {
    const fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) return error.SocketFailed;
    var sa = sockaddr_in{ .port = 0, .addr = std.mem.nativeToBig(u32, 0x7f00_0001) };
    if (bind(fd, &sa, @sizeOf(sockaddr_in)) != 0) return error.BindFailed;
    if (listen(fd, 4) != 0) return error.ListenFailed;
    var got = sockaddr_in{};
    var len: u32 = @sizeOf(sockaddr_in);
    if (getsockname(fd, &got, &len) != 0) return error.GetsocknameFailed;
    return .{ .fd = fd, .port = std.mem.bigToNative(u16, got.port) };
}

const MockDaemon = struct {
    listen_fd: c_int,
    port: u16,
    stream: std.atomic.Value(usize) = std.atomic.Value(usize).init(0), // accepted fd+1 (0 = none yet)
    write_mutex: std.c.pthread_mutex_t = .{},
    next_channel: u32 = 1,
    stop: std.atomic.Value(bool) = std.atomic.Value(bool).init(false),
    attaches: std.atomic.Value(u32) = std.atomic.Value(u32).init(0),
    /// When set, an ATTACH carrying `replay:"history"` (and no lastSeq) is
    /// answered with mode "history" plus a RESET-flagged OUTPUT carrying
    /// `history_payload`; the daemon side of docs/protocol.md §2.2.
    history_mode: bool = false,
    /// ATTACHes seen with `replay:"history"` and no lastSeq.
    history_attaches: std.atomic.Value(u32) = std.atomic.Value(u32).init(0),
    /// ATTACHes where the replay field appeared in any other combination
    /// (present alongside lastSeq, or on a channel that never opted in).
    replay_misuse: std.atomic.Value(u32) = std.atomic.Value(u32).init(0),

    const history_payload = "\x1b]2;hist-title\x07HIST";

    fn send(self: *MockDaemon, fd: c_int, ftype: u8, body: []const u8) void {
        var hdr: [5]u8 = undefined;
        std.mem.writeInt(u32, hdr[0..4], @intCast(1 + body.len), .little);
        hdr[4] = ftype;
        _ = std.c.pthread_mutex_lock(&self.write_mutex);
        defer _ = std.c.pthread_mutex_unlock(&self.write_mutex);
        var off: usize = 0;
        while (off < hdr.len) {
            const rc = write(fd, hdr[off..].ptr, hdr.len - off);
            if (rc <= 0) return;
            off += @intCast(rc);
        }
        off = 0;
        while (off < body.len) {
            const rc = write(fd, body.ptr + off, body.len - off);
            if (rc <= 0) return;
            off += @intCast(rc);
        }
    }

    // Accept one client, answer AUTH, and assign a fresh channel per ATTACH.
    fn serve(self: *MockDaemon) void {
        const fd = accept(self.listen_fd, null, null);
        if (fd < 0) return;
        self.stream.store(@as(usize, @intCast(fd)) + 1, .seq_cst);

        var dec = FrameDecoder.init(gpa);
        defer dec.deinit();
        var buf: [4096]u8 = undefined;
        while (!self.stop.load(.seq_cst)) {
            const rc = read(fd, &buf, buf.len);
            if (rc <= 0) break;
            dec.push(buf[0..@intCast(rc)]) catch break;
            while (dec.next() catch break) |frame| {
                switch (frame.frame_type) {
                    FrameType.auth => self.send(fd, FrameType.auth_ok, "{\"ackWindowBytes\":262144}"),
                    FrameType.attach => {
                        const Body = struct { sessionId: []const u8 = "", lastSeq: ?u64 = null, replay: ?[]const u8 = null };
                        const parsed = std.json.parseFromSlice(Body, gpa, frame.body, .{ .ignore_unknown_fields = true }) catch continue;
                        defer parsed.deinit();
                        const wants_history = if (parsed.value.replay) |r| std.mem.eql(u8, r, "history") else false;
                        if (wants_history and parsed.value.lastSeq == null) {
                            _ = self.history_attaches.fetchAdd(1, .seq_cst);
                        } else if (parsed.value.replay != null) {
                            _ = self.replay_misuse.fetchAdd(1, .seq_cst);
                        }
                        const chan = self.next_channel;
                        self.next_channel += 1;
                        var jb: [256]u8 = undefined;
                        if (self.history_mode and wants_history and parsed.value.lastSeq == null) {
                            const j = std.fmt.bufPrint(&jb, "{{\"sessionId\":\"{s}\",\"channel\":{d},\"seq\":{d},\"cols\":80,\"rows\":24,\"mode\":\"history\"}}", .{ parsed.value.sessionId, chan, history_payload.len }) catch continue;
                            self.send(fd, FrameType.attached, j);
                            var out: [13 + history_payload.len]u8 = undefined;
                            std.mem.writeInt(u32, out[0..4], chan, .little);
                            std.mem.writeInt(u64, out[4..12], history_payload.len, .little);
                            out[12] = FLAG_RESET; // history resync: RESET, no SNAPSHOT (§2.2)
                            @memcpy(out[13..], history_payload);
                            self.send(fd, FrameType.output, &out);
                        } else {
                            const j = std.fmt.bufPrint(&jb, "{{\"sessionId\":\"{s}\",\"channel\":{d},\"seq\":0,\"cols\":80,\"rows\":24,\"mode\":\"live\"}}", .{ parsed.value.sessionId, chan }) catch continue;
                            self.send(fd, FrameType.attached, j);
                        }
                        _ = self.attaches.fetchAdd(1, .seq_cst);
                    },
                    else => {}, // drain INPUT/ACK/DETACH
                }
            }
        }
    }

    // Hammer OUTPUT on channel 1 (the first-assigned id) so a surviving orphan
    // mapping would be fed across the close of channel A.
    fn stream1(self: *MockDaemon) void {
        var seq: u64 = 1;
        while (!self.stop.load(.seq_cst)) {
            const raw = self.stream.load(.seq_cst);
            if (raw == 0) {
                sleepMs(1);
                continue;
            }
            const fd: c_int = @intCast(raw - 1);
            var body: [14]u8 = undefined;
            std.mem.writeInt(u32, body[0..4], 1, .little); // channel 1
            std.mem.writeInt(u64, body[4..12], seq, .little);
            body[12] = 0; // no flags
            body[13] = 'x';
            self.send(fd, FrameType.output, &body);
            seq += 1;
            sleepMs(1);
        }
    }
};

fn stateCbCount(userdata: ?*anyopaque, state: c_int, _: ?[*:0]const u8) callconv(.c) void {
    if (state != RT_ATTACHED) return;
    const counter: *std.atomic.Value(u32) = @ptrCast(@alignCast(userdata orelse return));
    _ = counter.fetchAdd(1, .seq_cst);
}

test "close while output streams on an orphaned channel does not use-after-free" {
    // Loopback + reader threads use the c_allocator like production; this soaks
    // the real ndrt_open/close lifetime, so a UAF crashes the test process.
    const lb = try loopbackListen();
    var daemon = MockDaemon{ .listen_fd = lb.fd, .port = lb.port };
    defer _ = close(daemon.listen_fd);

    const serve_t = try std.Thread.spawn(.{}, MockDaemon.serve, .{&daemon});
    const stream_t = try std.Thread.spawn(.{}, MockDaemon.stream1, .{&daemon});

    var host_buf: [16]u8 = undefined;
    const host = try std.fmt.bufPrintZ(&host_buf, "127.0.0.1", .{});

    var attached = std.atomic.Value(u32).init(0);

    const a = ndrt_open(host.ptr, daemon.port, "sess-a", "", 80, 24, null, &stateCbCount, &attached) orelse return error.OpenFailed;
    // Wait until A is attached (channel 1 bound) before opening B.
    var spins: u32 = 0;
    while (attached.load(.seq_cst) < 1 and spins < 2000) : (spins += 1) sleepMs(1);
    try std.testing.expect(attached.load(.seq_cst) >= 1);

    // B on the same host:port re-ATTACHes A (fresh id) — A now rebinds to
    // channel 2 while the daemon still streams on channel 1.
    const b = ndrt_open(host.ptr, daemon.port, "sess-b", "", 80, 24, null, &stateCbCount, &attached) orelse return error.OpenFailed;
    spins = 0;
    while (daemon.attaches.load(.seq_cst) < 3 and spins < 2000) : (spins += 1) sleepMs(1);

    // Let the client process both re-ATTACHED binds, then close A while the
    // channel-1 stream is in flight.
    sleepMs(30);
    ndrt_close(a);
    sleepMs(30); // reader keeps servicing channel-1 output here
    ndrt_close(b); // last channel: joins the reader

    daemon.stop.store(true, .seq_cst);
    const raw = daemon.stream.load(.seq_cst);
    if (raw != 0) _ = shutdown(@intCast(raw - 1), SHUT_RDWR);
    serve_t.join();
    stream_t.join();
}

// ---------------------------------------------------------------------------
// History-replay attach (docs/protocol.md §2.2 replay: "history"): the wire
// request carries the field only on a lastSeq-less ATTACH, and the replayed
// bytes reach the fresh VT through the ordinary OUTPUT feed path.
// ---------------------------------------------------------------------------

const HistCap = struct {
    var title_ok = std.atomic.Value(bool).init(false);
    var mode_history = std.atomic.Value(bool).init(false);

    // The replayed payload carries an OSC 2 title: the VT parsing it (and the
    // effect surfacing here) proves the bytes were fed into the fresh VT.
    fn effectCb(_: ?*anyopaque, kind: c_int, text: ?[*:0]const u8, _: c_int) callconv(.c) void {
        if (kind != 0) return; // 0 = title
        const t = text orelse return;
        if (std.mem.eql(u8, std.mem.span(t), "hist-title")) title_ok.store(true, .seq_cst);
    }

    fn stateCb(_: ?*anyopaque, state: c_int, detail: ?[*:0]const u8) callconv(.c) void {
        if (state != RT_ATTACHED) return;
        const d = detail orelse return;
        if (std.mem.eql(u8, std.mem.span(d), "history")) mode_history.store(true, .seq_cst);
    }
};

test "ndrt_open_history requests replay:history and feeds the replayed bytes into the fresh VT" {
    const lb = try loopbackListen();
    var daemon = MockDaemon{ .listen_fd = lb.fd, .port = lb.port, .history_mode = true };
    defer _ = close(daemon.listen_fd);
    const serve_t = try std.Thread.spawn(.{}, MockDaemon.serve, .{&daemon});

    var host_buf: [16]u8 = undefined;
    const host = try std.fmt.bufPrintZ(&host_buf, "127.0.0.1", .{});

    HistCap.title_ok.store(false, .seq_cst);
    HistCap.mode_history.store(false, .seq_cst);

    const h = ndrt_open_history(host.ptr, daemon.port, "sess-h", "", 40, 10, null, HistCap.effectCb, HistCap.stateCb, null) orelse return error.OpenFailed;

    var spins: u32 = 0;
    while (!HistCap.title_ok.load(.seq_cst) and spins < 2000) : (spins += 1) sleepMs(1);
    try std.testing.expect(HistCap.title_ok.load(.seq_cst)); // replayed bytes reached the VT
    try std.testing.expect(HistCap.mode_history.load(.seq_cst)); // ATTACHED reported mode "history"
    try std.testing.expectEqual(@as(u32, 1), daemon.history_attaches.load(.seq_cst));
    try std.testing.expectEqual(@as(u32, 0), daemon.replay_misuse.load(.seq_cst));

    // A plain open on the shared connection re-ATTACHes every member: sess-p
    // must never carry the replay field, and sess-h's re-ATTACH now has a
    // lastSeq so it must not either (misuse counts both).
    var attached = std.atomic.Value(u32).init(0);
    const p = ndrt_open(host.ptr, daemon.port, "sess-p", "", 40, 10, null, &stateCbCount, &attached) orelse return error.OpenFailed;
    spins = 0;
    while (daemon.attaches.load(.seq_cst) < 3 and spins < 2000) : (spins += 1) sleepMs(1);
    try std.testing.expect(daemon.attaches.load(.seq_cst) >= 3);
    try std.testing.expectEqual(@as(u32, 0), daemon.replay_misuse.load(.seq_cst));

    ndrt_close(p);
    ndrt_close(h);

    daemon.stop.store(true, .seq_cst);
    const raw = daemon.stream.load(.seq_cst);
    if (raw != 0) _ = shutdown(@intCast(raw - 1), SHUT_RDWR);
    serve_t.join();
}
