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

/// Typed event payload. Exactly one field is set per event name (see the
/// schema's events[].payload): changed/activate -> text, toggled -> checked,
/// valueChanged -> value, selectionChanged -> index, clicked -> none.
/// Serialized with emit_null_optional_fields=false, so clicked still wires
/// as "payload":{} — byte-compatible with M4.
pub const EventPayload = struct {
    text: ?[]const u8 = null,
    checked: ?bool = null,
    value: ?f64 = null,
    index: ?i64 = null,
};

pub const Event = struct {
    type: []const u8 = "event",
    seq: u64,
    priority: []const u8 = "discrete",
    nodeId: u32,
    name: []const u8, // clicked | changed | activate | toggled | valueChanged | selectionChanged
    payload: EventPayload = .{},
};

/// Container attach metadata, extracted host-side from a child's create-op
/// props (gridRow/gridColumn/gridRowSpan/gridColumnSpan/tabLabel — the
/// schema's container.attachedProps). Not a wire frame itself; it rides
/// inside create-op props and is stashed in tree.NodeMeta.
/// `tab_label` is NOT owned here — tree.putMeta dupes it.
pub const Attached = struct {
    grid_row: i64 = 0,
    grid_column: i64 = 0,
    grid_row_span: i64 = 1,
    grid_column_span: i64 = 1,
    tab_label: ?[]const u8 = null,

    pub fn fromProps(props: ?std.json.Value) Attached {
        var a = Attached{};
        const v = props orelse return a;
        if (v != .object) return a;
        if (v.object.get("gridRow")) |f| { if (f == .integer) a.grid_row = f.integer; }
        if (v.object.get("gridColumn")) |f| { if (f == .integer) a.grid_column = f.integer; }
        if (v.object.get("gridRowSpan")) |f| { if (f == .integer) a.grid_row_span = f.integer; }
        if (v.object.get("gridColumnSpan")) |f| { if (f == .integer) a.grid_column_span = f.integer; }
        if (v.object.get("tabLabel")) |f| { if (f == .string) a.tab_label = f.string; }
        return a;
    }
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
    // insertBefore
    before: ?u32 = null,
};

pub const CommitBatch = struct {
    type: []const u8 = "commitBatch",
    commitId: u64,
    generation: u32,
    ops: []Op,
};

/// u32 LE length prefix + UTF-8 JSON. Caller frees the returned slice.
pub fn encodeFrameOpts(gpa: std.mem.Allocator, value: anytype, options: std.json.Stringify.Options) ![]u8 {
    const json = try std.json.Stringify.valueAlloc(gpa, value, options);
    defer gpa.free(json);
    const frame = try gpa.alloc(u8, 4 + json.len);
    std.mem.writeInt(u32, frame[0..4], @intCast(json.len), .little);
    @memcpy(frame[4..], json);
    return frame;
}

pub fn encodeFrame(gpa: std.mem.Allocator, value: anytype) ![]u8 {
    return encodeFrameOpts(gpa, value, .{});
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

test "event frame with payload omits null payload fields" {
    const gpa = std.testing.allocator;
    const ev = Event{ .seq = 7, .nodeId = 12, .name = "changed", .payload = .{ .text = "hi" } };
    const frame = try encodeFrameOpts(gpa, ev, .{ .emit_null_optional_fields = false });
    defer gpa.free(frame);
    const expected =
        \\{"type":"event","seq":7,"priority":"discrete","nodeId":12,"name":"changed","payload":{"text":"hi"}}
    ;
    try std.testing.expectEqualStrings(expected, frame[4..]);
}

test "clicked event payload stays empty object (M4 byte-compat)" {
    const gpa = std.testing.allocator;
    const ev = Event{ .seq = 1, .nodeId = 3, .name = "clicked" };
    const frame = try encodeFrameOpts(gpa, ev, .{ .emit_null_optional_fields = false });
    defer gpa.free(frame);
    try std.testing.expect(std.mem.indexOf(u8, frame[4..], "\"payload\":{}") != null);
}

test "attached fromProps extracts grid and tab metadata" {
    const gpa = std.testing.allocator;
    const parsed = try std.json.parseFromSlice(std.json.Value, gpa,
        "{\"gridRow\":2,\"gridColumn\":1,\"gridColumnSpan\":3,\"tabLabel\":\"Form\"}", .{});
    defer parsed.deinit();
    const a = Attached.fromProps(parsed.value);
    try std.testing.expectEqual(@as(i64, 2), a.grid_row);
    try std.testing.expectEqual(@as(i64, 1), a.grid_column);
    try std.testing.expectEqual(@as(i64, 1), a.grid_row_span);
    try std.testing.expectEqual(@as(i64, 3), a.grid_column_span);
    try std.testing.expectEqualStrings("Form", a.tab_label.?);
}

test "commitBatch with remove/insertBefore/hide/unhide decodes verbatim" {
    const gpa = std.testing.allocator;
    const doc =
        \\{"type":"commitBatch","commitId":7,"generation":1,"ops":[
        \\  {"op":"insertBefore","parent":2,"child":9,"before":3},
        \\  {"op":"remove","id":4},
        \\  {"op":"hide","id":5},
        \\  {"op":"unhide","id":5}
        \\]}
    ;
    const parsed = try std.json.parseFromSlice(CommitBatch, gpa, doc, .{ .ignore_unknown_fields = true });
    defer parsed.deinit();
    try std.testing.expectEqual(@as(u32, 1), parsed.value.generation);
    try std.testing.expectEqual(@as(usize, 4), parsed.value.ops.len);
    try std.testing.expectEqualStrings("insertBefore", parsed.value.ops[0].op);
    try std.testing.expectEqual(@as(u32, 2), parsed.value.ops[0].parent.?);
    try std.testing.expectEqual(@as(u32, 9), parsed.value.ops[0].child.?);
    try std.testing.expectEqual(@as(u32, 3), parsed.value.ops[0].before.?);
    try std.testing.expectEqualStrings("remove", parsed.value.ops[1].op);
    try std.testing.expectEqual(@as(u32, 4), parsed.value.ops[1].id.?);
    try std.testing.expectEqualStrings("hide", parsed.value.ops[2].op);
    try std.testing.expectEqualStrings("unhide", parsed.value.ops[3].op);
    try std.testing.expectEqual(@as(u32, 5), parsed.value.ops[3].id.?);
}
