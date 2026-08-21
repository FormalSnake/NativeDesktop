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
const Visual = anyopaque;

/// Xlib's XSetWindowAttributes, laid out for LP64. Only two fields are ever
/// set here, but the struct has to match byte for byte because Xlib reads the
/// ones the value mask names by offset.
const SetWindowAttributes = extern struct {
    background_pixmap: c_ulong = 0,
    background_pixel: c_ulong = 0,
    border_pixmap: c_ulong = 0,
    border_pixel: c_ulong = 0,
    bit_gravity: c_int = 0,
    win_gravity: c_int = 0,
    backing_store: c_int = 0,
    backing_planes: c_ulong = 0,
    backing_pixel: c_ulong = 0,
    save_under: c_int = 0,
    event_mask: c_long = 0,
    do_not_propagate_mask: c_long = 0,
    override_redirect: c_int = 0,
    colormap: c_ulong = 0,
    cursor: c_ulong = 0,
};

const CW_BACK_PIXEL: c_ulong = 1 << 1;
const CW_BORDER_PIXEL: c_ulong = 1 << 3;
const CW_COLORMAP: c_ulong = 1 << 13;
const INPUT_OUTPUT: c_uint = 1;

const FnInitThreads = *const fn () callconv(.c) c_int;
const FnCreateWindow = *const fn (*Display, Window, c_int, c_int, c_uint, c_uint, c_uint, c_int, c_uint, ?*Visual, c_ulong, *SetWindowAttributes) callconv(.c) Window;
const FnDefaultScreen = *const fn (*Display) callconv(.c) c_int;
const FnDefaultDepth = *const fn (*Display, c_int) callconv(.c) c_int;
const FnDefaultVisual = *const fn (*Display, c_int) callconv(.c) ?*Visual;
const FnDefaultColormap = *const fn (*Display, c_int) callconv(.c) c_ulong;
const FnWindowOnly = *const fn (*Display, Window) callconv(.c) c_int;
const FnMoveResize = *const fn (*Display, Window, c_int, c_int, c_uint, c_uint) callconv(.c) c_int;
const FnResize = *const fn (*Display, Window, c_uint, c_uint) callconv(.c) c_int;
const FnFlush = *const fn (*Display) callconv(.c) c_int;
const FnSync = *const fn (*Display, c_int) callconv(.c) c_int;
const FnSetInputFocus = *const fn (*Display, Window, c_int, c_ulong) callconv(.c) c_int;
const FnGetXDisplay = *const fn (*gdk.Display) callconv(.c) ?*Display;
const FnGetXid = *const fn (*gdk.Surface) callconv(.c) Window;
const FnTrap = *const fn (*gdk.Display) callconv(.c) void;

const Api = struct {
    init_threads: FnInitThreads,
    create_window: FnCreateWindow,
    default_screen: FnDefaultScreen,
    default_depth: FnDefaultDepth,
    default_visual: FnDefaultVisual,
    default_colormap: FnDefaultColormap,
    map_window: FnWindowOnly,
    unmap_window: FnWindowOnly,
    destroy_window: FnWindowOnly,
    move_resize_window: FnMoveResize,
    resize_window: FnResize,
    flush: FnFlush,
    sync: FnSync,
    set_input_focus: FnSetInputFocus,
    display_get_xdisplay: FnGetXDisplay,
    surface_get_xid: FnGetXid,
    /// GDK aborts the process on any untrapped X error from its own
    /// connection, and the windows this file touches can die underneath it.
    error_trap_push: FnTrap,
    error_trap_pop_ignored: FnTrap,
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
        .create_window = x.lookup(FnCreateWindow, "XCreateWindow") orelse return missing(&x, &g, "XCreateWindow"),
        .default_screen = x.lookup(FnDefaultScreen, "XDefaultScreen") orelse return missing(&x, &g, "XDefaultScreen"),
        .default_depth = x.lookup(FnDefaultDepth, "XDefaultDepth") orelse return missing(&x, &g, "XDefaultDepth"),
        .default_visual = x.lookup(FnDefaultVisual, "XDefaultVisual") orelse return missing(&x, &g, "XDefaultVisual"),
        .default_colormap = x.lookup(FnDefaultColormap, "XDefaultColormap") orelse return missing(&x, &g, "XDefaultColormap"),
        .map_window = x.lookup(FnWindowOnly, "XMapWindow") orelse return missing(&x, &g, "XMapWindow"),
        .unmap_window = x.lookup(FnWindowOnly, "XUnmapWindow") orelse return missing(&x, &g, "XUnmapWindow"),
        .destroy_window = x.lookup(FnWindowOnly, "XDestroyWindow") orelse return missing(&x, &g, "XDestroyWindow"),
        .move_resize_window = x.lookup(FnMoveResize, "XMoveResizeWindow") orelse return missing(&x, &g, "XMoveResizeWindow"),
        .resize_window = x.lookup(FnResize, "XResizeWindow") orelse return missing(&x, &g, "XResizeWindow"),
        .flush = x.lookup(FnFlush, "XFlush") orelse return missing(&x, &g, "XFlush"),
        .sync = x.lookup(FnSync, "XSync") orelse return missing(&x, &g, "XSync"),
        .set_input_focus = x.lookup(FnSetInputFocus, "XSetInputFocus") orelse return missing(&x, &g, "XSetInputFocus"),
        .display_get_xdisplay = g.lookup(FnGetXDisplay, "gdk_x11_display_get_xdisplay") orelse return missing(&x, &g, "gdk_x11_display_get_xdisplay"),
        .surface_get_xid = g.lookup(FnGetXid, "gdk_x11_surface_get_xid") orelse return missing(&x, &g, "gdk_x11_surface_get_xid"),
        .error_trap_push = g.lookup(FnTrap, "gdk_x11_display_error_trap_push") orelse return missing(&x, &g, "gdk_x11_display_error_trap_push"),
        .error_trap_pop_ignored = g.lookup(FnTrap, "gdk_x11_display_error_trap_pop_ignored") orelse return missing(&x, &g, "gdk_x11_display_error_trap_pop_ignored"),
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
    const c = conn() orelse return null;
    return c.x;
}

/// GDK's connection plus the trap that keeps an error on it from being fatal.
///
/// Every window this file touches can be destroyed by someone else first: the
/// X server tears down a toplevel's whole child subtree when the toplevel goes,
/// so a webview's own teardown routinely runs against XIDs that no longer
/// exist. GDK's error handler aborts the process on an untrapped error, which
/// turned closing a popup window into a crash. Every request below is trapped.
const Conn = struct {
    api: *const Api,
    gdk: *gdk.Display,
    x: *Display,

    fn push(self: Conn) void {
        self.api.error_trap_push(self.gdk);
    }

    /// Also flushes: the trap only covers requests the server has been asked
    /// about, and an error arrives with the reply.
    fn pop(self: Conn) void {
        _ = self.api.flush(self.x);
        self.api.error_trap_pop_ignored(self.gdk);
    }
};

fn conn() ?Conn {
    const a = loadApi() orelse return null;
    const gdk_display = gdk.Display.getDefault() orelse return null;
    if (!isX11(gdk_display)) return null;
    const x = a.display_get_xdisplay(gdk_display) orelse return null;
    return .{ .api = a, .gdk = gdk_display, .x = x };
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
    const c = conn() orelse return 0;
    if (parent == 0) return 0;
    // Explicit default visual rather than XCreateSimpleWindow's
    // CopyFromParent: GTK4 gives its toplevel a 32-bit ARGB visual, and a
    // container that inherits it is not something Chromium can parent its own
    // window into. Naming the screen's default depth/visual/colormap here (and
    // the border pixel that a differing depth requires) is what makes the
    // embedded window appear at all.
    const screen = c.api.default_screen(c.x);
    var attrs: SetWindowAttributes = .{
        .background_pixel = 0,
        .border_pixel = 0,
        .colormap = c.api.default_colormap(c.x, screen),
    };
    c.push();
    const child = c.api.create_window(
        c.x,
        parent,
        x,
        y,
        @max(w, 1),
        @max(h, 1),
        0,
        c.api.default_depth(c.x, screen),
        INPUT_OUTPUT,
        c.api.default_visual(c.x, screen),
        CW_BACK_PIXEL | CW_BORDER_PIXEL | CW_COLORMAP,
        &attrs,
    );
    if (child != 0) _ = c.api.sync(c.x, 0);
    c.pop();
    return child;
}

pub fn moveResize(window: Window, x: c_int, y: c_int, w: c_uint, h: c_uint) void {
    if (window == 0) return;
    const c = conn() orelse return;
    c.push();
    _ = c.api.move_resize_window(c.x, window, x, y, @max(w, 1), @max(h, 1));
    // Flushed because the caller's next request is CEF resizing its inner
    // window on ITS OWN connection (resizeOn flushes immediately). Left in
    // GDK's buffer, the outer clip window's resize can reach the server after
    // the inner one, and the server holds mismatched geometry until GTK's
    // next flush: visible as tearing or a stuck size during a live resize.
    _ = c.api.flush(c.x);
    c.pop();
}

/// CEF's own window is a child of the container and does not follow it: the
/// Linux platform delegate sizes it once at creation from window_info.bounds.
/// `dpy` is CEF's connection, not GDK's: the window belongs to CEF, and a
/// BadWindow raised on GDK's connection takes the whole host down.
pub fn resizeOn(dpy: *Display, window: Window, w: c_uint, h: c_uint) void {
    if (window == 0) return;
    const a = loadApi() orelse return;
    // Trapped on GDK's display even though the request goes out on CEF's:
    // Xlib's error handler is per PROCESS, so GDK's fatal one sees errors from
    // either connection.
    const gdk_display = gdk.Display.getDefault();
    if (gdk_display) |g| a.error_trap_push(g);
    _ = a.resize_window(dpy, window, @max(w, 1), @max(h, 1));
    _ = a.flush(dpy);
    if (gdk_display) |g| a.error_trap_pop_ignored(g);
}

/// Xlib RevertToParent: focus falls back to the parent window if the target
/// is unmapped later, never to PointerRoot (focus-follows-mouse surprises).
const REVERT_TO_PARENT: c_int = 2;
const CURRENT_TIME: c_ulong = 0;

/// Returns X input focus to the toplevel that hosts `widget`. GTK's own
/// grab_focus moves GTK's focus WIDGET but not X input focus, and after any
/// interaction with the page it is CEF's window that holds the latter; until
/// it comes back, the toplevel sees no key events and every accelerator in
/// the app is dead.
pub fn focusToplevel(widget: *gtk.Widget) void {
    const xid = toplevelXid(widget);
    if (xid == 0) return;
    const c = conn() orelse return;
    c.push();
    _ = c.api.set_input_focus(c.x, xid, REVERT_TO_PARENT, CURRENT_TIME);
    _ = c.api.flush(c.x);
    c.pop();
}

/// Mapping is synced for the same reason creation is: CEF only paints into a
/// viewable window, and it reads that state from its own connection.
pub fn show(window: Window) void {
    if (window == 0) return;
    const c = conn() orelse return;
    c.push();
    _ = c.api.map_window(c.x, window);
    _ = c.api.sync(c.x, 0);
    c.pop();
}

pub fn hide(window: Window) void {
    if (window == 0) return;
    const c = conn() orelse return;
    c.push();
    _ = c.api.unmap_window(c.x, window);
    c.pop();
}

/// Destroying the container is the one call that is EXPECTED to fail: closing
/// a window destroys its toplevel surface, the X server destroys that window's
/// whole child subtree with it, and the webview's own teardown then arrives at
/// an XID that is already gone. Untrapped, that BadWindow aborted the host
/// every time a popup window closed, and at quit for every live view.
pub fn destroy(window: Window) void {
    if (window == 0) return;
    const c = conn() orelse return;
    c.push();
    _ = c.api.destroy_window(c.x, window);
    c.pop();
}
