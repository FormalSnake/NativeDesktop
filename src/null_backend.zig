const std = @import("std");

// The null backend models a widget as an index into `nodes`. `tree.zig` holds
// `*Widget` pointers; we hand out `*Node` cast to the opaque widget pointer the
// tree stores. All state is inspectable for the conformance suite.
pub const Node = struct {
    kind: []const u8,
    text: ?[]const u8 = null,
    visible: bool = true,
    spacing: i64 = 0,
    orientation: []const u8 = "vertical",
    title: ?[]const u8 = null,
    children: std.ArrayList(*Node) = .empty,
    clicked_connected: bool = false,
};

// The tree stores `*Widget`; for the null backend `Widget` == `Node`.
pub const Widget = Node;

var gpa: std.mem.Allocator = undefined;
var initialized = false;
pub var nodes: std.ArrayList(*Node) = .empty;
pub var last_window: ?*Node = null;

pub fn init(allocator: std.mem.Allocator) void {
    gpa = allocator;
    initialized = true;
    nodes = .empty;
    last_window = null;
}

pub fn reset() void {
    for (nodes.items) |n| {
        n.children.deinit(gpa);
        gpa.destroy(n);
    }
    nodes.clearRetainingCapacity();
    last_window = null;
}

fn propStr(props: ?std.json.Value, key: []const u8) ?[]const u8 {
    const v = props orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .string => field.string,
        else => null,
    };
}
fn propInt(props: ?std.json.Value, key: []const u8) ?i64 {
    const v = props orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .integer => field.integer,
        else => null,
    };
}

pub fn setEventSink(_: *const fn (node_id: u32) void) void {}
pub fn getWindow() ?*Node {
    return last_window;
}

// Signature-parallel to gtk_backend.createWidget: same (app, kind, props) shape.
// `app` is unused (opaque *anyopaque so callers pass whatever they hold).
pub fn createWidget(_: *anyopaque, kind: []const u8, props: ?std.json.Value) !*Node {
    const node = try gpa.create(Node);
    node.* = .{ .kind = kind };
    if (std.mem.eql(u8, kind, "Window")) {
        node.title = propStr(props, "title");
        last_window = node;
    } else if (std.mem.eql(u8, kind, "Box")) {
        node.orientation = propStr(props, "orientation") orelse "vertical";
        node.spacing = propInt(props, "spacing") orelse 0;
    } else if (std.mem.eql(u8, kind, "Label")) {
        node.text = propStr(props, "text") orelse "";
    } else if (std.mem.eql(u8, kind, "Button")) {
        node.text = propStr(props, "label") orelse "Button";
    } else {
        gpa.destroy(node);
        return error.UnknownWidget;
    }
    try nodes.append(gpa, node);
    return node;
}

pub fn connectButtonClick(button: *Node, _: u32) void {
    button.clicked_connected = true;
}

pub fn appendChild(parent: *Node, child: *Node) void {
    parent.children.append(gpa, child) catch {};
}

pub fn setText(widget: *Node, text: []const u8) void {
    widget.text = text;
}

pub fn removeChild(parent: *Node, child: *Node) void {
    for (parent.children.items, 0..) |c, i| {
        if (c == child) {
            _ = parent.children.orderedRemove(i);
            return;
        }
    }
}

pub fn insertBefore(parent: *Node, child: *Node, before: ?*Node) void {
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

pub fn setVisible(widget: *Node, visible: bool) void {
    widget.visible = visible;
}

pub fn applyProps(widget: *Node, kind: []const u8, props: ?std.json.Value) void {
    if (std.mem.eql(u8, kind, "Box")) {
        if (propInt(props, "spacing")) |s| widget.spacing = s;
    } else if (std.mem.eql(u8, kind, "Window")) {
        if (propStr(props, "title")) |t| widget.title = t;
    }
}
