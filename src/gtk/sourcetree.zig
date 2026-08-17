// GtkListBox-of-AdwActionRows surface for the <sourcetree> widget: a
// hierarchical sidebar (id/parentId nodes, app-controlled expansion) with
// per-row trailing actions. Nodes arrive FLAT (SourceTreeNode shape); the
// module owns a parsed side copy (Store) per ListBox and rebuilds the whole
// row set on every `nodes`/`actions` update — AdwActionRow is a
// widget-per-row primitive, so there is no incremental model to diff into.
// All node-addressed events carry { nodeId } (actionClicked adds actionId);
// visible indexes are unstable across expand/collapse, so no index payloads.
// `selectedId` is the controlled selection prop; "" means no selection.
const std = @import("std");
const gtk = @import("gtk");
const gobject = @import("gobject");
const adw = @import("adw");
const protocol = @import("../protocol.zig");
const ndempty = @import("emptystate.zig");
const ndicons = @import("icons.zig");

pub const EmitFn = *const fn (node_id: u32, name: []const u8, payload: protocol.EventPayload) void;

var emit: ?EmitFn = null;
/// GtkListBox ptr -> the CURRENT parsed store (owned here; freed on the next
/// rebuild or on widget destroy).
var stores: std.AutoHashMapUnmanaged(usize, *Store) = .empty;
/// GtkListBox ptr -> the widget's ND node id (set by connectEvents).
var node_ids: std.AutoHashMapUnmanaged(usize, u32) = .empty;
/// > 0 while THIS module is mutating rows/selection programmatically — the
/// callbacks treat everything inside as echo and stay silent (same counter
/// shape as src/gtk/treeview.zig).
var sync_depth: u32 = 0;

const alloc = std.heap.page_allocator;

const Node = struct {
    id: [:0]u8,
    parent_id: ?[]u8 = null,
    title: [:0]u8,
    caption: ?[:0]u8 = null,
    icon: ?[:0]u8 = null,
    icon_data: ?[]u8 = null,
    caption_icon: ?[:0]u8 = null,
    badge: ?[:0]u8 = null,
    section: bool = false,
    has_children: bool = false,
    expanded: bool = false,
    selectable: bool = true,
    test_id: ?[]u8 = null,
    action_ids: std.ArrayList([]u8) = .empty,
    children: std.ArrayList(u32) = .empty,
};

const Action = struct {
    id: [:0]u8,
    icon: [:0]u8,
    label: ?[:0]u8 = null,
    tooltip: ?[:0]u8 = null,
    destructive: bool = false,
};

const Store = struct {
    nodes: std.ArrayList(Node) = .empty,
    by_id: std.StringHashMapUnmanaged(u32) = .empty,
    roots: std.ArrayList(u32) = .empty,
    actions: std.ArrayList(Action) = .empty,
    action_by_id: std.StringHashMapUnmanaged(u32) = .empty,
    // create-only knobs, carried across rebuilds
    hover_actions: bool = true,
    // Per-level step, matched to the disclosure gutter so a child's title
    // clears its parent's by exactly one gutter.
    indent: i64 = gutter_px,
    /// Whether ANY row in this tree draws a disclosure. The gutter is a
    /// per-TREE decision: a flat list reserves nothing, and one branch
    /// anywhere puts the gutter back on every row so branch and leaf titles
    /// stay on one origin. Recomputed with the nodes.
    expandable: bool = false,

    fn deinitNodes(self: *Store) void {
        for (self.nodes.items) |*n| {
            alloc.free(n.id);
            if (n.parent_id) |p| alloc.free(p);
            alloc.free(n.title);
            if (n.caption) |c| alloc.free(c);
            if (n.icon) |i| alloc.free(i);
            if (n.icon_data) |d| alloc.free(d);
            if (n.caption_icon) |i| alloc.free(i);
            if (n.badge) |b| alloc.free(b);
            if (n.test_id) |t| alloc.free(t);
            for (n.action_ids.items) |a| alloc.free(a);
            n.action_ids.deinit(alloc);
            n.children.deinit(alloc);
        }
        self.nodes.deinit(alloc);
        self.by_id.deinit(alloc);
        self.roots.deinit(alloc);
        self.nodes = .empty;
        self.by_id = .empty;
        self.roots = .empty;
    }

    fn deinitActions(self: *Store) void {
        for (self.actions.items) |*a| {
            alloc.free(a.id);
            alloc.free(a.icon);
            if (a.label) |l| alloc.free(l);
            if (a.tooltip) |t| alloc.free(t);
        }
        self.actions.deinit(alloc);
        self.action_by_id.deinit(alloc);
        self.actions = .empty;
        self.action_by_id = .empty;
    }

    fn deinit(self: *Store) void {
        self.deinitNodes();
        self.deinitActions();
    }
};

// ---- prop helpers (private mirror of treeview.zig's) -----------------------

fn propArray(props: ?std.json.Value, key: []const u8) ?std.json.Array {
    const v = props orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .array => field.array,
        else => null,
    };
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

fn asObject(ptr: anytype) *gobject.Object {
    return @ptrCast(@alignCast(ptr));
}

/// The tracked handle is the GtkScrolledWindow; GtkListBox is not
/// GtkScrollable, so GTK wraps it in an implicit GtkViewport (same shape as
/// SourceList — see codegen's scrolledWindowInner). The empty-state registry
/// wins while an AdwStatusPage is swapped in.
fn innerListBox(widget: *gtk.Widget) ?*gtk.ListBox {
    const sw: *gtk.ScrolledWindow = @ptrCast(@alignCast(widget));
    if (ndempty.innerOf(sw)) |inner| return @ptrCast(@alignCast(inner));
    const child = gtk.ScrolledWindow.getChild(sw) orelse return null;
    if (gobject.ext.isA(child, gtk.Viewport)) {
        const vp: *gtk.Viewport = @ptrCast(@alignCast(child));
        const inner = gtk.Viewport.getChild(vp) orelse return null;
        return @ptrCast(@alignCast(inner));
    }
    return @ptrCast(@alignCast(child));
}

// ---- flat-list parse -------------------------------------------------------

fn parseNodes(store: *Store, arr: std.json.Array) void {
    for (arr.items) |it| {
        if (it != .object) continue;
        const id = objStr(it.object, "id") orelse continue;
        const title = objStr(it.object, "title") orelse "";
        var node = Node{
            .id = alloc.dupeZ(u8, id) catch continue,
            .title = alloc.dupeZ(u8, title) catch continue,
        };
        if (objStr(it.object, "parentId")) |p| node.parent_id = alloc.dupe(u8, p) catch null;
        if (objStr(it.object, "caption")) |c| node.caption = alloc.dupeZ(u8, c) catch null;
        if (objStr(it.object, "iconName")) |i| node.icon = alloc.dupeZ(u8, i) catch null;
        if (objStr(it.object, "iconData")) |d| node.icon_data = alloc.dupe(u8, d) catch null;
        if (objStr(it.object, "captionIconName")) |i| node.caption_icon = alloc.dupeZ(u8, i) catch null;
        if (objStr(it.object, "badge")) |b| node.badge = alloc.dupeZ(u8, b) catch null;
        node.section = objBool(it.object, "section") orelse false;
        node.has_children = objBool(it.object, "hasChildren") orelse false;
        node.expanded = objBool(it.object, "expanded") orelse false;
        node.selectable = objBool(it.object, "selectable") orelse !node.section;
        if (objStr(it.object, "testID")) |t| node.test_id = alloc.dupe(u8, t) catch null;
        if (it.object.get("actionIds")) |ids| {
            if (ids == .array) for (ids.array.items) |aid| {
                if (aid != .string) continue;
                const owned = alloc.dupe(u8, aid.string) catch continue;
                node.action_ids.append(alloc, owned) catch alloc.free(owned);
            };
        }
        const idx: u32 = @intCast(store.nodes.items.len);
        store.nodes.append(alloc, node) catch continue;
        store.by_id.put(alloc, node.id, idx) catch {};
    }
    // Link children by parentId; an unknown parentId degrades to a root node.
    for (store.nodes.items, 0..) |*n, i| {
        const idx: u32 = @intCast(i);
        if (n.parent_id) |pid| {
            if (store.by_id.get(pid)) |pidx| {
                store.nodes.items[pidx].children.append(alloc, idx) catch {};
                continue;
            }
        }
        store.roots.append(alloc, idx) catch {};
    }
    // Section rows are excluded: they draw no disclosure (the app's `expanded`
    // flag is the only thing that opens a shelf), so a list of sections over
    // flat rows still owes its rows no gutter.
    store.expandable = false;
    for (store.nodes.items) |n| {
        if (n.section) continue;
        if (n.has_children or n.children.items.len > 0) {
            store.expandable = true;
            break;
        }
    }
}

fn parseActions(store: *Store, arr: std.json.Array) void {
    for (arr.items) |it| {
        if (it != .object) continue;
        const id = objStr(it.object, "id") orelse continue;
        const icon = objStr(it.object, "iconName") orelse "";
        var action = Action{
            .id = alloc.dupeZ(u8, id) catch continue,
            .icon = alloc.dupeZ(u8, icon) catch continue,
        };
        if (objStr(it.object, "label")) |l| action.label = alloc.dupeZ(u8, l) catch null;
        if (objStr(it.object, "tooltip")) |t| action.tooltip = alloc.dupeZ(u8, t) catch null;
        action.destructive = objBool(it.object, "destructive") orelse false;
        const idx: u32 = @intCast(store.actions.items.len);
        store.actions.append(alloc, action) catch continue;
        store.action_by_id.put(alloc, action.id, idx) catch {};
    }
}

// ---- row building ----------------------------------------------------------

/// Rows carry their node index as GObject data (idx+1 so 0 stays "unset");
/// action buttons additionally carry their action index the same way.
fn setIdx(obj: *gobject.Object, key: [*:0]const u8, idx: u32) void {
    gobject.Object.setData(obj, key, @ptrFromInt(@as(usize, idx) + 1));
}

fn getIdx(obj: *gobject.Object, key: [*:0]const u8) ?u32 {
    const raw = gobject.Object.getData(obj, key) orelse return null;
    return @intCast(@intFromPtr(raw) - 1);
}

/// Width every row reserves for its disclosure, branch or leaf. A branch's
/// inline arrow is wider than one indent step, so without a matching gutter on
/// the leaves a parent's title lands RIGHT of the children it indents.
const gutter_px: c_int = 24;

/// The arrow GtkTreeExpander draws, sized for the gutter. It is an image and
/// not a GtkButton because a themed button cannot reach 24px: libadwaita gives
/// `button.image-button` a 24px min-width PLUS 5px of side padding, and
/// set_size_request only ever raises a minimum.
fn makeDisclosure(box: *gtk.ListBox, node_idx: u32, expanded: bool) *gtk.Widget {
    const img = gtk.Image.newFromIconName(disclosureIconName(expanded));
    gtk.Widget.setValign(img.as(gtk.Widget), .center);
    gtk.Widget.setSizeRequest(img.as(gtk.Widget), gutter_px, -1);
    const click = gtk.GestureClick.new();
    gtk.GestureSingle.setButton(click.as(gtk.GestureSingle), 1);
    setIdx(asObject(click), "nd-node-idx", node_idx);
    _ = gtk.GestureClick.signals.pressed.connect(click, *gtk.ListBox, &cbDisclosurePressed, box, .{});
    _ = gtk.GestureClick.signals.released.connect(click, *gtk.ListBox, &cbDisclosureReleased, box, .{});
    gtk.Widget.addController(img.as(gtk.Widget), click.as(gtk.EventController));
    return img.as(gtk.Widget);
}

/// `pan-*` ships inside libgtk's own resource icon theme, which GTK falls back
/// to when the active theme carries no entry, so the name resolves with no
/// icon theme installed at all. A `gtk_icon_theme_has_icon` gate in front of a
/// `go-*` fallback bought nothing and hid the real failure: hasIcon answers 1
/// for a theme whose symbolic SVG rasterizes to zero pixels, which is the only
/// way this arrow has ever gone missing.
fn disclosureIconName(expanded: bool) [:0]const u8 {
    return if (expanded) "pan-down-symbolic" else "pan-end-symbolic";
}

fn makeGutterSpacer() *gtk.Widget {
    const img = gtk.Image.new();
    gtk.Widget.setSizeRequest(img.as(gtk.Widget), gutter_px, -1);
    return img.as(gtk.Widget);
}

/// `adw_action_row_add_prefix` PREPENDS, so a row wanting several prefixes has
/// to hand over ONE box or they land in reverse. Answers null when the row
/// owes no prefix at all: an empty box still costs the header its 6px spacing.
fn makePrefixBox(box: *gtk.ListBox, store: *const Store, node: *const Node, node_idx: u32) ?*gtk.Widget {
    if (!store.expandable and node.icon_data == null and node.icon == null and node.caption_icon == null) return null;
    const prefix = gtk.Box.new(.horizontal, 6);
    gtk.Widget.setValign(prefix.as(gtk.Widget), .center);
    if (node.has_children) {
        gtk.Box.append(prefix, makeDisclosure(box, node_idx, node.expanded));
    } else if (store.expandable) {
        gtk.Box.append(prefix, makeGutterSpacer());
    }
    // Image bytes beat a theme name: a favicon has no freedesktop name, and
    // this is the only way a browser sidebar can show one.
    if (node.icon_data) |data| {
        if (ndicons.imageFromData(data, "SourceTree")) |img| gtk.Box.append(prefix, img.as(gtk.Widget));
    } else if (node.icon) |ic| {
        const img = gtk.Image.newFromIconName(ndicons.symbolic(ic));
        gtk.Box.append(prefix, img.as(gtk.Widget));
    }
    if (node.caption_icon) |ic| {
        // No AdwActionRow slot exists on the subtitle line, so the caption icon
        // rides the same prefix run (documented asymmetry; AppKit inlines it).
        const img = gtk.Image.newFromIconName(ndicons.symbolic(ic));
        gtk.Box.append(prefix, img.as(gtk.Widget));
    }
    return prefix.as(gtk.Widget);
}

fn makeActionButton(box: *gtk.ListBox, node_idx: u32, action_idx: u32, action: Action) *gtk.Button {
    const btn = gtk.Button.new();
    const has_icon = action.icon.len > 0;
    if (action.label) |l| {
        if (has_icon) {
            const content = adw.ButtonContent.new();
            adw.ButtonContent.setIconName(content, ndicons.symbolic(action.icon));
            adw.ButtonContent.setLabel(content, l);
            gtk.Button.setChild(btn, content.as(gtk.Widget));
        } else {
            gtk.Button.setLabel(btn, l);
        }
    } else if (has_icon) {
        gtk.Button.setIconName(btn, ndicons.symbolic(action.icon));
        // Round chip, the GNOME shape for an icon-only row button.
        gtk.Widget.addCssClass(btn.as(gtk.Widget), "circular");
    }
    // Both shapes keep the button's own chrome: `.flat` leaves a labelled
    // action reading as bare text and an icon-only one as a bare glyph painted
    // straight onto the row fill, with no hit area next to its chromed sibling.
    if (action.destructive) gtk.Widget.addCssClass(btn.as(gtk.Widget), "destructive-action");
    if (action.tooltip) |t| gtk.Widget.setTooltipText(btn.as(gtk.Widget), t);
    gtk.Widget.setValign(btn.as(gtk.Widget), .center);
    setIdx(asObject(btn), "nd-node-idx", node_idx);
    setIdx(asObject(btn), "nd-action-idx", action_idx);
    _ = gobject.signalConnectData(asObject(btn), "clicked", @ptrCast(&cbActionClicked), box, null, .{});
    return btn;
}

/// Appends one row for `node`, then (when expanded) recurses into its
/// children — the depth-first flatten that honors app-controlled expansion.
fn appendRow(box: *gtk.ListBox, store: *Store, node_idx: u32, depth: u32) void {
    const node = &store.nodes.items[node_idx];
    const indent: c_int = @intCast(@as(i64, depth) * store.indent);

    if (node.section) {
        // Group header: a plain non-interactive caption, the GNOME sidebar
        // shape. libadwaita styles `.navigation-sidebar > .header > .heading`
        // with a 12px margin, which is exactly the AdwActionRow header inset
        // below it, so the label sits on the data rows' gutter origin. The
        // label must be the row's DIRECT child for that selector to match.
        // A collapsible section shelf is driven by the app (the `expanded`
        // flag), never by chrome inside the header.
        const row = gtk.ListBoxRow.new();
        gtk.ListBoxRow.setActivatable(row, 0);
        gtk.ListBoxRow.setSelectable(row, 0);
        gtk.Widget.addCssClass(row.as(gtk.Widget), "header");
        if (gtk.ListBox.getRowAtIndex(box, 0) == null) gtk.Widget.addCssClass(row.as(gtk.Widget), "first");
        const label = gtk.Label.new(node.title);
        gtk.Label.setXalign(label, 0.0);
        gtk.Widget.setHexpand(label.as(gtk.Widget), 1);
        gtk.Widget.addCssClass(label.as(gtk.Widget), "heading");
        gtk.Widget.addCssClass(label.as(gtk.Widget), "dimmed");
        gtk.ListBoxRow.setChild(row, label.as(gtk.Widget));
        gtk.Widget.setMarginStart(row.as(gtk.Widget), indent);
        setIdx(asObject(row), "nd-node-idx", node_idx);
        gtk.ListBox.append(box, row.as(gtk.Widget));
    } else {
        const row = adw.ActionRow.new();
        // One line each, end-ellipsized, matching what NDShell's source-list
        // cells already do (`.byTruncatingTail`). AdwActionRow wraps by
        // default, which in a sidebar makes row height a function of title
        // length — a two-line row next to one-line neighbours reads as a
        // different KIND of row rather than a longer one, and a wrapped URL
        // breaks mid-token. A navigation row is a fixed-height row.
        adw.ActionRow.setTitleLines(row, 1);
        adw.ActionRow.setSubtitleLines(row, 1);
        // Title and caption are data, not markup. AdwPreferencesRow parses
        // both as Pango by default, so an & or < in either fails the parse and
        // leaves that label EMPTY rather than escaped.
        adw.PreferencesRow.setUseMarkup(row.as(adw.PreferencesRow), 0);
        adw.PreferencesRow.setTitle(row.as(adw.PreferencesRow), node.title);
        if (node.caption) |c| adw.ActionRow.setSubtitle(row, c);
        if (!node.selectable) {
            gtk.ListBoxRow.setActivatable(row.as(gtk.ListBoxRow), 0);
            gtk.ListBoxRow.setSelectable(row.as(gtk.ListBoxRow), 0);
        }
        if (makePrefixBox(box, store, node, node_idx)) |prefix| adw.ActionRow.addPrefix(row, prefix);
        if (node.badge) |b| {
            const lbl = gtk.Label.new(b);
            gtk.Widget.addCssClass(lbl.as(gtk.Widget), "dimmed");
            gtk.Widget.addCssClass(lbl.as(gtk.Widget), "numeric");
            gtk.Widget.setValign(lbl.as(gtk.Widget), .center);
            adw.ActionRow.addSuffix(row, lbl.as(gtk.Widget));
        }
        // Trailing action buttons, chained via "nd-next-action" so the hover
        // controller can toggle them without walking AdwActionRow internals.
        var prev_btn: ?*gtk.Button = null;
        var first_btn: ?*gtk.Button = null;
        for (node.action_ids.items) |aid| {
            const action_idx = store.action_by_id.get(aid) orelse continue;
            const btn = makeActionButton(box, node_idx, action_idx, store.actions.items[action_idx]);
            adw.ActionRow.addSuffix(row, btn.as(gtk.Widget));
            if (first_btn == null) first_btn = btn;
            if (prev_btn) |p| gobject.Object.setData(asObject(p), "nd-next-action", @ptrCast(btn));
            prev_btn = btn;
        }
        if (first_btn) |fb| {
            gobject.Object.setData(asObject(row), "nd-first-action", @ptrCast(fb));
            if (store.hover_actions) {
                setActionsVisible(row.as(gtk.Widget), false);
                const ctrl = gtk.EventControllerMotion.new();
                _ = gtk.EventControllerMotion.signals.enter.connect(ctrl, ?*anyopaque, &cbRowEnter, @ptrCast(row), .{});
                _ = gtk.EventControllerMotion.signals.leave.connect(ctrl, ?*anyopaque, &cbRowLeave, @ptrCast(row), .{});
                gtk.Widget.addController(row.as(gtk.Widget), ctrl.as(gtk.EventController));
            }
        }
        gtk.Widget.setMarginStart(row.as(gtk.Widget), indent);
        setIdx(asObject(row), "nd-node-idx", node_idx);
        gtk.ListBox.append(box, row.as(gtk.Widget));
    }

    if (node.expanded) {
        for (node.children.items) |child_idx| appendRow(box, store, child_idx, depth + 1);
    }
}

/// Opacity, never visibility: an unmapped suffix hands its width back to the
/// row, so the title and badge slide right the moment the pointer arrives.
/// Sensitivity is what keeps a transparent button out of the pointer and
/// focus paths — a zero-opacity button that still takes clicks is worse than
/// the shift it was hiding.
fn setActionsVisible(row_widget: *gtk.Widget, visible: bool) void {
    var cursor = gobject.Object.getData(asObject(row_widget), "nd-first-action");
    while (cursor) |raw| {
        const btn: *gtk.Button = @ptrCast(@alignCast(raw));
        gtk.Widget.setOpacity(btn.as(gtk.Widget), if (visible) 1.0 else 0.0);
        gtk.Widget.setSensitive(btn.as(gtk.Widget), @intFromBool(visible));
        cursor = gobject.Object.getData(asObject(btn), "nd-next-action");
    }
}

/// Full rebuild under sync_depth (removeAll fires row-selected with a NULL
/// row the instant it destroys the selected row — same trap SourceList's
/// generated arm guards with blockEcho).
fn rebuildRows(box: *gtk.ListBox, store: *Store, select_id: ?[]const u8) void {
    sync_depth += 1;
    defer sync_depth -= 1;
    gtk.ListBox.removeAll(box);
    for (store.roots.items) |root_idx| appendRow(box, store, root_idx, 0);
    if (select_id) |sid| {
        if (sid.len > 0) {
            if (rowForId(box, store, sid)) |row| gtk.ListBox.selectRow(box, row);
        }
    }
}

fn rowForId(box: *gtk.ListBox, store: *Store, id: []const u8) ?*gtk.ListBoxRow {
    var i: c_int = 0;
    while (gtk.ListBox.getRowAtIndex(box, i)) |row| : (i += 1) {
        if (getIdx(asObject(row), "nd-node-idx")) |idx| {
            if (std.mem.eql(u8, store.nodes.items[idx].id, id)) return row;
        }
    }
    return null;
}

fn selectedNodeId(box: *gtk.ListBox, store: *Store) ?[]const u8 {
    const row = gtk.ListBox.getSelectedRow(box) orelse return null;
    const idx = getIdx(asObject(row), "nd-node-idx") orelse return null;
    return store.nodes.items[idx].id;
}

// ---- create / applyProps ---------------------------------------------------

pub fn create(props: ?std.json.Value, dupeZ: *const fn ([]const u8) [:0]const u8) *gtk.Widget {
    _ = dupeZ; // node strings live in the page-heap Store, not the backend arena
    const box = gtk.ListBox.new();
    gtk.ListBox.setSelectionMode(box, .browse);
    gtk.ListBox.setActivateOnSingleClick(box, 0);
    gtk.Widget.addCssClass(box.as(gtk.Widget), "navigation-sidebar");
    // Automation disambiguation flag: backend.zig's widgetKind resolves any
    // ScrolledWindow>GtkListBox pair to "SourceList" without it.
    gobject.Object.setData(asObject(box), "nd-sourcetree", @ptrFromInt(1));

    const store = alloc.create(Store) catch @panic("OOM creating SourceTree store");
    store.* = .{};
    if (propStr(props, "actionVisibility")) |v| store.hover_actions = !std.mem.eql(u8, v, "always");
    if (propInt(props, "indentationPerLevel")) |i| store.indent = i;
    if (propArray(props, "actions")) |arr| parseActions(store, arr);
    if (propArray(props, "nodes")) |arr| parseNodes(store, arr);
    stores.put(alloc, @intFromPtr(box), store) catch {};
    _ = gtk.Widget.signals.destroy.connect(box.as(gtk.Widget), ?*anyopaque, &cbBoxDestroyed, null, .{});

    rebuildRows(box, store, propStr(props, "selectedId"));

    const sw = gtk.ScrolledWindow.new();
    gtk.ScrolledWindow.setChild(sw, box.as(gtk.Widget));
    ndempty.register(sw, box.as(gtk.Widget));
    ndempty.configure(sw, propStr(props, "emptyIconName"), propStr(props, "emptyTitle"), propStr(props, "emptyDescription"));
    ndempty.update(sw, store.nodes.items.len == 0);
    _ = gtk.Widget.signals.map.connect(sw.as(gtk.Widget), ?*anyopaque, &cbSidebarMapped, null, .{});
    return sw.as(gtk.Widget);
}

/// `.navigation-sidebar` is background-less by design: libadwaita expects the
/// PANE around it to supply the fill, so a standalone <sourcetree> paints its
/// rows onto the bare window. `.sidebar-pane` is that fill (defined in the
/// 1.7 through 1.9 stylesheets, though absent from the public style-class
/// list), and a split view already shades its own sidebar slot, so skip it
/// there. Deferred to `map` because at create() the widget has no parent yet.
fn cbSidebarMapped(w: *gtk.Widget, _: ?*anyopaque) callconv(.c) void {
    if (inSidebarSlot(w)) {
        gtk.Widget.removeCssClass(w, "sidebar-pane");
    } else {
        gtk.Widget.addCssClass(w, "sidebar-pane");
    }
}

/// True when `w` sits inside the sidebar slot of any AdwOverlaySplitView
/// ancestor (a nested list-pane split counts too). Slot membership is checked
/// against the split view's LOGICAL sidebar child, because adw keeps internal
/// widgets between slot children and the split view itself. Private twin of
/// tabs.zig's identical check; neither module owns the other's file.
fn inSidebarSlot(w: *gtk.Widget) bool {
    var anc: ?*gtk.Widget = gtk.Widget.getAncestor(w, adw.OverlaySplitView.getGObjectType());
    while (anc) |a| {
        const sv: *adw.OverlaySplitView = @ptrCast(@alignCast(a));
        if (adw.OverlaySplitView.getSidebar(sv)) |sb| {
            if (w == sb or gtk.Widget.isAncestor(w, sb) != 0) return true;
        }
        anc = if (gtk.Widget.getParent(a)) |p| gtk.Widget.getAncestor(p, adw.OverlaySplitView.getGObjectType()) else null;
    }
    return false;
}

pub fn applyProps(widget: *gtk.Widget, props: ?std.json.Value, dupeZ: *const fn ([]const u8) [:0]const u8) void {
    _ = dupeZ;
    const box = innerListBox(widget) orelse return;
    const store = stores.get(@intFromPtr(box)) orelse return;

    ndempty.configure(@ptrCast(@alignCast(widget)), propStr(props, "emptyIconName"), propStr(props, "emptyTitle"), propStr(props, "emptyDescription"));
    const nodes_arr = propArray(props, "nodes");
    const actions_arr = propArray(props, "actions");
    if (nodes_arr != null or actions_arr != null) {
        // Selection preservation: an explicit selectedId in the same update
        // wins; otherwise re-select whatever node id was selected before.
        var prev_buf: [256]u8 = undefined;
        var prev_id: ?[]const u8 = null;
        if (selectedNodeId(box, store)) |cur| {
            if (cur.len <= prev_buf.len) {
                @memcpy(prev_buf[0..cur.len], cur);
                prev_id = prev_buf[0..cur.len];
            }
        }
        if (actions_arr) |arr| {
            store.deinitActions();
            parseActions(store, arr);
        }
        if (nodes_arr) |arr| {
            store.deinitNodes();
            parseNodes(store, arr);
        }
        rebuildRows(box, store, propStr(props, "selectedId") orelse prev_id);
        ndempty.update(@ptrCast(@alignCast(widget)), store.nodes.items.len == 0);
    }
    if (propStr(props, "selectedId")) |sid| {
        const cur = selectedNodeId(box, store);
        const same = if (cur) |c| std.mem.eql(u8, c, sid) else sid.len == 0;
        if (!same) {
            sync_depth += 1;
            defer sync_depth -= 1;
            if (sid.len == 0) {
                // unselectAll is a documented no-op in browse mode (which
                // stays: it is the native sidebar behavior). selectRow(null)
                // reaches the internal unselect that works in every mode.
                gtk.ListBox.selectRow(box, null);
            } else if (rowForId(box, store, sid)) |row| {
                gtk.ListBox.selectRow(box, row);
            }
        }
    }
}

// ---- events ----------------------------------------------------------------

pub fn connectEvents(widget: *gtk.Widget, node_id: u32, emit_fn: EmitFn) void {
    emit = emit_fn;
    const box = innerListBox(widget) orelse return;
    node_ids.put(alloc, @intFromPtr(box), node_id) catch {};
    _ = gobject.signalConnectData(asObject(box), "row-selected", @ptrCast(&cbRowSelected), @ptrFromInt(@as(usize, node_id)), null, .{});
    _ = gobject.signalConnectData(asObject(box), "row-activated", @ptrCast(&cbRowActivated), @ptrFromInt(@as(usize, node_id)), null, .{});
}

fn emitNode(node_id: u32, name: []const u8, tree_node_id: ?[]const u8) void {
    const f = emit orelse return;
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    const value: std.json.Value = if (tree_node_id) |i| .{ .string = i } else .null;
    payload.put(alloc, "nodeId", value) catch {};
    f(node_id, name, .{ .data = .{ .object = payload } });
}

fn emitAction(node_id: u32, tree_node_id: []const u8, action_id: []const u8) void {
    const f = emit orelse return;
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    payload.put(alloc, "nodeId", .{ .string = tree_node_id }) catch {};
    payload.put(alloc, "actionId", .{ .string = action_id }) catch {};
    f(node_id, "actionClicked", .{ .data = .{ .object = payload } });
}

// "row-selected" passes (box, nullable row, user_data).
fn cbRowSelected(obj: *gobject.Object, row: ?*gtk.ListBoxRow, data: ?*anyopaque) callconv(.c) void {
    if (sync_depth > 0) return;
    const node_id: u32 = @intCast(@intFromPtr(data));
    const box: *gtk.ListBox = @ptrCast(@alignCast(obj));
    const store = stores.get(@intFromPtr(box)) orelse return;
    if (row) |r| {
        const idx = getIdx(asObject(r), "nd-node-idx") orelse return;
        emitNode(node_id, "selectionChanged", store.nodes.items[idx].id);
    } else {
        emitNode(node_id, "selectionChanged", null);
    }
}

// "row-activated" passes (box, row, user_data).
fn cbRowActivated(obj: *gobject.Object, row: *gtk.ListBoxRow, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const box: *gtk.ListBox = @ptrCast(@alignCast(obj));
    const store = stores.get(@intFromPtr(box)) orelse return;
    const idx = getIdx(asObject(row), "nd-node-idx") orelse return;
    emitNode(node_id, "rowActivated", store.nodes.items[idx].id);
}

// The row's own click gesture selects and activates under the arrow; claiming
// the sequence on press keeps a disclosure click to expansion alone, which is
// what the GtkButton this replaced used to do for free.
fn cbDisclosurePressed(gesture: *gtk.GestureClick, _: c_int, _: f64, _: f64, _: *gtk.ListBox) callconv(.c) void {
    _ = gtk.Gesture.setState(gesture.as(gtk.Gesture), .claimed);
}

fn cbDisclosureReleased(gesture: *gtk.GestureClick, _: c_int, _: f64, _: f64, box: *gtk.ListBox) callconv(.c) void {
    const store = stores.get(@intFromPtr(box)) orelse return;
    const nd_id = node_ids.get(@intFromPtr(box)) orelse return;
    const idx = getIdx(asObject(gesture), "nd-node-idx") orelse return;
    const node = store.nodes.items[idx];
    // Expansion is app-controlled: emit only, never flip rows locally. The app
    // re-renders with the flipped `expanded` flag and the rebuild lands it.
    emitNode(nd_id, if (node.expanded) "nodeCollapsed" else "nodeExpanded", node.id);
}

fn cbActionClicked(obj: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const box: *gtk.ListBox = @ptrCast(@alignCast(data orelse return));
    const store = stores.get(@intFromPtr(box)) orelse return;
    const nd_id = node_ids.get(@intFromPtr(box)) orelse return;
    const node_idx = getIdx(obj, "nd-node-idx") orelse return;
    const action_idx = getIdx(obj, "nd-action-idx") orelse return;
    emitAction(nd_id, store.nodes.items[node_idx].id, store.actions.items[action_idx].id);
}

fn cbRowEnter(_: *gtk.EventControllerMotion, _: f64, _: f64, data: ?*anyopaque) callconv(.c) void {
    const row: *gtk.Widget = @ptrCast(@alignCast(data orelse return));
    setActionsVisible(row, true);
}

fn cbRowLeave(_: *gtk.EventControllerMotion, data: ?*anyopaque) callconv(.c) void {
    const row: *gtk.Widget = @ptrCast(@alignCast(data orelse return));
    setActionsVisible(row, false);
}

fn cbBoxDestroyed(w: *gtk.Widget, _: ?*anyopaque) callconv(.c) void {
    const key = @intFromPtr(w);
    if (stores.get(key)) |store| {
        store.deinit();
        alloc.destroy(store);
    }
    _ = stores.remove(key);
    _ = node_ids.remove(key);
}

// ---- automation (backend.zig semantic arms) --------------------------------

/// a11y value probe: the selected node's id, or null.
pub fn selectedIdOf(widget: *gtk.Widget) ?[]const u8 {
    const box = innerListBox(widget) orelse return null;
    const store = stores.get(@intFromPtr(box)) orelse return null;
    return selectedNodeId(box, store);
}

/// setValue {value: "<nodeId>"|""}: selects by node id, emitting exactly one
/// selectionChanged {nodeId} for the landed selection. "" deselects.
///
/// The emit is explicit rather than left to the box's own row-selected,
/// because GtkListBox raises that signal only when the selection MOVES, and
/// the box can already hold the requested row while the app does not: a
/// controlled `selectedId` frame computed before the last selection lands
/// after it, and applyProps re-applies it under the echo block, by design.
/// Leaving the event to the signal made one setValue emit zero events from
/// that state (the box moved nothing, so the app never learned it now
/// agreed), which is the sourcetree drive's intermittent selection-leg hang. Driving
/// the change under the block and emitting here makes the count one per call
/// whatever the box started from.
pub fn semanticSelect(widget: *gtk.Widget, id: []const u8) bool {
    const box = innerListBox(widget) orelse return false;
    const store = stores.get(@intFromPtr(box)) orelse return false;
    if (id.len == 0) {
        {
            // Same browse-mode constraint as applyProps: selectRow(null) is the
            // clear path that actually works; unselectAll no-ops in browse.
            sync_depth += 1;
            defer sync_depth -= 1;
            gtk.ListBox.selectRow(box, null);
        }
        emitSelection(box, null);
        return true;
    }
    const row = rowForId(box, store, id) orelse return false;
    if (gtk.ListBoxRow.getSelectable(row) == 0) return false;
    {
        sync_depth += 1;
        defer sync_depth -= 1;
        gtk.ListBox.selectRow(box, row);
    }
    emitSelection(box, id);
    return true;
}

/// selectionChanged for a selection this module drove itself, addressed to the
/// box's own ND node id. Silent when the tree carries no events yet.
fn emitSelection(box: *gtk.ListBox, tree_node_id: ?[]const u8) void {
    const nd_id = node_ids.get(@intFromPtr(box)) orelse return;
    emitNode(nd_id, "selectionChanged", tree_node_id);
}

/// rowAction {actionId, testId?}: dispatches a row's trailing action as if
/// its button were clicked, emitting actionClicked {nodeId, actionId}. testId
/// picks the node by its per-node testID; absent, the selected row is the target.
/// The row must be realized (visible under the current expansion, like a
/// user-reachable button) and the action declared on that node.
pub fn semanticRowAction(widget: *gtk.Widget, action_id: []const u8, test_id: ?[]const u8) bool {
    const box = innerListBox(widget) orelse return false;
    const store = stores.get(@intFromPtr(box)) orelse return false;
    const nd_id = node_ids.get(@intFromPtr(box)) orelse return false;
    const node: *Node = blk: {
        if (test_id) |tid| {
            for (store.nodes.items) |*n| {
                if (n.test_id) |nt| {
                    if (std.mem.eql(u8, nt, tid)) break :blk n;
                }
            }
            return false;
        }
        const row = gtk.ListBox.getSelectedRow(box) orelse return false;
        const idx = getIdx(asObject(row), "nd-node-idx") orelse return false;
        break :blk &store.nodes.items[idx];
    };
    if (rowForId(box, store, node.id) == null) return false;
    const action_idx = store.action_by_id.get(action_id) orelse return false;
    var declared = false;
    for (node.action_ids.items) |aid| {
        if (std.mem.eql(u8, aid, action_id)) {
            declared = true;
            break;
        }
    }
    if (!declared) return false;
    emitAction(nd_id, node.id, store.actions.items[action_idx].id);
    return true;
}

/// click — activates the currently-selected row (or the first selectable
/// row when nothing is selected), emitting rowActivated through the box's
/// own signal so the payload shape stays in one place.
pub fn semanticActivate(widget: *gtk.Widget) bool {
    const box = innerListBox(widget) orelse return false;
    const row = gtk.ListBox.getSelectedRow(box) orelse blk: {
        var i: c_int = 0;
        while (gtk.ListBox.getRowAtIndex(box, i)) |r| : (i += 1) {
            if (gtk.ListBoxRow.getSelectable(r) != 0) break :blk r;
        }
        return false;
    };
    gobject.signalEmitByName(asObject(box), "row-activated", row);
    return true;
}
