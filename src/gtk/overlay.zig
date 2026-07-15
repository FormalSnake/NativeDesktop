// Host-rendered crash overlay. UI-thread only: builds a GtkOverlay
// wrapping the window's current child, floats a chrome panel ("Runtime
// crashed", the error text, and — dev-mode only — a Restart button) on top,
// and registers each chrome widget in the tree under the reserved 0xFF
// generation so `getTree` exposes the crash to agents (an untracked
// overlay would leave the automation tree blind exactly when an
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

/// One crash overlay per window (multi-window): a JS crash is a single Bun
/// process failing, so it takes down every window's UI at once — the overlay
/// therefore paints on ALL open windows, and `clearAll` restores all of them.
/// Each state's `panel_box` (a `GtkBox`) is the ONLY overlay widget actually
/// attached to that window's live tree (via `gtk.Overlay.addOverlay`);
/// title/error/restart are its children, owned solely by it. Tracked here (not
/// in `tree.nodes`) so `clearAll` can detach exactly that one widget via
/// `gtk.Overlay.removeOverlay` — the dedicated teardown for `addOverlay`'d
/// children (a raw `Widget.unparent()` leaves GtkOverlay's internal
/// overlay-child bookkeeping holding a stale pointer, which crashed with a
/// `GTK_IS_WIDGET` assertion the next time GTK measured/allocated the
/// overlay). GTK's normal container teardown then destroys the
/// box's children for us; this module must NOT call any GTK function on those
/// children afterward (they are dangling pointers once the box is removed).
const OverlayState = struct {
    window: *adw.ApplicationWindow,
    overlay_widget: *gtk.Overlay,
    panel_box: *gtk.Widget,
    original_content: ?*gtk.Widget,
};
var states: std.ArrayListUnmanaged(OverlayState) = .empty;

/// Registers a host-created overlay widget in the tree under the reserved
/// 0xFF generation (parent 0 — overlay chrome is flat, not nested in the
/// automation tree's parent/child sense; `getTree` only needs each node's
/// own type/testID/text).
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

/// Paints a crash panel on EVERY open window (multi-window): a JS crash is one
/// Bun process dying, so all windows lose their live UI simultaneously. Clears
/// any existing overlays first so a second crash message replaces rather than
/// stacks. `dev` gates the Restart button (Restart/respawn are dev-only).
pub fn showAll(tree: *Tree, app: *gtk.Application, message: []const u8, dev: bool, restart: RestartFn) void {
    clearAll(tree);
    var node: ?*glib.List = gtk.Application.getWindows(app);
    while (node) |n| : (node = n.f_next) {
        const data = n.f_data orelse continue;
        const window: *gtk.Window = @ptrCast(@alignCast(data));
        show(tree, window, message, dev, restart);
    }
    std.debug.print("ND_OVERLAY_SHOWN dev={}\n", .{dev});
}

/// Builds and shows the crash panel over one window's current content, pushing
/// its teardown state. Internal to `showAll` — never call standalone, or the
/// per-window states leak from `clearAll`'s reach.
fn show(tree: *Tree, window: *gtk.Window, message: []const u8, dev: bool, restart: RestartFn) void {
    // The window is an adw.ApplicationWindow (edge-to-edge): its content is
    // owned via setContent/getContent, NOT gtk.Window.getChild/setChild —
    // those target the internal handle AdwApplicationWindow wraps.
    const app_win: *adw.ApplicationWindow = @ptrCast(@alignCast(window));
    const content = adw.ApplicationWindow.getContent(app_win);
    const overlay = gtk.Overlay.new();
    var original_content: ?*gtk.Widget = null;
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
    states.append(arena, .{
        .window = app_win,
        .overlay_widget = overlay,
        .panel_box = box.as(gtk.Widget),
        .original_content = original_content,
    }) catch {};
}

/// Drops every 0xFF-generation node's tree bookkeeping (the overlay chrome
/// across all windows), removes each window's panel box from its `GtkOverlay`
/// via the dedicated `removeOverlay` teardown (see `OverlayState`'s doc comment
/// for why a raw `unparent()` is wrong here), and restores every window's
/// pre-crash content as its direct child again (undoing `showAll`'s wrap).
/// GTK's own container teardown destroys the boxes' children (title/error/
/// restart); this function must NOT call any GTK function on those children
/// afterward. Used after a successful Restart re-mount clears the crash panels.
pub fn clearAll(tree: *Tree) void {
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
    for (states.items) |st| {
        gtk.Overlay.removeOverlay(st.overlay_widget, st.panel_box);
        if (st.original_content) |c| {
            _ = gobject.Object.ref(c.as(gobject.Object));
            gtk.Overlay.setChild(st.overlay_widget, null);
            adw.ApplicationWindow.setContent(st.window, c);
            _ = gobject.Object.unref(c.as(gobject.Object));
        } else {
            adw.ApplicationWindow.setContent(st.window, null);
        }
    }
    states.clearRetainingCapacity();
}
