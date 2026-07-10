const std = @import("std");
const protocol = @import("protocol.zig");

/// Schema-driven in-memory backend. `init(gpa, schema_json)` parses the widget
/// schema once; `createWidget`/`applyProps` record canonical-stringified prop
/// values generically (keyed by schema prop name), so conformance covers every
/// schema widget with zero per-widget code here. Structural ops honor the
/// schema's `container.childModel` (single/multi); event connection honors
/// `events[]`. All state is inspectable for the conformance suite.
pub const Node = struct {
    kind: []const u8, // points into the parsed schema (stable until deinitAll)
    props: std.StringArrayHashMapUnmanaged([]const u8) = .empty, // name -> canonical JSON string, gpa-owned
    text: ?[]const u8 = null,
    visible: bool = true,
    attached: protocol.Attached = .{},
    children: std.ArrayList(*Node) = .empty,
    events: std.ArrayList([]const u8) = .empty, // connected event names, schema order
};

// The tree stores `*Widget`; for the null backend `Widget` == `Node`.
pub const Widget = Node;

const ChildModel = enum { none, single, multi };

const WidgetDef = struct {
    name: []const u8,
    child_model: ChildModel,
    // events + props are read straight from the retained parsed schema Value.
    json: std.json.Value,
};

var gpa: std.mem.Allocator = undefined;
var parsed_schema: ?std.json.Parsed(std.json.Value) = null;
var defs: std.StringArrayHashMapUnmanaged(WidgetDef) = .empty;
pub var nodes: std.ArrayList(*Node) = .empty;
pub var last_window: ?*Node = null;

pub fn init(allocator: std.mem.Allocator, schema_json: []const u8) !void {
    gpa = allocator;
    parsed_schema = try std.json.parseFromSlice(std.json.Value, gpa, schema_json, .{});
    const widgets = parsed_schema.?.value.object.get("widgets").?.array;
    for (widgets.items) |w| {
        const name = w.object.get("name").?.string;
        const container = w.object.get("container").?;
        const cm: ChildModel = if (container == .null)
            .none
        else if (std.mem.eql(u8, container.object.get("childModel").?.string, "single"))
            .single
        else
            .multi;
        try defs.put(gpa, name, .{ .name = name, .child_model = cm, .json = w });
    }
}

/// Frees every node's owned allocations (canonical prop strings, children and
/// events lists) and the nodes themselves. Keeps `defs`/`parsed_schema` alive
/// so repeated init+reset in the same test binary is not required.
pub fn reset() void {
    for (nodes.items) |n| {
        var it = n.props.iterator();
        while (it.next()) |kv| gpa.free(kv.value_ptr.*);
        n.props.deinit(gpa);
        n.children.deinit(gpa);
        n.events.deinit(gpa);
        gpa.destroy(n);
    }
    nodes.deinit(gpa);
    nodes = .empty;
    last_window = null;
}

/// Full teardown: `reset()` plus the schema-derived state. Call from test
/// teardown once no further `init` calls are expected on this backend.
pub fn deinitAll() void {
    reset();
    defs.deinit(gpa);
    defs = .empty;
    if (parsed_schema) |*p| p.deinit();
    parsed_schema = null;
}

fn canon(value: std.json.Value) ![]const u8 {
    return std.json.Stringify.valueAlloc(gpa, value, .{});
}

pub fn setEventSink(_: *const fn (u32, []const u8, protocol.EventPayload) void) void {}
pub fn getWindow() ?*Node {
    return last_window;
}

// Signature-parallel to gtk_backend.createWidget: same (app, kind, props) shape.
// `app` is unused (opaque *anyopaque so callers pass whatever they hold).
pub fn createWidget(_: *anyopaque, kind: []const u8, props: ?std.json.Value) !*Node {
    const def = defs.get(kind) orelse return error.UnknownWidget;
    const node = try gpa.create(Node);
    node.* = .{ .kind = def.name };
    // Record every schema-declared non-meta prop: explicit value, else schema default.
    const sprops = def.json.object.get("props").?.array;
    for (sprops.items) |sp| {
        const pname = sp.object.get("name").?.string;
        const applies = sp.object.get("appliesTo").?.string;
        if (std.mem.eql(u8, applies, "meta")) continue;
        if (props != null and props.? == .object) {
            if (props.?.object.get(pname)) |v| {
                try node.props.put(gpa, pname, try canon(v));
                continue;
            }
        }
        if (sp.object.get("default")) |d| try node.props.put(gpa, pname, try canon(d));
    }
    // `style` is a cross-cutting prop injected into every intrinsic by codegen
    // (not a per-widget schema entry, mirrors tree.zig's applyStyleIfPresent),
    // so it's recorded generically here rather than via the schema prop loop.
    if (props != null and props.? == .object) {
        if (props.?.object.get("style")) |st| try node.props.put(gpa, "style", try canon(st));
    }
    if (std.mem.eql(u8, kind, "Window")) last_window = node;
    try nodes.append(gpa, node);
    return node;
}

pub fn applyProps(widget: *Node, kind: []const u8, props: ?std.json.Value) void {
    const def = defs.get(kind) orelse return;
    const v = props orelse return;
    if (v != .object) return;
    const sprops = def.json.object.get("props").?.array;
    for (sprops.items) |sp| {
        const pname = sp.object.get("name").?.string;
        if (!std.mem.eql(u8, sp.object.get("appliesTo").?.string, "createAndUpdate")) continue;
        if (v.object.get(pname)) |pv| {
            const c = canon(pv) catch continue;
            if (widget.props.fetchPut(gpa, pname, c) catch null) |old| gpa.free(old.value);
        }
    }
    if (v.object.get("style")) |st| {
        const c = canon(st) catch return;
        if (widget.props.fetchPut(gpa, "style", c) catch null) |old| gpa.free(old.value);
    }
}

pub fn connectEvents(node: *Node, kind: []const u8, node_id: u32) void {
    _ = node_id;
    const def = defs.get(kind) orelse return;
    const sevents = def.json.object.get("events").?.array;
    for (sevents.items) |se| {
        node.events.append(gpa, se.object.get("name").?.string) catch {};
    }
}

pub fn appendChild(parent: *Node, parent_kind: []const u8, child: *Node, attached: protocol.Attached) void {
    child.attached = attached;
    const def = defs.get(parent_kind) orelse return;
    if (def.child_model == .single) parent.children.clearRetainingCapacity();
    parent.children.append(gpa, child) catch {};
}

pub fn insertBefore(parent: *Node, parent_kind: []const u8, child: *Node, before: ?*Node, attached: protocol.Attached) void {
    child.attached = attached;
    const def = defs.get(parent_kind) orelse return;
    if (def.child_model == .single) {
        parent.children.clearRetainingCapacity();
        parent.children.append(gpa, child) catch {};
        return;
    }
    if (before) |b| {
        for (parent.children.items, 0..) |c, i| {
            if (c == b) {
                parent.children.insert(gpa, i, child) catch {};
                return;
            }
        }
    }
    parent.children.append(gpa, child) catch {};
}

pub fn removeChild(parent: *Node, parent_kind: []const u8, child: *Node) void {
    _ = parent_kind;
    for (parent.children.items, 0..) |c, i| {
        if (c == child) {
            _ = parent.children.orderedRemove(i);
            return;
        }
    }
}

pub fn setText(widget: *Node, text: []const u8) void {
    widget.text = text;
}

pub fn setVisible(widget: *Node, visible: bool) void {
    widget.visible = visible;
}

// Keeps the backend interface uniform with gtk_backend.zig; the null backend
// has no display to install a CSS provider on. Conformance asserts `style`
// rides create/update props generically (Task 6) instead.
pub fn applyStyle(_: *Node, _: u32, _: std.json.Value) void {}

// Generation GC helpers (M8-D9): the null backend has no real widget tree to
// unparent from — conformance for gcOldGenerations exercises tree.zig's meta
// map directly (see the "generation bump" test in tree.zig), so these are
// no-ops that keep the backend interface uniform.
pub fn hasParent(_: *Node) bool {
    return false;
}
pub fn unparentWidget(_: *Node) void {}
