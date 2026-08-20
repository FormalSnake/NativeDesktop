// The Chromium engine behind <webview> on GTK: Alloy-style, windowed, embedded
// into an X11 child window of the host's own toplevel. No CEF Views window, no
// CEF-created toplevel, ever.
//
// Threading. `multi_threaded_message_loop = 1`, so CEF owns a UI thread of its
// own with its own GMainContext and the GTK4 default loop is untouched. Every
// handler arm below therefore runs on the CEF UI thread, and nothing in it may
// touch GTK: events are boxed and handed to the GTK loop with g_idle_add, which
// is the one glib call that is safe from any thread. The other direction
// (navigate, back, reload) goes straight through, since CefBrowser and
// CefBrowserHost are documented callable on any browser-process thread.
//
// Window ordering. The browser is created only from the widget's `map`
// handler, once the toplevel has a realized GdkSurface with an XID behind it.
// A CEF browser created with a null or unrealized parent silently becomes its
// own top-level Chromium window, which is the failure this whole design exists
// to prevent, so the parent is asserted first and creation is skipped (loudly)
// rather than attempted.
const std = @import("std");
const gtk = @import("gtk");
const gdk = @import("gdk");
const glib = @import("glib");
const gobject = @import("gobject");
const graphene = @import("graphene");
const protocol = @import("../protocol.zig");
const capi = @import("capi.zig");
const c = capi.c;
const ref = @import("ref.zig");
const loader = @import("loader.zig");
const x11 = @import("x11.zig");
const types = @import("types.zig");

const alloc = std.heap.c_allocator;

const MARKER_KEY = "nd-webview-cef";
const VIEW_KEY = "nd-webview-cef-view";

/// net::ERR_ABORTED. A navigation replaced by another one reports this, and
/// surfacing it would fire loadFailed on every ordinary redirect.
const ERR_ABORTED: c_int = -3;

var emit: ?types.EmitFn = null;

var trace_on: ?bool = null;

/// ND_WEBVIEW_TRACE=1, the same switch the WebKit backend narrates itself with.
/// Embedding failures here are all geometry and window identity, and neither is
/// observable from the outside once the process has aborted.
fn tr(comptime fmt: []const u8, args: anytype) void {
    const on = trace_on orelse blk: {
        const on = std.c.getenv("ND_WEBVIEW_TRACE") != null;
        trace_on = on;
        break :blk on;
    };
    if (!on) return;
    std.debug.print("ND_CEF " ++ fmt ++ "\n", args);
}

// ============================================================================
// Engine selection and process roles
// ============================================================================

var requested: ?bool = null;

/// Whether this PROCESS is set up for CEF, which is a different question from
/// whether a given view asked for it. cef_execute_process and the X11 backend
/// pin both have to happen in main, before any widget exists, so the only
/// input they can have is the environment: `nd dev` and `nd package` deliver
/// the resolved `webview.engine` from nativedesktop.config.ts as
/// ND_WEBVIEW_ENGINE for exactly this reason.
fn engineRequested() bool {
    if (requested) |r| return r;
    const raw = std.c.getenv("ND_WEBVIEW_ENGINE");
    const r = raw != null and std.mem.eql(u8, std.mem.span(raw.?), "chromium");
    requested = r;
    return r;
}

/// Set once main has run cef_execute_process for this process. A view that
/// asks for `engine="chromium"` in an app whose config did not is refused
/// here: without the early call, CEF's own renderer subprocesses would re-exec
/// this binary and run the whole host in each of them.
var process_ready = false;

/// argv as the process received it, kept for cef_initialize (which is called
/// lazily at the first <webview>, long after main's frame is gone).
var main_argv: []const [*:0]const u8 = &.{};

/// The first thing main does. On Linux CEF re-execs this same binary with
/// `--type=renderer` and friends, so every process starts here; a non-negative
/// return means "this process was a CEF subprocess, it has finished, exit now".
pub fn earlyExecuteProcess(argv: []const [*:0]const u8) ?u8 {
    main_argv = argv;
    if (!engineRequested()) return null;
    const api = loader.load() orelse return null;
    const app = ensureApp() orelse return null;
    var args = mainArgs();
    const rc = api.execute_process(&args, app.handOut(), null);
    if (rc < 0) {
        process_ready = true;
        return null;
    }
    return @truncate(@as(u32, @bitCast(rc)));
}

fn mainArgs() c.cef_main_args_t {
    return .{
        .argc = @intCast(main_argv.len),
        .argv = @constCast(@ptrCast(main_argv.ptr)),
    };
}

/// Windowed embedding is compiled X11-only in CEF, so a Wayland session has to
/// run the whole app through XWayland for the XID parenting to exist. Only when
/// the Chromium engine is actually asked for: pinning the backend otherwise
/// would cost every other app its native Wayland surface.
pub fn pinDisplayBackend() void {
    if (!engineRequested()) return;
    // Before GTK opens the display, and before Chromium's threads reach Xlib.
    x11.initThreads();
    gdk.setAllowedBackends("x11");
}

pub fn shutdown() void {
    if (!initialized) return;
    const api = loader.loaded() orelse return;
    initialized = false;
    api.shutdown();
}

// ============================================================================
// cef_app_t
// ============================================================================

const AppObj = ref.Counted(c.cef_app_t, void);
var app_obj: ?*AppObj = null;

fn ensureApp() ?*AppObj {
    if (app_obj) |a| return a;
    const a = AppObj.create({}) orelse return null;
    a.cef.on_before_command_line_processing = &onBeforeCommandLine;
    app_obj = a;
    return a;
}

fn onBeforeCommandLine(
    _: [*c]c.cef_app_t,
    _: [*c]const c.cef_string_t,
    command_line: [*c]c.cef_command_line_t,
) callconv(.c) void {
    defer ref.releaseParam(command_line);
    if (command_line == null) return;
    if (command_line.*.append_switch_with_value) |append| {
        // Ozone would otherwise pick Wayland under a Wayland session and
        // ignore parent_window entirely; the GDK backend is pinned to x11 to
        // match.
        appendSwitch(command_line, append, "ozone-platform", "x11");
        // Chromium dlopens the already-loaded GTK with RTLD_NOLOAD, so it has
        // to be told which major version the host brought in.
        appendSwitch(command_line, append, "gtk-version", "4");
    }
    if (command_line.*.append_switch) |append| {
        // Alloy style is a browser-level choice; the process still boots
        // Chrome's runtime, which on a fresh cache path puts up a modal
        // "Additional Terms of Service" window and blocks cef_initialize
        // behind it. That window is a CEF-created top-level, which this
        // engine may not have at all.
        appendFlag(command_line, append, "no-first-run");
        appendFlag(command_line, append, "no-default-browser-check");
    }
}

fn appendSwitch(
    cl: [*c]c.cef_command_line_t,
    append: *const fn ([*c]c.cef_command_line_t, [*c]const c.cef_string_t, [*c]const c.cef_string_t) callconv(.c) void,
    name: []const u8,
    value: []const u8,
) void {
    var n = std.mem.zeroes(c.cef_string_t);
    var v = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&n);
    defer clearStr(&v);
    if (!setStr(&n, name) or !setStr(&v, value)) return;
    append(cl, &n, &v);
}

fn appendFlag(
    cl: [*c]c.cef_command_line_t,
    append: *const fn ([*c]c.cef_command_line_t, [*c]const c.cef_string_t) callconv(.c) void,
    name: []const u8,
) void {
    var n = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&n);
    if (!setStr(&n, name)) return;
    append(cl, &n);
}

// ============================================================================
// cef_initialize
// ============================================================================

var initialized = false;
var init_failed = false;

fn ensureInitialized() bool {
    if (initialized) return true;
    if (init_failed) return false;
    init_failed = true;

    const api = loader.load() orelse return false;
    const app = ensureApp() orelse return false;

    var settings = std.mem.zeroes(c.cef_settings_t);
    settings.size = @sizeOf(c.cef_settings_t);
    // CEF gets its own thread and its own non-default GMainContext (M86+), so
    // the host's GTK4 loop never has to pump it. external_message_pump is not
    // an option here: it breaks text input (CEF #2002, #3782).
    settings.multi_threaded_message_loop = 1;
    settings.log_severity = @intCast(c.LOGSEVERITY_WARNING);

    var resources: ?[:0]u8 = null;
    var locales: ?[:0]u8 = null;
    defer if (resources) |p| alloc.free(p);
    defer if (locales) |p| alloc.free(p);
    if (loader.resourcesDir()) |p| {
        resources = p;
        _ = setStr(&settings.resources_dir_path, p);
    }
    if (loader.localesDir()) |p| {
        locales = p;
        _ = setStr(&settings.locales_dir_path, p);
    }
    var cache_root: ?[:0]u8 = null;
    defer if (cache_root) |p| alloc.free(p);
    if (defaultCacheRoot()) |p| {
        cache_root = p;
        _ = setStr(&settings.root_cache_path, p);
    }
    defer clearStr(&settings.resources_dir_path);
    defer clearStr(&settings.locales_dir_path);
    defer clearStr(&settings.root_cache_path);

    var args = mainArgs();
    if (api.initialize(&args, &settings, app.handOut(), null) == 0) {
        std.debug.print("ND_WARN CEF: cef_initialize failed; falling back to the system engine\n", .{});
        return false;
    }
    initialized = true;
    init_failed = false;
    return true;
}

/// Per-profile cache directories hang off this in M2; M1 only needs CEF to
/// have somewhere of its own that is not the app's data root.
fn defaultCacheRoot() ?[:0]u8 {
    const base = glib.getUserDataDir();
    const path = std.fmt.allocPrintSentinel(alloc, "{s}/nd-webview-cef", .{std.mem.span(base)}, 0) catch return null;
    _ = glib.mkdirWithParents(path.ptr, 0o700);
    return path;
}

// ============================================================================
// Strings
// ============================================================================

fn setStr(out: *c.cef_string_t, s: []const u8) bool {
    const api = loader.loaded() orelse return false;
    if (s.len == 0) return false;
    return api.string_utf8_to_utf16(s.ptr, s.len, out) != 0;
}

fn clearStr(s: *c.cef_string_t) void {
    const api = loader.loaded() orelse return;
    api.string_utf16_clear(s);
}

/// Owned utf8 copy of a CEF string parameter. CEF strings are utf16 with a
/// length, never null-terminated, and the parameter itself is only valid for
/// the duration of the callback.
fn dupeStr(s: [*c]const c.cef_string_t) ?[]u8 {
    if (s == null) return null;
    if (s.*.str == null or s.*.length == 0) return null;
    const units: []const u16 = @as([*]const u16, @ptrCast(s.*.str))[0..s.*.length];
    return std.unicode.utf16LeToUtf8Alloc(alloc, units) catch null;
}

// ============================================================================
// Per-view state
// ============================================================================

const ClientObj = ref.Counted(c.cef_client_t, *View);
const DisplayObj = ref.Counted(c.cef_display_handler_t, *View);
const LoadObj = ref.Counted(c.cef_load_handler_t, *View);
const LifeObj = ref.Counted(c.cef_life_span_handler_t, *View);

const View = struct {
    widget: *gtk.Widget,
    node_id: u32 = 0,

    client: *ClientObj,
    display_handler: *DisplayObj,
    load_handler: *LoadObj,
    life_handler: *LifeObj,

    /// Written on the CEF UI thread from on_after_created / on_before_close,
    /// read on the GTK thread by every command. A pointer-width atomic rather
    /// than a lock because the callbacks have no Io to lock against, and the
    /// reference held from creation to close is what keeps the object alive
    /// across the read.
    browser: std.atomic.Value(usize) = .init(0),
    /// The window CEF made inside our container, for tracking the allocation.
    cef_window: std.atomic.Value(usize) = .init(0),

    // GTK thread only from here down.
    container: x11.Window = 0,
    created: bool = false,
    /// The "no X11 parent" warning is once per view: `createBrowser` is
    /// retried every frame until it succeeds, and the frame clock would turn
    /// one diagnosis into a scrolling wall.
    warned_no_parent: bool = false,
    tick_id: c_uint = 0,
    bounds: Bounds = .{},
    pending_url: ?[:0]u8 = null,

    // Last values the handlers pushed, answering `webviewInfo`.
    url: ?[]u8 = null,
    title: ?[]u8 = null,
    loading: bool = false,
    can_go_back: bool = false,
    can_go_forward: bool = false,
};

const Bounds = struct {
    x: c_int = -1,
    y: c_int = -1,
    w: c_uint = 0,
    h: c_uint = 0,
};

/// Views the GTK tree still holds, keyed by pointer. An event boxed on the CEF
/// UI thread can outlive the tab it came from, and this is what the idle
/// callback checks before dereferencing.
var live_views: std.AutoHashMapUnmanaged(usize, void) = .empty;

fn viewOf(widget: *gtk.Widget) ?*View {
    const raw = gobject.Object.getData(widget.as(gobject.Object), VIEW_KEY) orelse return null;
    return @ptrCast(@alignCast(raw));
}

pub fn isReal(widget: *gtk.Widget) bool {
    return gobject.Object.getData(widget.as(gobject.Object), MARKER_KEY) != null;
}

/// Unwinds a half-built view when one of its handler allocations fails. Each
/// handler starts at one reference, owned here, so releasing it is what frees
/// it; no CEF object has seen any of them yet.
fn abandon(view: *View, client: ?*ClientObj, display_handler: ?*DisplayObj, load_handler: ?*LoadObj) ?*gtk.Widget {
    if (load_handler) |h| h.drop();
    if (display_handler) |h| h.drop();
    if (client) |h| h.drop();
    alloc.destroy(view);
    return null;
}

fn browserOf(view: *View) ?*c.cef_browser_t {
    const raw = view.browser.load(.acquire);
    if (raw == 0) return null;
    return @ptrFromInt(raw);
}

// ============================================================================
// Creation
// ============================================================================

pub fn create(url: ?[*:0]const u8, profile: []const u8, context_menu_mode: []const u8) ?*gtk.Widget {
    if (!process_ready) {
        std.debug.print("ND_WARN WebView engine=\"chromium\": this process did not start under CEF (set webview.engine in nativedesktop.config.ts, or ND_WEBVIEW_ENGINE=chromium)\n", .{});
        return null;
    }
    if (!ensureInitialized()) return null;
    if (profile.len != 0) {
        std.debug.print("ND_WARN WebView engine=chromium: `profile` is not wired yet (M2); this view uses the global request context\n", .{});
    }
    if (std.mem.eql(u8, context_menu_mode, "suppress")) {
        std.debug.print("ND_WARN WebView engine=chromium: `contextMenuMode` is not wired yet (M2); Chromium's own menu is shown\n", .{});
    }

    // A plain drawable widget: it never paints anything itself, it reserves
    // the rectangle the X11 child window is tracked against.
    const area = gtk.DrawingArea.new();
    const widget = area.as(gtk.Widget);

    const view = alloc.create(View) catch return null;
    const client = ClientObj.create(view) orelse return abandon(view, null, null, null);
    const display_handler = DisplayObj.create(view) orelse return abandon(view, client, null, null);
    const load_handler = LoadObj.create(view) orelse return abandon(view, client, display_handler, null);
    const life_handler = LifeObj.create(view) orelse return abandon(view, client, display_handler, load_handler);

    view.* = .{
        .widget = widget,
        .client = client,
        .display_handler = display_handler,
        .load_handler = load_handler,
        .life_handler = life_handler,
    };
    if (url) |u| {
        if (u[0] != 0) view.pending_url = alloc.dupeZ(u8, std.mem.span(u)) catch null;
    }

    client.cef.get_display_handler = &clientGetDisplayHandler;
    client.cef.get_load_handler = &clientGetLoadHandler;
    client.cef.get_life_span_handler = &clientGetLifeSpanHandler;

    display_handler.cef.on_address_change = &onAddressChange;
    display_handler.cef.on_title_change = &onTitleChange;
    display_handler.cef.on_loading_progress_change = &onLoadingProgressChange;

    load_handler.cef.on_loading_state_change = &onLoadingStateChange;
    load_handler.cef.on_load_error = &onLoadError;

    life_handler.cef.on_before_popup = &onBeforePopup;
    life_handler.cef.on_before_dev_tools_popup = &onBeforeDevToolsPopup;
    life_handler.cef.on_after_created = &onAfterCreated;
    life_handler.cef.do_close = &onDoClose;
    life_handler.cef.on_before_close = &onBeforeClose;

    live_views.put(alloc, @intFromPtr(view), {}) catch {};
    gobject.Object.setData(widget.as(gobject.Object), MARKER_KEY, @ptrFromInt(1));
    gobject.Object.setData(widget.as(gobject.Object), VIEW_KEY, view);
    gtk.Widget.setHexpand(widget, 1);
    gtk.Widget.setVexpand(widget, 1);

    _ = gobject.signalConnectData(widget.as(gobject.Object), "map", @ptrCast(&onMap), view, null, .{});
    _ = gobject.signalConnectData(widget.as(gobject.Object), "unmap", @ptrCast(&onUnmap), view, null, .{});
    _ = gobject.signalConnectData(widget.as(gobject.Object), "destroy", @ptrCast(&onDestroy), view, null, .{});
    return widget;
}

fn onMap(_: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const view: *View = @ptrCast(@alignCast(data.?));
    if (view.tick_id == 0) {
        view.tick_id = gtk.Widget.addTickCallback(view.widget, &onTick, view, null);
    }
    syncBounds(view);
    maybeCreateBrowser(view);
    x11.show(view.container);
}

/// The browser is created from the first frame at which the widget has a real
/// allocation, not from `map`: GTK4 maps before it has necessarily allocated,
/// and a browser created into a 1x1 window comes up with a 1x1 compositor
/// surface that the software presenter then fails to read back.
fn maybeCreateBrowser(view: *View) void {
    if (view.created) return;
    if (view.bounds.w <= 1 or view.bounds.h <= 1) return;
    createBrowser(view);
}

fn onUnmap(_: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const view: *View = @ptrCast(@alignCast(data.?));
    if (view.tick_id != 0) {
        gtk.Widget.removeTickCallback(view.widget, view.tick_id);
        view.tick_id = 0;
    }
    x11.hide(view.container);
}

fn onDestroy(_: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const view: *View = @ptrCast(@alignCast(data.?));
    _ = live_views.remove(@intFromPtr(view));
    if (view.tick_id != 0) {
        gtk.Widget.removeTickCallback(view.widget, view.tick_id);
        view.tick_id = 0;
    }
    if (browserOf(view)) |browser| {
        if (browser.get_host) |get_host| {
            const host = get_host(browser);
            if (host != null) {
                defer ref.releaseOwned(host);
                if (host.*.close_browser) |close| close(host, 1);
            }
        }
    }
    x11.destroy(view.container);
    view.container = 0;
    // The View itself outlives the widget on purpose: CEF still holds the
    // handlers that point at it, and on_before_close is still to come.
}

/// GTK4 gives no widget a window of its own, so this is where one comes from:
/// an X11 child of the toplevel, positioned over the widget's allocation, and
/// the thing CEF is parented into.
fn createBrowser(view: *View) void {
    if (view.created) return;
    const api = loader.loaded() orelse return;

    const parent = x11.toplevelXid(view.widget);
    if (parent == 0) {
        if (!view.warned_no_parent) {
            view.warned_no_parent = true;
            std.debug.print("ND_WARN WebView engine=chromium: the toplevel has no X11 window (Wayland without the x11 backend pin?); browser not created\n", .{});
        }
        return;
    }
    syncBounds(view);
    const container = x11.createChild(parent, view.bounds.x, view.bounds.y, view.bounds.w, view.bounds.h);
    if (container == 0) {
        std.debug.print("ND_WARN WebView engine=chromium: could not create the embedding window; browser not created\n", .{});
        return;
    }
    view.container = container;
    x11.show(container);
    tr("embed node={d} parent=0x{x} container=0x{x} bounds={d}x{d}+{d}+{d}", .{
        view.node_id, parent, container, view.bounds.w, view.bounds.h, view.bounds.x, view.bounds.y,
    });

    var window_info = std.mem.zeroes(c.cef_window_info_t);
    window_info.size = @sizeOf(c.cef_window_info_t);
    window_info.parent_window = container;
    window_info.bounds = .{
        .x = 0,
        .y = 0,
        .width = @intCast(@max(view.bounds.w, 1)),
        .height = @intCast(@max(view.bounds.h, 1)),
    };
    // Explicit rather than inferred: Chrome style would bring Chrome's own
    // window and toolbar with it, which is the thing this engine must not do.
    window_info.runtime_style = @intCast(c.CEF_RUNTIME_STYLE_ALLOY);

    var browser_settings = std.mem.zeroes(c.cef_browser_settings_t);
    browser_settings.size = @sizeOf(c.cef_browser_settings_t);

    var url = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&url);
    const start: []const u8 = if (view.pending_url) |p| p else "about:blank";
    _ = setStr(&url, start);

    view.created = true;
    // The client reference is consumed by CEF (CToCpp::Wrap takes the caller's
    // ref), so this hands out an added one and keeps ours.
    if (api.create_browser(&window_info, view.client.handOut(), &url, &browser_settings, null, null) == 0) {
        view.created = false;
        std.debug.print("ND_WARN WebView engine=chromium: cef_browser_host_create_browser failed\n", .{});
    }
}

// ============================================================================
// Geometry
// ============================================================================

fn onTick(_: *gtk.Widget, _: *gdk.FrameClock, data: ?*anyopaque) callconv(.c) c_int {
    const view: *View = @ptrCast(@alignCast(data.?));
    syncBounds(view);
    maybeCreateBrowser(view);
    return 1; // G_SOURCE_CONTINUE
}

/// GTK4 has no size-allocate signal and no per-widget window, so the
/// allocation is re-read once a frame and only acted on when it moved. The
/// early-out is a compute_bounds call; the alternative is subclassing
/// GtkWidget to override size_allocate, which is where this goes in M2 if the
/// woken frame clock ever shows up in a profile.
fn syncBounds(view: *View) void {
    const native = gtk.Widget.getNative(view.widget) orelse return;
    const native_widget: *gtk.Widget = @ptrCast(@alignCast(native));
    var rect: graphene.Rect = undefined;
    if (gtk.Widget.computeBounds(view.widget, native_widget, &rect) == 0) return;

    // CSD shadows mean the toplevel widget's origin is not the surface's.
    var tx: f64 = 0;
    var ty: f64 = 0;
    gtk.Native.getSurfaceTransform(native, &tx, &ty);
    // X11 coordinates are device pixels; GTK's are logical.
    const scale: f64 = @floatFromInt(gtk.Widget.getScaleFactor(view.widget));

    const next: Bounds = .{
        .x = @intFromFloat(@round((@as(f64, rect.f_origin.f_x) + tx) * scale)),
        .y = @intFromFloat(@round((@as(f64, rect.f_origin.f_y) + ty) * scale)),
        .w = @intFromFloat(@max(@round(@as(f64, rect.f_size.f_width) * scale), 1)),
        .h = @intFromFloat(@max(@round(@as(f64, rect.f_size.f_height) * scale), 1)),
    };
    if (std.meta.eql(next, view.bounds)) return;
    view.bounds = next;
    if (view.container == 0) return;
    x11.moveResize(view.container, next.x, next.y, next.w, next.h);
    resizeCefWindow(view, next.w, next.h);
}

/// CEF's Linux platform delegate sizes its window once, from
/// window_info.bounds, and does not follow the parent afterwards.
fn resizeCefWindow(view: *View, w: c_uint, h: c_uint) void {
    const window = view.cef_window.load(.acquire);
    if (window == 0) return;
    const api = loader.loaded() orelse return;
    const dpy = api.get_xdisplay() orelse return;
    x11.resizeOn(@ptrCast(dpy), @intCast(window), w, h);
}

// ============================================================================
// Props and commands
// ============================================================================

pub fn setUrl(widget: *gtk.Widget, url: [:0]const u8) void {
    const view = viewOf(widget) orelse return;
    if (url.len == 0) return;
    // Same echo guard the WebKit backend carries: onNavigate feeds the URL back
    // into app state, which re-applies the prop.
    if (view.url) |cur| {
        if (std.mem.eql(u8, cur, url)) return;
    }
    if (browserOf(view)) |browser| {
        loadUrl(browser, url);
        return;
    }
    if (view.pending_url) |p| alloc.free(p);
    view.pending_url = alloc.dupeZ(u8, url) catch null;
}

fn loadUrl(browser: *c.cef_browser_t, url: []const u8) void {
    const get_frame = browser.get_main_frame orelse return;
    const frame = get_frame(browser);
    if (frame == null) return;
    defer ref.releaseOwned(frame);
    const load = frame.*.load_url orelse return;
    var s = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&s);
    if (!setStr(&s, url)) return;
    load(frame, &s);
}

pub fn command(widget: *gtk.Widget, cmd: []const u8, _: ?std.json.Value) void {
    const view = viewOf(widget) orelse return;
    const browser = browserOf(view) orelse return;
    if (std.mem.eql(u8, cmd, "goBack")) {
        if (browser.can_go_back) |can| {
            if (can(browser) != 0) {
                if (browser.go_back) |go| go(browser);
            }
        }
    } else if (std.mem.eql(u8, cmd, "goForward")) {
        if (browser.can_go_forward) |can| {
            if (can(browser) != 0) {
                if (browser.go_forward) |go| go(browser);
            }
        }
    } else if (std.mem.eql(u8, cmd, "reload")) {
        if (browser.reload) |f| f(browser);
    } else if (std.mem.eql(u8, cmd, "stop")) {
        if (browser.stop_load) |f| f(browser);
    } else {
        std.debug.print("ND_WARN WebView engine=chromium: command {s} is not wired yet (M2)\n", .{cmd});
    }
}

pub fn info(widget: *gtk.Widget) ?types.Info {
    const view = viewOf(widget) orelse return null;
    return .{
        .url = view.url,
        .title = view.title,
        .loading = view.loading,
        .can_go_back = view.can_go_back,
        .can_go_forward = view.can_go_forward,
    };
}

pub fn connectEvents(widget: *gtk.Widget, node_id: u32, emit_fn: types.EmitFn) void {
    const view = viewOf(widget) orelse return;
    emit = emit_fn;
    view.node_id = node_id;
}

// ============================================================================
// cef_client_t
// ============================================================================

fn clientGetDisplayHandler(self: [*c]c.cef_client_t) callconv(.c) [*c]c.cef_display_handler_t {
    return ClientObj.of(self).payload.display_handler.handOut();
}

fn clientGetLoadHandler(self: [*c]c.cef_client_t) callconv(.c) [*c]c.cef_load_handler_t {
    return ClientObj.of(self).payload.load_handler.handOut();
}

fn clientGetLifeSpanHandler(self: [*c]c.cef_client_t) callconv(.c) [*c]c.cef_life_span_handler_t {
    return ClientObj.of(self).payload.life_handler.handOut();
}

// ============================================================================
// cef_display_handler_t
// ============================================================================

fn onAddressChange(
    self: [*c]c.cef_display_handler_t,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    url: [*c]const c.cef_string_t,
) callconv(.c) void {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(frame);
    // Subframe navigations are not the view's address.
    if (frame != null) {
        if (frame.*.is_main) |is_main| {
            if (is_main(frame) == 0) return;
        }
    }
    post(.{ .view = DisplayObj.of(self).payload, .name = "navigate", .text = dupeStr(url) });
}

fn onTitleChange(
    self: [*c]c.cef_display_handler_t,
    browser: [*c]c.cef_browser_t,
    title: [*c]const c.cef_string_t,
) callconv(.c) void {
    defer ref.releaseParam(browser);
    post(.{ .view = DisplayObj.of(self).payload, .name = "titleChanged", .text = dupeStr(title) });
}

fn onLoadingProgressChange(
    self: [*c]c.cef_display_handler_t,
    browser: [*c]c.cef_browser_t,
    progress: f64,
) callconv(.c) void {
    defer ref.releaseParam(browser);
    post(.{ .view = DisplayObj.of(self).payload, .name = "loadProgress", .number = progress });
}

// ============================================================================
// cef_load_handler_t
// ============================================================================

fn onLoadingStateChange(
    self: [*c]c.cef_load_handler_t,
    browser: [*c]c.cef_browser_t,
    is_loading: c_int,
    can_go_back: c_int,
    can_go_forward: c_int,
) callconv(.c) void {
    defer ref.releaseParam(browser);
    const view = LoadObj.of(self).payload;
    post(.{ .view = view, .name = "loadingChanged", .flag = is_loading != 0 });
    post(.{ .view = view, .name = "backAvailable", .flag = can_go_back != 0 });
    post(.{ .view = view, .name = "forwardAvailable", .flag = can_go_forward != 0 });
}

fn onLoadError(
    self: [*c]c.cef_load_handler_t,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    error_code: c.cef_errorcode_t,
    error_text: [*c]const c.cef_string_t,
    failed_url: [*c]const c.cef_string_t,
) callconv(.c) void {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(frame);
    if (error_code == ERR_ABORTED) return;
    post(.{
        .view = LoadObj.of(self).payload,
        .name = "loadFailed",
        .text = dupeStr(failed_url),
        .extra = dupeStr(error_text),
    });
}

// ============================================================================
// cef_life_span_handler_t
// ============================================================================

/// The no-stray-window invariant. Returning 1 cancels the popup outright; the
/// app opens a tab off the emitted URL, exactly as it does on WebKitGTK's
/// `create` signal.
fn onBeforePopup(
    self: [*c]c.cef_life_span_handler_t,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    _: c_int,
    target_url: [*c]const c.cef_string_t,
    _: [*c]const c.cef_string_t,
    _: c.cef_window_open_disposition_t,
    _: c_int,
    _: [*c]const c.cef_popup_features_t,
    _: [*c]c.cef_window_info_t,
    _: [*c][*c]c.cef_client_t,
    _: [*c]c.cef_browser_settings_t,
    _: [*c][*c]c.cef_dictionary_value_t,
    _: [*c]c_int,
) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(frame);
    post(.{ .view = LifeObj.of(self).payload, .name = "newWindow", .text = dupeStr(target_url) });
    return 1;
}

/// Devtools would otherwise open CEF's own top-level window. M1 has no
/// devtools surface at all, so the only correct answer is to refuse the
/// default window (CEF #3165 makes parenting it into a GTK window a crash).
fn onBeforeDevToolsPopup(
    _: [*c]c.cef_life_span_handler_t,
    browser: [*c]c.cef_browser_t,
    _: [*c]c.cef_window_info_t,
    _: [*c][*c]c.cef_client_t,
    _: [*c]c.cef_browser_settings_t,
    _: [*c][*c]c.cef_dictionary_value_t,
    use_default_window: [*c]c_int,
) callconv(.c) void {
    defer ref.releaseParam(browser);
    if (use_default_window != null) use_default_window.* = 0;
}

/// The reference this parameter arrives with is deliberately kept: it is the
/// view's handle on its browser until on_before_close gives it back.
fn onAfterCreated(self: [*c]c.cef_life_span_handler_t, browser: [*c]c.cef_browser_t) callconv(.c) void {
    if (browser == null) return;
    const view = LifeObj.of(self).payload;
    view.browser.store(@intFromPtr(browser), .release);
    if (browser.*.get_host) |get_host| {
        const host = get_host(browser);
        if (host != null) {
            defer ref.releaseOwned(host);
            if (host.*.get_window_handle) |get_handle| {
                view.cef_window.store(@intCast(get_handle(host)), .release);
            }
        }
    }
    tr("created node={d} cefWindow=0x{x}", .{ view.node_id, view.cef_window.load(.acquire) });
    post(.{ .view = view, .name = "", .settle = true });
}

fn onDoClose(_: [*c]c.cef_life_span_handler_t, browser: [*c]c.cef_browser_t) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    return 0;
}

fn onBeforeClose(self: [*c]c.cef_life_span_handler_t, browser: [*c]c.cef_browser_t) callconv(.c) void {
    const view = LifeObj.of(self).payload;
    view.cef_window.store(0, .release);
    const held = view.browser.swap(0, .acq_rel);
    if (held != 0) ref.releaseParam(@as([*c]c.cef_browser_t, @ptrFromInt(held)));
    ref.releaseParam(browser);
}

// ============================================================================
// Event marshaling: CEF UI thread to GTK main thread
// ============================================================================

/// One event, boxed on the CEF UI thread and unboxed on the GTK one. `settle`
/// carries no event of its own: it is the "the browser exists now" hop that
/// applies the pending URL and re-reads the allocation.
const Emission = struct {
    view: *View,
    name: []const u8,
    text: ?[]u8 = null,
    extra: ?[]u8 = null,
    flag: bool = false,
    number: f64 = 0,
    settle: bool = false,
};

fn post(e: Emission) void {
    const box = alloc.create(Emission) catch {
        if (e.text) |t| alloc.free(t);
        if (e.extra) |t| alloc.free(t);
        return;
    };
    box.* = e;
    // g_idle_add is the one glib entry point safe to call from a foreign
    // thread; everything downstream of `deliver` runs on the GTK loop.
    _ = glib.idleAdd(&deliver, box);
}

fn deliver(data: ?*anyopaque) callconv(.c) c_int {
    const box: *Emission = @ptrCast(@alignCast(data.?));
    defer {
        if (box.text) |t| alloc.free(t);
        if (box.extra) |t| alloc.free(t);
        alloc.destroy(box);
    }
    // The tab this came from can have been closed while the event was in
    // flight; the widget, and with it the container window, is already gone.
    if (!live_views.contains(@intFromPtr(box.view))) return 0;
    const view = box.view;

    if (box.settle) {
        syncBounds(view);
        resizeCefWindow(view, view.bounds.w, view.bounds.h);
        if (view.pending_url) |p| {
            if (browserOf(view)) |browser| loadUrl(browser, p);
            alloc.free(p);
            view.pending_url = null;
        }
        return 0;
    }

    const f = emit orelse return 0;
    if (std.mem.eql(u8, box.name, "navigate")) {
        const text = box.text orelse return 0;
        remember(&view.url, text);
        f(view.node_id, "navigate", .{ .text = text });
    } else if (std.mem.eql(u8, box.name, "titleChanged")) {
        const text = box.text orelse return 0;
        remember(&view.title, text);
        f(view.node_id, "titleChanged", .{ .text = text });
    } else if (std.mem.eql(u8, box.name, "newWindow")) {
        const text = box.text orelse return 0;
        f(view.node_id, "newWindow", .{ .text = text });
    } else if (std.mem.eql(u8, box.name, "loadProgress")) {
        f(view.node_id, "loadProgress", .{ .value = box.number });
    } else if (std.mem.eql(u8, box.name, "loadingChanged")) {
        view.loading = box.flag;
        f(view.node_id, "loadingChanged", .{ .checked = box.flag });
    } else if (std.mem.eql(u8, box.name, "backAvailable")) {
        view.can_go_back = box.flag;
        f(view.node_id, "backAvailable", .{ .checked = box.flag });
    } else if (std.mem.eql(u8, box.name, "forwardAvailable")) {
        view.can_go_forward = box.flag;
        f(view.node_id, "forwardAvailable", .{ .checked = box.flag });
    } else if (std.mem.eql(u8, box.name, "loadFailed")) {
        var payload: std.json.ObjectMap = .empty;
        defer payload.deinit(alloc);
        const failed_url: []const u8 = if (box.text) |t| t else "";
        const message: []const u8 = if (box.extra) |t| t else "";
        payload.put(alloc, "url", .{ .string = failed_url }) catch return 0;
        payload.put(alloc, "error", .{ .string = message }) catch return 0;
        f(view.node_id, "loadFailed", .{ .data = .{ .object = payload } });
    }
    return 0; // G_SOURCE_REMOVE
}

fn remember(slot: *?[]u8, value: []const u8) void {
    const copy = alloc.dupe(u8, value) catch return;
    if (slot.*) |old| alloc.free(old);
    slot.* = copy;
}
