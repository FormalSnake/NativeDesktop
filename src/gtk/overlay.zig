// Host-rendered crash overlay (M8-D5). UI-thread only: builds a GtkOverlay
// wrapping the window's current child, floats a chrome panel ("Runtime
// crashed", the error text, and — dev-mode only — a Restart button) on top,
// and registers each chrome widget in the tree under the reserved 0xFFFF
// generation so `getTree` exposes the crash to agents (M8-D5 rationale: an
// untracked overlay would leave the automation tree blind exactly when an
// agent most needs to see the failure).
const std = @import("std");
const gtk = @import("gtk");
const adw = @import("adw");
const gobject = @import("gobject");
const glib = @import("glib");
const tree_mod = @import("../tree.zig");
const Tree = tree_mod.Tree;
const OVERLAY_GENERATION = tree_mod.OVERLAY_GENERATION;

pub const RestartFn = *const fn () void;

var arena_state = std.heap.ArenaAllocator.init(std.heap.page_allocator);
const arena = arena_state.allocator();

fn dupeZ(s: []const u8) [:0]const u8 {
    return arena.dupeZ(u8, s) catch @panic("OOM in overlay arena");
}

var overlay_seq: u32 = 0;
fn overlayId() u32 {
    overlay_seq += 1;
    return (OVERLAY_GENERATION << 24) | (overlay_seq & 0xffffff);
}

/// The panel `GtkBox` is the ONLY overlay widget actually attached to the
/// live widget tree (via `gtk.Overlay.addOverlay`); title/error/restart are
/// its children, owned solely by it. Tracked separately (not in `tree.nodes`
/// under its own extra bookkeeping) so `clear()` can detach exactly this one
/// widget via `gtk.Overlay.removeOverlay` — the dedicated teardown function
/// for `addOverlay`'d children (a raw `Widget.unparent()` leaves GtkOverlay's
/// internal overlay-child bookkeeping holding a stale pointer, which crashed
/// with a `GTK_IS_WIDGET` assertion the next time GTK measured/allocated the
/// overlay — verified this session). GTK's normal container teardown then
/// destroys the box's children for us; this module must NOT call any GTK
/// function on those children afterward (they are dangling pointers once the
/// box is removed).
var panel_box: ?*gtk.Widget = null;
var overlay_widget: ?*gtk.Overlay = null;
var original_content: ?*gtk.Widget = null;

/// Registers a host-created overlay widget in the tree under the reserved
/// 0xFFFF generation (parent 0 — overlay chrome is flat, not nested in the
/// automation tree's parent/child sense; `getTree` only needs each node's
/// own type/testID/text, matched by M8-D5).
fn registerOverlayNode(tree: *Tree, widget: *gtk.Widget, widget_type: []const u8, test_id: []const u8, text: ?[]const u8) void {
    const id = overlayId();
    tree.nodes.put(tree.gpa, id, widget) catch return;
    tree.putMeta(id, widget_type, test_id, text, 0, .{}) catch {};
}

fn onRestartClicked(_: *gtk.Button, data: ?*anyopaque) callconv(.c) void {
    const restart: RestartFn = @ptrCast(@alignCast(data.?));
    // Defer off the click signal's call stack (re-entrancy: the click
    // handler runs from inside the same GTK main-loop dispatch that would
    // be tearing down/rebuilding the widget tree the button lives in).
    const Ctx = struct { fn call(fn_data: ?*anyopaque) callconv(.c) c_int {
        const f: RestartFn = @ptrCast(@alignCast(fn_data.?));
        f();
        return 0; // G_SOURCE_REMOVE
    } };
    _ = glib.idleAdd(&Ctx.call, @constCast(@ptrCast(restart)));
}

/// Builds and shows the crash panel over the window's current content.
/// `dev` gates the Restart button (M8-D4: Restart/respawn are dev-only).
pub fn show(tree: *Tree, window: *gtk.Window, message: []const u8, dev: bool, restart: RestartFn) void {
    // The window is an adw.ApplicationWindow (edge-to-edge): its content is
    // owned via setContent/getContent, NOT gtk.Window.getChild/setChild —
    // those target the internal handle AdwApplicationWindow wraps.
    const app_win: *adw.ApplicationWindow = @ptrCast(@alignCast(window));
    const content = adw.ApplicationWindow.getContent(app_win);
    const overlay = gtk.Overlay.new();
    if (content) |c| {
        // Ref before detaching so the widget survives the brief
        // parent-less window between setContent(null) and Overlay.setChild.
        _ = gobject.Object.ref(c.as(gobject.Object));
        adw.ApplicationWindow.setContent(app_win, null);
        gtk.Overlay.setChild(overlay, c);
        _ = gobject.Object.unref(c.as(gobject.Object));
        original_content = c;
    }
    adw.ApplicationWindow.setContent(app_win, overlay.as(gtk.Widget));
    overlay_widget = overlay;

    const box = gtk.Box.new(.vertical, 8);
    const title = gtk.Label.new(dupeZ("Runtime crashed"));
    const err = gtk.Label.new(dupeZ(message));
    gtk.Box.append(box, title.as(gtk.Widget));
    gtk.Box.append(box, err.as(gtk.Widget));
    registerOverlayNode(tree, box.as(gtk.Widget), "Box", "nd-overlay-panel", null);
    registerOverlayNode(tree, title.as(gtk.Widget), "Label", "nd-overlay-title", "Runtime crashed");
    registerOverlayNode(tree, err.as(gtk.Widget), "Label", "nd-overlay-error", message);
    if (dev) {
        const btn = gtk.Button.newWithLabel(dupeZ("Restart"));
        _ = gtk.Button.signals.clicked.connect(btn, ?*anyopaque, &onRestartClicked, @constCast(@ptrCast(restart)), .{});
        gtk.Box.append(box, btn.as(gtk.Widget));
        registerOverlayNode(tree, btn.as(gtk.Widget), "Button", "nd-overlay-restart", "Restart");
    }
    gtk.Overlay.addOverlay(overlay, box.as(gtk.Widget));
    panel_box = box.as(gtk.Widget);
    std.debug.print("ND_OVERLAY_SHOWN dev={}\n", .{dev});
}

/// Drops every 0xFFFF-generation node's tree bookkeeping (the overlay
/// chrome), removes the panel box from the `GtkOverlay` via the dedicated
/// `removeOverlay` teardown (see `panel_box`'s doc comment for why a raw
/// `unparent()` is wrong here), and restores the window's pre-crash content
/// as its direct child again (undoing `show()`'s wrap). GTK's own container
/// teardown destroys the box's children (title/error/restart); this
/// function must NOT call any GTK function on those children afterward.
/// Used after a successful Restart re-mount clears the crash panel.
pub fn clear(tree: *Tree, window: *gtk.Window) void {
    var doomed: std.ArrayList(u32) = .empty;
    defer doomed.deinit(tree.gpa);
    var it = tree.meta.iterator();
    while (it.next()) |entry| {
        const id = entry.key_ptr.*;
        if (id >> 24 == OVERLAY_GENERATION) doomed.append(tree.gpa, id) catch continue;
    }
    for (doomed.items) |id| {
        _ = tree.nodes.remove(id);
        tree.removeMeta(id);
    }
    if (overlay_widget) |ov| {
        if (panel_box) |box| gtk.Overlay.removeOverlay(ov, box);
    }
    panel_box = null;
    const app_win: *adw.ApplicationWindow = @ptrCast(@alignCast(window));
    if (original_content) |c| {
        _ = gobject.Object.ref(c.as(gobject.Object));
        gtk.Overlay.setChild(overlay_widget.?, null);
        adw.ApplicationWindow.setContent(app_win, c);
        _ = gobject.Object.unref(c.as(gobject.Object));
    } else {
        adw.ApplicationWindow.setContent(app_win, null);
    }
    overlay_widget = null;
    original_content = null;
}
