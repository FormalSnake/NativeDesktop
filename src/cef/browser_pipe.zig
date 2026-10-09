// The browser target's DevTools protocol, over Chromium's
// --remote-debugging-pipe. The GTK peer of swift/Sources/NDShell/
// NDCefBrowserProtocol.swift.
//
// cdp.zig reaches one page target per browser, and a page session is refused
// the browser-level commands: the `Extensions` domain only runs its actions for
// a browser target (chrome/browser/devtools/chrome_devtools_session.cc). The
// pipe is the one browser session that needs no listening socket, so no other
// local process can drive the browser through it.
//
// Chromium reads the pipe from fd 3 and writes it to fd 4, both fixed on POSIX
// (content/browser/devtools/devtools_agent_host_impl.cc), and holds them for
// the run. Framing is its default ASCIIZ mode: one JSON message per NUL.
// Replies and events are handed to their callbacks on this file's reader
// thread; a callback that touches GTK marshals itself.
const std = @import("std");

const alloc = std.heap.c_allocator;

pub const Reply = *const fn (ctx: *anyopaque, ok: bool, result: std.json.Value) void;
pub const Event = *const fn (ctx: *anyopaque, method: []const u8, params: std.json.Value) void;

const Pending = struct { reply: Reply, ctx: *anyopaque };
const Listener = struct { event: Event, ctx: *anyopaque };

var to_cef: c_int = -1;
var from_cef: c_int = -1;
var busy: std.atomic.Value(bool) = .init(false);
var next_id: u32 = 1;
var pending: std.AutoHashMapUnmanaged(u32, Pending) = .empty;
var listeners: std.StringHashMapUnmanaged(Listener) = .empty;

fn lock() void {
    while (busy.cmpxchgWeak(false, true, .acquire, .monotonic) != null) std.atomic.spinLoopHint();
}

fn unlock() void {
    busy.store(false, .release);
}

fn isFree(fd: c_int) bool {
    return std.c.fcntl(fd, std.c.F.GETFD) == -1;
}

/// Claims fds 3 and 4, only when both are free: a launcher that handed this
/// process one of them keeps it, and the pipe is simply absent. Call in the
/// browser process before anything else opens a descriptor.
pub fn reserve() bool {
    if (to_cef >= 0) return true;
    if (!isFree(3) or !isFree(4)) return false;
    var in: [2]c_int = undefined;
    var out: [2]c_int = undefined;
    if (std.c.pipe(&in) != 0) return false;
    if (std.c.pipe(&out) != 0) {
        _ = std.c.close(in[0]);
        _ = std.c.close(in[1]);
        return false;
    }
    // pipe() takes the lowest free numbers, so in[0] may already sit on 3 and
    // out[1] on 4; the host's ends are moved off 3 and 4 before those are
    // filled.
    const host_write = std.c.fcntl(in[1], std.c.F.DUPFD_CLOEXEC, @as(c_int, 5));
    const host_read = std.c.fcntl(out[0], std.c.F.DUPFD_CLOEXEC, @as(c_int, 5));
    _ = std.c.close(in[1]);
    _ = std.c.close(out[0]);
    const placed = host_write >= 0 and host_read >= 0 and moveTo(in[0], 3) and moveTo(out[1], 4);
    if (!placed) {
        if (host_write >= 0) _ = std.c.close(host_write);
        if (host_read >= 0) _ = std.c.close(host_read);
        _ = std.c.close(3);
        _ = std.c.close(4);
        return false;
    }
    _ = std.c.fcntl(3, std.c.F.SETFD, @as(c_int, std.c.FD_CLOEXEC));
    _ = std.c.fcntl(4, std.c.F.SETFD, @as(c_int, std.c.FD_CLOEXEC));
    to_cef = host_write;
    from_cef = host_read;
    return true;
}

fn moveTo(from: c_int, to: c_int) bool {
    if (from == to) return true;
    if (std.c.dup2(from, to) != to) return false;
    _ = std.c.close(from);
    return true;
}

pub fn available() bool {
    return to_cef >= 0;
}

/// Starts the reader. Once, after cef_initialize.
pub fn start() void {
    if (from_cef < 0) return;
    _ = std.Thread.spawn(.{}, readLoop, .{}) catch {
        std.debug.print("ND_WARN CEF: the browser protocol reader did not start\n", .{});
    };
}

/// Sends one command. `params_json` is an object literal; `session` is a flat
/// session id from Target.attachToTarget, or null for the browser itself.
pub fn call(method: []const u8, params_json: []const u8, session: ?[]const u8, reply: Reply, ctx: *anyopaque) bool {
    if (to_cef < 0) return false;
    lock();
    const id = next_id;
    next_id += 1;
    pending.put(alloc, id, .{ .reply = reply, .ctx = ctx }) catch {
        unlock();
        return false;
    };
    unlock();
    const message = if (session) |s|
        std.fmt.allocPrint(alloc, "{{\"id\":{d},\"method\":\"{s}\",\"params\":{s},\"sessionId\":\"{s}\"}}\x00", .{ id, method, params_json, s })
    else
        std.fmt.allocPrint(alloc, "{{\"id\":{d},\"method\":\"{s}\",\"params\":{s}}}\x00", .{ id, method, params_json });
    const bytes = message catch {
        forget(id);
        return false;
    };
    defer alloc.free(bytes);
    lock();
    const written = writeAll(bytes);
    unlock();
    if (!written) forget(id);
    return written;
}

fn forget(id: u32) void {
    lock();
    _ = pending.remove(id);
    unlock();
}

fn writeAll(bytes: []const u8) bool {
    var offset: usize = 0;
    while (offset < bytes.len) {
        const n = std.c.write(to_cef, bytes[offset..].ptr, bytes.len - offset);
        if (n < 0) {
            if (std.c._errno().* == @intFromEnum(std.c.E.INTR)) continue;
            return false;
        }
        offset += @intCast(n);
    }
    return true;
}

/// Events for one flat session, or null to stop them. `session` is copied; ""
/// is the browser's own session.
pub fn listen(session: []const u8, wanted: ?Listener) void {
    lock();
    defer unlock();
    if (wanted) |l| {
        const key = alloc.dupe(u8, session) catch return;
        const slot = listeners.getOrPut(alloc, key) catch {
            alloc.free(key);
            return;
        };
        if (slot.found_existing) alloc.free(key);
        slot.value_ptr.* = l;
    } else if (listeners.fetchRemove(session)) |kv| {
        alloc.free(kv.key);
    }
}

pub fn listener(event: Event, ctx: *anyopaque) Listener {
    return .{ .event = event, .ctx = ctx };
}

fn readLoop() void {
    var buffer: std.ArrayList(u8) = .empty;
    defer buffer.deinit(alloc);
    var chunk: [64 * 1024]u8 = undefined;
    while (true) {
        const n = std.c.read(from_cef, &chunk, chunk.len);
        if (n < 0 and std.c._errno().* == @intFromEnum(std.c.E.INTR)) continue;
        if (n <= 0) return;
        buffer.appendSlice(alloc, chunk[0..@intCast(n)]) catch return;
        while (std.mem.indexOfScalar(u8, buffer.items, 0)) |end| {
            dispatch(buffer.items[0..end]);
            buffer.replaceRange(alloc, 0, end + 1, &.{}) catch return;
        }
    }
}

fn dispatch(bytes: []const u8) void {
    var parsed = std.json.parseFromSlice(std.json.Value, alloc, bytes, .{}) catch return;
    defer parsed.deinit();
    const root = switch (parsed.value) {
        .object => |o| o,
        else => return,
    };
    if (root.get("method")) |method| {
        if (method != .string) return;
        const session: []const u8 = switch (root.get("sessionId") orelse std.json.Value{ .string = "" }) {
            .string => |id| id,
            else => return,
        };
        lock();
        const l = listeners.get(session);
        unlock();
        if (l) |found| found.event(found.ctx, method.string, root.get("params") orelse .null);
        return;
    }
    const id_value = root.get("id") orelse return;
    if (id_value != .integer) return;
    lock();
    const entry = pending.fetchRemove(@intCast(id_value.integer));
    unlock();
    const p = (entry orelse return).value;
    if (root.get("error")) |err| {
        p.reply(p.ctx, false, err);
    } else {
        p.reply(p.ctx, true, root.get("result") orelse .null);
    }
}
