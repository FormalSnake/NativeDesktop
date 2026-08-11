// Shared empty-state chrome for the data-view widgets (SourceList / ListView /
// Table / TreeView / SourceTree). When the item array is empty AND the app set
// any of emptyIconName/emptyTitle/emptyDescription, the tracked
// GtkScrolledWindow swaps its child for a compact AdwStatusPage; the real view
// is parked here behind a registry-owned strong ref and restored when items
// return. Opt-in per widget: no empty-state props means no swap and no
// behavior change. `innerOf` is the registry's public read — generated
// scrolledWindowInner and the hand-written modules resolve the REAL view
// through it so a swapped scroller never leaks an AdwStatusPage where a
// GtkListBox/GtkColumnView cast is expected.
const std = @import("std");
const gtk = @import("gtk");
const gobject = @import("gobject");
const adw = @import("adw");
const ndicons = @import("icons.zig");

const alloc = std.heap.page_allocator;

const Entry = struct {
    inner: *gtk.Widget, // registry-owned ref (survives the swap)
    page: ?*adw.StatusPage = null, // registry-owned ref once built
    icon: ?[:0]u8 = null,
    title: ?[:0]u8 = null,
    desc: ?[:0]u8 = null,
    swapped: bool = false,
};

var entries: std.AutoHashMapUnmanaged(usize, *Entry) = .empty;

fn asObject(ptr: anytype) *gobject.Object {
    return @ptrCast(@alignCast(ptr));
}

/// Records the scroller's real inner view at create time. Idempotent per sw
/// (creates register exactly once).
pub fn register(sw: *gtk.ScrolledWindow, inner: *gtk.Widget) void {
    const e = alloc.create(Entry) catch return;
    e.* = .{ .inner = inner };
    _ = gobject.Object.ref(asObject(inner));
    entries.put(alloc, @intFromPtr(sw), e) catch {
        gobject.Object.unref(asObject(inner));
        alloc.destroy(e);
        return;
    };
    _ = gtk.Widget.signals.destroy.connect(sw.as(gtk.Widget), ?*anyopaque, &cbDestroyed, null, .{});
}

/// The registered inner view, whatever the scroller currently shows.
pub fn innerOf(sw: *gtk.ScrolledWindow) ?*gtk.Widget {
    const e = entries.get(@intFromPtr(sw)) orelse return null;
    return e.inner;
}

fn setStr(slot: *?[:0]u8, v: []const u8) void {
    if (slot.*) |old| alloc.free(old);
    slot.* = if (v.len == 0) null else (alloc.dupeZ(u8, v) catch null);
}

/// Merges the empty-state props (a diffed update carries only changed keys;
/// the empty string clears one). Refreshes a live page in place.
pub fn configure(sw: *gtk.ScrolledWindow, icon: ?[]const u8, title: ?[]const u8, desc: ?[]const u8) void {
    const e = entries.get(@intFromPtr(sw)) orelse return;
    if (icon) |v| setStr(&e.icon, v);
    if (title) |v| setStr(&e.title, v);
    if (desc) |v| setStr(&e.desc, v);
    if (e.page) |p| applyToPage(e, p);
}

fn configured(e: *Entry) bool {
    return e.icon != null or e.title != null or e.desc != null;
}

fn applyToPage(e: *Entry, page: *adw.StatusPage) void {
    adw.StatusPage.setIconName(page, if (e.icon) |ic| ndicons.symbolic(ic).ptr else null);
    adw.StatusPage.setTitle(page, if (e.title) |t| t.ptr else "");
    adw.StatusPage.setDescription(page, if (e.desc) |d| d.ptr else null);
}

/// Swap point, called wherever a widget's item array lands (create + item
/// updates). Ref bracket: setChild drops the scroller's ref on the outgoing
/// child; the registry's own refs on inner/page keep both alive across swaps.
pub fn update(sw: *gtk.ScrolledWindow, is_empty: bool) void {
    const e = entries.get(@intFromPtr(sw)) orelse return;
    if (is_empty and configured(e)) {
        if (e.swapped) return;
        const page = e.page orelse blk: {
            const p = adw.StatusPage.new();
            gtk.Widget.addCssClass(p.as(gtk.Widget), "compact");
            _ = gobject.Object.refSink(asObject(p));
            e.page = p;
            break :blk p;
        };
        applyToPage(e, page);
        gtk.ScrolledWindow.setChild(sw, page.as(gtk.Widget));
        e.swapped = true;
    } else if (e.swapped) {
        gtk.ScrolledWindow.setChild(sw, e.inner);
        e.swapped = false;
    }
}

fn cbDestroyed(w: *gtk.Widget, _: ?*anyopaque) callconv(.c) void {
    const kv = entries.fetchRemove(@intFromPtr(w)) orelse return;
    const e = kv.value;
    gobject.Object.unref(asObject(e.inner));
    if (e.page) |p| gobject.Object.unref(asObject(p));
    if (e.icon) |v| alloc.free(v);
    if (e.title) |v| alloc.free(v);
    if (e.desc) |v| alloc.free(v);
    alloc.destroy(e);
}
