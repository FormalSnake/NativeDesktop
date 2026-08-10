const std = @import("std");
const protocol = @import("protocol.zig");
const backend = @import("backend.zig").impl;

const Widget = backend.Widget;

/// Host-side node metadata: per-node type/testID/text/parent, independent of
/// the live widget handle. Populated in `apply` alongside `nodes.put`; this is
/// the snapshot source of truth for the automation `getTree` RPC — it is
/// never re-derived from props at query time.
pub const NodeMeta = struct {
    widget_type: []u8,
    /// NativeView factory key retained because update/command/remove ops omit props.
    view_kind: ?[]u8 = null,
    test_id: ?[]u8,
    text: ?[]u8,
    parent: u32,
    attached: protocol.Attached = .{}, // tab_label/slot duped/owned here
    /// ListView's row count: getTree reports this instead of
    /// dumping the recycled row widgets. Null for every non-data-driven widget.
    item_count: ?u32 = null,
    /// SourceList's row data: getTree reports these
    /// title/badge/iconName triples directly, mirroring `item_count` — never
    /// re-derived by walking the live AdwActionRow widgets. Null for every
    /// widget that isn't row-driven.
    rows: ?[]Row = null,
    /// Window's create-only tabGroup prop, retained for the automation
    /// `windows` RPC. Null for plain windows and every non-Window widget.
    tab_group: ?[]u8 = null,

    pub const Row = struct { title: []u8, badge: ?[]u8, icon_name: ?[]u8 };
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

/// ListView's `itemCount` is derived from `items.len`, never from
/// walking GTK's recycled row widgets. The same derivation covers
/// Table (`rows`) and TreeView (`nodes`) — see `rowCountOf`.
fn propArrayLen(props: ?std.json.Value, key: []const u8) ?u32 {
    const v = props orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .array => @intCast(field.array.items.len),
        else => null,
    };
}

/// Parses SourceList's `items` (an `objectList` of `{title, badge?,
/// iconName?}`) into heap-owned `NodeMeta.Row`s for `NodeMeta.rows`. Every
/// string is duped into `gpa` — the source `std.json.Value` tree is
/// transient (freed with the CommitBatch's parse arena once `apply`
/// returns). Returns null if `items` is absent or not an array; an
/// individual malformed row (missing/non-string `title`) is skipped, not
/// fatal to the rest of the list.
fn parseRows(gpa: std.mem.Allocator, props: ?std.json.Value) ?[]NodeMeta.Row {
    const v = props orelse return null;
    if (v != .object) return null;
    const field = v.object.get("items") orelse return null;
    if (field != .array) return null;
    var out: std.ArrayList(NodeMeta.Row) = .empty;
    for (field.array.items) |it| {
        if (it != .object) continue;
        const title_v = it.object.get("title") orelse continue;
        if (title_v != .string) continue;
        const title = gpa.dupe(u8, title_v.string) catch continue;
        var badge: ?[]u8 = null;
        if (it.object.get("badge")) |b| {
            if (b == .string) badge = gpa.dupe(u8, b.string) catch null;
        }
        var icon_name: ?[]u8 = null;
        if (it.object.get("iconName")) |ic| {
            if (ic == .string) icon_name = gpa.dupe(u8, ic.string) catch null;
        }
        out.append(gpa, .{ .title = title, .badge = badge, .icon_name = icon_name }) catch {
            gpa.free(title);
            if (badge) |b| gpa.free(b);
            if (icon_name) |i| gpa.free(i);
        };
    }
    return out.toOwnedSlice(gpa) catch null;
}

/// Frees a `NodeMeta.rows` slice (each row's owned strings, then the slice
/// itself) — mirrors the title/badge/icon_name ownership shape `removeMeta`/
/// `deinitMeta`/`setMetaRows` all share.
fn freeRows(gpa: std.mem.Allocator, rows: ?[]NodeMeta.Row) void {
    const r = rows orelse return;
    for (r) |row| {
        gpa.free(row.title);
        if (row.badge) |b| gpa.free(b);
        if (row.icon_name) |i| gpa.free(i);
    }
    gpa.free(r);
}

/// The data-driven widgets' row count for `NodeMeta.item_count`, whichever
/// objectList/stringList prop the widget carries: ListView `items`,
/// Table `rows`, TreeView `nodes`. No widget declares more than one.
fn rowCountOf(props: ?std.json.Value) ?u32 {
    return propArrayLen(props, "items") orelse
        propArrayLen(props, "rows") orelse
        propArrayLen(props, "nodes");
}

/// Invoked from both the create and update arms of `apply` — style is only
/// compiled when `props.style` is present (regression contract: no style
/// means zero CSS provider and zero margin call).
fn applyStyleIfPresent(widget: *Widget, id: u32, props: ?std.json.Value) void {
    const v = props orelse return;
    if (v != .object) return;
    if (v.object.get("style")) |st| backend.applyStyle(widget, id, st);
}

/// Reserved generation for host-created overlay chrome: never swept
/// by gcOldGenerations (an incoming app generation reaches 0xFF only after
/// 255 hot-reloads — reserved in practice). NOTE: ids are
/// `(generation << 24) | (seq & 0xFFFFFF)` (see packages/react/src/ids.ts),
/// an 8-bit generation field — 0xFFFF (16 bits) would silently truncate to
/// 0xFF00 through `u32` shift overflow, corrupting the reserved-generation
/// check (`gcOldGenerations`/`clearAppNodes` would never match
/// overlay-tagged ids with a wider constant).
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
    // Ordered per-parent sibling lists: `NodeMeta.parent` alone
    // cannot answer "in what order" — hashmap iteration order is bucket
    // layout, not insertion order. Maintained by the `append`/`insertBefore`/
    // `remove` op handlers in `apply`; this is the sole ordering source for
    // `getTree` (see automation.zig's `handleGetTree`).
    children: std.AutoHashMapUnmanaged(u32, std.ArrayList(u32)) = .{},
    // Native window handles orphaned by `clearAppNodes` (a crash / dev-mode
    // Restart respawn), keyed by the Window node id that owned them. The
    // respawned tree re-emits `create Window` with the SAME ids (a
    // deterministic render replays ids in order), so each rebinds to the
    // window it left open instead of opening a duplicate OS window.
    // Empty in steady state — so a
    // genuinely new `<window>` in a live tree always opens a fresh window.
    window_reuse: std.AutoHashMapUnmanaged(u32, *Widget) = .{},
    // Node ids whose stored handle carries NO backend ownership reference:
    // an AppKit window rebind stores the hierarchy-owned live content view
    // (resolve_window returns a different pointer there), not the
    // create-time +1 handle, so `releaseHandle` must skip these — releasing
    // would over-release a view the shell never handed us. GTK rebinds
    // return the handle unchanged and never land here.
    unowned: std.AutoHashMapUnmanaged(u32, void) = .{},

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

    /// The first Window node id (getTree's root), or null if none registered
    /// yet. With multiple `<window>` roots this returns whichever window the
    /// meta map yields first; per-window automation targeting is future work.
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

    /// Appends `root` and every descendant (depth-first) to `out`. React emits
    /// exactly one `remove` op for a removed subtree's root but creates the
    /// subtree recursively (`emitCreateIfNew`), so the host must walk the
    /// tracked child lists itself to purge the whole subtree's bookkeeping.
    fn collectSubtree(self: *Tree, root: u32, out: *std.ArrayList(u32)) void {
        out.append(self.gpa, root) catch return;
        if (self.children.getPtr(root)) |list| {
            for (list.items) |c| self.collectSubtree(c, out);
        }
    }

    pub fn deinitChildren(self: *Tree) void {
        var it = self.children.iterator();
        while (it.next()) |entry| entry.value_ptr.deinit(self.gpa);
        self.children.deinit(self.gpa);
    }

    pub fn setMetaItemCount(self: *Tree, id: u32, n: u32) void {
        if (self.meta.getPtr(id)) |m| m.item_count = n;
    }

    /// Replaces `id`'s SourceList row data, freeing the previously stored
    /// rows first (mirrors `setMetaText`'s free-then-replace contract).
    /// Takes ownership of `rows` — frees it instead if `id` has no meta
    /// entry (shouldn't happen in practice, but avoids a leak either way).
    pub fn setMetaRows(self: *Tree, id: u32, rows: []NodeMeta.Row) void {
        const m = self.meta.getPtr(id) orelse {
            freeRows(self.gpa, rows);
            return;
        };
        freeRows(self.gpa, m.rows);
        m.rows = rows;
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
        if (kv.value.view_kind) |v| self.gpa.free(v);
        if (kv.value.test_id) |v| self.gpa.free(v);
        if (kv.value.text) |v| self.gpa.free(v);
        if (kv.value.attached.tab_label) |v| self.gpa.free(v);
        if (kv.value.attached.slot) |v| self.gpa.free(v);
        freeRows(self.gpa, kv.value.rows);
        if (kv.value.tab_group) |v| self.gpa.free(v);
    }

    pub fn deinitMeta(self: *Tree) void {
        var it = self.meta.iterator();
        while (it.next()) |entry| {
            self.gpa.free(entry.value_ptr.widget_type);
            if (entry.value_ptr.view_kind) |v| self.gpa.free(v);
            if (entry.value_ptr.test_id) |v| self.gpa.free(v);
            if (entry.value_ptr.text) |v| self.gpa.free(v);
            if (entry.value_ptr.attached.tab_label) |v| self.gpa.free(v);
            if (entry.value_ptr.attached.slot) |v| self.gpa.free(v);
            freeRows(self.gpa, entry.value_ptr.rows);
            if (entry.value_ptr.tab_group) |v| self.gpa.free(v);
        }
        self.meta.deinit(self.gpa);
    }

    /// Drops the backend's ownership reference for a node id leaving `nodes`
    /// — the single call-site wrapper around the `release_node` vtable op.
    /// Skips ids marked `unowned` (AppKit window rebinds; see the field doc).
    fn releaseHandle(self: *Tree, id: u32, widget: *Widget) void {
        if (self.unowned.remove(id)) return;
        backend.releaseNode(widget);
    }

    /// UI-thread only. Applies an entire commit batch as one unit.
    pub fn apply(self: *Tree, batch: protocol.CommitBatch) void {
        const previous_gen = self.generation;
        self.generation = batch.generation;
        for (batch.ops) |op| {
            if (std.mem.eql(u8, op.op, "create")) {
                const app = self.app orelse continue;
                // A post-crash / dev-mode Restart respawn mounts a
                // brand-new reconciler root, whose first commit re-emits a
                // `create Window` op for every window — but the generated
                // Window arm unconditionally constructs a new native window.
                // Rebind each re-emitted window (matched by node id, which a
                // deterministic replay reproduces) to the native window it left
                // orphaned in `window_reuse` rather than opening a duplicate OS
                // window. A live --hot edit never re-creates a window at all
                // (the reconciler root/container survive — see hmr.ts's
                // globalThis singleton guard), and the pool is empty in steady
                // state, so a genuinely new `<window>` falls through to
                // `createWidget` and opens a fresh window — this is what lets
                // one tree present N independent OS windows.
                const widget = if (std.mem.eql(u8, op.widget.?, "Window")) blk: {
                    if (self.window_reuse.fetchRemove(op.id.?)) |kv| {
                        const resolved = backend.resolveWindow(kv.value);
                        // AppKit resolves to the hierarchy-owned live content
                        // view (a different pointer): the node's create-time
                        // +1 stays parked on the ORIGINAL handle, so the
                        // rebound one must never be released when this id
                        // drops. GTK returns the handle unchanged.
                        if (resolved != kv.value) self.unowned.put(self.gpa, op.id.?, {}) catch {};
                        break :blk resolved;
                    }
                    break :blk backend.createWidget(app, op.widget.?, op.props) catch continue;
                } else backend.createWidget(app, op.widget.?, op.props) catch continue;
                backend.connectEvents(widget, op.widget.?, op.id.?);
                // viewKind only means anything on a NativeView — never route
                // another kind's widget into the plugin because a stray
                // client put a viewKind prop on it.
                const view_kind: ?[]const u8 = if (std.mem.eql(u8, op.widget.?, "NativeView")) propStr(op.props, "viewKind") else null;
                if (view_kind) |vk| backend.nativeViewConnect(vk, widget, op.id.?);
                self.nodes.put(self.gpa, op.id.?, widget) catch continue;
                // testID is stored here for the automation getTree RPC and is
                // never applied to the GTK widget itself.
                const test_id = propStr(op.props, "testID");
                const initial_text = propStr(op.props, "text") orelse propStr(op.props, "label");
                const attached = protocol.Attached.fromProps(op.props);
                self.putMeta(op.id.?, op.widget.?, test_id, initial_text, 0, attached) catch {};
                if (view_kind != null) if (self.metaGet(op.id.?)) |m| {
                    m.view_kind = self.dupeOpt(view_kind) catch null;
                };
                if (propStr(op.props, "tabGroup")) |tg| if (self.metaGet(op.id.?)) |m| {
                    m.tab_group = self.dupeOpt(tg) catch null;
                };
                if (rowCountOf(op.props)) |n| self.setMetaItemCount(op.id.?, n);
                if (parseRows(self.gpa, op.props)) |rows| self.setMetaRows(op.id.?, rows);
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
                const meta = self.metaGet(op.id.?);
                const kind = if (meta) |m| m.widget_type else "";
                // A NativeView's nested plugin props route straight to the
                // plugin manager via the retained view_kind (update ops never
                // carry viewKind). Host-level props (cssClasses/testID) still
                // cross the backend's ordinary apply path below — the GTK
                // backend skips its generated NativeView forwarding so the
                // plugin isn't applied twice.
                if (meta) |m| {
                    if (m.view_kind) |view_kind| {
                        if (propStr(op.props, "props")) |props_json| backend.nativeViewApplyProps(view_kind, widget, props_json);
                    }
                }
                backend.applyProps(widget, kind, op.props);
                applyStyleIfPresent(widget, op.id.?, op.props);
                if (propStr(op.props, "testID")) |t| self.setMetaTestId(op.id.?, t);
                if (propStr(op.props, "text") orelse propStr(op.props, "label")) |t| {
                    self.setMetaText(op.id.?, t);
                }
                if (rowCountOf(op.props)) |n| self.setMetaItemCount(op.id.?, n);
                if (parseRows(self.gpa, op.props)) |rows| self.setMetaRows(op.id.?, rows);
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
                const id = op.id.?;
                const child = self.nodes.get(id) orelse continue;
                const cmeta = self.metaGet(id);
                // React emits ONE remove for the subtree root; unparenting it
                // cascade-destroys every descendant widget, so their bookkeeping
                // must be purged here too — otherwise stale (id -> freed-widget)
                // entries linger, alias later-recycled addresses, and crash
                // clearAppNodes/getTree with a use-after-free.
                var subtree: std.ArrayList(u32) = .empty;
                defer subtree.deinit(self.gpa);
                self.collectSubtree(id, &subtree);
                // Destroy plugin views across the whole subtree while every
                // widget is still alive — removeChild below finalizes them (the
                // container held the only ref; the host takes none of its own on
                // plugin views), so destroying after would hand a freed widget.
                for (subtree.items) |sid| {
                    const m = self.metaGet(sid) orelse continue;
                    if (m.view_kind) |view_kind| if (self.nodes.get(sid)) |w| backend.nativeViewDestroy(view_kind, w);
                }
                // A Window root has no tree parent to detach from — its
                // native teardown is a window/tab close instead ("window.close"
                // semantic action, M17). The backend no-ops on windows the
                // user already closed (the `closed`-event -> unmount path), so
                // remove stays idempotent with native close.
                const is_window_root = if (cmeta) |m| std.mem.eql(u8, m.widget_type, "Window") else false;
                if (is_window_root) {
                    backend.closeWindow(child, id);
                } else if (backend.hasParent(child)) {
                    // Portable parent lookup: the meta map already tracks
                    // each child's parent id (set by append/insertBefore), so the
                    // live parent widget comes from `self.nodes`, not a backend
                    // "get live GTK parent" call. `backend.hasParent` still guards
                    // against double-remove (mirrors gcOldGenerations/clearAppNodes).
                    if (cmeta) |m| {
                        if (self.nodes.get(m.parent)) |parent| {
                            const parent_kind = if (self.metaGet(m.parent)) |pmeta| pmeta.widget_type else "";
                            backend.removeChild(parent, parent_kind, child);
                        }
                    }
                }
                if (cmeta) |m| self.recordRemove(m.parent, id);
                // Purge bookkeeping for the entire doomed subtree, dropping
                // the core's ownership ref on each handle (release_node) —
                // the create-time ref is what kept a container-cascaded
                // widget's OBJECT alive for `getTree` probes; releasing here
                // lets it actually finalize.
                for (subtree.items) |sid| {
                    if (self.nodes.fetchRemove(sid)) |kv| self.releaseHandle(sid, kv.value);
                    self.removeMeta(sid);
                    if (self.children.getPtr(sid)) |list| list.deinit(self.gpa);
                    _ = self.children.remove(sid);
                }
                std.debug.print("ND_REMOVE id={d}\n", .{id});
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
        // A higher-generation CommitBatch means a hot reload landed a
        // fresh tree — sweep the previous generation's orphaned widgets
        // *after* applying the new ops (so the new generation's widgets
        // exist before the old ones are removed, avoiding a blank frame).
        // Skipped when `previous_gen == OVERLAY_GENERATION` (never true for
        // a real app commit) and on the steady state (batch.generation ==
        // previous_gen, the common case — zero cost).
        if (batch.generation > previous_gen and previous_gen != OVERLAY_GENERATION) {
            self.gcOldGenerations(batch.generation);
        }
        std.debug.print("ND_COMMIT_APPLIED commitId={d}\n", .{batch.commitId});
    }

    /// Sweeps every tracked node whose id-encoded generation is strictly less
    /// than `new_gen`, except the reserved overlay generation and the sole
    /// Window node (kept because the host reuses the existing native
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
            if (std.mem.eql(u8, entry.value_ptr.widget_type, "Window")) continue; // keep the window
            doomed.append(self.gpa, id) catch continue;
            doomed_set.put(self.gpa, id, {}) catch {};
        }
        // Destroy plugin-created native views first, while every doomed
        // widget is still alive: unparenting a doomed subtree root below
        // cascade-destroys its interior children (GTK), and the host holds
        // no ref of its own on plugin views.
        for (doomed.items) |id| {
            const m = self.metaGet(id) orelse continue;
            if (m.view_kind) |view_kind| if (self.nodes.get(id)) |w| backend.nativeViewDestroy(view_kind, w);
        }
        var swept: u32 = 0;
        for (doomed.items) |id| {
            if (self.metaGet(id)) |m| {
                // Only unparent a "root" of the swept subtree (see clearAppNodes
                // for the full rationale) — unparenting an interior widget
                // cascades GTK's own destruction of its children, so touching
                // those children afterward would be a use-after-free.
                if (!doomed_set.contains(m.parent)) {
                    if (self.nodes.get(id)) |w| {
                        if (backend.hasParent(w)) backend.unparentWidget(w);
                    }
                    // Only the doomed subtree's own root needs unlinking from
                    // its (surviving) parent's ordered list — a doomed interior
                    // node's entry lives in another doomed node's list, which is
                    // dropped wholesale below.
                    self.recordRemove(m.parent, id);
                }
            }
            if (self.nodes.fetchRemove(id)) |kv| self.releaseHandle(id, kv.value);
            self.removeMeta(id);
            if (self.children.getPtr(id)) |list| list.deinit(self.gpa);
            _ = self.children.remove(id);
            swept += 1;
        }
        std.debug.print("ND_GC_SWEEP gen={d} removed={d}\n", .{ new_gen, swept });
    }

    /// Clears every non-overlay node's bookkeeping (dev-mode Restart):
    /// a respawned child mounts a brand-new reconciler root at generation 0,
    /// which collides with the dead child's stale gen-0 ids, so the dead
    /// tree's entries must be dropped before the fresh mount rebuilds. Each
    /// Window widget itself is NOT unparented/destroyed here — it's stashed in
    /// `window_reuse` keyed by its node id so `apply`'s create-op arm rebinds
    /// the respawned tree's matching `create Window` to it; only its
    /// now-stale meta entry is dropped here, same as every other non-overlay
    /// node.
    pub fn clearAppNodes(self: *Tree) void {
        // Drop any handles a prior respawn left unclaimed before repopulating
        // (releasing the ownership ref each was parked with); the
        // just-installed tree's `create Window` ops drain this pass's set.
        var wr_it = self.window_reuse.iterator();
        while (wr_it.next()) |entry| self.releaseHandle(entry.key_ptr.*, entry.value_ptr.*);
        self.window_reuse.clearRetainingCapacity();
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
        // Destroy plugin-created native views first, while every doomed
        // widget is still alive — mirrors gcOldGenerations: the unparents
        // below cascade-destroy interior children, and the host holds no
        // ref of its own on plugin views.
        for (doomed.items) |id| {
            const m = self.metaGet(id) orelse continue;
            if (m.view_kind) |view_kind| if (self.nodes.get(id)) |w| backend.nativeViewDestroy(view_kind, w);
        }
        for (doomed.items) |id| {
            const m = self.metaGet(id) orelse continue;
            const is_window = std.mem.eql(u8, m.widget_type, "Window");
            // Stash the surviving native window so the respawned tree's matching
            // `create Window` rebinds to it instead of opening a duplicate.
            if (is_window) if (self.nodes.get(id)) |w| self.window_reuse.put(self.gpa, id, w) catch {};
            // Only unparent a "root" of the doomed subtree — a node whose
            // parent is NOT itself being cleared this pass. Unparenting an
            // interior node (e.g. a Box) destroys it and, via GTK's own
            // container teardown, its children too; unparenting those
            // children afterward would be a use-after-free (the identical
            // bug the overlay's `clear()` hit).
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
            // A stashed window's ownership ref is parked in `window_reuse`
            // (released above on the NEXT clear if unclaimed) — release only
            // the non-window handles here.
            if (self.nodes.fetchRemove(id)) |kv| {
                if (!is_window) self.releaseHandle(id, kv.value);
            }
            self.removeMeta(id);
            if (self.children.getPtr(id)) |list| list.deinit(self.gpa);
            _ = self.children.remove(id);
        }
        self.generation = 0;
        std.debug.print("ND_CLEAR_APP_NODES removed={d}\n", .{doomed.items.len});
    }

    /// Widget-preserving cross-window move: relocate a live node's native widget
    /// under `new_parent_id` (optionally before `before_id`) WITHOUT destroying
    /// it. React tears a subtree down when it moves to a different parent
    /// (unmount+remount → the host would remove+create, reloading a <webview>),
    /// so the moved node must stay pinned at a stable React position (a
    /// `createPortal(..., pool)` off-window host) and never actually change React
    /// parent. This is the imperative escape hatch that then relocates only the
    /// LIVE native widget — the loaded page / scroll / JS state survive. Driven
    /// by the reserved `"__ndReparent"` widgetCommand (runtime.zig), itself
    /// issued by the app's `moveNode()` after a drag between windows. The
    /// bookkeeping mirrors the append/insertBefore op arms exactly (detach from
    /// the old sibling list, insert into the new, retarget meta.parent).
    pub fn reparent(self: *Tree, child_id: u32, new_parent_id: u32, before_id: ?u32) void {
        const child = self.nodes.get(child_id) orelse return;
        const new_parent = self.nodes.get(new_parent_id) orelse return;
        const cmeta = self.metaGet(child_id) orelse return;
        const nmeta = self.metaGet(new_parent_id) orelse return;
        // old_parent may be absent — a still-pooled node (meta.parent 0, never
        // appended in React terms) is being shown in a window for the first time.
        const old_parent = self.nodes.get(cmeta.parent);
        const old_parent_kind = if (self.metaGet(cmeta.parent)) |m| m.widget_type else "";
        const before: ?*Widget = if (before_id) |b| self.nodes.get(b) else null;
        backend.reparentChild(child, old_parent, old_parent_kind, new_parent, nmeta.widget_type, before, cmeta.attached);
        if (before_id) |b| self.recordInsertBefore(new_parent_id, child_id, b) else self.recordAppend(new_parent_id, child_id);
        self.setMetaParent(child_id, new_parent_id);
        std.debug.print("ND_REPARENT child={d} parent={d}\n", .{ child_id, new_parent_id });
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

test "SourceList rows: parse round-trip, update replaces, no leak" {
    const gpa = std.testing.allocator;
    var t = Tree.initBare(gpa);
    defer t.deinitMeta();
    try t.putMeta(1, "SourceList", "gallery-sourcelist", null, 0, .{});

    const create_json =
        \\{"items":[{"title":"Inbox","badge":"3"},{"title":"Starred","iconName":"starred-symbolic"}]}
    ;
    const parsed1 = try std.json.parseFromSlice(std.json.Value, gpa, create_json, .{});
    defer parsed1.deinit();
    const rows1 = parseRows(gpa, parsed1.value).?;
    t.setMetaRows(1, rows1);

    const m1 = t.metaGet(1).?;
    try std.testing.expectEqual(@as(usize, 2), m1.rows.?.len);
    try std.testing.expectEqualStrings("Inbox", m1.rows.?[0].title);
    try std.testing.expectEqualStrings("3", m1.rows.?[0].badge.?);
    try std.testing.expect(m1.rows.?[0].icon_name == null);
    try std.testing.expectEqualStrings("Starred", m1.rows.?[1].title);
    try std.testing.expectEqualStrings("starred-symbolic", m1.rows.?[1].icon_name.?);
    try std.testing.expect(m1.rows.?[1].badge == null);

    // An update with a shorter `items` array must free the old rows, not
    // leak them (the testing allocator catches a missed free) — same
    // free-then-replace contract `setMetaText` uses for `text`.
    const update_json =
        \\{"items":[{"title":"Sent"}]}
    ;
    const parsed2 = try std.json.parseFromSlice(std.json.Value, gpa, update_json, .{});
    defer parsed2.deinit();
    const rows2 = parseRows(gpa, parsed2.value).?;
    t.setMetaRows(1, rows2);

    const m2 = t.metaGet(1).?;
    try std.testing.expectEqual(@as(usize, 1), m2.rows.?.len);
    try std.testing.expectEqualStrings("Sent", m2.rows.?[0].title);

    t.removeMeta(1);
    try std.testing.expect(t.metaGet(1) == null);
}
