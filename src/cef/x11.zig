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
const glib = @import("glib");

pub const Window = c_ulong;
pub const Display = anyopaque;
const Visual = anyopaque;

/// Xlib's XSetWindowAttributes, laid out for LP64. Only a few fields are ever
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

const CW_BACK_PIXMAP: c_ulong = 1 << 0;
const CW_BORDER_PIXEL: c_ulong = 1 << 3;
const CW_BACKING_STORE: c_ulong = 1 << 6;
const ALWAYS: c_int = 2;
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
const FnSelectInput = *const fn (*Display, Window, c_long) callconv(.c) c_int;
const FnMoveWindow = *const fn (*Display, Window, c_int, c_int) callconv(.c) c_int;
const FnSendEvent = *const fn (*Display, Window, c_int, c_long, *anyopaque) callconv(.c) c_int;
const FnGetAttributes = *const fn (*Display, Window, *WindowAttributes) callconv(.c) c_int;

/// Xlib's XWindowAttributes. Only `your_event_mask` and `override_redirect`
/// are read.
const WindowAttributes = extern struct {
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
    border_width: c_int,
    depth: c_int,
    visual: ?*Visual,
    root: Window,
    class: c_int,
    bit_gravity: c_int,
    win_gravity: c_int,
    backing_store: c_int,
    backing_planes: c_ulong,
    backing_pixel: c_ulong,
    save_under: c_int,
    colormap: c_ulong,
    map_installed: c_int,
    map_state: c_int,
    all_event_masks: c_long,
    your_event_mask: c_long,
    do_not_propagate_mask: c_long,
    override_redirect: c_int,
    screen: ?*anyopaque,
};

const Api = struct {
    init_threads: FnInitThreads,
    create_window: FnCreateWindow,
    default_screen: FnDefaultScreen,
    default_depth: FnDefaultDepth,
    default_visual: FnDefaultVisual,
    default_colormap: FnDefaultColormap,
    map_window: FnWindowOnly,
    unmap_window: FnWindowOnly,
    raise_window: FnWindowOnly,
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
    select_input: FnSelectInput,
    send_event: FnSendEvent,
    move_window: FnMoveWindow,
    get_window_attributes: FnGetAttributes,
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
        .raise_window = x.lookup(FnWindowOnly, "XRaiseWindow") orelse return missing(&x, &g, "XRaiseWindow"),
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
        .get_window_attributes = x.lookup(FnGetAttributes, "XGetWindowAttributes") orelse return missing(&x, &g, "XGetWindowAttributes"),
        .query_pointer = x.lookup(FnQueryPointer, "XQueryPointer") orelse return missing(&x, &g, "XQueryPointer"),
        .select_input = x.lookup(FnSelectInput, "XSelectInput") orelse return missing(&x, &g, "XSelectInput"),
        .send_event = x.lookup(FnSendEvent, "XSendEvent") orelse return missing(&x, &g, "XSendEvent"),
        .move_window = x.lookup(FnMoveWindow, "XMoveWindow") orelse return missing(&x, &g, "XMoveWindow"),
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
    // Background None: the server leaves an exposed area as it was instead of
    // clearing it. A tab switched to is a container moved back on screen, and
    // with a black background that read as a black frame until the page had
    // drawn into it; with none, the page it replaces stays until it has.
    //
    // Backing store: a parked page is off screen, and a window off screen
    // keeps nothing of what it showed, so moving it back exposed it and the
    // tab switched to waited on Chromium redrawing it (30 to 50 ms on an
    // Intel iGPU). Kept, its last frame is on show as soon as it is back.
    var attrs: SetWindowAttributes = .{
        .background_pixmap = 0,
        .border_pixel = 0,
        .backing_store = ALWAYS,
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
        CW_BACK_PIXMAP | CW_BORDER_PIXEL | CW_BACKING_STORE | CW_COLORMAP,
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

const FnXiUngrabDevice = *const fn (*Display, c_int, c_ulong) callconv(.c) c_int;
const FnGdkX11DeviceId = *const fn (*gdk.Device) callconv(.c) c_int;

var xi_attempted = false;
var xi_ungrab: ?FnXiUngrabDevice = null;
var gdk_device_id: ?FnGdkX11DeviceId = null;

/// Releases the XInput2 pointer and keyboard grabs GDK took for an autohide
/// popup. GDK grabs both devices with owner_events, so X delivers an event to
/// a window of GDK's own connection where it lands and everything else to the
/// popup's surface; Chromium's windows belong to Chromium's connection, so a
/// page inside a popover saw no click and no key at all. GDK keeps its own
/// record of the grab, which is what still closes the popover on a press
/// anywhere else in the app's windows.
pub fn releaseSeatGrab() void {
    const c = conn() orelse return;
    if (!xi_attempted) {
        xi_attempted = true;
        var xi = std.DynLib.open("libXi.so.6") catch std.DynLib.open("libXi.so") catch {
            std.debug.print("ND_WARN CEF: libXi not found; a page in a popover gets no input\n", .{});
            return;
        };
        xi_ungrab = xi.lookup(FnXiUngrabDevice, "XIUngrabDevice");
        gdk_device_id = gtklib.lookup(FnGdkX11DeviceId, "gdk_x11_device_get_id");
    }
    const ungrab = xi_ungrab orelse return;
    const device_id = gdk_device_id orelse return;
    const seat = gdk.Display.getDefaultSeat(c.gdk) orelse return;
    c.push();
    defer c.pop();
    if (gdk.Seat.getPointer(seat)) |pointer| _ = ungrab(c.x, device_id(pointer), CURRENT_TIME);
    if (gdk.Seat.getKeyboard(seat)) |keyboard| _ = ungrab(c.x, device_id(keyboard), CURRENT_TIME);
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

/// `reparent` on a connection that is not GDK's: CEF's own, for the Views
/// window it owns, from the CEF UI thread. Left unmapped, so the window is
/// never a top-level the window manager sees; CEF maps it when it shows it.
pub fn reparentOn(dpy: *anyopaque, window: Window, parent: Window, x: c_int, y: c_int) void {
    if (window == 0 or parent == 0) return;
    const a = loadApi() orelse return;
    const d: *Display = @ptrCast(dpy);
    _ = a.reparent_window(d, window, parent, x, y);
    _ = a.sync(d, 0);
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

/// Puts `window` above its siblings. The page and the docked inspector are
/// both children of the view's container, and the page is drawn inside the
/// inspector's own area, so the stacking order is what makes it visible.
pub fn raise(window: Window) void {
    if (window == 0) return;
    const c = conn() orelse return;
    c.push();
    _ = c.api.raise_window(c.x, window);
    _ = c.api.flush(c.x);
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
    return gdkSurface(window) != null;
}

/// The GDK surface behind `window`, when GDK made it.
pub fn gdkSurface(window: Window) ?*gdk.Surface {
    const c = conn() orelse return null;
    c.push();
    const surface = c.api.surface_lookup(c.gdk, window);
    c.pop();
    return surface;
}

/// True for a window the window manager is told to leave alone: Chromium's
/// menus, <select> lists, tooltips and autofill, which it places itself
/// against the page.
pub fn isOverrideRedirect(window: Window) bool {
    const c = conn() orelse return false;
    var attrs: WindowAttributes = undefined;
    c.push();
    const ok = c.api.get_window_attributes(c.x, window, &attrs);
    c.pop();
    return ok != 0 and attrs.override_redirect != 0;
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

// ---- XGetImage: a still of the page --------------------------------------------

/// Xlib's XImage, LP64, up to the fields read here. Only ever handled through
/// the pointer XGetImage returns.
const XImage = extern struct {
    width: c_int,
    height: c_int,
    xoffset: c_int,
    format: c_int,
    data: ?[*]u8,
    byte_order: c_int,
    bitmap_unit: c_int,
    bitmap_bit_order: c_int,
    bitmap_pad: c_int,
    depth: c_int,
    bytes_per_line: c_int,
    bits_per_pixel: c_int,
};
const FnGetImage = *const fn (*Display, Window, c_int, c_int, c_uint, c_uint, c_ulong, c_int) callconv(.c) ?*XImage;
const FnDestroyImage = *const fn (*XImage) callconv(.c) c_int;
const z_pixmap: c_int = 2;
const lsb_first: c_int = 0;

var image_attempted = false;
var get_image: ?FnGetImage = null;
var destroy_image: ?FnDestroyImage = null;

/// What `window` shows right now, as a texture GTK can draw in its place, or
/// null when the server cannot read it back as 32-bit little-endian pixels
/// (any other layout is left to the caller's fallback rather than converted).
/// Inferiors of the same depth are included, which is where Chromium draws.
///
/// Through a shared memory segment where the server offers one (MIT-SHM):
/// a page's worth of pixels written down the socket is most of the cost
/// otherwise, and this runs inside the frame a slide starts on.
pub fn capture(window: Window) ?*gdk.Texture {
    if (window == 0) return null;
    if (!image_attempted) {
        image_attempted = true;
        get_image = xlib.lookup(FnGetImage, "XGetImage");
        destroy_image = xlib.lookup(FnDestroyImage, "XDestroyImage");
    }
    const destroyImage = destroy_image orelse return null;
    const c = conn() orelse return null;
    var attrs: WindowAttributes = undefined;
    c.push();
    const known = c.api.get_window_attributes(c.x, window, &attrs);
    c.pop();
    if (known == 0 or attrs.width < 2 or attrs.height < 2) return null;
    const w: c_uint = @intCast(attrs.width);
    const h: c_uint = @intCast(attrs.height);
    if (captureShm(c, window, attrs, w, h)) |tex| return tex;
    const getImage = get_image orelse return null;
    c.push();
    const img = getImage(c.x, window, 0, 0, w, h, ~@as(c_ulong, 0), z_pixmap);
    c.pop();
    const image = img orelse return null;
    defer _ = destroyImage(image);
    return textureOf(image);
}

fn textureOf(image: *XImage) ?*gdk.Texture {
    if (image.bits_per_pixel != 32 or image.byte_order != lsb_first or image.data == null) return null;
    const stride: usize = @intCast(image.bytes_per_line);
    const rows: usize = @intCast(image.height);
    const bytes = glib.Bytes.new(image.data.?, stride * rows);
    defer glib.Bytes.unref(bytes);
    const tex = gdk.MemoryTexture.new(image.width, image.height, .b8g8r8x8, bytes, stride);
    return @ptrCast(tex);
}

/// Xext's XShmSegmentInfo.
const ShmSegment = extern struct {
    shmseg: c_ulong = 0,
    shmid: c_int = -1,
    shmaddr: ?[*]u8 = null,
    read_only: c_int = 0,
};
const FnShmQuery = *const fn (*Display) callconv(.c) c_int;
const FnShmCreateImage = *const fn (*Display, ?*Visual, c_uint, c_int, ?[*]u8, *ShmSegment, c_uint, c_uint) callconv(.c) ?*XImage;
const FnShmAttach = *const fn (*Display, *ShmSegment) callconv(.c) c_int;
const FnShmGetImage = *const fn (*Display, Window, *XImage, c_int, c_int, c_ulong) callconv(.c) c_int;

extern "c" fn shmget(key: c_int, size: usize, flags: c_int) c_int;
extern "c" fn shmat(id: c_int, addr: ?*anyopaque, flags: c_int) ?*anyopaque;
extern "c" fn shmdt(addr: ?*const anyopaque) c_int;
extern "c" fn shmctl(id: c_int, cmd: c_int, buf: ?*anyopaque) c_int;
const ipc_private: c_int = 0;
const ipc_creat: c_int = 0o1000;
const ipc_rmid: c_int = 0;

const Shm = struct {
    query: FnShmQuery,
    create_image: FnShmCreateImage,
    attach: FnShmAttach,
    get_image: FnShmGetImage,
    detach: FnShmAttach,
};
var shm_attempted = false;
var shm: ?Shm = null;
/// One segment, kept attached and grown when a bigger page asks: attaching is
/// a round trip of its own.
var shm_segment: ShmSegment = .{};
var shm_size: usize = 0;

fn loadShm(c: Conn) ?*const Shm {
    if (!shm_attempted) {
        shm_attempted = true;
        var ext = std.DynLib.open("libXext.so.6") catch std.DynLib.open("libXext.so") catch return null;
        const fns: Shm = .{
            .query = ext.lookup(FnShmQuery, "XShmQueryExtension") orelse return null,
            .create_image = ext.lookup(FnShmCreateImage, "XShmCreateImage") orelse return null,
            .attach = ext.lookup(FnShmAttach, "XShmAttach") orelse return null,
            .get_image = ext.lookup(FnShmGetImage, "XShmGetImage") orelse return null,
            .detach = ext.lookup(FnShmAttach, "XShmDetach") orelse return null,
        };
        if (fns.query(c.x) == 0) return null;
        shm = fns;
    }
    return if (shm != null) &shm.? else null;
}

fn captureShm(c: Conn, window: Window, attrs: WindowAttributes, w: c_uint, h: c_uint) ?*gdk.Texture {
    const ext = loadShm(c) orelse return null;
    const destroyImage = destroy_image orelse return null;
    if (attrs.depth != 24 and attrs.depth != 32) return null;
    const size: usize = @as(usize, w) * @as(usize, h) * 4;
    if (size > shm_size) {
        if (shm_segment.shmaddr != null) {
            c.push();
            _ = ext.detach(c.x, &shm_segment);
            _ = c.api.sync(c.x, 0);
            c.pop();
            _ = shmdt(shm_segment.shmaddr);
            shm_segment = .{};
            shm_size = 0;
        }
        const id = shmget(ipc_private, size, ipc_creat | 0o600);
        if (id < 0) return null;
        const addr = shmat(id, null, 0);
        // Marked for removal at once: it goes when the last process detaches,
        // a crash included.
        _ = shmctl(id, ipc_rmid, null);
        if (addr == null or @intFromPtr(addr) == std.math.maxInt(usize)) return null;
        shm_segment = .{ .shmid = id, .shmaddr = @ptrCast(addr), .read_only = 0 };
        c.push();
        const attached = ext.attach(c.x, &shm_segment);
        _ = c.api.sync(c.x, 0);
        c.pop();
        if (attached == 0) {
            _ = shmdt(addr);
            shm_segment = .{};
            return null;
        }
        shm_size = size;
    }
    // The image names the segment it reads into (XShmGetImage finds it there).
    const image = ext.create_image(c.x, attrs.visual, @intCast(attrs.depth), z_pixmap, shm_segment.shmaddr, &shm_segment, w, h) orelse return null;
    defer _ = destroyImage(image);
    if (@as(usize, @intCast(image.bytes_per_line)) * @as(usize, @intCast(image.height)) > shm_size) {
        image.data = null;
        return null;
    }
    c.push();
    const ok = ext.get_image(c.x, window, image, 0, 0, ~@as(c_ulong, 0));
    c.pop();
    defer image.data = null;
    if (ok == 0) return null;
    return textureOf(image);
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

/// Marks a page's window minimized, or not, the way a window manager would:
/// `_NET_WM_STATE_HIDDEN` in its `_NET_WM_STATE`. Chromium watches that
/// property on its own window and treats a minimized window's contents as
/// hidden. No window manager manages a child window, so nothing else writes it
/// but Chromium itself, whose own atoms (`_NET_WM_STATE_SKIP_TASKBAR`) stay.
pub fn setMinimized(window: Window, minimized: bool) void {
    if (window == 0) return;
    const c = conn() orelse return;
    const state = c.api.intern_atom(c.x, "_NET_WM_STATE", 0);
    const hidden = c.api.intern_atom(c.x, "_NET_WM_STATE_HIDDEN", 0);
    if (state == 0 or hidden == 0) return;
    const XA_ATOM: c_ulong = 4;
    const PROP_MODE_REPLACE: c_int = 0;
    var atoms: [16]c_ulong = undefined;
    var n: usize = 0;
    var actual_type: c_ulong = 0;
    var actual_format: c_int = 0;
    var nitems: c_ulong = 0;
    var bytes_after: c_ulong = 0;
    var data: [*]u8 = undefined;
    c.push();
    defer c.pop();
    if (c.api.get_window_property(c.x, window, state, 0, atoms.len, 0, XA_ATOM, &actual_type, &actual_format, &nitems, &bytes_after, &data) == 0) {
        defer _ = c.api.free(@ptrCast(data));
        if (actual_format == 32) {
            // Format 32 is a C long per item on the client side.
            const items: [*]const c_ulong = @ptrCast(@alignCast(data));
            for (items[0..@min(@as(usize, nitems), atoms.len - 1)]) |atom| {
                if (atom == hidden) continue;
                atoms[n] = atom;
                n += 1;
            }
        }
    }
    if (minimized) {
        atoms[n] = hidden;
        n += 1;
    }
    _ = c.api.change_property(c.x, window, state, XA_ATOM, 32, PROP_MODE_REPLACE, @ptrCast(&atoms), @intCast(n));
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

/// Copies `WM_CLASS` from `from` onto `to`, and answers whether `to` now has
/// one. Every window Chromium puts on the root (a dialog, the bubble it shows
/// on entering fullscreen) carries neither a class nor a name, and that is
/// what the compositor hands anything that enumerates windows: the owner's
/// screenshot picker offers it as an untitled window beside the app, and a
/// Hyprland rule keyed on an empty class and title moves it off to a corner.
/// Copied from the app's own toplevel rather than composed here, so the two
/// always agree on whatever GDK set.
pub fn copyClass(from: Window, to: Window) bool {
    if (from == 0 or to == 0) return false;
    const c = conn() orelse return false;
    const XA_WM_CLASS: c_ulong = 67;
    const XA_STRING: c_ulong = 31;
    const PROP_MODE_REPLACE: c_int = 0;
    var actual_type: c_ulong = 0;
    var actual_format: c_int = 0;
    var nitems: c_ulong = 0;
    var bytes_after: c_ulong = 0;
    var data: [*]u8 = undefined;
    c.push();
    const ok = c.api.get_window_property(c.x, from, XA_WM_CLASS, 0, 256, 0, XA_STRING, &actual_type, &actual_format, &nitems, &bytes_after, &data);
    c.pop();
    if (ok != 0 or nitems == 0 or actual_format != 8) return false;
    defer _ = c.api.free(@ptrCast(data));
    c.push();
    _ = c.api.change_property(c.x, to, XA_WM_CLASS, XA_STRING, 8, PROP_MODE_REPLACE, data, @intCast(nitems));
    _ = c.api.flush(c.x);
    c.pop();
    return true;
}

// ---- Naming Chromium's popups before the compositor maps them -------------
// Chromium names none of its popups (a <select> list, autofill, a tooltip):
// no WM_CLASS, no title. A compositor applies its window rules once, on a
// window's first frame, and the owner's Hyprland has one for windows with an
// empty class and title (Chrome's notifications) that floats them and pins
// them to the monitor's top right, so every <select> list opened there. The
// engine's watch stamps the app's class on Chromium's windows every 200 ms,
// and even GDK's own read of the CreateNotify lost the race whenever the GTK
// thread was busy laying out. This thread does it on a connection of its own
// as the CreateNotify arrives, ahead of the popup's first frame.
//
// XCB rather than Xlib: a request against a window that is already gone comes
// back as an event here, where Xlib's default handler would end the process.

const XcbConn = opaque {};
const XcbGenericEvent = extern struct { response_type: u8, pad0: u8, sequence: u16 };
const XcbCreateNotify = extern struct {
    response_type: u8,
    pad0: u8,
    sequence: u16,
    parent: u32,
    window: u32,
};
const XcbScreenIterator = extern struct { data: ?*const extern struct { root: u32 }, rem: c_int, index: c_int };
const XcbSetupHead = extern struct {
    status: u8,
    pad0: u8,
    protocol_major_version: u16,
    protocol_minor_version: u16,
    length: u16,
    release_number: u32,
    resource_id_base: u32,
    resource_id_mask: u32,
};
const XcbVoidCookie = extern struct { sequence: c_uint };

const Xcb = struct {
    connect: *const fn (?[*:0]const u8, ?*c_int) callconv(.c) ?*XcbConn,
    has_error: *const fn (*XcbConn) callconv(.c) c_int,
    get_setup: *const fn (*XcbConn) callconv(.c) *const XcbSetupHead,
    roots: *const fn (*const XcbSetupHead) callconv(.c) XcbScreenIterator,
    change_attributes: *const fn (*XcbConn, u32, u32, [*]const u32) callconv(.c) XcbVoidCookie,
    change_property: *const fn (*XcbConn, u8, u32, u32, u32, u8, u32, [*]const u8) callconv(.c) XcbVoidCookie,
    wait_for_event: *const fn (*XcbConn) callconv(.c) ?*XcbGenericEvent,
    flush: *const fn (*XcbConn) callconv(.c) c_int,
};

const XCB_CW_EVENT_MASK: u32 = 1 << 11;
const XCB_CREATE_NOTIFY: u8 = 16;
const XCB_ATOM_WM_CLASS: u32 = 67;
const XCB_ATOM_STRING: u32 = 31;

var namer_started = false;
/// Any window Chromium's connection made; every window it makes shares the
/// client part of this id. 0 until the first browser exists.
var namer_chromium_window: std.atomic.Value(u32) = .init(0);
/// The app toplevel's WM_CLASS, written once by the GTK thread before
/// `namer_class_len` is published.
var namer_class: [256]u8 = undefined;
var namer_class_len: std.atomic.Value(usize) = .init(0);

/// Starts the thread, once per process. Called from the CEF UI thread with a
/// window Chromium made, which is what names Chromium's client.
pub fn nameChromiumWindowsFrom(chromium_window: Window) void {
    if (chromium_window == 0) return;
    namer_chromium_window.store(@truncate(chromium_window), .release);
    if (@atomicRmw(bool, &namer_started, .Xchg, true, .acq_rel)) return;
    _ = std.Thread.spawn(.{}, namerLoop, .{}) catch {
        std.debug.print("ND_WARN CEF: the popup namer did not start; Chromium's popups stay nameless until the watch names them\n", .{});
    };
}

/// Hands the namer the class to stamp, read off the app's toplevel. GTK thread;
/// a no-op once published.
pub fn publishClass(toplevel: Window) void {
    if (toplevel == 0 or namer_class_len.load(.acquire) != 0) return;
    const c = conn() orelse return;
    const XA_WM_CLASS: c_ulong = 67;
    const XA_STRING: c_ulong = 31;
    var actual_type: c_ulong = 0;
    var actual_format: c_int = 0;
    var nitems: c_ulong = 0;
    var bytes_after: c_ulong = 0;
    var data: [*]u8 = undefined;
    c.push();
    const ok = c.api.get_window_property(c.x, toplevel, XA_WM_CLASS, 0, 64, 0, XA_STRING, &actual_type, &actual_format, &nitems, &bytes_after, &data);
    c.pop();
    if (ok != 0 or nitems == 0 or actual_format != 8) return;
    defer _ = c.api.free(@ptrCast(data));
    const n = @min(@as(usize, nitems), namer_class.len);
    @memcpy(namer_class[0..n], data[0..n]);
    namer_class_len.store(n, .release);
}

fn namerLoop() void {
    var lib = std.DynLib.open("libxcb.so.1") catch return;
    const x: Xcb = .{
        .connect = lib.lookup(@FieldType(Xcb, "connect"), "xcb_connect") orelse return,
        .has_error = lib.lookup(@FieldType(Xcb, "has_error"), "xcb_connection_has_error") orelse return,
        .get_setup = lib.lookup(@FieldType(Xcb, "get_setup"), "xcb_get_setup") orelse return,
        .roots = lib.lookup(@FieldType(Xcb, "roots"), "xcb_setup_roots_iterator") orelse return,
        .change_attributes = lib.lookup(@FieldType(Xcb, "change_attributes"), "xcb_change_window_attributes") orelse return,
        .change_property = lib.lookup(@FieldType(Xcb, "change_property"), "xcb_change_property") orelse return,
        .wait_for_event = lib.lookup(@FieldType(Xcb, "wait_for_event"), "xcb_wait_for_event") orelse return,
        .flush = lib.lookup(@FieldType(Xcb, "flush"), "xcb_flush") orelse return,
    };
    const c = x.connect(null, null) orelse return;
    if (x.has_error(c) != 0) return;
    const setup = x.get_setup(c);
    const mask = setup.resource_id_mask;
    const screen = x.roots(setup).data orelse return;
    // SubstructureNotify on the root is shared: every client may select it.
    const events = [_]u32{1 << 19};
    _ = x.change_attributes(c, screen.root, XCB_CW_EVENT_MASK, &events);
    _ = x.flush(c);
    while (x.wait_for_event(c)) |event| {
        defer std.c.free(event);
        if (event.response_type & 0x7f != XCB_CREATE_NOTIFY) continue;
        const created: *const XcbCreateNotify = @ptrCast(@alignCast(event));
        const known = namer_chromium_window.load(.acquire);
        if (known == 0 or (created.window & ~mask) != (known & ~mask)) continue;
        const len = namer_class_len.load(.acquire);
        if (len == 0) continue;
        _ = x.change_property(c, 0, created.window, XCB_ATOM_WM_CLASS, XCB_ATOM_STRING, 8, @intCast(len), &namer_class);
        _ = x.flush(c);
    }
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

const SUBSTRUCTURE_NOTIFY_MASK: c_long = 1 << 19;
const MAP_NOTIFY: c_int = 19;
const CONFIGURE_NOTIFY: c_int = 22;

/// Adds SubstructureNotify to what this connection already selects on the
/// root, so a top-level Chromium maps or resizes reaches GDK's `xevent` signal
/// the moment it happens. XSelectInput replaces the client's
/// whole mask, hence the read first.
pub fn watchRootMaps() bool {
    const c = conn() orelse return false;
    const root = c.api.default_root_window(c.x);
    var attrs: WindowAttributes = undefined;
    c.push();
    defer c.pop();
    if (c.api.get_window_attributes(c.x, root, &attrs) == 0) return false;
    _ = c.api.select_input(c.x, root, attrs.your_event_mask | SUBSTRUCTURE_NOTIFY_MASK);
    return true;
}

/// The window a MapNotify or ConfigureNotify is about, or 0 for any other
/// event. Both put the window at the same offset on LP64: type, serial,
/// send_event, display, event, window.
pub fn mappedWindow(xevent: *const anyopaque) Window {
    const bytes: [*]const u8 = @ptrCast(xevent);
    const kind = std.mem.readInt(c_int, bytes[0..4], .little);
    if (kind != MAP_NOTIFY and kind != CONFIGURE_NOTIFY) return 0;
    return std.mem.readInt(c_ulong, bytes[40..48], .little);
}

/// GDK's X display object, whose `xevent` signal carries every event read
/// off its connection.
pub fn gdkDisplay() ?*gdk.Display {
    const c = conn() orelse return null;
    return c.gdk;
}

// ---- XShape: cutting the page's window to what GTK shows of it --------------
// An X child window is drawn above everything its parent paints, so a GTK
// widget laid over a page is invisible under it and a page in a rounded card
// shows square corners. The Shape extension gives the child window a bounding
// region instead, and whatever is outside it is the parent's again, both for
// drawing and for input. libXext is optional: without it the page stays a
// rectangle, which is what it was before.

pub const Rect = @import("shape.zig").Rect;

const FnShapeRects = *const fn (*Display, Window, c_int, c_int, c_int, [*]const Rect, c_int, c_int, c_int) callconv(.c) void;
const FnShapeMask = *const fn (*Display, Window, c_int, c_int, c_int, c_ulong, c_int) callconv(.c) void;
const shape_bounding: c_int = 0;
const shape_input: c_int = 2;
const shape_set: c_int = 0;
const yx_banded: c_int = 3;

var shape_attempted = false;
var shape_rects: ?FnShapeRects = null;
var shape_mask: ?FnShapeMask = null;

fn loadShape() bool {
    if (!shape_attempted) {
        shape_attempted = true;
        var ext = std.DynLib.open("libXext.so.6") catch std.DynLib.open("libXext.so") catch {
            std.debug.print("ND_WARN CEF: libXext not found; a page keeps square corners and covers overlays\n", .{});
            return false;
        };
        shape_rects = ext.lookup(FnShapeRects, "XShapeCombineRectangles");
        shape_mask = ext.lookup(FnShapeMask, "XShapeCombineMask");
    }
    return shape_rects != null and shape_mask != null;
}

const FnSetBackgroundPixmap = *const fn (*Display, Window, c_ulong) callconv(.c) c_int;
var set_background_pixmap: ?FnSetBackgroundPixmap = null;
var background_attempted = false;

/// Takes `window`'s background away (`None`): the server then leaves what is
/// already in a region that becomes exposed instead of filling it. The page's
/// window drawn there is what stays, until the toolkit has drawn over it.
pub fn keepExposedContents(window: Window) void {
    if (window == 0) return;
    if (!background_attempted) {
        background_attempted = true;
        set_background_pixmap = xlib.lookup(FnSetBackgroundPixmap, "XSetWindowBackgroundPixmap");
    }
    const f = set_background_pixmap orelse return;
    const c = conn() orelse return;
    c.push();
    _ = f(c.x, window, 0);
    c.pop();
}

/// Gives `window` the bounding region `rects` (window coordinates, device
/// pixels, y-x banded). An empty list hides the whole window.
pub fn setShape(window: Window, rects: []const Rect) void {
    if (window == 0 or !loadShape()) return;
    const c = conn() orelse return;
    c.push();
    shape_rects.?(c.x, window, shape_bounding, 0, 0, rects.ptr, @intCast(rects.len), shape_set, yx_banded);
    c.pop();
}

/// Back to the plain rectangle.
pub fn clearShape(window: Window) void {
    if (window == 0 or !loadShape()) return;
    const c = conn() orelse return;
    c.push();
    shape_mask.?(c.x, window, shape_bounding, 0, 0, 0, shape_set);
    c.pop();
}

/// Lets the pointer through `window` to whatever is under it (an empty input
/// region), or gives it back its own. The bounding shape is left alone.
pub fn setInputPassthrough(window: Window, through: bool) void {
    if (window == 0 or !loadShape()) return;
    const c = conn() orelse return;
    c.push();
    if (through) {
        const none: [0]Rect = .{};
        shape_rects.?(c.x, window, shape_input, 0, 0, &none, 0, shape_set, yx_banded);
    } else {
        shape_mask.?(c.x, window, shape_input, 0, 0, 0, shape_set);
    }
    c.pop();
}

/// The window manager's own list of the top-levels it manages
/// (`_NET_CLIENT_LIST`), which names a client window whether or not the WM
/// reparented it into a frame. The root's children are the frames under a
/// reparenting WM (openbox, mutter, KWin), so a window Chromium maps and the
/// WM frames before the next look at the root is only findable here.
pub fn clientList(out: []Window) []Window {
    const c = conn() orelse return out[0..0];
    const atom = c.api.intern_atom(c.x, "_NET_CLIENT_LIST", 1);
    if (atom == 0) return out[0..0];
    const XA_WINDOW: c_ulong = 33;
    var actual_type: c_ulong = 0;
    var actual_format: c_int = 0;
    var nitems: c_ulong = 0;
    var bytes_after: c_ulong = 0;
    var data: [*]u8 = undefined;
    const root = c.api.default_root_window(c.x);
    c.push();
    const ok = c.api.get_window_property(c.x, root, atom, 0, @intCast(out.len), 0, XA_WINDOW, &actual_type, &actual_format, &nitems, &bytes_after, &data);
    c.pop();
    if (ok != 0 or actual_format != 32) return out[0..0];
    defer _ = c.api.free(@ptrCast(data));
    // Format 32 is a C long per item on the client side.
    const items: [*]const c_ulong = @ptrCast(@alignCast(data));
    const n = @min(out.len, @as(usize, nitems));
    for (0..n) |i| out[i] = items[i];
    return out[0..n];
}

/// WM_NORMAL_HINTS with an aspect ratio (PAspect): the video's own shape, kept
/// as the window is resized. Chromium's floating video sets it before it maps,
/// and no other window of Chromium's carries it.
pub fn keepsAspect(window: Window) bool {
    const c = conn() orelse return false;
    var actual_type: c_ulong = 0;
    var actual_format: c_int = 0;
    var nitems: c_ulong = 0;
    var bytes_after: c_ulong = 0;
    var data: [*]u8 = undefined;
    const XA_WM_NORMAL_HINTS: c_ulong = 40;
    const XA_WM_SIZE_HINTS: c_ulong = 41;
    const PAspect: c_ulong = 1 << 7;
    c.push();
    const ok = c.api.get_window_property(c.x, window, XA_WM_NORMAL_HINTS, 0, 18, 0, XA_WM_SIZE_HINTS, &actual_type, &actual_format, &nitems, &bytes_after, &data);
    c.pop();
    if (ok != 0 or actual_format != 32) return false;
    defer _ = c.api.free(@ptrCast(data));
    return nitems >= 1 and @as(*const c_ulong, @ptrCast(@alignCast(data))).* & PAspect != 0;
}

/// True for Chromium's picture-in-picture window, the one window it puts on
/// the root that keeps a fixed aspect ratio, stays above every other and shows
/// on every workspace. The aspect hint is there from the start; the other two
/// are EWMH requests that reach the properties only once the window manager
/// has acted on them (Hyprland never writes the keep-above one), and they
/// cover a window that lost its hint.
pub fn pictureInPicture(window: Window) bool {
    const c = conn() orelse return false;
    const XA_ATOM: c_ulong = 4;
    const XA_CARDINAL: c_ulong = 6;
    var actual_type: c_ulong = 0;
    var actual_format: c_int = 0;
    var nitems: c_ulong = 0;
    var bytes_after: c_ulong = 0;
    var data: [*]u8 = undefined;
    if (keepsAspect(window)) return true;
    const desktop = c.api.intern_atom(c.x, "_NET_WM_DESKTOP", 1);
    if (desktop != 0) {
        c.push();
        const ok = c.api.get_window_property(c.x, window, desktop, 0, 1, 0, XA_CARDINAL, &actual_type, &actual_format, &nitems, &bytes_after, &data);
        c.pop();
        if (ok == 0 and actual_format == 32) {
            defer _ = c.api.free(@ptrCast(data));
            if (nitems == 1 and @as(*const c_ulong, @ptrCast(@alignCast(data))).* == 0xFFFFFFFF) return true;
        }
    }
    const state = c.api.intern_atom(c.x, "_NET_WM_STATE", 1);
    const above = c.api.intern_atom(c.x, "_NET_WM_STATE_ABOVE", 1);
    if (state == 0 or above == 0) return false;
    c.push();
    const ok = c.api.get_window_property(c.x, window, state, 0, 32, 0, XA_ATOM, &actual_type, &actual_format, &nitems, &bytes_after, &data);
    c.pop();
    if (ok != 0 or actual_format != 32) return false;
    defer _ = c.api.free(@ptrCast(data));
    const atoms: [*]const c_ulong = @ptrCast(@alignCast(data));
    for (0..@as(usize, nitems)) |i| {
        if (atoms[i] == above) return true;
    }
    return false;
}

/// Asks for `window` at root position `x`, `y`. A configure request: the
/// window manager decides, and one that tiles the window ignores it.
pub fn moveWindow(window: Window, x: c_int, y: c_int) void {
    const c = conn() orelse return;
    c.push();
    _ = c.api.move_window(c.x, window, x, y);
    _ = c.api.flush(c.x);
    c.pop();
}

pub const Area = struct { x: c_int, y: c_int, w: c_int, h: c_int };

/// The window manager's work area (`_NET_WORKAREA` for the current desktop):
/// the screen less its panels. Null when the manager publishes none.
pub fn workArea() ?Area {
    const c = conn() orelse return null;
    const atom = c.api.intern_atom(c.x, "_NET_WORKAREA", 1);
    if (atom == 0) return null;
    const XA_CARDINAL: c_ulong = 6;
    var actual_type: c_ulong = 0;
    var actual_format: c_int = 0;
    var nitems: c_ulong = 0;
    var bytes_after: c_ulong = 0;
    var data: [*]u8 = undefined;
    c.push();
    const ok = c.api.get_window_property(c.x, c.api.default_root_window(c.x), atom, 0, 4, 0, XA_CARDINAL, &actual_type, &actual_format, &nitems, &bytes_after, &data);
    c.pop();
    if (ok != 0 or nitems < 4 or actual_format != 32) return null;
    defer _ = c.api.free(@ptrCast(data));
    const v: [*]const c_ulong = @ptrCast(@alignCast(data));
    return .{ .x = @intCast(v[0]), .y = @intCast(v[1]), .w = @intCast(v[2]), .h = @intCast(v[3]) };
}

/// The title every picture-in-picture window carries, whatever the page calls
/// itself: what a window rule can match, and what the Hyprland pin looks for.
pub const picture_in_picture_title = "Picture in Picture";

/// What a window manager rule can hold on to for a picture-in-picture window,
/// set on Chromium's window: the app's own class with an instance of its own
/// ("picture-in-picture"), a fixed title, the utility type a tiling manager
/// floats by default, and above plus sticky. The state goes as an EWMH request
/// to the window manager, since the window is already mapped; the others are
/// properties it reads (most only at map, which Chromium did before this runs,
/// so the title and class are what rules should use).
pub fn markPictureInPicture(window: Window, app: Window) void {
    if (window == 0) return;
    const c = conn() orelse return;
    const XA_ATOM: c_ulong = 4;
    const XA_STRING: c_ulong = 31;
    const XA_WM_CLASS: c_ulong = 67;
    const PROP_MODE_REPLACE: c_int = 0;
    c.push();
    defer c.pop();
    // The class half of the app's own WM_CLASS ("instance\0class\0").
    var class_buf: [256]u8 = undefined;
    var class_name: []const u8 = "";
    if (app != 0) {
        var actual_type: c_ulong = 0;
        var actual_format: c_int = 0;
        var nitems: c_ulong = 0;
        var bytes_after: c_ulong = 0;
        var data: [*]u8 = undefined;
        if (c.api.get_window_property(c.x, app, XA_WM_CLASS, 0, 64, 0, XA_STRING, &actual_type, &actual_format, &nitems, &bytes_after, &data) == 0 and actual_format == 8 and nitems > 0) {
            defer _ = c.api.free(@ptrCast(data));
            const raw = data[0..@intCast(nitems)];
            const first = std.mem.indexOfScalar(u8, raw, 0) orelse raw.len;
            const rest = raw[@min(first + 1, raw.len)..];
            const second = std.mem.indexOfScalar(u8, rest, 0) orelse rest.len;
            const n = @min(second, class_buf.len);
            @memcpy(class_buf[0..n], rest[0..n]);
            class_name = class_buf[0..n];
        }
    }
    if (class_name.len > 0) {
        var value: [300]u8 = undefined;
        const instance = "picture-in-picture";
        const composed = std.fmt.bufPrint(&value, "{s}\x00{s}\x00", .{ instance, class_name }) catch "";
        if (composed.len > 0) _ = c.api.change_property(c.x, window, XA_WM_CLASS, XA_STRING, 8, PROP_MODE_REPLACE, composed.ptr, @intCast(composed.len));
    }
    setTitleLocked(c, window);
    const type_atom = c.api.intern_atom(c.x, "_NET_WM_WINDOW_TYPE", 0);
    const utility = c.api.intern_atom(c.x, "_NET_WM_WINDOW_TYPE_UTILITY", 0);
    if (type_atom != 0 and utility != 0) {
        var value: c_ulong = utility;
        _ = c.api.change_property(c.x, window, type_atom, XA_ATOM, 32, PROP_MODE_REPLACE, @ptrCast(&value), 1);
    }
    const state = c.api.intern_atom(c.x, "_NET_WM_STATE", 0);
    const above = c.api.intern_atom(c.x, "_NET_WM_STATE_ABOVE", 0);
    const sticky = c.api.intern_atom(c.x, "_NET_WM_STATE_STICKY", 0);
    if (state != 0 and above != 0 and sticky != 0) {
        const root = c.api.default_root_window(c.x);
        var event: XEvent = .{
            .client = .{
                .type = 33, // ClientMessage
                .serial = 0,
                .send_event = 1,
                .display = c.x,
                .window = window,
                .message_type = state,
                .format = 32,
                // _NET_WM_STATE_ADD, two properties, source indication 1 (an
                // application).
                .data = .{ 1, @intCast(above), @intCast(sticky), 1, 0 },
            },
        };
        const SubstructureNotifyMask: c_long = 1 << 19;
        const SubstructureRedirectMask: c_long = 1 << 20;
        _ = c.api.send_event(c.x, root, 0, SubstructureNotifyMask | SubstructureRedirectMask, @ptrCast(&event));
    }
    _ = c.api.flush(c.x);
}

/// Puts the fixed title back when Chromium has renamed the window after the
/// page (a document window follows its page's title). Answers whether it had to.
pub fn holdPictureInPictureTitle(window: Window) bool {
    var buf: [128]u8 = undefined;
    if (std.mem.eql(u8, windowName(window, &buf), picture_in_picture_title)) return false;
    const c = conn() orelse return false;
    c.push();
    defer c.pop();
    setTitleLocked(c, window);
    _ = c.api.flush(c.x);
    return true;
}

fn setTitleLocked(c: Conn, window: Window) void {
    const XA_STRING: c_ulong = 31;
    const XA_WM_NAME: c_ulong = 39;
    const PROP_MODE_REPLACE: c_int = 0;
    const title = picture_in_picture_title;
    _ = c.api.change_property(c.x, window, XA_WM_NAME, XA_STRING, 8, PROP_MODE_REPLACE, title.ptr, @intCast(title.len));
    const net_name = c.api.intern_atom(c.x, "_NET_WM_NAME", 0);
    const utf8 = c.api.intern_atom(c.x, "UTF8_STRING", 0);
    if (net_name != 0 and utf8 != 0) _ = c.api.change_property(c.x, window, net_name, utf8, 8, PROP_MODE_REPLACE, title.ptr, @intCast(title.len));
}

/// Xlib's XClientMessageEvent inside the XEvent union (24 longs).
const XEvent = extern union {
    client: extern struct {
        type: c_int,
        serial: c_ulong,
        send_event: c_int,
        display: ?*Display,
        window: Window,
        message_type: c_ulong,
        format: c_int,
        data: [5]c_long,
    },
    pad: [24]c_long,
};

/// The window's title (`_NET_WM_NAME`), copied into `out`; empty when it has
/// none.
pub fn windowName(window: Window, out: []u8) []u8 {
    const c = conn() orelse return out[0..0];
    const prop = c.api.intern_atom(c.x, "_NET_WM_NAME", 1);
    const utf8 = c.api.intern_atom(c.x, "UTF8_STRING", 1);
    if (prop == 0 or utf8 == 0) return out[0..0];
    var actual_type: c_ulong = 0;
    var actual_format: c_int = 0;
    var nitems: c_ulong = 0;
    var bytes_after: c_ulong = 0;
    var data: [*]u8 = undefined;
    c.push();
    const ok = c.api.get_window_property(c.x, window, prop, 0, @intCast(out.len / 4), 0, utf8, &actual_type, &actual_format, &nitems, &bytes_after, &data);
    c.pop();
    if (ok != 0 or actual_format != 8) return out[0..0];
    defer _ = c.api.free(@ptrCast(data));
    const n = @min(out.len, @as(usize, nitems));
    @memcpy(out[0..n], data[0..n]);
    return out[0..n];
}

/// Asks the window manager to activate `window` (`_NET_ACTIVE_WINDOW`, as a
/// pager would, which is the source a WM honours from another client) and
/// sets X input focus on it for a session with no WM. Views takes keys only
/// in the widget it believes is active, which XSetInputFocus alone does not
/// make it under a WM.
pub fn activate(window: Window) void {
    if (window == 0) return;
    const c = conn() orelse return;
    c.push();
    defer c.pop();
    const active = c.api.intern_atom(c.x, "_NET_ACTIVE_WINDOW", 0);
    if (active != 0) {
        const root = c.api.default_root_window(c.x);
        var event: XEvent = .{
            .client = .{
                .type = 33, // ClientMessage
                .serial = 0,
                .send_event = 1,
                .display = c.x,
                .window = window,
                .message_type = active,
                .format = 32,
                // Source indication 2 (a pager), no timestamp, no requestor.
                .data = .{ 2, 0, 0, 0, 0 },
            },
        };
        const SubstructureNotifyMask: c_long = 1 << 19;
        const SubstructureRedirectMask: c_long = 1 << 20;
        _ = c.api.send_event(c.x, root, 0, SubstructureNotifyMask | SubstructureRedirectMask, @ptrCast(&event));
    }
    _ = c.api.set_input_focus(c.x, window, REVERT_TO_PARENT, CURRENT_TIME);
    _ = c.api.flush(c.x);
}

const FnFakeKey = *const fn (*Display, c_uint, c_int, c_ulong) callconv(.c) c_int;
const FnKeysymToKeycode = *const fn (*Display, c_ulong) callconv(.c) u8;
var xtest_attempted = false;
var fake_key: ?FnFakeKey = null;
var keysym_to_keycode: ?FnKeysymToKeycode = null;

pub const keysym_tab: c_ulong = 0xff09;
pub const keysym_space: c_ulong = 0x20;
pub const keysym_shift: c_ulong = 0xffe1;

fn loadXTest() bool {
    if (!xtest_attempted) {
        xtest_attempted = true;
        if (loadApi() == null) return false;
        var ext = std.DynLib.open("libXtst.so.6") catch std.DynLib.open("libXtst.so") catch {
            std.debug.print("ND_WARN CEF: libXtst not found; Chrome's install prompt is left for the user to answer\n", .{});
            return false;
        };
        fake_key = ext.lookup(FnFakeKey, "XTestFakeKeyEvent");
        keysym_to_keycode = xlib.lookup(FnKeysymToKeycode, "XKeysymToKeycode");
    }
    return fake_key != null and keysym_to_keycode != null;
}

/// Presses and releases `keysym` through XTest, which the X server delivers
/// like a key on the keyboard to whatever has focus. A key sent with
/// XSendEvent is marked synthetic, and Chromium drops it.
pub fn pressKey(keysym: c_ulong) bool {
    if (!loadXTest()) return false;
    const c = conn() orelse return false;
    c.push();
    defer c.pop();
    const code = keysym_to_keycode.?(c.x, keysym);
    if (code == 0) return false;
    _ = fake_key.?(c.x, code, 1, CURRENT_TIME);
    _ = fake_key.?(c.x, code, 0, CURRENT_TIME);
    _ = c.api.flush(c.x);
    return true;
}

/// Whether `window` is mapped and every ancestor is too.
pub fn viewable(window: Window) bool {
    if (window == 0) return false;
    const c = conn() orelse return false;
    c.push();
    defer c.pop();
    var attrs = std.mem.zeroes(WindowAttributes);
    if (c.api.get_window_attributes(c.x, window, &attrs) == 0) return false;
    const IS_VIEWABLE: c_int = 2;
    return attrs.map_state == IS_VIEWABLE;
}
