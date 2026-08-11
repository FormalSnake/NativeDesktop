// GtkColumnView surface for the <table> widget: columns/rows objectList
// props, single selection, header-click sort *indication*.
//
// Data model: each TableRow's cells are joined with the ASCII unit separator
// (0x1F) into ONE GtkStringList item, so the model itself carries the cell
// data (no side table to keep in sync with recycling); each column's bind
// callback splits the row string and takes its own cell index.
//
// Sorting: JS owns row order (only the app knows if a column is numeric/
// date/lexical). Each sortable column gets a trivial GtkCustomSorter purely so
// its header is clickable; the model is NEVER wrapped in GtkSortListModel.
// The view's own GtkColumnViewSorter (view-side bookkeeping of clicked column
// + direction) emits "changed", which we forward as sortChanged
// { columnId, direction } — gtk_column_view_sort_by_column would be the
// programmatic header-UI setter if a controlled sort prop lands later.
const std = @import("std");
const gtk = @import("gtk");
const gio = @import("gio");
const glib = @import("glib");
const gobject = @import("gobject");
const protocol = @import("../protocol.zig");
const ndempty = @import("emptystate.zig");

pub const EmitFn = *const fn (node_id: u32, name: []const u8, payload: protocol.EventPayload) void;

const CELL_SEP: u8 = 0x1F; // ASCII unit separator — never present in real cell text

var emit: ?EmitFn = null;
/// GtkSingleSelection ptr -> its notify::selected handler id (local echo
/// suppression — the generated blockEcho map is file-private to widgets.zig).
var select_handlers: std.AutoHashMapUnmanaged(usize, c_ulong) = .empty;

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

fn propBool(props: ?std.json.Value, key: []const u8) ?bool {
    const v = props orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .bool => field.bool,
        else => null,
    };
}

fn objStr(obj: std.json.ObjectMap, key: []const u8) ?[]const u8 {
    return switch (obj.get(key) orelse return null) {
        .string => |s| s,
        else => null,
    };
}

/// The tracked handle is the GtkScrolledWindow; GtkColumnView implements
/// GtkScrollable, so it is a direct child (never viewport-wrapped). The
/// empty-state registry wins while an AdwStatusPage is swapped in.
fn innerView(widget: *gtk.Widget) ?*gtk.ColumnView {
    const sw: *gtk.ScrolledWindow = @ptrCast(@alignCast(widget));
    if (ndempty.innerOf(sw)) |inner| return @ptrCast(@alignCast(inner));
    const child = gtk.ScrolledWindow.getChild(sw) orelse return null;
    return @ptrCast(@alignCast(child));
}

fn viewSelection(view: *gtk.ColumnView) ?*gtk.SingleSelection {
    const model = gtk.ColumnView.getModel(view) orelse return null;
    return @ptrCast(@alignCast(model));
}

// ---- row encoding ----------------------------------------------------------

const RowStrv = struct {
    buf: []?[*:0]const u8,
    strings: std.ArrayList([:0]u8),

    fn deinit(self: *RowStrv) void {
        for (self.strings.items) |s| std.heap.page_allocator.free(s);
        self.strings.deinit(std.heap.page_allocator);
        std.heap.page_allocator.free(self.buf);
    }
};

/// Builds a NULL-terminated strv of CELL_SEP-joined rows for bulk
/// GtkStringList construction/splice (both deep-copy, so the buffer is freed
/// right after the call — unlike widgets.zig's arena-backed buildStrv).
fn buildRowStrv(arr: ?std.json.Array) RowStrv {
    const alloc = std.heap.page_allocator;
    const items = arr orelse std.json.Array.init(alloc);
    const buf = alloc.alloc(?[*:0]const u8, items.items.len + 1) catch @panic("OOM building Table rows");
    var strings: std.ArrayList([:0]u8) = .empty;
    var n: usize = 0;
    for (items.items) |it| {
        if (it != .object) continue;
        const cells = it.object.get("cells") orelse continue;
        if (cells != .array) continue;
        var joined: std.ArrayList(u8) = .empty;
        defer joined.deinit(alloc);
        var first = true;
        for (cells.array.items) |cell| {
            if (!first) joined.append(alloc, CELL_SEP) catch {};
            first = false;
            if (cell == .string) joined.appendSlice(alloc, cell.string) catch {};
        }
        const z = alloc.dupeZ(u8, joined.items) catch continue;
        strings.append(alloc, z) catch {
            alloc.free(z);
            continue;
        };
        buf[n] = z.ptr;
        n += 1;
    }
    buf[n] = null;
    return .{ .buf = buf[0 .. n + 1], .strings = strings };
}

// ---- cell factory ----------------------------------------------------------

fn cellSetup(_: *gobject.Object, list_item: *gtk.ListItem, _: ?*anyopaque) callconv(.c) void {
    const label = gtk.Label.new(null);
    gtk.Label.setXalign(label, 0.0);
    gtk.ListItem.setChild(list_item, label.as(gtk.Widget));
}

/// user_data carries the column's cell index (columns and cells share one
/// order by contract — TableRow.cells is positional).
fn cellBind(_: *gobject.Object, list_item: *gtk.ListItem, data: ?*anyopaque) callconv(.c) void {
    const col: usize = @intFromPtr(data);
    const child = gtk.ListItem.getChild(list_item) orelse return;
    const label: *gtk.Label = @ptrCast(@alignCast(child));
    const obj = gtk.ListItem.getItem(list_item) orelse return;
    const so: *gtk.StringObject = @ptrCast(@alignCast(obj));
    const row = std.mem.span(gtk.StringObject.getString(so));
    var it = std.mem.splitScalar(u8, row, CELL_SEP);
    var i: usize = 0;
    while (it.next()) |cell| : (i += 1) {
        if (i != col) continue;
        const z = std.heap.page_allocator.dupeZ(u8, cell) catch return;
        defer std.heap.page_allocator.free(z);
        gtk.Label.setText(label, z);
        return;
    }
    gtk.Label.setText(label, "");
}

/// Trivial sorter compare: never reorders anything — it exists purely so the
/// column header is clickable and the view-side GtkColumnViewSorter records
/// the clicked column + direction.
fn noopCompare(_: ?*const anyopaque, _: ?*const anyopaque, _: ?*anyopaque) callconv(.c) c_int {
    return 0; // GTK_ORDERING_EQUAL
}

// ---- columns ---------------------------------------------------------------

fn rebuildColumns(view: *gtk.ColumnView, arr: std.json.Array, dupeZ: *const fn ([]const u8) [:0]const u8) void {
    // Remove existing columns (walk the live list model back-to-front).
    const columns = gtk.ColumnView.getColumns(view);
    var n = gio.ListModel.getNItems(columns);
    while (n > 0) : (n -= 1) {
        const item = gio.ListModel.getObject(columns, n - 1) orelse continue;
        defer gobject.Object.unref(item);
        gtk.ColumnView.removeColumn(view, @ptrCast(@alignCast(item)));
    }
    var idx: usize = 0;
    for (arr.items) |col| {
        if (col != .object) continue;
        const title = objStr(col.object, "title") orelse "";
        const factory = gtk.SignalListItemFactory.new();
        _ = gobject.signalConnectData(@ptrCast(@alignCast(factory)), "setup", @ptrCast(&cellSetup), null, null, .{});
        _ = gobject.signalConnectData(@ptrCast(@alignCast(factory)), "bind", @ptrCast(&cellBind), @ptrFromInt(idx), null, .{});
        const column = gtk.ColumnViewColumn.new(dupeZ(title), factory.as(gtk.ListItemFactory)); // transfer-full: column owns factory
        if (objStr(col.object, "id")) |id| gtk.ColumnViewColumn.setId(column, dupeZ(id));
        var fixed = false;
        if (col.object.get("width")) |w| {
            if (w == .integer and w.integer > 0) {
                gtk.ColumnViewColumn.setFixedWidth(column, @intCast(w.integer));
                fixed = true;
            }
        }
        if (!fixed) gtk.ColumnViewColumn.setExpand(column, 1);
        const sorter = gtk.CustomSorter.new(&noopCompare, null, null);
        gtk.ColumnViewColumn.setSorter(column, sorter.as(gtk.Sorter));
        gobject.Object.unref(@ptrCast(@alignCast(sorter))); // column holds the ref now
        gtk.ColumnView.appendColumn(view, column);
        gobject.Object.unref(@ptrCast(@alignCast(column))); // view holds the ref now
        idx += 1;
    }
}

// ---- create / applyProps ---------------------------------------------------

pub fn create(props: ?std.json.Value, dupeZ: *const fn ([]const u8) [:0]const u8) *gtk.Widget {
    var strv = buildRowStrv(propArray(props, "rows"));
    defer strv.deinit();
    const model = gtk.StringList.new(@ptrCast(strv.buf.ptr));
    const selection = gtk.SingleSelection.new(model.as(gio.ListModel)); // transfer-full: selection owns model
    gtk.SingleSelection.setAutoselect(selection, 0);
    gtk.SingleSelection.setCanUnselect(selection, 1);
    // gtk_single_selection_new() auto-selects position 0 as part of
    // constructing the selection over the model, before autoselect above
    // takes effect — clear it explicitly so selectedIndex: -1 (the default)
    // means no row is visibly selected, not a phantom row-0 highlight.
    gtk.SingleSelection.setSelected(selection, std.math.maxInt(c_uint));
    const sel_idx = propInt(props, "selectedIndex") orelse -1;
    if (sel_idx >= 0) gtk.SingleSelection.setSelected(selection, @intCast(sel_idx));

    const view = gtk.ColumnView.new(selection.as(gtk.SelectionModel)); // transfer-full: view owns selection
    gtk.Widget.addCssClass(view.as(gtk.Widget), "data-table");
    gtk.ColumnView.setShowRowSeparators(view, @intFromBool(propBool(props, "showRowSeparators") orelse true));
    if (propArray(props, "columns")) |cols| rebuildColumns(view, cols, dupeZ);

    const sw = gtk.ScrolledWindow.new();
    gtk.ScrolledWindow.setChild(sw, view.as(gtk.Widget));
    ndempty.register(sw, view.as(gtk.Widget));
    ndempty.configure(sw, propStr(props, "emptyIconName"), propStr(props, "emptyTitle"), propStr(props, "emptyDescription"));
    const n_rows: usize = if (propArray(props, "rows")) |arr| arr.items.len else 0;
    ndempty.update(sw, n_rows == 0);
    return sw.as(gtk.Widget);
}

fn blockSelect(selection: *gtk.SingleSelection) void {
    if (select_handlers.get(@intFromPtr(selection))) |hid| gobject.signalHandlerBlock(@ptrCast(@alignCast(selection)), hid);
}

fn unblockSelect(selection: *gtk.SingleSelection) void {
    if (select_handlers.get(@intFromPtr(selection))) |hid| gobject.signalHandlerUnblock(@ptrCast(@alignCast(selection)), hid);
}

pub fn applyProps(widget: *gtk.Widget, props: ?std.json.Value, dupeZ: *const fn ([]const u8) [:0]const u8) void {
    const view = innerView(widget) orelse return;
    const selection = viewSelection(view) orelse return;

    ndempty.configure(@ptrCast(@alignCast(widget)), propStr(props, "emptyIconName"), propStr(props, "emptyTitle"), propStr(props, "emptyDescription"));
    if (propArray(props, "columns")) |cols| rebuildColumns(view, cols, dupeZ);
    if (propArray(props, "rows")) |rows| {
        const model: *gtk.StringList = @ptrCast(@alignCast(gtk.SingleSelection.getModel(selection).?));
        const n_old = gio.ListModel.getNItems(model.as(gio.ListModel));
        var strv = buildRowStrv(rows);
        defer strv.deinit();
        // A replace-all splice can move/clear the selection — that's an echo
        // of the data change, not a user selection (SourceList's removeAll
        // lesson), so it stays blocked.
        blockSelect(selection);
        gtk.StringList.splice(model, 0, n_old, @ptrCast(strv.buf.ptr));
        unblockSelect(selection);
        ndempty.update(@ptrCast(@alignCast(widget)), rows.items.len == 0);
    }
    if (propInt(props, "selectedIndex")) |idx| {
        const cur = gtk.SingleSelection.getSelected(selection);
        const cur_idx: i64 = if (cur == std.math.maxInt(c_uint)) -1 else @intCast(cur);
        if (cur_idx != idx) {
            blockSelect(selection);
            gtk.SingleSelection.setSelected(selection, if (idx >= 0) @intCast(idx) else std.math.maxInt(c_uint));
            unblockSelect(selection);
        }
    }
    if (propBool(props, "showRowSeparators")) |s| gtk.ColumnView.setShowRowSeparators(view, @intFromBool(s));
}

// ---- events ----------------------------------------------------------------

pub fn connectEvents(widget: *gtk.Widget, node_id: u32, emit_fn: EmitFn) void {
    emit = emit_fn;
    const view = innerView(widget) orelse return;
    const data: ?*anyopaque = @ptrFromInt(@as(usize, node_id));

    if (viewSelection(view)) |selection| {
        const hid = gobject.signalConnectData(@ptrCast(@alignCast(selection)), "notify::selected", @ptrCast(&cbSelected), data, null, .{});
        select_handlers.put(std.heap.page_allocator, @intFromPtr(selection), hid) catch {};
    }
    _ = gobject.signalConnectData(@ptrCast(@alignCast(view)), "activate", @ptrCast(&cbActivate), data, null, .{});
    // The view's GtkColumnViewSorter is one stable instance for the view's
    // lifetime; header clicks emit its "changed".
    if (gtk.ColumnView.getSorter(view)) |sorter| {
        _ = gobject.signalConnectData(@ptrCast(@alignCast(sorter)), "changed", @ptrCast(&cbSorterChanged), data, null, .{});
    }
}

// notify:: handlers get (object, pspec, user_data).
fn cbSelected(obj: *gobject.Object, _: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const selection: *gtk.SingleSelection = @ptrCast(@alignCast(obj));
    const sel = gtk.SingleSelection.getSelected(selection);
    const idx: i64 = if (sel == std.math.maxInt(c_uint)) -1 else @intCast(sel);
    if (emit) |f| f(node_id, "selectionChanged", .{ .index = idx });
}

// the ColumnView `activate` signal passes (view, position, user_data).
fn cbActivate(_: *gobject.Object, position: c_uint, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    if (emit) |f| f(node_id, "rowActivated", .{ .index = @intCast(position) });
}

// GtkSorter "changed" passes (sorter, GtkSorterChange, user_data).
fn cbSorterChanged(obj: *gobject.Object, _: c_int, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const f = emit orelse return;
    const sorter: *gtk.ColumnViewSorter = @ptrCast(@alignCast(obj));
    const column = gtk.ColumnViewSorter.getPrimarySortColumn(sorter) orelse return; // sort cleared: nothing to report
    const id = gtk.ColumnViewColumn.getId(column) orelse return;
    const direction: []const u8 = switch (gtk.ColumnViewSorter.getPrimarySortOrder(sorter)) {
        .descending => "descending",
        else => "ascending",
    };
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(std.heap.page_allocator);
    payload.put(std.heap.page_allocator, "columnId", .{ .string = std.mem.span(id) }) catch {};
    payload.put(std.heap.page_allocator, "direction", .{ .string = direction }) catch {};
    f(node_id, "sortChanged", .{ .data = .{ .object = payload } });
}
