// Capability ACL model (M10 T6): per-window, namespaced permission grants.
//
// Default policy grants the core UI ops (`core:commit`, `core:window.create`)
// for every window so existing demos never break; everything else
// (`plugin:*` and any other explicitly-namespaced privileged permission)
// is default-deny. A grants manifest (JSON) can extend — never shrink — the
// default via per-window or `defaultWindow` (applies to all windows)
// permission sets. `window: 0` is treated the same as `defaultWindow`: the
// current single-window demos use window 0, so grants written against it
// must apply everywhere.
const std = @import("std");

const PermSet = std.StringHashMapUnmanaged(void);

pub const Acl = struct {
    arena: std.heap.ArenaAllocator,
    default_perms: PermSet = .{},
    per_window: std.AutoHashMapUnmanaged(u32, PermSet) = .{},

    /// Core UI ops granted by default so every existing demo keeps working;
    /// everything else (plugin commands, other privileged permissions) is
    /// default-deny.
    pub fn initDefault(gpa: std.mem.Allocator) Acl {
        var self = Acl{ .arena = std.heap.ArenaAllocator.init(gpa) };
        const a = self.arena.allocator();
        self.default_perms.put(a, "core:commit", {}) catch {};
        self.default_perms.put(a, "core:window.create", {}) catch {};
        return self;
    }

    /// Parses a grants manifest and extends the safe default. Malformed or
    /// non-object JSON falls back to `initDefault` alone (never breaks the
    /// core ops; never grants anything privileged).
    pub fn parse(gpa: std.mem.Allocator, json: []const u8) !Acl {
        var self = Acl.initDefault(gpa);
        const a = self.arena.allocator();
        const parsed = std.json.parseFromSlice(std.json.Value, gpa, json, .{}) catch return self;
        defer parsed.deinit();
        const root = parsed.value;
        if (root != .object) return self;

        if (root.object.get("defaultWindow")) |dw| {
            if (dw == .array) {
                for (dw.array.items) |item| {
                    if (item == .string) {
                        self.default_perms.put(a, try a.dupe(u8, item.string), {}) catch {};
                    }
                }
            }
        }

        if (root.object.get("grants")) |g| {
            if (g == .array) {
                for (g.array.items) |grant| {
                    if (grant != .object) continue;
                    const win: u32 = if (grant.object.get("window")) |w|
                        (if (w == .integer) @intCast(w.integer) else 0)
                    else
                        0;
                    const perms = grant.object.get("permissions") orelse continue;
                    if (perms != .array) continue;
                    const gop = self.per_window.getOrPut(a, win) catch continue;
                    if (!gop.found_existing) gop.value_ptr.* = .{};
                    for (perms.array.items) |p| {
                        if (p == .string) {
                            gop.value_ptr.put(a, try a.dupe(u8, p.string), {}) catch {};
                        }
                    }
                }
            }
        }

        return self;
    }

    /// True if `permission` is granted for `window_id`: either via the
    /// default set (applies to every window), the window's own explicit
    /// grants, or — since window 0 grants are the common "all windows" case
    /// for single-window demos — window 0's explicit grants.
    pub fn isAllowed(self: *Acl, window_id: u32, permission: []const u8) bool {
        if (self.default_perms.contains(permission)) return true;
        if (self.per_window.get(window_id)) |set| {
            if (set.contains(permission)) return true;
        }
        if (window_id != 0) {
            if (self.per_window.get(0)) |set0| {
                if (set0.contains(permission)) return true;
            }
        }
        return false;
    }

    pub fn deinit(self: *Acl) void {
        self.arena.deinit();
    }
};

test "default policy grants core ops, denies plugin ops" {
    var acl = Acl.initDefault(std.testing.allocator);
    defer acl.deinit();
    try std.testing.expect(acl.isAllowed(0, "core:commit"));
    try std.testing.expect(acl.isAllowed(7, "core:window.create")); // any window
    try std.testing.expect(!acl.isAllowed(0, "plugin:hello.greet")); // default-deny
    try std.testing.expect(!acl.isAllowed(0, "core:fs.write")); // unknown privileged
}

test "parsed grants extend the default" {
    const json =
        \\{"defaultWindow":["core:commit","core:window.create"],
        \\ "grants":[{"window":0,"permissions":["plugin:hello.greet"]}]}
    ;
    var acl = try Acl.parse(std.testing.allocator, json);
    defer acl.deinit();
    try std.testing.expect(acl.isAllowed(0, "core:commit"));
    try std.testing.expect(acl.isAllowed(0, "plugin:hello.greet"));
    try std.testing.expect(!acl.isAllowed(0, "plugin:other.cmd"));
}

test "empty/malformed manifest falls back to safe default (core granted)" {
    var acl = try Acl.parse(std.testing.allocator, "not json");
    defer acl.deinit();
    try std.testing.expect(acl.isAllowed(0, "core:commit")); // never break demos
    try std.testing.expect(!acl.isAllowed(0, "plugin:x.y"));
}

test "grant on a non-zero window is isolated (per-window, not global)" {
    const json =
        \\{"grants":[{"window":3,"permissions":["plugin:only.on.three"]}]}
    ;
    var acl = try Acl.parse(std.testing.allocator, json);
    defer acl.deinit();
    try std.testing.expect(acl.isAllowed(3, "plugin:only.on.three"));
    try std.testing.expect(!acl.isAllowed(4, "plugin:only.on.three"));
    // core defaults remain granted everywhere regardless of explicit grants.
    try std.testing.expect(acl.isAllowed(4, "core:commit"));
}

test "wildcard-shaped literal permission strings are not pattern-matched" {
    // The ACL is a plain namespaced string set: "plugin:*" only matches the
    // literal string "plugin:*", it is not a glob over "plugin:foo".
    const json =
        \\{"grants":[{"window":0,"permissions":["plugin:*"]}]}
    ;
    var acl = try Acl.parse(std.testing.allocator, json);
    defer acl.deinit();
    try std.testing.expect(acl.isAllowed(0, "plugin:*"));
    try std.testing.expect(!acl.isAllowed(0, "plugin:hello.greet"));
}

test "deny precedence: absence of a grant always denies, and window-0 grants apply to all windows" {
    const json =
        \\{"grants":[{"window":0,"permissions":["plugin:hello.greet"]}]}
    ;
    var acl = try Acl.parse(std.testing.allocator, json);
    defer acl.deinit();
    try std.testing.expect(acl.isAllowed(0, "plugin:hello.greet"));
    try std.testing.expect(!acl.isAllowed(0, "plugin:hello.other"));
    // window 0 grants are the "applies to all windows" case.
    try std.testing.expect(acl.isAllowed(1, "plugin:hello.greet"));
    try std.testing.expect(!acl.isAllowed(1, "plugin:hello.other"));
}
