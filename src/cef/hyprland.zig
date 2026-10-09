// The one thing Hyprland will not take from an X11 hint: keeping a window
// above the others.
//
// Chromium asks for its picture-in-picture window to stay above every other
// window and on every workspace (`_NET_WM_STATE_ABOVE`, `_NET_WM_DESKTOP`
// 0xFFFFFFFF). Hyprland's XWayland support replaces the state list with its
// own on map and has no keep-above of its own; what it has is `pin`, a
// floating window shown on every workspace and drawn over the rest. So under
// Hyprland the window is pinned through the compositor's own request socket,
// the same one `hyprctl` talks to. Anywhere else this does nothing: the X11
// hints are what every other window manager reads.
const std = @import("std");

const alloc = std.heap.c_allocator;

extern "c" fn getenv(name: [*:0]const u8) ?[*:0]const u8;
extern "c" fn socket(domain: c_int, socktype: c_int, protocol: c_int) c_int;
extern "c" fn connect(fd: c_int, addr: *const anyopaque, len: u32) c_int;
extern "c" fn read(fd: c_int, buf: [*]u8, nbyte: usize) isize;
extern "c" fn write(fd: c_int, buf: [*]const u8, nbyte: usize) isize;
extern "c" fn close(fd: c_int) c_int;
extern "c" fn setsockopt(fd: c_int, level: c_int, optname: c_int, optval: *const anyopaque, optlen: u32) c_int;

const AF_UNIX: c_int = 1;
const SOCK_STREAM: c_int = 1;
const SOL_SOCKET: c_int = 1;
const SO_RCVTIMEO: c_int = 20;
const SO_SNDTIMEO: c_int = 21;
const SockaddrUn = extern struct { family: u16, path: [108]u8 };
const Timeval = extern struct { sec: c_long, usec: c_long };

/// Whether this session is a Hyprland one at all.
pub fn running() bool {
    return getenv("HYPRLAND_INSTANCE_SIGNATURE") != null;
}

/// One request on Hyprland's socket, answered in full. Bounded: the call runs
/// on the GTK thread, and a compositor that stops answering must not take the
/// app's event loop with it.
fn request(command: []const u8) ?[]u8 {
    const signature = getenv("HYPRLAND_INSTANCE_SIGNATURE") orelse return null;
    // Hyprland moved the socket from /tmp/hypr to $XDG_RUNTIME_DIR/hypr; both
    // are still in use.
    const bases = [_]?[*:0]const u8{ getenv("XDG_RUNTIME_DIR"), "/tmp" };
    for (bases) |base_opt| {
        const base = base_opt orelse continue;
        var addr = SockaddrUn{ .family = AF_UNIX, .path = [_]u8{0} ** 108 };
        _ = std.fmt.bufPrint(&addr.path, "{s}/hypr/{s}/.socket.sock", .{ std.mem.span(base), std.mem.span(signature) }) catch continue;
        const fd = socket(AF_UNIX, SOCK_STREAM, 0);
        if (fd < 0) return null;
        defer _ = close(fd);
        const limit = Timeval{ .sec = 0, .usec = 300_000 };
        _ = setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &limit, @sizeOf(Timeval));
        _ = setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &limit, @sizeOf(Timeval));
        if (connect(fd, &addr, @sizeOf(SockaddrUn)) != 0) continue;
        if (write(fd, command.ptr, command.len) != @as(isize, @intCast(command.len))) return null;
        var answer: std.ArrayList(u8) = .empty;
        var chunk: [16 * 1024]u8 = undefined;
        while (true) {
            const n = read(fd, &chunk, chunk.len);
            if (n <= 0) break;
            answer.appendSlice(alloc, chunk[0..@intCast(n)]) catch {
                answer.deinit(alloc);
                return null;
            };
        }
        return answer.toOwnedSlice(alloc) catch null;
    }
    return null;
}

pub const Pin = enum { pinned, not_yet, unavailable };

/// Pins this process's floating window titled `title`. `not_yet` while
/// Hyprland does not list it (the map is still on its way); `unavailable`
/// when there is no Hyprland to ask.
pub fn pinWindow(pid: u32, title: []const u8) Pin {
    const clients = request("j/clients") orelse return .unavailable;
    defer alloc.free(clients);
    const parsed = std.json.parseFromSlice(std.json.Value, alloc, clients, .{}) catch return .not_yet;
    defer parsed.deinit();
    if (parsed.value != .array) return .not_yet;
    for (parsed.value.array.items) |client| {
        if (client != .object) continue;
        const o = client.object;
        const client_pid = o.get("pid") orelse continue;
        if (client_pid != .integer or client_pid.integer != pid) continue;
        const client_title = o.get("title") orelse continue;
        if (client_title != .string or !std.mem.eql(u8, client_title.string, title)) continue;
        const address = o.get("address") orelse continue;
        if (address != .string) continue;
        // Pin toggles, and a user's own window rule may already have pinned it.
        const pinned = if (o.get("pinned")) |p| p == .bool and p.bool else false;
        if (pinned) return .pinned;
        // Pinning takes a floating window only.
        const floating = if (o.get("floating")) |f| f == .bool and f.bool else false;
        if (!floating) _ = dispatchOnWindow("setfloating", "float", address.string);
        return if (dispatchOnWindow("pin", "pin", address.string)) .pinned else .not_yet;
    }
    return .not_yet;
}

/// A window dispatcher by its legacy name, then by its Lua one: a session
/// configured in Lua (Hyprland 0.55 and later) reads `dispatch` as Lua and
/// refuses the legacy spelling.
fn dispatchOnWindow(legacy: []const u8, lua: []const u8, address: []const u8) bool {
    var cmd: [192]u8 = undefined;
    const old = std.fmt.bufPrint(&cmd, "dispatch {s} address:{s}", .{ legacy, address }) catch return false;
    if (request(old)) |answer| {
        defer alloc.free(answer);
        if (std.mem.startsWith(u8, answer, "ok")) return true;
    }
    const new = std.fmt.bufPrint(&cmd, "eval hl.dispatch(hl.dsp.window.{s}({{ action = \"enable\", window = \"address:{s}\" }}))", .{ lua, address }) catch return false;
    const answer = request(new) orelse return false;
    defer alloc.free(answer);
    return std.mem.startsWith(u8, answer, "ok");
}

