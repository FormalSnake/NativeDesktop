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
const cdp = @import("cdp.zig");
const ctxmenu = @import("../gtk/context_menu.zig");
const automation_dialogs = @import("../automation_dialogs.zig");
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
const BrowserProcessObj = ref.Counted(c.cef_browser_process_handler_t, void);
var app_obj: ?*AppObj = null;
var browser_process_obj: ?*BrowserProcessObj = null;

fn ensureApp() ?*AppObj {
    if (app_obj) |a| return a;
    const a = AppObj.create({}) orelse return null;
    a.cef.on_before_command_line_processing = &onBeforeCommandLine;
    a.cef.on_register_custom_schemes = &onRegisterCustomSchemes;
    a.cef.get_browser_process_handler = &appGetBrowserProcessHandler;
    app_obj = a;
    return a;
}

/// A child process's command line is built through the browser process
/// handler, not through OnBeforeCommandLineProcessing: that one only ever sees
/// this process's own. Without this hook a renderer never learns which schemes
/// are standard, and every custom-scheme URL it is asked to parse is opaque.
fn appGetBrowserProcessHandler(_: [*c]c.cef_app_t) callconv(.c) [*c]c.cef_browser_process_handler_t {
    if (browser_process_obj == null) {
        const h = BrowserProcessObj.create({}) orelse return null;
        h.cef.on_before_child_process_launch = &onBeforeChildProcessLaunch;
        browser_process_obj = h;
    }
    return browser_process_obj.?.handOut();
}

fn onBeforeChildProcessLaunch(
    _: [*c]c.cef_browser_process_handler_t,
    command_line: [*c]c.cef_command_line_t,
) callconv(.c) void {
    defer ref.releaseParam(command_line);
    if (command_line == null) return;
    appendSchemesSwitch(command_line);
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
        // Chromium's popup blocker drops a non-gesture window.open before
        // on_before_popup ever runs, which would silently swallow a navigation
        // the app is supposed to decide about. WebKitGTK's `create` signal
        // fires for every window.open, and the <webview> contract is one
        // `newWindow` event per attempt on both engines, so the decision
        // belongs to the app, not to the engine.
        appendFlag(command_line, append, "disable-popup-blocking");
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

    cdp.setSink(.{ .result = &cdpResultSink, .event = &cdpEventSink });

    var args = mainArgs();
    if (api.initialize(&args, &settings, app.handOut(), null) == 0) {
        std.debug.print("ND_WARN CEF: cef_initialize failed; falling back to the system engine\n", .{});
        return false;
    }
    initialized = true;
    init_failed = false;
    ensureSchemeFactories();
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
// JSON helpers
// ============================================================================

fn argObject(arg: ?std.json.Value) ?std.json.ObjectMap {
    return switch (arg orelse return null) {
        .object => |o| o,
        else => null,
    };
}

fn objStr(obj: std.json.ObjectMap, key: []const u8) ?[]const u8 {
    return switch (obj.get(key) orelse return null) {
        .string => |s| s,
        else => null,
    };
}

fn objBool(obj: std.json.ObjectMap, key: []const u8) ?bool {
    return switch (obj.get(key) orelse return null) {
        .bool => |b| b,
        else => null,
    };
}

fn objStrList(obj: std.json.ObjectMap, key: []const u8) ?std.json.Array {
    return switch (obj.get(key) orelse return null) {
        .array => |a| a,
        else => null,
    };
}

// ============================================================================
// Per-view state
// ============================================================================

const ClientObj = ref.Counted(c.cef_client_t, *View);
const DisplayObj = ref.Counted(c.cef_display_handler_t, *View);
const LoadObj = ref.Counted(c.cef_load_handler_t, *View);
const LifeObj = ref.Counted(c.cef_life_span_handler_t, *View);
const FindObj = ref.Counted(c.cef_find_handler_t, *View);
const DownloadObj = ref.Counted(c.cef_download_handler_t, *View);
const JsDialogHandlerObj = ref.Counted(c.cef_jsdialog_handler_t, *View);
const ContextMenuObj = ref.Counted(c.cef_context_menu_handler_t, *View);

const View = struct {
    widget: *gtk.Widget,
    node_id: u32 = 0,

    client: *ClientObj,
    display_handler: *DisplayObj,
    load_handler: *LoadObj,
    life_handler: *LifeObj,
    find_handler: *FindObj,
    download_handler: *DownloadObj,
    jsdialog_handler: *JsDialogHandlerObj,
    context_menu_handler: *ContextMenuObj,

    /// The request context this view's browser was created with, or null for
    /// the global one. Held so the view keeps the profile alive.
    context: ?*c.cef_request_context_t = null,

    /// The app's `setContextMenuItems` tree, the id-to-item map for the menu
    /// currently on screen, and the page URL a click reports. All four are
    /// written on the GTK thread and read on the CEF UI thread while a menu is
    /// being built, which is what `menu_lock` is for.
    /// Read on the CEF UI thread on every right-click, written on the GTK one
    /// by the `contextMenuMode` prop.
    suppress_menu: std.atomic.Value(bool) = .init(false),
    menu_lock: SpinLock = .{},
    menu_items: []ctxmenu.Item = &.{},
    menu_commands: std.AutoHashMapUnmanaged(c_int, MenuCommand) = .empty,
    next_menu_command: c_int = menu_command_first,
    menu_page_url_slot: ?[]u8 = null,

    /// The last `findStart` text, so findNext/findPrevious can re-issue it:
    /// CEF's find takes the search text on every call.
    last_find: ?[]u8 = null,

    /// Written on the CEF UI thread from on_after_created / on_before_close,
    /// read on the GTK thread by every command. A pointer-width atomic rather
    /// than a lock because the callbacks have no Io to lock against, and the
    /// reference held from creation to close is what keeps the object alive
    /// across the read.
    browser: std.atomic.Value(usize) = .init(0),
    /// The window CEF made inside our container, for tracking the allocation.
    cef_window: std.atomic.Value(usize) = .init(0),
    /// The view's own long-lived host reference, taken with the browser and
    /// released with it. Every CDP call goes through it, and re-deriving it per
    /// call would churn a reference on whichever thread happened to ask.
    host: std.atomic.Value(usize) = .init(0),

    // GTK thread only from here down.
    container: x11.Window = 0,
    created: bool = false,
    /// What the browser was actually created with. A `url` prop applied while
    /// the browser was still being created lands in `pending_url` alone, so
    /// adoption has to reconcile the two or the view sits on the old address
    /// forever. Mounting a view with `url=""` and arming it on the next commit
    /// is the normal shape for a tab, a background page or a popup.
    created_url: []u8 = no_bytes[0..0],
    /// Retries `maybeCreateBrowser` until the toplevel exists. GTK4 has no
    /// signal for "your window is here" that reaches a widget which is never
    /// mapped, and a view that is never shown (an extension background page, a
    /// background tab) still has to load.
    create_timer: c_uint = 0,
    create_attempts: u32 = 0,

    /// The "no X11 parent" warning is once per view: `createBrowser` is
    /// retried every frame until it succeeds, and the frame clock would turn
    /// one diagnosis into a scrolling wall.
    warned_no_parent: bool = false,
    /// The toplevel surface's `layout` handler, which is where the allocation
    /// is re-read. See `onSurfaceLayout`.
    layout_handler: c_ulong = 0,
    layout_surface: ?*gdk.Surface = null,
    bounds: Bounds = .{},
    pending_url: ?[:0]u8 = null,

    // The CDP substrate. GTK thread only: results and events are marshaled
    // before anything here is touched.
    session: cdp.Session = .{},
    domains_enabled: bool = false,
    /// Set when Page.enable has ANSWERED. Sending a devtools method with a
    /// params dictionary before the agent is attached takes the process down,
    /// and every command the app issues between create and on_after_created
    /// arrives before that, so calls are parked rather than sent or dropped.
    cdp_ready: bool = false,
    queued: std.ArrayList(Queued) = .empty,
    /// Per-script-id install counter. A script identifier comes back
    /// asynchronously, so an id that is re-added (or removed) while its own
    /// install is in flight would otherwise store the stale identifier and
    /// leak the live script.
    script_gens: std.StringHashMapUnmanaged(u64) = .empty,
    worlds: std.StringHashMapUnmanaged(WorldState) = .empty,
    scripts: std.StringHashMapUnmanaged(ScriptEntry) = .empty,
    channels: std.StringHashMapUnmanaged(Channel) = .empty,
    /// Work parked until the world it names has an execution context.
    deferred: std.ArrayList(Deferred) = .empty,

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

pub fn setContextMenuMode(widget: *gtk.Widget, mode: []const u8) void {
    const view = viewOf(widget) orelse return;
    view.suppress_menu.store(std.mem.eql(u8, mode, "suppress"), .release);
}

pub fn create(url: ?[*:0]const u8, profile: []const u8, context_menu_mode: []const u8) ?*gtk.Widget {
    if (!process_ready) {
        std.debug.print("ND_WARN WebView engine=\"chromium\": this process did not start under CEF (set webview.engine in nativedesktop.config.ts, or ND_WEBVIEW_ENGINE=chromium)\n", .{});
        return null;
    }
    if (!ensureInitialized()) return null;

    // A plain drawable widget: it never paints anything itself, it reserves
    // the rectangle the X11 child window is tracked against.
    const area = gtk.DrawingArea.new();
    const widget = area.as(gtk.Widget);

    const view = alloc.create(View) catch return null;
    const client = ClientObj.create(view) orelse return abandon(view, null, null, null);
    const display_handler = DisplayObj.create(view) orelse return abandon(view, client, null, null);
    const load_handler = LoadObj.create(view) orelse return abandon(view, client, display_handler, null);
    const life_handler = LifeObj.create(view) orelse return abandon(view, client, display_handler, load_handler);
    const find_handler = FindObj.create(view) orelse return abandon(view, client, display_handler, load_handler);
    const download_handler = DownloadObj.create(view) orelse return abandon(view, client, display_handler, load_handler);
    const jsdialog_handler = JsDialogHandlerObj.create(view) orelse return abandon(view, client, display_handler, load_handler);
    const context_menu_handler = ContextMenuObj.create(view) orelse return abandon(view, client, display_handler, load_handler);

    view.* = .{
        .widget = widget,
        .client = client,
        .display_handler = display_handler,
        .load_handler = load_handler,
        .life_handler = life_handler,
        .find_handler = find_handler,
        .download_handler = download_handler,
        .jsdialog_handler = jsdialog_handler,
        .context_menu_handler = context_menu_handler,
    };
    view.suppress_menu.store(std.mem.eql(u8, context_menu_mode, "suppress"), .release);
    view.context = requestContext(profile);
    if (url) |u| {
        if (u[0] != 0) view.pending_url = alloc.dupeZ(u8, std.mem.span(u)) catch null;
    }

    client.cef.get_display_handler = &clientGetDisplayHandler;
    client.cef.get_load_handler = &clientGetLoadHandler;
    client.cef.get_life_span_handler = &clientGetLifeSpanHandler;
    client.cef.get_find_handler = &clientGetFindHandler;
    client.cef.get_download_handler = &clientGetDownloadHandler;
    client.cef.get_jsdialog_handler = &clientGetJsDialogHandler;
    client.cef.get_context_menu_handler = &clientGetContextMenuHandler;

    display_handler.cef.on_address_change = &onAddressChange;
    display_handler.cef.on_title_change = &onTitleChange;
    display_handler.cef.on_loading_progress_change = &onLoadingProgressChange;
    display_handler.cef.on_favicon_urlchange = &onFaviconUrlChange;

    load_handler.cef.on_loading_state_change = &onLoadingStateChange;
    load_handler.cef.on_load_error = &onLoadError;

    life_handler.cef.on_before_popup = &onBeforePopup;
    life_handler.cef.on_before_dev_tools_popup = &onBeforeDevToolsPopup;
    life_handler.cef.on_after_created = &onAfterCreated;
    life_handler.cef.do_close = &onDoClose;
    life_handler.cef.on_before_close = &onBeforeClose;
    find_handler.cef.on_find_result = &onFindResult;
    download_handler.cef.on_before_download = &onBeforeDownload;
    download_handler.cef.can_download = &onCanDownload;
    jsdialog_handler.cef.on_jsdialog = &onJsDialog;
    jsdialog_handler.cef.on_before_unload_dialog = &onBeforeUnloadDialog;
    context_menu_handler.cef.on_before_context_menu = &onBeforeContextMenu;
    context_menu_handler.cef.run_context_menu = &onRunContextMenu;
    context_menu_handler.cef.on_context_menu_command = &onContextMenuCommand;

    live_views.put(alloc, @intFromPtr(view), {}) catch {};
    gobject.Object.setData(widget.as(gobject.Object), MARKER_KEY, @ptrFromInt(1));
    gobject.Object.setData(widget.as(gobject.Object), VIEW_KEY, view);
    gtk.Widget.setHexpand(widget, 1);
    gtk.Widget.setVexpand(widget, 1);
    // The `focus` command grabs GTK focus as well as CEF's, and a
    // GtkDrawingArea takes none by default.
    gtk.Widget.setCanFocus(widget, 1);
    gtk.Widget.setFocusable(widget, 1);

    _ = gobject.signalConnectData(widget.as(gobject.Object), "map", @ptrCast(&onMap), view, null, .{});
    _ = gobject.signalConnectData(widget.as(gobject.Object), "unmap", @ptrCast(&onUnmap), view, null, .{});
    _ = gobject.signalConnectData(widget.as(gobject.Object), "destroy", @ptrCast(&onDestroy), view, null, .{});
    armCreateTimer(view);
    return widget;
}

fn onMap(_: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const view: *View = @ptrCast(@alignCast(data.?));
    connectLayout(view);
    syncBounds(view);
    maybeCreateBrowser(view);
    x11.show(view.container);
}

/// GTK4 has no size-allocate signal and gives no widget a window of its own, so
/// the allocation is re-read from the toplevel surface's `layout` phase: GTK
/// emits it once per frame in which it has laid the toplevel out, which is
/// exactly when a child's bounds can have moved. A frame-clock tick callback
/// would see the same changes, but it also keeps the clock awake for as long as
/// a view is mapped, which is a running cost for a window that is not changing.
fn connectLayout(view: *View) void {
    if (view.layout_handler != 0) return;
    const native = gtk.Widget.getNative(view.widget) orelse return;
    const surface = gtk.Native.getSurface(native) orelse return;
    view.layout_surface = surface;
    view.layout_handler = gobject.signalConnectData(
        @ptrCast(@alignCast(surface)),
        "layout",
        @ptrCast(&onSurfaceLayout),
        view,
        null,
        .{},
    );
}

fn disconnectLayout(view: *View) void {
    if (view.layout_handler == 0) return;
    if (view.layout_surface) |surface| {
        gobject.signalHandlerDisconnect(@ptrCast(@alignCast(surface)), view.layout_handler);
    }
    view.layout_handler = 0;
    view.layout_surface = null;
}

fn onSurfaceLayout(_: *gobject.Object, _: c_int, _: c_int, data: ?*anyopaque) callconv(.c) void {
    const view: *View = @ptrCast(@alignCast(data.?));
    syncBounds(view);
    maybeCreateBrowser(view);
}

/// A mapped view waits for its first real allocation: GTK4 maps before it has
/// necessarily allocated, and a browser created into a 1x1 window comes up with
/// a 1x1 compositor surface the software presenter then fails to read back.
///
/// A view that is NOT mapped waits for nothing. It may never be mapped at all,
/// and it still has to load: an extension's background page and a background
/// tab opened by target=_blank are both webviews that are navigated while
/// hidden, and gating their browser on an allocation left them on the raw URL
/// forever. Their window is created minimal and unmapped, and `syncBounds`
/// gives it the real geometry if the view is ever shown.
fn maybeCreateBrowser(view: *View) void {
    if (view.created) return;
    const mapped = gtk.Widget.getMapped(view.widget) != 0;
    if (mapped and (view.bounds.w <= 1 or view.bounds.h <= 1)) return;
    createBrowser(view);
}

/// Retried rather than signalled: see `View.create_timer`. Stops on the first
/// success, and gives up loudly rather than ticking for the process's life.
const create_retry_ms: c_uint = 50;
const create_retry_limit: u32 = 400;

fn armCreateTimer(view: *View) void {
    if (view.create_timer != 0 or view.created) return;
    view.create_timer = glib.timeoutAdd(create_retry_ms, &onCreateTimer, view);
}

fn disarmCreateTimer(view: *View) void {
    if (view.create_timer == 0) return;
    _ = glib.Source.remove(view.create_timer);
    view.create_timer = 0;
}

fn onCreateTimer(data: ?*anyopaque) callconv(.c) c_int {
    const view: *View = @ptrCast(@alignCast(data.?));
    if (view.created) {
        view.create_timer = 0;
        return 0;
    }
    view.create_attempts += 1;
    syncBounds(view);
    maybeCreateBrowser(view);
    if (view.created) {
        view.create_timer = 0;
        return 0;
    }
    if (view.create_attempts >= create_retry_limit) {
        view.create_timer = 0;
        // Last resort for a view that is mapped but has been allocated
        // nothing for this long: a browser in a degenerate window that
        // `syncBounds` will resize is still better than a view that never
        // loads at all, which is the outcome this whole path exists to remove.
        std.debug.print("ND_WARN WebView engine=chromium: no allocation after {d}ms; creating the browser anyway\n", .{create_retry_limit * create_retry_ms});
        createBrowser(view);
        return 0;
    }
    return 1; // G_SOURCE_CONTINUE
}

fn onUnmap(_: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const view: *View = @ptrCast(@alignCast(data.?));
    disconnectLayout(view);
    x11.hide(view.container);
}

fn onDestroy(_: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const view: *View = @ptrCast(@alignCast(data.?));
    _ = live_views.remove(@intFromPtr(view));
    disarmCreateTimer(view);
    disconnectLayout(view);
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
    // Only a mapped view's window is mapped. A hidden view gets a real browser
    // in an unmapped container, which is what lets it load without ever being
    // shown, and `onMap` maps it if the app shows it later.
    if (gtk.Widget.getMapped(view.widget) != 0) x11.show(container);
    tr("embed node={d} parent=0x{x} container=0x{x} bounds={d}x{d}+{d}+{d} mapped={}", .{
        view.node_id, parent, container, view.bounds.w, view.bounds.h, view.bounds.x, view.bounds.y,
        gtk.Widget.getMapped(view.widget) != 0,
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
    // Remembered rather than cleared. The creation URL is a browser-initiated
    // navigation and must not be re-issued through CefFrame::LoadURL, which is
    // renderer-initiated and which Chromium refuses for a custom scheme; but a
    // `url` prop that lands between here and on_after_created goes into
    // `pending_url` alone, and adoption has to notice the two disagree.
    alloc.free(view.created_url);
    view.created_url = dupeOwned(start);

    view.created = true;
    disarmCreateTimer(view);
    // Both the client and the request context are consumed by CEF (CToCpp::Wrap
    // takes the caller's reference), so each hands out an added one and this
    // view keeps its own.
    const context: [*c]c.cef_request_context_t = if (view.context) |ctx| blk: {
        ref.addRefParam(ctx);
        break :blk ctx;
    } else null;
    if (api.create_browser(&window_info, view.client.handOut(), &url, &browser_settings, null, context) == 0) {
        view.created = false;
        std.debug.print("ND_WARN WebView engine=chromium: cef_browser_host_create_browser failed\n", .{});
    }
}

// ============================================================================
// Geometry
// ============================================================================

/// Re-reads the widget's allocation and moves the embedding window to match.
/// Cheap enough to run per layout pass: the early-out is one compute_bounds.
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
    tr("setUrl node={d} url={s}", .{ view.node_id, url });
    // Same echo guard the WebKit backend carries: onNavigate feeds the URL back
    // into app state, which re-applies the prop.
    if (view.url) |cur| {
        if (std.mem.eql(u8, cur, url)) return;
    }
    // Before the first navigate event there is no `url` yet, and the address
    // the browser was created with is the only answer to "where is this view".
    if (view.url == null and view.created and std.mem.eql(u8, view.created_url, url)) return;
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

pub fn command(widget: *gtk.Widget, cmd: []const u8, arg: ?std.json.Value) void {
    const view = viewOf(widget) orelse return;
    // Only the navigation commands need a live browser. Everything else is a
    // devtools call, and those are parked until the agent is up precisely
    // because an app configures a view (user scripts, message channels) in the
    // same commit that creates it, long before CEF has made the browser.
    if (std.mem.eql(u8, cmd, "executeJavaScript")) return cmdExecuteJavaScript(view, arg);
    if (std.mem.eql(u8, cmd, "addUserScript")) return cmdAddUserScript(view, arg);
    if (std.mem.eql(u8, cmd, "removeUserScript")) return cmdRemoveUserScript(view, arg);
    if (std.mem.eql(u8, cmd, "clearUserScripts")) return cmdClearUserScripts(view, arg);
    if (std.mem.eql(u8, cmd, "registerScriptMessage")) return cmdRegisterScriptMessage(view, arg);
    if (std.mem.eql(u8, cmd, "unregisterScriptMessage")) return cmdUnregisterScriptMessage(view, arg);
    if (std.mem.eql(u8, cmd, "getCookies")) return cmdGetCookies(view, arg);
    if (std.mem.eql(u8, cmd, "setCookie")) return cmdSetCookie(view, arg);
    if (std.mem.eql(u8, cmd, "deleteCookie")) return cmdDeleteCookie(view, arg);
    if (std.mem.eql(u8, cmd, "setUserAgent")) return cmdSetUserAgent(view, arg);
    if (std.mem.eql(u8, cmd, "respondScheme")) return cmdRespondScheme(arg);
    if (std.mem.eql(u8, cmd, "setContextMenuItems")) return cmdSetContextMenuItems(view, arg);

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
    } else if (std.mem.eql(u8, cmd, "findStart")) {
        cmdFindStart(view, arg);
    } else if (std.mem.eql(u8, cmd, "findNext")) {
        cmdFindStep(view, true);
    } else if (std.mem.eql(u8, cmd, "findPrevious")) {
        cmdFindStep(view, false);
    } else if (std.mem.eql(u8, cmd, "findStop")) {
        cmdFindStop(view);
    } else if (std.mem.eql(u8, cmd, "setMuted")) {
        cmdSetMuted(view, arg);
    } else if (std.mem.eql(u8, cmd, "setZoom")) {
        cmdSetZoom(view, arg);
    } else if (std.mem.eql(u8, cmd, "focus")) {
        cmdFocus(view);
    } else if (std.mem.eql(u8, cmd, "saveSession")) {
        cmdSaveSession(view, arg);
    } else if (std.mem.eql(u8, cmd, "restoreSession")) {
        std.debug.print("ND_WARN WebView engine=chromium: restoreSession has no CEF equivalent (no session serialization API)\n", .{});
    } else {
        std.debug.print("ND_WARN WebView engine=chromium: command {s} is not wired yet\n", .{cmd});
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

fn clientGetFindHandler(self: [*c]c.cef_client_t) callconv(.c) [*c]c.cef_find_handler_t {
    return ClientObj.of(self).payload.find_handler.handOut();
}

fn clientGetDownloadHandler(self: [*c]c.cef_client_t) callconv(.c) [*c]c.cef_download_handler_t {
    return ClientObj.of(self).payload.download_handler.handOut();
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
    const view = DisplayObj.of(self).payload;
    const next = dupeStr(url);
    tr("navigate node={d} url={?s}", .{ view.node_id, next });
    post(.{ .view = view, .name = "navigate", .text = next });
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
    if (browser.*.get_identifier) |get_id| {
        post(.{ .view = view, .name = "", .settle = false, .cdp_result = false, .browser_id = get_id(browser) });
    }
    if (browser.*.get_host) |get_host| {
        const host = get_host(browser);
        if (host != null) {
            // Kept, like the browser reference above: every devtools call needs
            // it, and on_before_close is what gives both back.
            view.host.store(@intFromPtr(host), .release);
            if (host.*.get_window_handle) |get_handle| {
                view.cef_window.store(@intCast(get_handle(host)), .release);
            }
            // Written here rather than on the GTK thread because the observer
            // has to exist before the first protocol message; the settle hop
            // below is the first GTK-side read of it.
            view.session = cdp.attach(host, @intFromPtr(view));
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
    cdp.detach(&view.session);
    const host = view.host.swap(0, .acq_rel);
    if (host != 0) ref.releaseParam(@as([*c]c.cef_browser_host_t, @ptrFromInt(host)));
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
    /// Devtools traffic rides the same hop: a protocol result arrives on the
    /// CEF UI thread, and everything that interprets it (the pending-call
    /// table, the world map, the script registry) lives on the GTK one.
    cdp_result: bool = false,
    cdp_event: bool = false,
    message_id: c_int = 0,
    ok: bool = false,
    /// A parked scheme request being handed from the IO thread to the GTK one.
    scheme_obj: ?*ResourceObj = null,
    /// Non-zero on the hop that records a new browser's identifier.
    browser_id: c_int = 0,
    /// Context-menu payloads, owned by the emission because the params they
    /// were read from are gone by the time the GTK loop runs.
    menu_hit: ?*MenuHit = null,
    menu_click: ?*MenuClick = null,
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
        if (box.menu_hit) |h| {
            h.deinit();
            alloc.destroy(h);
        }
        if (box.menu_click) |click| {
            click.deinit();
            alloc.destroy(click);
        }
        alloc.destroy(box);
    }
    // A scheme request is keyed by browser id, not by view pointer: the
    // factory runs on the IO thread with no view in hand.
    if (box.scheme_obj) |obj| {
        announceSchemeRequest(obj);
        return 0;
    }
    // The tab this came from can have been closed while the event was in
    // flight; the widget, and with it the container window, is already gone.
    if (!live_views.contains(@intFromPtr(box.view))) return 0;
    const view = box.view;

    if (box.cdp_result) {
        onCdpResult(view, box.message_id, box.ok, if (box.text) |t| t else "");
        return 0;
    }
    if (box.cdp_event) {
        const method = box.extra orelse return 0;
        onCdpEvent(view, method, if (box.text) |t| t else "");
        return 0;
    }

    if (box.browser_id != 0) {
        browsers_by_id.put(alloc, box.browser_id, view) catch {};
        return 0;
    }

    if (box.settle) {
        // Belt and braces with the agent-attached event: CEF attaches the
        // protocol agent when the first observer is registered, but whether
        // that transition is reported to the observer is version-dependent,
        // and Page.enable is idempotent.
        enableDomains(view);
        syncBounds(view);
        resizeCefWindow(view, view.bounds.w, view.bounds.h);
        tr("settle node={d} pending={?s} created={s}", .{ view.node_id, view.pending_url, view.created_url });
        // Adoption: the address the app last asked for wins over the one the
        // browser happened to be created with.
        if (view.pending_url) |p| {
            if (!std.mem.eql(u8, p, view.created_url)) {
                if (browserOf(view)) |browser| loadUrl(browser, p);
            }
            alloc.free(p);
            view.pending_url = null;
        }
        return 0;
    }

    const f = emit orelse return 0;
    if (std.mem.eql(u8, box.name, "navigate")) {
        const text = box.text orelse return 0;
        remember(&view.url, text);
        // The menu handlers read this from the CEF UI thread.
        view.menu_lock.lock();
        remember(&view.menu_page_url_slot, text);
        view.menu_lock.unlock();
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
    } else if (std.mem.eql(u8, box.name, "contextMenu")) {
        const hit = box.menu_hit orelse return 0;
        var payload: std.json.ObjectMap = .empty;
        defer payload.deinit(alloc);
        payload.put(alloc, "x", .{ .integer = hit.x }) catch return 0;
        payload.put(alloc, "y", .{ .integer = hit.y }) catch return 0;
        if (hit.link.len > 0) payload.put(alloc, "link", .{ .string = hit.link }) catch return 0;
        if (hit.image.len > 0) payload.put(alloc, "image", .{ .string = hit.image }) catch return 0;
        if (hit.selection.len > 0) payload.put(alloc, "selection", .{ .string = hit.selection }) catch return 0;
        payload.put(alloc, "editable", .{ .bool = hit.editable }) catch return 0;
        payload.put(alloc, "hasSelection", .{ .bool = hit.selection.len > 0 }) catch return 0;
        f(view.node_id, "contextMenu", .{ .data = .{ .object = payload } });
    } else if (std.mem.eql(u8, box.name, "contextMenuItemClicked")) {
        const click = box.menu_click orelse return 0;
        var payload: std.json.ObjectMap = .empty;
        defer payload.deinit(alloc);
        payload.put(alloc, "id", .{ .string = click.id }) catch return 0;
        payload.put(alloc, "pageUrl", .{ .string = click.page_url }) catch return 0;
        if (click.link.len > 0) payload.put(alloc, "linkUrl", .{ .string = click.link }) catch return 0;
        if (click.image.len > 0) payload.put(alloc, "imageUrl", .{ .string = click.image }) catch return 0;
        if (click.selection.len > 0) payload.put(alloc, "selectionText", .{ .string = click.selection }) catch return 0;
        payload.put(alloc, "editable", .{ .bool = click.editable }) catch return 0;
        if (click.checked) |v| payload.put(alloc, "checked", .{ .bool = v }) catch return 0;
        if (click.was_checked) |v| payload.put(alloc, "wasChecked", .{ .bool = v }) catch return 0;
        f(view.node_id, "contextMenuItemClicked", .{ .data = .{ .object = payload } });
    } else if (std.mem.eql(u8, box.name, "faviconChanged")) {
        const text = box.text orelse return 0;
        var payload: std.json.ObjectMap = .empty;
        defer payload.deinit(alloc);
        payload.put(alloc, "iconUrl", .{ .string = text }) catch return 0;
        f(view.node_id, "faviconChanged", .{ .data = .{ .object = payload } });
    } else if (std.mem.eql(u8, box.name, "findResult")) {
        const count: i64 = @intFromFloat(box.number);
        var payload: std.json.ObjectMap = .empty;
        defer payload.deinit(alloc);
        payload.put(alloc, "matchFound", .{ .bool = count > 0 }) catch return 0;
        payload.put(alloc, "matchCount", .{ .integer = count }) catch return 0;
        payload.put(alloc, "done", .{ .bool = box.flag }) catch return 0;
        f(view.node_id, "findResult", .{ .data = .{ .object = payload } });
    } else if (std.mem.eql(u8, box.name, "downloadRequested")) {
        var payload: std.json.ObjectMap = .empty;
        defer payload.deinit(alloc);
        payload.put(alloc, "url", .{ .string = if (box.text) |t| t else "" }) catch return 0;
        if (box.extra) |name| payload.put(alloc, "suggestedFilename", .{ .string = name }) catch return 0;
        f(view.node_id, "downloadRequested", .{ .data = .{ .object = payload } });
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

/// The devtools sink, both arms on the CEF UI thread. `tag` is the View
/// pointer cdp.attach was handed.
fn cdpResultSink(tag: usize, message_id: c_int, ok: bool, json: []const u8) void {
    const view: *View = @ptrFromInt(tag);
    post(.{
        .view = view,
        .name = "",
        .cdp_result = true,
        .message_id = message_id,
        .ok = ok,
        .text = alloc.dupe(u8, json) catch null,
    });
}

fn cdpEventSink(tag: usize, method: []const u8, json: []const u8) void {
    const view: *View = @ptrFromInt(tag);
    // Only the three events this engine acts on are worth a hop; the Page and
    // Runtime domains are chatty enough that forwarding everything would put a
    // GTK idle source behind every DOM mutation.
    if (!std.mem.eql(u8, method, "Runtime.executionContextCreated") and
        !std.mem.eql(u8, method, "Runtime.executionContextsCleared") and
        !std.mem.eql(u8, method, "Runtime.executionContextDestroyed") and
        !std.mem.eql(u8, method, "Runtime.bindingCalled") and
        !std.mem.eql(u8, method, "Network.responseReceived") and
        !std.mem.eql(u8, method, cdp.agent_attached) and
        !std.mem.eql(u8, method, cdp.agent_detached)) return;
    post(.{
        .view = view,
        .name = "",
        .cdp_event = true,
        .extra = alloc.dupe(u8, method) catch null,
        .text = alloc.dupe(u8, json) catch null,
    });
}

fn remember(slot: *?[]u8, value: []const u8) void {
    const copy = alloc.dupe(u8, value) catch return;
    if (slot.*) |old| alloc.free(old);
    slot.* = copy;
}

// ============================================================================
// The CDP substrate
// ============================================================================
//
// Everything the <webview> contract asks for beyond navigation is a DevTools
// call on Alloy-style CEF: there is no user-script API, no isolated-world API
// and no script-message channel to bind to. The shapes below are chosen so an
// app (and the extension broker above it) cannot tell which engine answered:
// `javaScriptResult` carries the same stringified value WebKitGTK's
// jsc_value_to_string produces, and `scriptMessage` carries the same
// {name, world, body} triple.

const WorldState = struct {
    /// The live Runtime.ExecutionContextId, or 0 while the world has no
    /// document. Isolated worlds die with their document and come back with a
    /// new id on the next load, which is why this is refreshed from
    /// Runtime.executionContextCreated rather than remembered once.
    context_id: i64 = 0,
    /// Whether the new-document stub that re-creates this world on every load
    /// has been registered.
    requested: bool = false,
};

const ScriptEntry = struct { identifier: []u8, world: []u8 };
const Channel = struct { world: []u8, script: ?[]u8 = null };

/// A call that cannot be issued until its world has an execution context.
const Deferred = union(enum) {
    eval: struct { sink: EvalSink, code: []u8, world: []u8 },
};

/// Where the string form of one evaluation goes.
const EvalSink = union(enum) {
    /// `executeJavaScript`: emits `javaScriptResult` with this correlation id.
    app: []u8,
    /// `webviewEval`: settles this entry in `pending_evals`.
    auto: u64,
    /// The `pageTextContains` cache.
    page_text: u32,
    /// Fire and forget: the result is not wanted, only the side effect.
    discard,
};

const Call = union(enum) {
    ignore,
    eval: EvalSink,
    /// The second hop for a non-primitive result: String(value) computed in the
    /// page rather than approximated from the RemoteObject's description.
    stringify: struct { sink: EvalSink, object_id: []u8 },
    add_user_script: struct { id: []u8, world: []u8, gen: u64 },
    add_channel_script: struct { name: []u8 },
    /// A getCookies correlation id.
    cookies: []u8,
    /// Page.enable's own reply: the gate every other call waits behind.
    agent_ready,
};

const Queued = struct { method: []u8, params: []u8, call: Call };

const PendingCall = struct { view: *View, call: Call };

/// GTK thread only: every CDP result is marshaled before it is looked up here.
var pending_calls: std.AutoHashMapUnmanaged(c_int, PendingCall) = .empty;

fn sinkFree(sink: EvalSink) void {
    switch (sink) {
        .app => |id| alloc.free(id),
        else => {},
    }
}

fn callFree(call: Call) void {
    switch (call) {
        .ignore => {},
        .eval => |sink| sinkFree(sink),
        .stringify => |s| {
            sinkFree(s.sink);
            alloc.free(s.object_id);
        },
        .add_user_script => |s| {
            alloc.free(s.id);
            alloc.free(s.world);
        },
        .add_channel_script => |s| alloc.free(s.name),
        .cookies => |id| alloc.free(id),
        .agent_ready => {},
    }
}

fn hostOf(view: *View) ?*c.cef_browser_host_t {
    const raw = view.host.load(.acquire);
    if (raw == 0) return null;
    return @ptrFromInt(raw);
}

/// Sends now, without waiting for the agent. Only Page.enable itself and the
/// drain below use this.
fn cdpSendRaw(view: *View, method: []const u8, params_json: []const u8, call: Call) bool {
    const host = hostOf(view) orelse {
        callFree(call);
        return false;
    };
    const id = cdp.send(host, method, params_json) orelse {
        callFree(call);
        return false;
    };
    tr("cdp -> node={d} id={d} {s} {s}", .{ view.node_id, id, method, params_json });
    if (std.meta.activeTag(call) == .ignore) return true;
    pending_calls.put(alloc, id, .{ .view = view, .call = call }) catch {
        callFree(call);
        return false;
    };
    return true;
}

/// Queues one CDP call behind the agent handshake and records what to do with
/// its answer. Everything above this line in the file goes through here.
fn cdpSend(view: *View, method: []const u8, params_json: []const u8, call: Call) bool {
    if (view.cdp_ready) return cdpSendRaw(view, method, params_json, call);
    const method_copy = alloc.dupe(u8, method) catch {
        callFree(call);
        return false;
    };
    const params_copy = alloc.dupe(u8, params_json) catch {
        alloc.free(method_copy);
        callFree(call);
        return false;
    };
    view.queued.append(alloc, .{ .method = method_copy, .params = params_copy, .call = call }) catch {
        alloc.free(method_copy);
        alloc.free(params_copy);
        callFree(call);
        return false;
    };
    return true;
}

/// Page and Runtime, once per browser. Runtime is what makes worlds tractable:
/// executionContextCreated names every isolated world as it is re-made on each
/// navigation, so no world id ever has to be re-derived by hand.
fn enableDomains(view: *View) void {
    if (view.domains_enabled) return;
    // Latched only on a send that happened. Setting the flag first meant one
    // failed send (no host yet) parked every queued call for the view's life,
    // with nothing left to retry it.
    if (!cdpSendRaw(view, "Page.enable", "", .agent_ready)) return;
    view.domains_enabled = true;
}

/// The agent has answered. Runtime.enable goes first because everything parked
/// behind it (bindings, world stubs, evaluations) is ordered after it on the
/// same channel.
fn agentReady(view: *View) void {
    if (view.cdp_ready) return;
    _ = cdpSendRaw(view, "Runtime.enable", "", .ignore);
    // Network carries both the cookie surface and, on the main document's
    // response, the TLS state securityChanged reports. The Security domain
    // would say the same thing in one event, but CEF's protocol subset does
    // not answer Security.enable at all.
    _ = cdpSendRaw(view, "Network.enable", "", .ignore);
    view.cdp_ready = true;
    const items = view.queued.toOwnedSlice(alloc) catch return;
    defer alloc.free(items);
    for (items) |q| {
        defer alloc.free(q.method);
        defer alloc.free(q.params);
        _ = cdpSendRaw(view, q.method, q.params, q.call);
    }
}

// ============================================================================
// Worlds
// ============================================================================

fn worldContextId(view: *View, world: []const u8) i64 {
    if (world.len == 0) return 0; // the page's own world needs no contextId
    const entry = view.worlds.get(world) orelse return 0;
    return entry.context_id;
}

/// Makes sure `world` exists now and after every future navigation. The
/// mechanism is a new-document script carrying the world name: CDP creates the
/// isolated world to run it in, on every load, which is exactly the lifetime an
/// isolated world needs. Page.createIsolatedWorld would answer with an id
/// sooner but only for the current document, and the id it returns is dead
/// after the next navigation.
fn ensureWorld(view: *View, world: []const u8) void {
    if (world.len == 0) return;
    const gop = view.worlds.getOrPut(alloc, world) catch return;
    if (!gop.found_existing) {
        gop.key_ptr.* = alloc.dupe(u8, world) catch {
            _ = view.worlds.remove(world);
            return;
        };
        gop.value_ptr.* = .{};
    }
    if (gop.value_ptr.requested) return;
    gop.value_ptr.requested = true;

    var params: std.ArrayList(u8) = .empty;
    defer params.deinit(alloc);
    params.appendSlice(alloc, "{\"source\":\"\",\"runImmediately\":true,\"worldName\":") catch return;
    cdp.quote(&params, world);
    params.appendSlice(alloc, "}") catch return;
    _ = cdpSend(view, "Page.addScriptToEvaluateOnNewDocument", params.items, .ignore);
}

fn setWorldContext(view: *View, world: []const u8, context_id: i64) void {
    const gop = view.worlds.getOrPut(alloc, world) catch return;
    if (!gop.found_existing) {
        gop.key_ptr.* = alloc.dupe(u8, world) catch {
            _ = view.worlds.remove(world);
            return;
        };
        gop.value_ptr.* = .{};
    }
    gop.value_ptr.context_id = context_id;
    tr("worldContext node={d} world={s} id={d}", .{ view.node_id, world, context_id });
    drainDeferred(view);
}

fn clearWorldContexts(view: *View) void {
    var it = view.worlds.iterator();
    while (it.next()) |e| e.value_ptr.context_id = 0;
}

fn drainDeferred(view: *View) void {
    if (view.deferred.items.len == 0) return;
    var still: std.ArrayList(Deferred) = .empty;
    // Take the list first: issuing a call can defer again, and appending to a
    // list being iterated is how that turns into a loop.
    const items = view.deferred.toOwnedSlice(alloc) catch return;
    defer alloc.free(items);
    for (items) |item| switch (item) {
        .eval => |e| {
            if (worldContextId(view, e.world) == 0) {
                still.append(alloc, item) catch {
                    sinkFree(e.sink);
                    alloc.free(e.code);
                    alloc.free(e.world);
                };
                continue;
            }
            _ = issueEval(view, e.sink, e.code, e.world);
            alloc.free(e.code);
            alloc.free(e.world);
        },
    };
    view.deferred = still;
}

// ============================================================================
// Evaluation
// ============================================================================

/// One evaluation, world-aware. A world with no execution context yet parks the
/// call rather than running it in the wrong one.
fn startEval(view: *View, sink: EvalSink, code: []const u8, world: []const u8) bool {
    if (world.len != 0) {
        ensureWorld(view, world);
        if (worldContextId(view, world) == 0) {
            const code_copy = alloc.dupe(u8, code) catch return false;
            const world_copy = alloc.dupe(u8, world) catch {
                alloc.free(code_copy);
                return false;
            };
            view.deferred.append(alloc, .{ .eval = .{
                .sink = sink,
                .code = code_copy,
                .world = world_copy,
            } }) catch {
                alloc.free(code_copy);
                alloc.free(world_copy);
                return false;
            };
            return true;
        }
    }
    return issueEval(view, sink, code, world);
}

fn issueEval(view: *View, sink: EvalSink, code: []const u8, world: []const u8) bool {
    var params: std.ArrayList(u8) = .empty;
    defer params.deinit(alloc);
    params.appendSlice(alloc, "{\"expression\":") catch return false;
    cdp.quote(&params, code);
    // returnByValue stays false so an object comes back as a handle this can
    // stringify in the page; awaitPromise stays false because WebKitGTK's
    // evaluate_javascript does not await either, and a Promise has to
    // stringify as "[object Promise]" on both engines.
    params.appendSlice(alloc, ",\"returnByValue\":false,\"awaitPromise\":false,\"objectGroup\":\"nd\"") catch return false;
    const context_id = worldContextId(view, world);
    if (context_id != 0) {
        var buf: [32]u8 = undefined;
        const n = std.fmt.bufPrint(&buf, ",\"contextId\":{d}", .{context_id}) catch return false;
        params.appendSlice(alloc, n) catch return false;
    }
    params.appendSlice(alloc, "}") catch return false;
    return cdpSend(view, "Runtime.evaluate", params.items, .{ .eval = sink });
}

/// The string form of a Runtime.RemoteObject, or null when only the page can
/// answer (an object, an array, a function: everything whose String() is not
/// derivable from the JSON scalar CDP sent).
fn remoteToString(result: std.json.Value) ?[]u8 {
    const obj = switch (result) {
        .object => |o| o,
        else => return null,
    };
    const kind = switch (obj.get("type") orelse return null) {
        .string => |s| s,
        else => return null,
    };
    if (std.mem.eql(u8, kind, "undefined")) return alloc.dupe(u8, "undefined") catch null;
    if (std.mem.eql(u8, kind, "object")) {
        if (obj.get("subtype")) |sub| {
            switch (sub) {
                .string => |s| if (std.mem.eql(u8, s, "null")) return alloc.dupe(u8, "null") catch null,
                else => {},
            }
        }
        return null;
    }
    const value = obj.get("value") orelse return null;
    return switch (value) {
        .string => |s| alloc.dupe(u8, s) catch null,
        .bool => |b| alloc.dupe(u8, if (b) "true" else "false") catch null,
        .integer => |i| std.fmt.allocPrint(alloc, "{d}", .{i}) catch null,
        .float => |f| std.fmt.allocPrint(alloc, "{d}", .{f}) catch null,
        .null => alloc.dupe(u8, "null") catch null,
        // A number CDP could not express as JSON (NaN, Infinity) arrives as
        // `unserializableValue`, which is already the string form.
        else => blk: {
            const raw = obj.get("unserializableValue") orelse break :blk null;
            break :blk switch (raw) {
                .string => |s| alloc.dupe(u8, s) catch null,
                else => null,
            };
        },
    };
}

fn objectIdOf(result: std.json.Value) ?[]const u8 {
    const obj = switch (result) {
        .object => |o| o,
        else => return null,
    };
    return switch (obj.get("objectId") orelse return null) {
        .string => |s| s,
        else => null,
    };
}

/// Hands the value back to the page to stringify. `String(this)` is exactly
/// what jsc_value_to_string does on the WebKit side, so "[object Object]" and
/// "1,2,3" come out the same on both engines.
fn stringifyRemote(view: *View, sink: EvalSink, object_id: []const u8) void {
    const owned = alloc.dupe(u8, object_id) catch {
        finishEval(view, sink, false, "out of memory");
        return;
    };
    var params: std.ArrayList(u8) = .empty;
    defer params.deinit(alloc);
    params.appendSlice(alloc, "{\"objectId\":") catch {
        alloc.free(owned);
        finishEval(view, sink, false, "out of memory");
        return;
    };
    cdp.quote(&params, object_id);
    params.appendSlice(alloc, ",\"functionDeclaration\":\"function(){return String(this)}\",\"returnByValue\":true}") catch {
        alloc.free(owned);
        finishEval(view, sink, false, "out of memory");
        return;
    };
    if (!cdpSend(view, "Runtime.callFunctionOn", params.items, .{ .stringify = .{ .sink = sink, .object_id = owned } })) {
        finishEval(view, sink, false, "Runtime.callFunctionOn could not be sent");
    }
}

fn releaseRemote(view: *View, object_id: []const u8) void {
    var params: std.ArrayList(u8) = .empty;
    defer params.deinit(alloc);
    params.appendSlice(alloc, "{\"objectId\":") catch return;
    cdp.quote(&params, object_id);
    params.appendSlice(alloc, "}") catch return;
    _ = cdpSend(view, "Runtime.releaseObject", params.items, .ignore);
}

/// `exceptionDetails.exception.description` is the message WebKit would have
/// put in its GError; the text after it is the fallback chain.
fn exceptionText(details: std.json.Value) []const u8 {
    const obj = switch (details) {
        .object => |o| o,
        else => return "unknown error",
    };
    if (obj.get("exception")) |ex| {
        if (ex == .object) {
            if (ex.object.get("description")) |d| {
                if (d == .string) return d.string;
            }
            if (ex.object.get("value")) |v| {
                if (v == .string) return v.string;
            }
        }
    }
    if (obj.get("text")) |t| {
        if (t == .string) return t.string;
    }
    return "unknown error";
}

fn finishEval(view: *View, sink: EvalSink, ok: bool, text: []const u8) void {
    switch (sink) {
        .app => |id| {
            defer alloc.free(id);
            const f = emit orelse return;
            var payload: std.json.ObjectMap = .empty;
            defer payload.deinit(alloc);
            payload.put(alloc, "id", .{ .string = id }) catch return;
            payload.put(alloc, "ok", .{ .bool = ok }) catch return;
            payload.put(alloc, if (ok) "value" else "error", .{ .string = text }) catch return;
            f(view.node_id, "javaScriptResult", .{ .data = .{ .object = payload } });
        },
        .auto => |eval_id| {
            const entry = pending_evals.get(eval_id) orelse return;
            entry.done = true;
            entry.ok = ok;
            const copy = alloc.dupe(u8, text) catch return;
            if (ok) {
                if (entry.value) |old| alloc.free(old);
                entry.value = copy;
            } else {
                if (entry.err) |old| alloc.free(old);
                entry.err = copy;
            }
        },
        .discard => {},
        .page_text => |node_id| {
            const entry = page_texts.getPtr(node_id) orelse return;
            entry.in_flight = false;
            entry.stamp_us = glib.getMonotonicTime();
            if (!ok) return;
            const copy = alloc.dupe(u8, text) catch return;
            if (entry.text) |old| alloc.free(old);
            entry.text = copy;
        },
    }
}

// ============================================================================
// Result and event routing (GTK thread)
// ============================================================================

fn onCdpResult(view: *View, message_id: c_int, ok: bool, json: []const u8) void {
    tr("cdp <- node={d} id={d} ok={} {s}", .{ view.node_id, message_id, ok, json });
    const entry = pending_calls.fetchRemove(message_id) orelse return;
    const call = entry.value.call;
    if (call == .agent_ready) {
        // Failure is still an answer: the agent either attached or never will,
        // and parking the queue forever is worse than one loud call.
        if (!ok) std.debug.print("ND_WARN CEF: Page.enable was refused; the devtools surface is unavailable on this view\n", .{});
        agentReady(view);
        return;
    }
    var parsed = std.json.parseFromSlice(std.json.Value, alloc, json, .{}) catch {
        failCall(view, call, "the engine returned no parsable result");
        return;
    };
    defer parsed.deinit();
    const root = parsed.value;

    if (!ok) {
        failCall(view, call, cdpErrorText(root));
        return;
    }

    switch (call) {
        .ignore, .agent_ready => {},
        .eval => |sink| {
            if (root == .object) {
                if (root.object.get("exceptionDetails")) |details| {
                    finishEval(view, sink, false, exceptionText(details));
                    return;
                }
                if (root.object.get("result")) |result| {
                    if (remoteToString(result)) |text| {
                        defer alloc.free(text);
                        finishEval(view, sink, true, text);
                        return;
                    }
                    if (objectIdOf(result)) |object_id| {
                        stringifyRemote(view, sink, object_id);
                        return;
                    }
                }
            }
            finishEval(view, sink, true, "undefined");
        },
        .stringify => |s| {
            defer alloc.free(s.object_id);
            releaseRemote(view, s.object_id);
            if (root == .object) {
                if (root.object.get("exceptionDetails")) |details| {
                    finishEval(view, s.sink, false, exceptionText(details));
                    return;
                }
                if (root.object.get("result")) |result| {
                    if (remoteToString(result)) |text| {
                        defer alloc.free(text);
                        finishEval(view, s.sink, true, text);
                        return;
                    }
                }
            }
            finishEval(view, s.sink, true, "");
        },
        .add_user_script => |s| {
            defer alloc.free(s.id);
            defer alloc.free(s.world);
            const identifier = stringField(root, "identifier") orelse return;
            // The install this identifier belongs to has since been replaced or
            // removed. Storing it would point the registry at a dead script and
            // leave the live one uninstallable, so it is taken straight back
            // out instead.
            if ((view.script_gens.get(s.id) orelse 0) != s.gen) {
                removeNewDocumentScript(view, identifier);
                return;
            }
            const id_copy = alloc.dupe(u8, identifier) catch return;
            const key = alloc.dupe(u8, s.id) catch {
                alloc.free(id_copy);
                return;
            };
            const world_copy = alloc.dupe(u8, s.world) catch {
                alloc.free(id_copy);
                alloc.free(key);
                return;
            };
            putScript(view, key, .{ .identifier = id_copy, .world = world_copy });
        },
        .cookies => |id| {
            defer alloc.free(id);
            emitCookies(view, id, root);
        },
        .add_channel_script => |s| {
            defer alloc.free(s.name);
            const identifier = stringField(root, "identifier") orelse return;
            const channel = view.channels.getPtr(s.name) orelse return;
            if (channel.script) |old| alloc.free(old);
            channel.script = alloc.dupe(u8, identifier) catch null;
        },
    }
}

fn putScript(view: *View, key: []u8, entry: ScriptEntry) void {
    if (view.scripts.fetchRemove(key)) |old| {
        alloc.free(old.key);
        alloc.free(old.value.identifier);
        alloc.free(old.value.world);
    }
    view.scripts.put(alloc, key, entry) catch {
        alloc.free(key);
        alloc.free(entry.identifier);
        alloc.free(entry.world);
    };
}

fn failCall(view: *View, call: Call, message: []const u8) void {
    switch (call) {
        .cookies => |id| {
            defer alloc.free(id);
            cookiesError(view, id, message);
        },
        .eval => |sink| finishEval(view, sink, false, message),
        .stringify => |s| {
            alloc.free(s.object_id);
            finishEval(view, s.sink, false, message);
        },
        else => callFree(call),
    }
}

fn cdpErrorText(root: std.json.Value) []const u8 {
    if (root == .object) {
        if (root.object.get("message")) |m| {
            if (m == .string) return m.string;
        }
    }
    return "the devtools method failed";
}

fn stringField(root: std.json.Value, key: []const u8) ?[]const u8 {
    if (root != .object) return null;
    return switch (root.object.get(key) orelse return null) {
        .string => |s| s,
        else => null,
    };
}

fn onCdpEvent(view: *View, method: []const u8, json: []const u8) void {
    // The agent transitions carry no parameters and gate everything else: no
    // devtools method may be sent before the agent has attached.
    if (std.mem.eql(u8, method, cdp.agent_attached)) {
        tr("cdp agent attached node={d}", .{view.node_id});
        enableDomains(view);
        return;
    }
    if (std.mem.eql(u8, method, cdp.agent_detached)) {
        tr("cdp agent detached node={d}", .{view.node_id});
        view.cdp_ready = false;
        view.domains_enabled = false;
        clearWorldContexts(view);
        return;
    }
    var parsed = std.json.parseFromSlice(std.json.Value, alloc, json, .{}) catch return;
    defer parsed.deinit();
    const root = parsed.value;

    if (std.mem.eql(u8, method, "Runtime.executionContextCreated")) {
        const context = switch (root) {
            .object => |o| o.get("context") orelse return,
            else => return,
        };
        if (context != .object) return;
        const id = switch (context.object.get("id") orelse return) {
            .integer => |i| i,
            .float => |f| @as(i64, @intFromFloat(f)),
            else => return,
        };
        const name = switch (context.object.get("name") orelse .null) {
            .string => |s| s,
            else => "",
        };
        if (name.len == 0) return; // the page's own world needs no id
        setWorldContext(view, name, id);
        return;
    }
    if (std.mem.eql(u8, method, "Runtime.executionContextsCleared")) {
        clearWorldContexts(view);
        return;
    }
    if (std.mem.eql(u8, method, "Runtime.executionContextDestroyed")) {
        // The per-context event, which is what an ordinary navigation or
        // reload actually sends; executionContextsCleared only arrives on the
        // transitions that reset the whole agent. Without this a world keeps
        // the id of the document it had BEFORE the reload, and the next
        // world-scoped evaluation is aimed at a context that no longer exists.
        const id = switch (root) {
            .object => |o| switch (o.get("executionContextId") orelse return) {
                .integer => |i| i,
                .float => |fv| @as(i64, @intFromFloat(fv)),
                else => return,
            },
            else => return,
        };
        var it = view.worlds.iterator();
        while (it.next()) |entry| {
            if (entry.value_ptr.context_id == id) {
                tr("worldContextGone node={d} world={s} id={d}", .{ view.node_id, entry.key_ptr.*, id });
                entry.value_ptr.context_id = 0;
            }
        }
        return;
    }
    if (std.mem.eql(u8, method, "Runtime.bindingCalled")) {
        onBindingCalled(view, root);
        return;
    }
    if (std.mem.eql(u8, method, "Network.responseReceived")) {
        // Only the main document's response describes the page's own TLS
        // state; a subresource's would report the last image loaded.
        const kind = stringField(root, "type") orelse return;
        if (!std.mem.eql(u8, kind, "Document")) return;
        const response = switch (root) {
            .object => |o| o.get("response") orelse return,
            else => return,
        };
        const state = stringField(response, "securityState") orelse return;
        // Not Chromium's notion of a trustworthy origin: http://127.0.0.1 is
        // "secure" to Chromium and is not TLS, and `secure` on this event has
        // always meant "came over TLS with no certificate errors" because that
        // is what WebKitGTK's get_tls_info answers.
        const url = stringField(response, "url") orelse "";
        const secure = std.mem.startsWith(u8, url, "https://") and
            !std.mem.eql(u8, state, "insecure") and
            !std.mem.eql(u8, state, "insecure-broken");
        const f = emit orelse return;
        var payload: std.json.ObjectMap = .empty;
        defer payload.deinit(alloc);
        payload.put(alloc, "secure", .{ .bool = secure }) catch return;
        // "insecure-broken" is mixed content or a failed certificate; the
        // WebKit backend spells the first of those as insecureContent.
        payload.put(alloc, "insecureContent", .{ .bool = std.mem.eql(u8, state, "insecure-broken") }) catch return;
        f(view.node_id, "securityChanged", .{ .data = .{ .object = payload } });
        return;
    }
}

// ============================================================================
// Script messages
// ============================================================================
//
// The page-side API is `window.webkit.messageHandlers.NAME.postMessage(v)` on
// both engines, because that is what app code and the extension broker are
// written against. On CEF it is a shim: a document-start script per channel
// that forwards through a Runtime binding, one binding per world so the world
// a message came from is known from the binding name rather than guessed from
// an execution context id.

const page_binding = "__ndScriptMessage";

fn bindingName(world: []const u8) ?[]u8 {
    if (world.len == 0) return alloc.dupe(u8, page_binding) catch null;
    var out: std.ArrayList(u8) = .empty;
    out.appendSlice(alloc, page_binding) catch return null;
    out.append(alloc, '_') catch return null;
    // A binding name is a JS identifier, and a world name is not constrained
    // to be one.
    for (world) |ch| {
        const safe: u8 = if (std.ascii.isAlphanumeric(ch) or ch == '_') ch else '_';
        out.append(alloc, safe) catch return null;
    }
    return out.toOwnedSlice(alloc) catch null;
}

fn worldForBinding(view: *View, binding: []const u8) ?[]const u8 {
    if (std.mem.eql(u8, binding, page_binding)) return "";
    var it = view.worlds.keyIterator();
    while (it.next()) |key| {
        const candidate = bindingName(key.*) orelse continue;
        defer alloc.free(candidate);
        if (std.mem.eql(u8, candidate, binding)) return key.*;
    }
    return null;
}

fn onBindingCalled(view: *View, root: std.json.Value) void {
    const binding = stringField(root, "name") orelse return;
    const payload_text = stringField(root, "payload") orelse return;
    const world = worldForBinding(view, binding) orelse return;

    var parsed = std.json.parseFromSlice(std.json.Value, alloc, payload_text, .{}) catch return;
    defer parsed.deinit();
    if (parsed.value != .object) return;
    const name = switch (parsed.value.object.get("name") orelse return) {
        .string => |s| s,
        else => return,
    };
    // A channel the app has since unregistered must not keep delivering.
    if (!view.channels.contains(name)) return;

    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    payload.put(alloc, "name", .{ .string = name }) catch return;
    payload.put(alloc, "world", .{ .string = world }) catch return;
    payload.put(alloc, "body", parsed.value.object.get("body") orelse .null) catch return;
    const f = emit orelse return;
    f(view.node_id, "scriptMessage", .{ .data = .{ .object = payload } });
}

fn cmdRegisterScriptMessage(view: *View, arg: ?std.json.Value) void {
    const obj = argObject(arg) orelse return;
    const name = objStr(obj, "name") orelse {
        std.debug.print("ND_WARN WebView registerScriptMessage: missing name\n", .{});
        return;
    };
    const world = objStr(obj, "world") orelse "";

    if (view.channels.get(name)) |existing| {
        if (!std.mem.eql(u8, existing.world, world)) {
            std.debug.print(
                "ND_WARN WebView registerScriptMessage: '{s}' is already registered on this view in world '{s}'; a handler name is per view, not per world, so give world '{s}' a name of its own\n",
                .{ name, existing.world, world },
            );
        }
        return;
    }
    ensureWorld(view, world);

    const binding = bindingName(world) orelse return;
    defer alloc.free(binding);

    // The binding is per world, and re-adding one is a protocol error rather
    // than a no-op, so it is added with the world's first channel only.
    if (!bindingLive(view, world)) {
        var params: std.ArrayList(u8) = .empty;
        defer params.deinit(alloc);
        params.appendSlice(alloc, "{\"name\":") catch return;
        cdp.quote(&params, binding);
        if (world.len != 0) {
            params.appendSlice(alloc, ",\"executionContextName\":") catch return;
            cdp.quote(&params, world);
        }
        params.appendSlice(alloc, "}") catch return;
        _ = cdpSend(view, "Runtime.addBinding", params.items, .ignore);
    }

    const key = alloc.dupe(u8, name) catch return;
    const world_copy = alloc.dupe(u8, world) catch {
        alloc.free(key);
        return;
    };
    view.channels.put(alloc, key, .{ .world = world_copy }) catch {
        alloc.free(key);
        alloc.free(world_copy);
        return;
    };
    tr("registerScriptMessage node={d} name={s} world={s}", .{ view.node_id, name, world });

    const source = shimSource(name, binding) orelse return;
    defer alloc.free(source);
    const call_name = alloc.dupe(u8, name) catch return;
    addNewDocumentScript(view, source, world, true, .{ .add_channel_script = .{ .name = call_name } });
}

fn bindingLive(view: *View, world: []const u8) bool {
    var it = view.channels.valueIterator();
    while (it.next()) |channel| {
        if (std.mem.eql(u8, channel.world, world)) return true;
    }
    return false;
}

fn shimSource(name: []const u8, binding: []const u8) ?[]u8 {
    var out: std.ArrayList(u8) = .empty;
    out.appendSlice(alloc,
        \\(function(){var w=window;w.webkit=w.webkit||{};var m=w.webkit.messageHandlers=w.webkit.messageHandlers||{};m[
    ) catch return null;
    cdp.quote(&out, name);
    out.appendSlice(alloc, "]={postMessage:function(v){w[") catch return null;
    cdp.quote(&out, binding);
    out.appendSlice(alloc, "](JSON.stringify({name:") catch return null;
    cdp.quote(&out, name);
    out.appendSlice(alloc, ",body:v===undefined?null:v}))}};})();") catch return null;
    return out.toOwnedSlice(alloc) catch null;
}

fn cmdUnregisterScriptMessage(view: *View, arg: ?std.json.Value) void {
    const obj = argObject(arg) orelse return;
    const name = objStr(obj, "name") orelse return;
    const entry = view.channels.fetchRemove(name) orelse return;
    defer alloc.free(entry.key);
    defer alloc.free(entry.value.world);
    if (entry.value.script) |identifier| {
        defer alloc.free(identifier);
        removeNewDocumentScript(view, identifier);
    }
    // Removing the new-document script only affects the NEXT document, and
    // WebKit's unregister takes the handler away from the live page too.
    var code: std.ArrayList(u8) = .empty;
    defer code.deinit(alloc);
    code.appendSlice(alloc, "(function(){try{delete window.webkit.messageHandlers[") catch return;
    cdp.quote(&code, name);
    code.appendSlice(alloc, "]}catch(e){}})()") catch return;
    _ = startEval(view, .discard, code.items, entry.value.world);
}

// ============================================================================
// User scripts
// ============================================================================

fn addNewDocumentScript(view: *View, source: []const u8, world: []const u8, run_now: bool, call: Call) void {
    var params: std.ArrayList(u8) = .empty;
    defer params.deinit(alloc);
    params.appendSlice(alloc, "{\"source\":") catch {
        callFree(call);
        return;
    };
    cdp.quote(&params, source);
    if (run_now) params.appendSlice(alloc, ",\"runImmediately\":true") catch {};
    if (world.len != 0) {
        params.appendSlice(alloc, ",\"worldName\":") catch {};
        cdp.quote(&params, world);
    }
    params.appendSlice(alloc, "}") catch {
        callFree(call);
        return;
    };
    _ = cdpSend(view, "Page.addScriptToEvaluateOnNewDocument", params.items, call);
}

fn removeNewDocumentScript(view: *View, identifier: []const u8) void {
    var params: std.ArrayList(u8) = .empty;
    defer params.deinit(alloc);
    params.appendSlice(alloc, "{\"identifier\":") catch return;
    cdp.quote(&params, identifier);
    params.appendSlice(alloc, "}") catch return;
    _ = cdpSend(view, "Page.removeScriptToEvaluateOnNewDocument", params.items, .ignore);
}

/// CDP injects new-document scripts at document-start and has no document-end
/// option, so an end script is wrapped in the wait for DOMContentLoaded. The
/// one visible difference from WebKitGTK is scope: the wrapped source runs
/// inside a function, so a bare `var` in it is no longer a global.
fn wrapForDocumentEnd(source: []const u8) ?[]u8 {
    var out: std.ArrayList(u8) = .empty;
    out.appendSlice(alloc, "(function(){var f=function(){") catch return null;
    out.appendSlice(alloc, source) catch return null;
    out.appendSlice(alloc, "\n};if(document.readyState===\"loading\"){document.addEventListener(\"DOMContentLoaded\",f)}else{f()}})();") catch return null;
    return out.toOwnedSlice(alloc) catch null;
}

fn cmdAddUserScript(view: *View, arg: ?std.json.Value) void {
    const obj = argObject(arg) orelse {
        std.debug.print("ND_WARN WebView addUserScript: malformed arg (expected an object)\n", .{});
        return;
    };
    const id = objStr(obj, "id") orelse {
        std.debug.print("ND_WARN WebView addUserScript: missing id\n", .{});
        return;
    };
    const source = objStr(obj, "source") orelse {
        std.debug.print("ND_WARN WebView addUserScript: missing source\n", .{});
        return;
    };
    const world = objStr(obj, "world") orelse "";
    const at_start = if (objStr(obj, "injectionTime")) |t| std.mem.eql(u8, t, "start") else false;
    if (objStrList(obj, "allowList") != null or objStrList(obj, "blockList") != null) {
        std.debug.print("ND_WARN WebView engine=chromium addUserScript: allowList/blockList have no DevTools equivalent and are ignored\n", .{});
    }
    ensureWorld(view, world);
    removeScriptById(view, id);
    const gen = bumpScriptGen(view, id) orelse return;

    const wrapped: ?[]u8 = if (at_start) null else wrapForDocumentEnd(source);
    defer if (wrapped) |w| alloc.free(w);
    const body: []const u8 = if (wrapped) |w| w else source;

    const id_copy = alloc.dupe(u8, id) catch return;
    const world_copy = alloc.dupe(u8, world) catch {
        alloc.free(id_copy);
        return;
    };
    tr("addUserScript node={d} id={s} world={s} at={s}", .{ view.node_id, id, world, if (at_start) "start" else "end" });
    addNewDocumentScript(view, body, world, false, .{ .add_user_script = .{ .id = id_copy, .world = world_copy, .gen = gen } });
}

/// Invalidates any install for `id` that is still in flight and returns the
/// generation the next one will carry.
fn bumpScriptGen(view: *View, id: []const u8) ?u64 {
    const gop = view.script_gens.getOrPut(alloc, id) catch return null;
    if (!gop.found_existing) {
        gop.key_ptr.* = alloc.dupe(u8, id) catch {
            _ = view.script_gens.remove(id);
            return null;
        };
        gop.value_ptr.* = 0;
    }
    gop.value_ptr.* += 1;
    return gop.value_ptr.*;
}

fn removeScriptById(view: *View, id: []const u8) void {
    // Also invalidates an install still in flight for this id: its identifier
    // arrives later and would otherwise re-register a script the app removed.
    _ = bumpScriptGen(view, id);
    const entry = view.scripts.fetchRemove(id) orelse return;
    defer alloc.free(entry.key);
    defer alloc.free(entry.value.identifier);
    defer alloc.free(entry.value.world);
    removeNewDocumentScript(view, entry.value.identifier);
}

fn cmdRemoveUserScript(view: *View, arg: ?std.json.Value) void {
    const obj = argObject(arg) orelse return;
    const id = objStr(obj, "id") orelse {
        std.debug.print("ND_WARN WebView removeUserScript: missing id\n", .{});
        return;
    };
    removeScriptById(view, id);
}

fn cmdClearUserScripts(view: *View, arg: ?std.json.Value) void {
    tr("clearUserScripts node={d}", .{view.node_id});
    const world: ?[]const u8 = if (argObject(arg)) |o| objStr(o, "world") else null;
    var ids: std.ArrayList([]const u8) = .empty;
    defer ids.deinit(alloc);
    var it = view.scripts.iterator();
    while (it.next()) |e| {
        if (world) |w| {
            if (!std.mem.eql(u8, e.value_ptr.world, w)) continue;
        }
        ids.append(alloc, e.key_ptr.*) catch {};
    }
    for (ids.items) |id| removeScriptById(view, id);
}

fn cmdExecuteJavaScript(view: *View, arg: ?std.json.Value) void {
    const obj = argObject(arg) orelse {
        std.debug.print("ND_WARN WebView executeJavaScript: malformed arg (expected {{id, code}})\n", .{});
        return;
    };
    const id = objStr(obj, "id") orelse {
        std.debug.print("ND_WARN WebView executeJavaScript: malformed arg (expected {{id, code}})\n", .{});
        return;
    };
    const code = objStr(obj, "code") orelse {
        std.debug.print("ND_WARN WebView executeJavaScript: malformed arg (expected {{id, code}})\n", .{});
        return;
    };
    const world = objStr(obj, "world") orelse "";
    const id_copy = alloc.dupe(u8, id) catch return;
    if (!startEval(view, .{ .app = id_copy }, code, world)) alloc.free(id_copy);
}

// ============================================================================
// Automation: webviewEval and the pageText cache
// ============================================================================

const PendingEval = struct {
    done: bool = false,
    ok: bool = false,
    value: ?[]u8 = null,
    err: ?[]u8 = null,
};

var pending_evals: std.AutoHashMapUnmanaged(u64, *PendingEval) = .empty;
var next_eval_id: u64 = 1;

/// Null only for a widget this engine did not create. A view whose browser is
/// still attaching does NOT fail: the call is parked with every other devtools
/// call and settles when the agent comes up, which the poller already reads as
/// "not done yet". Answering an error there turned a background page that had
/// simply not loaded yet into a hard -32602.
pub fn evalStart(widget: *gtk.Widget, code: []const u8, world: ?[]const u8) ?u64 {
    const view = viewOf(widget) orelse return null;
    const entry = alloc.create(PendingEval) catch return null;
    entry.* = .{};
    const id = next_eval_id;
    pending_evals.put(alloc, id, entry) catch {
        alloc.destroy(entry);
        return null;
    };
    next_eval_id += 1;
    if (!startEval(view, .{ .auto = id }, code, world orelse "")) {
        entry.done = true;
        entry.ok = false;
        entry.err = alloc.dupe(u8, "the devtools call could not be sent") catch null;
    }
    return id;
}

pub const EvalState = struct { done: bool, ok: bool, value: ?[]const u8, err: ?[]const u8 };

pub fn evalPoll(id: u64) ?EvalState {
    const entry = pending_evals.get(id) orelse return null;
    return .{ .done = entry.done, .ok = entry.ok, .value = entry.value, .err = entry.err };
}

pub fn evalRelease(id: u64) void {
    const entry = pending_evals.get(id) orelse return;
    if (!entry.done) return;
    _ = pending_evals.remove(id);
    if (entry.value) |v| alloc.free(v);
    if (entry.err) |e| alloc.free(e);
    alloc.destroy(entry);
}

const page_text_interval_us: i64 = 250_000;
const PageText = struct { text: ?[]u8 = null, stamp_us: i64 = 0, in_flight: bool = false };
var page_texts: std.AutoHashMapUnmanaged(u32, PageText) = .empty;

pub fn pageText(widget: *gtk.Widget) ?[]const u8 {
    const view = viewOf(widget) orelse return null;
    if (view.node_id == 0) return null;
    const gop = page_texts.getOrPut(alloc, view.node_id) catch return null;
    if (!gop.found_existing) gop.value_ptr.* = .{};
    const entry = gop.value_ptr;
    const now = glib.getMonotonicTime();
    if (!entry.in_flight and now - entry.stamp_us >= page_text_interval_us) {
        entry.in_flight = true;
        if (!startEval(view, .{ .page_text = view.node_id }, "document.body ? document.body.innerText : \"\"", "")) {
            entry.in_flight = false;
        }
    }
    return entry.text;
}

// ============================================================================
// Cookies (CDP Network domain)
// ============================================================================
//
// The Network domain rather than cef_cookie_manager_t: the manager's API is a
// visitor object plus a completion callback per call, all of it asynchronous
// across threads, and the protocol gives the same three operations against the
// view's own request context with no extra ref-counted objects to get wrong.

fn cookiesError(view: *View, id: []const u8, message: []const u8) void {
    const f = emit orelse return;
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    payload.put(alloc, "id", .{ .string = id }) catch return;
    payload.put(alloc, "ok", .{ .bool = false }) catch return;
    payload.put(alloc, "error", .{ .string = message }) catch return;
    f(view.node_id, "cookiesResult", .{ .data = .{ .object = payload } });
}

fn cmdGetCookies(view: *View, arg: ?std.json.Value) void {
    const obj = argObject(arg) orelse return;
    const id = objStr(obj, "id") orelse {
        std.debug.print("ND_WARN WebView getCookies: missing id\n", .{});
        return;
    };
    var params: std.ArrayList(u8) = .empty;
    defer params.deinit(alloc);
    if (objStr(obj, "url")) |url| {
        params.appendSlice(alloc, "{\"urls\":[") catch return;
        cdp.quote(&params, url);
        params.appendSlice(alloc, "]}") catch return;
    }
    const id_copy = alloc.dupe(u8, id) catch return;
    if (!cdpSend(view, "Network.getCookies", params.items, .{ .cookies = id_copy })) {
        cookiesError(view, id, "the devtools call could not be sent");
    }
}

/// CDP's cookie shape into the one both engines emit. `expires` is seconds
/// since the epoch as a double, with -1 for a session cookie, which is the
/// null the WebKit backend emits.
fn emitCookies(view: *View, id: []const u8, root: std.json.Value) void {
    const f = emit orelse return;
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    var cookies: std.json.Array = .init(alloc);
    defer {
        for (cookies.items) |item| switch (item) {
            .object => |o| {
                var m = o;
                m.deinit(alloc);
            },
            else => {},
        };
        cookies.deinit();
    }
    if (root == .object) {
        if (root.object.get("cookies")) |list| {
            if (list == .array) {
                for (list.array.items) |raw| {
                    if (raw != .object) continue;
                    const src = raw.object;
                    var out: std.json.ObjectMap = .empty;
                    out.put(alloc, "name", src.get("name") orelse .{ .string = "" }) catch {};
                    out.put(alloc, "value", src.get("value") orelse .{ .string = "" }) catch {};
                    out.put(alloc, "domain", src.get("domain") orelse .{ .string = "" }) catch {};
                    out.put(alloc, "path", src.get("path") orelse .{ .string = "" }) catch {};
                    out.put(alloc, "secure", src.get("secure") orelse .{ .bool = false }) catch {};
                    out.put(alloc, "httpOnly", src.get("httpOnly") orelse .{ .bool = false }) catch {};
                    const expires: std.json.Value = blk: {
                        const raw_exp = src.get("expires") orelse break :blk .null;
                        const seconds: f64 = switch (raw_exp) {
                            .float => |x| x,
                            .integer => |x| @floatFromInt(x),
                            else => break :blk .null,
                        };
                        if (seconds <= 0) break :blk .null;
                        break :blk .{ .integer = @intFromFloat(seconds) };
                    };
                    out.put(alloc, "expires", expires) catch {};
                    out.put(alloc, "sameSite", src.get("sameSite") orelse .{ .string = "None" }) catch {};
                    cookies.append(.{ .object = out }) catch {};
                }
            }
        }
    }
    payload.put(alloc, "id", .{ .string = id }) catch return;
    payload.put(alloc, "ok", .{ .bool = true }) catch return;
    payload.put(alloc, "cookies", .{ .array = cookies }) catch return;
    f(view.node_id, "cookiesResult", .{ .data = .{ .object = payload } });
}

fn cookieUrl(out: *std.ArrayList(u8), domain: []const u8, path: []const u8, secure: bool) void {
    out.appendSlice(alloc, if (secure) "https://" else "http://") catch return;
    // A leading dot is the cookie-domain spelling, not a hostname.
    const host = if (domain.len > 0 and domain[0] == '.') domain[1..] else domain;
    out.appendSlice(alloc, host) catch return;
    if (path.len == 0 or path[0] != '/') out.append(alloc, '/') catch return;
    out.appendSlice(alloc, path) catch return;
}

fn cmdSetCookie(view: *View, arg: ?std.json.Value) void {
    const obj = argObject(arg) orelse return;
    const name = objStr(obj, "name") orelse return;
    const value = objStr(obj, "value") orelse "";
    const domain = objStr(obj, "domain") orelse return;
    const path = objStr(obj, "path") orelse "/";
    const secure = objBool(obj, "secure") orelse false;

    var url: std.ArrayList(u8) = .empty;
    defer url.deinit(alloc);
    cookieUrl(&url, domain, path, secure);

    var params: std.ArrayList(u8) = .empty;
    defer params.deinit(alloc);
    params.appendSlice(alloc, "{\"name\":") catch return;
    cdp.quote(&params, name);
    params.appendSlice(alloc, ",\"value\":") catch return;
    cdp.quote(&params, value);
    params.appendSlice(alloc, ",\"domain\":") catch return;
    cdp.quote(&params, domain);
    params.appendSlice(alloc, ",\"path\":") catch return;
    cdp.quote(&params, path);
    params.appendSlice(alloc, ",\"url\":") catch return;
    cdp.quote(&params, url.items);
    params.appendSlice(alloc, if (secure) ",\"secure\":true" else ",\"secure\":false") catch return;
    if (objBool(obj, "httpOnly") orelse false) params.appendSlice(alloc, ",\"httpOnly\":true") catch return;
    if (obj.get("expires")) |exp| {
        switch (exp) {
            .integer => |i| {
                var buf: [40]u8 = undefined;
                const n = std.fmt.bufPrint(&buf, ",\"expires\":{d}", .{i}) catch return;
                params.appendSlice(alloc, n) catch return;
            },
            else => {},
        }
    }
    params.appendSlice(alloc, "}") catch return;
    _ = cdpSend(view, "Network.setCookie", params.items, .ignore);
    emitCookiesChanged(view);
}

fn cmdDeleteCookie(view: *View, arg: ?std.json.Value) void {
    const obj = argObject(arg) orelse return;
    const name = objStr(obj, "name") orelse return;
    const domain = objStr(obj, "domain") orelse return;
    const path = objStr(obj, "path") orelse "/";
    var params: std.ArrayList(u8) = .empty;
    defer params.deinit(alloc);
    params.appendSlice(alloc, "{\"name\":") catch return;
    cdp.quote(&params, name);
    params.appendSlice(alloc, ",\"domain\":") catch return;
    cdp.quote(&params, domain);
    params.appendSlice(alloc, ",\"path\":") catch return;
    cdp.quote(&params, path);
    params.appendSlice(alloc, "}") catch return;
    _ = cdpSend(view, "Network.deleteCookies", params.items, .ignore);
    emitCookiesChanged(view);
}

fn emitCookiesChanged(view: *View) void {
    const f = emit orelse return;
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    f(view.node_id, "cookiesChanged", .{ .data = .{ .object = payload } });
}

// ============================================================================
// Find, focus, audio, zoom and user agent
// ============================================================================

fn cmdFindStart(view: *View, arg: ?std.json.Value) void {
    const host = hostOf(view) orelse return;
    const find = host.find orelse return;
    const obj = argObject(arg) orelse return;
    const text = objStr(obj, "text") orelse return;
    const case_sensitive = objBool(obj, "caseSensitive") orelse false;
    var s = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&s);
    if (!setStr(&s, text)) return;
    if (view.last_find) |old| alloc.free(old);
    view.last_find = alloc.dupe(u8, text) catch null;
    find(host, &s, 1, @intFromBool(case_sensitive), 0);
}

fn cmdFindStep(view: *View, forward: bool) void {
    const host = hostOf(view) orelse return;
    const find = host.find orelse return;
    const last = view.last_find orelse return;
    var s = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&s);
    if (!setStr(&s, last)) return;
    find(host, &s, @intFromBool(forward), 0, 1);
}

fn cmdFindStop(view: *View) void {
    const host = hostOf(view) orelse return;
    if (host.stop_finding) |stop| stop(host, 1);
}

fn cmdSetMuted(view: *View, arg: ?std.json.Value) void {
    const host = hostOf(view) orelse return;
    const set_muted = host.set_audio_muted orelse return;
    const muted = switch (arg orelse return) {
        .bool => |b| b,
        else => return,
    };
    set_muted(host, @intFromBool(muted));
    // CEF reports playback state through cef_audio_handler's capture stream,
    // not as a mute notification, so the state change the app asked for is
    // reported from here. `playing` stays false: nothing is observed.
    const f = emit orelse return;
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    payload.put(alloc, "playing", .{ .bool = false }) catch return;
    payload.put(alloc, "muted", .{ .bool = muted }) catch return;
    f(view.node_id, "audioStateChanged", .{ .data = .{ .object = payload } });
}

fn cmdSetZoom(view: *View, arg: ?std.json.Value) void {
    const host = hostOf(view) orelse return;
    const set_zoom = host.set_zoom_level orelse return;
    const factor: f64 = switch (arg orelse return) {
        .float => |x| x,
        .integer => |x| @floatFromInt(x),
        else => {
            std.debug.print("ND_WARN WebView setZoom: malformed arg (expected number)\n", .{});
            return;
        },
    };
    if (factor <= 0) return;
    // CEF's zoom level is logarithmic (0 is 100%), the prop is a linear factor.
    set_zoom(host, std.math.log2(factor) / std.math.log2(1.2));
}

/// A live view's user agent is a CDP override: the request context's own
/// `user_agent` is fixed at context creation.
fn cmdSetUserAgent(view: *View, arg: ?std.json.Value) void {
    const ua: []const u8 = switch (arg orelse return) {
        .string => |s| s,
        else => return,
    };
    var params: std.ArrayList(u8) = .empty;
    defer params.deinit(alloc);
    if (ua.len == 0) {
        _ = cdpSend(view, "Emulation.setUserAgentOverride", "{\"userAgent\":\"\"}", .ignore);
        return;
    }
    params.appendSlice(alloc, "{\"userAgent\":") catch return;
    cdp.quote(&params, ua);
    params.appendSlice(alloc, "}") catch return;
    _ = cdpSend(view, "Emulation.setUserAgentOverride", params.items, .ignore);
}

fn cmdSetContextMenuItems(view: *View, arg: ?std.json.Value) void {
    const items = ctxmenu.parse(alloc, arg orelse .null) catch {
        std.debug.print("ND_WARN WebView setContextMenuItems: out of memory, items unchanged\n", .{});
        return;
    };
    // A menu being built on the CEF UI thread is walking the old tree.
    view.menu_lock.lock();
    const old = view.menu_items;
    view.menu_items = items;
    view.menu_lock.unlock();
    ctxmenu.freeItems(alloc, old);
    tr("setContextMenuItems node={d} items={d}", .{ view.node_id, items.len });
}

fn cmdFocus(view: *View) void {
    _ = gtk.Widget.grabFocus(view.widget);
    const host = hostOf(view) orelse return;
    if (host.set_focus) |set| set(host, 1);
}

// ============================================================================
// Find, favicon and download handlers
// ============================================================================

fn onFindResult(
    self: [*c]c.cef_find_handler_t,
    browser: [*c]c.cef_browser_t,
    _: c_int,
    count: c_int,
    _: [*c]const c.cef_rect_t,
    _: c_int,
    final_update: c_int,
) callconv(.c) void {
    defer ref.releaseParam(browser);
    const view = FindObj.of(self).payload;
    post(.{
        .view = view,
        .name = "findResult",
        .flag = final_update != 0,
        .number = @floatFromInt(count),
    });
}

/// The probe accepts an icon URL without the bytes, and downloading the image
/// to a data URL is a second async hop the contract does not require, so this
/// reports the first URL the page named.
fn onFaviconUrlChange(
    self: [*c]c.cef_display_handler_t,
    browser: [*c]c.cef_browser_t,
    icon_urls: c.cef_string_list_t,
) callconv(.c) void {
    defer ref.releaseParam(browser);
    const api = loader.loaded() orelse return;
    if (icon_urls == null) return;
    if (api.string_list_size(icon_urls) == 0) return;
    var first = std.mem.zeroes(c.cef_string_t);
    defer api.string_utf16_clear(&first);
    if (api.string_list_value(icon_urls, 0, &first) == 0) return;
    post(.{
        .view = DisplayObj.of(self).payload,
        .name = "faviconChanged",
        .text = dupeStr(&first),
    });
}

/// Downloads are the app's to run: cancelling and emitting is what the WebKit
/// backend does, and an engine that silently wrote to ~/Downloads would be a
/// surprise on either.
fn onBeforeDownload(
    self: [*c]c.cef_download_handler_t,
    browser: [*c]c.cef_browser_t,
    download_item: [*c]c.cef_download_item_t,
    suggested_name: [*c]const c.cef_string_t,
    callback: [*c]c.cef_before_download_callback_t,
) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(download_item);
    defer ref.releaseParam(callback);
    const view = DownloadObj.of(self).payload;
    var url: ?[]u8 = null;
    if (download_item != null) {
        if (download_item.*.get_url) |get_url| {
            const raw = get_url(download_item);
            if (raw != null) {
                defer freeUserfree(raw);
                url = dupeStr(raw);
            }
        }
    }
    post(.{
        .view = view,
        .name = "downloadRequested",
        .text = url,
        .extra = dupeStr(suggested_name),
    });
    // 0 means "do not continue": no path is chosen, so the download never runs.
    return 0;
}

fn freeUserfree(s: c.cef_string_userfree_t) void {
    const api = loader.loaded() orelse return;
    api.string_userfree_utf16_free(s);
}

// ============================================================================
// Profiles: one request context per profile
// ============================================================================
//
// The `profile` prop means one cookie jar and one cache, so it maps onto a CEF
// request context: "" is the global one, a named profile is a persistent
// context under the shared root_cache_path, and "private…" is a context with no
// cache path at all, which is CEF's spelling of in-memory.

/// Named profiles are shared: two views asking for the same name must see the
/// same jar. Ephemeral ones are not, by construction.
var profile_contexts: std.StringHashMapUnmanaged(*c.cef_request_context_t) = .empty;

fn requestContext(profile: []const u8) ?*c.cef_request_context_t {
    if (profile.len == 0) return null;
    const api = loader.loaded() orelse return null;
    if (!std.mem.startsWith(u8, profile, "private")) {
        if (profile_contexts.get(profile)) |ctx| return ctx;
    }

    var settings = std.mem.zeroes(c.cef_request_context_settings_t);
    settings.size = @sizeOf(c.cef_request_context_settings_t);
    var path: ?[:0]u8 = null;
    defer if (path) |p| alloc.free(p);
    defer clearStr(&settings.cache_path);

    if (!std.mem.startsWith(u8, profile, "private")) {
        // CEF requires a per-context cache path to sit under root_cache_path,
        // which cef_settings already names.
        const root = defaultCacheRoot() orelse return null;
        defer alloc.free(root);
        const dir = std.fmt.allocPrintSentinel(alloc, "{s}/profiles/{s}", .{ root, profile }, 0) catch return null;
        path = dir;
        _ = glib.mkdirWithParents(dir.ptr, 0o700);
        _ = setStr(&settings.cache_path, dir);
        settings.persist_session_cookies = 1;
    }

    const ctx = api.request_context_create_context(&settings, null);
    if (ctx == null) {
        std.debug.print("ND_WARN WebView engine=chromium: could not create a request context for profile \"{s}\"\n", .{profile});
        return null;
    }
    const typed: *c.cef_request_context_t = @ptrCast(ctx);
    if (std.mem.startsWith(u8, profile, "private")) return typed;

    const key = alloc.dupe(u8, profile) catch return typed;
    profile_contexts.put(alloc, key, typed) catch alloc.free(key);
    return typed;
}

/// CEF exposes no session serialization: there is no equivalent of
/// WebKitWebViewSessionState, and the navigation entries CEF does expose carry
/// no restorable form. The command still answers, because a caller awaiting
/// `sessionSaved` would otherwise wait forever.
fn cmdSaveSession(view: *View, arg: ?std.json.Value) void {
    const obj = argObject(arg) orelse return;
    const id = objStr(obj, "id") orelse return;
    std.debug.print("ND_WARN WebView engine=chromium: saveSession has no CEF equivalent (no session serialization API)\n", .{});
    const f = emit orelse return;
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    payload.put(alloc, "id", .{ .string = id }) catch return;
    payload.put(alloc, "state", .{ .string = "" }) catch return;
    f(view.node_id, "sessionSaved", .{ .data = .{ .object = payload } });
}

// ============================================================================
// JavaScript dialogs
// ============================================================================
//
// `alert`, `confirm` and `prompt` park the page's JS thread until the browser
// answers, and Chrome's own dialog is a window this engine may not have. The
// handler suppresses it and answers from the same scripted-automation path the
// WebKit backend uses, so a headless run behaves identically on both.

const JSDIALOGTYPE_ALERT: c_uint = 0;
const JSDIALOGTYPE_CONFIRM: c_uint = 1;
const JSDIALOGTYPE_PROMPT: c_uint = 2;

fn clientGetJsDialogHandler(self: [*c]c.cef_client_t) callconv(.c) [*c]c.cef_jsdialog_handler_t {
    return ClientObj.of(self).payload.jsdialog_handler.handOut();
}

fn answerJsDialog(callback: [*c]c.cef_jsdialog_callback_t, accepted: bool, text: ?[]const u8) void {
    if (callback == null) return;
    const cont = callback.*.cont orelse return;
    var s = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&s);
    if (text) |t| _ = setStr(&s, t);
    cont(callback, @intFromBool(accepted), &s);
}

fn onJsDialog(
    _: [*c]c.cef_jsdialog_handler_t,
    browser: [*c]c.cef_browser_t,
    _: [*c]const c.cef_string_t,
    dialog_type: c.cef_jsdialog_type_t,
    _: [*c]const c.cef_string_t,
    _: [*c]const c.cef_string_t,
    callback: [*c]c.cef_jsdialog_callback_t,
    suppress_message: [*c]c_int,
) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(callback);
    if (suppress_message != null) suppress_message.* = 0;

    const next = automation_dialogs.take("webview.scriptDialog");
    switch (next) {
        .unscripted => {
            // No app-side sheet on this engine yet, and a dialog nobody answers
            // parks the page's JS thread for good, so it is dismissed rather
            // than left open. An alert is "seen" either way.
            answerJsDialog(callback, dialog_type == JSDIALOGTYPE_ALERT, null);
            return 1;
        },
        .exhausted => {
            std.debug.print("ND_WARN WebView scriptDialog: the automation dialog script ran out of answers; dismissing\n", .{});
            answerJsDialog(callback, false, null);
            return 1;
        },
        .response => |raw| {
            var parsed = std.json.parseFromSlice(std.json.Value, alloc, raw, .{}) catch {
                std.debug.print("ND_WARN WebView scriptDialog: malformed scripted answer {s}; dismissing\n", .{raw});
                answerJsDialog(callback, false, null);
                return 1;
            };
            defer parsed.deinit();
            var accepted = true;
            var text: ?[]const u8 = null;
            if (parsed.value == .object) {
                if (parsed.value.object.get("accepted")) |a| {
                    if (a == .bool) accepted = a.bool;
                }
                if (parsed.value.object.get("text")) |t| {
                    if (t == .string) text = t.string;
                }
            }
            answerJsDialog(callback, accepted, if (accepted) text else null);
            return 1;
        },
    }
}

/// Leaving a page is never blocked: the framework gives an app no way to
/// express a policy for onbeforeunload, and the WebKit backend answers the
/// same way for the same reason.
fn onBeforeUnloadDialog(
    _: [*c]c.cef_jsdialog_handler_t,
    browser: [*c]c.cef_browser_t,
    _: [*c]const c.cef_string_t,
    _: c_int,
    callback: [*c]c.cef_jsdialog_callback_t,
) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(callback);
    answerJsDialog(callback, true, null);
    return 1;
}

/// Explicit rather than defaulted: a null `can_download` leaves the decision to
/// Chromium, and the contract is that the app decides.
fn onCanDownload(
    _: [*c]c.cef_download_handler_t,
    browser: [*c]c.cef_browser_t,
    _: [*c]const c.cef_string_t,
    _: [*c]const c.cef_string_t,
) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    return 1;
}

// ============================================================================
// Custom URI schemes
// ============================================================================
//
// Two halves that have to agree. The scheme has to be a STANDARD scheme in
// every process, or Chromium will not parse `ndprobe://host/path` as a
// navigable URL at all, and that registration happens during startup, long
// before an app exists: the browser process knows the list because
// `registerScheme` ran before cef_initialize, and every subprocess reads it off
// the command line the browser appended it to. The other half is the factory
// that serves the requests, which parks each one until the app answers it,
// exactly as the WebKitGTK backend's `respondScheme` does.

const CEF_SCHEME_OPTION_STANDARD: c_int = 1 << 0;
const CEF_SCHEME_OPTION_CORS_ENABLED: c_int = 1 << 4;
const CEF_SCHEME_OPTION_SECURE: c_int = 1 << 3;
const CEF_SCHEME_OPTION_FETCH_ENABLED: c_int = 1 << 5;

const SchemeSpec = struct { name: []u8, cors: bool, secure: bool };

var custom_schemes: std.ArrayList(SchemeSpec) = .empty;
/// Schemes whose factory is already registered, so a second view does not
/// register a second one.
var scheme_factories: std.StringHashMapUnmanaged(void) = .empty;

/// The origin properties a launch-declared scheme gets, matching the AppKit
/// engine's `app_register_schemes` byte for byte: an app that declares one
/// scheme for both platforms must not get two different origins.
const env_scheme_options: c_int = CEF_SCHEME_OPTION_STANDARD | CEF_SCHEME_OPTION_SECURE |
    CEF_SCHEME_OPTION_CORS_ENABLED | CEF_SCHEME_OPTION_FETCH_ENABLED;

var env_schemes: std.ArrayList([]u8) = .empty;
var env_schemes_read = false;

/// `ND_CEF_SCHEMES`, comma separated. The launch path sets it because a scheme
/// only becomes standard during startup, in EVERY process, and by the time an
/// app could call `registerScheme` the renderers have already been told what
/// they will and will not parse. Child processes inherit the environment, so
/// this needs no propagation of its own.
fn envSchemes() []const []u8 {
    if (env_schemes_read) return env_schemes.items;
    env_schemes_read = true;
    const raw = std.c.getenv("ND_CEF_SCHEMES") orelse return env_schemes.items;
    var it = std.mem.splitScalar(u8, std.mem.span(raw), ',');
    while (it.next()) |part| {
        const name = std.mem.trim(u8, part, " \t");
        if (name.len == 0 or name.len >= 64) continue;
        const copy = alloc.dupe(u8, name) catch continue;
        env_schemes.append(alloc, copy) catch alloc.free(copy);
    }
    return env_schemes.items;
}

fn isEnvScheme(name: []const u8) bool {
    for (envSchemes()) |declared| {
        if (std.mem.eql(u8, declared, name)) return true;
    }
    return false;
}

/// Records a scheme for `on_register_custom_schemes` and gives it a handler
/// factory. Returns false when the scheme cannot be served at all.
///
/// After cef_initialize the origin half is closed: a scheme that is not already
/// standard cannot become one. A scheme the launch path declared through
/// ND_CEF_SCHEMES is already standard in every process, so a late call for one
/// of those still gets its factory, which is the whole point of splitting the
/// two halves.
pub fn registerScheme(scheme: []const u8, cors_enabled: bool, secure: bool) bool {
    tr("registerScheme {s} cors={} secure={} initialized={} declared={}", .{
        scheme, cors_enabled, secure, initialized, isEnvScheme(scheme),
    });
    for (custom_schemes.items) |s| {
        if (std.mem.eql(u8, s.name, scheme)) {
            if (initialized) ensureSchemeFactories();
            return true;
        }
    }
    if (initialized and !isEnvScheme(scheme)) return false;
    const name = alloc.dupe(u8, scheme) catch return false;
    custom_schemes.append(alloc, .{ .name = name, .cors = cors_enabled, .secure = secure }) catch {
        alloc.free(name);
        return false;
    };
    if (initialized) ensureSchemeFactories();
    return true;
}

fn addCustomScheme(
    registrar: [*c]c.cef_scheme_registrar_t,
    add: *const fn ([*c]c.cef_scheme_registrar_t, [*c]const c.cef_string_t, c_int) callconv(.c) c_int,
    name: []const u8,
    options: c_int,
) void {
    var s = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&s);
    if (!setStr(&s, name)) return;
    if (add(registrar, &s, options) == 0) {
        std.debug.print("ND_WARN CEF scheme {s}: the engine refused to register it\n", .{name});
    }
}

fn onRegisterCustomSchemes(_: [*c]c.cef_app_t, registrar: [*c]c.cef_scheme_registrar_t) callconv(.c) void {
    if (registrar == null) return;
    const add = registrar.*.add_custom_scheme orelse return;
    // Launch-declared schemes first: they are the ones a subprocess can know
    // about, and their options are fixed by the contract.
    for (envSchemes()) |name| addCustomScheme(registrar, add, name, env_scheme_options);
    // A subprocess has no app and no runtime registrations of its own; the
    // browser put those on its command line for exactly this moment.
    if (custom_schemes.items.len == 0) adoptSchemesFromCommandLine();
    tr("onRegisterCustomSchemes env={d} runtime={d}", .{ envSchemes().len, custom_schemes.items.len });
    for (custom_schemes.items) |spec| {
        // Already registered above, with the properties the contract fixes.
        if (isEnvScheme(spec.name)) continue;
        var options: c_int = CEF_SCHEME_OPTION_STANDARD | CEF_SCHEME_OPTION_FETCH_ENABLED;
        if (spec.cors) options |= CEF_SCHEME_OPTION_CORS_ENABLED;
        if (spec.secure) options |= CEF_SCHEME_OPTION_SECURE;
        addCustomScheme(registrar, add, spec.name, options);
    }
}

const schemes_switch = "nd-schemes";

fn adoptSchemesFromCommandLine() void {
    const api = loader.loaded() orelse return;
    const cl = api.command_line_get_global();
    if (cl == null) return;
    defer ref.releaseParam(cl);
    const get = cl.*.get_switch_value orelse return;
    var name = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&name);
    if (!setStr(&name, schemes_switch)) return;
    const raw = get(cl, &name);
    if (raw == null) return;
    defer freeUserfree(raw);
    const list = dupeStr(raw) orelse return;
    defer alloc.free(list);
    var it = std.mem.splitScalar(u8, list, ',');
    while (it.next()) |part| {
        if (part.len == 0) continue;
        // Only the name survives the trip: options are a browser-process
        // concern for the factory, and the child needs the scheme to be
        // standard and fetchable, which is the same for all of them.
        const copy = alloc.dupe(u8, part) catch continue;
        custom_schemes.append(alloc, .{ .name = copy, .cors = true, .secure = true }) catch alloc.free(copy);
    }
}

fn appendSchemesSwitch(cl: [*c]c.cef_command_line_t) void {
    if (custom_schemes.items.len == 0) return;
    const append = cl.*.append_switch_with_value orelse return;
    var joined: std.ArrayList(u8) = .empty;
    defer joined.deinit(alloc);
    for (custom_schemes.items, 0..) |spec, i| {
        if (i != 0) joined.append(alloc, ',') catch return;
        joined.appendSlice(alloc, spec.name) catch return;
    }
    appendSwitch(cl, append, schemes_switch, joined.items);
}

// ---- Serving -------------------------------------------------------------

const FactoryObj = ref.Counted(c.cef_scheme_handler_factory_t, void);
const ResourceObj = ref.Counted(c.cef_resource_handler_t, *Resource);

/// One parked scheme request. Created on the IO thread, filled on the GTK
/// thread when the app answers, read back on the IO thread afterwards. The
/// `cont` call is the handoff between the two, and nothing touches `body`
/// before it.
const Resource = struct {
    id: []u8,
    url: []u8,
    scheme: []u8,
    browser_id: c_int,
    callback: std.atomic.Value(usize) = .init(0),
    body: []u8 = &.{},
    mime: []u8 = &.{},
    status: c_int = 200,
    offset: usize = 0,
    failed: bool = false,
};

var pending_scheme_requests: std.StringHashMapUnmanaged(*ResourceObj) = .empty;
var scheme_seq: u64 = 0;
/// Browser identifier to view, so a request arriving on the IO thread knows
/// which node to raise `schemeRequest` on. CEF hands out a fresh wrapper
/// pointer per callback, so identity has to come from the id.
var browsers_by_id: std.AutoHashMapUnmanaged(c_int, *View) = .empty;

fn ensureSchemeFactories() void {
    const api = loader.loaded() orelse return;
    tr("ensureSchemeFactories count={d}", .{custom_schemes.items.len});
    for (custom_schemes.items) |spec| {
        if (scheme_factories.contains(spec.name)) continue;
        const factory = FactoryObj.create({}) orelse continue;
        factory.cef.create = &factoryCreate;
        var name = std.mem.zeroes(c.cef_string_t);
        var domain = std.mem.zeroes(c.cef_string_t);
        defer clearStr(&name);
        defer clearStr(&domain);
        if (!setStr(&name, spec.name)) {
            factory.drop();
            continue;
        }
        // The factory reference is consumed by the registration.
        if (api.register_scheme_handler_factory(&name, &domain, factory.handOut()) == 0) {
            factory.drop();
            std.debug.print("ND_WARN WebView engine=chromium: could not register a handler factory for \"{s}\"\n", .{spec.name});
            continue;
        }
        factory.drop();
        const key = alloc.dupe(u8, spec.name) catch continue;
        scheme_factories.put(alloc, key, {}) catch alloc.free(key);
    }
}

fn factoryCreate(
    _: [*c]c.cef_scheme_handler_factory_t,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    scheme_name: [*c]const c.cef_string_t,
    request: [*c]c.cef_request_t,
) callconv(.c) [*c]c.cef_resource_handler_t {
    defer ref.releaseParam(frame);
    defer ref.releaseParam(request);
    var browser_id: c_int = 0;
    if (browser != null) {
        if (browser.*.get_identifier) |get_id| browser_id = get_id(browser);
    }
    ref.releaseParam(browser);
    tr("factoryCreate browser={d}", .{browser_id});
    var url: []u8 = &.{};
    if (request != null) {
        if (request.*.get_url) |get_url| {
            const raw = get_url(request);
            if (raw != null) {
                defer freeUserfree(raw);
                url = dupeStr(raw) orelse &.{};
            }
        }
    }
    const scheme: []u8 = dupeStr(scheme_name) orelse (alloc.dupe(u8, "") catch return null);

    const res = alloc.create(Resource) catch return null;
    res.* = .{ .id = &.{}, .url = url, .scheme = scheme, .browser_id = browser_id };
    const obj = ResourceObj.create(res) orelse {
        alloc.destroy(res);
        return null;
    };
    obj.cef.open = &resourceOpen;
    obj.cef.get_response_headers = &resourceGetResponseHeaders;
    obj.cef.read = &resourceRead;
    obj.cef.cancel = &resourceCancel;
    // The caller takes the reference this returns.
    return obj.cptr();
}

fn resourceOpen(
    self: [*c]c.cef_resource_handler_t,
    request: [*c]c.cef_request_t,
    handle_request: [*c]c_int,
    callback: [*c]c.cef_callback_t,
) callconv(.c) c_int {
    defer ref.releaseParam(request);
    const obj = ResourceObj.of(self);
    // Kept, not released: this is what wakes the request once the app answers.
    obj.payload.callback.store(@intFromPtr(callback), .release);
    tr("resourceOpen url={s}", .{obj.payload.url});
    if (handle_request != null) handle_request.* = 0;
    // The registry and the event both live on the GTK thread, so the request is
    // handed over rather than announced from here.
    post(.{ .view = @ptrFromInt(@intFromPtr(obj)), .name = "schemeRequest", .scheme_obj = obj });
    return 1;
}

fn resourceGetResponseHeaders(
    self: [*c]c.cef_resource_handler_t,
    response: [*c]c.cef_response_t,
    response_length: [*c]i64,
    _: [*c]c.cef_string_t,
) callconv(.c) void {
    const res = ResourceObj.of(self).payload;
    if (response != null) {
        if (response.*.set_status) |set| set(response, if (res.failed) 500 else res.status);
        if (res.mime.len > 0) {
            if (response.*.set_mime_type) |set| {
                var s = std.mem.zeroes(c.cef_string_t);
                defer clearStr(&s);
                if (setStr(&s, res.mime)) set(response, &s);
            }
        }
    }
    if (response_length != null) response_length.* = @intCast(res.body.len);
}

fn resourceRead(
    self: [*c]c.cef_resource_handler_t,
    data_out: ?*anyopaque,
    bytes_to_read: c_int,
    bytes_read: [*c]c_int,
    _: [*c]c.cef_resource_read_callback_t,
) callconv(.c) c_int {
    const res = ResourceObj.of(self).payload;
    if (bytes_read != null) bytes_read.* = 0;
    if (res.offset >= res.body.len) return 0; // 0 with no bytes is completion
    const out = data_out orelse return 0;
    const n = @min(@as(usize, @intCast(@max(bytes_to_read, 0))), res.body.len - res.offset);
    if (n == 0) return 0;
    @memcpy(@as([*]u8, @ptrCast(out))[0..n], res.body[res.offset..][0..n]);
    res.offset += n;
    if (bytes_read != null) bytes_read.* = @intCast(n);
    return 1;
}

fn resourceCancel(self: [*c]c.cef_resource_handler_t) callconv(.c) void {
    const obj = ResourceObj.of(self);
    const raw = obj.payload.callback.swap(0, .acq_rel);
    if (raw != 0) ref.releaseParam(@as([*c]c.cef_callback_t, @ptrFromInt(raw)));
}

/// GTK thread: registers the parked request and raises the event the app
/// answers with `respondScheme`.
fn announceSchemeRequest(obj: *ResourceObj) void {
    const res = obj.payload;
    const view = browsers_by_id.get(res.browser_id);
    tr("announceSchemeRequest browser={d} view={?d}", .{ res.browser_id, if (view) |v| v.node_id else null });
    scheme_seq += 1;
    const id = std.fmt.allocPrint(alloc, "cefscheme-{d}", .{scheme_seq}) catch return;
    res.id = id;
    if (view == null) {
        failSchemeRequest(obj, "no view for this browser");
        return;
    }
    // The map holds a reference of its own: a cancelled request releases CEF's,
    // and the answer may still be in flight.
    _ = obj.handOut();
    pending_scheme_requests.put(alloc, id, obj) catch {
        obj.drop();
        failSchemeRequest(obj, "out of memory");
        return;
    };
    const f = emit orelse return;
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    payload.put(alloc, "id", .{ .string = id }) catch return;
    payload.put(alloc, "url", .{ .string = res.url }) catch return;
    payload.put(alloc, "scheme", .{ .string = res.scheme }) catch return;
    f(view.?.node_id, "schemeRequest", .{ .data = .{ .object = payload } });
}

fn continueSchemeRequest(obj: *ResourceObj) void {
    const raw = obj.payload.callback.swap(0, .acq_rel);
    if (raw == 0) return;
    const callback: [*c]c.cef_callback_t = @ptrFromInt(raw);
    defer ref.releaseParam(callback);
    if (callback.*.cont) |cont| cont(callback);
}

fn failSchemeRequest(obj: *ResourceObj, message: []const u8) void {
    std.debug.print("ND_WARN WebView engine=chromium schemeRequest: {s}\n", .{message});
    obj.payload.failed = true;
    continueSchemeRequest(obj);
}

fn cmdRespondScheme(arg: ?std.json.Value) void {
    const obj_arg = argObject(arg) orelse return;
    const id = objStr(obj_arg, "id") orelse {
        std.debug.print("ND_WARN WebView respondScheme: missing id\n", .{});
        return;
    };
    const entry = pending_scheme_requests.fetchRemove(id) orelse {
        std.debug.print("ND_WARN WebView respondScheme: unknown request id {s}\n", .{id});
        return;
    };
    defer alloc.free(entry.key);
    const obj = entry.value;
    defer obj.drop();
    const res = obj.payload;

    if (objStr(obj_arg, "error")) |msg| {
        failSchemeRequest(obj, msg);
        return;
    }
    const b64 = objStr(obj_arg, "base64") orelse {
        failSchemeRequest(obj, "respondScheme: missing base64 body");
        return;
    };
    const decoder = std.base64.standard.Decoder;
    const size = decoder.calcSizeForSlice(b64) catch {
        failSchemeRequest(obj, "respondScheme: malformed base64 body");
        return;
    };
    const buf = alloc.alloc(u8, @max(size, 1)) catch {
        failSchemeRequest(obj, "respondScheme: out of memory");
        return;
    };
    decoder.decode(buf[0..size], b64) catch {
        alloc.free(buf);
        failSchemeRequest(obj, "respondScheme: malformed base64 body");
        return;
    };
    res.body = buf[0..size];
    // `mime` is the key the contract uses, the same one the WebKitGTK backend
    // reads; `contentType` is accepted because it is the obvious guess.
    const mime = objStr(obj_arg, "mime") orelse objStr(obj_arg, "contentType") orelse "application/octet-stream";
    res.mime = alloc.dupe(u8, mime) catch &.{};
    if (obj_arg.get("status")) |st| {
        switch (st) {
            .integer => |i| res.status = @intCast(i),
            else => {},
        }
    }
    continueSchemeRequest(obj);
}

// ============================================================================
// Context menus
// ============================================================================
//
// `native` keeps Chromium's own menu and appends the app's matching items after
// a separator; `suppress` shows nothing and leaves the whole decision to the
// app's `contextMenu` event. Both modes emit that event, which is what the
// WebKitGTK backend does, so an app can drive its own menu on either engine.
//
// Everything here runs on the CEF UI thread and has to answer before it
// returns: a menu model must be populated before `on_before_context_menu`
// returns, so unlike every other event on this backend it cannot be marshaled
// to GTK first. The app's tree is read under `menu_lock` instead, and only the
// resulting events take the usual hop.

/// A lock the CEF UI thread can take. std.Io.Mutex needs an `Io` to block
/// against and a CEF callback has none; both critical sections here are a menu
/// tree walk, and contention needs a right-click to land inside the same
/// microsecond as a `setContextMenuItems`.
const SpinLock = struct {
    held: std.atomic.Value(bool) = .init(false),

    fn lock(self: *SpinLock) void {
        while (self.held.cmpxchgWeak(false, true, .acquire, .monotonic) != null) {
            std.Thread.yield() catch {};
        }
    }

    fn unlock(self: *SpinLock) void {
        self.held.store(false, .release);
    }
};

/// CEF reserves everything outside [MENU_ID_USER_FIRST, MENU_ID_USER_LAST] for
/// Chromium's own commands.
const menu_command_first: c_int = 26500;
const menu_command_last: c_int = 28500;

const CM_TYPEFLAG_SELECTION: c_uint = 1 << 4;
const CM_TYPEFLAG_EDITABLE: c_uint = 1 << 5;

/// An owned copy, falling back to an empty (still freeable) slice. Every
/// string in a menu payload is copied because the params it came from are gone
/// before the GTK loop sees it.
var no_bytes: [0]u8 = .{};

fn dupeOwned(text: []const u8) []u8 {
    return alloc.dupe(u8, text) catch no_bytes[0..0];
}

/// One item currently on screen. The id and check state are copied because the
/// app may replace its whole tree between the menu opening and a click landing.
const MenuCommand = struct {
    id: []u8,
    kind: ctxmenu.Kind,
    checked: bool,
};

/// What the click landed on, owned so it can outlive the params object.
const MenuHit = struct {
    link: []u8 = no_bytes[0..0],
    image: []u8 = no_bytes[0..0],
    selection: []u8 = no_bytes[0..0],
    editable: bool = false,
    x: c_int = 0,
    y: c_int = 0,

    fn deinit(self: *MenuHit) void {
        alloc.free(self.link);
        alloc.free(self.image);
        alloc.free(self.selection);
    }

    fn asCtx(self: *const MenuHit) ctxmenu.Hit {
        return .{
            .link = self.link,
            .image = self.image,
            .selection = self.selection,
            .editable = self.editable,
            .has_selection = self.selection.len > 0,
        };
    }
};

fn readMenuHit(params: [*c]c.cef_context_menu_params_t) MenuHit {
    var hit: MenuHit = .{};
    if (params == null) return hit;
    if (params.*.get_xcoord) |f| hit.x = f(params);
    if (params.*.get_ycoord) |f| hit.y = f(params);
    if (params.*.get_type_flags) |f| {
        const flags = f(params);
        hit.editable = (flags & CM_TYPEFLAG_EDITABLE) != 0;
    }
    if (params.*.is_editable) |f| {
        if (f(params) != 0) hit.editable = true;
    }
    hit.link = ownedFromUserfree(params, params.*.get_link_url);
    hit.image = ownedFromUserfree(params, params.*.get_source_url);
    hit.selection = ownedFromUserfree(params, params.*.get_selection_text);
    return hit;
}

fn ownedFromUserfree(
    params: [*c]c.cef_context_menu_params_t,
    getter: ?*const fn ([*c]c.cef_context_menu_params_t) callconv(.c) c.cef_string_userfree_t,
) []u8 {
    const get = getter orelse return no_bytes[0..0];
    const raw = get(params);
    if (raw == null) return no_bytes[0..0];
    defer freeUserfree(raw);
    return dupeStr(raw) orelse no_bytes[0..0];
}

fn clientGetContextMenuHandler(self: [*c]c.cef_client_t) callconv(.c) [*c]c.cef_context_menu_handler_t {
    return ClientObj.of(self).payload.context_menu_handler.handOut();
}

fn clearMenuCommands(view: *View) void {
    var it = view.menu_commands.valueIterator();
    while (it.next()) |cmd| alloc.free(cmd.id);
    view.menu_commands.clearRetainingCapacity();
    view.next_menu_command = menu_command_first;
}

fn onBeforeContextMenu(
    self: [*c]c.cef_context_menu_handler_t,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    params: [*c]c.cef_context_menu_params_t,
    model: [*c]c.cef_menu_model_t,
) callconv(.c) void {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(frame);
    defer ref.releaseParam(params);
    defer ref.releaseParam(model);
    const view = ContextMenuObj.of(self).payload;

    if (view.suppress_menu.load(.acquire)) {
        // Nothing of Chromium's is shown in suppress mode, and an empty model
        // is what makes that true even if run_context_menu is never consulted.
        if (model != null) {
            if (model.*.clear) |clear| _ = clear(model);
        }
        return;
    }
    if (model == null) return;

    var hit = readMenuHit(params);
    defer hit.deinit();

    view.menu_lock.lock();
    defer view.menu_lock.unlock();
    clearMenuCommands(view);
    if (view.menu_items.len == 0) return;

    const ctx_hit = hit.asCtx();
    var any = false;
    for (view.menu_items) |item| {
        if (item.kind == .separator) continue;
        if (!ctxmenu.survives(item, ctx_hit)) continue;
        any = true;
        break;
    }
    if (!any) return;
    if (model.*.add_separator) |sep| _ = sep(model);
    appendMenuItems(view, view.menu_items, model, ctx_hit);
}

fn appendMenuItems(
    view: *View,
    items: []const ctxmenu.Item,
    model: [*c]c.cef_menu_model_t,
    hit: ctxmenu.Hit,
) void {
    var appended: usize = 0;
    var pending_separator = false;
    for (items) |item| {
        if (item.kind == .separator) {
            if (appended > 0) pending_separator = true;
            continue;
        }
        if (!ctxmenu.survives(item, hit)) continue;
        if (pending_separator) {
            if (model.*.add_separator) |sep| _ = sep(model);
            pending_separator = false;
        }
        const command_id = nextMenuCommand(view) orelse return;
        var label = std.mem.zeroes(c.cef_string_t);
        defer clearStr(&label);
        if (!setStr(&label, item.label)) continue;

        if (item.children.len > 0) {
            const add_sub = model.*.add_sub_menu orelse continue;
            const submenu = add_sub(model, command_id, &label);
            if (submenu == null) continue;
            defer ref.releaseOwned(submenu);
            appendMenuItems(view, item.children, submenu, hit);
            appended += 1;
            continue;
        }

        const key = alloc.dupe(u8, item.id) catch continue;
        view.menu_commands.put(alloc, command_id, .{
            .id = key,
            .kind = item.kind,
            .checked = item.checked,
        }) catch {
            alloc.free(key);
            continue;
        };
        if (item.kind == .checkbox or item.kind == .radio) {
            if (model.*.add_check_item) |add| _ = add(model, command_id, &label);
            if (model.*.set_checked) |set| _ = set(model, command_id, @intFromBool(item.checked));
        } else {
            if (model.*.add_item) |add| _ = add(model, command_id, &label);
        }
        if (!item.enabled) {
            if (model.*.set_enabled) |set| _ = set(model, command_id, 0);
        }
        appended += 1;
    }
}

fn nextMenuCommand(view: *View) ?c_int {
    if (view.next_menu_command >= menu_command_last) return null;
    const id = view.next_menu_command;
    view.next_menu_command += 1;
    return id;
}

fn onRunContextMenu(
    self: [*c]c.cef_context_menu_handler_t,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    params: [*c]c.cef_context_menu_params_t,
    model: [*c]c.cef_menu_model_t,
    callback: [*c]c.cef_run_context_menu_callback_t,
) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(frame);
    defer ref.releaseParam(params);
    defer ref.releaseParam(model);
    defer ref.releaseParam(callback);
    const view = ContextMenuObj.of(self).payload;

    // Emitted in both modes, exactly as the WebKitGTK backend does: an app that
    // wants to decorate the native menu and an app that wants to replace it
    // both need to know where the click landed.
    var hit = readMenuHit(params);
    const boxed = alloc.create(MenuHit) catch {
        hit.deinit();
        return 0;
    };
    boxed.* = hit;
    post(.{ .view = view, .name = "contextMenu", .menu_hit = boxed });

    if (!view.suppress_menu.load(.acquire)) return 0;
    if (callback != null) {
        if (callback.*.cancel) |cancel| cancel(callback);
    }
    return 1;
}

fn onContextMenuCommand(
    self: [*c]c.cef_context_menu_handler_t,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    params: [*c]c.cef_context_menu_params_t,
    command_id: c_int,
    _: c.cef_event_flags_t,
) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(frame);
    defer ref.releaseParam(params);
    const view = ContextMenuObj.of(self).payload;

    var hit = readMenuHit(params);
    defer hit.deinit();

    view.menu_lock.lock();
    const entry = view.menu_commands.get(command_id);
    const page_url = dupeOwned(if (view.menu_page_url_slot) |u| u else "");
    view.menu_lock.unlock();

    const cmd = entry orelse {
        alloc.free(page_url);
        return 0; // one of Chromium's own commands
    };

    const click = alloc.create(MenuClick) catch {
        alloc.free(page_url);
        return 1;
    };
    click.* = .{
        .id = dupeOwned(cmd.id),
        .page_url = page_url,
        .link = dupeOwned(hit.link),
        .image = dupeOwned(hit.image),
        .selection = dupeOwned(hit.selection),
        .editable = hit.editable,
        // The framework reports the state the click IMPLIES and does not mutate
        // its own copy: the app owns the model and answers with the next
        // setContextMenuItems.
        .checked = switch (cmd.kind) {
            .checkbox => !cmd.checked,
            .radio => true,
            else => null,
        },
        .was_checked = switch (cmd.kind) {
            .checkbox, .radio => cmd.checked,
            else => null,
        },
    };
    post(.{ .view = view, .name = "contextMenuItemClicked", .menu_click = click });
    return 1;
}

const MenuClick = struct {
    id: []u8,
    page_url: []u8,
    link: []u8,
    image: []u8,
    selection: []u8,
    editable: bool,
    checked: ?bool,
    was_checked: ?bool,

    fn deinit(self: *MenuClick) void {
        alloc.free(self.id);
        alloc.free(self.page_url);
        alloc.free(self.link);
        alloc.free(self.image);
        alloc.free(self.selection);
    }
};
