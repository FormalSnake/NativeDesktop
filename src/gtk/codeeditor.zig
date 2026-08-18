// GTK/Linux host-side of the <codeeditor> widget: GtkSourceView, which is what
// GNOME Builder and GNOME Text Editor are built on, so this is the native
// answer rather than a re-implementation of one.
//
// GtkSourceView is deliberately NOT a link-time dependency (the house rule
// webview.zig's WebKitGTK, system.zig's libsecret and audio.zig's GStreamer
// already follow): the dozen C entry points are resolved once with
// std.DynLib, build.zig stays untouched, and a machine without the library
// degrades to a plain GtkTextView instead of failing to link. GtkSourceView
// IS a GtkTextView subclass, so the two modes differ only in what the loaded
// symbols add on top: line numbers, the syntax engine and the style scheme.
//
// What degrades and what does not, when the library is missing:
//   text, readOnly, tabWidth, changed, cursorMoved  — identical (GtkTextView)
//   diagnostics, diagnosticClicked                  — identical (GtkTextTags)
//   language, theme, showLineNumbers                — inert
//
// Diagnostics are GtkTextTags on the buffer, not GtkSourceMarks, for exactly
// that reason: one implementation serves both modes, and the squiggle is the
// same Pango error underline either way.
//
// LSP is out of scope BY DESIGN. The widget renders the `diagnostics` array it
// is handed and never analyses the text — a language server belongs in
// app-side TS, the same division <table> draws between its rows and whatever
// produced them.
const std = @import("std");
const gtk = @import("gtk");
const gobject = @import("gobject");
const glib = @import("glib");
const pango = @import("pango");
const protocol = @import("../protocol.zig");

pub const EmitFn = *const fn (node_id: u32, name: []const u8, payload: protocol.EventPayload) void;

const alloc = std.heap.page_allocator;

var emit: ?EmitFn = null;

/// GtkScrolledWindow (the tracked handle) -> its store.
var stores: std.AutoHashMapUnmanaged(usize, *Store) = .empty;

// ============================================================================
// GtkSourceView dlopen table (lazy, one-time — mirrors audio.zig's loadGst)
// ============================================================================

const FnVoid = *const fn () callconv(.c) void;
const FnNewView = *const fn () callconv(.c) ?*gtk.Widget;
const FnSetFlag = *const fn (?*anyopaque, c_int) callconv(.c) void;
const FnSetUint = *const fn (?*anyopaque, c_uint) callconv(.c) void;
const FnGetDefault = *const fn () callconv(.c) ?*anyopaque;
const FnLookup = *const fn (?*anyopaque, [*:0]const u8) callconv(.c) ?*anyopaque;
const FnSetObject = *const fn (?*anyopaque, ?*anyopaque) callconv(.c) void;

const SourceApi = struct {
    view_new: FnNewView,
    set_show_line_numbers: FnSetFlag,
    set_highlight_current_line: FnSetFlag,
    set_auto_indent: FnSetFlag,
    set_tab_width: FnSetUint,
    lang_manager_default: FnGetDefault,
    lang_manager_get: FnLookup,
    buffer_set_language: FnSetObject,
    scheme_manager_default: FnGetDefault,
    scheme_manager_get: FnLookup,
    buffer_set_style_scheme: FnSetObject,
    // gtk_source_init exists to register the library's resources; optional so
    // an older packaging that does not export it still gives us the editor.
    init: ?FnVoid,
};

var source_api: ?SourceApi = null;
var source_load_attempted = false;
// Held open for the process lifetime: the syntax engine keeps registry state
// (languages, style schemes) that is not safe to unload once initialized.
var source_lib: std.DynLib = undefined;

fn lookupAll(lib: *std.DynLib) ?SourceApi {
    return .{
        .view_new = lib.lookup(FnNewView, "gtk_source_view_new") orelse return null,
        .set_show_line_numbers = lib.lookup(FnSetFlag, "gtk_source_view_set_show_line_numbers") orelse return null,
        .set_highlight_current_line = lib.lookup(FnSetFlag, "gtk_source_view_set_highlight_current_line") orelse return null,
        .set_auto_indent = lib.lookup(FnSetFlag, "gtk_source_view_set_auto_indent") orelse return null,
        .set_tab_width = lib.lookup(FnSetUint, "gtk_source_view_set_tab_width") orelse return null,
        .lang_manager_default = lib.lookup(FnGetDefault, "gtk_source_language_manager_get_default") orelse return null,
        .lang_manager_get = lib.lookup(FnLookup, "gtk_source_language_manager_get_language") orelse return null,
        .buffer_set_language = lib.lookup(FnSetObject, "gtk_source_buffer_set_language") orelse return null,
        .scheme_manager_default = lib.lookup(FnGetDefault, "gtk_source_style_scheme_manager_get_default") orelse return null,
        .scheme_manager_get = lib.lookup(FnLookup, "gtk_source_style_scheme_manager_get_scheme") orelse return null,
        .buffer_set_style_scheme = lib.lookup(FnSetObject, "gtk_source_buffer_set_style_scheme") orelse return null,
        .init = lib.lookup(FnVoid, "gtk_source_init"),
    };
}

fn loadSource() ?*const SourceApi {
    if (source_load_attempted) return if (source_api != null) &source_api.? else null;
    source_load_attempted = true;
    // The .0 soname is what distros ship; the dylib variants cover a brew/mac
    // GTK stack that also carries GtkSourceView.
    const candidates = [_][]const u8{
        "libgtksourceview-5.so.0",
        "libgtksourceview-5.so",
        "libgtksourceview-5.0.dylib",
        "libgtksourceview-5.dylib",
    };
    for (candidates) |name| {
        var lib = std.DynLib.open(name) catch continue;
        if (lookupAll(&lib)) |api| {
            source_lib = lib;
            source_api = api;
            if (api.init) |f| f();
            std.debug.print("ND_CODE_ENGINE gtksourceview ({s})\n", .{name});
            return &source_api.?;
        }
        lib.close();
    }
    std.debug.print("ND_CODE_ENGINE textview (gtksourceview not found)\n", .{});
    return null;
}

// ============================================================================
// Store
// ============================================================================

const Severity = enum(u2) { err, warning, info, hint };

fn parseSeverity(name: []const u8) Severity {
    if (std.mem.eql(u8, name, "warning")) return .warning;
    if (std.mem.eql(u8, name, "info")) return .info;
    if (std.mem.eql(u8, name, "hint")) return .hint;
    return .err;
}

const Diagnostic = struct {
    line: i64 = 1,
    column: i64 = 1,
    severity: Severity = .err,
    severity_name: [:0]u8,
    message: [:0]u8,
};

const Store = struct {
    node_id: u32 = 0,
    view: *gtk.TextView,
    /// True when the view is a real GtkSourceView.
    highlighted: bool = false,
    diagnostics: std.ArrayListUnmanaged(Diagnostic) = .empty,
    /// One tag per severity, created on the buffer's tag table the first time
    /// a diagnostic lands. Reused so an update never grows the table.
    tags: [4]?*gtk.TextTag = .{ null, null, null, null },
    /// Set while applyProps writes `text`, so the buffer's own changed signal
    /// cannot echo a React-driven write back as a user edit.
    suppress: bool = false,

    fn clearDiagnostics(self: *Store) void {
        for (self.diagnostics.items) |d| {
            alloc.free(d.severity_name);
            alloc.free(d.message);
        }
        self.diagnostics.clearRetainingCapacity();
    }

    fn deinit(self: *Store) void {
        self.clearDiagnostics();
        self.diagnostics.deinit(alloc);
    }
};

fn storeOf(widget: *gtk.Widget) ?*Store {
    return stores.get(@intFromPtr(widget));
}

// ============================================================================
// Prop helpers (module-local, the table.zig/chart.zig idiom)
// ============================================================================

fn propStr(props: ?std.json.Value, key: []const u8) ?[]const u8 {
    const v = props orelse return null;
    if (v != .object) return null;
    return switch (v.object.get(key) orelse return null) {
        .string => |s| s,
        else => null,
    };
}

fn propInt(props: ?std.json.Value, key: []const u8) ?i64 {
    const v = props orelse return null;
    if (v != .object) return null;
    return switch (v.object.get(key) orelse return null) {
        .integer => |i| i,
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

fn propArray(props: ?std.json.Value, key: []const u8) ?std.json.Array {
    const v = props orelse return null;
    if (v != .object) return null;
    return switch (v.object.get(key) orelse return null) {
        .array => |a| a,
        else => null,
    };
}

fn objStr(obj: std.json.ObjectMap, key: []const u8) ?[]const u8 {
    return switch (obj.get(key) orelse return null) {
        .string => |s| s,
        else => null,
    };
}

fn objInt(obj: std.json.ObjectMap, key: []const u8) ?i64 {
    return switch (obj.get(key) orelse return null) {
        .integer => |i| i,
        else => null,
    };
}

fn asObject(widget: anytype) *gobject.Object {
    return @ptrCast(@alignCast(widget));
}

// ============================================================================
// Language and theme
// ============================================================================

/// The app's `language` string is tried against GtkSourceView's own language
/// ids first, so any id the installed language pack ships works without this
/// table. The aliases only cover the names where the common spelling and the
/// .lang id genuinely differ.
const LANGUAGE_ALIASES = [_]struct { from: []const u8, to: [:0]const u8 }{
    .{ .from = "javascript", .to = "js" },
    .{ .from = "jsx", .to = "js" },
    .{ .from = "tsx", .to = "typescript" },
    .{ .from = "c++", .to = "cpp" },
    .{ .from = "c#", .to = "c-sharp" },
    .{ .from = "bash", .to = "sh" },
    .{ .from = "shell", .to = "sh" },
    .{ .from = "yml", .to = "yaml" },
    .{ .from = "md", .to = "markdown" },
};

fn applyLanguage(api: *const SourceApi, buffer: *gtk.TextBuffer, id: []const u8, dupeZ: *const fn ([]const u8) [:0]const u8) void {
    if (id.len == 0) {
        api.buffer_set_language(buffer, null);
        return;
    }
    const manager = api.lang_manager_default() orelse return;
    if (api.lang_manager_get(manager, dupeZ(id).ptr)) |lang| {
        api.buffer_set_language(buffer, lang);
        return;
    }
    for (LANGUAGE_ALIASES) |a| {
        if (!std.mem.eql(u8, a.from, id)) continue;
        if (api.lang_manager_get(manager, a.to.ptr)) |lang| api.buffer_set_language(buffer, lang);
        return;
    }
    // Unknown id: plain text beats guessing at the app's expense.
    api.buffer_set_language(buffer, null);
}

fn applyTheme(api: *const SourceApi, buffer: *gtk.TextBuffer, id: []const u8, dupeZ: *const fn ([]const u8) [:0]const u8) void {
    if (id.len == 0) return; // no theme prop: the scheme manager's default follows the desktop
    const manager = api.scheme_manager_default() orelse return;
    const scheme = api.scheme_manager_get(manager, dupeZ(id).ptr) orelse return;
    api.buffer_set_style_scheme(buffer, scheme);
}

// ============================================================================
// Diagnostics
// ============================================================================

fn severityTag(store: *Store, buffer: *gtk.TextBuffer, severity: Severity) ?*gtk.TextTag {
    const slot = @intFromEnum(severity);
    if (store.tags[slot]) |t| return t;
    const tag = gtk.TextTag.new(null);
    // A Pango error underline is the squiggle every native editor draws; the
    // colour rides GtkSourceView's scheme when one is loaded, so only the
    // shape is set here.
    var v: gobject.Value = std.mem.zeroes(gobject.Value);
    _ = gobject.Value.init(&v, pango.Underline.getGObjectType());
    gobject.Value.setEnum(&v, @intFromEnum(switch (severity) {
        .err, .warning => pango.Underline.@"error",
        .info, .hint => pango.Underline.single,
    }));
    gobject.Object.setProperty(asObject(tag), "underline", &v);
    gobject.Value.unset(&v);
    _ = gtk.TextTagTable.add(gtk.TextBuffer.getTagTable(buffer), tag);
    store.tags[slot] = tag;
    return tag;
}

fn ingestDiagnostics(store: *Store, arr: std.json.Array) void {
    store.clearDiagnostics();
    for (arr.items) |item| {
        if (item != .object) continue;
        const obj = item.object;
        const message = objStr(obj, "message") orelse "";
        const severity = objStr(obj, "severity") orelse "error";
        const owned_msg = alloc.dupeZ(u8, message) catch continue;
        const owned_sev = alloc.dupeZ(u8, severity) catch {
            alloc.free(owned_msg);
            continue;
        };
        store.diagnostics.append(alloc, .{
            .line = objInt(obj, "line") orelse 1,
            .column = objInt(obj, "column") orelse 1,
            .severity = parseSeverity(severity),
            .severity_name = owned_sev,
            .message = owned_msg,
        }) catch {
            alloc.free(owned_msg);
            alloc.free(owned_sev);
        };
    }
}

/// Repaints every squiggle from the store. Only the severity tags this module
/// owns are cleared — `remove_all_tags` would also strip the syntax engine's
/// own tags out from under it.
fn repaintDiagnostics(store: *Store) void {
    const buffer = gtk.TextView.getBuffer(store.view);
    var start: gtk.TextIter = undefined;
    var end: gtk.TextIter = undefined;
    gtk.TextBuffer.getStartIter(buffer, &start);
    gtk.TextBuffer.getEndIter(buffer, &end);
    for (store.tags) |maybe_tag| {
        if (maybe_tag) |tag| gtk.TextBuffer.removeTag(buffer, tag, &start, &end);
    }

    const lines = gtk.TextBuffer.getLineCount(buffer);
    for (store.diagnostics.items) |d| {
        // 1-based on the wire (what every compiler and language server
        // reports), 0-based in GtkTextBuffer.
        const line: c_int = @intCast(@max(0, @min(@as(i64, lines) - 1, d.line - 1)));
        const column: c_int = @intCast(@max(0, d.column - 1));
        const tag = severityTag(store, buffer, d.severity) orelse continue;
        var from: gtk.TextIter = undefined;
        var to: gtk.TextIter = undefined;
        _ = gtk.TextBuffer.getIterAtLine(buffer, &from, line);
        _ = gtk.TextBuffer.getIterAtLineOffset(buffer, &to, line, column);
        // A column past the line end clamps back to the line start, which is
        // the only way to keep the squiggle on the line the app named.
        if (gtk.TextIter.getLineOffset(&to) >= column) from = to;
        to = from;
        if (gtk.TextIter.forwardToLineEnd(&to) == 0) gtk.TextBuffer.getEndIter(buffer, &to);
        gtk.TextBuffer.applyTag(buffer, tag, &from, &to);
    }
}

// ============================================================================
// Signals
// ============================================================================

fn cbChanged(buffer: *gtk.TextBuffer, data: ?*anyopaque) callconv(.c) void {
    const store: *Store = @ptrCast(@alignCast(data.?));
    if (store.suppress or store.node_id == 0) return;
    const f = emit orelse return;
    var start: gtk.TextIter = undefined;
    var end: gtk.TextIter = undefined;
    gtk.TextBuffer.getStartIter(buffer, &start);
    gtk.TextBuffer.getEndIter(buffer, &end);
    const text = gtk.TextBuffer.getText(buffer, &start, &end, 0);
    defer glib.free(@ptrCast(text));
    f(store.node_id, "changed", .{ .text = std.mem.span(text) });
}

/// notify:: handlers get (object, pspec, user_data).
fn cbCursorMoved(obj: *gobject.Object, _: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
    const store: *Store = @ptrCast(@alignCast(data.?));
    if (store.node_id == 0) return;
    const f = emit orelse return;
    const buffer: *gtk.TextBuffer = @ptrCast(@alignCast(obj));
    var iter: gtk.TextIter = undefined;
    gtk.TextBuffer.getIterAtMark(buffer, &iter, gtk.TextBuffer.getInsert(buffer));
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    payload.put(alloc, "line", .{ .integer = @as(i64, gtk.TextIter.getLine(&iter)) + 1 }) catch {};
    payload.put(alloc, "column", .{ .integer = @as(i64, gtk.TextIter.getLineOffset(&iter)) + 1 }) catch {};
    f(store.node_id, "cursorMoved", .{ .data = .{ .object = payload } });
}

/// A click reports the diagnostic on the clicked LINE, not the exact glyph:
/// the squiggle spans the line from its column, and asking the user to hit
/// one character is not a target anyone can reach.
fn cbClicked(gesture: *gtk.GestureClick, _: c_int, x: f64, y: f64, data: ?*anyopaque) callconv(.c) void {
    const store: *Store = @ptrCast(@alignCast(data.?));
    if (store.node_id == 0 or store.diagnostics.items.len == 0) return;
    const f = emit orelse return;
    _ = gesture;

    var bx: c_int = 0;
    var by: c_int = 0;
    gtk.TextView.windowToBufferCoords(store.view, .widget, @intFromFloat(x), @intFromFloat(y), &bx, &by);
    var iter: gtk.TextIter = undefined;
    _ = gtk.TextView.getIterAtLocation(store.view, &iter, bx, by);
    const line: i64 = @as(i64, gtk.TextIter.getLine(&iter)) + 1;

    for (store.diagnostics.items) |d| {
        if (d.line != line) continue;
        var payload: std.json.ObjectMap = .empty;
        defer payload.deinit(alloc);
        payload.put(alloc, "line", .{ .integer = d.line }) catch {};
        payload.put(alloc, "column", .{ .integer = d.column }) catch {};
        payload.put(alloc, "severity", .{ .string = d.severity_name }) catch {};
        payload.put(alloc, "message", .{ .string = d.message }) catch {};
        f(store.node_id, "diagnosticClicked", .{ .data = .{ .object = payload } });
        return;
    }
}

fn cbDestroyed(widget: *gtk.Widget, _: ?*anyopaque) callconv(.c) void {
    if (stores.fetchRemove(@intFromPtr(widget))) |kv| {
        kv.value.deinit();
        alloc.destroy(kv.value);
    }
}

// ============================================================================
// Generated-dispatcher seam
// ============================================================================

pub fn create(props: ?std.json.Value, dupeZ: *const fn ([]const u8) [:0]const u8) *gtk.Widget {
    const api = loadSource();
    const view: *gtk.TextView = blk: {
        if (api) |a| {
            if (a.view_new()) |w| break :blk @ptrCast(@alignCast(w));
        }
        break :blk gtk.TextView.new();
    };
    gtk.TextView.setMonospace(view, 1);
    // Code does not reflow: a wrapped line breaks the column arithmetic every
    // diagnostic and every cursor report is expressed in.
    gtk.TextView.setWrapMode(view, .none);

    // Scroller-as-handle (src/gtk/table.zig, the TextArea arm): a bare text
    // view has no boundary and no scrolling, so the tracked widget is the
    // frame around it, and everything reaching for the view goes through the
    // store.
    const sw = gtk.ScrolledWindow.new();
    gtk.ScrolledWindow.setChild(sw, view.as(gtk.Widget));
    gtk.ScrolledWindow.setHasFrame(sw, 1);
    gtk.ScrolledWindow.setMinContentHeight(sw, 160);
    const handle = sw.as(gtk.Widget);
    gtk.Widget.setHexpand(handle, 1);
    gtk.Widget.setVexpand(handle, 1);

    const store = alloc.create(Store) catch return handle;
    store.* = .{ .view = view, .highlighted = api != null };
    stores.put(alloc, @intFromPtr(handle), store) catch {
        alloc.destroy(store);
        return handle;
    };
    _ = gtk.Widget.signals.destroy.connect(handle, ?*anyopaque, &cbDestroyed, null, .{});

    const buffer = gtk.TextView.getBuffer(view);
    _ = gtk.TextBuffer.signals.changed.connect(buffer, ?*anyopaque, &cbChanged, store, .{});
    _ = gobject.signalConnectData(asObject(buffer), "notify::cursor-position", @ptrCast(&cbCursorMoved), store, null, .{});
    const click = gtk.GestureClick.new();
    gtk.GestureSingle.setButton(click.as(gtk.GestureSingle), 1);
    _ = gtk.GestureClick.signals.released.connect(click, ?*anyopaque, &cbClicked, store, .{});
    gtk.Widget.addController(view.as(gtk.Widget), click.as(gtk.EventController));

    if (api) |a| {
        a.set_highlight_current_line(view, 1);
        a.set_auto_indent(view, 1);
    }
    ingest(store, props, dupeZ);
    return handle;
}

pub fn applyProps(widget: *gtk.Widget, props: ?std.json.Value, dupeZ: *const fn ([]const u8) [:0]const u8) void {
    const store = storeOf(widget) orelse return;
    ingest(store, props, dupeZ);
}

/// Every key is read the same way on create and update. `text` is the only
/// one that can echo, and it is compared before it is written.
fn ingest(store: *Store, props: ?std.json.Value, dupeZ: *const fn ([]const u8) [:0]const u8) void {
    const view = store.view;
    const buffer = gtk.TextView.getBuffer(view);
    const api = if (store.highlighted) loadSource() else null;

    if (propStr(props, "text")) |t| {
        var start: gtk.TextIter = undefined;
        var end: gtk.TextIter = undefined;
        gtk.TextBuffer.getStartIter(buffer, &start);
        gtk.TextBuffer.getEndIter(buffer, &end);
        const current = gtk.TextBuffer.getText(buffer, &start, &end, 0);
        defer glib.free(@ptrCast(current));
        if (!std.mem.eql(u8, std.mem.span(current), t)) {
            store.suppress = true;
            gtk.TextBuffer.setText(buffer, dupeZ(t), -1);
            store.suppress = false;
        }
    }
    if (propBool(props, "readOnly")) |ro| {
        gtk.TextView.setEditable(view, @intFromBool(!ro));
        gtk.TextView.setCursorVisible(view, @intFromBool(!ro));
    }
    if (propInt(props, "tabWidth")) |w| {
        if (api) |a| a.set_tab_width(view, @intCast(@max(1, w)));
    }
    if (propBool(props, "showLineNumbers")) |on| {
        if (api) |a| a.set_show_line_numbers(view, @intFromBool(on));
    }
    if (propStr(props, "language")) |id| {
        if (api) |a| applyLanguage(a, buffer, id, dupeZ);
    }
    if (propStr(props, "theme")) |id| {
        if (api) |a| applyTheme(a, buffer, id, dupeZ);
    }
    // Diagnostics repaint after a text write, so a squiggle always lands on
    // the line numbering of the text the app just sent.
    if (propArray(props, "diagnostics")) |arr| {
        ingestDiagnostics(store, arr);
        repaintDiagnostics(store);
    } else if (propStr(props, "text") != null) {
        repaintDiagnostics(store);
    }
}

pub fn connectEvents(widget: *gtk.Widget, node_id: u32, emit_fn: EmitFn) void {
    emit = emit_fn;
    const store = storeOf(widget) orelse return;
    store.node_id = node_id;
}
