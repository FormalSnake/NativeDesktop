// GtkColumnView surface for the <table> widget: columns/rows objectList
// props, none/single/multiple selection, header-click sort *indication*,
// user column resizing and header-drag reordering.
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
//
// Geometry: columns are resizable, and a user drag writes the column's
// fixed-width, so columnsResized rides notify::fixed-width (debounced, since
// the property tracks the pointer). Header reordering moves entries in the
// view's own columns list model, so columnsReordered rides its items-changed.
// Both are silenced while rebuildColumns runs — a data-driven rebuild is not
// a user gesture.
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
/// GtkSelectionModel ptr -> its selection-changed handler id (local echo
/// suppression — the generated blockEcho map is file-private to widgets.zig).
var select_handlers: std.AutoHashMapUnmanaged(usize, c_ulong) = .empty;
/// True for the duration of rebuildColumns: the column geometry signals below
/// fire for our own writes as readily as for a user drag.
var rebuilding = false;

/// The GtkStringList behind whichever selection model the mode picked, and
/// the node id, both stashed on the tracked GtkScrolledWindow so applyProps
/// never has to know which of the three models is in play.
const ROWS_KEY = "nd-table-rows";
const NODE_ID_KEY = "nd-table-node-id";
const RESIZE_TIMEOUT_KEY = "nd-table-resize-timeout";

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

fn viewSelection(view: *gtk.ColumnView) ?*gtk.SelectionModel {
    return gtk.ColumnView.getModel(view);
}

fn rowModel(widget: *gtk.Widget) ?*gtk.StringList {
    const raw = gobject.Object.getData(asObject(widget), ROWS_KEY) orelse return null;
    return @ptrCast(@alignCast(raw));
}

fn asObject(ptr: anytype) *gobject.Object {
    return @ptrCast(@alignCast(ptr));
}

fn nodeIdOf(view: *gtk.ColumnView) ?u32 {
    const raw = gobject.Object.getData(asObject(view), NODE_ID_KEY) orelse return null;
    return @intCast(@intFromPtr(raw));
}

/// The `selectionMode` prop, create-only: GTK picks the selection model at
/// construction and swapping it later would strand the selection handler.
fn makeSelection(mode: []const u8, rows: *gtk.StringList) *gtk.SelectionModel {
    if (std.mem.eql(u8, mode, "none")) {
        return @ptrCast(@alignCast(gtk.NoSelection.new(rows.as(gio.ListModel))));
    }
    if (std.mem.eql(u8, mode, "multiple")) {
        return @ptrCast(@alignCast(gtk.MultiSelection.new(rows.as(gio.ListModel))));
    }
    const single = gtk.SingleSelection.new(rows.as(gio.ListModel));
    gtk.SingleSelection.setAutoselect(single, 0);
    gtk.SingleSelection.setCanUnselect(single, 1);
    // gtk_single_selection_new() auto-selects position 0 as part of
    // constructing the selection over the model, before autoselect above
    // takes effect — clear it explicitly so selectedIndex: -1 (the default)
    // means no row is visibly selected, not a phantom row-0 highlight.
    gtk.SingleSelection.setSelected(single, std.math.maxInt(c_uint));
    return @ptrCast(@alignCast(single));
}

/// Selection as row indexes, ascending. One shape for all three modes, so the
/// wire payload never depends on which model is underneath.
fn selectedRows(model: *gtk.SelectionModel, out: *std.ArrayList(i64)) void {
    const bits = gtk.SelectionModel.getSelection(model);
    defer gtk.Bitset.unref(bits);
    const n = gtk.Bitset.getSize(bits);
    var i: u64 = 0;
    while (i < n) : (i += 1) {
        out.append(std.heap.page_allocator, @intCast(gtk.Bitset.getNth(bits, @intCast(i)))) catch return;
    }
}

fn applySelection(model: *gtk.SelectionModel, rows: *gtk.StringList, indexes: []const i64) void {
    const count = gio.ListModel.getNItems(rows.as(gio.ListModel));
    const selected = gtk.Bitset.newEmpty();
    defer gtk.Bitset.unref(selected);
    for (indexes) |idx| {
        if (idx >= 0 and idx < count) _ = gtk.Bitset.add(selected, @intCast(idx));
    }
    const mask = gtk.Bitset.newRange(0, count);
    defer gtk.Bitset.unref(mask);
    _ = gtk.SelectionModel.setSelection(model, selected, mask);
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
    rebuilding = true;
    defer rebuilding = false;
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
        // Header-divider dragging is GTK's own; the app hears the result and
        // decides whether to persist it.
        gtk.ColumnViewColumn.setResizable(column, 1);
        _ = gobject.signalConnectData(@ptrCast(@alignCast(column)), "notify::fixed-width", @ptrCast(&cbColumnWidth), view, null, .{});
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
    const selection = makeSelection(propStr(props, "selectionMode") orelse "single", model); // transfer-full: selection owns model

    const view = gtk.ColumnView.new(selection); // transfer-full: view owns selection
    gtk.Widget.addCssClass(view.as(gtk.Widget), "data-table");
    gtk.ColumnView.setShowRowSeparators(view, @intFromBool(propBool(props, "showRowSeparators") orelse true));
    gtk.ColumnView.setReorderable(view, @intFromBool(propBool(props, "columnsReorderable") orelse false));
    if (propArray(props, "columns")) |cols| rebuildColumns(view, cols, dupeZ);

    const sel_idx = propInt(props, "selectedIndex") orelse -1;
    if (sel_idx >= 0) applySelection(selection, model, &[_]i64{sel_idx});
    if (propArray(props, "selectedIndexes")) |arr| {
        var rows: std.ArrayList(i64) = .empty;
        defer rows.deinit(std.heap.page_allocator);
        for (arr.items) |it| {
            if (it == .integer) rows.append(std.heap.page_allocator, it.integer) catch {};
        }
        applySelection(selection, model, rows.items);
    }

    const sw = gtk.ScrolledWindow.new();
    gtk.ScrolledWindow.setChild(sw, view.as(gtk.Widget));
    gobject.Object.setData(asObject(sw), ROWS_KEY, @ptrCast(model));
    ndempty.register(sw, view.as(gtk.Widget));
    ndempty.configure(sw, propStr(props, "emptyIconName"), propStr(props, "emptyTitle"), propStr(props, "emptyDescription"));
    const n_rows: usize = if (propArray(props, "rows")) |arr| arr.items.len else 0;
    ndempty.update(sw, n_rows == 0);
    return sw.as(gtk.Widget);
}

fn blockSelect(selection: *gtk.SelectionModel) void {
    if (select_handlers.get(@intFromPtr(selection))) |hid| gobject.signalHandlerBlock(asObject(selection), hid);
}

fn unblockSelect(selection: *gtk.SelectionModel) void {
    if (select_handlers.get(@intFromPtr(selection))) |hid| gobject.signalHandlerUnblock(asObject(selection), hid);
}

pub fn applyProps(widget: *gtk.Widget, props: ?std.json.Value, dupeZ: *const fn ([]const u8) [:0]const u8) void {
    const view = innerView(widget) orelse return;
    const selection = viewSelection(view) orelse return;
    const model = rowModel(widget) orelse return;

    ndempty.configure(@ptrCast(@alignCast(widget)), propStr(props, "emptyIconName"), propStr(props, "emptyTitle"), propStr(props, "emptyDescription"));
    if (propArray(props, "columns")) |cols| rebuildColumns(view, cols, dupeZ);
    if (propBool(props, "columnsReorderable")) |r| gtk.ColumnView.setReorderable(view, @intFromBool(r));
    if (propArray(props, "rows")) |rows| {
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
    // `selectedIndexes` wins when both land in one update: the app that sends
    // it is the one driving a multiple selection.
    if (propArray(props, "selectedIndexes")) |arr| {
        var rows: std.ArrayList(i64) = .empty;
        defer rows.deinit(std.heap.page_allocator);
        for (arr.items) |it| {
            if (it == .integer) rows.append(std.heap.page_allocator, it.integer) catch {};
        }
        blockSelect(selection);
        applySelection(selection, model, rows.items);
        unblockSelect(selection);
    } else if (propInt(props, "selectedIndex")) |idx| {
        var cur: std.ArrayList(i64) = .empty;
        defer cur.deinit(std.heap.page_allocator);
        selectedRows(selection, &cur);
        const same = cur.items.len == 1 and cur.items[0] == idx;
        const cleared = cur.items.len == 0 and idx < 0;
        if (!same and !cleared) {
            blockSelect(selection);
            if (idx >= 0) applySelection(selection, model, &[_]i64{idx}) else applySelection(selection, model, &[_]i64{});
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
    gobject.Object.setData(asObject(view), NODE_ID_KEY, data);

    if (viewSelection(view)) |selection| {
        const hid = gobject.signalConnectData(asObject(selection), "selection-changed", @ptrCast(&cbSelected), data, null, .{});
        select_handlers.put(std.heap.page_allocator, @intFromPtr(selection), hid) catch {};
    }
    _ = gobject.signalConnectData(@ptrCast(@alignCast(view)), "activate", @ptrCast(&cbActivate), data, null, .{});
    // The view's GtkColumnViewSorter is one stable instance for the view's
    // lifetime; header clicks emit its "changed".
    if (gtk.ColumnView.getSorter(view)) |sorter| {
        _ = gobject.signalConnectData(@ptrCast(@alignCast(sorter)), "changed", @ptrCast(&cbSorterChanged), data, null, .{});
    }
    // A header drag reorders the view's own columns list model in place.
    _ = gobject.signalConnectData(asObject(gtk.ColumnView.getColumns(view)), "items-changed", @ptrCast(&cbColumnsMoved), view, null, .{});
}

// GtkSelectionModel "selection-changed" passes (model, position, n_items,
// user_data). The payload carries the whole selection AND its first row in
// `index`, so a single-select app reading e.index is untouched by the
// multiple mode existing.
fn cbSelected(obj: *gobject.Object, _: c_uint, _: c_uint, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const f = emit orelse return;
    const selection: *gtk.SelectionModel = @ptrCast(@alignCast(obj));
    var rows: std.ArrayList(i64) = .empty;
    defer rows.deinit(std.heap.page_allocator);
    selectedRows(selection, &rows);

    var list: std.json.Array = .init(std.heap.page_allocator);
    defer list.deinit();
    for (rows.items) |idx| list.append(.{ .integer = idx }) catch {};
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(std.heap.page_allocator);
    payload.put(std.heap.page_allocator, "indexes", .{ .array = list }) catch {};

    const first: i64 = if (rows.items.len > 0) rows.items[0] else -1;
    f(node_id, "selectionChanged", .{ .index = first, .data = .{ .object = payload } });
}

/// A column's fixed-width tracks the pointer through a divider drag, so the
/// report waits for it to settle (the Paned debounce, one timeout per view).
fn cbColumnWidth(_: *gobject.Object, _: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
    if (rebuilding) return;
    const view: *gtk.ColumnView = @ptrCast(@alignCast(data.?));
    if (nodeIdOf(view) == null) return; // not connected yet: a create-time width write
    const obj = asObject(view);
    if (gobject.Object.getData(obj, RESIZE_TIMEOUT_KEY)) |raw| {
        _ = glib.Source.remove(@intCast(@intFromPtr(raw)));
    }
    const id = glib.timeoutAdd(120, &cbColumnWidthSettled, view);
    gobject.Object.setData(obj, RESIZE_TIMEOUT_KEY, @ptrFromInt(@as(usize, id)));
}

fn cbColumnWidthSettled(data: ?*anyopaque) callconv(.c) c_int {
    const view: *gtk.ColumnView = @ptrCast(@alignCast(data.?));
    gobject.Object.setData(asObject(view), RESIZE_TIMEOUT_KEY, null);
    const node_id = nodeIdOf(view) orelse return 0;
    const f = emit orelse return 0;
    const alloc = std.heap.page_allocator;
    const columns = gtk.ColumnView.getColumns(view);
    var list: std.json.Array = .init(alloc);
    defer list.deinit();
    const n = gio.ListModel.getNItems(columns);
    var i: c_uint = 0;
    while (i < n) : (i += 1) {
        const item = gio.ListModel.getObject(columns, i) orelse continue;
        defer gobject.Object.unref(item);
        const column: *gtk.ColumnViewColumn = @ptrCast(@alignCast(item));
        const id = gtk.ColumnViewColumn.getId(column) orelse continue;
        var entry: std.json.ObjectMap = .empty;
        entry.put(alloc, "id", .{ .string = std.mem.span(id) }) catch {};
        // fixed-width is what a divider drag writes; -1 is still "natural",
        // which is the honest answer for a column nobody has resized.
        entry.put(alloc, "width", .{ .integer = gtk.ColumnViewColumn.getFixedWidth(column) }) catch {};
        list.append(.{ .object = entry }) catch {};
    }
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    payload.put(alloc, "columns", .{ .array = list }) catch {};
    f(node_id, "columnsResized", .{ .data = .{ .object = payload } });
    return 0; // G_SOURCE_REMOVE
}

// GListModel "items-changed" passes (model, position, removed, added, data).
fn cbColumnsMoved(_: *gobject.Object, _: c_uint, _: c_uint, _: c_uint, data: ?*anyopaque) callconv(.c) void {
    if (rebuilding) return;
    const view: *gtk.ColumnView = @ptrCast(@alignCast(data.?));
    const node_id = nodeIdOf(view) orelse return;
    const f = emit orelse return;
    const alloc = std.heap.page_allocator;
    const columns = gtk.ColumnView.getColumns(view);
    var ids: std.json.Array = .init(alloc);
    defer ids.deinit();
    const n = gio.ListModel.getNItems(columns);
    var i: c_uint = 0;
    while (i < n) : (i += 1) {
        const item = gio.ListModel.getObject(columns, i) orelse continue;
        defer gobject.Object.unref(item);
        const id = gtk.ColumnViewColumn.getId(@ptrCast(@alignCast(item))) orelse continue;
        ids.append(.{ .string = std.mem.span(id) }) catch {};
    }
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    payload.put(alloc, "columnIds", .{ .array = ids }) catch {};
    f(node_id, "columnsReordered", .{ .data = .{ .object = payload } });
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
