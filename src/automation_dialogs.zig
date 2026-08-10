//! ND_AUTOMATION_DIALOG_SCRIPT: scripted answers for native dialogs so
//! automated runs never block on real UI. The env var carries inline JSON
//! (or `@/path/to.json`) mapping a method name to a FIFO of responses:
//!
//!   { "dialog.openFile": [{ "paths": ["/tmp/a.txt"] }, { "paths": [] }],
//!     "window.showAlert": [{ "buttonId": "ok" }] }
//!
//! `take` shifts the head and returns it verbatim (the entry must carry the
//! method's real response/event shape). An exhausted queue answers
//! `.exhausted` — the interceptors in runtime.zig turn that into a LOUD
//! failure, never a silent fall-through to real UI. Honored only when
//! NATIVE_AUTOMATION=1. Consumed from the reader thread (systemRequest) and
//! the UI thread (widgetCommand), hence the mutex.

const std = @import("std");

pub const Next = union(enum) { unscripted, exhausted, response: []const u8 };

const Queue = struct { entries: [][]u8, next: usize = 0 };

// A spinlock, not std.Io.Mutex: this module has no std.Io handle, the
// critical section is a hashmap lookup, and contention (reader thread vs UI
// thread, both consulting rarely) is effectively nil.
var lock_state: std.atomic.Value(bool) = .init(false);
var initialized = false;
var queues: std.StringHashMapUnmanaged(Queue) = .empty;
var gpa: std.mem.Allocator = std.heap.page_allocator;

fn lock() void {
    while (lock_state.swap(true, .acquire)) std.atomic.spinLoopHint();
}
fn unlock() void {
    lock_state.store(false, .release);
}

/// Shifts the next scripted response for `method`. `.unscripted` when the
/// script never mentions the method (callers show the real dialog);
/// `.exhausted` when it did but the queue ran dry (callers fail loudly).
/// The returned slice is module-owned — never freed by the caller.
pub fn take(method: []const u8) Next {
    lock();
    defer unlock();
    ensureInit();
    const q = queues.getPtr(method) orelse return .unscripted;
    if (q.next >= q.entries.len) return .exhausted;
    const r = q.entries[q.next];
    q.next += 1;
    return .{ .response = r };
}

fn ensureInit() void {
    if (initialized) return;
    initialized = true;
    const auto = std.c.getenv("NATIVE_AUTOMATION") orelse return;
    if (!std.mem.eql(u8, std.mem.span(auto), "1")) return;
    const raw = std.c.getenv("ND_AUTOMATION_DIALOG_SCRIPT") orelse return;
    var script: []const u8 = std.mem.span(raw);
    if (script.len == 0) return;
    if (script[0] == '@') {
        script = readScriptFile(script[1..]) orelse return;
    }
    loadFromJson(gpa, script) catch {
        std.debug.print("ND_DIALOG_SCRIPT_ERROR malformed script\n", .{});
    };
}

/// libc-level read (the module has no std.Io handle; this runs once, before
/// any dialog can fire). Returns null (with a diagnostic) on any failure.
fn readScriptFile(path: []const u8) ?[]const u8 {
    const path_z = gpa.dupeZ(u8, path) catch return null;
    defer gpa.free(path_z);
    const f = std.c.fopen(path_z, "rb") orelse {
        std.debug.print("ND_DIALOG_SCRIPT_ERROR cannot open {s}\n", .{path});
        return null;
    };
    defer _ = std.c.fclose(f);
    var buf: std.ArrayList(u8) = .empty;
    var chunk: [4096]u8 = undefined;
    while (true) {
        const n = std.c.fread(&chunk, 1, chunk.len, f);
        if (n == 0) break;
        buf.appendSlice(gpa, chunk[0..n]) catch return null;
    }
    return buf.toOwnedSlice(gpa) catch null;
}

/// Parses `{ "<method>": [ <response>, ... ], ... }` into per-method FIFO
/// queues, each response re-serialized into a module-owned string. Split out
/// from `ensureInit` so tests can load a script without env vars.
fn loadFromJson(alloc: std.mem.Allocator, json: []const u8) !void {
    const parsed = try std.json.parseFromSlice(std.json.Value, alloc, json, .{});
    defer parsed.deinit();
    if (parsed.value != .object) return error.NotAnObject;
    var it = parsed.value.object.iterator();
    while (it.next()) |entry| {
        if (entry.value_ptr.* != .array) return error.MethodNotAnArray;
        const items = entry.value_ptr.array.items;
        const entries = try alloc.alloc([]u8, items.len);
        for (items, 0..) |item, i| {
            entries[i] = try std.json.Stringify.valueAlloc(alloc, item, .{});
        }
        const key = try alloc.dupe(u8, entry.key_ptr.*);
        try queues.put(alloc, key, .{ .entries = entries });
    }
}

/// Test-only reset (the process-lifetime maps are otherwise never torn down).
fn resetForTest(alloc: std.mem.Allocator) void {
    var it = queues.iterator();
    while (it.next()) |entry| {
        for (entry.value_ptr.entries) |e| alloc.free(e);
        alloc.free(entry.value_ptr.entries);
        alloc.free(entry.key_ptr.*);
    }
    queues.deinit(alloc);
    queues = .empty;
    initialized = false;
}

test "FIFO per method, exhausted after drain, unscripted for unknown" {
    const alloc = std.testing.allocator;
    initialized = true; // bypass env
    try loadFromJson(alloc,
        \\{"dialog.openFile":[{"paths":["/tmp/a.txt"]},{"paths":[]}],"window.showAlert":[{"buttonId":"ok"}]}
    );
    defer resetForTest(alloc);

    const first = take("dialog.openFile");
    try std.testing.expect(first == .response);
    try std.testing.expectEqualStrings("{\"paths\":[\"/tmp/a.txt\"]}", first.response);
    const second = take("dialog.openFile");
    try std.testing.expect(second == .response);
    try std.testing.expectEqualStrings("{\"paths\":[]}", second.response);
    try std.testing.expect(take("dialog.openFile") == .exhausted);
    try std.testing.expect(take("dialog.openFile") == .exhausted);

    const alert = take("window.showAlert");
    try std.testing.expect(alert == .response);
    try std.testing.expectEqualStrings("{\"buttonId\":\"ok\"}", alert.response);
    try std.testing.expect(take("window.showAlert") == .exhausted);

    try std.testing.expect(take("dialog.saveFile") == .unscripted);
}

test "malformed script leaves every method unscripted" {
    const alloc = std.testing.allocator;
    initialized = true;
    try std.testing.expectError(error.NotAnObject, loadFromJson(alloc, "[1,2]"));
    defer resetForTest(alloc);
    try std.testing.expect(take("dialog.openFile") == .unscripted);
}
