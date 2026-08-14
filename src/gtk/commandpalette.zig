// AdwDialog surface for the <commandpalette> widget: a centered, dimmed,
// modal Cmd-K overlay (GtkSearchEntry over a GtkListBox of AdwActionRow
// results). CONTROLLED: the app owns `query` and `items`; the widget never
// filters or reorders — every keystroke fires queryChanged and the app feeds
// back the next result set. The tracked handle is a host-only GtkBox that
// lives in the tree only so gtk_widget_get_root resolves the window to
// present over; the AdwDialog itself is presented, never packed.
//
// Highlight is internal (Up/Down/Home/End clamp within the current results);
// onActivate carries the highlighted/clicked row's stable id, onSubmit the
// raw query text (Enter on no highlight, or Ctrl+Enter regardless) so a
// directory picker can accept a typed path that matches no listed row.
const std = @import("std");
const gtk = @import("gtk");
const gdk = @import("gdk");
const gio = @import("gio");
const glib = @import("glib");
const gobject = @import("gobject");
const adw = @import("adw");
const graphene = @import("graphene");
const protocol = @import("../protocol.zig");
const ndicons = @import("icons.zig");

pub const EmitFn = *const fn (node_id: u32, name: []const u8, payload: protocol.EventPayload) void;

const STATE_KEY = "nd-command-palette-state";

var emit: ?EmitFn = null;

const State = struct {
    node_id: u32 = 0,
    handle: *gtk.Widget,
    dialog: *adw.Dialog,
    entry: *gtk.SearchEntry,
    list: *gtk.ListBox,
    scroller: *gtk.ScrolledWindow,
    ids: std.ArrayListUnmanaged([]u8) = .empty,
    // Content signature of the currently-rendered rows. A controlled app
    // re-renders on every state change and hands back a fresh `items` array
    // each time; without this, applyProps would tear down and rebuild the
    // ListBox on every render, dropping clicks onto rows being destroyed and
    // yanking keyboard focus off the search entry. Rebuild only when the
    // signature actually changes.
    items_sig: ?[]u8 = null,
    highlight: i32 = -1,
    presented: bool = false,
    pending_open: bool = false,
    programmatic_close: bool = false,
    search_changed_hid: c_ulong = 0,
};

fn asObj(p: anytype) *gobject.Object {
    return @ptrCast(@alignCast(p));
}

fn stateOf(widget: *gtk.Widget) ?*State {
    const raw = gobject.Object.getData(asObj(widget), STATE_KEY) orelse return null;
    return @ptrCast(@alignCast(raw));
}

fn propArray(props: ?std.json.Value, key: []const u8) ?std.json.Array {
    const v = props orelse return null;
    if (v != .object) return null;
    return switch (v.object.get(key) orelse return null) {
        .array => |a| a,
        else => null,
    };
}

fn propStr(props: ?std.json.Value, key: []const u8) ?[]const u8 {
    const v = props orelse return null;
    if (v != .object) return null;
    return switch (v.object.get(key) orelse return null) {
        .string => |s| s,
        else => null,
    };
}

fn propBool(props: ?std.json.Value, key: []const u8) ?bool {
    const v = props orelse return null;
    if (v != .object) return null;
    return switch (v.object.get(key) orelse return null) {
        .bool => |b| b,
        else => null,
    };
}

fn objStr(obj: std.json.ObjectMap, key: []const u8) ?[]const u8 {
    return switch (obj.get(key) orelse return null) {
        .string => |s| s,
        else => null,
    };
}

// ---- rows ------------------------------------------------------------------

fn freeIds(state: *State) void {
    for (state.ids.items) |id| std.heap.page_allocator.free(id);
    state.ids.clearRetainingCapacity();
}

/// Content fingerprint of `arr` (id/title/subtitle/iconName of each row), used
/// to skip the destructive ListBox rebuild when a re-render hands back rows
/// that render identically. Caller owns the returned slice.
fn itemsSignature(arr: ?std.json.Array) ?[]u8 {
    var buf: std.ArrayListUnmanaged(u8) = .empty;
    errdefer buf.deinit(std.heap.page_allocator);
    const a = std.heap.page_allocator;
    if (arr) |items| {
        for (items.items) |it| {
            if (it != .object) continue;
            inline for (.{ "id", "title", "subtitle", "iconName" }) |key| {
                buf.appendSlice(a, objStr(it.object, key) orelse "") catch return null;
                buf.append(a, 0x1f) catch return null;
            }
            buf.append(a, 0x1e) catch return null;
        }
    }
    return buf.toOwnedSlice(a) catch null;
}

/// Re-assert keyboard focus on the search entry after a rebuild, but only when
/// focus has actually drifted off it — a fresh row set (or a churn tick under a
/// sheet-mode dialog) moves the toplevel focus onto an internal list widget,
/// which starves the capture-phase key controller of Return. Skipping when the
/// entry already owns focus keeps normal typing (which re-grabs would disrupt
/// by reselecting the text) untouched.
fn refocusEntry(state: *State) void {
    const entry_w = state.entry.as(gtk.Widget);
    if (gtk.Widget.getRoot(entry_w)) |root| {
        const win: *gtk.Window = @ptrCast(@alignCast(root));
        if (gtk.Window.getFocus(win)) |fw| {
            if (fw == entry_w or gtk.Widget.isAncestor(fw, entry_w) != 0) return;
        }
    }
    _ = gtk.Widget.grabFocus(entry_w);
}

fn rebuildRows(state: *State, arr: ?std.json.Array, dupeZ: *const fn ([]const u8) [:0]const u8) void {
    const new_sig = itemsSignature(arr);
    if (state.items_sig) |old| {
        if (new_sig) |ns| {
            if (std.mem.eql(u8, old, ns)) {
                std.heap.page_allocator.free(ns);
                return; // rows render identically: no teardown, keep highlight/focus
            }
        }
    }
    if (state.items_sig) |old| std.heap.page_allocator.free(old);
    state.items_sig = new_sig;

    gtk.ListBox.removeAll(state.list);
    freeIds(state);
    if (arr) |items| {
        for (items.items) |it| {
            if (it != .object) continue;
            const id = objStr(it.object, "id") orelse "";
            const title = objStr(it.object, "title") orelse "";
            const row = adw.ActionRow.new();
            gtk.Widget.setFocusable(row.as(gtk.Widget), 0); // keyboard stays on the entry; see the ListBox setup in create()
            adw.PreferencesRow.setTitle(row.as(adw.PreferencesRow), dupeZ(title));
            if (objStr(it.object, "subtitle")) |sub| adw.ActionRow.setSubtitle(row, dupeZ(sub));
            if (objStr(it.object, "iconName")) |ic| {
                const img = gtk.Image.newFromIconName(ndicons.symbolic(dupeZ(ic)));
                adw.ActionRow.addPrefix(row, img.as(gtk.Widget));
            }
            gtk.ListBox.append(state.list, row.as(gtk.Widget));
            const id_copy = std.heap.page_allocator.dupe(u8, id) catch continue;
            state.ids.append(std.heap.page_allocator, id_copy) catch {
                std.heap.page_allocator.free(id_copy);
            };
        }
    }
    // Fresh results: the top row is the highlighted default (Enter drills in).
    setHighlight(state, if (state.ids.items.len > 0) 0 else -1);
    if (state.presented) refocusEntry(state);
}

fn scrollToRow(state: *State, row: *gtk.ListBoxRow) void {
    var rect: graphene.Rect = undefined;
    if (gtk.Widget.computeBounds(row.as(gtk.Widget), state.list.as(gtk.Widget), &rect) == 0) return;
    const adj = gtk.ScrolledWindow.getVadjustment(state.scroller);
    const top: f64 = rect.f_origin.f_y;
    gtk.Adjustment.clampPage(adj, top, top + rect.f_size.f_height);
}

fn setHighlight(state: *State, idx: i32) void {
    state.highlight = idx;
    if (idx >= 0) {
        if (gtk.ListBox.getRowAtIndex(state.list, idx)) |row| {
            gtk.ListBox.selectRow(state.list, row);
            scrollToRow(state, row);
        }
    } else {
        gtk.ListBox.unselectAll(state.list);
    }
}

// ---- present / dismiss -----------------------------------------------------

fn present(state: *State) void {
    if (state.presented) return;
    const root = gtk.Widget.getRoot(state.handle) orelse {
        state.pending_open = true; // not rooted yet: cbHandleMapped presents
        return;
    };
    // Present over the application's active window (the visible window/tab),
    // not merely the handle's own root, so the overlay is modal over whatever
    // the user is looking at regardless of where the handle sits in the tree.
    // Fall back to the handle's root when there's no active window.
    const parent: *gtk.Widget = blk: {
        const win: *gtk.Window = @ptrCast(@alignCast(root));
        if (gtk.Window.getApplication(win)) |app| {
            if (gtk.Application.getActiveWindow(app)) |active| break :blk active.as(gtk.Widget);
        }
        break :blk state.handle;
    };
    adw.Dialog.present(state.dialog, parent);
    state.presented = true;
    state.pending_open = false;
    _ = gtk.Widget.grabFocus(state.entry.as(gtk.Widget));
    setHighlight(state, if (state.ids.items.len > 0) 0 else -1);
}

fn dismiss(state: *State, programmatic: bool) void {
    state.pending_open = false;
    if (!state.presented) return;
    state.programmatic_close = programmatic;
    _ = adw.Dialog.close(state.dialog);
}

// ---- create / applyProps ---------------------------------------------------

pub fn create(props: ?std.json.Value, dupeZ: *const fn ([]const u8) [:0]const u8) *gtk.Widget {
    const state = std.heap.page_allocator.create(State) catch @panic("OOM CommandPalette");

    const dialog = adw.Dialog.new();
    // Own a strong ref (sink the floating one): adw_dialog_close drops the
    // window's ref, so without ours the dialog would be destroyed on the first
    // close and a re-present (open flipping true again) would use freed memory.
    _ = gobject.Object.refSink(asObj(dialog));
    adw.Dialog.setPresentationMode(dialog, .floating); // centered + scrim, never a bottom sheet
    adw.Dialog.setContentWidth(dialog, 600);

    const content = gtk.Box.new(.vertical, 8);
    gtk.Widget.setMarginTop(content.as(gtk.Widget), 12);
    gtk.Widget.setMarginBottom(content.as(gtk.Widget), 12);
    gtk.Widget.setMarginStart(content.as(gtk.Widget), 12);
    gtk.Widget.setMarginEnd(content.as(gtk.Widget), 12);

    const entry = gtk.SearchEntry.new();
    if (propStr(props, "placeholder")) |ph| gtk.SearchEntry.setPlaceholderText(entry, dupeZ(ph));
    if (propStr(props, "query")) |q| {
        if (q.len > 0) gtk.Editable.setText(entry.as(gtk.Editable), dupeZ(q));
    }

    const scroller = gtk.ScrolledWindow.new();
    gtk.ScrolledWindow.setPolicy(scroller, .never, .automatic);
    gtk.ScrolledWindow.setMinContentHeight(scroller, 320);
    gtk.Widget.setVexpand(scroller.as(gtk.Widget), 1);

    const list = gtk.ListBox.new();
    gtk.ListBox.setSelectionMode(list, .browse);
    gtk.ListBox.setActivateOnSingleClick(list, 1);
    gtk.Widget.addCssClass(list.as(gtk.Widget), "boxed-list");
    // Keyboard focus stays on the search entry, always: its capture-phase key
    // controller is the ONLY keyboard route to `activate` (onKeyPressed). If the
    // list (or a row) could hold focus, a pointer click that drills into a folder
    // would move focus onto the list, and the next Return would fire GtkListBox's
    // activate-cursor-row -> row-activated -> a SECOND `activate` alongside
    // onKeyPressed's — the add-project-twice bug. A non-focusable list can't be
    // that second path; mouse activation (activate-on-single-click) and the
    // automation row-activated emit are gesture/signal driven, not focus driven,
    // so both still fire exactly once. Rows are made non-focusable in rebuildRows.
    gtk.Widget.setFocusable(list.as(gtk.Widget), 0);
    gtk.ScrolledWindow.setChild(scroller, list.as(gtk.Widget));

    gtk.Box.append(content, entry.as(gtk.Widget));
    gtk.Box.append(content, scroller.as(gtk.Widget));
    adw.Dialog.setChild(dialog, content.as(gtk.Widget));

    const handle = gtk.Box.new(.vertical, 0);
    state.* = .{ .handle = handle.as(gtk.Widget), .dialog = dialog, .entry = entry, .list = list, .scroller = scroller };
    rebuildRows(state, propArray(props, "items"), dupeZ);
    if (propBool(props, "open") orelse false) state.pending_open = true;

    gobject.Object.setData(asObj(handle), STATE_KEY, state);
    _ = gobject.signalConnectData(asObj(handle), "map", @ptrCast(&cbHandleMapped), state, null, .{});
    _ = gobject.signalConnectData(asObj(handle), "destroy", @ptrCast(&cbHandleDestroyed), state, null, .{});
    return handle.as(gtk.Widget);
}

pub fn applyProps(widget: *gtk.Widget, props: ?std.json.Value, dupeZ: *const fn ([]const u8) [:0]const u8) void {
    const state = stateOf(widget) orelse return;
    if (propStr(props, "placeholder")) |ph| gtk.SearchEntry.setPlaceholderText(state.entry, dupeZ(ph));
    if (propStr(props, "query")) |q| {
        const editable = state.entry.as(gtk.Editable);
        const cur = std.mem.span(gtk.Editable.getText(editable));
        if (!std.mem.eql(u8, cur, q)) {
            // Controlled echo suppression: setting the text must not re-fire
            // queryChanged (SearchInput's blockEcho lesson).
            if (state.search_changed_hid != 0) gobject.signalHandlerBlock(asObj(state.entry), state.search_changed_hid);
            gtk.Editable.setText(editable, dupeZ(q));
            if (state.search_changed_hid != 0) gobject.signalHandlerUnblock(asObj(state.entry), state.search_changed_hid);
        }
    }
    if (propArray(props, "items")) |arr| rebuildRows(state, arr, dupeZ);
    if (propBool(props, "open")) |o| {
        if (o) present(state) else dismiss(state, true);
    }
}

// ---- events ----------------------------------------------------------------

pub fn connectEvents(widget: *gtk.Widget, node_id: u32, emit_fn: EmitFn) void {
    emit = emit_fn;
    const state = stateOf(widget) orelse return;
    state.node_id = node_id;

    // "changed", not "search-changed": the latter is debounced ~150ms except on
    // an empty value, which splits gtk_editable_set_text's delete and insert
    // across two turns and collapses a controlled query. Peer of the
    // SearchInput.changed entry in tools/codegen.ts.
    state.search_changed_hid = gobject.signalConnectData(asObj(state.entry), "changed", @ptrCast(&cbSearchChanged), state, null, .{});
    _ = gobject.signalConnectData(asObj(state.list), "row-activated", @ptrCast(&cbRowActivated), state, null, .{});
    _ = gobject.signalConnectData(asObj(state.dialog), "closed", @ptrCast(&cbDialogClosed), state, null, .{});

    // Capture-phase key controller on the entry: intercept navigation/commit
    // keys before GtkSearchEntry consumes them (its own Esc clears the text),
    // let plain typing fall through to drive the entry's changed signal.
    const key_ctrl = gtk.EventControllerKey.new();
    gtk.EventController.setPropagationPhase(key_ctrl.as(gtk.EventController), .capture);
    _ = gtk.EventControllerKey.signals.key_pressed.connect(key_ctrl, *State, &onKeyPressed, state, .{});
    gtk.Widget.addController(state.entry.as(gtk.Widget), key_ctrl.as(gtk.EventController));
}

fn emitActivate(state: *State, idx: i32) void {
    if (idx < 0 or idx >= @as(i32, @intCast(state.ids.items.len))) return;
    if (emit) |f| f(state.node_id, "activate", .{ .text = state.ids.items[@intCast(idx)] });
}

fn emitSubmit(state: *State) void {
    const text = std.mem.span(gtk.Editable.getText(state.entry.as(gtk.Editable)));
    if (emit) |f| f(state.node_id, "submit", .{ .text = text });
}

// ---- automation ------------------------------------------------------------
// The tracked node is the host box; the real entry/list live in the separately
// presented dialog, so the generic setValue/type/click dispatch (which sniffs
// the host box as a plain GtkBox) can't reach them. The backend routes those
// three actions here instead, driving the same paths a user would: query text
// fires queryChanged, an integer index fires the ListBox row-activated ->
// onActivate, a bool submits, and click activates the current highlight.

/// True for the palette's host box (carries the palette state); lets the
/// backend's node_visible/node_bounds/semantic_action arms special-case it.
pub fn isPaletteHandle(widget: *gtk.Widget) bool {
    return gobject.Object.getData(asObj(widget), STATE_KEY) != null;
}

/// Actionable only while presented — a closed palette has no entry/list to act
/// on, so getTree/automation treats it as not-actionable.
pub fn isPresented(widget: *gtk.Widget) bool {
    const state = stateOf(widget) orelse return false;
    return state.presented;
}

fn cpMallocZ(json: []const u8) ?[*:0]u8 {
    const buf: [*]u8 = @ptrCast(std.c.malloc(json.len + 1) orelse return null);
    @memcpy(buf[0..json.len], json);
    buf[json.len] = 0;
    return @ptrCast(buf);
}

fn cpSetResult(out: *?[*:0]u8, value: anytype) void {
    const json = std.json.Stringify.valueAlloc(std.heap.page_allocator, value, .{}) catch return;
    defer std.heap.page_allocator.free(json);
    out.* = cpMallocZ(json);
}

fn cpSetErr(out: *?[*:0]u8, node_id: u32) i32 {
    var buf: [48]u8 = undefined;
    const json = std.fmt.bufPrint(&buf, "{{\"ref\":{d}}}", .{node_id}) catch return -32602;
    out.* = cpMallocZ(json);
    return -32602;
}

fn activateRowByIndex(state: *State, idx: i32) void {
    const row = gtk.ListBox.getRowAtIndex(state.list, idx) orelse return;
    gobject.signalEmitByName(asObj(state.list), "row-activated", row); // -> cbRowActivated -> emitActivate
}

/// Routes the palette node's setValue/type/click automation actions to the real
/// entry/list. Returns 0 ok / -32001 closed / -32602 invalid value.
pub fn automationAction(handle: *gtk.Widget, node_id: u32, action: []const u8, args: ?std.json.Value, result_out: *?[*:0]u8, err_out: *?[*:0]u8) i32 {
    const state = stateOf(handle) orelse return cpSetErr(err_out, node_id);
    if (!state.presented) {
        _ = cpSetErr(err_out, node_id);
        return -32001; // not actionable while closed
    }
    const obj: ?std.json.ObjectMap = if (args) |a| (if (a == .object) a.object else null) else null;

    if (std.mem.eql(u8, action, "setValue")) {
        const value = (if (obj) |o| o.get("value") else null) orelse return cpSetErr(err_out, node_id);
        switch (value) {
            .string => |s| {
                const z = std.heap.page_allocator.dupeZ(u8, s) catch return cpSetErr(err_out, node_id);
                defer std.heap.page_allocator.free(z);
                gtk.Editable.setText(state.entry.as(gtk.Editable), z); // fires changed -> queryChanged
            },
            .integer => |i| {
                if (i < 0 or i >= @as(i64, @intCast(state.ids.items.len))) return cpSetErr(err_out, node_id);
                activateRowByIndex(state, @intCast(i));
            },
            .bool => |b| {
                if (!b) return cpSetErr(err_out, node_id);
                emitSubmit(state);
            },
            else => return cpSetErr(err_out, node_id),
        }
        cpSetResult(result_out, .{ .ref = node_id, .applied = true });
        return 0;
    } else if (std.mem.eql(u8, action, "type")) {
        const t = (if (obj) |o| o.get("text") else null) orelse return cpSetErr(err_out, node_id);
        if (t != .string) return cpSetErr(err_out, node_id);
        const editable = state.entry.as(gtk.Editable);
        const cur = std.mem.span(gtk.Editable.getText(editable));
        var pos: c_int = @intCast(std.unicode.utf8CountCodepoints(cur) catch cur.len);
        const z = std.heap.page_allocator.dupeZ(u8, t.string) catch return cpSetErr(err_out, node_id);
        defer std.heap.page_allocator.free(z);
        gtk.Editable.insertText(editable, z, -1, &pos); // fires changed -> queryChanged
        cpSetResult(result_out, .{ .ref = node_id, .text = std.mem.span(gtk.Editable.getText(editable)) });
        return 0;
    } else if (std.mem.eql(u8, action, "click")) {
        const n: i32 = @intCast(state.ids.items.len);
        const idx: i32 = if (state.highlight >= 0 and state.highlight < n) state.highlight else if (n > 0) 0 else -1;
        if (idx >= 0) activateRowByIndex(state, idx);
        cpSetResult(result_out, .{ .ref = node_id, .dispatched = true });
        return 0;
    }
    _ = cpSetErr(err_out, node_id);
    return -32601;
}

fn cbSearchChanged(obj: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const state: *State = @ptrCast(@alignCast(data.?));
    const editable = @as(*gtk.SearchEntry, @ptrCast(@alignCast(obj))).as(gtk.Editable);
    const text = std.mem.span(gtk.Editable.getText(editable));
    if (emit) |f| f(state.node_id, "queryChanged", .{ .text = text });
}

// GtkListBox "row-activated" passes (box, row, user_data); single-click
// activation is on, so a click lands here directly.
fn cbRowActivated(_: *gobject.Object, row: *gtk.ListBoxRow, data: ?*anyopaque) callconv(.c) void {
    const state: *State = @ptrCast(@alignCast(data.?));
    const idx = gtk.ListBoxRow.getIndex(row);
    state.highlight = idx;
    emitActivate(state, idx);
}

// AdwDialog "closed" fires for user dismissal (Esc / click-outside) AND
// programmatic close; only the former reaches the app as onCancel.
fn cbDialogClosed(_: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const state: *State = @ptrCast(@alignCast(data.?));
    state.presented = false;
    if (state.programmatic_close) {
        state.programmatic_close = false;
        return;
    }
    if (emit) |f| f(state.node_id, "cancel", .{});
}

fn onKeyPressed(_: *gtk.EventControllerKey, keyval: c_uint, _: c_uint, mods: gdk.ModifierType, state: *State) callconv(.c) c_int {
    const n: i32 = @intCast(state.ids.items.len);
    switch (keyval) {
        gdk.KEY_Up => {
            if (n > 0) setHighlight(state, if (state.highlight <= 0) 0 else state.highlight - 1);
            return 1;
        },
        gdk.KEY_Down => {
            if (n > 0) setHighlight(state, if (state.highlight < 0) 0 else if (state.highlight >= n - 1) n - 1 else state.highlight + 1);
            return 1;
        },
        gdk.KEY_Home => {
            if (n > 0) setHighlight(state, 0);
            return 1;
        },
        gdk.KEY_End => {
            if (n > 0) setHighlight(state, n - 1);
            return 1;
        },
        gdk.KEY_Escape => {
            dismiss(state, false);
            return 1;
        },
        gdk.KEY_Return, gdk.KEY_KP_Enter => {
            if (mods.control_mask or state.highlight < 0 or state.highlight >= n) {
                emitSubmit(state);
            } else {
                emitActivate(state, state.highlight);
            }
            return 1;
        },
        else => return 0, // typing drives the entry's changed signal
    }
}

fn cbHandleMapped(_: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const state: *State = @ptrCast(@alignCast(data.?));
    if (state.pending_open) present(state);
}

fn cbHandleDestroyed(_: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const state: *State = @ptrCast(@alignCast(data.?));
    // The dialog is presented over the window, not parented under this handle,
    // so destroying the handle never reaches it — close it explicitly (else an
    // unmount-while-open leaves an orphaned dialog on screen), then drop our
    // refSink so it (and its handlers) are destroyed before `state` is freed.
    if (state.presented) {
        state.programmatic_close = true;
        _ = adw.Dialog.forceClose(state.dialog);
    }
    gobject.Object.unref(asObj(state.dialog));
    freeIds(state);
    state.ids.deinit(std.heap.page_allocator);
    if (state.items_sig) |sig| std.heap.page_allocator.free(sig);
    std.heap.page_allocator.destroy(state);
}
