// Presenting an AdwDialog over a window that is showing a page.
//
// A dialog presented in-window is drawn by the toplevel's own surface, and the
// Chromium engine renders into an X11 child of that surface. The X server
// stacks a child above everything its parent draws, so over a webview the
// dialog and its scrim are painted, reported by GTK as presented, and never
// seen: the window is modal-blocked with nothing on screen until Escape. The
// pages in that window therefore stand aside while a dialog is up, which is
// the engine's business; this is where it is told.
//
// Every in-window dialog surface goes through here. A new one that presents an
// AdwDialog itself belongs here too.
const gtk = @import("gtk");
const gobject = @import("gobject");
const adw = @import("adw");
const cef = @import("../cef/backend.zig");

/// The window a presented dialog belongs to, held by reference: the dialog
/// outlives the widget that presented it, and its closed handler runs while
/// the window is being torn down.
const WINDOW_KEY = "nd-dialog-surface-window";
const WIRED_KEY = "nd-dialog-surface-wired";

pub fn present(dialog: *adw.Dialog, parent: *gtk.Widget) void {
    adw.Dialog.present(dialog, parent);
    const obj: *gobject.Object = @ptrCast(@alignCast(dialog));
    if (gobject.Object.getData(obj, WIRED_KEY) == null) {
        gobject.Object.setData(obj, WIRED_KEY, @ptrFromInt(1));
        _ = gobject.signalConnectData(obj, "closed", @ptrCast(&cbClosed), null, null, .{ .after = true });
    }
    const root = gtk.Widget.getRoot(parent) orelse return;
    setWindow(obj, @ptrCast(@alignCast(root)));
    cef.refreshDialogOcclusion(@ptrCast(@alignCast(root)));
}

fn setWindow(dialog: *gobject.Object, window: ?*gtk.Window) void {
    if (gobject.Object.getData(dialog, WINDOW_KEY)) |old| {
        gobject.Object.unref(@ptrCast(@alignCast(old)));
    }
    gobject.Object.setData(dialog, WINDOW_KEY, window);
    if (window) |w| _ = gobject.Object.ref(w.as(gobject.Object));
}

fn cbClosed(dialog: *gobject.Object, _: ?*anyopaque) callconv(.c) void {
    const raw = gobject.Object.getData(dialog, WINDOW_KEY) orelse return;
    const window: *gtk.Window = @ptrCast(@alignCast(raw));
    cef.refreshDialogOcclusion(window.as(gtk.Widget));
    setWindow(dialog, null);
}
