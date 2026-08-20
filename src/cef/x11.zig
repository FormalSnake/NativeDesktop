// The X11 side of windowed embedding.
//
// GTK4 has no per-widget windows: only a GtkNative (the toplevel) owns a
// GdkSurface, so a <webview> in the middle of a tree has no XID of its own to
// hand CEF. What it gets instead is a bare X11 child window created here,
// parented to the toplevel's XID and tracked against the widget's allocation.
// That child is what goes into cef_window_info_t.parent_window.
//
// Both libraries are resolved at runtime for the same reason libcef is: an app
// that never asks for the Chromium engine must not gain a link-time dependency
// on libX11, and the GTK4 build on macOS has no X11 backend to find.
const std = @import("std");
const gdk = @import("gdk");
const gtk = @import("gtk");
const gobject = @import("gobject");

pub const Window = c_ulong;
pub const Display = anyopaque;

const FnInitThreads = *const fn () callconv(.c) c_int;
const FnCreateSimpleWindow = *const fn (*Display, Window, c_int, c_int, c_uint, c_uint, c_uint, c_ulong, c_ulong) callconv(.c) Window;
const FnWindowOnly = *const fn (*Display, Window) callconv(.c) c_int;
const FnMoveResize = *const fn (*Display, Window, c_int, c_int, c_uint, c_uint) callconv(.c) c_int;
const FnResize = *const fn (*Display, Window, c_uint, c_uint) callconv(.c) c_int;
const FnFlush = *const fn (*Display) callconv(.c) c_int;
const FnSync = *const fn (*Display, c_int) callconv(.c) c_int;
const FnGetXDisplay = *const fn (*gdk.Display) callconv(.c) ?*Display;
const FnGetXid = *const fn (*gdk.Surface) callconv(.c) Window;

const Api = struct {
    init_threads: FnInitThreads,
    create_simple_window: FnCreateSimpleWindow,
    map_window: FnWindowOnly,
    unmap_window: FnWindowOnly,
    destroy_window: FnWindowOnly,
    move_resize_window: FnMoveResize,
    resize_window: FnResize,
    flush: FnFlush,
    sync: FnSync,
    display_get_xdisplay: FnGetXDisplay,
    surface_get_xid: FnGetXid,
};

var api: ?Api = null;
var attempted = false;
var xlib: std.DynLib = undefined;
var gtklib: std.DynLib = undefined;

fn loadApi() ?*const Api {
    if (attempted) return if (api != null) &api.? else null;
    attempted = true;

    var x = std.DynLib.open("libX11.so.6") catch std.DynLib.open("libX11.so") catch {
        std.debug.print("ND_WARN CEF: libX11 not found; windowed embedding needs it\n", .{});
        return null;
    };
    // GTK4 links its GDK backends into one shared object, so the X11 helpers
    // resolve out of the library GTK already has open.
    var g = std.DynLib.open("libgtk-4.so.1") catch std.DynLib.open("libgtk-4.so") catch {
        x.close();
        std.debug.print("ND_WARN CEF: libgtk-4 handle unavailable; cannot reach the GDK X11 helpers\n", .{});
        return null;
    };

    const resolved: Api = .{
        .init_threads = x.lookup(FnInitThreads, "XInitThreads") orelse return missing(&x, &g, "XInitThreads"),
        .create_simple_window = x.lookup(FnCreateSimpleWindow, "XCreateSimpleWindow") orelse return missing(&x, &g, "XCreateSimpleWindow"),
        .map_window = x.lookup(FnWindowOnly, "XMapWindow") orelse return missing(&x, &g, "XMapWindow"),
        .unmap_window = x.lookup(FnWindowOnly, "XUnmapWindow") orelse return missing(&x, &g, "XUnmapWindow"),
        .destroy_window = x.lookup(FnWindowOnly, "XDestroyWindow") orelse return missing(&x, &g, "XDestroyWindow"),
        .move_resize_window = x.lookup(FnMoveResize, "XMoveResizeWindow") orelse return missing(&x, &g, "XMoveResizeWindow"),
        .resize_window = x.lookup(FnResize, "XResizeWindow") orelse return missing(&x, &g, "XResizeWindow"),
        .flush = x.lookup(FnFlush, "XFlush") orelse return missing(&x, &g, "XFlush"),
        .sync = x.lookup(FnSync, "XSync") orelse return missing(&x, &g, "XSync"),
        .display_get_xdisplay = g.lookup(FnGetXDisplay, "gdk_x11_display_get_xdisplay") orelse return missing(&x, &g, "gdk_x11_display_get_xdisplay"),
        .surface_get_xid = g.lookup(FnGetXid, "gdk_x11_surface_get_xid") orelse return missing(&x, &g, "gdk_x11_surface_get_xid"),
    };
    xlib = x;
    gtklib = g;
    api = resolved;
    return &api.?;
}

fn missing(x: *std.DynLib, g: *std.DynLib, symbol: []const u8) ?*const Api {
    std.debug.print("ND_WARN CEF: missing symbol {s}; windowed embedding disabled\n", .{symbol});
    x.close();
    g.close();
    return null;
}

/// Chromium touches Xlib from its own threads. Without this the first
/// concurrent request corrupts the connection, usually as an unrelated-looking
/// BadWindow much later. Must run before GTK opens the display.
pub fn initThreads() void {
    const a = loadApi() orelse return;
    _ = a.init_threads();
}

/// The GDK display connection, or null when this session is not X11 (a Wayland
/// session where the backend pin did not take, or a build with no X11 backend).
pub fn display() ?*Display {
    const a = loadApi() orelse return null;
    const gdk_display = gdk.Display.getDefault() orelse return null;
    if (!isX11(gdk_display)) return null;
    return a.display_get_xdisplay(gdk_display);
}

fn isX11(gdk_display: *gdk.Display) bool {
    const name = gobject.typeNameFromInstance(@ptrCast(@alignCast(gdk_display)));
    return std.mem.startsWith(u8, std.mem.span(name), "GdkX11");
}

/// XID of the toplevel this widget is inside, or 0 when it is not realized yet.
/// Zero is the whole point of the check: CEF given a null parent quietly opens
/// its own top-level Chromium window.
pub fn toplevelXid(widget: *gtk.Widget) Window {
    const a = loadApi() orelse return 0;
    const native = gtk.Widget.getNative(widget) orelse return 0;
    const surface = gtk.Native.getSurface(native) orelse return 0;
    const gdk_display = gdk.Surface.getDisplay(surface);
    if (!isX11(gdk_display)) return 0;
    return a.surface_get_xid(surface);
}

/// The container CEF is parented into. Created unmapped so nothing flashes
/// before the browser exists; `show` maps it.
///
/// XSync, not XFlush: the next thing that happens is CEF, on its own X
/// connection and its own thread, creating a window whose parent is this XID.
/// A flush only queues the request, so that XCreateWindow can reach the server
/// first and fail with BadWindow, and what the caller sees afterwards is a
/// browser that never paints.
pub fn createChild(parent: Window, x: c_int, y: c_int, w: c_uint, h: c_uint) Window {
    const a = loadApi() orelse return 0;
    const dpy = display() orelse return 0;
    if (parent == 0) return 0;
    const child = a.create_simple_window(dpy, parent, x, y, @max(w, 1), @max(h, 1), 0, 0, 0);
    if (child != 0) _ = a.sync(dpy, 0);
    return child;
}

pub fn moveResize(window: Window, x: c_int, y: c_int, w: c_uint, h: c_uint) void {
    if (window == 0) return;
    const a = loadApi() orelse return;
    const dpy = display() orelse return;
    _ = a.move_resize_window(dpy, window, x, y, @max(w, 1), @max(h, 1));
    _ = a.flush(dpy);
}

/// CEF's own window is a child of the container and does not follow it: the
/// Linux platform delegate sizes it once at creation from window_info.bounds.
/// `dpy` is CEF's connection, not GDK's: the window belongs to CEF, and a
/// BadWindow raised on GDK's connection takes the whole host down.
pub fn resizeOn(dpy: *Display, window: Window, w: c_uint, h: c_uint) void {
    if (window == 0) return;
    const a = loadApi() orelse return;
    _ = a.resize_window(dpy, window, @max(w, 1), @max(h, 1));
    _ = a.flush(dpy);
}

/// Mapping is synced for the same reason creation is: CEF only paints into a
/// viewable window, and it reads that state from its own connection.
pub fn show(window: Window) void {
    if (window == 0) return;
    const a = loadApi() orelse return;
    const dpy = display() orelse return;
    _ = a.map_window(dpy, window);
    _ = a.sync(dpy, 0);
}

pub fn hide(window: Window) void {
    if (window == 0) return;
    const a = loadApi() orelse return;
    const dpy = display() orelse return;
    _ = a.unmap_window(dpy, window);
    _ = a.flush(dpy);
}

pub fn destroy(window: Window) void {
    if (window == 0) return;
    const a = loadApi() orelse return;
    const dpy = display() orelse return;
    _ = a.destroy_window(dpy, window);
    _ = a.flush(dpy);
}
