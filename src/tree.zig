const std = @import("std");
const protocol = @import("protocol.zig");
const backend = @import("backend.zig").impl;

const Widget = backend.Widget;

/// Host-side node metadata: per-node type/testID/text/parent, independent of
/// the live widget handle. Populated in `apply` alongside `nodes.put`; this is
/// the snapshot source of truth for the automation `getTree` RPC (M4) — it is
/// never re-derived from props at query time.
pub const NodeMeta = struct {
    widget_type: []u8,
    test_id: ?[]u8,
    text: ?[]u8,
    parent: u32,
    attached: protocol.Attached = .{}, // tab_label/slot duped/owned here
    /// ListView's row count (M5c-D4): getTree reports this instead of
    /// dumping the recycled row widgets. Null for every non-data-driven widget.
    item_count: ?u32 = null,
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

/// ListView's `itemCount` (M5c-D4) is derived from `items.len`, never from
/// walking GTK's recycled row widgets.
fn propArrayLen(props: ?std.json.Value, key: []const u8) ?u32 {
    const v = props orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .array => @intCast(field.array.items.len),
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

/// Reserved generation for host-created overlay chrome (M8-D5): never swept
/// by gcOldGenerations (an incoming app generation reaches 0xFF only after
/// 255 hot-reloads — reserved in practice). NOTE: ids are
/// `(generation << 24) | (seq & 0xFFFFFF)` (see packages/react/src/ids.ts),
/// an 8-bit generation field — 0xFFFF (16 bits) would silently truncate to
/// 0xFF00 through `u32` shift overflow, corrupting the reserved-generation
/// check (caught this session: `gcOldGenerations`/`clearAppNodes` never
/// matched overlay-tagged ids with a wider constant).
pub const OVERLAY_GENERATION: u32 = 0xFF;

/// Node ids are (generation << 24) | (seq & 0xFFFFFF) — see packages/react/src/ids.ts.
fn genOf(id: u32) u32 {
    return id >> 24;
}

pub const Tree = struct {
    gpa: std.mem.Allocator,
    // Opaque embedder-app handle (GtkApplication* / NSApplication instance);
    // the core never dereferences it — it only threads through to
    // `backend.createWidget`'s first (embedder-defined) argument.
    app: ?*anyopaque,
    nodes: std.AutoHashMapUnmanaged(u32, *Widget) = .{},
    meta: std.AutoHashMapUnmanaged(u32, NodeMeta) = .{},
    generation: u32 = 0,
    // Ordered per-parent sibling lists (Task 3): `NodeMeta.parent` alone
    // cannot answer "in what order" — hashmap iteration order is bucket
    // layout, not insertion order. Maintained by the `append`/`insertBefore`/
    // `remove` op handlers in `apply`; this is the sole ordering source for
    // `getTree` (see automation.zig's `handleGetTree`).
    children: std.AutoHashMapUnmanaged(u32, std.ArrayList(u32)) = .{},

    pub fn init(gpa: std.mem.Allocator, app: *anyopaque) Tree {
        return .{ .gpa = gpa, .app = app };
    }

    /// A ctor that skips the embedder app handle for pure-meta unit tests
    /// (no real embedder app needed to exercise the meta map in isolation).
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
        owned_attached.slot = try self.dupeOpt(attached.slot);
        errdefer if (owned_attached.slot) |v| self.gpa.free(v);
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

    /// Appends `child` to `parent`'s ordered sibling list. Detaches it from
    /// its current parent first (read from meta, which still holds the OLD
    /// parent at this point) — `append`/`appendChild` of an already-mounted
    /// child is a move-to-end, not a duplicate (mirrors the null backend's
    /// own move semantics, commits 2ba8701/a20b925/465106f).
    pub fn recordAppend(self: *Tree, parent: u32, child: u32) void {
        self.detachFromParent(child);
        const gop = self.children.getOrPut(self.gpa, parent) catch return;
        if (!gop.found_existing) gop.value_ptr.* = .empty;
        gop.value_ptr.append(self.gpa, child) catch {};
    }

    /// Inserts `child` into `parent`'s ordered sibling list immediately
    /// before `before` (append-to-end if `before` isn't found). Same
    /// detach-first move semantics as `recordAppend`.
    pub fn recordInsertBefore(self: *Tree, parent: u32, child: u32, before: u32) void {
        self.detachFromParent(child);
        const gop = self.children.getOrPut(self.gpa, parent) catch return;
        if (!gop.found_existing) gop.value_ptr.* = .empty;
        const list = gop.value_ptr;
        var idx: usize = list.items.len;
        for (list.items, 0..) |c, i| if (c == before) {
            idx = i;
            break;
        };
        list.insert(self.gpa, idx, child) catch {};
    }

    /// Removes `child` from `parent`'s ordered sibling list.
    pub fn recordRemove(self: *Tree, parent: u32, child: u32) void {
        const list = self.children.getPtr(parent) orelse return;
        for (list.items, 0..) |c, i| if (c == child) {
            _ = list.orderedRemove(i);
            return;
        };
    }

    /// Drops `child` from its CURRENT parent's ordered list, read from meta
    /// (called before `setMetaParent` overwrites it — see `apply`'s
    /// append/insertBefore arms).
    fn detachFromParent(self: *Tree, child: u32) void {
        const m = self.meta.getPtr(child) orelse return;
        if (m.parent == 0) return;
        self.recordRemove(m.parent, child);
    }

    /// The ordered child ids of `id`, or an empty slice if none (never
    /// tracked, or tracked with zero children).
    pub fn childrenOf(self: *Tree, id: u32) []const u32 {
        const list = self.children.getPtr(id) orelse return &.{};
        return list.items;
    }

    pub fn deinitChildren(self: *Tree) void {
        var it = self.children.iterator();
        while (it.next()) |entry| entry.value_ptr.deinit(self.gpa);
        self.children.deinit(self.gpa);
    }

    pub fn setMetaItemCount(self: *Tree, id: u32, n: u32) void {
        if (self.meta.getPtr(id)) |m| m.item_count = n;
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
        if (kv.value.attached.slot) |v| self.gpa.free(v);
    }

    pub fn deinitMeta(self: *Tree) void {
        var it = self.meta.iterator();
        while (it.next()) |entry| {
            self.gpa.free(entry.value_ptr.widget_type);
            if (entry.value_ptr.test_id) |v| self.gpa.free(v);
            if (entry.value_ptr.text) |v| self.gpa.free(v);
            if (entry.value_ptr.attached.tab_label) |v| self.gpa.free(v);
            if (entry.value_ptr.attached.slot) |v| self.gpa.free(v);
        }
        self.meta.deinit(self.gpa);
    }

    /// UI-thread only. Applies an entire commit batch as one unit.
    pub fn apply(self: *Tree, batch: protocol.CommitBatch) void {
        const previous_gen = self.generation;
        self.generation = batch.generation;
        for (batch.ops) |op| {
            if (std.mem.eql(u8, op.op, "create")) {
                const app = self.app orelse continue;
                // M8-D9a: a post-crash respawn mounts a brand-new reconciler
                // root, whose first commit re-emits a `create Window` op —
                // but the generated Window arm unconditionally constructs a
                // new native window. Bind to the surviving window widget
                // instead of opening a second OS window; this is the only
                // Window `create` this branch ever sees post-crash (a live
                // --hot edit never re-creates the window at all, since the
                // reconciler root/container survive — see hmr.ts's
                // globalThis singleton guard).
                const widget = if (std.mem.eql(u8, op.widget.?, "Window") and backend.getWindow() != null)
                    backend.getWindow().?
                else
                    backend.createWidget(app, op.widget.?, op.props) catch continue;
                backend.connectEvents(widget, op.widget.?, op.id.?);
                self.nodes.put(self.gpa, op.id.?, widget) catch continue;
                // testID is stored here for the automation getTree RPC (M4) and is
                // never applied to the GTK widget itself.
                const test_id = propStr(op.props, "testID");
                const initial_text = propStr(op.props, "text") orelse propStr(op.props, "label");
                const attached = protocol.Attached.fromProps(op.props);
                self.putMeta(op.id.?, op.widget.?, test_id, initial_text, 0, attached) catch {};
                if (propArrayLen(op.props, "items")) |n| self.setMetaItemCount(op.id.?, n);
                applyStyleIfPresent(widget, op.id.?, op.props);
            } else if (std.mem.eql(u8, op.op, "append")) {
                const parent_widget = self.nodes.get(op.parent.?) orelse continue;
                const child_widget = self.nodes.get(op.child.?) orelse continue;
                const pmeta = self.metaGet(op.parent.?) orelse continue;
                const cmeta = self.metaGet(op.child.?) orelse continue;
                backend.appendChild(parent_widget, pmeta.widget_type, child_widget, cmeta.attached);
                self.recordAppend(op.parent.?, op.child.?);
                self.setMetaParent(op.child.?, op.parent.?);
            } else if (std.mem.eql(u8, op.op, "setText")) {
                const widget = self.nodes.get(op.id.?) orelse continue;
                backend.setText(widget, op.text.?);
                self.setMetaText(op.id.?, op.text.?);
            } else if (std.mem.eql(u8, op.op, "update")) {
                const widget = self.nodes.get(op.id.?) orelse continue;
                // React's host-config `commitUpdate` never sends the widget-kind
                // field on update ops (only `create` ops carry it) — resolve the
                // kind from the retained tree's own create-time record instead of
                // `op.widget` (always null on a real update), or every kind-
                // dispatched prop applier would silently no-op on every update.
                const kind = if (self.metaGet(op.id.?)) |m| m.widget_type else "";
                backend.applyProps(widget, kind, op.props);
                applyStyleIfPresent(widget, op.id.?, op.props);
                if (propStr(op.props, "testID")) |t| self.setMetaTestId(op.id.?, t);
                if (propStr(op.props, "text") orelse propStr(op.props, "label")) |t| {
                    self.setMetaText(op.id.?, t);
                }
                if (propArrayLen(op.props, "items")) |n| self.setMetaItemCount(op.id.?, n);
            } else if (std.mem.eql(u8, op.op, "insertBefore")) {
                const parent_widget = self.nodes.get(op.parent.?) orelse continue;
                const child_widget = self.nodes.get(op.child.?) orelse continue;
                const before: ?*Widget = if (op.before) |b| self.nodes.get(b) else null;
                const pmeta = self.metaGet(op.parent.?) orelse continue;
                const cmeta = self.metaGet(op.child.?) orelse continue;
                backend.insertBefore(parent_widget, pmeta.widget_type, child_widget, before, cmeta.attached);
                if (op.before) |b| self.recordInsertBefore(op.parent.?, op.child.?, b) else self.recordAppend(op.parent.?, op.child.?);
                self.setMetaParent(op.child.?, op.parent.?);
            } else if (std.mem.eql(u8, op.op, "remove")) {
                const child = self.nodes.get(op.id.?) orelse continue;
                // Portable parent lookup (Task 3): the meta map already tracks
                // each child's parent id (set by append/insertBefore), so the
                // live parent widget comes from `self.nodes`, not a backend
                // "get live GTK parent" call. `backend.hasParent` still guards
                // against double-remove (mirrors gcOldGenerations/clearAppNodes).
                if (backend.hasParent(child)) {
                    if (self.metaGet(op.id.?)) |cmeta| {
                        if (self.nodes.get(cmeta.parent)) |parent| {
                            const parent_kind = if (self.metaGet(cmeta.parent)) |pmeta| pmeta.widget_type else "";
                            backend.removeChild(parent, parent_kind, child);
                        }
                    }
                }
                if (self.metaGet(op.id.?)) |cmeta| self.recordRemove(cmeta.parent, op.id.?);
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
        // M8-D9: a higher-generation CommitBatch means a hot reload landed a
        // fresh tree — sweep the previous generation's orphaned widgets
        // *after* applying the new ops (so the new generation's widgets
        // exist before the old ones are removed, avoiding a blank frame).
        // Skipped when `previous_gen == OVERLAY_GENERATION` (never true for
        // a real app commit) and on the steady state (batch.generation ==
        // previous_gen, the common case — zero cost, byte-identical to M5c).
        if (batch.generation > previous_gen and previous_gen != OVERLAY_GENERATION) {
            self.gcOldGenerations(batch.generation);
        }
        std.debug.print("ND_COMMIT_APPLIED commitId={d}\n", .{batch.commitId});
    }

    /// Sweeps every tracked node whose id-encoded generation is strictly less
    /// than `new_gen`, except the reserved overlay generation and the sole
    /// Window node (kept per M8-D9a — the host reuses the existing native
    /// window widget, replacing only its content, rather than opening a
    /// second OS window). Collect-then-remove: never mutate `nodes`/`meta`
    /// while iterating them.
    fn gcOldGenerations(self: *Tree, new_gen: u32) void {
        var doomed: std.ArrayList(u32) = .empty;
        defer doomed.deinit(self.gpa);
        var doomed_set: std.AutoHashMapUnmanaged(u32, void) = .empty;
        defer doomed_set.deinit(self.gpa);
        var it = self.meta.iterator();
        while (it.next()) |entry| {
            const id = entry.key_ptr.*;
            if (genOf(id) >= new_gen or genOf(id) == OVERLAY_GENERATION) continue;
            if (std.mem.eql(u8, entry.value_ptr.widget_type, "Window")) continue; // keep the window (M8-D9a)
            doomed.append(self.gpa, id) catch continue;
            doomed_set.put(self.gpa, id, {}) catch {};
        }
        var swept: u32 = 0;
        for (doomed.items) |id| {
            // Only unparent a "root" of the swept subtree (see clearAppNodes
            // for the full rationale) — unparenting an interior widget
            // cascades GTK's own destruction of its children, so touching
            // those children afterward would be a use-after-free.
            const parent_also_doomed = if (self.metaGet(id)) |m| doomed_set.contains(m.parent) else false;
            if (!parent_also_doomed) {
                if (self.nodes.get(id)) |w| {
                    if (backend.hasParent(w)) backend.unparentWidget(w);
                }
                // Only the doomed subtree's own root needs unlinking from
                // its (surviving) parent's ordered list — a doomed interior
                // node's entry lives in another doomed node's list, which is
                // dropped wholesale below.
                if (self.metaGet(id)) |m| self.recordRemove(m.parent, id);
            }
            _ = self.nodes.remove(id);
            self.removeMeta(id);
            if (self.children.getPtr(id)) |list| list.deinit(self.gpa);
            _ = self.children.remove(id);
            swept += 1;
        }
        std.debug.print("ND_GC_SWEEP gen={d} removed={d}\n", .{ new_gen, swept });
    }

    /// Clears every non-overlay node's bookkeeping (M8 dev-mode Restart):
    /// a respawned child mounts a brand-new reconciler root at generation 0,
    /// which collides with the dead child's stale gen-0 ids, so the dead
    /// tree's entries must be dropped before the fresh mount rebuilds. The
    /// Window widget itself is NOT unparented/destroyed here — `apply`'s
    /// create-op arm rebinds the next Window `create` op to the surviving
    /// widget by identity (M8-D9a); only its now-stale meta entry is
    /// dropped here, same as every other non-overlay node.
    pub fn clearAppNodes(self: *Tree) void {
        var doomed: std.ArrayList(u32) = .empty;
        defer doomed.deinit(self.gpa);
        var doomed_set: std.AutoHashMapUnmanaged(u32, void) = .empty;
        defer doomed_set.deinit(self.gpa);
        var it = self.meta.iterator();
        while (it.next()) |entry| {
            const id = entry.key_ptr.*;
            if (genOf(id) == OVERLAY_GENERATION) continue;
            doomed.append(self.gpa, id) catch continue;
            doomed_set.put(self.gpa, id, {}) catch {};
        }
        for (doomed.items) |id| {
            const m = self.metaGet(id) orelse continue;
            const is_window = std.mem.eql(u8, m.widget_type, "Window");
            // Only unparent a "root" of the doomed subtree — a node whose
            // parent is NOT itself being cleared this pass. Unparenting an
            // interior node (e.g. a Box) destroys it and, via GTK's own
            // container teardown, its children too; unparenting those
            // children afterward would be a use-after-free (verified this
            // session — the identical bug the overlay's `clear()` hit).
            const parent_also_doomed = doomed_set.contains(m.parent);
            if (!is_window and !parent_also_doomed) {
                if (self.nodes.get(id)) |w| {
                    if (backend.hasParent(w)) backend.unparentWidget(w);
                }
                // Unlink from a surviving parent's (window/overlay) ordered
                // list — a doomed interior node's entry lives in another
                // doomed node's own list, dropped wholesale below.
                self.recordRemove(m.parent, id);
            }
            _ = self.nodes.remove(id);
            self.removeMeta(id);
            if (self.children.getPtr(id)) |list| list.deinit(self.gpa);
            _ = self.children.remove(id);
        }
        self.generation = 0;
        std.debug.print("ND_CLEAR_APP_NODES removed={d}\n", .{doomed.items.len});
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

test "childrenOf preserves append + insertBefore order" {
    const gpa = std.testing.allocator;
    var tree = Tree.initBare(gpa);
    defer tree.deinitMeta();
    defer tree.deinitChildren();

    // Parent p=1, children appended 10, 20, 30.
    try tree.putMeta(1, "Box", null, null, 0, .{});
    try tree.putMeta(10, "Label", null, null, 0, .{});
    try tree.putMeta(20, "Label", null, null, 0, .{});
    try tree.putMeta(30, "Label", null, null, 0, .{});
    tree.recordAppend(1, 10);
    tree.recordAppend(1, 20);
    tree.recordAppend(1, 30);
    try std.testing.expectEqualSlices(u32, &.{ 10, 20, 30 }, tree.childrenOf(1));

    // insertBefore 30 -> new child 25 lands between 20 and 30.
    try tree.putMeta(25, "Label", null, null, 0, .{});
    tree.recordInsertBefore(1, 25, 30);
    try std.testing.expectEqualSlices(u32, &.{ 10, 20, 25, 30 }, tree.childrenOf(1));

    // remove 20 -> order compacts.
    tree.recordRemove(1, 20);
    try std.testing.expectEqualSlices(u32, &.{ 10, 25, 30 }, tree.childrenOf(1));
}

test "generation bump sweeps old-generation nodes, keeps window and overlay" {
    const gpa = std.testing.allocator;
    var t = Tree.initBare(gpa);
    defer t.deinitMeta();
    // gen 0: window(0x000001), box(0x000002), label(0x000003).
    try t.putMeta(0x000001, "Window", null, null, 0, .{});
    try t.putMeta(0x000002, "Box", null, null, 0x000001, .{});
    try t.putMeta(0x000003, "Label", null, "old", 0x000002, .{});
    // The reserved overlay generation must survive any sweep too.
    const overlay_id: u32 = (OVERLAY_GENERATION << 24) | 1;
    try t.putMeta(overlay_id, "Label", "nd-overlay-error", "boom", 0, .{});

    t.gcOldGenerations(1); // simulate a gen-1 CommitBatch having just landed
    try std.testing.expect(t.metaGet(0x000002) == null); // box swept
    try std.testing.expect(t.metaGet(0x000003) == null); // label swept
    try std.testing.expect(t.metaGet(0x000001) != null); // window kept
    try std.testing.expect(t.metaGet(overlay_id) != null); // overlay kept
}
