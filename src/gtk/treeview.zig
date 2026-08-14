// GtkListView + GtkTreeListModel + GtkTreeExpander surface for the <treeview>
// widget. Nodes arrive as a FLAT id/parentId objectList (TreeNode
// shape); the module owns a parsed side copy (TreeData) whose lifetime is
// tied to its GtkTreeListModel via the model's DestroyNotify. passthrough and
// autoexpand are both FALSE (required for GtkTreeExpander; expansion is
// driven from the JS `expanded` flags, never auto). All node-addressed events
// carry { nodeId } — flattened visible indexes are unstable across
// expand/collapse, so only selectedIndex (the controlled prop) uses them.
const std = @import("std");
const gtk = @import("gtk");
const gio = @import("gio");
const glib = @import("glib");
const gobject = @import("gobject");
const protocol = @import("../protocol.zig");
const ndempty = @import("emptystate.zig");

pub const EmitFn = *const fn (node_id: u32, name: []const u8, payload: protocol.EventPayload) void;

var emit: ?EmitFn = null;
/// GtkListView ptr -> the CURRENT parsed tree (freed by its model's
/// DestroyNotify, so this map only points, never owns).
var views: std.AutoHashMapUnmanaged(usize, *TreeData) = .empty;
/// GtkListView ptr -> the widget's ND node id (set by connectEvents; rebuilds
/// read it to rewire the fresh GtkSingleSelection).
var node_ids: std.AutoHashMapUnmanaged(usize, u32) = .empty;
/// > 0 while THIS module is mutating expansion/selection programmatically —
/// the callbacks treat everything inside as echo and stay silent.
var sync_depth: u32 = 0;

const alloc = std.heap.page_allocator;

const Node = struct {
    id: [:0]u8,
    parent_id: ?[]u8 = null,
    title: [:0]u8,
    badge: ?[:0]u8 = null,
    icon: ?[:0]u8 = null,
    has_children: bool = false,
    expanded: bool = false,
    children: std.ArrayList(u32) = .empty,
};

const TreeData = struct {
    nodes: std.ArrayList(Node) = .empty,
    by_id: std.StringHashMapUnmanaged(u32) = .empty,
    roots: std.ArrayList(u32) = .empty,
    /// indentationPerLevel: create-only, so it is carried across the
    /// `nodes` rebuilds that replace the rest of this struct.
    indent: i64 = 16,

    fn deinit(self: *TreeData) void {
        for (self.nodes.items) |*n| {
            alloc.free(n.id);
            if (n.parent_id) |p| alloc.free(p);
            alloc.free(n.title);
            if (n.badge) |b| alloc.free(b);
            if (n.icon) |i| alloc.free(i);
            n.children.deinit(alloc);
        }
        self.nodes.deinit(alloc);
        self.by_id.deinit(alloc);
        self.roots.deinit(alloc);
    }
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

fn propArray(props: ?std.json.Value, key: []const u8) ?std.json.Array {
    const v = props orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .array => field.array,
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

fn objStr(obj: std.json.ObjectMap, key: []const u8) ?[]const u8 {
    return switch (obj.get(key) orelse return null) {
        .string => |s| s,
        else => null,
    };
}

fn objBool(obj: std.json.ObjectMap, key: []const u8) ?bool {
    return switch (obj.get(key) orelse return null) {
        .bool => |b| b,
        else => null,
    };
}

/// The tracked handle is the GtkScrolledWindow; GtkListView implements
/// GtkScrollable, so it is a direct child (never viewport-wrapped). The
/// empty-state registry wins while an AdwStatusPage is swapped in.
fn innerList(widget: *gtk.Widget) ?*gtk.ListView {
    const sw: *gtk.ScrolledWindow = @ptrCast(@alignCast(widget));
    if (ndempty.innerOf(sw)) |inner| return @ptrCast(@alignCast(inner));
    const child = gtk.ScrolledWindow.getChild(sw) orelse return null;
    return @ptrCast(@alignCast(child));
}

fn currentSelection(lv: *gtk.ListView) ?*gtk.SingleSelection {
    const model = gtk.ListView.getModel(lv) orelse return null;
    return @ptrCast(@alignCast(model));
}

fn rowNodeId(row: *gtk.TreeListRow, buf: *[256]u8) ?[]const u8 {
    const inner = gtk.TreeListRow.getItem(row) orelse return null; // transfer full
    defer gobject.Object.unref(inner);
    const so: *gtk.StringObject = @ptrCast(@alignCast(inner));
    const s = std.mem.span(gtk.StringObject.getString(so));
    if (s.len > buf.len) return null;
    @memcpy(buf[0..s.len], s);
    return buf[0..s.len];
}

// ---- flat-list parse -------------------------------------------------------

fn parseNodes(arr: ?std.json.Array) *TreeData {
    const tree = alloc.create(TreeData) catch @panic("OOM parsing TreeView nodes");
    tree.* = .{};
    const items = arr orelse return tree;
    for (items.items) |it| {
        if (it != .object) continue;
        const id = objStr(it.object, "id") orelse continue;
        const title = objStr(it.object, "title") orelse "";
        var node = Node{
            .id = alloc.dupeZ(u8, id) catch continue,
            .title = alloc.dupeZ(u8, title) catch continue,
        };
        if (objStr(it.object, "parentId")) |p| node.parent_id = alloc.dupe(u8, p) catch null;
        if (objStr(it.object, "badge")) |b| node.badge = alloc.dupeZ(u8, b) catch null;
        if (objStr(it.object, "iconName")) |i| node.icon = alloc.dupeZ(u8, i) catch null;
        node.has_children = objBool(it.object, "hasChildren") orelse false;
        node.expanded = objBool(it.object, "expanded") orelse false;
        const idx: u32 = @intCast(tree.nodes.items.len);
        tree.nodes.append(alloc, node) catch continue;
        tree.by_id.put(alloc, node.id, idx) catch {};
    }
    // Link children by parentId; an unknown parentId degrades to a root node.
    for (tree.nodes.items, 0..) |*n, i| {
        const idx: u32 = @intCast(i);
        if (n.parent_id) |pid| {
            if (tree.by_id.get(pid)) |pidx| {
                tree.nodes.items[pidx].children.append(alloc, idx) catch {};
                continue;
            }
        }
        tree.roots.append(alloc, idx) catch {};
    }
    return tree;
}

fn cbTreeDataDestroy(data: ?*anyopaque) callconv(.c) void {
    const tree: *TreeData = @ptrCast(@alignCast(data orelse return));
    tree.deinit();
    alloc.destroy(tree);
}

/// GtkTreeListModelCreateModelFunc: children of `item` (a StringObject node
/// id), or null for a leaf (which is what hides the expander). A node with
/// hasChildren=true but no child nodes yet (lazy loading) gets an EMPTY model
/// so its expander shows; the app appends children on nodeExpanded.
fn cbCreateChildModel(item: *gobject.Object, user_data: ?*anyopaque) callconv(.c) ?*gio.ListModel {
    const tree: *TreeData = @ptrCast(@alignCast(user_data orelse return null));
    const so: *gtk.StringObject = @ptrCast(@alignCast(item));
    const id = std.mem.span(gtk.StringObject.getString(so));
    const idx = tree.by_id.get(id) orelse return null;
    const node = tree.nodes.items[idx];
    if (node.children.items.len == 0 and !node.has_children) return null;
    const sl = gtk.StringList.new(null);
    for (node.children.items) |c| gtk.StringList.append(sl, tree.nodes.items[c].id);
    return sl.as(gio.ListModel);
}

// ---- model chain -----------------------------------------------------------

fn buildSelection(tree: *TreeData) *gtk.SingleSelection {
    const roots = gtk.StringList.new(null);
    for (tree.roots.items) |ri| gtk.StringList.append(roots, tree.nodes.items[ri].id);
    // passthrough=FALSE (items are GtkTreeListRow wrappers, required for
    // GtkTreeExpander); autoexpand=FALSE (JS `expanded` flags drive it).
    const tlm = gtk.TreeListModel.new(roots.as(gio.ListModel), 0, 0, &cbCreateChildModel, tree, &cbTreeDataDestroy);
    const selection = gtk.SingleSelection.new(tlm.as(gio.ListModel)); // transfer-full: selection owns model
    gtk.SingleSelection.setAutoselect(selection, 0);
    gtk.SingleSelection.setCanUnselect(selection, 1);
    // gtk_single_selection_new() auto-selects position 0 as part of
    // constructing the selection over the model, before autoselect above
    // takes effect — clear it explicitly so selectedIndex: -1 (the default)
    // means no row is visibly selected, not a phantom row-0 highlight.
    gtk.SingleSelection.setSelected(selection, std.math.maxInt(c_uint));
    return selection;
}

/// Walks the visible rows top-down, driving each row's expansion from its
/// node's `expanded` flag. Expanding row i inserts its children at i+1, so
/// the re-checked getNItems bound picks newly revealed rows up in the same
/// pass. sync_depth silences the per-row notify::expanded echoes.
fn syncExpansion(selection: *gtk.SingleSelection, tree: *TreeData) void {
    sync_depth += 1;
    defer sync_depth -= 1;
    const tlm: *gtk.TreeListModel = @ptrCast(@alignCast(gtk.SingleSelection.getModel(selection).?));
    var i: c_uint = 0;
    while (i < gio.ListModel.getNItems(tlm.as(gio.ListModel))) : (i += 1) {
        const row = gtk.TreeListModel.getRow(tlm, i) orelse continue; // transfer full
        defer gobject.Object.unref(@ptrCast(@alignCast(row)));
        var buf: [256]u8 = undefined;
        const id = rowNodeId(row, &buf) orelse continue;
        const idx = tree.by_id.get(id) orelse continue;
        const want: c_int = @intFromBool(tree.nodes.items[idx].expanded);
        if (gtk.TreeListRow.getExpanded(row) != want) gtk.TreeListRow.setExpanded(row, want);
    }
}

// ---- row factory -----------------------------------------------------------

fn tvSetup(_: *gobject.Object, list_item: *gtk.ListItem, _: ?*anyopaque) callconv(.c) void {
    const expander = gtk.TreeExpander.new();
    // GtkTreeExpander's own depth indent is theme-fixed; tvBind applies
    // indentationPerLevel as the row's margin instead, so the two must not
    // stack. indent-for-icon stays on so leaf titles line up with siblings.
    gtk.TreeExpander.setIndentForDepth(expander, 0);
    const box = gtk.Box.new(.horizontal, 6);
    const icon = gtk.Image.new();
    gtk.Widget.setVisible(icon.as(gtk.Widget), 0);
    const label = gtk.Label.new(null);
    gtk.Label.setXalign(label, 0.0);
    gtk.Widget.setHexpand(label.as(gtk.Widget), 1);
    const badge = gtk.Label.new(null);
    gtk.Widget.addCssClass(badge.as(gtk.Widget), "dimmed");
    gtk.Widget.addCssClass(badge.as(gtk.Widget), "numeric");
    gtk.Widget.setVisible(badge.as(gtk.Widget), 0);
    gtk.Box.append(box, icon.as(gtk.Widget));
    gtk.Box.append(box, label.as(gtk.Widget));
    gtk.Box.append(box, badge.as(gtk.Widget));
    gtk.TreeExpander.setChild(expander, box.as(gtk.Widget));
    const eobj: *gobject.Object = @ptrCast(@alignCast(expander));
    gobject.Object.setData(eobj, "nd-icon", @ptrCast(icon));
    gobject.Object.setData(eobj, "nd-label", @ptrCast(label));
    gobject.Object.setData(eobj, "nd-badge", @ptrCast(badge));
    gtk.ListItem.setChild(list_item, expander.as(gtk.Widget));
    // Let the expander's own keybindings handle navigation.
    gtk.ListItem.setFocusable(list_item, 0);
}

/// bind user_data = the GtkListView ptr (the factory outlives every model
/// rebuild, so it can't capture a TreeData — it looks the CURRENT one up).
fn tvBind(_: *gobject.Object, list_item: *gtk.ListItem, data: ?*anyopaque) callconv(.c) void {
    const lv_ptr: usize = @intFromPtr(data);
    const child = gtk.ListItem.getChild(list_item) orelse return;
    const expander: *gtk.TreeExpander = @ptrCast(@alignCast(child));
    const item = gtk.ListItem.getItem(list_item) orelse return; // transfer none
    const row: *gtk.TreeListRow = @ptrCast(@alignCast(item));
    gtk.TreeExpander.setListRow(expander, row);

    var buf: [256]u8 = undefined;
    const id = rowNodeId(row, &buf) orelse return;
    const tree = views.get(lv_ptr) orelse return;
    const idx = tree.by_id.get(id) orelse return;
    const node = tree.nodes.items[idx];

    // Depth indent, the AppKit peer being NSOutlineView.indentationPerLevel
    // (same margin-start shape sourcetree.zig uses on its rows).
    const depth: u32 = gtk.TreeListRow.getDepth(row);
    gtk.Widget.setMarginStart(child, @intCast(@as(i64, depth) * tree.indent));

    const eobj: *gobject.Object = @ptrCast(@alignCast(expander));
    if (gobject.Object.getData(eobj, "nd-label")) |raw| {
        gtk.Label.setText(@ptrCast(@alignCast(raw)), node.title);
    }
    if (gobject.Object.getData(eobj, "nd-icon")) |raw| {
        const icon: *gtk.Image = @ptrCast(@alignCast(raw));
        if (node.icon) |ic| {
            gtk.Image.setFromIconName(icon, ic);
            gtk.Widget.setVisible(icon.as(gtk.Widget), 1);
        } else {
            gtk.Widget.setVisible(icon.as(gtk.Widget), 0);
        }
    }
    if (gobject.Object.getData(eobj, "nd-badge")) |raw| {
        const badge: *gtk.Label = @ptrCast(@alignCast(raw));
        if (node.badge) |b| {
            gtk.Label.setText(badge, b);
            gtk.Widget.setVisible(badge.as(gtk.Widget), 1);
        } else {
            gtk.Widget.setVisible(badge.as(gtk.Widget), 0);
        }
    }

    // Wire the expansion notification once per live GtkTreeListRow (rows
    // persist while visible; list ITEMS recycle around them).
    const robj: *gobject.Object = @ptrCast(@alignCast(row));
    if (gobject.Object.getData(robj, "nd-exp-wired") == null) {
        gobject.Object.setData(robj, "nd-exp-wired", @ptrFromInt(1));
        gobject.Object.setData(robj, "nd-lv", @ptrFromInt(lv_ptr));
        const widget_node: usize = node_ids.get(lv_ptr) orelse 0;
        _ = gobject.signalConnectData(robj, "notify::expanded", @ptrCast(&cbRowExpanded), @ptrFromInt(widget_node), null, .{});
    }
}

// ---- create / applyProps ---------------------------------------------------

pub fn create(props: ?std.json.Value, dupeZ: *const fn ([]const u8) [:0]const u8) *gtk.Widget {
    _ = dupeZ; // node strings live in the page-heap TreeData, not the backend arena
    const tree = parseNodes(propArray(props, "nodes"));
    tree.indent = propInt(props, "indentationPerLevel") orelse 16;
    const selection = buildSelection(tree);
    const factory = gtk.SignalListItemFactory.new();
    const lv = gtk.ListView.new(selection.as(gtk.SelectionModel), factory.as(gtk.ListItemFactory)); // transfer-full: list owns selection+factory
    _ = gobject.signalConnectData(@ptrCast(@alignCast(factory)), "setup", @ptrCast(&tvSetup), null, null, .{});
    _ = gobject.signalConnectData(@ptrCast(@alignCast(factory)), "bind", @ptrCast(&tvBind), @ptrFromInt(@intFromPtr(lv)), null, .{});
    views.put(alloc, @intFromPtr(lv), tree) catch {};
    _ = gtk.Widget.signals.destroy.connect(lv.as(gtk.Widget), ?*anyopaque, &cbViewDestroyed, null, .{});

    syncExpansion(selection, tree);
    const sel_idx = propInt(props, "selectedIndex") orelse -1;
    if (sel_idx >= 0) gtk.SingleSelection.setSelected(selection, @intCast(sel_idx));

    const sw = gtk.ScrolledWindow.new();
    gtk.ScrolledWindow.setChild(sw, lv.as(gtk.Widget));
    ndempty.register(sw, lv.as(gtk.Widget));
    ndempty.configure(sw, propStr(props, "emptyIconName"), propStr(props, "emptyTitle"), propStr(props, "emptyDescription"));
    const n_nodes: usize = if (propArray(props, "nodes")) |arr| arr.items.len else 0;
    ndempty.update(sw, n_nodes == 0);
    return sw.as(gtk.Widget);
}

pub fn applyProps(widget: *gtk.Widget, props: ?std.json.Value, dupeZ: *const fn ([]const u8) [:0]const u8) void {
    _ = dupeZ;
    const lv = innerList(widget) orelse return;
    const lv_ptr = @intFromPtr(lv);

    ndempty.configure(@ptrCast(@alignCast(widget)), propStr(props, "emptyIconName"), propStr(props, "emptyTitle"), propStr(props, "emptyDescription"));
    if (propArray(props, "nodes")) |arr| {
        // Full rebuild: fresh TreeData + model chain; the OLD TreeData is
        // freed by the old model's DestroyNotify once setModel releases it.
        const tree = parseNodes(arr);
        if (views.get(lv_ptr)) |old| tree.indent = old.indent;
        const selection = buildSelection(tree);
        views.put(alloc, lv_ptr, tree) catch {};
        sync_depth += 1;
        gtk.ListView.setModel(lv, selection.as(gtk.SelectionModel));
        sync_depth -= 1;
        gobject.Object.unref(@ptrCast(@alignCast(selection))); // setModel refs; the view owns it now
        ndempty.update(@ptrCast(@alignCast(widget)), arr.items.len == 0);
        const live = currentSelection(lv) orelse return;
        syncExpansion(live, tree);
        if (node_ids.get(lv_ptr)) |nid| connectSelection(live, nid);
    }
    if (propInt(props, "selectedIndex")) |idx| {
        const selection = currentSelection(lv) orelse return;
        const cur = gtk.SingleSelection.getSelected(selection);
        const cur_idx: i64 = if (cur == std.math.maxInt(c_uint)) -1 else @intCast(cur);
        if (cur_idx != idx) {
            sync_depth += 1;
            gtk.SingleSelection.setSelected(selection, if (idx >= 0) @intCast(idx) else std.math.maxInt(c_uint));
            sync_depth -= 1;
        }
    }
}

// ---- events ----------------------------------------------------------------

fn connectSelection(selection: *gtk.SingleSelection, node_id: u32) void {
    _ = gobject.signalConnectData(@ptrCast(@alignCast(selection)), "notify::selected", @ptrCast(&cbSelected), @ptrFromInt(@as(usize, node_id)), null, .{});
}

pub fn connectEvents(widget: *gtk.Widget, node_id: u32, emit_fn: EmitFn) void {
    emit = emit_fn;
    const lv = innerList(widget) orelse return;
    node_ids.put(alloc, @intFromPtr(lv), node_id) catch {};
    if (currentSelection(lv)) |selection| connectSelection(selection, node_id);
    _ = gobject.signalConnectData(@ptrCast(@alignCast(lv)), "activate", @ptrCast(&cbActivate), @ptrFromInt(@as(usize, node_id)), null, .{});
}

fn emitNode(node_id: u32, name: []const u8, tree_node_id: ?[]const u8) void {
    const f = emit orelse return;
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(std.heap.page_allocator);
    const value: std.json.Value = if (tree_node_id) |i| .{ .string = i } else .null;
    payload.put(std.heap.page_allocator, "nodeId", value) catch {};
    f(node_id, name, .{ .data = .{ .object = payload } });
}

// notify:: handlers get (object, pspec, user_data).
fn cbSelected(obj: *gobject.Object, _: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
    if (sync_depth > 0) return;
    const node_id: u32 = @intCast(@intFromPtr(data));
    const selection: *gtk.SingleSelection = @ptrCast(@alignCast(obj));
    const item = gtk.SingleSelection.getSelectedItem(selection); // transfer none
    if (item) |it| {
        const row: *gtk.TreeListRow = @ptrCast(@alignCast(it));
        var buf: [256]u8 = undefined;
        emitNode(node_id, "selectionChanged", rowNodeId(row, &buf));
    } else {
        emitNode(node_id, "selectionChanged", null);
    }
}

// the ListView `activate` signal passes (list_view, position, user_data).
fn cbActivate(obj: *gobject.Object, position: c_uint, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const lv: *gtk.ListView = @ptrCast(@alignCast(obj));
    const model = gtk.ListView.getModel(lv) orelse return;
    const item = gio.ListModel.getObject(model.as(gio.ListModel), position) orelse return; // transfer full
    defer gobject.Object.unref(item);
    const row: *gtk.TreeListRow = @ptrCast(@alignCast(item));
    var buf: [256]u8 = undefined;
    emitNode(node_id, "rowActivated", rowNodeId(row, &buf));
}

fn cbRowExpanded(obj: *gobject.Object, _: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
    if (sync_depth > 0) return;
    const node_id: u32 = @intCast(@intFromPtr(data));
    if (node_id == 0) return;
    const row: *gtk.TreeListRow = @ptrCast(@alignCast(obj));
    const expanded = gtk.TreeListRow.getExpanded(row) != 0;
    var buf: [256]u8 = undefined;
    const id = rowNodeId(row, &buf) orelse return;
    // Keep the side copy coherent so a later syncExpansion pass doesn't
    // fight a user-driven expand the app hasn't re-rendered yet.
    if (gobject.Object.getData(obj, "nd-lv")) |lvp| {
        if (views.get(@intFromPtr(lvp))) |tree| {
            if (tree.by_id.get(id)) |idx| tree.nodes.items[idx].expanded = expanded;
        }
    }
    emitNode(node_id, if (expanded) "nodeExpanded" else "nodeCollapsed", id);
}

fn cbViewDestroyed(w: *gtk.Widget, _: ?*anyopaque) callconv(.c) void {
    const key = @intFromPtr(w);
    _ = views.remove(key); // the TreeData itself is freed by its model's DestroyNotify
    _ = node_ids.remove(key);
}
