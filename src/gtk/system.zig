// GTK/Linux host-side of the system-capabilities seam (ABI vtable op #22,
// `system_request`). Every request eventually answers exactly once via
// `nd_system_response`; app-level events (file drops, OS launch delivery, app
// activation) are pushed with `nd_system_event`. All of this runs on the GTK
// UI thread (runtime.zig marshals `system_request` before calling us), so the
// async GObject dialogs/clipboard reads carry their (ctx, id) in a heap job
// freed inside the completion callback.
//
// Optional native libs follow the same house rule as webview.zig: libsecret is
// dlopen'd at runtime, never hard-linked — absent at runtime means credential
// methods degrade to a clean error, and build.zig stays untouched.
const std = @import("std");
const gtk = @import("gtk");
const gdk = @import("gdk");
const gio = @import("gio");
const glib = @import("glib");
const gobject = @import("gobject");
const abi = @import("../abi.zig");
const audio = @import("audio.zig");

const alloc = std.heap.page_allocator;

// GtkDialogError (gtk/gtkdialogerror.h): FAILED=0, CANCELLED=1, DISMISSED=2.
// GTK is linked into this host (unlike dlopen'd WebKitGTK), so the quark is a
// plain extern — it distinguishes a user cancel (empty/null result) from a
// real dialog failure (ok=false).
extern fn gtk_dialog_error_quark() glib.Quark;
const DIALOG_ERROR_CANCELLED: c_int = 1;
const DIALOG_ERROR_DISMISSED: c_int = 2;

var the_ctx: ?*abi.NdContext = null;
var the_app: ?*gtk.Application = null;

/// Set by `backend.setCtx`/`backend.setApp` (main.zig) before the first
/// commit — needed by the event-emitting paths (drops, activation, OS open)
/// that fire outside a `system_request` call and by the dialog parent lookup.
pub fn setCtx(ctx: *abi.NdContext) void {
    the_ctx = ctx;
}
pub fn setApp(app: *gtk.Application) void {
    the_app = app;
}

// ============================================================================
// Dispatch
// ============================================================================

/// vtable `system_request` body: routes a whitelisted method (the core already
/// ACL-gated it and rejected unknown methods) to its implementation. The
/// `audio.*` family lives in audio.zig (GStreamer, dlopen'd like webview.zig).
pub fn handleRequest(ctx: *abi.NdContext, id: u32, method: [*:0]const u8, params: [*:0]const u8) void {
    const m = std.mem.span(method);
    const p = std.mem.span(params);
    if (std.mem.startsWith(u8, m, "audio.")) return audio.handleRequest(ctx, id, m, p);
    if (std.mem.eql(u8, m, "dialog.openFile")) return openFile(ctx, id, p);
    if (std.mem.eql(u8, m, "dialog.saveFile")) return saveFile(ctx, id, p);
    if (std.mem.eql(u8, m, "dialog.showMessage")) return showMessage(ctx, id, p);
    if (std.mem.eql(u8, m, "clipboard.readText")) return clipboardReadText(ctx, id);
    if (std.mem.eql(u8, m, "clipboard.readImage")) return clipboardReadImage(ctx, id);
    if (std.mem.eql(u8, m, "clipboard.writeText")) return clipboardWriteText(ctx, id, p);
    if (std.mem.eql(u8, m, "notification.show")) return notificationShow(ctx, id, p);
    if (std.mem.eql(u8, m, "recent.add")) return recentAdd(ctx, id, p);
    if (std.mem.eql(u8, m, "recent.clear")) return recentClear(ctx, id);
    if (std.mem.eql(u8, m, "credentials.set")) return credentialsSet(ctx, id, p);
    if (std.mem.eql(u8, m, "credentials.get")) return credentialsGet(ctx, id, p);
    if (std.mem.eql(u8, m, "credentials.delete")) return credentialsDelete(ctx, id, p);
    respond(ctx, id, false, "not implemented");
}

// ============================================================================
// Response / event helpers
// ============================================================================

/// The one reply per request. On ok=true `json` must be well-formed JSON (the
/// method result); on ok=false it is a plain error-message string. The core
/// serializes synchronously, so any borrowed slice (e.g. a GError message) is
/// safe to free right after this returns.
fn respond(ctx: *abi.NdContext, id: u32, ok: bool, json: []const u8) void {
    const z = alloc.dupeZ(u8, json) catch return;
    defer alloc.free(z);
    abi.nd_system_response(ctx, id, ok, z.ptr);
}

/// ok=true reply whose result is a JSON-serialized value — strings and string
/// arrays get proper JSON escaping (never raw-spliced).
fn respondValue(ctx: *abi.NdContext, id: u32, value: anytype) void {
    const json = std.json.Stringify.valueAlloc(alloc, value, .{}) catch return respond(ctx, id, false, "oom");
    defer alloc.free(json);
    respond(ctx, id, true, json);
}

fn emitSystemEvent(ctx: *abi.NdContext, channel: []const u8, value: anytype) void {
    const json = std.json.Stringify.valueAlloc(alloc, value, .{}) catch return;
    defer alloc.free(json);
    const json_z = alloc.dupeZ(u8, json) catch return;
    defer alloc.free(json_z);
    const chan_z = alloc.dupeZ(u8, channel) catch return;
    defer alloc.free(chan_z);
    abi.nd_system_event(ctx, chan_z.ptr, json_z.ptr);
}

/// A null dialog result carries a GError on cancel (a dismissed/cancelled
/// GtkDialogError) or a genuine failure. Cancel is not an error — it replies
/// ok=true with the method's empty result (`[]` for openFile, `null` for
/// saveFile); a real GError replies ok=false with its message.
fn respondCancelOrError(ctx: *abi.NdContext, id: u32, err: ?*glib.Error, cancel_json: []const u8) void {
    if (err) |e| {
        if (e.f_domain == gtk_dialog_error_quark() and (e.f_code == DIALOG_ERROR_CANCELLED or e.f_code == DIALOG_ERROR_DISMISSED)) {
            respond(ctx, id, true, cancel_json);
        } else {
            const msg: []const u8 = if (e.f_message) |m| std.mem.span(m) else "dialog failed";
            respond(ctx, id, false, msg);
        }
        e.free();
        return;
    }
    respond(ctx, id, true, cancel_json);
}

/// `g_object_unref` on any GObject-derived opaque handle (interfaces included);
/// the opaque bindings are alignment-1, so the cast to the 8-aligned Object
/// needs the explicit @alignCast.
fn objUnref(obj: anytype) void {
    gobject.Object.unref(@ptrCast(@alignCast(obj)));
}

fn parseParams(comptime T: type, p: []const u8) ?std.json.Parsed(T) {
    return std.json.parseFromSlice(T, alloc, p, .{ .ignore_unknown_fields = true }) catch null;
}

fn activeWindow() ?*gtk.Window {
    const app = the_app orelse return null;
    return gtk.Application.getActiveWindow(app);
}

// ============================================================================
// clipboard.*
// ============================================================================

fn clipboardWriteText(ctx: *abi.NdContext, id: u32, p: []const u8) void {
    const P = struct { text: []const u8 = "" };
    const parsed = parseParams(P, p) orelse return respond(ctx, id, false, "invalid params");
    defer parsed.deinit();
    const display = gdk.Display.getDefault() orelse return respond(ctx, id, false, "no display");
    const clipboard = gdk.Display.getClipboard(display);
    const z = alloc.dupeZ(u8, parsed.value.text) catch return respond(ctx, id, false, "oom");
    defer alloc.free(z);
    gdk.Clipboard.setText(clipboard, z.ptr);
    respond(ctx, id, true, "null");
}

fn clipboardReadText(ctx: *abi.NdContext, id: u32) void {
    const display = gdk.Display.getDefault() orelse return respond(ctx, id, false, "no display");
    const clipboard = gdk.Display.getClipboard(display);
    const job = alloc.create(Job) catch return respond(ctx, id, false, "oom");
    job.* = .{ .ctx = ctx, .id = id };
    gdk.Clipboard.readTextAsync(clipboard, null, &cbClipboardRead, job);
}

const Job = struct { ctx: *abi.NdContext, id: u32 };

/// WP-B1: read a bitmap image off the clipboard, write it to a host-local temp
/// PNG, and return {path,width,height}. Privileged (core:clipboard.read.image,
/// default-deny) — image bytes never enter NDP, only the resulting path does.
fn clipboardReadImage(ctx: *abi.NdContext, id: u32) void {
    const display = gdk.Display.getDefault() orelse return respond(ctx, id, false, "no display");
    const clipboard = gdk.Display.getClipboard(display);
    const job = alloc.create(Job) catch return respond(ctx, id, false, "oom");
    job.* = .{ .ctx = ctx, .id = id };
    gdk.Clipboard.readTextureAsync(clipboard, null, &cbClipboardReadImage, job);
}

var clip_image_counter: std.atomic.Value(u64) = std.atomic.Value(u64).init(0);

fn cbClipboardReadImage(source: ?*gobject.Object, res: *gio.AsyncResult, user_data: ?*anyopaque) callconv(.c) void {
    const job: *Job = @ptrCast(@alignCast(user_data.?));
    defer alloc.destroy(job);
    const clipboard: *gdk.Clipboard = @ptrCast(@alignCast(source.?));
    var err: ?*glib.Error = null;
    const texture = gdk.Clipboard.readTextureFinish(clipboard, res, &err);
    const tex = texture orelse {
        if (err) |e| e.free();
        return respond(job.ctx, job.id, false, "no image on clipboard");
    };
    defer objUnref(tex);

    var buf: [256]u8 = undefined;
    const dir_z = std.c.getenv("TMPDIR");
    const dir: []const u8 = if (dir_z) |d| std.mem.span(d) else "/tmp";
    const n = clip_image_counter.fetchAdd(1, .monotonic);
    const path = std.fmt.bufPrintZ(&buf, "{s}/nd-clip-{d}-{d}.png", .{ dir, std.c.getpid(), n }) catch
        return respond(job.ctx, job.id, false, "path");
    if (gdk.Texture.saveToPng(tex, path.ptr) == 0) return respond(job.ctx, job.id, false, "save failed");

    const width = gdk.Texture.getWidth(tex);
    const height = gdk.Texture.getHeight(tex);
    respondValue(job.ctx, job.id, .{ .path = @as([]const u8, path), .width = width, .height = height });
}

fn cbClipboardRead(source: ?*gobject.Object, res: *gio.AsyncResult, user_data: ?*anyopaque) callconv(.c) void {
    const job: *Job = @ptrCast(@alignCast(user_data.?));
    defer alloc.destroy(job);
    const clipboard: *gdk.Clipboard = @ptrCast(@alignCast(source.?));
    var err: ?*glib.Error = null;
    const text = gdk.Clipboard.readTextFinish(clipboard, res, &err);
    if (text) |t| {
        defer glib.free(t);
        respondValue(job.ctx, job.id, std.mem.span(t));
    } else {
        if (err) |e| e.free();
        respondValue(job.ctx, job.id, @as([]const u8, "")); // empty string when no text
    }
}

// ============================================================================
// dialog.*
// ============================================================================

const Filter = struct { name: ?[]const u8 = null, extensions: ?[]const []const u8 = null };

const FileMode = enum { single, multiple, folder, folders, save };
const FileJob = struct { ctx: *abi.NdContext, id: u32, dialog: *gtk.FileDialog, mode: FileMode };

fn openFile(ctx: *abi.NdContext, id: u32, p: []const u8) void {
    const P = struct {
        title: ?[]const u8 = null,
        defaultPath: ?[]const u8 = null,
        filters: ?[]const Filter = null,
        multiple: bool = false,
        directories: bool = false,
    };
    const parsed = parseParams(P, p) orelse return respond(ctx, id, false, "invalid params");
    defer parsed.deinit();
    const v = parsed.value;

    const dialog = gtk.FileDialog.new();
    if (v.title) |t| setDialogTitle(dialog, t);
    if (v.defaultPath) |dp| setInitialFile(dialog, dp);
    applyFilters(dialog, v.filters);

    const mode: FileMode = if (v.directories)
        (if (v.multiple) .folders else .folder)
    else
        (if (v.multiple) .multiple else .single);

    const job = alloc.create(FileJob) catch {
        gtk.FileDialog.unref(dialog);
        return respond(ctx, id, false, "oom");
    };
    job.* = .{ .ctx = ctx, .id = id, .dialog = dialog, .mode = mode };
    const parent = activeWindow();
    switch (mode) {
        .single => gtk.FileDialog.open(dialog, parent, null, &cbFileDialog, job),
        .multiple => gtk.FileDialog.openMultiple(dialog, parent, null, &cbFileDialog, job),
        .folder => gtk.FileDialog.selectFolder(dialog, parent, null, &cbFileDialog, job),
        .folders => gtk.FileDialog.selectMultipleFolders(dialog, parent, null, &cbFileDialog, job),
        .save => unreachable,
    }
}

fn saveFile(ctx: *abi.NdContext, id: u32, p: []const u8) void {
    const P = struct {
        title: ?[]const u8 = null,
        defaultPath: ?[]const u8 = null,
        defaultName: ?[]const u8 = null,
        filters: ?[]const Filter = null,
    };
    const parsed = parseParams(P, p) orelse return respond(ctx, id, false, "invalid params");
    defer parsed.deinit();
    const v = parsed.value;

    const dialog = gtk.FileDialog.new();
    if (v.title) |t| setDialogTitle(dialog, t);
    if (v.defaultPath) |dp| setInitialFile(dialog, dp);
    if (v.defaultName) |dn| setInitialName(dialog, dn);
    applyFilters(dialog, v.filters);

    const job = alloc.create(FileJob) catch {
        gtk.FileDialog.unref(dialog);
        return respond(ctx, id, false, "oom");
    };
    job.* = .{ .ctx = ctx, .id = id, .dialog = dialog, .mode = .save };
    gtk.FileDialog.save(dialog, activeWindow(), null, &cbFileDialog, job);
}

fn cbFileDialog(_: ?*gobject.Object, res: *gio.AsyncResult, user_data: ?*anyopaque) callconv(.c) void {
    const job: *FileJob = @ptrCast(@alignCast(user_data.?));
    defer {
        gtk.FileDialog.unref(job.dialog);
        alloc.destroy(job);
    }
    var err: ?*glib.Error = null;
    switch (job.mode) {
        .single => respondSingleFile(job, gtk.FileDialog.openFinish(job.dialog, res, &err), err),
        .folder => respondSingleFile(job, gtk.FileDialog.selectFolderFinish(job.dialog, res, &err), err),
        .multiple => respondFileList(job, gtk.FileDialog.openMultipleFinish(job.dialog, res, &err), err),
        .folders => respondFileList(job, gtk.FileDialog.selectMultipleFoldersFinish(job.dialog, res, &err), err),
        .save => respondSaveFile(job, gtk.FileDialog.saveFinish(job.dialog, res, &err), err),
    }
}

fn respondSingleFile(job: *FileJob, file: ?*gio.File, err: ?*glib.Error) void {
    if (file) |f| {
        defer objUnref(f);
        const pz = gio.File.getPath(f) orelse return respondValue(job.ctx, job.id, @as([]const []const u8, &.{}));
        defer glib.free(pz);
        const one = [_][]const u8{std.mem.span(pz)};
        respondValue(job.ctx, job.id, @as([]const []const u8, &one));
        return;
    }
    respondCancelOrError(job.ctx, job.id, err, "[]");
}

fn respondSaveFile(job: *FileJob, file: ?*gio.File, err: ?*glib.Error) void {
    if (file) |f| {
        defer objUnref(f);
        const pz = gio.File.getPath(f) orelse return respond(job.ctx, job.id, true, "null");
        defer glib.free(pz);
        respondValue(job.ctx, job.id, std.mem.span(pz));
        return;
    }
    respondCancelOrError(job.ctx, job.id, err, "null");
}

fn respondFileList(job: *FileJob, list: ?*gio.ListModel, err: ?*glib.Error) void {
    const lm = list orelse return respondCancelOrError(job.ctx, job.id, err, "[]");
    defer objUnref(lm);
    const n = gio.ListModel.getNItems(lm);
    const paths = alloc.alloc([]const u8, n) catch return respond(job.ctx, job.id, false, "oom");
    var count: usize = 0;
    defer {
        for (paths[0..count]) |it| alloc.free(it);
        alloc.free(paths);
    }
    var i: c_uint = 0;
    while (i < n) : (i += 1) {
        const item = gio.ListModel.getItem(lm, i) orelse continue;
        const file: *gio.File = @ptrCast(@alignCast(item));
        defer objUnref(file);
        const pz = gio.File.getPath(file) orelse continue;
        defer glib.free(pz);
        const dup = alloc.dupe(u8, std.mem.span(pz)) catch continue;
        paths[count] = dup;
        count += 1;
    }
    respondValue(job.ctx, job.id, paths[0..count]);
}

fn setDialogTitle(dialog: *gtk.FileDialog, title: []const u8) void {
    const z = alloc.dupeZ(u8, title) catch return;
    defer alloc.free(z);
    gtk.FileDialog.setTitle(dialog, z.ptr);
}

fn setInitialFile(dialog: *gtk.FileDialog, path: []const u8) void {
    const z = alloc.dupeZ(u8, path) catch return;
    defer alloc.free(z);
    const file = gio.File.newForPath(z.ptr);
    defer objUnref(file);
    gtk.FileDialog.setInitialFile(dialog, file);
}

fn setInitialName(dialog: *gtk.FileDialog, name: []const u8) void {
    const z = alloc.dupeZ(u8, name) catch return;
    defer alloc.free(z);
    gtk.FileDialog.setInitialName(dialog, z.ptr);
}

fn applyFilters(dialog: *gtk.FileDialog, filters: ?[]const Filter) void {
    const fs = filters orelse return;
    if (fs.len == 0) return;
    const store = gio.ListStore.new(gtk.FileFilter.getGObjectType());
    defer objUnref(store); // dialog takes its own ref
    for (fs) |flt| {
        const ff = gtk.FileFilter.new();
        if (flt.name) |nm| {
            const nz = alloc.dupeZ(u8, nm) catch null;
            if (nz) |n| {
                gtk.FileFilter.setName(ff, n.ptr);
                alloc.free(n);
            }
        }
        if (flt.extensions) |exts| {
            for (exts) |ext| {
                const ez = alloc.dupeZ(u8, ext) catch continue;
                defer alloc.free(ez);
                gtk.FileFilter.addSuffix(ff, ez.ptr);
            }
        }
        gio.ListStore.append(store, ff.as(gobject.Object));
        gobject.Object.unref(ff.as(gobject.Object)); // store holds its own ref
    }
    gtk.FileDialog.setFilters(dialog, store.as(gio.ListModel));
}

const AlertJob = struct { ctx: *abi.NdContext, id: u32, dialog: *gtk.AlertDialog, default_button: c_int };

fn showMessage(ctx: *abi.NdContext, id: u32, p: []const u8) void {
    const P = struct {
        message: []const u8 = "",
        detail: ?[]const u8 = null,
        level: ?[]const u8 = null,
        buttons: ?[]const []const u8 = null,
        defaultButton: ?i64 = null,
    };
    const parsed = parseParams(P, p) orelse return respond(ctx, id, false, "invalid params");
    defer parsed.deinit();
    const v = parsed.value;

    // Build with an empty format string (never the message itself — a message
    // with a `%` would be interpreted as a printf directive) and set the real
    // text through the non-formatting setter.
    const dialog = gtk.AlertDialog.new("");
    const msg_z = alloc.dupeZ(u8, v.message) catch {
        objUnref(dialog);
        return respond(ctx, id, false, "oom");
    };
    gtk.AlertDialog.setMessage(dialog, msg_z.ptr);
    alloc.free(msg_z);
    if (v.detail) |d| {
        const dz = alloc.dupeZ(u8, d) catch null;
        if (dz) |dd| {
            gtk.AlertDialog.setDetail(dialog, dd.ptr);
            alloc.free(dd);
        }
    }
    // GtkAlertDialog has no per-severity styling, so `level` is accepted but
    // has no visual effect on this backend.
    setAlertButtons(dialog, v.buttons);
    const default_button: c_int = if (v.defaultButton) |db| @intCast(db) else 0;
    gtk.AlertDialog.setDefaultButton(dialog, default_button);

    const job = alloc.create(AlertJob) catch {
        objUnref(dialog);
        return respond(ctx, id, false, "oom");
    };
    job.* = .{ .ctx = ctx, .id = id, .dialog = dialog, .default_button = default_button };
    gtk.AlertDialog.choose(dialog, activeWindow(), null, &cbAlert, job);
}

/// Builds the NULL-terminated C strv GtkAlertDialog wants (default `["OK"]`).
/// setButtons copies the array (g_strdupv), so the dup'd strings can be freed
/// immediately after.
fn setAlertButtons(dialog: *gtk.AlertDialog, buttons: ?[]const []const u8) void {
    const default_btns = [_][]const u8{"OK"};
    const btns: []const []const u8 = buttons orelse &default_btns;
    const cstrs = alloc.alloc(?[*:0]const u8, btns.len + 1) catch return;
    defer alloc.free(cstrs);
    var made: usize = 0;
    defer for (cstrs[0..made]) |c| if (c) |cc| alloc.free(std.mem.span(cc));
    for (btns, 0..) |b, i| {
        const z = alloc.dupeZ(u8, b) catch break;
        cstrs[i] = z.ptr;
        made += 1;
    }
    cstrs[made] = null;
    gtk.AlertDialog.setButtons(dialog, @ptrCast(cstrs.ptr));
}

fn cbAlert(_: ?*gobject.Object, res: *gio.AsyncResult, user_data: ?*anyopaque) callconv(.c) void {
    const job: *AlertJob = @ptrCast(@alignCast(user_data.?));
    defer {
        objUnref(job.dialog);
        alloc.destroy(job);
    }
    var err: ?*glib.Error = null;
    const idx = gtk.AlertDialog.chooseFinish(job.dialog, res, &err);
    if (err) |e| {
        e.free();
        // Dismissed (Escape / close) — no button clicked; fall back to default.
        respondValue(job.ctx, job.id, @as(i64, job.default_button));
        return;
    }
    respondValue(job.ctx, job.id, @as(i64, idx));
}

// ============================================================================
// notification.*
// ============================================================================

var notif_counter: u64 = 0;
var notif_action_registered = false;

fn notificationShow(ctx: *abi.NdContext, id: u32, p: []const u8) void {
    const P = struct { title: []const u8 = "", body: ?[]const u8 = null };
    const parsed = parseParams(P, p) orelse return respond(ctx, id, false, "invalid params");
    defer parsed.deinit();

    const app = gio.Application.getDefault() orelse return respond(ctx, id, false, "notifications require an application id");
    if (gio.Application.getApplicationId(app) == null) return respond(ctx, id, false, "notifications require an application id");
    ensureNotifAction(app);

    notif_counter += 1;
    const nid = std.fmt.allocPrintSentinel(alloc, "nd-notif-{d}", .{notif_counter}, 0) catch return respond(ctx, id, false, "oom");
    defer alloc.free(nid);
    const title_z = alloc.dupeZ(u8, parsed.value.title) catch return respond(ctx, id, false, "oom");
    defer alloc.free(title_z);

    const notif = gio.Notification.new(title_z.ptr);
    defer objUnref(notif);
    if (parsed.value.body) |b| {
        const bz = alloc.dupeZ(u8, b) catch return respond(ctx, id, false, "oom");
        defer alloc.free(bz);
        gio.Notification.setBody(notif, bz.ptr);
    }
    // `::<id>` is the string-target shorthand of a detailed action name; the id
    // is delivered as the action parameter when the notification is clicked.
    const action = std.fmt.allocPrintSentinel(alloc, "app.nd-notification-clicked::{s}", .{nid}, 0) catch return respond(ctx, id, false, "oom");
    defer alloc.free(action);
    gio.Notification.setDefaultAction(notif, action.ptr);

    gio.Application.sendNotification(app, nid.ptr, notif);
    respondValue(ctx, id, @as([]const u8, nid));
}

fn ensureNotifAction(app: *gio.Application) void {
    if (notif_action_registered) return;
    notif_action_registered = true;
    const vt = glib.VariantType.new("s"); // copied by g_simple_action_new
    const action = gio.SimpleAction.new("nd-notification-clicked", vt);
    _ = gio.SimpleAction.signals.activate.connect(action, ?*anyopaque, &onNotifClick, null, .{});
    gio.ActionMap.addAction(app.as(gio.ActionMap), action.as(gio.Action));
    objUnref(action); // map holds its own ref
}

fn onNotifClick(_: *gio.SimpleAction, param: ?*glib.Variant, _: ?*anyopaque) callconv(.c) void {
    const ctx = the_ctx orelse return;
    const pv = param orelse return;
    const s = glib.Variant.getString(pv, null);
    emitSystemEvent(ctx, "notification.click", .{ .id = std.mem.span(s) });
}

// ============================================================================
// recent.*
// ============================================================================

fn recentAdd(ctx: *abi.NdContext, id: u32, p: []const u8) void {
    const P = struct { path: []const u8 = "" };
    const parsed = parseParams(P, p) orelse return respond(ctx, id, false, "invalid params");
    defer parsed.deinit();
    if (parsed.value.path.len == 0) return respond(ctx, id, false, "path required");
    const pz = alloc.dupeZ(u8, parsed.value.path) catch return respond(ctx, id, false, "oom");
    defer alloc.free(pz);
    const file = gio.File.newForPath(pz.ptr);
    defer objUnref(file);
    const uri = gio.File.getUri(file);
    defer glib.free(uri);
    _ = gtk.RecentManager.addItem(gtk.RecentManager.getDefault(), uri);
    respond(ctx, id, true, "null");
}

fn recentClear(ctx: *abi.NdContext, id: u32) void {
    var err: ?*glib.Error = null;
    _ = gtk.RecentManager.purgeItems(gtk.RecentManager.getDefault(), &err);
    if (err) |e| e.free();
    respond(ctx, id, true, "null");
}

// ============================================================================
// credentials.* — libsecret, dlopen'd (never hard-linked; mirrors webview.zig)
// ============================================================================

// SecretSchema / attribute layout mirrors libsecret/secret-schema.h so the
// dlopen'd `*v_sync` calls (non-variadic, GHashTable-based — no vararg ABI
// risk) can match stored items by (service, account) string attributes.
const SecretSchemaAttribute = extern struct { name: ?[*:0]const u8, typ: c_int };
const SecretSchema = extern struct {
    name: [*:0]const u8,
    flags: c_int = 0,
    attributes: [32]SecretSchemaAttribute,
    reserved: c_int = 0,
    reserved1: ?*anyopaque = null,
    reserved2: ?*anyopaque = null,
    reserved3: ?*anyopaque = null,
    reserved4: ?*anyopaque = null,
    reserved5: ?*anyopaque = null,
    reserved6: ?*anyopaque = null,
    reserved7: ?*anyopaque = null,
};

const secret_schema: SecretSchema = blk: {
    var attrs = [_]SecretSchemaAttribute{.{ .name = null, .typ = 0 }} ** 32;
    attrs[0] = .{ .name = "service", .typ = 0 }; // SECRET_SCHEMA_ATTRIBUTE_STRING
    attrs[1] = .{ .name = "account", .typ = 0 };
    break :blk .{ .name = "dev.nativedesktop.credentials", .attributes = attrs };
};

const FnStorev = *const fn (*const SecretSchema, *glib.HashTable, ?[*:0]const u8, [*:0]const u8, [*:0]const u8, ?*anyopaque, ?*?*glib.Error) callconv(.c) c_int;
const FnLookupv = *const fn (*const SecretSchema, *glib.HashTable, ?*anyopaque, ?*?*glib.Error) callconv(.c) ?[*:0]u8;
const FnClearv = *const fn (*const SecretSchema, *glib.HashTable, ?*anyopaque, ?*?*glib.Error) callconv(.c) c_int;
const FnFree = *const fn (?[*:0]u8) callconv(.c) void;

const SecretApi = struct {
    storev: FnStorev,
    lookupv: FnLookupv,
    clearv: FnClearv,
    free: ?FnFree,
};

var secret_api: ?SecretApi = null;
var secret_load_attempted = false;
var secret_lib: std.DynLib = undefined;

fn loadSecret() ?*const SecretApi {
    if (secret_load_attempted) return if (secret_api != null) &secret_api.? else null;
    secret_load_attempted = true;
    const candidates = [_][]const u8{ "libsecret-1.so.0", "libsecret-1.so", "libsecret-1.dylib" };
    for (candidates) |name| {
        var lib = std.DynLib.open(name) catch continue;
        const storev = lib.lookup(FnStorev, "secret_password_storev_sync") orelse {
            lib.close();
            continue;
        };
        const lookupv = lib.lookup(FnLookupv, "secret_password_lookupv_sync") orelse {
            lib.close();
            continue;
        };
        const clearv = lib.lookup(FnClearv, "secret_password_clearv_sync") orelse {
            lib.close();
            continue;
        };
        secret_lib = lib;
        secret_api = .{ .storev = storev, .lookupv = lookupv, .clearv = clearv, .free = lib.lookup(FnFree, "secret_password_free") };
        std.debug.print("ND_CREDENTIALS_ENGINE libsecret ({s})\n", .{name});
        return &secret_api.?;
    }
    return null;
}

fn constPtr(p: [*:0]const u8) *anyopaque {
    return @ptrCast(@constCast(p));
}

/// {service, account} attribute table matching `secret_schema`. Keys are the
/// static attribute names; values borrow the caller's live NUL-terminated
/// buffers, so they must outlive the sync call.
fn buildAttrs(service: [*:0]const u8, account: [*:0]const u8) *glib.HashTable {
    const hash_fn: glib.HashFunc = @ptrCast(&glib.strHash);
    const eq_fn: glib.EqualFunc = @ptrCast(&glib.strEqual);
    const t = glib.HashTable.new(hash_fn, eq_fn);
    _ = glib.HashTable.insert(t, constPtr("service"), constPtr(service));
    _ = glib.HashTable.insert(t, constPtr("account"), constPtr(account));
    return t;
}

const ServiceAccount = struct { service: []const u8 = "", account: []const u8 = "" };

fn credentialsSet(ctx: *abi.NdContext, id: u32, p: []const u8) void {
    const P = struct { service: []const u8 = "", account: []const u8 = "", secret: []const u8 = "" };
    const parsed = parseParams(P, p) orelse return respond(ctx, id, false, "invalid params");
    defer parsed.deinit();
    const api = loadSecret() orelse return respond(ctx, id, false, "credential store unavailable: libsecret not found");

    const svc = alloc.dupeZ(u8, parsed.value.service) catch return respond(ctx, id, false, "oom");
    defer alloc.free(svc);
    const acc = alloc.dupeZ(u8, parsed.value.account) catch return respond(ctx, id, false, "oom");
    defer alloc.free(acc);
    const sec = alloc.dupeZ(u8, parsed.value.secret) catch return respond(ctx, id, false, "oom");
    defer alloc.free(sec);
    const label = std.fmt.allocPrintSentinel(alloc, "{s} ({s})", .{ parsed.value.service, parsed.value.account }, 0) catch return respond(ctx, id, false, "oom");
    defer alloc.free(label);

    const attrs = buildAttrs(svc.ptr, acc.ptr);
    defer glib.HashTable.unref(attrs);
    var err: ?*glib.Error = null;
    const ok = api.storev(&secret_schema, attrs, null, label.ptr, sec.ptr, null, &err);
    if (ok == 0) {
        const msg: []const u8 = if (err) |e| (if (e.f_message) |m| std.mem.span(m) else "credential store failed") else "credential store failed";
        respond(ctx, id, false, msg);
        if (err) |e| e.free();
        return;
    }
    respond(ctx, id, true, "null");
}

fn credentialsGet(ctx: *abi.NdContext, id: u32, p: []const u8) void {
    const parsed = parseParams(ServiceAccount, p) orelse return respond(ctx, id, false, "invalid params");
    defer parsed.deinit();
    const api = loadSecret() orelse return respond(ctx, id, false, "credential store unavailable: libsecret not found");

    const svc = alloc.dupeZ(u8, parsed.value.service) catch return respond(ctx, id, false, "oom");
    defer alloc.free(svc);
    const acc = alloc.dupeZ(u8, parsed.value.account) catch return respond(ctx, id, false, "oom");
    defer alloc.free(acc);

    const attrs = buildAttrs(svc.ptr, acc.ptr);
    defer glib.HashTable.unref(attrs);
    var err: ?*glib.Error = null;
    const pw = api.lookupv(&secret_schema, attrs, null, &err);
    if (pw) |secret_z| {
        respondValue(ctx, id, std.mem.span(secret_z));
        secretFree(api, secret_z);
        return;
    }
    if (err) |e| {
        const msg: []const u8 = if (e.f_message) |m| std.mem.span(m) else "credential lookup failed";
        respond(ctx, id, false, msg);
        e.free();
        return;
    }
    respond(ctx, id, true, "null"); // not found
}

fn credentialsDelete(ctx: *abi.NdContext, id: u32, p: []const u8) void {
    const parsed = parseParams(ServiceAccount, p) orelse return respond(ctx, id, false, "invalid params");
    defer parsed.deinit();
    const api = loadSecret() orelse return respond(ctx, id, false, "credential store unavailable: libsecret not found");

    const svc = alloc.dupeZ(u8, parsed.value.service) catch return respond(ctx, id, false, "oom");
    defer alloc.free(svc);
    const acc = alloc.dupeZ(u8, parsed.value.account) catch return respond(ctx, id, false, "oom");
    defer alloc.free(acc);

    const attrs = buildAttrs(svc.ptr, acc.ptr);
    defer glib.HashTable.unref(attrs);
    var err: ?*glib.Error = null;
    _ = api.clearv(&secret_schema, attrs, null, &err); // idempotent: absent is not an error
    if (err) |e| e.free();
    respond(ctx, id, true, "null");
}

fn secretFree(api: *const SecretApi, p: [*:0]u8) void {
    if (api.free) |f| f(p) else glib.free(p);
}

// ============================================================================
// Events: window file drop, OS launch delivery
// ============================================================================

/// Attaches a file drop target to a Window widget (called from `backend`'s
/// create path for kind=="Window"). A drop emits `fileDrop` with the absolute
/// paths. Multi-window: the node id isn't available at create time, so
/// `windowId` is 0 (per the seam contract's fallback).
pub fn attachWindowDropTarget(window: *gtk.Widget) void {
    const target = gtk.DropTarget.new(gdk.FileList.getGObjectType(), .{ .copy = true });
    _ = gtk.DropTarget.signals.drop.connect(target, ?*anyopaque, &cbDrop, null, .{});
    gtk.Widget.addController(window, target.as(gtk.EventController));
}

fn cbDrop(_: *gtk.DropTarget, value: *gobject.Value, _: f64, _: f64, _: ?*anyopaque) callconv(.c) c_int {
    const ctx = the_ctx orelse return 0;
    const boxed = gobject.Value.getBoxed(value) orelse return 0;
    const file_list: *gdk.FileList = @ptrCast(@alignCast(boxed));
    const files = gdk.FileList.getFiles(file_list); // borrowed; do not free

    var n: usize = 0;
    var node: ?*glib.SList = files;
    while (node) |nd| : (node = nd.f_next) n += 1;
    if (n == 0) return 0;

    const paths = alloc.alloc([]const u8, n) catch return 0;
    var count: usize = 0;
    defer {
        for (paths[0..count]) |it| alloc.free(it);
        alloc.free(paths);
    }
    node = files;
    while (node) |nd| : (node = nd.f_next) {
        const file: *gio.File = @ptrCast(@alignCast(nd.f_data orelse continue));
        const pz = gio.File.getPath(file) orelse continue;
        defer glib.free(pz);
        const dup = alloc.dupe(u8, std.mem.span(pz)) catch continue;
        paths[count] = dup;
        count += 1;
    }
    if (count == 0) return 0;
    emitSystemEvent(ctx, "fileDrop", .{ .paths = paths[0..count], .windowId = @as(u32, 0) });
    return 1; // drop accepted
}

/// GApplication `open` delivery (main.zig connects the signal; the app is
/// launched with HANDLES_OPEN). Local files (a resolvable path) batch into one
/// `app.openFile`; non-local schemes emit `app.openUrl` per URI.
pub fn handleOpen(ctx: *abi.NdContext, files: [*]*gio.File, n_files: c_int) void {
    const count: usize = @intCast(@max(n_files, 0));
    if (count == 0) return;
    const paths = alloc.alloc([]const u8, count) catch return;
    var pcount: usize = 0;
    defer {
        for (paths[0..pcount]) |it| alloc.free(it);
        alloc.free(paths);
    }
    var i: usize = 0;
    while (i < count) : (i += 1) {
        const file = files[i];
        if (gio.File.getPath(file)) |pz| {
            defer glib.free(pz);
            const dup = alloc.dupe(u8, std.mem.span(pz)) catch continue;
            paths[pcount] = dup;
            pcount += 1;
        } else {
            const uz = gio.File.getUri(file);
            defer glib.free(uz);
            emitSystemEvent(ctx, "app.openUrl", .{ .url = std.mem.span(uz) });
        }
    }
    if (pcount > 0) emitSystemEvent(ctx, "app.openFile", .{ .paths = paths[0..pcount] });
}
