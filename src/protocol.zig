const std = @import("std");

// Frame/struct shapes are GENERATED from schema/protocol.json (the single
// source of truth shared with the TS mirror,
// packages/react/src/generated/protocol.ts) — a field rename or type change
// there regenerates both sides, so drift is a compile error, not a silent
// wire break. This file re-exports them under the existing `protocol.*`
// names and keeps the hand-written framing/encode helpers + golden-byte
// tests below.
const frames = @import("generated/protocol.zig");

pub const ndp_version = frames.ndp_version;
pub const Runtime = frames.Runtime;
pub const Hello = frames.Hello;
pub const HelloAck = frames.HelloAck;
pub const ErrorFrame = frames.ErrorFrame;
pub const EventPayload = frames.EventPayload;
pub const Event = frames.Event;
pub const Op = frames.Op;
pub const CommitBatch = frames.CommitBatch;
pub const Ping = frames.Ping;
pub const Pong = frames.Pong;
pub const RuntimeError = frames.RuntimeError;
pub const PluginCommand = frames.PluginCommand;
pub const PluginResult = frames.PluginResult;
pub const WidgetCommand = frames.WidgetCommand;
pub const SystemRequest = frames.SystemRequest;
pub const SystemResponse = frames.SystemResponse;
pub const SystemEvent = frames.SystemEvent;

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
    tab_icon: ?[]const u8 = null,
    slot: ?[]const u8 = null,

    pub fn fromProps(props: ?std.json.Value) Attached {
        var a = Attached{};
        const v = props orelse return a;
        if (v != .object) return a;
        if (v.object.get("gridRow")) |f| {
            if (f == .integer) a.grid_row = f.integer;
        }
        if (v.object.get("gridColumn")) |f| {
            if (f == .integer) a.grid_column = f.integer;
        }
        if (v.object.get("gridRowSpan")) |f| {
            if (f == .integer) a.grid_row_span = f.integer;
        }
        if (v.object.get("gridColumnSpan")) |f| {
            if (f == .integer) a.grid_column_span = f.integer;
        }
        if (v.object.get("tabLabel")) |f| {
            if (f == .string) a.tab_label = f.string;
        }
        if (v.object.get("tabIcon")) |f| {
            if (f == .string) a.tab_icon = f.string;
        }
        if (v.object.get("slot")) |f| {
            if (f == .string) a.slot = f.string;
        }
        return a;
    }
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

/// Longest `type` value the fast path accepts. Frame type names are short
/// identifiers; a longer run of bytes is not one of ours.
const type_name_max = 32;

/// The frame's `type`, for routing, without parsing the frame.
///
/// Every frame the host receives is our own encoder's output
/// (runtime/ndp.ts), which builds each frame from an object literal whose
/// first member is `type`, so the value starts at a fixed offset and routing
/// costs a prefix compare instead of a JSON parse of the whole (possibly
/// multi-megabyte) frame. A frame shaped any other way falls back to the full
/// parse, whose result is copied into `scratch` because the parse arena dies
/// with this call. Null means the frame carries no routable `type`.
pub fn peekType(gpa: std.mem.Allocator, json_bytes: []const u8, scratch: []u8) ?[]const u8 {
    const prefix = "{\"type\":\"";
    if (std.mem.startsWith(u8, json_bytes, prefix)) {
        const rest = json_bytes[prefix.len..];
        const head = rest[0..@min(rest.len, type_name_max + 1)];
        if (std.mem.indexOfScalar(u8, head, '"')) |end| {
            // A backslash would mean the raw bytes and the decoded string
            // differ; no frame type contains one, so hand those to the parse
            // rather than returning something subtly wrong.
            if (std.mem.indexOfScalar(u8, head[0..end], '\\') == null) return rest[0..end];
        }
    }
    const TypeOnly = struct { type: []const u8 };
    const parsed = std.json.parseFromSlice(TypeOnly, gpa, json_bytes, .{ .ignore_unknown_fields = true }) catch return null;
    defer parsed.deinit();
    const name = parsed.value.type;
    if (name.len > scratch.len) return null;
    @memcpy(scratch[0..name.len], name);
    return scratch[0..name.len];
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
    var scratch: [type_name_max]u8 = undefined;
    try std.testing.expectEqualStrings("hello", peekType(gpa, "{\"type\":\"hello\",\"ndpVersion\":1}", &scratch).?);
    try std.testing.expectEqualStrings("commitBatch", peekType(gpa, "{\"type\":\"commitBatch\",\"commitId\":3,\"generation\":0,\"ops\":[]}", &scratch).?);
}

test "peekType falls back to the parse when type is not the first member" {
    const gpa = std.testing.allocator;
    var scratch: [type_name_max]u8 = undefined;
    // Not our encoder's shape: the prefix compare misses and the full parse answers.
    try std.testing.expectEqualStrings("ping", peekType(gpa, "{\"seq\":4,\"type\":\"ping\"}", &scratch).?);
    // Whitespace before the member, and an escaped (but legal) type value.
    try std.testing.expectEqualStrings("pong", peekType(gpa, "{ \"type\": \"pong\" }", &scratch).?);
    try std.testing.expectEqualStrings("event", peekType(gpa, "{\"type\":\"even\\u0074\"}", &scratch).?);
    // Nothing routable.
    try std.testing.expect(peekType(gpa, "{\"commitId\":1}", &scratch) == null);
    try std.testing.expect(peekType(gpa, "not json", &scratch) == null);
    try std.testing.expect(peekType(gpa, "", &scratch) == null);
}

test "peekType fast path does not run past the type-name bound" {
    const gpa = std.testing.allocator;
    var scratch: [type_name_max]u8 = undefined;
    // A frame whose `type` value is longer than any real frame name: the fast
    // path stops looking, and the fallback parse also refuses to copy a name
    // that cannot fit the caller's scratch buffer.
    const long = "{\"type\":\"" ++ ("x" ** (type_name_max + 8)) ++ "\"}";
    try std.testing.expect(peekType(gpa, long, &scratch) == null);
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

test "styleError event payload serializes key-only, other fields still omitted" {
    const gpa = std.testing.allocator;
    const ev = Event{ .seq = 9, .nodeId = 5, .name = "styleError", .payload = .{ .key = "display" } };
    const frame = try encodeFrameOpts(gpa, ev, .{ .emit_null_optional_fields = false });
    defer gpa.free(frame);
    const expected =
        \\{"type":"event","seq":9,"priority":"discrete","nodeId":5,"name":"styleError","payload":{"key":"display"}}
    ;
    try std.testing.expectEqualStrings(expected, frame[4..]);
}

test "attached fromProps extracts grid and tab metadata" {
    const gpa = std.testing.allocator;
    const parsed = try std.json.parseFromSlice(std.json.Value, gpa, "{\"gridRow\":2,\"gridColumn\":1,\"gridColumnSpan\":3,\"tabLabel\":\"Form\",\"slot\":\"sidebar\"}", .{});
    defer parsed.deinit();
    const a = Attached.fromProps(parsed.value);
    try std.testing.expectEqual(@as(i64, 2), a.grid_row);
    try std.testing.expectEqual(@as(i64, 1), a.grid_column);
    try std.testing.expectEqual(@as(i64, 1), a.grid_row_span);
    try std.testing.expectEqual(@as(i64, 3), a.grid_column_span);
    try std.testing.expectEqualStrings("Form", a.tab_label.?);
    try std.testing.expectEqualStrings("sidebar", a.slot.?);
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
