const std = @import("std");
const protocol = @import("protocol.zig");
const widget_types = @import("generated/widget_types.zig");

// NDP binary spec: docs/superpowers/specs/2026-07-09-ndp-binary-encoding.md
// This file is the M10 decoder+tracer for the CommitBatch binary layout that
// spec defines (§3-§9). protocol.zig stays JSON-only (spec §10); this is the
// sibling that decodes binary payloads into the same protocol.CommitBatch/Op
// shapes tree.apply already consumes.

pub const Decoded = struct {
    arena: *std.heap.ArenaAllocator,
    batch: protocol.CommitBatch,
    pub fn deinit(self: *Decoded) void {
        const gpa = self.arena.child_allocator;
        self.arena.deinit();
        gpa.destroy(self.arena);
    }
};

/// magic+version sniff (spec §3.1): payload[0]==0x4E, payload[1]==0x01.
pub fn isBinaryPayload(payload: []const u8) bool {
    return payload.len >= 2 and payload[0] == 0x4e and payload[1] == 0x01;
}

const StringTable = struct {
    entries: [][]const u8,
    fn get(self: StringTable, idx: u32) ![]const u8 {
        if (idx >= self.entries.len) return error.BadStringRef;
        return self.entries[idx];
    }
};

/// String table (spec §6): u32 count, then count x (u32 len, len bytes UTF-8, no NUL).
fn readStringTable(a: std.mem.Allocator, payload: []const u8, off: u32) !StringTable {
    if (off > payload.len or off + 4 > payload.len) return error.Truncated;
    var p: usize = off;
    const count = std.mem.readInt(u32, payload[p..][0..4], .little);
    p += 4;
    const entries = try a.alloc([]const u8, count);
    for (entries) |*e| {
        if (p + 4 > payload.len) return error.Truncated;
        const len = std.mem.readInt(u32, payload[p..][0..4], .little);
        p += 4;
        if (len > payload.len - p) return error.Truncated;
        e.* = try a.dupe(u8, payload[p .. p + len]); // owned by arena
        p += len;
    }
    return .{ .entries = entries };
}

/// Prop entry layout (spec §5.2): u32 keyRef, u8 valueTag, value per §5.3.
fn decodeProps(a: std.mem.Allocator, payload: []const u8, p: *usize, prop_count: u16, st: StringTable) !std.json.Value {
    var obj: std.json.ObjectMap = .empty;
    var i: u16 = 0;
    while (i < prop_count) : (i += 1) {
        if (p.* + 4 > payload.len) return error.Truncated;
        const key_ref = std.mem.readInt(u32, payload[p.*..][0..4], .little);
        p.* += 4;
        const key = try st.get(key_ref);
        if (p.* + 1 > payload.len) return error.Truncated;
        const tag = payload[p.*];
        p.* += 1;
        const v: std.json.Value = switch (tag) {
            0x00 => .null,
            0x01 => blk: {
                if (p.* + 1 > payload.len) return error.Truncated;
                const b = payload[p.*] != 0;
                p.* += 1;
                break :blk .{ .bool = b };
            },
            0x02 => blk: {
                if (p.* + 8 > payload.len) return error.Truncated;
                const n = std.mem.readInt(i64, payload[p.*..][0..8], .little);
                p.* += 8;
                break :blk .{ .integer = n };
            },
            0x03 => blk: {
                if (p.* + 8 > payload.len) return error.Truncated;
                const bits = std.mem.readInt(u64, payload[p.*..][0..8], .little);
                p.* += 8;
                break :blk .{ .float = @bitCast(bits) };
            },
            0x04 => blk: {
                if (p.* + 4 > payload.len) return error.Truncated;
                const r = std.mem.readInt(u32, payload[p.*..][0..4], .little);
                p.* += 4;
                break :blk .{ .string = try st.get(r) };
            },
            else => return error.BadValueTag,
        };
        try obj.put(a, key, v);
    }
    return .{ .object = obj };
}

fn readId(payload: []const u8, p: *usize) !u32 {
    if (p.* + 4 > payload.len) return error.Truncated;
    const v = std.mem.readInt(u32, payload[p.*..][0..4], .little);
    p.* += 4;
    return v;
}

/// Decodes a binary CommitBatch payload (spec §3-§7) into the same
/// protocol.CommitBatch/Op shapes the JSON path parses, so tree.apply can
/// consume either without a fork. Caller owns the returned Decoded and must
/// call .deinit() (frees the arena backing batch.ops/props/strings).
pub fn decodeCommitBatch(gpa: std.mem.Allocator, payload: []const u8) !Decoded {
    if (payload.len < 28) return error.Truncated;
    if (payload[0] != 0x4e) return error.BadMagic;
    if (payload[1] != 0x01) return error.BadVersion;
    const arena = try gpa.create(std.heap.ArenaAllocator);
    arena.* = std.heap.ArenaAllocator.init(gpa);
    errdefer {
        arena.deinit();
        gpa.destroy(arena);
    }
    const a = arena.allocator();

    const commit_id = std.mem.readInt(u64, payload[8..16], .little);
    const generation = std.mem.readInt(u32, payload[16..20], .little);
    const op_count = std.mem.readInt(u32, payload[20..24], .little);
    const st_off = std.mem.readInt(u32, payload[24..28], .little);
    const st = try readStringTable(a, payload, st_off);

    const ops = try a.alloc(protocol.Op, op_count);
    var p: usize = 28;
    for (ops) |*op| {
        if (p + 1 > payload.len) return error.Truncated;
        const opcode = payload[p];
        p += 1;
        switch (opcode) {
            0x01 => { // create
                const id = try readId(payload, &p);
                if (p + 2 > payload.len) return error.Truncated;
                const wt = std.mem.readInt(u16, payload[p..][0..2], .little);
                p += 2;
                const widget = widget_types.widgetNameOf(wt) orelse return error.BadWidgetType;
                if (p + 2 > payload.len) return error.Truncated;
                const pc = std.mem.readInt(u16, payload[p..][0..2], .little);
                p += 2;
                const props = try decodeProps(a, payload, &p, pc, st);
                op.* = protocol.Op{ .op = "create", .id = id, .widget = widget, .props = props };
            },
            0x02 => { // append
                const parent = try readId(payload, &p);
                const child = try readId(payload, &p);
                op.* = protocol.Op{ .op = "append", .parent = parent, .child = child };
            },
            0x03 => { // insertBefore
                const parent = try readId(payload, &p);
                const child = try readId(payload, &p);
                const before = try readId(payload, &p);
                op.* = protocol.Op{ .op = "insertBefore", .parent = parent, .child = child, .before = if (before == 0) null else before };
            },
            0x04 => { // remove
                const id = try readId(payload, &p);
                op.* = protocol.Op{ .op = "remove", .id = id };
            },
            0x05 => { // setText
                const id = try readId(payload, &p);
                const r = try readId(payload, &p);
                op.* = protocol.Op{ .op = "setText", .id = id, .text = try st.get(r) };
            },
            0x06 => { // update
                const id = try readId(payload, &p);
                if (p + 2 > payload.len) return error.Truncated;
                const pc = std.mem.readInt(u16, payload[p..][0..2], .little);
                p += 2;
                const props = try decodeProps(a, payload, &p, pc, st);
                op.* = protocol.Op{ .op = "update", .id = id, .props = props };
            },
            0x07 => { // hide
                const id = try readId(payload, &p);
                op.* = protocol.Op{ .op = "hide", .id = id };
            },
            0x08 => { // unhide
                const id = try readId(payload, &p);
                op.* = protocol.Op{ .op = "unhide", .id = id };
            },
            // spec §5.1: reject an unrecognized opcode rather than guess its
            // field width — unlike the JSON path's skip-and-continue, binary
            // framing has no self-delimiting fallback and would desync.
            else => return error.BadOpcode,
        }
    }
    return .{ .arena = arena, .batch = .{ .commitId = commit_id, .generation = generation, .ops = ops } };
}

/// Decodes then re-serializes to the same JSON text the JSON path produces
/// (NDP_TRACE parity, spec §9): protocol.CommitBatch already serializes the
/// way the JSON wire does, so tracing is just decode + json.Stringify.
pub fn traceToJson(gpa: std.mem.Allocator, payload: []const u8) ![]u8 {
    var decoded = try decodeCommitBatch(gpa, payload);
    defer decoded.deinit();
    return std.json.Stringify.valueAlloc(gpa, decoded.batch, .{});
}

// ndp-binary spec §8 golden vector: the counter demo's first commit.
// commitId=1, generation=0, ops=[create Window{title:Hi}, create Label{text:Clicks: 0},
// append(1,2), setText(2,"Clicks: 1")]. Bytes reproduced from §8.3.
//
// DEVIATION from the spec doc's prose arithmetic: §8.3's per-op byte-count
// comments ("19 bytes"/"18 bytes") and §8.4's "op stream = 55 bytes,
// stringTableOffset = 83" don't match the spec's own annotated byte offsets
// (28->46->64->73->82, i.e. 18+18+9+9 = 54 bytes). create's actual wire size
// is opcode(1)+id(4)+widgetType(2)+propCount(2)+propEntry(keyRef 4+tag 1+
// value 4=9) = 18 bytes, not 19 -- the spec's prose double-counts one byte.
// This fixture reproduces the spec's byte-for-byte hex dump (the
// authoritative bytes) with stringTableOffset corrected to 82 so the header
// is internally consistent with the op stream that follows it.
const golden_payload = [_]u8{
    // header (28 bytes)
    0x4e, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // magic,version,6x reserved
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // commitId=1
    0x00, 0x00, 0x00, 0x00, // generation=0
    0x04, 0x00, 0x00, 0x00, // opCount=4
    0x52, 0x00, 0x00, 0x00, // stringTableOffset=82 (corrected; see DEVIATION note above)
    // op[0] create Window(1) props{title:"Hi"}  (19 bytes)
    0x01, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x00,
    // op[1] create Label(2) props{text:"Clicks: 0"}  (18 bytes)
    0x01, 0x02, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x00,
    0x02, 0x00, 0x00, 0x00, 0x04, 0x03, 0x00, 0x00, 0x00,
    // op[2] append(1,2)  (9 bytes)
    0x02, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00,
    // op[3] setText(2, ref 4)  (9 bytes)
    0x05, 0x02, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00,
    // string table @82: count=5
    0x05, 0x00, 0x00, 0x00,
    0x05, 0x00, 0x00, 0x00, 0x74, 0x69, 0x74, 0x6c, 0x65, // "title"
    0x02, 0x00, 0x00, 0x00, 0x48, 0x69, // "Hi"
    0x04, 0x00, 0x00, 0x00, 0x74, 0x65, 0x78, 0x74, // "text"
    0x09, 0x00, 0x00, 0x00, 0x43, 0x6c, 0x69, 0x63, 0x6b, 0x73, 0x3a, 0x20, 0x30, // "Clicks: 0"
    0x09, 0x00, 0x00, 0x00, 0x43, 0x6c, 0x69, 0x63, 0x6b, 0x73, 0x3a, 0x20, 0x31, // "Clicks: 1"
};

test "golden vector decodes to the expected CommitBatch" {
    const gpa = std.testing.allocator;
    var decoded = try decodeCommitBatch(gpa, &golden_payload);
    defer decoded.deinit();
    const b = decoded.batch;
    try std.testing.expectEqual(@as(u64, 1), b.commitId);
    try std.testing.expectEqual(@as(u32, 0), b.generation);
    try std.testing.expectEqual(@as(usize, 4), b.ops.len);
    try std.testing.expectEqualStrings("create", b.ops[0].op);
    try std.testing.expectEqualStrings("Window", b.ops[0].widget.?);
    try std.testing.expectEqualStrings("Hi", b.ops[0].props.?.object.get("title").?.string);
    try std.testing.expectEqualStrings("Label", b.ops[1].widget.?);
    try std.testing.expectEqualStrings("append", b.ops[2].op);
    try std.testing.expectEqual(@as(u32, 1), b.ops[2].parent.?);
    try std.testing.expectEqual(@as(u32, 2), b.ops[2].child.?);
    try std.testing.expectEqualStrings("setText", b.ops[3].op);
    try std.testing.expectEqualStrings("Clicks: 1", b.ops[3].text.?);
}

test "isBinaryPayload sniffs magic+version" {
    try std.testing.expect(isBinaryPayload(&golden_payload));
    try std.testing.expect(!isBinaryPayload("{\"type\":\"commitBatch\"}"));
    try std.testing.expect(!isBinaryPayload(&[_]u8{ 0x4e, 0x02 })); // wrong version
}

test "traceToJson round-trips golden to greppable JSON" {
    const gpa = std.testing.allocator;
    const json = try traceToJson(gpa, &golden_payload);
    defer gpa.free(json);
    try std.testing.expect(std.mem.indexOf(u8, json, "\"commitId\":1") != null);
    try std.testing.expect(std.mem.indexOf(u8, json, "\"op\":\"create\"") != null);
    try std.testing.expect(std.mem.indexOf(u8, json, "\"widget\":\"Window\"") != null);
    try std.testing.expect(std.mem.indexOf(u8, json, "\"text\":\"Clicks: 1\"") != null);
}

test "rejects bad magic/version/opcode loudly" {
    const gpa = std.testing.allocator;
    var bad = golden_payload;
    bad[0] = 0x00; // wrong magic
    try std.testing.expectError(error.BadMagic, decodeCommitBatch(gpa, &bad));

    var bad_version = golden_payload;
    bad_version[1] = 0x02; // wrong version
    try std.testing.expectError(error.BadVersion, decodeCommitBatch(gpa, &bad_version));

    var bad_opcode = golden_payload;
    bad_opcode[28] = 0xff; // unassigned opcode, must reject not skip (spec §5.1)
    try std.testing.expectError(error.BadOpcode, decodeCommitBatch(gpa, &bad_opcode));

    try std.testing.expectError(error.Truncated, decodeCommitBatch(gpa, golden_payload[0..27]));
}
