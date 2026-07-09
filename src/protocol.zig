const std = @import("std");

pub const ndp_version: u32 = 1;

pub const Hello = struct {
    type: []const u8 = "hello",
    ndpVersion: u32,
    runtime: Runtime,
    pub const Runtime = struct { name: []const u8, version: []const u8 };
};

pub const HelloAck = struct {
    type: []const u8 = "helloAck",
    ndpVersion: u32,
    encodings: []const []const u8,
};

pub const ErrorFrame = struct {
    type: []const u8 = "error",
    message: []const u8,
    expected: u32,
    got: u32,
};

pub const Event = struct {
    type: []const u8 = "event",
    seq: u64,
    priority: []const u8 = "discrete",
    nodeId: u32,
    name: []const u8,
    payload: struct {} = .{},
};

/// One of create|append|setText|update. Decoded with a permissive struct:
/// optional fields cover the union of all op shapes; the `op` string discriminates.
pub const Op = struct {
    op: []const u8,
    // create
    id: ?u32 = null,
    widget: ?[]const u8 = null, // "Window" | "Box" | "Label" | "Button"
    props: ?std.json.Value = null,
    // append
    parent: ?u32 = null,
    child: ?u32 = null,
    // setText
    text: ?[]const u8 = null,
};

pub const CommitBatch = struct {
    type: []const u8 = "commitBatch",
    commitId: u64,
    generation: u32,
    ops: []Op,
};

/// u32 LE length prefix + UTF-8 JSON. Caller frees the returned slice.
pub fn encodeFrame(gpa: std.mem.Allocator, value: anytype) ![]u8 {
    const json = try std.json.Stringify.valueAlloc(gpa, value, .{});
    defer gpa.free(json);
    const frame = try gpa.alloc(u8, 4 + json.len);
    std.mem.writeInt(u32, frame[0..4], @intCast(json.len), .little);
    @memcpy(frame[4..], json);
    return frame;
}

/// Reads only the "type" field so the reader can route without a full union parse.
pub fn peekType(gpa: std.mem.Allocator, json_bytes: []const u8) ![]const u8 {
    const TypeOnly = struct { type: []const u8 };
    const parsed = try std.json.parseFromSlice(TypeOnly, gpa, json_bytes, .{ .ignore_unknown_fields = true });
    defer parsed.deinit();
    return gpa.dupe(u8, parsed.value.type);
}

test "encodeFrame writes u32 LE length prefix + json, golden bytes" {
    const gpa = std.testing.allocator;
    const hello = Hello{ .ndpVersion = 1, .runtime = .{ .name = "bun", .version = "1.3.13" } };
    const frame = try encodeFrame(gpa, hello);
    defer gpa.free(frame);

    // Prefix is the JSON byte length, little-endian.
    const json_len = std.mem.readInt(u32, frame[0..4], .little);
    try std.testing.expectEqual(@as(usize, json_len), frame.len - 4);

    // Round-trips back to the same struct.
    const parsed = try std.json.parseFromSlice(Hello, gpa, frame[4..], .{ .ignore_unknown_fields = true });
    defer parsed.deinit();
    try std.testing.expectEqual(@as(u32, 1), parsed.value.ndpVersion);
    try std.testing.expectEqualStrings("bun", parsed.value.runtime.name);

    // Byte-layout anchor: the first four bytes are the length low byte first,
    // and byte 4 is '{'.
    try std.testing.expectEqual(@as(u8, '{'), frame[4]);
    try std.testing.expectEqual(frame[0], @as(u8, @truncate(json_len)));
}

test "peekType routes commitBatch vs hello" {
    const gpa = std.testing.allocator;
    const t1 = try peekType(gpa, "{\"type\":\"hello\",\"ndpVersion\":1}");
    defer gpa.free(t1);
    try std.testing.expectEqualStrings("hello", t1);

    const t2 = try peekType(gpa, "{\"type\":\"commitBatch\",\"commitId\":3,\"generation\":0,\"ops\":[]}");
    defer gpa.free(t2);
    try std.testing.expectEqualStrings("commitBatch", t2);
}

test "commitBatch with a create op decodes with field names verbatim" {
    const gpa = std.testing.allocator;
    const doc =
        \\{"type":"commitBatch","commitId":1,"generation":0,"ops":[
        \\  {"op":"create","id":1,"widget":"Window","props":{"title":"Hi","defaultWidth":480,"defaultHeight":320}},
        \\  {"op":"create","id":2,"widget":"Box","props":{"orientation":"vertical","spacing":8}},
        \\  {"op":"append","parent":1,"child":2},
        \\  {"op":"create","id":3,"widget":"Label","props":{"text":"Clicks: 0"}},
        \\  {"op":"setText","id":3,"text":"Clicks: 1"}
        \\]}
    ;
    const parsed = try std.json.parseFromSlice(CommitBatch, gpa, doc, .{ .ignore_unknown_fields = true });
    defer parsed.deinit();
    try std.testing.expectEqual(@as(u64, 1), parsed.value.commitId);
    try std.testing.expectEqual(@as(usize, 5), parsed.value.ops.len);
    try std.testing.expectEqualStrings("create", parsed.value.ops[0].op);
    try std.testing.expectEqualStrings("Window", parsed.value.ops[0].widget.?);
    try std.testing.expectEqual(@as(u32, 2), parsed.value.ops[2].child.?);
    try std.testing.expectEqualStrings("Clicks: 1", parsed.value.ops[4].text.?);
}
