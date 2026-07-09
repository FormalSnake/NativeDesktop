const std = @import("std");
const gtk = @import("gtk");
const protocol = @import("protocol.zig");
const backend = @import("backend.zig").impl;

const Widget = backend.Widget;

/// Host-side node metadata: per-node type/testID/text/parent, independent of
/// the live `*gtk.Widget`. Populated in `apply` alongside `nodes.put`; this is
/// the snapshot source of truth for the automation `getTree` RPC (M4) — it is
/// never re-derived from props at query time.
pub const NodeMeta = struct {
    widget_type: []u8,
    test_id: ?[]u8,
    text: ?[]u8,
    parent: u32,
    attached: protocol.Attached = .{}, // tab_label duped/owned here
};

fn propStr(props: ?std.json.Value, key: []const u8) ?[]const u8 {
    const v = props orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .string => field.string,
        else => null,
    };
}

/// Invoked from both the create and update arms of `apply` — style is only
/// compiled when `props.style` is present (regression contract: no style =
/// zero CSS provider, zero margin call, byte-identical to M5b).
fn applyStyleIfPresent(widget: *Widget, id: u32, props: ?std.json.Value) void {
    const v = props orelse return;
    if (v != .object) return;
    if (v.object.get("style")) |st| backend.applyStyle(widget, id, st);
}

pub const Tree = struct {
    gpa: std.mem.Allocator,
    app: ?*gtk.Application,
    nodes: std.AutoHashMapUnmanaged(u32, *Widget) = .{},
    meta: std.AutoHashMapUnmanaged(u32, NodeMeta) = .{},
    generation: u32 = 0,

    pub fn init(gpa: std.mem.Allocator, app: *gtk.Application) Tree {
        return .{ .gpa = gpa, .app = app };
    }

    /// A ctor that skips the `*gtk.Application` for pure-meta unit tests
    /// (no real GTK app needed to exercise the meta map in isolation).
    pub fn initBare(gpa: std.mem.Allocator) Tree {
        return .{ .gpa = gpa, .app = null };
    }

    pub fn get(self: *Tree, id: u32) ?*Widget {
        return self.nodes.get(id);
    }

    pub fn metaGet(self: *Tree, id: u32) ?*NodeMeta {
        return self.meta.getPtr(id);
    }

    /// The sole Window node id, or null if none registered yet.
    pub fn rootId(self: *Tree) ?u32 {
        var it = self.meta.iterator();
        while (it.next()) |entry| {
            if (std.mem.eql(u8, entry.value_ptr.widget_type, "Window")) return entry.key_ptr.*;
        }
        return null;
    }

    fn dupeOpt(self: *Tree, s: ?[]const u8) !?[]u8 {
        const v = s orelse return null;
        return try self.gpa.dupe(u8, v);
    }

    pub fn putMeta(self: *Tree, id: u32, widget_type: []const u8, test_id: ?[]const u8, text: ?[]const u8, parent: u32, attached: protocol.Attached) !void {
        const owned_type = try self.gpa.dupe(u8, widget_type);
        errdefer self.gpa.free(owned_type);
        const owned_test_id = try self.dupeOpt(test_id);
        errdefer if (owned_test_id) |v| self.gpa.free(v);
        const owned_text = try self.dupeOpt(text);
        errdefer if (owned_text) |v| self.gpa.free(v);
        var owned_attached = attached;
        owned_attached.tab_label = try self.dupeOpt(attached.tab_label);
        errdefer if (owned_attached.tab_label) |v| self.gpa.free(v);
        try self.meta.put(self.gpa, id, .{
            .widget_type = owned_type,
            .test_id = owned_test_id,
            .text = owned_text,
            .parent = parent,
            .attached = owned_attached,
        });
    }

    pub fn setMetaParent(self: *Tree, id: u32, parent: u32) void {
        if (self.meta.getPtr(id)) |m| m.parent = parent;
    }

    pub fn setMetaText(self: *Tree, id: u32, text: []const u8) void {
        const m = self.meta.getPtr(id) orelse return;
        if (m.text) |old| self.gpa.free(old);
        m.text = self.gpa.dupe(u8, text) catch null;
    }

    pub fn setMetaTestId(self: *Tree, id: u32, test_id: ?[]const u8) void {
        const m = self.meta.getPtr(id) orelse return;
        if (m.test_id) |old| self.gpa.free(old);
        m.test_id = self.dupeOpt(test_id) catch null;
    }

    pub fn removeMeta(self: *Tree, id: u32) void {
        const kv = self.meta.fetchRemove(id) orelse return;
        self.gpa.free(kv.value.widget_type);
        if (kv.value.test_id) |v| self.gpa.free(v);
        if (kv.value.text) |v| self.gpa.free(v);
        if (kv.value.attached.tab_label) |v| self.gpa.free(v);
    }

    pub fn deinitMeta(self: *Tree) void {
        var it = self.meta.iterator();
        while (it.next()) |entry| {
            self.gpa.free(entry.value_ptr.widget_type);
            if (entry.value_ptr.test_id) |v| self.gpa.free(v);
            if (entry.value_ptr.text) |v| self.gpa.free(v);
            if (entry.value_ptr.attached.tab_label) |v| self.gpa.free(v);
        }
        self.meta.deinit(self.gpa);
    }

    /// UI-thread only. Applies an entire commit batch as one unit.
    pub fn apply(self: *Tree, batch: protocol.CommitBatch) void {
        self.generation = batch.generation;
        for (batch.ops) |op| {
            if (std.mem.eql(u8, op.op, "create")) {
                const app = self.app orelse continue;
                const widget = backend.createWidget(app, op.widget.?, op.props) catch continue;
                backend.connectEvents(widget, op.widget.?, op.id.?);
                self.nodes.put(self.gpa, op.id.?, widget) catch continue;
                // testID is stored here for the automation getTree RPC (M4) and is
                // never applied to the GTK widget itself.
                const test_id = propStr(op.props, "testID");
                const initial_text = propStr(op.props, "text") orelse propStr(op.props, "label");
                const attached = protocol.Attached.fromProps(op.props);
                self.putMeta(op.id.?, op.widget.?, test_id, initial_text, 0, attached) catch {};
                applyStyleIfPresent(widget, op.id.?, op.props);
            } else if (std.mem.eql(u8, op.op, "append")) {
                const parent_widget = self.nodes.get(op.parent.?) orelse continue;
                const child_widget = self.nodes.get(op.child.?) orelse continue;
                const pmeta = self.metaGet(op.parent.?) orelse continue;
                const cmeta = self.metaGet(op.child.?) orelse continue;
                backend.appendChild(parent_widget, pmeta.widget_type, child_widget, cmeta.attached);
                self.setMetaParent(op.child.?, op.parent.?);
            } else if (std.mem.eql(u8, op.op, "setText")) {
                const widget = self.nodes.get(op.id.?) orelse continue;
                backend.setText(widget, op.text.?);
                self.setMetaText(op.id.?, op.text.?);
            } else if (std.mem.eql(u8, op.op, "update")) {
                const widget = self.nodes.get(op.id.?) orelse continue;
                backend.applyProps(widget, op.widget orelse "", op.props);
                applyStyleIfPresent(widget, op.id.?, op.props);
                if (propStr(op.props, "testID")) |t| self.setMetaTestId(op.id.?, t);
                if (propStr(op.props, "text") orelse propStr(op.props, "label")) |t| {
                    self.setMetaText(op.id.?, t);
                }
            } else if (std.mem.eql(u8, op.op, "insertBefore")) {
                const parent_widget = self.nodes.get(op.parent.?) orelse continue;
                const child_widget = self.nodes.get(op.child.?) orelse continue;
                const before: ?*Widget = if (op.before) |b| self.nodes.get(b) else null;
                const pmeta = self.metaGet(op.parent.?) orelse continue;
                const cmeta = self.metaGet(op.child.?) orelse continue;
                backend.insertBefore(parent_widget, pmeta.widget_type, child_widget, before, cmeta.attached);
                self.setMetaParent(op.child.?, op.parent.?);
            } else if (std.mem.eql(u8, op.op, "remove")) {
                const child = self.nodes.get(op.id.?) orelse continue;
                if (child.getParent()) |parent| {
                    const parent_kind = if (self.metaGet(op.id.?)) |cmeta|
                        (if (self.metaGet(cmeta.parent)) |pmeta| pmeta.widget_type else "")
                    else
                        "";
                    backend.removeChild(parent, parent_kind, child);
                }
                _ = self.nodes.remove(op.id.?);
                self.removeMeta(op.id.?);
                std.debug.print("ND_REMOVE id={d}\n", .{op.id.?});
            } else if (std.mem.eql(u8, op.op, "hide")) {
                const widget = self.nodes.get(op.id.?) orelse continue;
                backend.setVisible(widget, false);
                std.debug.print("ND_HIDE id={d}\n", .{op.id.?});
            } else if (std.mem.eql(u8, op.op, "unhide")) {
                const widget = self.nodes.get(op.id.?) orelse continue;
                backend.setVisible(widget, true);
                std.debug.print("ND_UNHIDE id={d}\n", .{op.id.?});
            } else {
                std.debug.print("ND_WARN unknown op={s}\n", .{op.op});
            }
        }
        std.debug.print("ND_COMMIT_APPLIED commitId={d}\n", .{batch.commitId});
    }
};

test "node meta stores type/testID/text and frees on remove" {
    const gpa = std.testing.allocator;
    var t = Tree.initBare(gpa);
    defer t.deinitMeta();
    try t.putMeta(1, "Button", "increment-button", "Increment", 0, .{});
    const m = t.metaGet(1).?;
    try std.testing.expectEqualStrings("Button", m.widget_type);
    try std.testing.expectEqualStrings("increment-button", m.test_id.?);
    try std.testing.expectEqualStrings("Increment", m.text.?);
    t.removeMeta(1);
    try std.testing.expect(t.metaGet(1) == null);
}
