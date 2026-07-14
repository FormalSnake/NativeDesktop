// GTK4/libadwaita dialog surface for the <window> widget's imperative
// commands (M15 widget expansion): showAlert (AdwAlertDialog), openFile /
// saveFile (GtkFileDialog, async), showAbout (AdwAboutDialog). Results fire
// the Window node's alertResult/openFileResult/saveFileResult events —
// the same widgetCommand → same-node result-event pattern webview.zig's
// executeJavaScript → javaScriptResult established.
//
// Multi-window correctness: every dialog parents on the Window node's OWN
// widget handle (the AdwApplicationWindow the command was addressed to),
// never a global — two windows can each run their own dialog.
const std = @import("std");
const gtk = @import("gtk");
const gio = @import("gio");
const glib = @import("glib");
const gobject = @import("gobject");
const adw = @import("adw");
const protocol = @import("../protocol.zig");

/// Peer of the generated widgets.zig EmitFn (same shape, same protocol module
/// instance) — handed over once by the generated connectEvents Window arm.
pub const EmitFn = *const fn (node_id: u32, name: []const u8, payload: protocol.EventPayload) void;

const NODE_ID_KEY = "nd-window-node-id";
var emit: ?EmitFn = null;

/// Generated connectEvents Window arm: stash the node id on the window widget
/// so `command` (which gets no node id of its own) can address result events.
pub fn connectEvents(widget: *gtk.Widget, node_id: u32, emit_fn: EmitFn) void {
    emit = emit_fn;
    gobject.Object.setData(widget.as(gobject.Object), NODE_ID_KEY, @ptrFromInt(@as(usize, node_id)));
}

fn widgetNodeId(widget: *gtk.Widget) ?u32 {
    const raw = gobject.Object.getData(widget.as(gobject.Object), NODE_ID_KEY) orelse return null;
    return @intCast(@intFromPtr(raw));
}

fn argObject(arg: ?std.json.Value) ?std.json.ObjectMap {
    return switch (arg orelse return null) {
        .object => |o| o,
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

fn objPutStr(obj: *std.json.ObjectMap, key: []const u8, val: []const u8) void {
    obj.put(std.heap.page_allocator, key, .{ .string = val }) catch {};
}

fn objPutBool(obj: *std.json.ObjectMap, key: []const u8, val: bool) void {
    obj.put(std.heap.page_allocator, key, .{ .bool = val }) catch {};
}

/// widgetCommand dispatch (generated widgets.zig Window arm). `widget` is the
/// Window node's handle — an AdwApplicationWindow stored as *gtk.Widget.
pub fn command(widget: *gtk.Widget, cmd: []const u8, arg: ?std.json.Value) void {
    if (std.mem.eql(u8, cmd, "showAlert")) {
        cmdShowAlert(widget, arg);
    } else if (std.mem.eql(u8, cmd, "openFile")) {
        cmdOpenFile(widget, arg);
    } else if (std.mem.eql(u8, cmd, "saveFile")) {
        cmdSaveFile(widget, arg);
    } else if (std.mem.eql(u8, cmd, "showAbout")) {
        cmdShowAbout(widget, arg);
    } else {
        std.debug.print("ND_WARN unknown Window command {s}\n", .{cmd});
    }
}

// ---- showAlert (AdwAlertDialog, adw >= 1.5) --------------------------------
// arg: { title, body?, buttons: [{ id, label, style?: "default"|"suggested"|
// "destructive" }], defaultId?, closeId? } -> alertResult { buttonId }.
// HIG: at most one suggested and one destructive response; Enter fires the
// default response (never defaulted to a destructive one here — the app must
// opt in explicitly via defaultId), Esc fires the close response.

fn cmdShowAlert(widget: *gtk.Widget, arg: ?std.json.Value) void {
    const node_id = widgetNodeId(widget) orelse return;
    const obj = argObject(arg) orelse {
        std.debug.print("ND_WARN Window showAlert: malformed arg (expected {{title, buttons}})\n", .{});
        return;
    };
    var arena_state = std.heap.ArenaAllocator.init(std.heap.page_allocator);
    defer arena_state.deinit(); // AdwAlertDialog copies every string it is handed
    const arena = arena_state.allocator();

    const title = objStr(obj, "title") orelse "";
    const title_z = arena.dupeZ(u8, title) catch return;
    const body_z: ?[*:0]const u8 = if (objStr(obj, "body")) |b| (arena.dupeZ(u8, b) catch return).ptr else null;
    const dialog = adw.AlertDialog.new(title_z, body_z);

    var suggested_id: ?[:0]const u8 = null;
    if (obj.get("buttons")) |btns| {
        if (btns == .array) for (btns.array.items) |b| {
            if (b != .object) continue;
            const id = objStr(b.object, "id") orelse continue;
            const label = objStr(b.object, "label") orelse id;
            const id_z = arena.dupeZ(u8, id) catch continue;
            const label_z = arena.dupeZ(u8, label) catch continue;
            adw.AlertDialog.addResponse(dialog, id_z, label_z);
            if (objStr(b.object, "style")) |style| {
                if (std.mem.eql(u8, style, "suggested")) {
                    adw.AlertDialog.setResponseAppearance(dialog, id_z, .suggested);
                    if (suggested_id == null) suggested_id = id_z;
                } else if (std.mem.eql(u8, style, "destructive")) {
                    adw.AlertDialog.setResponseAppearance(dialog, id_z, .destructive);
                }
            }
        };
    }
    if (objStr(obj, "defaultId")) |d| {
        adw.AlertDialog.setDefaultResponse(dialog, (arena.dupeZ(u8, d) catch return).ptr);
    } else if (suggested_id) |s| {
        adw.AlertDialog.setDefaultResponse(dialog, s.ptr); // Enter = the suggested action (never destructive)
    }
    if (objStr(obj, "closeId")) |c| {
        adw.AlertDialog.setCloseResponse(dialog, arena.dupeZ(u8, c) catch return);
    }

    _ = gobject.signalConnectData(@ptrCast(@alignCast(dialog)), "response", @ptrCast(&cbAlertResponse), @ptrFromInt(@as(usize, node_id)), null, .{});
    adw.Dialog.present(dialog.as(adw.Dialog), widget);
}

fn cbAlertResponse(_: *gobject.Object, response: [*:0]const u8, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const f = emit orelse return;
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(std.heap.page_allocator);
    objPutStr(&payload, "buttonId", std.mem.span(response));
    f(node_id, "alertResult", .{ .data = .{ .object = payload } });
}

// ---- openFile / saveFile (GtkFileDialog, GTK >= 4.10, async) ---------------
// openFile arg: { multiple?, directories?, filters?: [{ name, extensions[] }] }
//   -> openFileResult { canceled, paths: string[] }
// saveFile arg: { suggestedName?, defaultDir?, filters? }
//   -> saveFileResult { canceled, path: string|null }

const FileCtx = struct {
    node_id: u32,
    mode: enum { open_single, open_multiple, folder_single, folder_multiple, save },
};

fn buildFileDialog(arena: std.mem.Allocator, obj: ?std.json.ObjectMap) *gtk.FileDialog {
    const dialog = gtk.FileDialog.new();
    const o = obj orelse return dialog;
    if (objStr(o, "suggestedName")) |n| {
        if (arena.dupeZ(u8, n) catch null) |z| gtk.FileDialog.setInitialName(dialog, z);
    }
    if (objStr(o, "defaultDir")) |d| {
        if (arena.dupeZ(u8, d) catch null) |z| {
            const folder = gio.File.newForPath(z);
            defer gobject.Object.unref(@ptrCast(@alignCast(folder)));
            gtk.FileDialog.setInitialFolder(dialog, folder);
        }
    }
    if (o.get("filters")) |filters| {
        if (filters == .array and filters.array.items.len > 0) {
            const store = gio.ListStore.new(gtk.FileFilter.getGObjectType());
            var first: ?*gtk.FileFilter = null;
            for (filters.array.items) |fv| {
                if (fv != .object) continue;
                const filter = gtk.FileFilter.new();
                if (objStr(fv.object, "name")) |n| {
                    if (arena.dupeZ(u8, n) catch null) |z| gtk.FileFilter.setName(filter, z);
                }
                if (fv.object.get("extensions")) |exts| {
                    if (exts == .array) for (exts.array.items) |e| {
                        if (e != .string) continue;
                        if (arena.dupeZ(u8, e.string) catch null) |z| gtk.FileFilter.addSuffix(filter, z);
                    };
                }
                gio.ListStore.append(store, @ptrCast(@alignCast(filter)));
                gobject.Object.unref(@ptrCast(@alignCast(filter))); // store holds the ref now
                if (first == null) first = filter;
            }
            gtk.FileDialog.setFilters(dialog, store.as(gio.ListModel));
            gobject.Object.unref(@ptrCast(@alignCast(store))); // dialog holds the ref now
            if (first) |filter| gtk.FileDialog.setDefaultFilter(dialog, filter);
        }
    }
    return dialog;
}

fn cmdOpenFile(widget: *gtk.Widget, arg: ?std.json.Value) void {
    const node_id = widgetNodeId(widget) orelse return;
    const obj = argObject(arg);
    var arena_state = std.heap.ArenaAllocator.init(std.heap.page_allocator);
    defer arena_state.deinit(); // GtkFileDialog copies its config strings
    const dialog = buildFileDialog(arena_state.allocator(), obj);
    const window: *gtk.Window = @ptrCast(@alignCast(widget));
    const multiple = if (obj) |o| (objBool(o, "multiple") orelse false) else false;
    const directories = if (obj) |o| (objBool(o, "directories") orelse false) else false;

    const ctx = std.heap.page_allocator.create(FileCtx) catch return;
    if (directories and multiple) {
        ctx.* = .{ .node_id = node_id, .mode = .folder_multiple };
        gtk.FileDialog.selectMultipleFolders(dialog, window, null, &cbFileMany, ctx);
    } else if (directories) {
        ctx.* = .{ .node_id = node_id, .mode = .folder_single };
        gtk.FileDialog.selectFolder(dialog, window, null, &cbFileOne, ctx);
    } else if (multiple) {
        ctx.* = .{ .node_id = node_id, .mode = .open_multiple };
        gtk.FileDialog.openMultiple(dialog, window, null, &cbFileMany, ctx);
    } else {
        ctx.* = .{ .node_id = node_id, .mode = .open_single };
        gtk.FileDialog.open(dialog, window, null, &cbFileOne, ctx);
    }
}

fn cmdSaveFile(widget: *gtk.Widget, arg: ?std.json.Value) void {
    const node_id = widgetNodeId(widget) orelse return;
    const obj = argObject(arg);
    var arena_state = std.heap.ArenaAllocator.init(std.heap.page_allocator);
    defer arena_state.deinit();
    const dialog = buildFileDialog(arena_state.allocator(), obj);
    const window: *gtk.Window = @ptrCast(@alignCast(widget));
    const ctx = std.heap.page_allocator.create(FileCtx) catch return;
    ctx.* = .{ .node_id = node_id, .mode = .save };
    gtk.FileDialog.save(dialog, window, null, &cbFileOne, ctx);
}

/// Completion for the single-GFile dialogs (open / selectFolder / save).
fn cbFileOne(source: ?*gobject.Object, res: *gio.AsyncResult, data: ?*anyopaque) callconv(.c) void {
    const ctx: *FileCtx = @ptrCast(@alignCast(data orelse return));
    defer std.heap.page_allocator.destroy(ctx);
    const dialog: *gtk.FileDialog = @ptrCast(@alignCast(source orelse return));
    var err: ?*glib.Error = null;
    const file: ?*gio.File = switch (ctx.mode) {
        .open_single => gtk.FileDialog.openFinish(dialog, res, &err),
        .folder_single => gtk.FileDialog.selectFolderFinish(dialog, res, &err),
        .save => gtk.FileDialog.saveFinish(dialog, res, &err),
        else => null,
    };
    if (err) |e| e.free(); // dismissal surfaces as GTK_DIALOG_ERROR_DISMISSED — reported as canceled
    const f = emit orelse return;

    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(std.heap.page_allocator);
    if (ctx.mode == .save) {
        objPutBool(&payload, "canceled", file == null);
        if (file) |file_| {
            defer gobject.Object.unref(@ptrCast(@alignCast(file_)));
            if (gio.File.getPath(file_)) |p| {
                defer glib.free(p);
                objPutStr(&payload, "path", std.mem.span(p));
            } else {
                payload.put(std.heap.page_allocator, "path", .null) catch {};
            }
        } else {
            payload.put(std.heap.page_allocator, "path", .null) catch {};
        }
        f(ctx.node_id, "saveFileResult", .{ .data = .{ .object = payload } });
        return;
    }
    objPutBool(&payload, "canceled", file == null);
    var paths: std.json.Array = .init(std.heap.page_allocator);
    defer paths.deinit();
    if (file) |file_| {
        defer gobject.Object.unref(@ptrCast(@alignCast(file_)));
        if (gio.File.getPath(file_)) |p| {
            defer glib.free(p);
            paths.append(.{ .string = std.mem.span(p) }) catch {};
        }
    }
    payload.put(std.heap.page_allocator, "paths", .{ .array = paths }) catch {};
    f(ctx.node_id, "openFileResult", .{ .data = .{ .object = payload } });
}

/// Completion for the GListModel-of-GFile dialogs (openMultiple /
/// selectMultipleFolders).
fn cbFileMany(source: ?*gobject.Object, res: *gio.AsyncResult, data: ?*anyopaque) callconv(.c) void {
    const ctx: *FileCtx = @ptrCast(@alignCast(data orelse return));
    defer std.heap.page_allocator.destroy(ctx);
    const dialog: *gtk.FileDialog = @ptrCast(@alignCast(source orelse return));
    var err: ?*glib.Error = null;
    const list: ?*gio.ListModel = switch (ctx.mode) {
        .open_multiple => gtk.FileDialog.openMultipleFinish(dialog, res, &err),
        .folder_multiple => gtk.FileDialog.selectMultipleFoldersFinish(dialog, res, &err),
        else => null,
    };
    if (err) |e| e.free();
    const f = emit orelse return;

    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(std.heap.page_allocator);
    objPutBool(&payload, "canceled", list == null);
    var paths: std.json.Array = .init(std.heap.page_allocator);
    defer paths.deinit();
    if (list) |model| {
        defer gobject.Object.unref(@ptrCast(@alignCast(model)));
        const n = gio.ListModel.getNItems(model);
        var i: c_uint = 0;
        while (i < n) : (i += 1) {
            const item = gio.ListModel.getItem(model, i) orelse continue;
            defer gobject.Object.unref(item);
            const file: *gio.File = @ptrCast(@alignCast(item));
            if (gio.File.getPath(file)) |p| {
                defer glib.free(p);
                // dupe: the span dies with glib.free above, but the payload is
                // stringified synchronously inside emit -> safe to keep a copy
                // in the arena-less page allocator for the emit's duration.
                const copy = std.heap.page_allocator.dupe(u8, std.mem.span(p)) catch continue;
                paths.append(.{ .string = copy }) catch {};
            }
        }
    }
    payload.put(std.heap.page_allocator, "paths", .{ .array = paths }) catch {};
    f(ctx.node_id, "openFileResult", .{ .data = .{ .object = payload } });
    for (paths.items) |v| {
        if (v == .string) std.heap.page_allocator.free(v.string);
    }
}

// ---- showAbout (AdwAboutDialog, adw >= 1.5) --------------------------------
// arg: { appName, version?, developer?, website? } — fire-and-forget (no
// result event), mirroring WebView's goBack/reload/stop.

fn cmdShowAbout(widget: *gtk.Widget, arg: ?std.json.Value) void {
    const obj = argObject(arg) orelse {
        std.debug.print("ND_WARN Window showAbout: malformed arg (expected {{appName, ...}})\n", .{});
        return;
    };
    var arena_state = std.heap.ArenaAllocator.init(std.heap.page_allocator);
    defer arena_state.deinit(); // AdwAboutDialog copies its property strings
    const arena = arena_state.allocator();
    const dialog = adw.AboutDialog.new();
    if (objStr(obj, "appName")) |n| {
        adw.AboutDialog.setApplicationName(dialog, arena.dupeZ(u8, n) catch return);
    }
    if (objStr(obj, "version")) |v| {
        adw.AboutDialog.setVersion(dialog, arena.dupeZ(u8, v) catch return);
    }
    if (objStr(obj, "developer")) |d| {
        adw.AboutDialog.setDeveloperName(dialog, arena.dupeZ(u8, d) catch return);
    }
    if (objStr(obj, "website")) |w| {
        adw.AboutDialog.setWebsite(dialog, arena.dupeZ(u8, w) catch return);
    }
    adw.Dialog.present(dialog.as(adw.Dialog), widget);
}
