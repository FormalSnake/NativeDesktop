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
const FnReparent = *const fn (*Display, Window, Window, c_int, c_int) callconv(.c) c_int;
const FnResize = *const fn (*Display, Window, c_uint, c_uint) callconv(.c) c_int;
const FnFlush = *const fn (*Display) callconv(.c) c_int;
const FnSync = *const fn (*Display, c_int) callconv(.c) c_int;
const FnSetInputFocus = *const fn (*Display, Window, c_int, c_ulong) callconv(.c) c_int;
const FnGetInputFocus = *const fn (*Display, *Window, *c_int) callconv(.c) c_int;
const FnQueryPointer = *const fn (*Display, Window, *Window, *Window, *c_int, *c_int, *c_int, *c_int, *c_uint) callconv(.c) c_int;
const FnGetXDisplay = *const fn (*gdk.Display) callconv(.c) ?*Display;
const FnGetXid = *const fn (*gdk.Surface) callconv(.c) Window;
const FnTrap = *const fn (*gdk.Display) callconv(.c) void;
const FnSurfaceLookup = *const fn (*gdk.Display, Window) callconv(.c) ?*gdk.Surface;
const FnRootWindow = *const fn (*Display) callconv(.c) Window;
const FnQueryTree = *const fn (*Display, Window, *Window, *Window, *[*]Window, *c_uint) callconv(.c) c_int;
const FnTranslate = *const fn (*Display, Window, Window, c_int, c_int, *c_int, *c_int, *Window) callconv(.c) c_int;
const FnFree = *const fn (?*anyopaque) callconv(.c) c_int;
const FnGeometry = *const fn (*Display, Window, *Window, *c_int, *c_int, *c_uint, *c_uint, *c_uint, *c_uint) callconv(.c) c_int;
const FnInternAtom = *const fn (*Display, [*:0]const u8, c_int) callconv(.c) c_ulong;
const FnGetProperty = *const fn (*Display, Window, c_ulong, c_long, c_long, c_int, c_ulong, *c_ulong, *c_int, *c_ulong, *c_ulong, *[*]u8) callconv(.c) c_int;
const FnChangeProperty = *const fn (*Display, Window, c_ulong, c_ulong, c_int, c_int, [*]const u8, c_int) callconv(.c) c_int;
const FnSetTransientFor = *const fn (*Display, Window, Window) callconv(.c) c_int;

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
    reparent_window: FnReparent,
    resize_window: FnResize,
    flush: FnFlush,
    sync: FnSync,
    set_input_focus: FnSetInputFocus,
    get_input_focus: FnGetInputFocus,
    default_root_window: FnRootWindow,
    query_tree: FnQueryTree,
    translate_coordinates: FnTranslate,
    free: FnFree,
    get_geometry: FnGeometry,
    intern_atom: FnInternAtom,
    get_window_property: FnGetProperty,
    change_property: FnChangeProperty,
    set_transient_for_hint: FnSetTransientFor,
    query_pointer: FnQueryPointer,
    display_get_xdisplay: FnGetXDisplay,
    surface_get_xid: FnGetXid,
    surface_lookup: FnSurfaceLookup,
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
        .reparent_window = x.lookup(FnReparent, "XReparentWindow") orelse return missing(&x, &g, "XReparentWindow"),
        .resize_window = x.lookup(FnResize, "XResizeWindow") orelse return missing(&x, &g, "XResizeWindow"),
        .flush = x.lookup(FnFlush, "XFlush") orelse return missing(&x, &g, "XFlush"),
        .sync = x.lookup(FnSync, "XSync") orelse return missing(&x, &g, "XSync"),
        .set_input_focus = x.lookup(FnSetInputFocus, "XSetInputFocus") orelse return missing(&x, &g, "XSetInputFocus"),
        .get_input_focus = x.lookup(FnGetInputFocus, "XGetInputFocus") orelse return missing(&x, &g, "XGetInputFocus"),
        .default_root_window = x.lookup(FnRootWindow, "XDefaultRootWindow") orelse return missing(&x, &g, "XDefaultRootWindow"),
        .query_tree = x.lookup(FnQueryTree, "XQueryTree") orelse return missing(&x, &g, "XQueryTree"),
        .translate_coordinates = x.lookup(FnTranslate, "XTranslateCoordinates") orelse return missing(&x, &g, "XTranslateCoordinates"),
        .free = x.lookup(FnFree, "XFree") orelse return missing(&x, &g, "XFree"),
        .get_geometry = x.lookup(FnGeometry, "XGetGeometry") orelse return missing(&x, &g, "XGetGeometry"),
        .intern_atom = x.lookup(FnInternAtom, "XInternAtom") orelse return missing(&x, &g, "XInternAtom"),
        .get_window_property = x.lookup(FnGetProperty, "XGetWindowProperty") orelse return missing(&x, &g, "XGetWindowProperty"),
        .change_property = x.lookup(FnChangeProperty, "XChangeProperty") orelse return missing(&x, &g, "XChangeProperty"),
        .set_transient_for_hint = x.lookup(FnSetTransientFor, "XSetTransientForHint") orelse return missing(&x, &g, "XSetTransientForHint"),
        .query_pointer = x.lookup(FnQueryPointer, "XQueryPointer") orelse return missing(&x, &g, "XQueryPointer"),
        .display_get_xdisplay = g.lookup(FnGetXDisplay, "gdk_x11_display_get_xdisplay") orelse return missing(&x, &g, "gdk_x11_display_get_xdisplay"),
        .surface_get_xid = g.lookup(FnGetXid, "gdk_x11_surface_get_xid") orelse return missing(&x, &g, "gdk_x11_surface_get_xid"),
        .surface_lookup = g.lookup(FnSurfaceLookup, "gdk_x11_surface_lookup_for_display") orelse return missing(&x, &g, "gdk_x11_surface_lookup_for_display"),
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

/// Resizes a window on GDK's connection. Used for the two windows Chromium
/// owns as well as our own: see `applyLayout` for why they are not issued on
/// CEF's connection.
pub fn resize(window: Window, w: c_uint, h: c_uint) void {
    if (window == 0) return;
    const c = conn() orelse return;
    c.push();
    _ = c.api.resize_window(c.x, window, @max(w, 1), @max(h, 1));
    _ = c.api.flush(c.x);
    c.pop();
}

pub fn moveResize(window: Window, x: c_int, y: c_int, w: c_uint, h: c_uint) void {
    if (window == 0) return;
    const c = conn() orelse return;
    c.push();
    _ = c.api.move_resize_window(c.x, window, x, y, @max(w, 1), @max(h, 1));
    // Flushed because the caller's next request is the inner window's own
    // resize. Left in GDK's buffer, the outer clip window's resize can reach
    // the server after the inner one, and the server holds mismatched geometry
    // until GTK's next flush: visible as tearing or a stuck size during a live
    // resize.
    _ = c.api.flush(c.x);
    c.pop();
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
    focus(toplevelXid(widget));
}

/// Moves X input focus to `window`. Issued on GDK's connection even for a
/// window Chromium owns: XSetInputFocus is not restricted to the owner, and
/// this has to be ordered with the focus-widget change that provoked it.
pub fn focus(window: Window) void {
    if (window == 0) return;
    const c = conn() orelse return;
    c.push();
    _ = c.api.set_input_focus(c.x, window, REVERT_TO_PARENT, CURRENT_TIME);
    _ = c.api.flush(c.x);
    c.pop();
}

/// The window the X server currently reports as holding input focus, or 0.
/// PointerRoot and None both read as 0: neither is a window this engine set.
/// Untrapped on purpose: XGetInputFocus names no window, so it cannot raise a
/// window error, and it is a round trip. GDK's trap bookkeeping records the
/// request sequence a trap ends at, and a round trip inside one lets GDK end
/// that trap underneath us; popping it afterwards aborts the process on
/// `trap->end_sequence == 0`.
pub fn focused() Window {
    const c = conn() orelse return 0;
    var window: Window = 0;
    var revert: c_int = 0;
    _ = c.api.get_input_focus(c.x, &window, &revert);
    return if (window <= 1) 0 else window;
}

/// Button1Mask through Button5Mask of the modifier mask XQueryPointer answers
/// with.
const BUTTON_MASK: c_uint = 0x1f00;

/// True while any pointer button is held. Untrapped for the same reason
/// `focused` is: it is a round trip, and the only window it names is the root,
/// which cannot be gone.
pub fn pointerButtonsDown() bool {
    const c = conn() orelse return false;
    const root = c.api.default_root_window(c.x);
    var got_root: Window = 0;
    var child: Window = 0;
    var rx: c_int = 0;
    var ry: c_int = 0;
    var wx: c_int = 0;
    var wy: c_int = 0;
    var mask: c_uint = 0;
    if (c.api.query_pointer(c.x, root, &got_root, &child, &rx, &ry, &wx, &wy, &mask) == 0) return false;
    return (mask & BUTTON_MASK) != 0;
}

/// Moves an embedding container under a different toplevel, which is what a
/// tab dragged into another window needs: GTK relocates the widget, and the X
/// child holding the browser is GTK's to know nothing about. Remapped
/// afterwards because the server unmaps a mapped window it reparents, and
/// synced for the same reason creation is: CEF paints only into a viewable
/// window and reads that state on its own connection.
pub fn reparent(window: Window, parent: Window, x: c_int, y: c_int) void {
    if (window == 0 or parent == 0) return;
    const c = conn() orelse return;
    c.push();
    _ = c.api.reparent_window(c.x, window, parent, x, y);
    _ = c.api.map_window(c.x, window);
    _ = c.api.sync(c.x, 0);
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

/// The root's direct children, oldest first, copied into `out`. The answer is
/// the whole set of top-level windows on this display, which is what makes a
/// window Chrome put up on its own findable: it belongs to no CEF browser and
/// has no callback, but it is a child of the root and this process did not
/// create it.
/// The root's children, newest last. `truncated` reports a root with more
/// children than the caller's buffer: a real desktop can have hundreds of X
/// clients, and a watcher that silently reads the first few would stop seeing
/// windows exactly on the busy session where it matters.
pub fn rootChildren(out: []Window, truncated: *bool) []Window {
    truncated.* = false;
    const c = conn() orelse return out[0..0];
    const root = c.api.default_root_window(c.x);
    var parent: Window = 0;
    var got_root: Window = 0;
    var kids: [*]Window = undefined;
    var count: c_uint = 0;
    c.push();
    const ok = c.api.query_tree(c.x, root, &got_root, &parent, &kids, &count);
    c.pop();
    if (ok == 0) return out[0..0];
    defer _ = c.api.free(@ptrCast(kids));
    truncated.* = @as(usize, count) > out.len;
    const n = @min(out.len, @as(usize, count));
    for (0..n) |i| out[i] = kids[i];
    return out[0..n];
}

/// The top-level a window sits under: CEF answers `get_window_handle` with the
/// browser's own window, which for a window Chrome owns is a child of the
/// Widget's top-level, and unmapping the child leaves the frame on screen.
pub fn toplevelOf(window: Window) Window {
    if (window == 0) return 0;
    const c = conn() orelse return window;
    const root = c.api.default_root_window(c.x);
    var current = window;
    // Bounded rather than while(true): a broken tree would otherwise spin.
    for (0..16) |_| {
        var parent: Window = 0;
        var got_root: Window = 0;
        var kids: [*]Window = undefined;
        var count: c_uint = 0;
        c.push();
        const ok = c.api.query_tree(c.x, current, &got_root, &parent, &kids, &count);
        c.pop();
        if (ok == 0) return current;
        _ = c.api.free(@ptrCast(kids));
        if (parent == root or parent == 0) return current;
        current = parent;
    }
    return current;
}

/// True for a window GDK made: a toplevel, but also a popover or a menu, which
/// get X windows of their own without being GtkWindows. Chromium's browser
/// process is this process, so `_NET_WM_PID` does not tell its windows from the
/// app's; this does, because GDK only knows the ones it created itself.
pub fn isGdkSurface(window: Window) bool {
    const c = conn() orelse return false;
    c.push();
    const surface = c.api.surface_lookup(c.gdk, window);
    c.pop();
    return surface != null;
}

pub const Geometry = struct { x: c_int, y: c_int, w: c_uint, h: c_uint };

pub fn geometry(window: Window) ?Geometry {
    const c = conn() orelse return null;
    var root: Window = 0;
    var gx: c_int = 0;
    var gy: c_int = 0;
    var gw: c_uint = 0;
    var gh: c_uint = 0;
    var border: c_uint = 0;
    var depth: c_uint = 0;
    c.push();
    const ok = c.api.get_geometry(c.x, window, &root, &gx, &gy, &gw, &gh, &border, &depth);
    c.pop();
    if (ok == 0) return null;
    return .{ .x = gx, .y = gy, .w = gw, .h = gh };
}

/// `_NET_WM_PID`, or 0 when the window does not carry it. This is what tells a
/// window Chromium put up from every other client's window on the same display:
/// Chromium's browser process is this process, so its top-levels carry this
/// pid, and a window without the property is never touched.
pub fn windowPid(window: Window) u32 {
    const c = conn() orelse return 0;
    const atom = c.api.intern_atom(c.x, "_NET_WM_PID", 1);
    if (atom == 0) return 0;
    const XA_CARDINAL: c_ulong = 6;
    var actual_type: c_ulong = 0;
    var actual_format: c_int = 0;
    var nitems: c_ulong = 0;
    var bytes_after: c_ulong = 0;
    var data: [*]u8 = undefined;
    c.push();
    const ok = c.api.get_window_property(c.x, window, atom, 0, 1, 0, XA_CARDINAL, &actual_type, &actual_format, &nitems, &bytes_after, &data);
    c.pop();
    if (ok != 0 or nitems == 0 or actual_format != 32) return 0;
    defer _ = c.api.free(@ptrCast(data));
    // Format 32 means "long", not "32 bits", on a 64-bit server connection.
    const value = @as(*const c_ulong, @ptrCast(@alignCast(data))).*;
    return @truncate(value);
}

/// Tells the window manager that `window` is a dialog belonging to `parent`.
/// A compositor that manages XWayland top-levels itself places them by its own
/// rules and discards a client's ConfigureRequest; the two hints below are what
/// it reads instead, and a transient dialog is floated and centred on its
/// parent rather than tiled wherever a new top-level would go.
pub fn markDialogFor(window: Window, parent: Window) void {
    if (window == 0 or parent == 0) return;
    const c = conn() orelse return;
    const type_atom = c.api.intern_atom(c.x, "_NET_WM_WINDOW_TYPE", 0);
    const dialog_atom = c.api.intern_atom(c.x, "_NET_WM_WINDOW_TYPE_DIALOG", 0);
    const XA_ATOM: c_ulong = 4;
    const PROP_MODE_REPLACE: c_int = 0;
    c.push();
    _ = c.api.set_transient_for_hint(c.x, window, parent);
    if (type_atom != 0 and dialog_atom != 0) {
        var value: c_ulong = dialog_atom;
        _ = c.api.change_property(c.x, window, type_atom, XA_ATOM, 32, PROP_MODE_REPLACE, @ptrCast(&value), 1);
    }
    _ = c.api.flush(c.x);
    c.pop();
}

/// The window a window is a child of, or 0. A reparented dialog is told apart
/// from one still sitting on the root by this and nothing else.
pub const Origin = struct { x: c_int, y: c_int };

/// A window's origin in root coordinates.
pub fn originOnRoot(window: Window) Origin {
    const c = conn() orelse return .{ .x = 0, .y = 0 };
    const root = c.api.default_root_window(c.x);
    var rx: c_int = 0;
    var ry: c_int = 0;
    var child: Window = 0;
    c.push();
    _ = c.api.translate_coordinates(c.x, window, root, 0, 0, &rx, &ry, &child);
    c.pop();
    return .{ .x = rx, .y = ry };
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
