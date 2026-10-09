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
const gio = @import("gio");
const gobject = @import("gobject");
const adw = @import("adw");
const graphene = @import("graphene");
const protocol = @import("../protocol.zig");
const marker = @import("../marker.zig");
const capi = @import("capi.zig");
const c = capi.c;
const ref = @import("ref.zig");
const loader = @import("loader.zig");
const x11 = @import("x11.zig");
const shape = @import("shape.zig");
const app_dir = @import("app_dir.zig");
const cdp = @import("cdp.zig");
const browser_pipe = @import("browser_pipe.zig");
const hyprland = @import("hyprland.zig");
const pip_landing = @import("pip_landing.zig");
const ctxmenu = @import("../gtk/context_menu.zig");
const ndchrome = @import("../gtk/chrome.zig");
const gtkmenu = @import("gtkmenu.zig");
const automation_dialogs = @import("../automation_dialogs.zig");
const types = @import("types.zig");
const adblock = @import("../adblock.zig");

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
        // The browser process, before anything else here opens a descriptor.
        if (chromeStyle() and !browser_pipe.reserve()) {
            std.debug.print("ND_WARN CEF: fds 3 and 4 are taken, so extension action clicks are unavailable\n", .{});
        }
        return null;
    }
    return @truncate(@as(u32, @bitCast(rc)));
}

/// A SIGTERM or SIGINT sent to the host's process group or cgroup (Ctrl+C,
/// `kill -- -pgid`, a logout) also reaches the zygote, GPU and utility
/// processes. Killed under a live browser, the GPU process cannot be relaunched
/// through the dead zygote, and Chromium LOG(FATAL)s "GPU process isn't
/// usable" (a SIGTRAP) before the host's graceful quit closes the browsers.
/// Ignoring them leaves the children to exit when the browser process does.
/// Set here rather than before cef_execute_process because Chromium resets
/// both signals to the default early in every child.
fn ignoreHostQuitSignals() void {
    const ignore: std.posix.Sigaction = .{
        .handler = .{ .handler = std.posix.SIG.IGN },
        .mask = std.posix.sigemptyset(),
        .flags = 0,
    };
    std.posix.sigaction(.TERM, &ignore, null);
    std.posix.sigaction(.INT, &ignore, null);
}

fn mainArgs() c.cef_main_args_t {
    return .{
        .argc = @intCast(main_argv.len),
        .argv = @ptrCast(@constCast(main_argv.ptr)),
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
    closeBrowsersInOrder();
    initialized = false;
    api.shutdown();
}

/// Closes every browser still open, devtools first, before `cef_shutdown`.
///
/// Quitting the host tears the node tree down without destroying the webview
/// widgets, so `onDestroy` never runs for a live view: nothing has asked CEF
/// to close anything by the time `cef_shutdown` starts, and CEF then unwinds
/// the browsers in whatever order its own teardown reaches them. With a
/// devtools browser attached that order is wrong twice over. The inspected
/// browser goes first and its frames are deleted from a task that runs after
/// its `CefBrowserContentsDelegate` is gone, which is the SIGSEGV in
/// `CefBrowserInfo::RemoveFrame` under
/// `BackForwardCacheImpl::DestroyEvictedFrames`; and the devtools browser,
/// left behind, never reports closed, so `CefUIThread::Stop` joins a run loop
/// that will not quit. Asking in this order and waiting for each
/// `on_before_close` costs about 25ms and both go away.
/// Set for the rest of the process's life by `closeBrowsersInOrder`. A layout
/// pass that reaches the server after a browser's window is gone raises
/// BadWindow on CEF's connection, where GDK's error trap cannot see it (the
/// trap matches request sequences on GDK's own connection), and GDK's handler
/// aborts the host on the way out.
var shutting_down = false;

fn closeBrowsersInOrder() void {
    shutting_down = true;
    // Chrome's own browser first: the install and post-install dialogs are
    // anchored to it, and leaving it for cef_shutdown to unwind is the SIGSEGV
    // a host takes on the way out after a Web Store install.
    if (kept_host) |host| {
        if (host.*.close_browser) |close| close(host, 1);
        var waited: u32 = 0;
        while (kept_host != null and waited < 3000) : (waited += 5) glib.usleep(5 * 1000);
        tr("shutdown chrome browser closed after {d}ms", .{waited});
    }

    var devtools = false;
    var it = live_views.keyIterator();
    while (it.next()) |key| {
        const view: *View = @ptrFromInt(key.*);
        const host = hostOf(view) orelse continue;
        if (view.devtools_window.load(.acquire) == 0) continue;
        if (host.*.close_dev_tools) |close_dev_tools| close_dev_tools(host);
        devtools = true;
    }
    if (devtools) _ = waitForViews(&View.devtoolsOpen, 3000);

    it = live_views.keyIterator();
    while (it.next()) |key| {
        const view: *View = @ptrFromInt(key.*);
        const host = hostOf(view) orelse continue;
        if (host.*.close_browser) |close| close(host, 1);
    }
    const waited = waitForViews(&View.browserOpen, 5000);
    tr("shutdown browsers closed after {d}ms", .{waited});
    const windows = waitForViews(&View.viewsWindowOpen, 3000);
    tr("shutdown views windows destroyed after {d}ms", .{windows});
}

/// Blocks the quitting thread until no live view answers `open`, or until the
/// deadline; returns how long it waited. The CEF UI thread has its own loop, so
/// this thread has nothing left to serve. std.Thread.sleep is gone in Zig 0.16
/// and there is no Io here to sleep against, so the wait goes through glib.
fn waitForViews(open: *const fn (*View) bool, timeout_ms: u32) u32 {
    const step_ms: u32 = 5;
    var waited: u32 = 0;
    while (waited < timeout_ms) : (waited += step_ms) {
        var any = false;
        var it = live_views.keyIterator();
        while (it.next()) |key| {
            const view: *View = @ptrFromInt(key.*);
            if (open(view)) any = true;
        }
        if (!any) return waited;
        glib.usleep(step_ms * 1000);
    }
    return waited;
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
    a.cef.get_render_process_handler = &appGetRenderProcessHandler;
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
        h.cef.get_default_client = &getDefaultClient;
        h.cef.on_already_running_app_relaunch = &onAlreadyRunningAppRelaunch;
        h.cef.on_context_initialized = &onContextInitialized;
        browser_process_obj = h;
    }
    return browser_process_obj.?.handOut();
}

/// Another launch on this cache directory. Chromium's process singleton has
/// forwarded its command line here and will make that process's
/// cef_initialize fail; left unanswered, Chrome's StartupBrowserCreator opens a
/// "New Tab" browser window in this process, with "Restore pages?" over it when
/// the profile's last exit was a crash. Answered as handled, so nothing opens.
fn onAlreadyRunningAppRelaunch(
    _: [*c]c.cef_browser_process_handler_t,
    command_line: [*c]c.cef_command_line_t,
    _: [*c]const c.cef_string_t,
) callconv(.c) c_int {
    ref.releaseParam(command_line);
    std.debug.print("ND_CEF_RELAUNCH_REFUSED another launch on this cache directory was turned away\n", .{});
    return 1;
}

fn onContextInitialized(_: [*c]c.cef_browser_process_handler_t) callconv(.c) void {
    const api = loader.loaded() orelse return;
    const ctx = api.request_context_get_global_context();
    if (ctx == null) return;
    defer ref.releaseParam(ctx);
    writeStartupPrefs(ctx);
}

const StartupPrefsObj = ref.Counted(c.cef_request_context_handler_t, void);
var startup_prefs_handler: ?*StartupPrefsObj = null;

/// The handler every profile context is created with, so each Chrome profile
/// gets the same startup prefs as the global one.
fn startupPrefsHandler() [*c]c.cef_request_context_handler_t {
    if (startup_prefs_handler == null) {
        ensureAdblockBlock();
        ensureOriginFix();
        const h = StartupPrefsObj.create({}) orelse return null;
        h.cef.on_request_context_initialized = &onRequestContextInitialized;
        h.cef.get_resource_request_handler = &onContextGetResourceRequestHandler;
        startup_prefs_handler = h;
    }
    return startup_prefs_handler.?.handOut();
}

fn onRequestContextInitialized(_: [*c]c.cef_request_context_handler_t, ctx: [*c]c.cef_request_context_t) callconv(.c) void {
    defer ref.releaseParam(ctx);
    if (ctx != null) writeStartupPrefs(ctx);
}

/// Chrome restores the profile's last session into the first browser window
/// it tracks when `session.restore_on_startup` is 1 ("continue where you left
/// off"), and a profile this engine creates on CEF 151.3.23 carries 1 in its
/// Preferences. Every view is such a window, so after a kill the previous
/// run's pages came back as browsers Chrome built for itself. 5 is "open the new tab page",
/// which under an embedder that opens no startup window means nothing at all
/// (chrome/browser/prefs/session_startup_pref.h, kPrefValueNewTab). Written on
/// every context initialization, on the UI thread, before any view exists in
/// it.
fn writeStartupPrefs(ctx: [*c]c.cef_request_context_t) void {
    const api = loader.loaded() orelse return;
    const set = ctx.*.base.set_preference orelse return;
    const value = api.value_create();
    if (value == null) return;
    if (value.*.set_int) |set_int| _ = set_int(value, 5);
    var name = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&name);
    var err = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&err);
    if (!setStr(&name, "session.restore_on_startup")) return;
    // `value` is consumed by the call.
    if (set(&ctx.*.base, &name, value, &err) == 0) {
        const why = dupeStr(&err);
        defer if (why) |w| alloc.free(w);
        std.debug.print("ND_WARN CEF: session.restore_on_startup was not written: {?s}\n", .{why});
        return;
    }
    tr("startupPrefs session.restore_on_startup=5", .{});
    writeBoolPref(ctx, "download_bubble.partial_view_enabled", false);
    for (bubble_prefs) |key| writeBoolPref(ctx, key, false);
}

/// Features that raise a Chrome bubble from the page itself, anchored to a
/// location bar this embedding does not have: the password manager (save and
/// update password; password extensions fill without it), autofill saving
/// (save card, save address) and translate. BUBBLES.md has the full table.
const bubble_prefs = [_][]const u8{
    "credentials_enable_service",
    "credentials_enable_autosignin",
    "autofill.profile_enabled",
    "autofill.credit_card_enabled",
    "translate.enabled",
};

/// `download_bubble.partial_view_enabled`: Chrome style pops its download
/// bubble whenever a download it runs finishes, anchored to a toolbar this
/// embedding does not have; the app's own UI reports downloads instead.
fn writeBoolPref(ctx: [*c]c.cef_request_context_t, key: []const u8, on: bool) void {
    const api = loader.loaded() orelse return;
    const set = ctx.*.base.set_preference orelse return;
    const value = api.value_create();
    if (value == null) return;
    if (value.*.set_bool) |set_bool| _ = set_bool(value, @intFromBool(on));
    var name = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&name);
    var err = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&err);
    if (!setStr(&name, key)) return;
    if (set(&ctx.*.base, &name, value, &err) == 0) {
        const why = dupeStr(&err);
        defer if (why) |w| alloc.free(w);
        std.debug.print("ND_WARN CEF: {s} was not written: {?s}\n", .{ key, why });
        return;
    }
    tr("pref {s}={}", .{ key, on });
}

// ============================================================================
// Browsers Chrome creates on its own
// ============================================================================
//
// Chrome style answers `chrome.windows.create`, `chrome.tabs.create` and
// `chrome.runtime.openOptionsPage` by building a Chrome browser window, which
// goes through none of the hooks a renderer-initiated popup does: no
// on_before_popup, no on_open_urlfrom_tab, no command id. The one seam CEF
// leaves is get_default_client, called for exactly those browsers. The client
// it gets unmaps the window before it can be presented, hands the app the URL
// as `newWindow` (the same contract every other route uses) and closes it.

const SinkClientObj = ref.Counted(c.cef_client_t, void);
const SinkLifeObj = ref.Counted(c.cef_life_span_handler_t, void);
const SinkRequestObj = ref.Counted(c.cef_request_handler_t, void);
var sink_client: ?*SinkClientObj = null;
var sink_life: ?*SinkLifeObj = null;
var sink_request: ?*SinkRequestObj = null;

/// The view the app hears a Chrome-created browser about. Chrome's own window
/// belongs to no `<webview>`, and the app's tab-opening handler is per view, so
/// the one that last took focus is the one that stands in for "this window".
var focused_view: ?*View = null;

fn getDefaultClient(_: [*c]c.cef_browser_process_handler_t) callconv(.c) [*c]c.cef_client_t {
    if (!chromeStyle()) return null;
    if (sink_client == null) {
        const life = SinkLifeObj.create({}) orelse return null;
        life.cef.on_after_created = &onSinkBrowserCreated;
        life.cef.on_before_close = &onSinkBrowserClosed;
        const request = SinkRequestObj.create({}) orelse {
            life.drop();
            return null;
        };
        request.cef.on_before_browse = &onSinkBeforeBrowse;
        const client = SinkClientObj.create({}) orelse {
            life.drop();
            request.drop();
            return null;
        };
        client.cef.get_life_span_handler = &sinkGetLifeSpanHandler;
        client.cef.get_request_handler = &sinkGetRequestHandler;
        sink_life = life;
        sink_request = request;
        sink_client = client;
    }
    return sink_client.?.handOut();
}

fn sinkGetLifeSpanHandler(_: [*c]c.cef_client_t) callconv(.c) [*c]c.cef_life_span_handler_t {
    return (sink_life orelse return null).handOut();
}

fn sinkGetRequestHandler(_: [*c]c.cef_client_t) callconv(.c) [*c]c.cef_request_handler_t {
    return (sink_request orelse return null).handOut();
}

/// The one Chrome-created browser this engine does not close, and the timer
/// that keeps it off screen.
///
/// An extension install asks for a tabbed browser through
/// `ScopedTabbedBrowserDisplayer` and holds a raw `BrowserWindowInterface*` to
/// it across the asynchronous wait in `extensions::TriggerPostInstallDialog`,
/// so a browser closed inside on_after_created is dereferenced after it is
/// freed and the process dies the moment the install lands. Chrome never picks
/// one of this engine's browsers for that: CEF makes every BrowserView-hosted
/// browser a TYPE_POPUP (libcef chrome_browser_host_impl.cc), and the lookup
/// wants a tabbed one. So the first browser Chrome makes for itself is kept as
/// the tabbed browser every later lookup finds, and every one after it is
/// closed as before.
/// The top-level, not the browser's own window: CEF answers
/// `get_window_handle` with the window the web contents draw into, which for a
/// browser Chrome owns is a child of the Widget's frame, and unmapping the
/// child leaves the frame on screen. `ScopedTabbedBrowserDisplayer` also shows
/// the browser it picked every time it is used, so one unmap at creation does
/// not hold and the window watcher below unmaps it on every tick.
var kept_window: usize = 0;
/// The kept browser's host, held so the ordered shutdown can close it. Cleared
/// when CEF reports the browser closed.
var kept_host: ?[*c]c.cef_browser_host_t = null;
var kept_browser_id: c_int = 0;

fn onSinkBrowserCreated(_: [*c]c.cef_life_span_handler_t, browser: [*c]c.cef_browser_t) callconv(.c) void {
    defer ref.releaseParam(browser);
    if (browser == null) return;
    const get_host = browser.*.get_host orelse return;
    const host = get_host(browser);
    if (host == null) return;
    defer ref.releaseParam(host);

    // Unmapped before anything else: closing a browser is asynchronous and the
    // window would otherwise be on screen for the length of the teardown.
    var window: usize = 0;
    if (host.*.get_window_handle) |get_window| {
        const own: usize = @intCast(get_window(host));
        window = x11.toplevelOf(own);
        // A popup Chrome made on its own (`chrome.windows.create` with type
        // "popup", `chrome.identity.launchWebAuthFlow`) is a window in Chrome
        // too, and the extension waits on that browser: closing it fails the
        // auth flow at once.
        if (window != 0 and x11.inPopupRole(own)) {
            tr("sinkCreated popup window={x} left to Chrome", .{window});
            return;
        }
        if (window != 0) x11.hide(window);
    }
    // No window yet, so nothing says whether it is a popup: decided once Views
    // has made one (`onSinkTimer`).
    if (window == 0) {
        var browser_id: c_int = 0;
        if (browser.*.get_identifier) |get_id| browser_id = get_id(browser);
        const early = mainFrameUrl(browser);
        tr("sinkCreated id={d} no window yet url={?s}", .{ browser_id, early });
        ref.addRefParam(host);
        pending_sink.append(alloc, .{
            .id = browser_id,
            .host = host,
            .window = 0,
            .keep = false,
            .url = if (early) |u| (if (u.len != 0) u else blk: {
                alloc.free(u);
                break :blk null;
            }) else null,
            .deadline_us = glib.getMonotonicTime() + sink_url_wait_us,
        }) catch {
            ref.releaseParam(host);
            if (host.*.close_browser) |close| close(host, 1);
            return;
        };
        armSinkTimer();
        return;
    }
    const keep = kept_window == 0 and window != 0;
    if (keep) {
        kept_window = window;
        ref.addRefParam(host);
        kept_host = host;
        if (browser.*.get_identifier) |get_id| kept_browser_id = get_id(browser);
    }

    const url = mainFrameUrl(browser);
    tr("sinkCreated id={d} window={x} keep={} url={?s}", .{
        if (browser.*.get_identifier) |get_id| get_id(browser) else 0,
        window,
        keep,
        url,
    });
    // A browser Chrome has only just made has not started its navigation, so
    // its main frame's URL is empty. Reporting that as the new window's URL is
    // what handed the app a dead about:blank tab for every chrome.tabs.create
    // an extension makes; the real destination arrives in on_before_browse.
    if (url == null or url.?.len == 0) {
        if (url) |u| alloc.free(u);
        var browser_id: c_int = 0;
        if (browser.*.get_identifier) |get_id| browser_id = get_id(browser);
        ref.addRefParam(host);
        pending_sink.append(alloc, .{
            .id = browser_id,
            .host = host,
            .window = window,
            .keep = keep,
            .deadline_us = glib.getMonotonicTime() + sink_url_wait_us,
        }) catch {
            ref.releaseParam(host);
            if (!keep) {
                if (host.*.close_browser) |close| close(host, 1);
            }
            return;
        };
        armSinkTimer();
        return;
    }
    if (keep and newTabPage(url)) {
        if (url) |u| alloc.free(u);
        return;
    }
    reportSinkUrl(url);
    if (keep) return;
    if (host.*.close_browser) |close| close(host, 1);
}

/// A browser Chrome created whose first navigation this engine is still waiting
/// for. The window is already unmapped; the entry only exists so the URL the
/// app is told is the one the extension asked for.
const PendingSink = struct {
    id: c_int,
    host: [*c]c.cef_browser_host_t,
    /// Its top-level on the X server. Views shows the window it made again
    /// after `on_after_created` has unmapped it, exactly as it does for the
    /// kept browser, so the window watcher unmaps it on every tick instead.
    window: usize,
    keep: bool,
    /// Where it is going, when that was known before its window was.
    url: ?[]u8 = null,
    deadline_us: i64,
};

/// How long a Chrome-created browser may go without starting a navigation
/// before it is closed. Nothing is reported for it: a window whose destination
/// never existed is a tab the app could only open on about:blank, which is
/// exactly the dead tab this path exists to stop producing.
const sink_url_wait_us: i64 = 1500 * std.time.us_per_ms;
const sink_timer_interval_ms: c_uint = 250;

var pending_sink: std.ArrayList(PendingSink) = .empty;
var sink_timer: c_uint = 0;

fn armSinkTimer() void {
    if (sink_timer != 0) return;
    sink_timer = glib.timeoutAdd(sink_timer_interval_ms, &onSinkTimer, null);
}

fn onSinkTimer(_: ?*anyopaque) callconv(.c) c_int {
    const now = glib.getMonotonicTime();
    var i: usize = 0;
    while (i < pending_sink.items.len) {
        const entry = &pending_sink.items[i];
        if (entry.window == 0) {
            if (sinkWindow(entry.host)) |found| {
                const window = found.top;
                if (found.popup) {
                    tr("sinkPending id={d} popup window={x} left to Chrome", .{ entry.id, window });
                    const gone = pending_sink.orderedRemove(i);
                    if (gone.url) |u| alloc.free(u);
                    ref.releaseParam(gone.host);
                    continue;
                }
                x11.hide(window);
                entry.window = window;
                tr("sinkPending id={d} window={x} kept={x}", .{ entry.id, window, kept_window });
                if (entry.url != null) {
                    const done = pending_sink.orderedRemove(i);
                    reportSinkUrl(done.url);
                    if (done.host.*.close_browser) |close| close(done.host, 1);
                    ref.releaseParam(done.host);
                    continue;
                }
            }
        }
        if (entry.deadline_us > now) {
            i += 1;
            continue;
        }
        const done = pending_sink.orderedRemove(i);
        if (done.url != null) reportSinkUrl(done.url);
        if (!done.keep) {
            if (done.host.*.close_browser) |close| close(done.host, 1);
        }
        ref.releaseParam(done.host);
    }
    if (pending_sink.items.len != 0) return 1;
    sink_timer = 0;
    return 0;
}

const SinkWindow = struct { top: usize, popup: bool };

fn sinkWindow(host: [*c]c.cef_browser_host_t) ?SinkWindow {
    const get_window = host.*.get_window_handle orelse return null;
    const own: usize = @intCast(get_window(host));
    const top = x11.toplevelOf(own);
    if (top == 0) return null;
    return .{ .top = top, .popup = x11.inPopupRole(own) };
}

fn takePendingSink(browser_id: c_int) ?PendingSink {
    for (pending_sink.items, 0..) |entry, i| {
        if (entry.id != browser_id) continue;
        return pending_sink.orderedRemove(i);
    }
    return null;
}

/// Hands the app the URL as `newWindow`, the contract every other
/// window-opening route uses. `url` is adopted.
fn reportSinkUrl(url: ?[]u8) void {
    // The view on show, the one a user reads as "this window": an app keeps
    // hidden views too (an extension registry a few pixels across, a
    // background tab), and the first browser created, which is often such a
    // view, was also the first focus record, so the tab went where nothing
    // listens for it. Any live view after that rather than none: a view is
    // destroyed with the window that held it and takes the focus record with
    // it, and a tab an extension asked for must not be dropped because nothing
    // has taken focus since.
    const target = if (focused_view) |v| (if (viewOnShow(v)) v else null) else null;
    const chosen = target orelse anyShownView() orelse focused_view orelse anyLiveView();
    if (chosen) |view| {
        tr("sinkNewWindow node={d} url={?s}", .{ view.node_id, url });
        // A tab Chrome made on its own (an extension's chrome.tabs.create, the
        // page an extension opens on install) arrives here with no
        // disposition; `active` defaults to true, and that is the tab Chrome
        // shows, at the end of the strip rather than next to any page.
        post(.{ .view = view, .name = "newWindow", .text = url, .extra = alloc.dupe(u8, "foregroundTab") catch null, .unrelated = true });
        return;
    }
    tr("sinkNewWindow dropped url={?s}", .{url});
    if (url) |u| alloc.free(u);
}

/// Mapped and big enough to show a page: a view a few pixels across is kept
/// only for its browser.
fn viewOnShow(view: *View) bool {
    if (view.container == 0 or gtk.Widget.getMapped(view.widget) == 0) return false;
    return view.size_w.load(.acquire) >= 10 and view.size_h.load(.acquire) >= 10;
}

fn anyShownView() ?*View {
    var it = live_views.keyIterator();
    while (it.next()) |key| {
        const view: *View = @ptrFromInt(key.*);
        if (viewOnShow(view)) return view;
    }
    return null;
}

fn anyLiveView() ?*View {
    var it = live_views.keyIterator();
    while (it.next()) |key| {
        const view: *View = @ptrFromInt(key.*);
        if (view.container != 0) return view;
    }
    return null;
}

/// True while the window is showing a dialog of its own: an AdwDialog
/// presented into it, which is drawn by the toplevel's surface. The X server
/// stacks a child window above everything its parent draws, so over a page the
/// dialog and its scrim are painted, reported as presented, and never seen.
/// Read from GTK rather than counted as dialogs come and go: a count that
/// leaks leaves a window with no page in it.
fn windowHasVisibleDialog(root: *gtk.Root) bool {
    const widget: *gtk.Widget = @ptrCast(@alignCast(root));
    if (gobject.ext.cast(adw.ApplicationWindow, widget)) |window| {
        return adw.ApplicationWindow.getVisibleDialog(window) != null;
    }
    if (gobject.ext.cast(adw.Window, widget)) |window| {
        return adw.Window.getVisibleDialog(window) != null;
    }
    return false;
}

fn dialogOverView(view: *View) bool {
    const root = gtk.Widget.getRoot(view.widget) orelse return false;
    return windowHasVisibleDialog(root);
}

/// Where a page waits out a dialog: the place a hidden tab's window waits,
/// which keeps it mapped and running. Parked rather than merely moved: an X
/// window that comes back from behind its parent's edge has no contents to
/// show and Chromium presents a new frame only when its window changes size,
/// so a page that was moved back and not resized returned as a blank rectangle.
///
/// Read off the page's window before it goes: the dialog's scrim is meant to
/// dim the page, and with the window parked there is nothing under it but the
/// window's background.
fn standAside(view: *View) void {
    if (view.container == 0) return;
    if (view.aside_still == null and !view.moving and view.placed) {
        const page = view.cef_window.load(.acquire);
        if (x11.capture(if (page != 0) @intCast(page) else view.container)) |still| {
            view.aside_still = still;
            gtk.Picture.setPaintable(@ptrCast(@alignCast(view.widget)), @ptrCast(still));
        }
    }
    view.aside = true;
    parkContainer(view);
}

fn dropAsideStill(view: *View) void {
    const still = view.aside_still orelse return;
    view.aside_still = null;
    if (!view.moving) gtk.Picture.setPaintable(@ptrCast(@alignCast(view.widget)), null);
    gobject.Object.unref(@ptrCast(@alignCast(still)));
}

/// Re-reads the dialog state of `widget`'s window and moves the pages there out
/// of the way or back. Called by the GTK side as a dialog is presented and
/// again when it closes; `onEngineTick` is the backstop for a dialog that went
/// away without one.
pub fn refreshDialogOcclusion(widget: *gtk.Widget) void {
    const root = gtk.Widget.getRoot(widget) orelse return;
    var it = live_views.keyIterator();
    while (it.next()) |key| {
        const view: *View = @ptrFromInt(key.*);
        if (view.container == 0) continue;
        if (gtk.Widget.getMapped(view.widget) == 0) continue;
        const view_root = gtk.Widget.getRoot(view.widget) orelse continue;
        if (view_root != root) continue;
        syncBounds(view);
    }
}

fn mainFrameUrl(browser: [*c]c.cef_browser_t) ?[]u8 {
    const get_frame = browser.*.get_main_frame orelse return null;
    const frame = get_frame(browser);
    if (frame == null) return null;
    defer ref.releaseParam(frame);
    const get_url = frame.*.get_url orelse return null;
    const raw = get_url(frame);
    if (raw == null) return null;
    defer freeUserfree(raw);
    return dupeStr(raw);
}

/// The first navigation of a Chrome-created browser, which is where its real
/// destination finally exists. The browser is closed here rather than in
/// on_after_created, so nothing of it is ever fetched or drawn.
fn onSinkBeforeBrowse(
    _: [*c]c.cef_request_handler_t,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    request: [*c]c.cef_request_t,
    _: c_int,
    _: c_int,
) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(frame);
    defer ref.releaseParam(request);
    if (browser == null) return 0;
    var browser_id: c_int = 0;
    if (browser.*.get_identifier) |get_id| browser_id = get_id(browser);
    const entry = takePendingSink(browser_id) orelse {
        tr("sinkBrowse id={d} pending=false", .{browser_id});
        return 0;
    };
    defer ref.releaseParam(entry.host);
    if (entry.url) |u| alloc.free(u);
    if (entry.window == 0) {
        if (sinkWindow(entry.host)) |found| {
            if (found.popup) {
                tr("sinkBrowse id={d} popup window={x} left to Chrome", .{ browser_id, found.top });
                return 0;
            }
        }
    }

    var url: ?[]u8 = null;
    if (request != null) {
        if (request.*.get_url) |get_url| {
            const raw = get_url(request);
            if (raw != null) {
                defer freeUserfree(raw);
                url = dupeStr(raw);
            }
        }
    }
    if (entry.keep and newTabPage(url)) {
        tr("sinkBrowse id={d} kept browser's own new tab page, not reported", .{browser_id});
        if (url) |u| alloc.free(u);
        return 0;
    }
    reportSinkUrl(url);
    if (entry.keep) return 0;
    if (entry.host.*.close_browser) |close| close(entry.host, 1);
    return 1;
}

/// The page Chrome opens in a tabbed browser it builds for itself. After an
/// extension install that is the browser the post-install dialog asks for,
/// which no extension and no page asked for, and reporting it gave the app a
/// stray new tab after every first install.
fn newTabPage(url: ?[]const u8) bool {
    const u = url orelse return false;
    return std.mem.startsWith(u8, u, "chrome://new-tab-page") or std.mem.startsWith(u8, u, "chrome://newtab");
}

// ============================================================================
// Windows Chrome puts up that belong to no browser
// ============================================================================
//
// The extension install prompt and the post-install dialog are Views widgets
// rather than browsers, so no CEF callback is consulted about them: they arrive
// as top-level windows on the X server and nowhere else. Chrome style watches
// the root for top-levels that carry this process's `_NET_WM_PID`, are not one
// of GTK's own windows and are not the kept browser, moves each one over the
// view that has focus and reports it to the app as `chromeDialog`. Chrome's own
// dialog is still Chrome's, but it is drawn on the app's content instead of
// wherever Views decided to put it.

const chrome_watch_interval_ms: c_uint = 200;
/// Chromium's parked scaffolding (the clipboard owner, the drag proxy, the
/// omnibox popup host) is 1x1, 10x10 or 44x55; a real dialog or bubble is
/// bigger than all of them. The floor used to be 200x80, which dropped every
/// bubble narrower than a browser window.
const chrome_dialog_min_w: c_uint = 60;
const chrome_dialog_min_h: c_uint = 30;
var chrome_watch_timer: c_uint = 0;
var chrome_watch_truncation_warned = false;
var adopted_windows: std.AutoHashMapUnmanaged(usize, void) = .empty;
/// Windows already stamped with the app's class, so the stamp costs one X
/// request per window rather than one per tick.
var named_windows: std.AutoHashMapUnmanaged(usize, void) = .empty;
/// Picture-in-picture windows already kept above under Hyprland.
var pinned_windows: std.AutoHashMapUnmanaged(usize, void) = .empty;
/// Picture-in-picture windows on screen, with the view a document window
/// belongs to (a floating video's tab is named by no callback).
const PipWindow = struct { view: ?*View };
var pip_windows: std.AutoHashMapUnmanaged(usize, PipWindow) = .empty;
/// A document window was asked for (on_before_popup) and Chromium has yet to
/// map it: the next top-level of this process is that window. Chromium builds
/// it as a browser of its own with no client of ours, so nothing else says
/// which window it is. Set on the CEF UI thread itself rather than through the
/// GTK hop: the window can map before an idle callback runs, and the watch
/// would then take it for one of Chromium's dialogs.
var pending_doc_pip_view: std.atomic.Value(usize) = .init(0);
var pending_doc_pip_until: std.atomic.Value(i64) = .init(0);
/// Windows already looked at and found not to be picture-in-picture: a window
/// does not become one later (the floating video's aspect hint is there before
/// it maps, a document window is new), so each costs its X round trips once
/// rather than on every tick of the watch.
var not_pip_windows: std.AutoHashMapUnmanaged(usize, void) = .empty;
const pending_doc_pip_us: i64 = 5 * std.time.us_per_s;
var self_pid: u32 = 0;

fn startChromeWindowWatch() void {
    if (!chromeStyle() or chrome_watch_timer != 0) return;
    self_pid = @intCast(std.c.getpid());
    chrome_watch_timer = glib.timeoutAdd(chrome_watch_interval_ms, &onChromeWindowWatch, null);
}

fn isPendingSinkWindow(window: usize) bool {
    if (window == 0) return false;
    for (pending_sink.items) |entry| {
        if (entry.window == window) return true;
    }
    return false;
}

/// Chrome's download-started animation: an arrow in a small top-level of its
/// own, mapped as a download begins, which the watch below would otherwise
/// move onto the view as a dialog. No feature, switch or pref in CEF 151 turns
/// it off (the partial-view pref covers the bubble only). For a few seconds
/// after the app accepts a download, every top-level this process maps or
/// resizes is checked from GDK's `xevent` as the event is read (Views maps the
/// window at 1x1 and sizes it after), and one of that shape is unmapped there,
/// before the next frame.
const download_watch_us: i64 = 5 * std.time.us_per_s;
const download_animation_max: c_uint = 96;
var download_watch_until_us: i64 = 0;
var download_map_hook = false;

fn watchDownloadAnimation() void {
    if (!chromeStyle()) return;
    download_watch_until_us = glib.getMonotonicTime() + download_watch_us;
    hookRootMaps();
}

/// Every top-level this process maps or resizes, seen as GDK reads the event.
fn hookRootMaps() void {
    if (download_map_hook) return;
    const display = x11.gdkDisplay() orelse return;
    if (!x11.watchRootMaps()) return;
    _ = gobject.signalConnectData(display.as(gobject.Object), "xevent", @ptrCast(&onXEvent), null, null, .{});
    download_map_hook = true;
}

fn onXEvent(_: *gdk.Display, xevent: *const anyopaque, _: ?*anyopaque) callconv(.c) c_int {
    const w = x11.mappedWindow(xevent);
    if (w != 0 and !claimInstallPrompt(w)) hideDownloadAnimation(w);
    return 0;
}

// ============================================================================
// Chrome's install prompt, answered unseen
// ============================================================================
//
// An app that confirms a Web Store install in a dialog of its own lets the
// store page go on and arms this (`acceptExtensionInstall`); Chromium then
// raises its own "Add <name>?" prompt, a Views top-level no CEF callback is
// asked about and no switch or policy skips. The first top-level of this
// process big enough to be a dialog that maps while armed is taken as the
// prompt: it stays mapped, which Views needs before it enables the accept
// button, with an empty shape so nothing of it is drawn. Views takes no key
// sent with XSendEvent and none in a widget it does not believe active, so the
// prompt is activated through the window manager and answered with XTest:
// Tab from the Cancel button it focuses first to "Add extension", then Space.
// A prompt still up a few seconds later is shaped back and put over the view
// for the user to answer.

const install_arm_us: i64 = 20 * std.time.us_per_s;
const install_first_press_ms: c_uint = 1200;
const install_settle_ms: c_uint = 250;
const install_check_ms: c_uint = 200;
const install_wait_us: i64 = 4 * std.time.us_per_s;
const install_min_w: c_uint = 200;
const install_min_h: c_uint = 80;
var install_armed_until_us: i64 = 0;
var install_window: usize = 0;
var install_deadline_us: i64 = 0;
/// The prompt has been handed back to the user.
var install_shown = false;

fn cmdAcceptExtensionInstall() void {
    if (!chromeStyle()) return;
    install_armed_until_us = glib.getMonotonicTime() + install_arm_us;
    hookRootMaps();
    tr("installPrompt armed", .{});
}

/// Takes `w` as the prompt when an install is armed; true when it did.
fn claimInstallPrompt(w: usize) bool {
    if (install_window != 0 or glib.getMonotonicTime() >= install_armed_until_us) return false;
    if (w == 0 or w == kept_window or isPendingSinkWindow(w)) return false;
    if (x11.isGdkSurface(w) or x11.windowPid(w) != self_pid or x11.isOverrideRedirect(w)) return false;
    const geo = x11.geometry(w) orelse return false;
    if (geo.w < install_min_w or geo.h < install_min_h) return false;
    install_armed_until_us = 0;
    install_window = w;
    install_shown = false;
    adopted_windows.put(alloc, w, {}) catch {};
    x11.setShape(w, &.{});
    tr("installPrompt claimed window={x} {d}x{d}", .{ w, geo.w, geo.h });
    _ = glib.timeoutAdd(install_first_press_ms, &onInstallActivate, null);
    _ = glib.timeoutAdd(install_shape_ms, &onInstallShape, null);
    return true;
}

/// Views gives its dialogs a shape of its own (the rounded frame) whenever it
/// lays one out, which undoes the empty one; it is put back until the prompt
/// is gone.
const install_shape_ms: c_uint = 40;

fn onInstallShape(_: ?*anyopaque) callconv(.c) c_int {
    const w = install_window;
    if (w == 0 or install_shown) return 0;
    x11.setShape(w, &.{});
    return 1;
}

fn installPromptGone(w: usize) bool {
    return x11.geometry(w) == null or !x11.viewable(w);
}

fn onInstallActivate(_: ?*anyopaque) callconv(.c) c_int {
    const w = install_window;
    if (w == 0) return 0;
    if (installPromptGone(w)) return finishInstallPrompt("answered");
    x11.activate(w);
    _ = glib.timeoutAdd(install_settle_ms, &onInstallPress, null);
    return 0;
}

fn onInstallPress(_: ?*anyopaque) callconv(.c) c_int {
    const w = install_window;
    if (w == 0) return 0;
    // Under XWayland the first XTest key, the one that switches the master
    // keyboard over from the compositor's slave, never reaches the prompt:
    // that was the Tab, so Space pressed Cancel. A Shift press, which no
    // dialog acts on, goes first and is the one lost.
    if (!x11.pressKey(x11.keysym_shift) or !x11.pressKey(x11.keysym_tab) or !x11.pressKey(x11.keysym_space)) return finishInstallPrompt("unanswered, no XTest");
    install_deadline_us = glib.getMonotonicTime() + install_wait_us;
    _ = glib.timeoutAdd(install_check_ms, &onInstallCheck, null);
    return 0;
}

fn onInstallCheck(_: ?*anyopaque) callconv(.c) c_int {
    const w = install_window;
    if (w == 0) return 0;
    if (installPromptGone(w)) return finishInstallPrompt("answered");
    if (glib.getMonotonicTime() < install_deadline_us) return 1;
    return finishInstallPrompt("unanswered");
}

/// The prompt is gone, or is handed back to the user.
fn finishInstallPrompt(how: []const u8) c_int {
    const w = install_window;
    install_window = 0;
    tr("installPrompt {s} window={x}", .{ how, w });
    if (std.mem.eql(u8, how, "answered")) {
        // Activating the prompt took the app's window out of focus.
        x11.activate(appToplevel());
        return 0;
    }
    install_shown = true;
    x11.clearShape(w);
    adoptChromeWindow(w);
    return 0;
}

fn hideDownloadAnimation(w: usize) void {
    if (glib.getMonotonicTime() >= download_watch_until_us) return;
    if (w == 0 or w == kept_window or isPendingSinkWindow(w)) return;
    if (x11.isGdkSurface(w)) return;
    if (x11.windowPid(w) != self_pid) return;
    const geo = x11.geometry(w) orelse return;
    if (geo.w > download_animation_max or geo.h > download_animation_max or geo.w < chrome_dialog_min_w) return;
    if (!adopted_windows.contains(w)) {
        adopted_windows.put(alloc, w, {}) catch {};
        tr("download animation kept off screen window={x} {d}x{d}", .{ w, geo.w, geo.h });
    }
    x11.hide(w);
}

/// Backstop for a MapNotify the hook missed.
fn hideDownloadAnimations() void {
    if (glib.getMonotonicTime() >= download_watch_until_us) return;
    var buf: [1024]x11.Window = undefined;
    var truncated = false;
    for (x11.rootChildren(&buf, &truncated)) |w| hideDownloadAnimation(w);
}

fn onChromeWindowWatch(_: ?*anyopaque) callconv(.c) c_int {
    x11.publishClass(appToplevel());
    if (kept_window != 0) x11.hide(kept_window);
    hideDownloadAnimations();
    for (pending_sink.items) |entry| {
        if (entry.window != 0) x11.hide(entry.window);
    }
    // Sized for a real session rather than for a gate's bare X server: the root
    // on the owner's desktop carries every X client on it.
    var buf: [1024]x11.Window = undefined;
    var truncated = false;
    const children = x11.rootChildren(&buf, &truncated);
    if (truncated and !chrome_watch_truncation_warned) {
        chrome_watch_truncation_warned = true;
        std.debug.print("ND_WARN WebView engine=chromium: more than {d} windows on the root; a Chrome dialog can be missed\n", .{buf.len});
    }
    // The X server reuses window ids, so a dialog that is gone has to be
    // forgotten or the next window to land on its id is never adopted.
    var gone: std.ArrayList(usize) = .empty;
    defer gone.deinit(alloc);
    var it = adopted_windows.keyIterator();
    while (it.next()) |key| {
        if (std.mem.indexOfScalar(x11.Window, children, key.*) == null) gone.append(alloc, key.*) catch {};
    }
    for (gone.items) |w| _ = adopted_windows.remove(w);
    // A reparenting window manager frames a client before this ever looks at
    // the root, and the frame is what the root lists; the WM's own client list
    // still names the window inside it.
    var client_buf: [1024]x11.Window = undefined;
    const clients = x11.clientList(&client_buf);
    gone.clearRetainingCapacity();
    var named_it = named_windows.keyIterator();
    while (named_it.next()) |key| {
        if (std.mem.indexOfScalar(x11.Window, children, key.*) == null and
            std.mem.indexOfScalar(x11.Window, clients, key.*) == null) gone.append(alloc, key.*) catch {};
    }
    for (gone.items) |w| _ = named_windows.remove(w);
    gone.clearRetainingCapacity();
    var pinned_it = pinned_windows.keyIterator();
    while (pinned_it.next()) |key| {
        if (std.mem.indexOfScalar(x11.Window, children, key.*) == null) gone.append(alloc, key.*) catch {};
    }
    for (gone.items) |w| _ = pinned_windows.remove(w);
    forgetGonePictureInPicture(children, clients);
    for (clients) |w| {
        if (w == 0) continue;
        if (!adopted_windows.contains(w)) _ = claimInstallPrompt(w);
        if (named_windows.contains(w)) {
            _ = pictureInPicture(w);
            continue;
        }
        if (x11.windowPid(w) != self_pid or x11.isGdkSurface(w)) continue;
        if (x11.copyClass(appToplevel(), w)) named_windows.put(alloc, w, {}) catch {};
        _ = pictureInPicture(w);
    }
    for (children) |w| {
        if (w == 0 or w == kept_window) continue;
        // A browser this engine is still waiting on a URL for is not a dialog
        // Chrome drew; adopting one would move it onto the view and leave it
        // there, which is the stray window the whole sink exists to prevent.
        if (isPendingSinkWindow(w)) continue;
        if (adopted_windows.contains(w)) continue;
        if (claimInstallPrompt(w)) continue;
        if (x11.windowPid(w) != self_pid) continue;
        // GDK knows every window it made, which is the app's toplevels and the
        // override-redirect ones a popover or the page's GTK context menu puts
        // on the root; moving one of those would be a good deal worse than
        // leaving a Chrome dialog where Views put it.
        if (x11.isGdkSurface(w)) continue;
        // Whatever Chromium put up, the compositor is showing it as a window of
        // this app: it gets the app's class, so nothing enumerating windows
        // offers it as an untitled one of its own. Cheaper than the move below
        // and done for every one of them, the small parked ones included.
        if (!named_windows.contains(w)) {
            if (x11.copyClass(appToplevel(), w)) named_windows.put(alloc, w, {}) catch {};
        }
        // A popup Chromium draws for the page (a <select> list, autofill, the
        // date picker) is placed against its control by Chromium itself.
        // Centred on the view instead, a list opened by a click closed again
        // before it could be used, and one opened from the keyboard sat in
        // the middle of the page.
        if (x11.isOverrideRedirect(w)) continue;
        // The picture-in-picture window is the one Chromium keeps above every
        // other: it belongs in the screen's corner, above other apps, and a
        // dialog's treatment (transient for the app, centred on the page)
        // would tie it to this window instead.
        if (pictureInPicture(w)) continue;
        // Kept above, as before, but left as Chromium named it.
        if (x11.pictureInPicture(w)) {
            keepAbove(w);
            continue;
        }
        // A popup window is placed where the page asked for it, as in Chrome,
        // not centred on the view like a dialog.
        if (x11.isPopupRole(w)) {
            adopted_windows.put(alloc, w, {}) catch {};
            continue;
        }
        // A compositor that manages XWayland top-levels itself (Hyprland does)
        // places them by its own rules and discards the ConfigureRequest the
        // move below sends, so the hints go on before anything else: a dialog
        // marked transient for the host window is floated and centred on it
        // rather than tiled into whatever corner a new top-level gets.
        if (anchorView()) |anchor| x11.markDialogFor(w, x11.toplevelXid(anchor.view.widget));
        // Chromium keeps a handful of small parked top-levels of its own (the
        // omnibox popup host, the drag proxy) that are never presented; moving
        // one onto the view would drag scaffolding into the page.
        const geo = x11.geometry(w) orelse continue;
        if (geo.w < chrome_dialog_min_w or geo.h < chrome_dialog_min_h) continue;
        adoptChromeWindow(w);
    }
    return 1;
}

/// A floating video or a document window: given what a window manager rule
/// can match on (x11.markPictureInPicture) once, its fixed title back whenever
/// Chromium renames it, and kept above. Answers whether `w` is one, or may
/// still turn out to be (the watch leaves it alone this tick).
fn pictureInPicture(w: usize) bool {
    if (pip_windows.contains(w)) {
        _ = x11.holdPictureInPictureTitle(w);
        keepAbove(w);
        return true;
    }
    if (not_pip_windows.contains(w)) return false;
    if (x11.windowPid(w) != self_pid or x11.isGdkSurface(w) or x11.isOverrideRedirect(w)) {
        not_pip_windows.put(alloc, w, {}) catch {};
        return false;
    }
    var view: ?*View = null;
    const pending = pending_doc_pip_view.load(.acquire);
    if (pending != 0) {
        const geo = x11.geometry(w);
        if (glib.getMonotonicTime() > pending_doc_pip_until.load(.acquire) or !live_views.contains(pending)) {
            pending_doc_pip_view.store(0, .release);
        } else if (geo != null and geo.?.w >= 100 and geo.?.h >= 60) {
            view = @ptrFromInt(pending);
            pending_doc_pip_view.store(0, .release);
        } else {
            // Views maps a window at 1x1 and sizes it after: until it has its
            // size it is neither decided nor anyone else's to adopt.
            return true;
        }
    }
    // Only the two signals that name the window outright are taken: a window
    // the window manager put above every other (x11.pictureInPicture's wider
    // test) may be one of Chromium's dialogs, and renaming that would hide it.
    if (view == null and !x11.keepsAspect(w)) {
        not_pip_windows.put(alloc, w, {}) catch {};
        return false;
    }
    pip_windows.put(alloc, w, .{ .view = view }) catch return false;
    named_windows.put(alloc, w, {}) catch {};
    x11.markPictureInPicture(w, appToplevel());
    pip_landing.track(w, view == null);
    tr("picture in picture window={x} kind={s}", .{ w, if (view != null) "document" else "video" });
    if (view) |v| post(.{ .view = v, .name = "pictureInPicture", .text = alloc.dupe(u8, "opened") catch null, .extra = alloc.dupe(u8, "document") catch null });
    keepAbove(w);
    return true;
}

fn forgetGonePictureInPicture(children: []const x11.Window, clients: []const x11.Window) void {
    var gone: std.ArrayList(usize) = .empty;
    defer gone.deinit(alloc);
    // The X server reuses window ids.
    var seen = not_pip_windows.keyIterator();
    while (seen.next()) |key| {
        if (std.mem.indexOfScalar(x11.Window, children, key.*) == null and std.mem.indexOfScalar(x11.Window, clients, key.*) == null) gone.append(alloc, key.*) catch {};
    }
    for (gone.items) |w| _ = not_pip_windows.remove(w);
    gone.clearRetainingCapacity();
    var it = pip_windows.iterator();
    while (it.next()) |entry| {
        const w = entry.key_ptr.*;
        if (std.mem.indexOfScalar(x11.Window, children, w) == null and std.mem.indexOfScalar(x11.Window, clients, w) == null) gone.append(alloc, w) catch {};
    }
    for (gone.items) |w| {
        const entry = pip_windows.fetchRemove(w) orelse continue;
        pip_landing.forget(w);
        tr("picture in picture window={x} gone", .{w});
        const view = entry.value.view orelse continue;
        // The opener's tab may be what closed it.
        if (!live_views.contains(@intFromPtr(view))) continue;
        post(.{ .view = view, .name = "pictureInPicture", .text = alloc.dupe(u8, "closed") catch null, .extra = alloc.dupe(u8, "document") catch null });
    }
}

/// Under Hyprland, which reads no keep-above hint from an XWayland window, the
/// picture-in-picture window is pinned instead; tried on every tick until
/// Hyprland lists it.
fn keepAbove(window: usize) void {
    if (pinned_windows.contains(window)) return;
    if (!hyprland.running()) {
        pinned_windows.put(alloc, window, {}) catch {};
        return;
    }
    var buf: [256]u8 = undefined;
    const title = x11.windowName(window, &buf);
    if (title.len == 0) return;
    switch (hyprland.pinWindow(self_pid, title)) {
        .pinned, .unavailable => pinned_windows.put(alloc, window, {}) catch {},
        .not_yet => {},
    }
}

/// The view a Chrome dialog is drawn over. A view that is not mapped sits at
/// the park origin far off-screen (see `parkContainer`), and centring a dialog
/// on one of those would hide it rather than place it, so an on-screen view
/// wins over the focused one.
/// The app's toplevel, where the class Chromium's windows borrow comes from.
/// Unlike `anchorView` it does not ask for a view on screen: a window dragged
/// partly past the screen's left edge is still the app's.
fn appToplevel() x11.Window {
    if (focused_view) |view| {
        const xid = x11.toplevelXid(view.widget);
        if (xid != 0) return xid;
    }
    var it = live_views.keyIterator();
    while (it.next()) |key| {
        const view: *View = @ptrFromInt(key.*);
        const xid = x11.toplevelXid(view.widget);
        if (xid != 0) return xid;
    }
    return 0;
}

fn anchorView() ?struct { view: *View, origin: x11.Origin } {
    if (focused_view) |view| {
        const origin = x11.originOnRoot(view.container);
        if (origin.x >= 0 and origin.y >= 0) return .{ .view = view, .origin = origin };
    }
    var it = live_views.keyIterator();
    while (it.next()) |key| {
        const view: *View = @ptrFromInt(key.*);
        if (view.container == 0) continue;
        const origin = x11.originOnRoot(view.container);
        if (origin.x >= 0 and origin.y >= 0 and view.size_w.load(.acquire) > 0) {
            return .{ .view = view, .origin = origin };
        }
    }
    return null;
}

/// Centred on the view that owns the dialog, in root coordinates. The dialog
/// stays a top-level where Chromium thinks it is: reparenting it into the
/// host's window puts it on screen in the right place but leaves Chromium
/// hit-testing clicks against the coordinates it had before, and its buttons
/// stop working (measured: the gate's uninstall confirmation stops answering).
/// The move is sent once. A compositor that owns XWayland placement puts the
/// window back within the frame, and re-sending the move on every tick only
/// trades positions with it; see docs/webview.md.
fn chromeDialogSpot(origin: x11.Origin, vw: c_int, vh: c_int, geo: x11.Geometry) struct { x: c_int, y: c_int } {
    const gw: c_int = @intCast(geo.w);
    const gh: c_int = @intCast(geo.h);
    var x = origin.x;
    var y = origin.y;
    if (vw > gw) x += @divTrunc(vw - gw, 2);
    if (vh > gh) y += @divTrunc(vh - gh, 2);
    return .{ .x = x, .y = y };
}

fn adoptChromeWindow(window: usize) void {
    adopted_windows.put(alloc, window, {}) catch return;
    const anchor = anchorView() orelse return;
    const view = anchor.view;
    const geo = x11.geometry(window) orelse return;
    const vw: c_int = @intCast(view.size_w.load(.acquire));
    const vh: c_int = @intCast(view.size_h.load(.acquire));
    const spot = chromeDialogSpot(anchor.origin, vw, vh, geo);
    x11.moveResize(window, spot.x, spot.y, geo.w, geo.h);
    tr("chromeDialog node={d} window={x} {d}x{d} at {d},{d}", .{ view.node_id, window, geo.w, geo.h, spot.x, spot.y });

    const f = emit orelse return;
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    payload.put(alloc, "x", .{ .integer = spot.x }) catch return;
    payload.put(alloc, "y", .{ .integer = spot.y }) catch return;
    payload.put(alloc, "width", .{ .integer = geo.w }) catch return;
    payload.put(alloc, "height", .{ .integer = geo.h }) catch return;
    f(view.node_id, "chromeDialog", .{ .data = .{ .object = payload } });
}

/// Every browser Chrome made for itself reports here, including the ones this
/// engine closed on the spot, so the kept one is told apart by its identifier.
fn onSinkBrowserClosed(_: [*c]c.cef_life_span_handler_t, browser: [*c]c.cef_browser_t) callconv(.c) void {
    defer ref.releaseParam(browser);
    const get_id = browser.*.get_identifier orelse return;
    if (takePendingSink(get_id(browser))) |entry| {
        if (entry.url) |u| alloc.free(u);
        ref.releaseParam(entry.host);
    }
    const host = kept_host orelse return;
    if (get_id(browser) != kept_browser_id) return;
    kept_host = null;
    kept_window = 0;
    ref.releaseParam(host);
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
    process_type: [*c]const c.cef_string_t,
    command_line: [*c]c.cef_command_line_t,
) callconv(.c) void {
    defer ref.releaseParam(command_line);
    if (process_type != null and process_type.*.length > 0) ignoreHostQuitSignals();
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
        // on_before_popup ever runs, and reports it only to an omnibox this
        // engine does not have. The same decision is made in on_before_popup
        // instead (`popupsAllowed`), where the app can be told.
        appendFlag(command_line, append, "disable-popup-blocking");
        // Read by StartupBrowserCreator, which CEF skips at startup and a
        // refused relaunch never reaches; this covers any other route into it.
        // Not --no-startup-window: it holds a keep-alive, and CefShutdown then
        // waits forever for a UI thread that never quits.
        appendFlag(command_line, append, "hide-crash-restore-bubble");
        if (browser_pipe.available()) appendFlag(command_line, append, "remote-debugging-pipe");
    }
    // The pipe alone turns on Blink's AutomationControlled, which is what sets
    // navigator.webdriver, and sites such as Google Search then answer with a
    // bot check. The pipe is the framework's own channel, not automation.
    if (browser_pipe.available()) appendJoined(command_line, "disable-blink-features", "AutomationControlled");
    if (chromeStyle()) if (framework_extension_dir) |dir| appendJoined(command_line, "load-extension", dir);
    if (command_line.*.append_switch) |append| {
        // Domain Reliability uploads network error samples to Google; nothing
        // in an embedded browser reads them.
        appendFlag(command_line, append, "disable-domain-reliability");
    }
    // VA-API decode through the GL path, which Chromium leaves off on Linux.
    // Without libva or a driver for the GPU the decoder stays in software.
    appendJoined(command_line, "enable-features", "AcceleratedVideoDecodeLinuxGL,AcceleratedVideoDecodeLinuxZeroCopyGL");
    // Background services with no surface in an embedded browser: Cast device
    // discovery, Google's page hints and autofill form signatures, and
    // Translate, whose language detection otherwise runs on every page load.
    // The omnibox popups as WebUI preload two pages for every browser, and
    // every view here is a browser whose omnibox nobody sees; the Views popup
    // is only built when an omnibox opens one.
    appendJoined(command_line, "disable-features", "MediaRouter,OptimizationHints,AutofillServerCommunication,Translate,WebUIOmniboxPopup,WebUIOmniboxFullPopup,WebUIOmniboxAimPopup");
}

/// The framework's own extension, loaded into Chrome style beside the app's.
/// Its only job is `chrome.downloads.setUiOptions({ enabled: false })`, the one
/// switch that keeps Chromium's download bubble and download-started animation
/// from being created at all (no feature, command-line switch or pref in CEF
/// 151 does, and --load-component-extension is not in the release build).
/// The AppKit host writes the same files (NDCefFrameworkExtension.swift).
pub const framework_extension_id = "pfbmaghgajhpjaobhbamhamgbcelckhd";
const framework_extension_manifest = @embedFile("framework-extension/manifest.json");
const framework_extension_background = @embedFile("framework-extension/background.js");
var framework_extension_dir: ?[:0]u8 = null;

fn writeFrameworkExtension(root: []const u8) void {
    const dir = std.fmt.allocPrintSentinel(alloc, "{s}/nd-framework-extension", .{root}, 0) catch return;
    _ = glib.mkdirWithParents(dir.ptr, 0o700);
    const files = [_]struct { name: []const u8, body: []const u8 }{
        .{ .name = "manifest.json", .body = framework_extension_manifest },
        .{ .name = "background.js", .body = framework_extension_background },
    };
    for (files) |f| {
        const path = std.fmt.allocPrintSentinel(alloc, "{s}/{s}", .{ dir, f.name }, 0) catch {
            alloc.free(dir);
            return;
        };
        defer alloc.free(path);
        var err: ?*glib.Error = null;
        if (glib.fileSetContents(path.ptr, f.body.ptr, @intCast(f.body.len), &err) == 0) {
            std.debug.print("ND_WARN CEF: the framework extension was not written to {s}\n", .{path});
            if (err) |e| e.free();
            alloc.free(dir);
            return;
        }
    }
    framework_extension_dir = dir;
}

/// Joins whatever value the launch already carries for `name`: Chromium reads
/// one comma-separated switch, and a second append would replace the earlier
/// list rather than add to it.
fn appendJoined(cl: [*c]c.cef_command_line_t, name: []const u8, value: []const u8) void {
    const append = cl.*.append_switch_with_value orelse return;
    var joined: std.ArrayList(u8) = .empty;
    defer joined.deinit(alloc);
    if (cl.*.get_switch_value) |get| {
        var key = std.mem.zeroes(c.cef_string_t);
        defer clearStr(&key);
        if (setStr(&key, name)) {
            const raw = get(cl, &key);
            if (raw != null) {
                defer freeUserfree(raw);
                if (dupeStr(raw)) |existing| {
                    defer alloc.free(existing);
                    joined.appendSlice(alloc, existing) catch return;
                }
            }
        }
    }
    if (joined.items.len > 0) joined.append(alloc, ',') catch return;
    joined.appendSlice(alloc, value) catch return;
    appendSwitch(cl, append, name, joined.items);
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
// Browser style
// ============================================================================

var chrome_style: ?bool = null;

/// Whether this process actually came up as the CEF browser process. An app
/// that asked for chromium and got a host with no loadable distribution is
/// running WebKitGTK, and this is what tells the two apart.
pub fn started() bool {
    return process_ready;
}

var views_hosted: ?bool = null;

/// `ND_CEF_VIEWS_HOSTED=1`, Chrome style only: the page is born in a CEF Views
/// window (a BrowserView in a frameless CefWindow) that is reparented into the
/// view's container, instead of in the child window CEF makes for a
/// `parent_window`. A browser made the second way gets CEF's own
/// ChildBrowserViewDelegate, which answers CEF_CTT_NONE, so it has no Chrome
/// toolbar and no ExtensionsContainer; the first way asks for one and hides it,
/// which is what `Extensions.triggerAction` needs. Opt-in until it has cleared
/// the same gates as the child window path.
pub fn viewsHosted() bool {
    if (views_hosted) |v| return v;
    const raw = std.c.getenv("ND_CEF_VIEWS_HOSTED");
    const v = chromeStyle() and raw != null and std.mem.eql(u8, std.mem.span(raw.?), "1");
    views_hosted = v;
    return v;
}

/// A chrome:// page keeps the child window even then. Chrome anchors its
/// extension dialogs to the toolbar of the browser they are raised from, and
/// an anchored bubble in a window the window manager never activates
/// closes as soon as it opens, which reads as "canceled by user". A browser
/// with no toolbar gets the modal dialog instead, which is what the registry
/// view needs.
fn viewsHostedFor(url: []const u8) bool {
    return viewsHosted() and !std.mem.startsWith(u8, url, "chrome://");
}

/// Chrome style gives the embedded browser Chromium's own browser runtime,
/// which is what runs the extension system; Alloy style is the content layer
/// with CEF's extra client callbacks. `webview.cef.style` in the app config
/// arrives here as ND_CEF_STYLE, and the launch path sets it in every process
/// because the command line and the browser style have to agree.
pub fn chromeStyle() bool {
    if (chrome_style) |v| return v;
    const raw = std.c.getenv("ND_CEF_STYLE");
    const v = raw != null and std.mem.eql(u8, std.mem.span(raw.?), "chrome");
    chrome_style = v;
    return v;
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
    settings.log_severity = if (std.c.getenv("ND_CEF_VERBOSE") != null)
        @intCast(c.LOGSEVERITY_VERBOSE)
    else
        @intCast(c.LOGSEVERITY_WARNING);

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
    var default_cache: ?[:0]u8 = null;
    defer if (cache_root) |p| alloc.free(p);
    defer if (default_cache) |p| alloc.free(p);
    if (defaultCacheRoot()) |p| {
        cache_root = p;
        _ = setStr(&settings.root_cache_path, p);
        linkNativeMessagingHosts(p);
        // The profile-less case needs a cache path of its own: a global
        // context with none is in-memory, so every cookie, every login and
        // every consent is forgotten at quit. The WebKitGTK backend names a
        // file for the same reason (persistCookies).
        const dir = std.fmt.allocPrintSentinel(alloc, "{s}/default", .{p}, 0) catch null;
        if (dir) |d| {
            default_cache = d;
            _ = glib.mkdirWithParents(d.ptr, 0o700);
            _ = setStr(&settings.cache_path, d);
            settings.persist_session_cookies = 1;
        }
    }
    defer clearStr(&settings.resources_dir_path);
    defer clearStr(&settings.locales_dir_path);
    defer clearStr(&settings.root_cache_path);
    defer clearStr(&settings.cache_path);

    cdp.setSink(.{ .result = &cdpResultSink, .event = &cdpEventSink });

    if (chromeStyle()) {
        if (cache_root) |root| writeFrameworkExtension(root);
    }
    var args = mainArgs();
    if (api.initialize(&args, &settings, app.handOut(), null) == 0) {
        if (cache_root) |root| {
            if (reportProfileInUse(root)) return false;
        }
        std.debug.print("ND_WARN CEF: cef_initialize failed; falling back to the system engine\n", .{});
        return false;
    }
    initialized = true;
    init_failed = false;
    browser_pipe.start();
    ensureSchemeFactories();
    // cef_initialize swapped the process's signal actions for Chromium's; give
    // the embedder its chance to take them back (src/gtk/main.zig).
    if (on_initialized) |cb| cb();
    return true;
}

var on_initialized: ?*const fn () callconv(.c) void = null;

/// Runs cef_initialize now rather than at the first <webview>. The host calls
/// it right after spawning the Bun child, so Chromium's startup overlaps the
/// child loading its modules instead of sitting inside the first commit.
pub fn warmUp() void {
    if (process_ready) _ = ensureInitialized();
}

/// Called on the thread that ran cef_initialize, right after it succeeds.
pub fn setOnInitialized(cb: *const fn () callconv(.c) void) void {
    on_initialized = cb;
}

/// Chromium's process singleton names its holder in `SingletonLock`, a symlink
/// to "<hostname>-<pid>" in the root cache directory. A failed cef_initialize
/// with a live holder that is not this process is another host on the same
/// profile, which Chromium has already handed this launch to.
extern "c" fn kill(pid: c_int, sig: c_int) c_int;

fn reportProfileInUse(root: [:0]const u8) bool {
    var path_buf: [4096]u8 = undefined;
    const lock = std.fmt.bufPrintZ(&path_buf, "{s}/SingletonLock", .{root}) catch return false;
    var target_buf: [256]u8 = undefined;
    const n = std.c.readlink(lock.ptr, &target_buf, target_buf.len);
    if (n <= 0) return false;
    const target = target_buf[0..@intCast(n)];
    const dash = std.mem.lastIndexOfScalar(u8, target, '-') orelse return false;
    const pid = std.fmt.parseInt(c_int, target[dash + 1 ..], 10) catch return false;
    // Signal 0 is the liveness probe, which std's SIG enum has no member for.
    if (pid == std.c.getpid() or kill(pid, 0) != 0) return false;
    std.debug.print("ND_CEF_PROFILE_IN_USE root={s} holder={s}; the chromium engine is not started\n", .{ root, target });
    return true;
}

// Chromium looks for a native messaging host manifest in
// <user-data-dir>/NativeMessagingHosts and /etc/chromium/native-messaging-hosts
// only, and there is no switch to add a directory. Desktop apps (1Password,
// Bitwarden, KeePassXC) install theirs for the browsers they know, so each
// start links those into the root cache. A name already present is left
// alone; a link whose target is gone is dropped first, so a manifest removed
// or moved upstream is picked up again from the next source that has it.
const native_messaging_user_dirs = [_][]const u8{
    "google-chrome",
    "google-chrome-beta",
    "google-chrome-unstable",
    "chromium",
    "BraveSoftware/Brave-Browser",
    "microsoft-edge",
    "microsoft-edge-beta",
    "microsoft-edge-dev",
    "vivaldi",
    "net.imput.helium",
};
const native_messaging_system_dirs = [_][]const u8{
    "/etc/opt/chrome/native-messaging-hosts",
    "/etc/opt/edge/native-messaging-hosts",
};

extern "c" fn g_dir_open(path: [*:0]const u8, flags: c_uint, err: ?*anyopaque) ?*anyopaque;
extern "c" fn g_dir_read_name(dir: *anyopaque) ?[*:0]const u8;
extern "c" fn g_dir_close(dir: *anyopaque) void;
extern "c" fn g_file_test(path: [*:0]const u8, tests: c_uint) c_int;
const g_file_test_is_symlink: c_uint = 1 << 1;
const g_file_test_exists: c_uint = 1 << 4;

fn linkNativeMessagingHosts(root: [:0]const u8) void {
    var dest_buf: [4096]u8 = undefined;
    const dest = std.fmt.bufPrintZ(&dest_buf, "{s}/NativeMessagingHosts", .{root}) catch return;
    _ = glib.mkdirWithParents(dest.ptr, 0o700);

    var path_buf: [4096]u8 = undefined;
    if (g_dir_open(dest.ptr, 0, null)) |dir| {
        defer g_dir_close(dir);
        while (g_dir_read_name(dir)) |name| {
            const path = std.fmt.bufPrintZ(&path_buf, "{s}/{s}", .{ dest, std.mem.span(name) }) catch continue;
            if (g_file_test(path.ptr, g_file_test_is_symlink) != 0 and g_file_test(path.ptr, g_file_test_exists) == 0)
                _ = std.c.unlink(path.ptr);
        }
    }

    const config = std.mem.span(glib.getUserConfigDir());
    var linked: usize = 0;
    var src_buf: [4096]u8 = undefined;
    for (native_messaging_user_dirs) |sub| {
        const src = std.fmt.bufPrintZ(&src_buf, "{s}/{s}/NativeMessagingHosts", .{ config, sub }) catch continue;
        linked += linkManifestsFrom(src, dest);
    }
    for (native_messaging_system_dirs) |src| {
        const z = std.fmt.bufPrintZ(&src_buf, "{s}", .{src}) catch continue;
        linked += linkManifestsFrom(z, dest);
    }
    if (linked > 0) std.debug.print("ND_CEF_NATIVE_MESSAGING linked={d} dir={s}\n", .{ linked, dest });
}

fn linkManifestsFrom(src: [:0]const u8, dest: [:0]const u8) usize {
    const dir = g_dir_open(src.ptr, 0, null) orelse return 0;
    defer g_dir_close(dir);
    var linked: usize = 0;
    var from_buf: [4096]u8 = undefined;
    var to_buf: [4096]u8 = undefined;
    while (g_dir_read_name(dir)) |name_z| {
        const name = std.mem.span(name_z);
        if (!std.mem.endsWith(u8, name, ".json")) continue;
        const to = std.fmt.bufPrintZ(&to_buf, "{s}/{s}", .{ dest, name }) catch continue;
        if (g_file_test(to.ptr, g_file_test_exists | g_file_test_is_symlink) != 0) continue;
        const from = std.fmt.bufPrintZ(&from_buf, "{s}/{s}", .{ src, name }) catch continue;
        if (std.c.symlink(from.ptr, to.ptr) == 0) linked += 1;
    }
    return linked;
}

/// Chromium's root_cache_path: `$XDG_DATA_HOME/<app>/cef`, inside the app's own
/// data directory (app_dir.zig), so no two NativeDesktop apps share a profile.
fn defaultCacheRoot() ?[:0]u8 {
    // ND_CEF_CACHE names the root, as on AppKit (NDCefEngine.swift). A run
    // that sets it expects its own profile; without this a Linux rig opened
    // the user's own CEF profile under the data dir, extensions and all.
    if (std.c.getenv("ND_CEF_CACHE")) |raw| {
        const override = std.mem.span(raw);
        if (override.len > 0) {
            const path = alloc.dupeZ(u8, override) catch return null;
            _ = glib.mkdirWithParents(path.ptr, 0o700);
            return path;
        }
    }
    const name = appDataName() orelse return null;
    defer alloc.free(name);
    const base = glib.getUserDataDir();
    const path = std.fmt.allocPrintSentinel(alloc, "{s}/{s}/cef", .{ std.mem.span(base), name }, 0) catch return null;
    _ = glib.mkdirWithParents(path.ptr, 0o700);
    return path;
}

extern "c" fn g_get_current_dir() [*:0]u8;
extern "c" fn g_get_prgname() ?[*:0]const u8;
extern "c" fn g_file_get_contents(path: [*:0]const u8, contents: *?[*]u8, length: *usize, err: ?*anyopaque) c_int;
extern "c" fn g_free(ptr: ?*anyopaque) void;

/// nd-app.json found walking up from cwd (a packaged run's AppRun cds into the
/// bundle's app dir), else the cwd's package.json (a dev run), else the
/// executable's own name.
fn appDataName() ?[]u8 {
    const cwd = g_get_current_dir();
    defer g_free(cwd);
    var dir: []const u8 = std.mem.span(cwd);
    while (true) {
        if (nameInFile(dir, "nd-app.json", &app_dir.manifest_keys)) |name| return name;
        const parent = std.fs.path.dirname(dir) orelse break;
        if (std.mem.eql(u8, parent, dir)) break;
        dir = parent;
    }
    if (nameInFile(std.mem.span(cwd), "package.json", &app_dir.package_keys)) |name| return name;
    const prg = g_get_prgname() orelse return alloc.dupe(u8, "nativedesktop") catch null;
    return alloc.dupe(u8, std.mem.span(prg)) catch null;
}

fn nameInFile(dir: []const u8, file: []const u8, keys: []const []const u8) ?[]u8 {
    var path_buf: [4096]u8 = undefined;
    const path = std.fmt.bufPrintZ(&path_buf, "{s}/{s}", .{ dir, file }) catch return null;
    var contents: ?[*]u8 = null;
    var len: usize = 0;
    if (g_file_get_contents(path.ptr, &contents, &len, null) == 0) return null;
    defer g_free(contents);
    return app_dir.nameFrom(alloc, contents.?[0..len], keys);
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
const DialogHandlerObj = ref.Counted(c.cef_dialog_handler_t, *View);
const ContextMenuObj = ref.Counted(c.cef_context_menu_handler_t, *View);
const FocusObj = ref.Counted(c.cef_focus_handler_t, *View);
const CommandObj = ref.Counted(c.cef_command_handler_t, *View);
const KeyboardObj = ref.Counted(c.cef_keyboard_handler_t, *View);
const RequestHandlerObj = ref.Counted(c.cef_request_handler_t, *View);
const PermissionObj = ref.Counted(c.cef_permission_handler_t, *View);

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
    dialog_handler: *DialogHandlerObj,
    context_menu_handler: *ContextMenuObj,
    focus_handler: *FocusObj,
    command_handler: *CommandObj,
    keyboard_handler: *KeyboardObj,
    request_handler: *RequestHandlerObj,
    permission_handler: *PermissionObj,

    /// Blocked requests since the last main-frame navigation. Bumped on the
    /// IO thread, read on the GTK one when `contentBlocked` is emitted.
    adblock_count: std.atomic.Value(u32) = .init(0),
    adblock_report_queued: std.atomic.Value(bool) = .init(false),

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
    /// Armed by openDevTools on the GTK thread, consumed by
    /// onBeforeDevToolsPopup on the CEF UI thread: the devtools window is
    /// allowed only when the app asked for it, never conjured by the page.
    devtools_requested: std.atomic.Value(bool) = .init(false),
    menu_lock: SpinLock = .{},
    /// The answer `installExtension` parks for the directory chooser that
    /// `chrome.developerPrivate.loadUnpacked` opens. Written on the GTK thread
    /// by the command, read on the CEF UI thread by `on_file_dialog`.
    dialog_lock: SpinLock = .{},
    pending_dialog_path: ?[]u8 = null,
    /// Whether an `installExtension` is still waiting for an answer on this
    /// view. A chooser that opens while one is, with no path parked for it, is
    /// the install asking a second time; letting CEF put its own directory
    /// chooser up there is a dialog nobody will ever answer and an install
    /// promise that never settles.
    install_in_flight: bool = false,
    menu_items: []ctxmenu.Item = &.{},
    menu_commands: std.AutoHashMapUnmanaged(c_int, MenuCommand) = .empty,
    next_menu_command: c_int = menu_command_first,
    menu_page_url_slot: ?[]u8 = null,
    /// The GTK menu currently on screen for this view, GTK thread only. One at
    /// a time: a second right-click dismisses the first, which answers its
    /// callback before the new one takes the slot.
    menu_popup: ?*gtkmenu.Popup = null,
    menu_request: ?*MenuRequest = null,
    /// A menu Chromium asked for while a mouse button was still down, and the
    /// timeout watching for that button to come up. See `openNativeMenu`.
    menu_pending: ?*MenuRequest = null,
    menu_open_source: c_uint = 0,
    menu_open_deadline_us: i64 = 0,

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
    /// The Views-hosted embedding only, CEF UI thread only: the BrowserView and
    /// the frameless CEF window it sits in, both held until the browser closes.
    views_browser_view: ?*c.cef_browser_view_t = null,
    views_size: struct { w: c_uint = 0, h: c_uint = 0 } = .{},
    views_window: ?*c.cef_window_t = null,
    /// From the window's creation until CEF reports it destroyed. Closing it
    /// is asynchronous, and cef_shutdown started while one is still up joins
    /// a UI thread whose run loop never quits.
    views_window_live: std.atomic.Value(bool) = .init(false),
    /// The docked devtools, Chrome style only: our own X child inside the
    /// container, CEF's devtools window inside that, and the view's last known
    /// size so the CEF UI thread can lay the split out without reading the
    /// GTK-owned bounds.
    devtools_container: std.atomic.Value(usize) = .init(0),
    devtools_window: std.atomic.Value(usize) = .init(0),
    /// The dock container, kept from the first docking to the view's end.
    /// `devtools_container` is the same window while devtools is up and 0 when
    /// it is not, which is what the layout reads.
    dock_container: x11.Window = 0,
    /// The inspector's own browser host, kept while it is docked: the protocol
    /// session that reads the frontend's page rectangle sends through it.
    devtools_host: std.atomic.Value(usize) = .init(0),
    dock_session: cdp.Session = .{},
    /// The id of the handshake call the frontend's hook waits on.
    dock_handshake: std.atomic.Value(c_int) = .init(0),
    /// The rectangle the frontend announced for the page, in view pixels. A
    /// width of 0 is "it has not said yet", which lays the page out at Chrome's
    /// own right-dock default until it does.
    page_x: std.atomic.Value(u32) = .init(0),
    page_y: std.atomic.Value(u32) = .init(0),
    page_w: std.atomic.Value(u32) = .init(0),
    page_h: std.atomic.Value(u32) = .init(0),
    size_w: std.atomic.Value(u32) = .init(0),
    size_h: std.atomic.Value(u32) = .init(0),
    /// The view's own long-lived host reference, taken with the browser and
    /// released with it. Every CDP call goes through it, and re-deriving it per
    /// call would churn a reference on whichever thread happened to ask.
    host: std.atomic.Value(usize) = .init(0),
    /// The zoom level (CEF's log scale) last reported as `zoomChanged`. CEF UI
    /// thread only.
    reported_zoom: f64 = 0,

    // GTK thread only from here down.
    container: x11.Window = 0,
    /// The container of a removed view whose browser is still closing. It is
    /// destroyed once on_before_close is through (see `onDestroy`).
    closing_container: x11.Window = 0,
    /// Born in a CEF Views window (`viewsHostedFor`), so it has the toolbar
    /// `triggerExtensionAction` clicks through.
    views_hosted: bool = false,
    /// A dialog is up over the page: the page is off to the side of the window,
    /// or under the command bar (`under_bar`). The keyboard stays off it, and
    /// the engine tick can spot one whose dialog went away unannounced.
    aside: bool = false,
    /// The page stays where it is under the command bar, dimmed, cut around
    /// the bar's card and letting the pointer through. See `setUnderBar`.
    under_bar: bool = false,
    /// The split the page is in is sliding its sidebar. See `syncMotion`.
    moving: bool = false,
    /// What the page showed when the slide began, drawn stretched over the
    /// page's rectangle while its window is cut to nothing.
    still: ?*gdk.Texture = null,
    /// What the page showed as a dialog came up over it, drawn in its place
    /// under the dialog's scrim while the page itself stands aside.
    aside_still: ?*gdk.Texture = null,
    motion_started_us: i64 = 0,
    motion_ended_us: i64 = 0,
    /// Each resize of the page during a slide asks the page to report back
    /// once it has painted at that size; the still goes when the last one has.
    probe_gen: u32 = 0,
    painted_gen: u32 = 0,
    probe_bounds: Bounds = .{},
    motion_timer: c_uint = 0,
    /// The frame from which the page's window may be cut away: the still has
    /// to have been drawn under it first. 0 once it has been.
    motion_hide_frame: i64 = 0,
    /// The toplevel the container is a child of. A view moved into another
    /// window by `moveNode` has to take its X child with it.
    container_parent: x11.Window = 0,
    /// Whether the container carries a bounding shape (see `syncShape`), so a
    /// view that needs none clears it once rather than on every layout.
    shaped: bool = false,
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
    /// The toplevel's `is-active` and `focus-widget` handlers, and the window
    /// they are on. See `connectActive`.
    active_handler: c_ulong = 0,
    focus_widget_handler: c_ulong = 0,
    active_window: ?*gtk.Window = null,
    /// The toplevel's `is-active` as the handler last saw it, so a window that
    /// loses activation is told apart from a focus-widget change.
    window_was_active: bool = false,
    /// Whether the page currently holds the keyboard, as last told to the
    /// browser. Transitions are what the app hears as `focusChanged`.
    page_focused: bool = false,
    /// Set while the engine moves focus off a page that gave it up itself
    /// (Tab past its last element), for `syncBrowserFocus` to read.
    page_released: bool = false,
    bounds: Bounds = .{},
    /// Whether the view has been put on screen at its own bounds, and whether
    /// its page is marked minimized while it is parked, and the timer that
    /// minimizes it. See `parkPage`.
    placed: bool = false,
    page_minimized: bool = false,
    park_timer: c_uint = 0,
    /// The deferred `parkContainer` of an unmapped view. See `onUnmap`.
    park_source: c_uint = 0,
    /// The frame clock (held) and its `after-paint` handler a deferred park
    /// waits on. See `onUnmap`.
    park_clock: ?*gdk.FrameClock = null,
    park_handler: c_ulong = 0,
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
    /// The browser closed and will not come back: Chromium can close one on
    /// its own (a page's window.close(), a tab it tears down), and every call
    /// after that, or parked before it, is answered with an error here rather
    /// than waiting on an agent that will never attach again.
    browser_gone: std.atomic.Value(bool) = .init(false),
    /// Drawn in an autohide popover, for the CEF UI thread's Escape check.
    in_popover: std.atomic.Value(bool) = .init(false),
    /// The `adoptPopups` prop: a window.open from this page gets a browser of
    /// its own that the app mounts with `<webview popup=…>`. Written on the GTK
    /// thread, read in on_before_popup.
    adopt_popups: std.atomic.Value(bool) = .init(false),
    /// A popup's view between on_before_popup and the app adopting it: no
    /// widget, its events held in `popup_backlog`. Cleared on the GTK thread
    /// when the app adopts it or gives up on it.
    waiting_popup: std.atomic.Value(bool) = .init(false),
    /// The app never adopted it, so its browser is closed as soon as it exists.
    popup_abandoned: std.atomic.Value(bool) = .init(false),
    popup_id: u32 = 0,
    popup_timer: c_uint = 0,
    popup_backlog: std.ArrayList(*Emission) = .empty,
    /// When this engine last handed the browser the focus itself, so the CEF
    /// UI thread can tell that `on_got_focus` from one a click caused.
    focus_set_us: std.atomic.Value(i64) = .init(0),
    queued: std.ArrayList(Queued) = .empty,
    /// Per-script-id install counter. A script identifier comes back
    /// asynchronously, so an id that is re-added (or removed) while its own
    /// install is in flight would otherwise store the stale identifier and
    /// leak the live script.
    script_gens: std.StringHashMapUnmanaged(u64) = .empty,
    /// World names whose document-start stub is registered, so it is sent once
    /// per view rather than once per frame.
    worlds_requested: std.StringHashMapUnmanaged(void) = .empty,
    /// "<world>\x00<frameId>" to its live execution context id.
    world_contexts: std.StringHashMapUnmanaged(i64) = .empty,
    /// The frame an app means by "this view". Learned from Page.getFrameTree
    /// and kept current by Page.frameNavigated.
    main_frame: ?[]u8 = null,
    scripts: std.StringHashMapUnmanaged(ScriptEntry) = .empty,
    channels: std.StringHashMapUnmanaged(Channel) = .empty,
    /// Whether the registry-change binding is already on this view. Adding one
    /// twice is a protocol error rather than a no-op.
    extensions_watched: bool = false,
    /// Work parked until the world it names has an execution context, and the
    /// clock that expires it if that never happens.
    deferred: std.ArrayList(Deferred) = .empty,
    deferred_timer: c_uint = 0,

    // Last values the handlers pushed, answering `webviewInfo`.
    url: ?[]u8 = null,
    title: ?[]u8 = null,
    loading: bool = false,
    can_go_back: bool = false,
    can_go_forward: bool = false,

    fn browserOpen(self: *View) bool {
        return self.browser.load(.acquire) != 0;
    }

    fn viewsWindowOpen(self: *View) bool {
        return self.views_window_live.load(.acquire);
    }

    fn devtoolsOpen(self: *View) bool {
        return self.devtools_window.load(.acquire) != 0;
    }
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
fn abandonNew(view: *View, client: ?*ClientObj, display_handler: ?*DisplayObj, load_handler: ?*LoadObj) ?*View {
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

pub fn create(url: ?[*:0]const u8, profile: []const u8, context_menu_mode: []const u8, popup: []const u8) ?*gtk.Widget {
    if (!process_ready) {
        std.debug.print("ND_WARN WebView engine=\"chromium\": this process did not start under CEF (set webview.engine in nativedesktop.config.ts, or ND_WEBVIEW_ENGINE=chromium)\n", .{});
        return null;
    }
    if (!ensureInitialized()) return null;

    const adopted = if (popup.len != 0) takePendingPopup(popup) else null;
    if (popup.len != 0 and adopted == null) tr("popup {s} is no longer waiting; creating a fresh view", .{popup});
    const view = adopted orelse (newView() orelse return null);
    ensureAdblockBlock();
    ensureOriginFix();

    // Reserves the rectangle the X11 child window is tracked against, and
    // draws nothing but a still of the page: the one a sliding sidebar
    // stretches (syncMotion), or the one under a dialog (standAside).
    const picture = gtk.Picture.new();
    gtk.Picture.setCanShrink(picture, 1);
    gtk.Picture.setContentFit(picture, .fill);
    const widget = picture.as(gtk.Widget);
    view.widget = widget;

    view.suppress_menu.store(std.mem.eql(u8, context_menu_mode, "suppress"), .release);
    if (adopted == null) view.context = requestContext(profile);
    if (url) |u| {
        if (u[0] != 0) view.pending_url = alloc.dupeZ(u8, std.mem.span(u)) catch null;
    }

    live_views.put(alloc, @intFromPtr(view), {}) catch {};
    startChromeWindowWatch();
    gobject.Object.setData(widget.as(gobject.Object), MARKER_KEY, @ptrFromInt(1));
    gobject.Object.setData(widget.as(gobject.Object), VIEW_KEY, view);
    gtk.Widget.setHexpand(widget, 1);
    gtk.Widget.setVexpand(widget, 1);
    // The `focus` command grabs GTK focus as well as CEF's, and a
    // GtkPicture takes none by default.
    gtk.Widget.setCanFocus(widget, 1);
    gtk.Widget.setFocusable(widget, 1);

    _ = gobject.signalConnectData(widget.as(gobject.Object), "map", @ptrCast(&onMap), view, null, .{});
    _ = gobject.signalConnectData(widget.as(gobject.Object), "unmap", @ptrCast(&onUnmap), view, null, .{});
    _ = gobject.signalConnectData(widget.as(gobject.Object), "destroy", @ptrCast(&onDestroy), view, null, .{});
    if (adopted) |v| {
        // What the browser did before the app took it is replayed once the
        // view has its node id, which `connectEvents` sets after this returns.
        _ = glib.idleAddFull(glib.PRIORITY_DEFAULT, &replayPopupBacklog, v, null);
    } else {
        armCreateTimer(view);
    }
    return widget;
}

/// A view and every CEF handler it answers through, with no widget yet. Safe
/// off the GTK thread: it only allocates, which is what lets on_before_popup
/// build the view a popup's browser belongs to before the app has asked for
/// it.
fn newView() ?*View {
    const view = alloc.create(View) catch return null;
    const client = ClientObj.create(view) orelse return abandonNew(view, null, null, null);
    const display_handler = DisplayObj.create(view) orelse return abandonNew(view, client, null, null);
    const load_handler = LoadObj.create(view) orelse return abandonNew(view, client, display_handler, null);
    const life_handler = LifeObj.create(view) orelse return abandonNew(view, client, display_handler, load_handler);
    const find_handler = FindObj.create(view) orelse return abandonNew(view, client, display_handler, load_handler);
    const download_handler = DownloadObj.create(view) orelse return abandonNew(view, client, display_handler, load_handler);
    const jsdialog_handler = JsDialogHandlerObj.create(view) orelse return abandonNew(view, client, display_handler, load_handler);
    const dialog_handler = DialogHandlerObj.create(view) orelse return abandonNew(view, client, display_handler, load_handler);
    const context_menu_handler = ContextMenuObj.create(view) orelse return abandonNew(view, client, display_handler, load_handler);
    const focus_handler = FocusObj.create(view) orelse return abandonNew(view, client, display_handler, load_handler);
    const command_handler = CommandObj.create(view) orelse return abandonNew(view, client, display_handler, load_handler);
    const keyboard_handler = KeyboardObj.create(view) orelse return abandonNew(view, client, display_handler, load_handler);
    const request_handler = RequestHandlerObj.create(view) orelse return abandonNew(view, client, display_handler, load_handler);
    const permission_handler = PermissionObj.create(view) orelse return abandonNew(view, client, display_handler, load_handler);

    view.* = .{
        .widget = undefined,
        .client = client,
        .display_handler = display_handler,
        .load_handler = load_handler,
        .life_handler = life_handler,
        .find_handler = find_handler,
        .download_handler = download_handler,
        .jsdialog_handler = jsdialog_handler,
        .dialog_handler = dialog_handler,
        .context_menu_handler = context_menu_handler,
        .focus_handler = focus_handler,
        .command_handler = command_handler,
        .keyboard_handler = keyboard_handler,
        .request_handler = request_handler,
        .permission_handler = permission_handler,
    };

    client.cef.get_display_handler = &clientGetDisplayHandler;
    client.cef.get_load_handler = &clientGetLoadHandler;
    client.cef.get_life_span_handler = &clientGetLifeSpanHandler;
    client.cef.get_find_handler = &clientGetFindHandler;
    client.cef.get_download_handler = &clientGetDownloadHandler;
    client.cef.get_jsdialog_handler = &clientGetJsDialogHandler;
    client.cef.get_dialog_handler = &clientGetDialogHandler;
    client.cef.get_context_menu_handler = &clientGetContextMenuHandler;
    client.cef.get_focus_handler = &clientGetFocusHandler;
    client.cef.get_command_handler = &clientGetCommandHandler;
    client.cef.get_keyboard_handler = &clientGetKeyboardHandler;
    client.cef.get_request_handler = &clientGetRequestHandler;
    client.cef.get_permission_handler = &clientGetPermissionHandler;

    display_handler.cef.on_address_change = &onAddressChange;
    display_handler.cef.on_title_change = &onTitleChange;
    display_handler.cef.on_loading_progress_change = &onLoadingProgressChange;
    display_handler.cef.on_favicon_urlchange = &onFaviconUrlChange;

    load_handler.cef.on_load_start = &onLoadStart;
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
    download_handler.cef.on_download_updated = &onDownloadUpdated;
    jsdialog_handler.cef.on_jsdialog = &onJsDialog;
    jsdialog_handler.cef.on_before_unload_dialog = &onBeforeUnloadDialog;
    dialog_handler.cef.on_file_dialog = &onFileDialog;
    context_menu_handler.cef.on_before_context_menu = &onBeforeContextMenu;
    context_menu_handler.cef.run_context_menu = &onRunContextMenu;
    context_menu_handler.cef.on_context_menu_command = &onContextMenuCommand;
    focus_handler.cef.on_set_focus = &onSetFocus;
    focus_handler.cef.on_take_focus = &onTakeFocus;
    focus_handler.cef.on_got_focus = &onGotFocus;
    keyboard_handler.cef.on_pre_key_event = &onPreKeyEvent;
    command_handler.cef.on_chrome_command = &onChromeCommand;
    command_handler.cef.is_chrome_app_menu_item_visible = &isChromeAppMenuItemVisible;
    command_handler.cef.is_chrome_page_action_icon_visible = &isChromePageActionIconVisible;
    command_handler.cef.is_chrome_toolbar_button_visible = &isChromeToolbarButtonVisible;
    request_handler.cef.on_open_urlfrom_tab = &onOpenUrlFromTab;
    request_handler.cef.on_before_browse = &onBeforeBrowse;
    request_handler.cef.get_auth_credentials = &onGetAuthCredentials;
    request_handler.cef.get_resource_request_handler = &onGetResourceRequestHandler;
    permission_handler.cef.on_show_permission_prompt = &onShowPermissionPrompt;
    permission_handler.cef.on_request_media_access_permission = &onRequestMediaAccessPermission;
    permission_handler.cef.on_dismiss_permission_prompt = &onDismissPermissionPrompt;
    return view;
}

fn onMap(_: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const view: *View = @ptrCast(@alignCast(data.?));
    marker.lat("cef.map", "node={d} minimized={}", .{ view.node_id, view.page_minimized });
    defer marker.lat("cef.mapped", "node={d} x={d} y={d} w={d} h={d}", .{ view.node_id, view.bounds.x, view.bounds.y, view.bounds.w, view.bounds.h });
    connectLayout(view);
    connectActive(view);
    syncBounds(view);
    maybeCreateBrowser(view);
    x11.show(view.container);
    // Above the page it replaces, which stays where it is until the commit
    // is through (`onUnmap`): stacked under it, the tab switched to stayed
    // covered until then.
    x11.raise(view.container);
    releasePopoverGrab(view);
}

/// A page in an autohide GtkPopover (an extension's action popup) never saw a
/// click or a key: the popover's seat grab sends every event for a window that
/// is not GDK's to the popover's own surface. GDK grabs again each time it
/// presents the popup, so this runs on every layout of the popover as well.
fn releasePopoverGrab(view: *View) void {
    const popover = autohidePopoverOf(view);
    const was = view.in_popover.swap(popover != null, .acq_rel);
    const pop = popover orelse return;
    x11.releaseSeatGrab();
    if (was) return;
    watchOutsidePresses(pop);
    // Chrome gives an extension popup the keyboard as it opens. `settle`
    // hands it to the browser once one exists.
    _ = gtk.Widget.grabFocus(view.widget);
    syncBrowserFocus(view);
}

const OUTSIDE_PRESS_KEY = "nd-cef-outside-press";

/// With the grab gone GDK no longer closes the popover on a press elsewhere
/// in its window, so the window's own presses do, ahead of every widget.
fn watchOutsidePresses(popover: *gtk.Popover) void {
    const root = gtk.Widget.getRoot(popover.as(gtk.Widget)) orelse return;
    const window = (gobject.ext.cast(gtk.Window, root) orelse return).as(gtk.Widget);
    if (gobject.Object.getData(window.as(gobject.Object), OUTSIDE_PRESS_KEY) != null) return;
    const click = gtk.GestureClick.new();
    gtk.GestureSingle.setButton(click.as(gtk.GestureSingle), 0);
    gtk.EventController.setPropagationPhase(click.as(gtk.EventController), .capture);
    _ = gtk.GestureClick.signals.pressed.connect(click, ?*anyopaque, &onOutsidePress, null, .{});
    gtk.Widget.addController(window, click.as(gtk.EventController));
    gobject.Object.setData(window.as(gobject.Object), OUTSIDE_PRESS_KEY, click);
}

/// A press inside a popover is on the popover's own surface and reaches the
/// window's controllers too, since the popover is the window's descendant;
/// only presses on other surfaces close it.
fn onOutsidePress(gesture: *gtk.GestureClick, _: c_int, _: f64, _: f64, _: ?*anyopaque) callconv(.c) void {
    const controller = gesture.as(gtk.EventController);
    const window = gtk.EventController.getWidget(controller) orelse return;
    const event = gtk.EventController.getCurrentEvent(controller);
    const surface = if (event) |e| gdk.Event.getSurface(e) else null;
    var open: std.ArrayList(*gtk.Popover) = .empty;
    defer open.deinit(alloc);
    var it = live_views.keyIterator();
    while (it.next()) |key| {
        const view: *View = @ptrFromInt(key.*);
        if (gtk.Widget.getMapped(view.widget) == 0) continue;
        const popover = autohidePopoverOf(view) orelse continue;
        const root = gtk.Widget.getRoot(popover.as(gtk.Widget)) orelse continue;
        const root_window = gobject.ext.cast(gtk.Window, root) orelse continue;
        if (root_window.as(gtk.Widget) != window) continue;
        const native = gtk.Widget.getNative(view.widget) orelse continue;
        if (surface != null and gtk.Native.getSurface(native) == surface) continue;
        tr("popdown node={d} for a press outside", .{view.node_id});
        open.append(alloc, popover) catch {};
    }
    for (open.items) |popover| gtk.Popover.popdown(popover);
}

/// An extension page in an autohide popover: the app's take on Chrome's action
/// popup. Chrome's popup is no tab, and extension APIs resolve the current
/// window to the browser it hangs from. Under Chrome style every page here
/// is a Chrome Browser of its own, so as soon as the popup took focus it became
/// the last active window, and chrome.tabs.query with currentWindow or
/// lastFocusedWindow, from the popup or from the extension's worker, answered
/// with the popup itself. An Alloy-style browser has no Browser, so it is never
/// in Chrome's activation order and the page under it stays the current
/// window, while CEF still runs the extension in it (chrome_extension_util.cc
/// GetAlloyTabById).
fn isActionPopup(view: *View) bool {
    if (!chromeStyle()) return false;
    const url = view.pending_url orelse return false;
    if (!std.mem.startsWith(u8, url, "chrome-extension://")) return false;
    return autohidePopoverOf(view) != null;
}

/// The autohide popover a view is drawn in, if any.
fn autohidePopoverOf(view: *View) ?*gtk.Popover {
    const native = gtk.Widget.getNative(view.widget) orelse return null;
    const popover = gobject.ext.cast(gtk.Popover, native) orelse return null;
    if (gtk.Popover.getAutohide(popover) == 0) return null;
    return popover;
}

/// Closes every autohide popover with a page in it other than `except`'s: on
/// Escape in that page, and when a click moves the keyboard to another page,
/// which GDK never sees.
fn popdownPagePopovers(except: ?*View) void {
    // Collected first: closing a popover destroys the views in it.
    var open: std.ArrayList(*gtk.Popover) = .empty;
    defer open.deinit(alloc);
    var it = live_views.keyIterator();
    while (it.next()) |key| {
        const other: *View = @ptrFromInt(key.*);
        if (other == except or gtk.Widget.getMapped(other.widget) == 0) continue;
        const popover = autohidePopoverOf(other) orelse continue;
        tr("popdown node={d}", .{other.node_id});
        open.append(alloc, popover) catch {};
    }
    for (open.items) |popover| gtk.Popover.popdown(popover);
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
    if (marker.latOn()) latPaints(surface);
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

/// ND_LAT_TRACE: a `gtk.paint` line each time GTK has drawn the window a page
/// is in, which is when the app's own chrome (a sidebar row marked as the one
/// on show) has been redrawn.
var lat_paint_surface: ?*gdk.Surface = null;

fn latPaints(surface: *gdk.Surface) void {
    if (lat_paint_surface == surface) return;
    lat_paint_surface = surface;
    const clock = gdk.Surface.getFrameClock(surface);
    _ = gobject.signalConnectData(clock.as(gobject.Object), "before-paint", @ptrCast(&onLatFrame), null, null, .{});
    _ = gobject.signalConnectData(clock.as(gobject.Object), "layout", @ptrCast(&onLatLayout), null, null, .{});
    _ = gobject.signalConnectData(clock.as(gobject.Object), "paint", @ptrCast(&onLatPaintStart), null, null, .{});
    _ = gobject.signalConnectData(clock.as(gobject.Object), "after-paint", @ptrCast(&onLatPaint), null, null, .{});
    lat_tick_last = marker.nowMicros();
    _ = glib.timeoutAdd(2, &onLatTick, null);
}

/// ND_LAT_TRACE: a `host.stall` line, at its end, for each stretch the GTK
/// thread went without getting back to its main loop.
var lat_tick_last: i64 = 0;

fn onLatTick(_: ?*anyopaque) callconv(.c) c_int {
    const now = marker.nowMicros();
    if (now - lat_tick_last > 8 * std.time.us_per_ms) marker.lat("host.stall", "us={d}", .{now - lat_tick_last});
    lat_tick_last = now;
    return 1;
}

fn onLatFrame(_: *gobject.Object, _: ?*anyopaque) callconv(.c) void {
    marker.lat("gtk.frame", "", .{});
}

fn onLatLayout(_: *gobject.Object, _: ?*anyopaque) callconv(.c) void {
    marker.lat("gtk.layout", "", .{});
}

fn onLatPaintStart(_: *gobject.Object, _: ?*anyopaque) callconv(.c) void {
    marker.lat("gtk.paintStart", "", .{});
}

fn onLatPaint(_: *gobject.Object, _: ?*anyopaque) callconv(.c) void {
    marker.lat("gtk.paint", "", .{});
}

/// Keyboard routing between the app's own widgets and the page.
///
/// Two things have to agree. X input focus decides which window the server
/// delivers a key press to, and Chromium moves it onto its own child as soon
/// as the page is clicked, so nothing the app does with GTK focus afterwards
/// gets the keyboard back: typing a URL went into the page. CEF's own
/// `SetFocus` decides whether the web contents act on what they receive, and
/// without it a page could not be typed into at all, which is the call
/// cefclient's GTK sample makes from RootWindowGtk::WindowFocusIn.
///
/// GTK's focus widget is what both follow: the view holds the keyboard while
/// it is the focused widget (which `on_got_focus` arranges when the user
/// clicks into the page), and the focus proxy holds it otherwise, so a key
/// press reaches GTK wherever the pointer is.
///
/// Only a mapped view is connected, so a parked background tab never takes the
/// keyboard from the one on screen.
fn connectActive(view: *View) void {
    if (view.active_handler != 0) return;
    const root = gtk.Widget.getRoot(view.widget) orelse return;
    const window = gobject.ext.cast(gtk.Window, root) orelse return;
    view.active_window = window;
    view.active_handler = gobject.signalConnectData(
        window.as(gobject.Object),
        "notify::is-active",
        @ptrCast(&onToplevelFocusChanged),
        view,
        null,
        .{},
    );
    view.focus_widget_handler = gobject.signalConnectData(
        window.as(gobject.Object),
        "notify::focus-widget",
        @ptrCast(&onToplevelFocusChanged),
        view,
        null,
        .{},
    );
    view.window_was_active = gtk.Window.isActive(window) != 0;
    syncBrowserFocus(view);
}

fn disconnectActive(view: *View) void {
    if (view.active_window) |window| {
        if (view.active_handler != 0) gobject.signalHandlerDisconnect(window.as(gobject.Object), view.active_handler);
        if (view.focus_widget_handler != 0) gobject.signalHandlerDisconnect(window.as(gobject.Object), view.focus_widget_handler);
    }
    view.active_handler = 0;
    view.focus_widget_handler = 0;
    view.active_window = null;
}

fn onToplevelFocusChanged(_: *gobject.Object, _: *gobject.ParamSpec, data: ?*anyopaque) callconv(.c) void {
    const view: *View = @ptrCast(@alignCast(data.?));
    syncBrowserFocus(view);
    const window = view.active_window orelse return;
    const active = gtk.Window.isActive(window) != 0;
    if (view.window_was_active and !active) dropHover(view);
    view.window_was_active = active;
}

/// The window just lost activation: everything hovering over it goes.
///
/// Chromium's link status bubble, a page's title tooltip and GTK's own
/// tooltips are override-redirect windows on the root, which no window manager
/// stacks with the app. They close when the pointer leaves, but focus that
/// moves by keyboard moves no pointer: on a Hyprland scrolling layout the
/// app's column slides away under a pointer that never left it, and a popup
/// stayed on screen over whatever slid in. So the page is told the pointer
/// left, which is the path that closes Chromium's own popups the way it
/// always does, and a GTK tooltip is hidden directly.
fn dropHover(view: *View) void {
    if (hostOf(view)) |host| {
        if (host.send_mouse_move_event) |send| {
            const event = std.mem.zeroInit(c.cef_mouse_event_t, .{ .x = -1, .y = -1 });
            send(host, &event, 1);
        }
    }
    hideTooltips();
}

/// GTK hides a tooltip on the pointer leaving its widget and on nothing to do
/// with activation. GtkTooltip reads the tooltip window's visibility as its own
/// state, so hiding the window is what its own timeout would have done, and
/// the next hover shows it again.
fn hideTooltips() void {
    var buf: [1024]x11.Window = undefined;
    var truncated = false;
    for (x11.rootChildren(&buf, &truncated)) |w| {
        if (w == 0 or !x11.isOverrideRedirect(w) or !x11.viewable(w)) continue;
        const surface = x11.gdkSurface(w) orelse continue;
        const native = gtk.Native.getForSurface(surface) orelse continue;
        const widget: *gtk.Widget = @ptrCast(@alignCast(native));
        const instance: *gobject.TypeInstance = @ptrCast(@alignCast(widget));
        if (!std.mem.eql(u8, std.mem.span(gobject.typeNameFromInstance(instance)), "GtkTooltipWindow")) continue;
        gtk.Widget.setVisible(widget, 0);
    }
}

fn syncBrowserFocus(view: *View) void {
    if (gtk.Widget.getMapped(view.widget) == 0) return;
    if (!focusEligible(view)) {
        // Never granted, and never told otherwise either: every set_focus is a
        // round trip that a second browser in the window answers by taking the
        // focus back. X input focus Chromium moved there on its own still
        // comes home.
        const cef_window = view.cef_window.load(.acquire);
        if (cef_window != 0 and x11.focused() == @as(x11.Window, @intCast(cef_window))) x11.focusToplevel(view.widget);
        if (view.page_focused) {
            view.page_focused = false;
            if (emit) |f| f(view.node_id, "focusChanged", .{ .checked = false });
        }
        return;
    }
    const root = gtk.Widget.getRoot(view.widget) orelse return;
    const window = gobject.ext.cast(gtk.Window, root) orelse return;
    const mine = if (gtk.Window.getFocus(window)) |focused| focused == view.widget else false;
    const active = gtk.Window.isActive(window) != 0;
    const cef_window = view.cef_window.load(.acquire);
    if (mine and active) {
        if (cef_window != 0) x11.focus(@intCast(cef_window));
    } else if (active and (view.page_focused or (cef_window != 0 and x11.focused() == @as(x11.Window, @intCast(cef_window))))) {
        // Back to the app, and only from the view that is holding the
        // keyboard: the others share this toplevel and would take it off
        // whichever one has it. `page_focused` as well as the X comparison,
        // because Chromium can move input focus below the window this engine
        // reparented, and the comparison then answers false while the keyboard
        // is very much the page's.
        //
        // Only in the active window: returning focus activates the toplevel,
        // and a page in a window that just lost to another one would take the
        // activation straight back. Two windows with a page each did that
        // 1.2 million times in one engine gate run and starved GTK of layout.
        if (view.page_released) x11.focusToplevel(view.widget) else returnFocusToApp(view);
    }
    const host = hostOf(view) orelse return;
    const wants = active and mine;
    if (wants) view.focus_set_us.store(glib.getMonotonicTime(), .release);
    if (host.set_focus) |set| set(host, @intFromBool(wants));
    if (wants != view.page_focused) {
        view.page_focused = wants;
        if (emit) |f| f(view.node_id, "focusChanged", .{ .checked = wants });
    }
}

/// Whether this X server is XWayland, where the compositor owns activation.
/// A Wayland session always names its socket in the environment, and a GTK app
/// on X11 in one is on XWayland by definition.
fn onXWayland() bool {
    if (xwayland) |v| return v;
    const v = std.c.getenv("WAYLAND_DISPLAY") != null;
    xwayland = v;
    return v;
}
var xwayland: ?bool = null;

/// Hands X input focus back to the app's own chrome after the page had it.
///
/// X delivers a key press to the window under the pointer whenever that window
/// sits below the focus window, so focus on the toplevel itself leaves every
/// key with the browser's child while the pointer is over the page: the app
/// asks for its address field, keeps GTK's focus widget, and still types into
/// the page. Asking the window manager to activate the window instead parks
/// input focus on a focus proxy of its own, which is not an ancestor of the
/// browser's window; that is the arrangement a user already gets by clicking
/// the field, which is why that route always worked.
///
/// Not on XWayland. There the compositor answers the activation itself, pins
/// input focus to the toplevel exactly as before, and costs the page its next
/// click; measured on the wlroots rig, where it took five legs that click into
/// the page down with it.
fn returnFocusToApp(view: *View) void {
    if (!onXWayland()) {
        if (gtk.Widget.getNative(view.widget)) |native| {
            if (gtk.Native.getSurface(native)) |surface| {
                if (gobject.ext.cast(gdk.Toplevel, surface)) |toplevel| {
                    gdk.Toplevel.focus(toplevel, 0);
                    return;
                }
            }
        }
    }
    x11.focusToplevel(view.widget);
}

/// Smallest side, in device pixels, of a view the keyboard can belong to. Apps
/// keep functional-but-invisible browsers at 2x2 (Chromium will not run a page
/// whose window has a side of 1), such as the one an extension registry is
/// read from. With an extension loaded, chrome://extensions has something
/// focusable, and that view and the page on show took the focus from each
/// other without end: each grant posts to the CEF UI thread, whose wakeup pipe
/// filled until both it and the GTK thread blocked writing to it.
const focusable_min_px: u32 = 32;

/// Read from the atomics `syncBounds` keeps, because the CEF UI thread asks.
fn focusEligible(view: *View) bool {
    // A page standing aside for a dialog is parked off the window: the dialog
    // has the keyboard, and the page may not take it back until it comes home.
    if (view.aside) return false;
    return view.size_w.load(.acquire) >= focusable_min_px and view.size_h.load(.acquire) >= focusable_min_px;
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
    marker.lat("cef.layout", "node={d}", .{view.node_id});
    syncBounds(view);
    maybeCreateBrowser(view);
    releasePopoverGrab(view);
}

/// Where a hidden view's window lives. It has to be MAPPED, because Chromium
/// does not run a page whose window is not: an extension's background page
/// sat at "did not finish starting" for exactly that reason. X clips a child
/// to its parent, so a window parked this far outside the parent's bounds is
/// mapped, running, and completely invisible.
const park_origin: c_int = -8192;

/// A parked page keeps the size it had on screen, and a page never shown takes
/// the size of the last one that was, so showing it is a move and not a resize:
/// a resize costs the page a relayout and a full raster before its first frame,
/// which on a tab switch was most of the time to pixels. 1024x768 until any
/// page has been on screen.
var shown_w: c_uint = 1024;
var shown_h: c_uint = 768;

const ParkSize = struct { w: c_uint, h: c_uint };

fn parkSize(view: *View) ParkSize {
    if (view.placed and view.bounds.w >= focusable_min_px and view.bounds.h >= focusable_min_px) return .{ .w = view.bounds.w, .h = view.bounds.h };
    return .{ .w = shown_w, .h = shown_h };
}

/// Puts the view's window where a hidden view's window belongs, and keeps it
/// mapped. Sized like the page on screen rather than 1x1 so the page lays out
/// the way it will when the view is shown (see `parkSize`).
fn parkContainer(view: *View) void {
    if (view.container == 0) return;
    // A page parked while it holds X input focus (a tab switched away from, a
    // page standing aside for a palette) keeps it on a window nobody sees, and
    // no focus-widget change brings it home: every key after that, Escape
    // included, went to the parked page.
    const cef_window = view.cef_window.load(.acquire);
    if (cef_window != 0 and x11.focused() == @as(x11.Window, @intCast(cef_window))) x11.focusToplevel(view.widget);
    const size = parkSize(view);
    x11.moveResize(view.container, park_origin, park_origin, size.w, size.h);
    x11.show(view.container);
    layoutContents(view, size.w, size.h);
    parkPage(view);
}

/// The container stays mapped while parked, and Chromium takes a page whose
/// window is mapped for one on screen: every background tab kept drawing
/// frames and running timers at full rate with `visibilityState` "visible".
/// CEF's `was_hidden` is for windowless browsers only, and Chromium does not
/// follow its window being unmapped from outside, but it does follow the
/// window being minimized, so the page's window is marked so. Only for a view
/// that has been on screen: a page never shown (an extension's background page,
/// a tab opened behind the current one) still has to load.
///
/// Not at once: a page coming back from minimized takes ~120 ms to draw its
/// first frame, against one frame for a page that kept running, and that is
/// the delay a tab switch shows. The last few pages parked stay running for a
/// while, the way Chrome keeps recently used tabs ready, so switching between
/// a handful of tabs is instant and everything else is throttled.
const live_parked_max = 3;
const live_parked_ms: c_uint = 120 * std.time.ms_per_s;
var live_parked: std.ArrayList(*View) = .empty;

fn parkPage(view: *View) void {
    if (!view.placed or view.page_minimized) return;
    if (view.cef_window.load(.acquire) == 0) return;
    if (std.mem.indexOfScalar(*View, live_parked.items, view) != null) return;
    live_parked.append(alloc, view) catch return minimizePage(view);
    view.park_timer = glib.timeoutAdd(live_parked_ms, &onParkTimer, view);
    if (live_parked.items.len > live_parked_max) minimizePage(live_parked.items[0]);
}

fn onParkTimer(data: ?*anyopaque) callconv(.c) c_int {
    const view: *View = @ptrCast(@alignCast(data.?));
    view.park_timer = 0;
    minimizePage(view);
    return 0;
}

/// Drops `view` from the pages kept running while parked.
fn forgetParked(view: *View) void {
    if (view.park_timer != 0) {
        _ = glib.Source.remove(view.park_timer);
        view.park_timer = 0;
    }
    if (std.mem.indexOfScalar(*View, live_parked.items, view)) |i| _ = live_parked.orderedRemove(i);
}

fn minimizePage(view: *View) void {
    forgetParked(view);
    const page = view.cef_window.load(.acquire);
    if (page == 0 or view.page_minimized) return;
    x11.setMinimized(@intCast(page), true);
    view.page_minimized = true;
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
    view.in_popover.store(false, .release);
    marker.lat("cef.unmap", "node={d}", .{view.node_id});
    disconnectLayout(view);
    disconnectActive(view);
    motionFinish(view, false);
    // Parked, not hidden: a background tab whose window is unmapped stops
    // running, and a tab the user comes back to has to still be the page they
    // left. `onMap` puts it back where it belongs.
    //
    // Once the commit that hid it is through: a tab switch hides the old page
    // before it shows the new one, and parking at once uncovered the window
    // behind it, black, for the 3 ms until the new page was on screen and the
    // frame after that until it had drawn. Left in place, the old page's
    // pixels stay until the new page's replace them, as in Chrome.
    //
    // And once the window has painted that commit: parking makes X round
    // trips (3 to 9 ms under XWayland), and run before the frame they held
    // back the app's own chrome, the sidebar row marked as on show.
    if (view.park_source != 0 or view.park_clock != null) return;
    if (gtk.Widget.getFrameClock(view.widget)) |clock| {
        view.park_clock = clock;
        _ = gobject.Object.ref(clock.as(gobject.Object));
        view.park_handler = gobject.signalConnectData(clock.as(gobject.Object), "after-paint", @ptrCast(&onParkPaint), view, null, .{});
        gdk.FrameClock.requestPhase(clock, .{ .after_paint = true });
        return;
    }
    view.park_source = glib.timeoutAdd(0, &onParkSource, view);
}

fn onParkSource(data: ?*anyopaque) callconv(.c) c_int {
    const view: *View = @ptrCast(@alignCast(data.?));
    view.park_source = 0;
    parkUnmapped(view);
    return 0;
}

fn onParkPaint(_: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const view: *View = @ptrCast(@alignCast(data.?));
    dropParkPaint(view);
    parkUnmapped(view);
}

fn dropParkPaint(view: *View) void {
    const clock = view.park_clock orelse return;
    gobject.signalHandlerDisconnect(clock.as(gobject.Object), view.park_handler);
    gobject.Object.unref(clock.as(gobject.Object));
    view.park_clock = null;
    view.park_handler = 0;
}

fn parkUnmapped(view: *View) void {
    marker.lat("cef.park", "node={d}", .{view.node_id});
    defer marker.lat("cef.parked", "node={d}", .{view.node_id});
    if (gtk.Widget.getMapped(view.widget) == 0) parkContainer(view);
}

fn onDestroy(_: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const view: *View = @ptrCast(@alignCast(data.?));
    _ = live_views.remove(@intFromPtr(view));
    if (focused_view == view) focused_view = null;
    forgetParked(view);
    motionFinish(view, false);
    dropAsideStill(view);
    if (view.park_source != 0) {
        _ = glib.Source.remove(view.park_source);
        view.park_source = 0;
    }
    dropParkPaint(view);
    closeNativeMenu(view);
    disarmCreateTimer(view);
    if (view.deferred_timer != 0) {
        _ = glib.Source.remove(view.deferred_timer);
        view.deferred_timer = 0;
    }
    disconnectLayout(view);
    disconnectActive(view);
    if (browserOf(view)) |browser| {
        if (browser.get_host) |get_host| {
            const host = get_host(browser);
            if (host != null) {
                defer ref.releaseOwned(host);
                // Devtools first, for the ordering `closeBrowsersInOrder`
                // documents: its browser cannot outlive the one it inspects.
                if (view.devtools_window.load(.acquire) != 0) {
                    if (host.*.close_dev_tools) |close_dev_tools| close_dev_tools(host);
                }
                if (host.*.close_browser) |close| close(host, 1);
            }
        }
        // The container outlives the close. Destroying it here took the
        // browser's X window with it while the GPU process still had a swap
        // in flight on it, and tearing down that window's compositor then
        // waited in Mesa's DRI3 swap barrier for a Present that a destroyed
        // window never completes: the GPU main thread stopped for every page
        // until Chromium's watchdog killed it some 30 s later. Parked off
        // screen, still mapped, the swap completes and nothing is seen.
        if (view.container != 0) {
            const size = parkSize(view);
            x11.moveResize(view.container, park_origin, park_origin, size.w, size.h);
            view.closing_container = view.container;
            view.container = 0;
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
        // A toplevel with no surface yet is not realized: the create timer
        // gets here first when the first commit lands before the window does,
        // and its next tick finds the window.
        const native = gtk.Widget.getNative(view.widget);
        if (native == null or gtk.Native.getSurface(native.?) == null) return;
        if (!view.warned_no_parent) {
            view.warned_no_parent = true;
            std.debug.print("ND_WARN WebView engine=chromium: the toplevel has no X11 window (Wayland without the x11 backend pin?); browser not created\n", .{});
        }
        return;
    }
    syncBounds(view);
    const hidden = gtk.Widget.getMapped(view.widget) == 0;
    const park = parkSize(view);
    const start_x: c_int = if (hidden) park_origin else view.bounds.x;
    const start_y: c_int = if (hidden) park_origin else view.bounds.y;
    const start_w: c_uint = if (hidden) park.w else view.bounds.w;
    const start_h: c_uint = if (hidden) park.h else view.bounds.h;
    const container = x11.createChild(parent, start_x, start_y, start_w, start_h);
    if (container == 0) {
        std.debug.print("ND_WARN WebView engine=chromium: could not create the embedding window; browser not created\n", .{});
        return;
    }
    view.container = container;
    view.container_parent = parent;
    armAccelTimer();
    const mapped = gtk.Widget.getMapped(view.widget) != 0;
    if (mapped) {
        x11.show(container);
    } else {
        x11.moveResize(container, park_origin, park_origin, park.w, park.h);
        x11.show(container);
    }
    tr("embed node={d} parent=0x{x} container=0x{x} bounds={d}x{d}+{d}+{d} mapped={}", .{
        view.node_id, parent, container, view.bounds.w, view.bounds.h, view.bounds.x, view.bounds.y, mapped,
    });

    var window_info = std.mem.zeroes(c.cef_window_info_t);
    window_info.size = @sizeOf(c.cef_window_info_t);
    window_info.parent_window = container;
    window_info.bounds = .{
        .x = 0,
        .y = 0,
        .width = @intCast(@max(start_w, 1)),
        .height = @intCast(@max(start_h, 1)),
    };
    // Explicit rather than inferred: the default depends on how the browser is
    // hosted, and this engine always hosts it the same way.
    const action_popup = isActionPopup(view);
    window_info.runtime_style = if (chromeStyle() and !action_popup)
        @intCast(c.CEF_RUNTIME_STYLE_CHROME)
    else
        @intCast(c.CEF_RUNTIME_STYLE_ALLOY);
    tr("createBrowser node={d} action_popup={}", .{ view.node_id, action_popup });

    var browser_settings = std.mem.zeroes(c.cef_browser_settings_t);
    browser_settings.size = @sizeOf(c.cef_browser_settings_t);
    // The app draws its own zoom indicator ("Page zoom reported to the app").
    browser_settings.chrome_zoom_bubble = c.STATE_DISABLED;

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
    if (!action_popup and viewsHostedFor(start)) {
        view.views_hosted = true;
        if (!createViewsBrowser(view, start, start_w, start_h, context)) {
            view.created = false;
            std.debug.print("ND_WARN WebView engine=chromium: the Views-hosted browser could not be scheduled\n", .{});
        }
        return;
    }
    if (api.create_browser(&window_info, view.client.handOut(), &url, &browser_settings, null, context) == 0) {
        view.created = false;
        std.debug.print("ND_WARN WebView engine=chromium: cef_browser_host_create_browser failed\n", .{});
    }
}

// ============================================================================
// Views-hosted embedding
// ============================================================================

const BrowserViewDelegateObj = ref.Counted(c.cef_browser_view_delegate_t, *View);
const WindowDelegateObj = ref.Counted(c.cef_window_delegate_t, *View);

const ViewsCreate = struct {
    view: *View,
    url: []u8,
    w: c_uint,
    h: c_uint,
    context: [*c]c.cef_request_context_t,
};
const ViewsCreateObj = ref.Counted(c.cef_task_t, ViewsCreate);

/// Views objects are CEF UI thread only, and createBrowser runs on the GTK one.
/// `context` arrives with a reference for CEF to consume.
fn createViewsBrowser(view: *View, url: []const u8, w: c_uint, h: c_uint, context: [*c]c.cef_request_context_t) bool {
    const api = loader.loaded() orelse return false;
    const url_copy = alloc.dupe(u8, url) catch return false;
    const task = ViewsCreateObj.create(.{ .view = view, .url = url_copy, .w = w, .h = h, .context = context }) orelse {
        alloc.free(url_copy);
        return false;
    };
    task.cef.execute = &runViewsCreate;
    const posted = api.post_task(c.TID_UI, task.handOut()) != 0;
    if (!posted) task.drop();
    task.drop();
    return posted;
}

fn runViewsCreate(self: [*c]c.cef_task_t) callconv(.c) void {
    const job = ViewsCreateObj.of(self).payload;
    defer alloc.free(job.url);
    const api = loader.loaded() orelse return;
    const view = job.view;
    view.views_size = .{ .w = job.w, .h = job.h };

    var url = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&url);
    _ = setStr(&url, job.url);
    var settings = std.mem.zeroes(c.cef_browser_settings_t);
    settings.size = @sizeOf(c.cef_browser_settings_t);
    settings.chrome_zoom_bubble = c.STATE_DISABLED;

    const delegate = BrowserViewDelegateObj.create(view) orelse return;
    delegate.cef.get_chrome_toolbar_type = &viewsToolbarType;
    delegate.cef.get_browser_runtime_style = &viewsRuntimeStyle;
    const browser_view = api.browser_view_create(view.client.handOut(), &url, &settings, null, job.context, delegate.handOut());
    delegate.drop();
    if (browser_view == null) {
        std.debug.print("ND_WARN WebView engine=chromium: cef_browser_view_create failed\n", .{});
        return;
    }
    view.views_browser_view = browser_view;

    const window_delegate = WindowDelegateObj.create(view) orelse return;
    window_delegate.cef.on_window_created = &viewsWindowCreated;
    window_delegate.cef.is_frameless = &viewsFrameless;
    window_delegate.cef.get_initial_bounds = &viewsInitialBounds;
    window_delegate.cef.on_window_destroyed = &viewsWindowDestroyed;
    window_delegate.cef.can_close = &viewsCanClose;
    const window = api.window_create_top_level(window_delegate.handOut());
    window_delegate.drop();
    if (window == null) std.debug.print("ND_WARN WebView engine=chromium: cef_window_create_top_level failed\n", .{});
    // The returned reference is the same object on_window_created was handed
    // and kept; this one is not needed.
    ref.releaseParam(window);
}

fn viewsToolbarType(_: [*c]c.cef_browser_view_delegate_t, browser_view: [*c]c.cef_browser_view_t) callconv(.c) c.cef_chrome_toolbar_type_t {
    ref.releaseParam(browser_view);
    return c.CEF_CTT_NORMAL;
}

fn viewsRuntimeStyle(_: [*c]c.cef_browser_view_delegate_t) callconv(.c) c.cef_runtime_style_t {
    return c.CEF_RUNTIME_STYLE_CHROME;
}

fn viewsFrameless(_: [*c]c.cef_window_delegate_t, window: [*c]c.cef_window_t) callconv(.c) c_int {
    ref.releaseParam(window);
    return 1;
}

fn viewsInitialBounds(self: [*c]c.cef_window_delegate_t, window: [*c]c.cef_window_t) callconv(.c) c.cef_rect_t {
    ref.releaseParam(window);
    const view = WindowDelegateObj.of(self).payload;
    return .{ .x = 0, .y = 0, .width = @intCast(@max(view.views_size.w, 1)), .height = @intCast(@max(view.views_size.h, 1)) };
}

/// The window exists and is not mapped yet. It goes into the container before
/// it is shown, so it is never a top-level the window manager takes over, and
/// the Chrome toolbar that came with the BrowserView is hidden before its
/// first frame.
fn viewsWindowCreated(self: [*c]c.cef_window_delegate_t, window: [*c]c.cef_window_t) callconv(.c) void {
    const view = WindowDelegateObj.of(self).payload;
    if (window == null) return;
    view.views_window = window;
    view.views_window_live.store(true, .release);
    const panel: [*c]c.cef_panel_t = @ptrCast(window);
    if (panel.*.set_to_fill_layout) |fill| {
        if (fill(panel)) |layout| ref.releaseParam(layout);
    }
    const browser_view = view.views_browser_view orelse return;
    if (panel.*.add_child_view) |add| {
        ref.addRefParam(browser_view);
        add(panel, @ptrCast(browser_view));
    }
    if (browser_view.get_chrome_toolbar) |get| {
        const toolbar = get(browser_view);
        if (toolbar != null) {
            if (toolbar.*.set_visible) |set| set(toolbar, 0);
            ref.releaseParam(toolbar);
        } else {
            std.debug.print("ND_WARN WebView engine=chromium: the Views-hosted browser has no Chrome toolbar\n", .{});
        }
    }
    const api = loader.loaded() orelse return;
    const xid: x11.Window = if (window.*.get_window_handle) |get| @intCast(get(window)) else 0;
    const dpy = api.get_xdisplay() orelse return;
    x11.reparentOn(dpy, xid, view.container, 0, 0);
    tr("viewsWindow node={d} xid=0x{x} container=0x{x}", .{ view.node_id, xid, view.container });
    if (window.*.show) |show| show(window);
}

/// A delegate that leaves `can_close` unset answers false through CEF's C to
/// C++ wrapper, and `close()` then never takes the window down.
fn viewsCanClose(_: [*c]c.cef_window_delegate_t, window: [*c]c.cef_window_t) callconv(.c) c_int {
    ref.releaseParam(window);
    return 1;
}

fn viewsWindowDestroyed(self: [*c]c.cef_window_delegate_t, window: [*c]c.cef_window_t) callconv(.c) void {
    ref.releaseParam(window);
    const view = WindowDelegateObj.of(self).payload;
    view.views_window_live.store(false, .release);
    tr("viewsWindow node={d} destroyed", .{view.node_id});
}

/// The browser is gone; its window goes with it. CEF UI thread.
fn releaseViews(view: *View) void {
    if (view.views_window) |window| {
        view.views_window = null;
        tr("viewsWindow node={d} close", .{view.node_id});
        if (window.close) |close| close(window);
        ref.releaseParam(window);
    }
    if (view.views_browser_view) |browser_view| {
        view.views_browser_view = null;
        ref.releaseParam(browser_view);
    }
}

// ============================================================================
// Geometry
// ============================================================================

/// Re-reads the widget's allocation and moves the embedding window to match.
/// Cheap enough to run per layout pass: the early-out is one compute_bounds.
fn syncBounds(view: *View) void {
    // A parked view has no meaningful allocation to track, and moving its
    // window to one would put it back on screen.
    if (gtk.Widget.getMapped(view.widget) == 0) return;
    // Same for a page standing aside: a dialog is up over this window, and
    // tracking the allocation would put the page straight back on top of it.
    // The command bar is the exception, cut out of the page instead.
    const bar = barCardOver(view);
    if (bar == null and dialogOverView(view)) {
        setUnderBar(view, false);
        return standAside(view);
    }
    view.aside = bar != null;
    dropAsideStill(view);
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

    var next: Bounds = .{
        .x = @intFromFloat(@round((@as(f64, rect.f_origin.f_x) + tx) * scale)),
        .y = @intFromFloat(@round((@as(f64, rect.f_origin.f_y) + ty) * scale)),
        .w = @intFromFloat(@max(@round(@as(f64, rect.f_size.f_width) * scale), 1)),
        .h = @intFromFloat(@max(@round(@as(f64, rect.f_size.f_height) * scale), 1)),
    };
    const shift = ndchrome.pageMotionShift(view.widget);
    if (shift != null and !view.moving and bar == null) {
        // The first frames of a slide only put the still up: the window
        // keeps its place until it is out of sight.
        if (motionBegin(view)) return;
    }
    if (view.moving and view.motion_hide_frame != 0) {
        // The still goes under the page before the page goes: the window
        // keeps its place, on top, until a frame with the still has been
        // drawn, or cutting it away shows the frame before it, which has
        // nothing there.
        if (frameCounter(view) < view.motion_hide_frame) {
            // A slide that is already over lays out no more frames of its own.
            gtk.Widget.queueAllocate(view.widget);
            return;
        }
        hideForMotion(view);
    }
    if (view.moving) {
        if (shift) |dx| {
            const d: i64 = @intFromFloat(@round(dx * scale));
            if (gtk.Widget.getDirection(view.widget) != .rtl) next.x += @intCast(d);
            next.w = @intCast(@max(@as(i64, next.w) - d, 1));
        }
    }
    // No unchanged early-out: under XWayland both the container and CEF's
    // inner window can be reconfigured behind GTK's back (the compositor's
    // resizes land on the X windows first), and a stale cache here is what
    // left the webview stuck at the wrong size after a live resize. The
    // layout signal only fires on frames GTK actually laid the toplevel out,
    // so re-asserting costs one request per real layout, not one per tick.
    //
    // A view shown again (a tab switched to) is mapped before GTK has
    // allocated it, and until its next layout it reads as 1x1 at the slot's
    // corner. Moving the page there resized it to nothing and back, a relayout
    // and a full raster before the tab's first frame; it goes back where it
    // was last on show instead, and the layout pass that follows corrects it
    // if the slot has moved since.
    const unallocated = next.w <= 1 or next.h <= 1;
    const reuse = unallocated and view.placed and view.bounds.w > 1 and view.bounds.h > 1;
    if (reuse) next = view.bounds;
    view.bounds = next;
    view.size_w.store(next.w, .release);
    view.size_h.store(next.h, .release);
    // GTK focus follows X focus off a view too small to hold the keyboard:
    // Tab out of the page otherwise walked onto an app's 2x2 helper browser
    // and every key after that went to a view nobody can see.
    gtk.Widget.setFocusable(view.widget, @intFromBool(focusEligible(view)));
    if (view.container == 0) return;
    // The widget can have been relocated into another window since the last
    // pass (`moveNode`); GTK moves its own hierarchy and knows nothing about
    // the X child the browser lives in.
    const parent = x11.toplevelXid(view.widget);
    if (parent != 0 and parent != view.container_parent) {
        x11.reparent(view.container, parent, next.x, next.y);
        view.container_parent = parent;
    }
    x11.moveResize(view.container, next.x, next.y, next.w, next.h);
    view.placed = true;
    if (next.w >= focusable_min_px and next.h >= focusable_min_px) {
        shown_w = next.w;
        shown_h = next.h;
    }
    forgetParked(view);
    if (view.page_minimized) {
        const page = view.cef_window.load(.acquire);
        if (page != 0) x11.setMinimized(@intCast(page), false);
        view.page_minimized = false;
    }
    var bar_rect: ?graphene.Rect = null;
    if (bar) |card| {
        var r: graphene.Rect = undefined;
        if (gtk.Widget.computeBounds(card, native_widget, &r) != 0) bar_rect = r;
    }
    if (view.moving and !syncMotion(view, shift == null)) {
        layoutContents(view, next.w, next.h);
        return;
    }
    if (!reuse) syncShape(view, native_widget, rect, scale, bar_rect);
    layoutContents(view, next.w, next.h);
    setUnderBar(view, bar != null);
}

// ---- A sliding sidebar -------------------------------------------------------
// While the split the page is in slides its sidebar (src/gtk/chrome.zig), the
// page does not follow it frame by frame: Chromium re-laying out and
// re-rastering on every frame of a 200 ms slide is what a slow machine drops
// frames on, and the page lags its window anyway. Instead the page is
// swapped for a still of itself, which GTK stretches over the page's
// rectangle as it moves, and its window, cut to nothing, takes the size the
// slide lands on right away. The still goes once the slide is over and the
// page has painted at that size, so there is never a frame of stale or blank
// page.

/// How long a page that never reports a paint keeps its still past the end of
/// the slide.
const motion_paint_patience_ms: c_uint = 400;

var motion_live: ?bool = null;

/// ND_SPLIT_PAGE_MOTION=live: the page follows the slide frame by frame, for
/// comparing the two.
fn motionStillsWanted() bool {
    const live = motion_live orelse blk: {
        const v = std.c.getenv("ND_SPLIT_PAGE_MOTION");
        const on = v != null and std.mem.eql(u8, std.mem.span(v.?), "live");
        motion_live = on;
        break :blk on;
    };
    return !live;
}

/// Takes the still and puts it up in the page's place. False when there is
/// nothing to take one of, which leaves the page following the slide live.
fn motionBegin(view: *View) bool {
    if (!motionStillsWanted() or view.container == 0 or !view.placed) return false;
    if (view.bounds.w < 32 or view.bounds.h < 32 or view.devtoolsOpen()) return false;
    const began = glib.getMonotonicTime();
    const page = view.cef_window.load(.acquire);
    const still = x11.capture(if (page != 0) @intCast(page) else view.container) orelse return false;
    view.moving = true;
    view.still = still;
    view.motion_started_us = began;
    view.motion_ended_us = 0;
    view.painted_gen = view.probe_gen;
    view.probe_bounds = view.bounds;
    gtk.Picture.setPaintable(@ptrCast(@alignCast(view.widget)), @ptrCast(still));
    view.motion_hide_frame = frameCounter(view) + 2;
    if (motionTraced()) std.debug.print("ND_PAGE_MOTION_BEGIN node={d} still={d}x{d} capture_us={d}\n", .{
        view.node_id, gdk.Texture.getWidth(still), gdk.Texture.getHeight(still), glib.getMonotonicTime() - began,
    });
    return true;
}

fn frameCounter(view: *View) i64 {
    const clock = gtk.Widget.getFrameClock(view.widget) orelse return 0;
    return gdk.FrameClock.getFrameCounter(clock);
}

/// Cuts the page's window down to one pixel in its bottom corner. Cut to
/// nothing, the X server reports the window fully obscured and Chromium stops
/// drawing it, so the page would never paint at the size the slide lands it
/// on until it was back on show, stale for a frame or more. One pixel keeps
/// it drawing, and for the length of a slide nobody sees it.
fn hideForMotion(view: *View) void {
    x11.keepExposedContents(x11.toplevelXid(view.widget));
    const corner = [_]x11.Rect{.{ .x = 0, .y = @intCast(@max(@as(i64, view.bounds.h) - 1, 0)), .width = 1, .height = 1 }};
    x11.setShape(view.container, &corner);
    view.shaped = true;
    view.motion_hide_frame = 0;
}

/// One layout of a page under a slide, after its window has been moved to
/// `view.bounds`. True once the still can go: the slide is over and the page
/// has painted at the size it landed on.
fn syncMotion(view: *View, ended: bool) bool {
    if (view.probe_bounds.w != view.bounds.w or view.probe_bounds.h != view.bounds.h) probePaint(view);
    if (!ended) return false;
    if (view.motion_ended_us == 0) {
        view.motion_ended_us = glib.getMonotonicTime();
        view.motion_timer = glib.timeoutAdd(motion_paint_patience_ms, &onMotionPatience, view);
    }
    if (view.painted_gen != view.probe_gen) return false;
    motionFinish(view, false);
    return true;
}

fn motionFinish(view: *View, timed_out: bool) void {
    if (!view.moving) return;
    view.moving = false;
    if (view.motion_timer != 0) {
        _ = glib.Source.remove(view.motion_timer);
        view.motion_timer = 0;
    }
    gtk.Picture.setPaintable(@ptrCast(@alignCast(view.widget)), null);
    if (view.still) |still| gobject.Object.unref(@ptrCast(@alignCast(still)));
    view.still = null;
    const now = glib.getMonotonicTime();
    if (motionTraced()) std.debug.print("ND_PAGE_MOTION node={d} held_ms={d:.1} after_end_ms={d:.1} resizes={d} timed_out={}\n", .{
        view.node_id,
        @as(f64, @floatFromInt(now - view.motion_started_us)) / 1000,
        @as(f64, @floatFromInt(if (view.motion_ended_us != 0) now - view.motion_ended_us else 0)) / 1000,
        view.probe_gen -% view.painted_gen +% 1,
        timed_out,
    });
}

var motion_trace_on: ?bool = null;

fn motionTraced() bool {
    return motion_trace_on orelse blk: {
        const on = std.c.getenv("ND_MOTION_TRACE") != null;
        motion_trace_on = on;
        break :blk on;
    };
}

/// Asks the page to answer once it has drawn a frame at the window's current
/// width: two animation frames after `innerWidth` reaches it, the second so
/// the frame carrying that width has been handed to the compositor.
fn probePaint(view: *View) void {
    view.probe_gen +%= 1;
    view.probe_bounds = view.bounds;
    if (!view.cdp_ready) {
        view.painted_gen = view.probe_gen;
        return;
    }
    var buf: [512]u8 = undefined;
    const params = std.fmt.bufPrint(&buf, "{{\"expression\":\"new Promise(r=>{{const w={d},t=performance.now(),f=()=>{{if(Math.abs(innerWidth*devicePixelRatio-w)<3||performance.now()-t>{d})requestAnimationFrame(()=>r(1));else requestAnimationFrame(f)}};requestAnimationFrame(f)}})\",\"awaitPromise\":true,\"returnByValue\":true}}", .{ view.bounds.w, motion_paint_patience_ms }) catch return;
    if (!cdpSend(view, "Runtime.evaluate", params, .{ .painted = view.probe_gen })) view.painted_gen = view.probe_gen;
}

fn motionPainted(view: *View, gen: u32) void {
    if (gen != view.probe_gen or !view.moving) return;
    view.painted_gen = gen;
    if (view.motion_ended_us != 0) syncBounds(view);
}

fn onMotionPatience(data: ?*anyopaque) callconv(.c) c_int {
    const view: *View = @ptrCast(@alignCast(data.?));
    if (!live_views.contains(@intFromPtr(view))) return 0;
    view.motion_timer = 0;
    if (!view.moving) return 0;
    tr("motionPatience node={d} probe={d} painted={d}", .{ view.node_id, view.probe_gen, view.painted_gen });
    motionFinish(view, true);
    syncBounds(view);
    return 0;
}


/// The command bar's card when the bar is the dialog up over `view`'s window
/// (commandpalette.zig). The bar is a card on a light scrim, so the page is
/// left on show under it, the way AppKit shows it, rather than taken away like
/// it is for a sheet.
fn barCardOver(view: *View) ?*gtk.Widget {
    const root = gtk.Widget.getRoot(view.widget) orelse return null;
    const widget: *gtk.Widget = @ptrCast(@alignCast(root));
    const dialog: *adw.Dialog = blk: {
        if (gobject.ext.cast(adw.ApplicationWindow, widget)) |w| break :blk adw.ApplicationWindow.getVisibleDialog(w) orelse return null;
        if (gobject.ext.cast(adw.Window, widget)) |w| break :blk adw.Window.getVisibleDialog(w) orelse return null;
        return null;
    };
    const dw: *gtk.Widget = @ptrCast(@alignCast(dialog));
    if (gtk.Widget.hasCssClass(dw, "nd-palette") == 0) return null;
    return findCssClass(dw, "nd-palette-card");
}

fn findCssClass(w: *gtk.Widget, class: [:0]const u8) ?*gtk.Widget {
    if (gtk.Widget.hasCssClass(w, class) != 0) return w;
    var child = gtk.Widget.getFirstChild(w);
    while (child) |kid| : (child = gtk.Widget.getNextSibling(kid)) {
        if (findCssClass(kid, class)) |found| return found;
    }
    return null;
}

/// The bar's scrim (basecss.zig, `dialog.nd-palette ... > dimming`), drawn
/// into the page by Chromium's own overlay: GTK cannot paint over the page's
/// X window, and the overlay leaves the document alone.
const bar_dim_params = "{\"x\":0,\"y\":0,\"width\":32768,\"height\":32768,\"color\":{\"r\":0,\"g\":0,\"b\":0,\"a\":0.15}}";

/// Puts the page under the command bar or takes it back out: dimmed like the
/// rest of the window, the pointer passed through to the scrim (a click there
/// closes the bar, as on AppKit) and the keyboard kept on the bar.
fn setUnderBar(view: *View, under: bool) void {
    if (view.under_bar == under) return;
    view.under_bar = under;
    x11.setInputPassthrough(view.container, under);
    if (under) {
        _ = cdpSend(view, "DOM.enable", "", .ignore);
        _ = cdpSend(view, "Overlay.enable", "", .ignore);
        _ = cdpSend(view, "Overlay.highlightRect", bar_dim_params, .ignore);
    } else {
        _ = cdpSend(view, "Overlay.hideHighlight", "", .ignore);
        _ = cdpSend(view, "Overlay.disable", "", .ignore);
        _ = cdpSend(view, "DOM.disable", "", .ignore);
    }
    syncBrowserFocus(view);
}

/// Cuts the page's X window to what GTK shows of it: inside the rounded card
/// it sits in (`contentStyle="card"`, or a `card` box), and clear of whatever
/// GTK floats over it: a split view's sidebar sliding over the page, and the
/// layers of an overlay the page is the base of (a load bar, an error page).
/// A GTK widget cannot be drawn over an X child window, so without this the
/// card's corners were square and every one of those layers was hidden under
/// the page.
fn syncShape(view: *View, native: *gtk.Widget, page: graphene.Rect, scale: f64, bar: ?graphene.Rect) void {
    var card: ?graphene.Rect = null;
    if (ndchrome.cardAncestor(view.widget)) |cw| {
        var r: graphene.Rect = undefined;
        if (gtk.Widget.computeBounds(cw, native, &r) != 0) card = r;
    }
    var covers: [max_covers]graphene.Rect = undefined;
    // Corner radius per cover: the command bar's card is rounded, the rest
    // are square.
    var radii: [max_covers]f64 = .{0} ** max_covers;
    var cover_n: usize = 0;
    if (bar) |r| {
        covers[0] = r;
        radii[0] = bar_card_radius;
        cover_n = 1;
    }
    var cover_widgets: [max_covers]*gtk.Widget = undefined;
    const found = ndchrome.coversOver(view.widget, cover_widgets[0 .. max_covers - cover_n]);
    for (cover_widgets[0..found]) |cw| {
        var r: graphene.Rect = undefined;
        if (gtk.Widget.computeBounds(cw, native, &r) == 0) continue;
        // A hairline anchor (the find bar's) and a probe view's couple of
        // pixels are layout, not something to see: cutting them would put a
        // line or a speck of window background in the page.
        const cw_w = r.f_size.f_width;
        const cw_h = r.f_size.f_height;
        if (cw_w < 2 or cw_h < 2 or (cw_w < 8 and cw_h < 8)) continue;
        covers[cover_n] = r;
        cover_n += 1;
    }
    // A probe view a couple of pixels square (an extension action's badge
    // reader) sits wherever the layout leaves it, often in a card's corner,
    // and cutting it to nothing would stop it rendering what is read off it.
    const tiny = view.bounds.w < 32 or view.bounds.h < 32;
    if ((card == null and cover_n == 0) or tiny) {
        if (view.shaped) x11.clearShape(view.container);
        view.shaped = false;
        return;
    }
    const w: usize = @intCast(view.bounds.w);
    const h: usize = @intCast(view.bounds.h);
    const radius = ndchrome.card_radius;
    const px: f64 = page.f_origin.f_x;
    const py: f64 = page.f_origin.f_y;

    var rects: std.ArrayList(x11.Rect) = .empty;
    defer rects.deinit(alloc);
    // One device row at a time; rows with the same spans merge into one band,
    // so a plain card is a handful of rectangles plus its corner rows.
    const Span = [2]i32;
    var band_start: usize = 0;
    var band: [max_covers + 1]Span = undefined;
    var band_n: usize = 0;
    var y: usize = 0;
    while (y <= h) : (y += 1) {
        var spans: [max_covers + 1]Span = undefined;
        var n: usize = 0;
        if (y < h) {
            const ly = py + (@as(f64, @floatFromInt(y)) + 0.5) / scale;
            var lo: f64 = px;
            var hi: f64 = px + @as(f64, @floatFromInt(w)) / scale;
            var visible = true;
            if (card) |cr| {
                const top: f64 = cr.f_origin.f_y;
                const bottom: f64 = top + cr.f_size.f_height;
                if (ly < top or ly >= bottom) visible = false;
                var inset: f64 = 0;
                const r = @min(radius, @min(cr.f_size.f_width, cr.f_size.f_height) / 2);
                if (ly < top + r) inset = r - @sqrt(@max(0, r * r - (top + r - ly) * (top + r - ly)));
                if (ly > bottom - r) inset = r - @sqrt(@max(0, r * r - (ly - (bottom - r)) * (ly - (bottom - r))));
                lo = @max(lo, cr.f_origin.f_x + inset);
                hi = @min(hi, cr.f_origin.f_x + cr.f_size.f_width - inset);
            }
            if (visible and hi > lo) {
                spans[0] = .{ @intFromFloat(@round((lo - px) * scale)), @intFromFloat(@round((hi - px) * scale)) };
                n = 1;
                // Each layer over this row takes its run out of the spans.
                for (covers[0..cover_n], radii[0..cover_n]) |cv, cr| {
                    const c_top: f64 = cv.f_origin.f_y;
                    const c_bottom: f64 = c_top + cv.f_size.f_height;
                    if (ly < c_top or ly >= c_bottom) continue;
                    const c_r = @min(cr, @min(cv.f_size.f_width, cv.f_size.f_height) / 2);
                    var c_in: f64 = 0;
                    if (ly < c_top + c_r) c_in = c_r - @sqrt(@max(0, c_r * c_r - (c_top + c_r - ly) * (c_top + c_r - ly)));
                    if (ly > c_bottom - c_r) c_in = c_r - @sqrt(@max(0, c_r * c_r - (ly - (c_bottom - c_r)) * (ly - (c_bottom - c_r))));
                    const c_lo: i32 = @intFromFloat(@round((cv.f_origin.f_x + c_in - px) * scale));
                    const c_hi: i32 = @intFromFloat(@round((cv.f_origin.f_x + cv.f_size.f_width - c_in - px) * scale));
                    var next: [max_covers + 1]Span = undefined;
                    var m: usize = 0;
                    for (spans[0..n]) |sp| {
                        if (c_hi <= sp[0] or c_lo >= sp[1]) {
                            next[m] = sp;
                            m += 1;
                            continue;
                        }
                        if (c_lo > sp[0] and m < next.len) {
                            next[m] = .{ sp[0], c_lo };
                            m += 1;
                        }
                        if (c_hi < sp[1] and m < next.len) {
                            next[m] = .{ c_hi, sp[1] };
                            m += 1;
                        }
                    }
                    spans = next;
                    n = m;
                }
            }
        }
        var same = y < h and n == band_n;
        if (same) {
            for (spans[0..n], band[0..n]) |a, b| {
                if (a[0] != b[0] or a[1] != b[1]) same = false;
            }
        }
        if (same) continue;
        // Close the band that ends here.
        for (band[0..band_n]) |sp| {
            const r = shape.bandRect(sp[0], sp[1], band_start, y) orelse continue;
            rects.append(alloc, r) catch return;
        }
        band_start = y;
        band_n = n;
        band = spans;
    }
    x11.setShape(view.container, rects.items);
    view.shaped = true;
}

/// How many floating layers one page can be cut around.
const max_covers = 6;

/// `.nd-palette-card`'s border-radius in basecss.zig.
const bar_card_radius: f64 = 15;

/// The view's inner windows: the page browser, and the docked devtools beside
/// it when it is open. Reads the XIDs on the calling thread and hands the X
/// requests to `applyLayoutOnUi`, which is where they are allowed to run.
fn layoutContents(view: *View, w: c_uint, h: c_uint) void {
    const dock = view.devtools_container.load(.acquire);
    var plan: Layout = .{
        .page = view.cef_window.load(.acquire),
        .page_w = w,
        .page_h = h,
        .h = h,
    };
    if (dock != 0) {
        // A docked inspector is the whole view: the frontend lays its panels
        // out around the hole it keeps for the page, and the page goes in that
        // hole, stacked above it. Until the frontend has announced the hole the
        // page takes Chrome's own right-dock default, so it is in its usual
        // column for the frame or two before the first announcement.
        plan.dock = dock;
        plan.dock_x = 0;
        plan.dock_w = w;
        plan.inner = view.devtools_window.load(.acquire);
        const announced = view.page_w.load(.acquire);
        if (announced != 0) {
            plan.page_x = @intCast(view.page_x.load(.acquire));
            plan.page_y = @intCast(view.page_y.load(.acquire));
            plan.page_w = announced;
            plan.page_h = view.page_h.load(.acquire);
        } else {
            plan.page_w = w - dockWidth(w);
        }
    }
    if (onGtkThread()) {
        applyLayout(plan);
        return;
    }
    // A browser created or closed on the CEF UI thread lays its view out from
    // there; the requests belong on the GTK one.
    post(.{ .view = view, .name = "", .relayout = true });
}

/// True on the thread that owns GTK and GDK's connection. CEF's UI thread is
/// its own (`multi_threaded_message_loop`), and the IO and renderer threads
/// never reach any of this.
fn onGtkThread() bool {
    const api = loader.loaded() orelse return true;
    return api.currently_on(c.TID_UI) == 0;
}

/// Chrome's own right-dock default, floored at the width the frontend's toolbar
/// needs: below that the toolbar overflows to the right and takes the close
/// button off screen with it (397px on 151.3.23). The upper clamp leaves a
/// narrow view a page.
fn dockWidth(w: c_uint) c_uint {
    const wanted = w * 45 / 100;
    return @min(@max(wanted, 400), if (w > 240) w - 240 else w / 2);
}

/// One pass of the view's inner geometry, as window ids rather than a *View:
/// the plan is read on the GTK thread and applied on the CEF one, and a view
/// whose tab closed in between must not be dereferenced there. An id that has
/// since been destroyed raises BadWindow, which the trap in `x11` swallows.
const Layout = struct {
    /// CEF's own window, whose Linux platform delegate sizes it once from
    /// window_info.bounds and never follows the parent afterwards.
    page: usize = 0,
    page_x: c_int = 0,
    page_y: c_int = 0,
    page_w: c_uint = 0,
    page_h: c_uint = 0,
    /// The docked devtools, Chrome style only: our X child on the right, and
    /// CEF's devtools window inside it. Both 0 while the inspector is closed.
    dock: usize = 0,
    dock_x: c_int = 0,
    dock_w: c_uint = 0,
    inner: usize = 0,
    h: c_uint = 0,
};

/// GDK's connection, from the GTK thread, for every one of these windows,
/// including the two Chromium owns: XSetInputFocus and ConfigureWindow are not
/// restricted to a window's owner, and the alternative is worse. CEF's own
/// connection is only reachable through `cef_get_xdisplay`, which answers null
/// anywhere but the CEF UI thread, and issuing from there means pushing GDK's
/// error trap off the GTK thread. That trap list is per display and not thread
/// safe: doing it took the host down inside
/// `delete_outdated_error_traps` from `gdk_x11_display_error_trap_push`.
/// The origin of the two Chromium owns is re-asserted, not just their size:
/// they are children of containers this engine created with a black
/// background, so either one sitting at anything but 0,0 shows as a strip of
/// bare background beside the page or the inspector. Chromium moves its own
/// widget's window on paths this engine has no callback for, and the size-only
/// call left whatever origin it had.
fn applyLayout(plan: Layout) void {
    if (shutting_down) return;
    if (plan.dock != 0) {
        x11.moveResize(@intCast(plan.dock), plan.dock_x, 0, plan.dock_w, plan.h);
        if (plan.inner != 0) x11.moveResize(@intCast(plan.inner), 0, 0, plan.dock_w, plan.h);
    }
    if (plan.page == 0) return;
    x11.moveResize(@intCast(plan.page), plan.page_x, plan.page_y, plan.page_w, plan.page_h);
    if (plan.dock != 0) x11.raise(@intCast(plan.page));
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
    const get_frame = browser.get_main_frame orelse {
        tr("loadUrl: no get_main_frame", .{});
        return;
    };
    const frame = get_frame(browser);
    if (frame == null) {
        tr("loadUrl: no main frame for {s}", .{url});
        return;
    }
    defer ref.releaseOwned(frame);
    const load = frame.*.load_url orelse {
        tr("loadUrl: no load_url for {s}", .{url});
        return;
    };
    var s = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&s);
    if (!setStr(&s, url)) return;
    tr("loadUrl issued {s}", .{url});
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
    if (std.mem.eql(u8, cmd, "respondPermission")) return cmdRespondPermission(arg);
    if (std.mem.eql(u8, cmd, "respondDownload")) return cmdRespondDownload(arg);
    if (std.mem.eql(u8, cmd, "acceptExtensionInstall")) return cmdAcceptExtensionInstall();
    if (std.mem.eql(u8, cmd, "resetPermissions")) return cmdResetPermissions(view, arg);
    if (std.mem.eql(u8, cmd, "allowPopups")) return cmdAllowPopups(view, arg);
    if (std.mem.eql(u8, cmd, "pauseDownload") or std.mem.eql(u8, cmd, "resumeDownload") or std.mem.eql(u8, cmd, "cancelDownload")) return cmdControlDownload(cmd, arg);
    if (std.mem.eql(u8, cmd, "setContextMenuItems")) return cmdSetContextMenuItems(view, arg);
    if (std.mem.eql(u8, cmd, "listExtensions")) return cmdListExtensions(view, arg);
    if (std.mem.eql(u8, cmd, "watchExtensions")) return cmdWatchExtensions(view, arg);
    if (std.mem.eql(u8, cmd, "listExtensionActions")) return cmdListExtensionActions(view, arg);
    if (std.mem.eql(u8, cmd, "readExtensionAction")) return cmdReadExtensionAction(view, arg);
    if (std.mem.eql(u8, cmd, "triggerExtensionAction")) return cmdTriggerExtensionAction(view, arg);
    if (std.mem.eql(u8, cmd, "installExtension")) return cmdInstallExtension(view, arg);
    if (std.mem.eql(u8, cmd, "uninstallExtension")) return cmdUninstallExtension(view, arg);
    if (std.mem.eql(u8, cmd, "setExtensionEnabled")) return cmdSetExtensionEnabled(view, arg);

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
    } else if (std.mem.eql(u8, cmd, "startDownload")) {
        cmdStartDownload(view, arg);
    } else if (std.mem.eql(u8, cmd, "setMuted")) {
        cmdSetMuted(view, arg);
    } else if (std.mem.eql(u8, cmd, "setZoom")) {
        cmdSetZoom(view, arg);
    } else if (std.mem.eql(u8, cmd, "focus")) {
        cmdFocus(view);
    } else if (std.mem.eql(u8, cmd, "openDevTools")) {
        cmdOpenDevTools(view, arg);
    } else if (std.mem.eql(u8, cmd, "closeDevTools")) {
        cmdCloseDevTools(view);
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

fn clientGetKeyboardHandler(self: [*c]c.cef_client_t) callconv(.c) [*c]c.cef_keyboard_handler_t {
    return ClientObj.of(self).payload.keyboard_handler.handOut();
}

fn clientGetCommandHandler(self: [*c]c.cef_client_t) callconv(.c) [*c]c.cef_command_handler_t {
    return ClientObj.of(self).payload.command_handler.handOut();
}

fn clientGetRequestHandler(self: [*c]c.cef_client_t) callconv(.c) [*c]c.cef_request_handler_t {
    return ClientObj.of(self).payload.request_handler.handOut();
}

// ============================================================================
// cef_command_handler_t: no Chrome window and no Chrome UI
// ============================================================================

/// Commands Chrome answers by opening a window, a tab or a Chrome UI surface,
/// as the context menu offers them. `onChromeCommand` refuses these and every
/// other id outside `allowed_chrome_commands`; this list is what the native
/// context menu reads: it drops every item on it that is not also an open-link
/// one, so the menu never offers an entry that does nothing. The ones that
/// carry a URL reach the app as `newWindow` (on_before_popup,
/// on_open_urlfrom_tab, on_context_menu_command). The devtools commands are
/// deliberately absent: Chrome style docks the inspector inside the browser's
/// own contents container, so they open no window, and IDC_DEV_TOOLS is how
/// `openDevTools` itself is served.
const blocked_chrome_commands = [_][*:0]const u8{
    "IDC_NEW_WINDOW",
    "IDC_NEW_INCOGNITO_WINDOW",
    "IDC_NEW_TAB",
    "IDC_RESTORE_TAB",
    "IDC_CLOSE_WINDOW",
    "IDC_VIEW_SOURCE",
    "IDC_PRINT",
    "IDC_BASIC_PRINT",
    "IDC_TASK_MANAGER",
    "IDC_SHOW_HISTORY",
    "IDC_SHOW_DOWNLOADS",
    "IDC_SHOW_BOOKMARK_MANAGER",
    "IDC_CLEAR_BROWSING_DATA",
    "IDC_OPTIONS",
    "IDC_ABOUT",
    "IDC_MANAGE_EXTENSIONS",
    "IDC_OPEN_FILE",
    "IDC_CONTENT_CONTEXT_OPENLINKNEWTAB",
    "IDC_CONTENT_CONTEXT_OPENLINKNEWWINDOW",
    "IDC_CONTENT_CONTEXT_OPENLINKOFFTHERECORD",
    "IDC_CONTENT_CONTEXT_OPENLINKINPROFILE",
    "IDC_CONTENT_CONTEXT_OPENLINKBOOKMARKAPP",
    "IDC_CONTENT_CONTEXT_OPENIMAGENEWTAB",
    "IDC_CONTENT_CONTEXT_OPENAVNEWTAB",
    "IDC_CONTENT_CONTEXT_PICTUREINPICTURE",
    "IDC_CONTENT_CONTEXT_VIEWFRAMESOURCE",
    // Chrome answers these with a bubble anchored to browser chrome this
    // engine does not have, so they raise nothing at all when they run.
    "IDC_ROUTE_MEDIA",
    "IDC_CONTENT_CONTEXT_GENERATE_QR_CODE",
    "IDC_CONTENT_CONTEXT_SEARCHLENSFORIMAGE",
    "IDC_CONTENT_CONTEXT_TRANSLATE",
    // Opens Chromium's own password manager page.
    "IDC_CONTENT_CONTEXT_AUTOFILL_FALLBACK_PASSWORDS_IMPORT_PASSWORDS",
    // "Use enhanced spell check" asks through a Chromium dialog window of its own.
    "IDC_CONTENT_CONTEXT_SPELLING_TOGGLE",
};

/// Resolved once on the CEF UI thread, which is the only thread that asks.
/// -1 is what cef_id_for_command_id_name answers for a name this Chromium
/// build does not have, and it can never equal a real command id.
var blocked_chrome_ids: ?[blocked_chrome_commands.len]c_int = null;

fn chromeCommandBlocked(command_id: c_int) bool {
    if (blocked_chrome_ids == null) {
        const api = loader.loaded() orelse return false;
        var ids: [blocked_chrome_commands.len]c_int = undefined;
        for (blocked_chrome_commands, 0..) |name, i| ids[i] = api.id_for_command_id_name(name);
        blocked_chrome_ids = ids;
    }
    for (blocked_chrome_ids.?) |id| {
        if (id >= 0 and id == command_id) return true;
    }
    return false;
}

/// The subset of the list above that Chrome's own context menu offers for a
/// link, and that the app gets as `newWindow` with the link's URL. The
/// incognito item is not one of them: `newWindow` carries no privacy, so the
/// app would open the link in an ordinary tab under a label that promised
/// otherwise, and the item is dropped from the menu instead.
const open_link_commands = [_][*:0]const u8{
    "IDC_CONTENT_CONTEXT_OPENLINKNEWTAB",
    "IDC_CONTENT_CONTEXT_OPENLINKNEWWINDOW",
    "IDC_CONTENT_CONTEXT_OPENLINKINPROFILE",
    "IDC_CONTENT_CONTEXT_OPENLINKBOOKMARKAPP",
};

var open_link_ids: ?[open_link_commands.len]c_int = null;

fn openLinkCommand(command_id: c_int) bool {
    return openLinkIndex(command_id) != null;
}

/// Which of `open_link_commands` the id is.
fn openLinkIndex(command_id: c_int) ?usize {
    if (open_link_ids == null) {
        const api = loader.loaded() orelse return null;
        var ids: [open_link_commands.len]c_int = undefined;
        for (open_link_commands, 0..) |name, i| ids[i] = api.id_for_command_id_name(name);
        open_link_ids = ids;
    }
    for (open_link_ids.?, 0..) |id, i| {
        if (id >= 0 and id == command_id) return i;
    }
    return null;
}

/// How a page asked for a new window, in the words `newWindow` carries:
/// Chrome puts a foreground tab after the opener and a background one after
/// the opener's other children, so the app needs to know which it was.
fn dispositionName(disposition: c.cef_window_open_disposition_t) []const u8 {
    return switch (disposition) {
        c.CEF_WOD_NEW_BACKGROUND_TAB => "backgroundTab",
        c.CEF_WOD_NEW_WINDOW, c.CEF_WOD_OFF_THE_RECORD => "window",
        c.CEF_WOD_NEW_POPUP => "popup",
        else => "foregroundTab",
    };
}

fn postNewWindow(view: *View, url: ?[]u8, disposition: []const u8, user_gesture: bool) void {
    post(.{ .view = view, .name = "newWindow", .text = url, .extra = alloc.dupe(u8, disposition) catch null, .flag = user_gesture });
}

const ChromeCommandTask = struct { host: *c.cef_browser_host_t, command_id: c_int };
const ChromeCommandObj = ref.Counted(c.cef_task_t, ChromeCommandTask);

/// execute_chrome_command only runs on the CEF UI thread, and every webview
/// command arrives on the GTK one.
fn executeChromeCommand(host: *c.cef_browser_host_t, command_id: c_int) void {
    const api = loader.loaded() orelse return;
    if (api.currently_on(c.TID_UI) != 0) {
        if (host.execute_chrome_command) |exec| exec(host, command_id, c.CEF_WOD_CURRENT_TAB);
        return;
    }
    const task = ChromeCommandObj.create(.{ .host = host, .command_id = command_id }) orelse return;
    task.cef.execute = &runChromeCommandTask;
    // Same as cdp.send: the view's reference can go before the task runs.
    ref.addRefParam(host);
    if (api.post_task(c.TID_UI, task.handOut()) == 0) {
        ref.releaseParam(host);
        task.drop();
        return;
    }
    task.drop();
}

fn runChromeCommandTask(self: [*c]c.cef_task_t) callconv(.c) void {
    const call = ChromeCommandObj.of(self).payload;
    defer ref.releaseParam(call.host);
    if (call.host.execute_chrome_command) |exec| exec(call.host, call.command_id, c.CEF_WOD_CURRENT_TAB);
}


// ============================================================================
// App accelerators
// ============================================================================

/// The accelerators the app has declared, as GTK's canonical names, with the
/// action each one runs. Written on the GTK thread, read on the CEF UI thread
/// by `onPreKeyEvent`, which has to answer synchronously whether the page gets
/// the key.
var accel_lock: SpinLock = .{};
var accel_actions: std.StringHashMapUnmanaged([]u8) = .empty;
var accel_timer: c_uint = 0;

fn armAccelTimer() void {
    if (accel_timer != 0) return;
    accel_timer = glib.timeoutAdd(500, &onEngineTick, null);
}

/// GTK's focus-widget notification is an edge, and the state it describes can
/// be reached without one: clicking a widget that already has focus moves
/// nothing, while the browser has taken X input focus for itself in the
/// meantime and keeps every key. Re-asserting here is what makes the routing
/// hold whatever the user clicked last.
fn onEngineTick(_: ?*anyopaque) callconv(.c) c_int {
    refreshAccels();
    var xfocus: ?x11.Window = null;
    var it = live_views.keyIterator();
    while (it.next()) |key| {
        const view: *View = @ptrFromInt(key.*);
        if (gtk.Widget.getMapped(view.widget) == 0) {
            // Chromium gives a new browser's window X input focus even when it
            // is a background tab created hidden, and no focus sync runs for an
            // unmapped view: the keyboard then sat on a page nobody could see,
            // and the palette opened over it never got a key.
            const cef_window = view.cef_window.load(.acquire);
            if (cef_window == 0) continue;
            const focused = xfocus orelse blk: {
                const f = x11.focused();
                xfocus = f;
                break :blk f;
            };
            if (focused == @as(x11.Window, @intCast(cef_window))) {
                x11.focusToplevel(view.widget);
                xfocus = null;
            }
            continue;
        }
        syncBrowserFocus(view);
        // A page that stood aside for a dialog nobody told the engine about
        // closing would otherwise stay there.
        if (view.aside and !dialogOverView(view)) syncBounds(view);
    }
    return 1;
}

/// GTK thread. `list_action_descriptions` is every action that has an
/// accelerator, which is exactly what a `<menuitem accelerator=…>` registers.
fn refreshAccels() void {
    const app = gio.Application.getDefault() orelse return;
    const gtk_app = gobject.ext.cast(gtk.Application, app) orelse return;
    const described = gtk.Application.listActionDescriptions(gtk_app);
    defer glib.strfreev(@ptrCast(described));

    var next: std.StringHashMapUnmanaged([]u8) = .empty;
    var i: usize = 0;
    while (@intFromPtr(described[i]) != 0) : (i += 1) {
        const detailed = described[i];
        const accels = gtk.Application.getAccelsForAction(gtk_app, detailed);
        defer glib.strfreev(@ptrCast(accels));
        var j: usize = 0;
        while (@intFromPtr(accels[j]) != 0) : (j += 1) {
            const canonical = declaredAccel(std.mem.span(accels[j])) orelse continue;
            const action = alloc.dupe(u8, std.mem.span(detailed)) catch {
                alloc.free(canonical);
                continue;
            };
            next.put(alloc, canonical, action) catch {
                alloc.free(canonical);
                alloc.free(action);
            };
        }
    }

    accel_lock.lock();
    var old = accel_actions;
    accel_actions = next;
    accel_lock.unlock();
    var it = old.iterator();
    while (it.next()) |entry| {
        alloc.free(entry.key_ptr.*);
        alloc.free(entry.value_ptr.*);
    }
    old.deinit(alloc);
}

/// One spelling both sides can build: "ctrl+shift+t", "f12". The GTK side
/// converts what the app declared; `onPreKeyEvent` builds the same string from
/// a key event without touching GTK at all, because it runs on the CEF UI
/// thread and GTK belongs to the other one.
fn plainAccel(ctrl: bool, alt: bool, shift: bool, key: []const u8) ?[]u8 {
    var buf: [48]u8 = undefined;
    var used: usize = 0;
    inline for (.{ .{ ctrl, "ctrl+" }, .{ alt, "alt+" }, .{ shift, "shift+" } }) |pair| {
        if (pair[0]) {
            if (used + pair[1].len > buf.len) return null;
            @memcpy(buf[used..][0..pair[1].len], pair[1]);
            used += pair[1].len;
        }
    }
    if (used + key.len > buf.len) return null;
    for (key, 0..) |ch, i| buf[used + i] = std.ascii.toLower(ch);
    used += key.len;
    return alloc.dupe(u8, buf[0..used]) catch null;
}

/// GTK thread only: parsing an accelerator and naming a keyval are GTK's.
fn declaredAccel(spec: []const u8) ?[]u8 {
    const owned = alloc.dupeZ(u8, spec) catch return null;
    defer alloc.free(owned);
    var key: c_uint = 0;
    var mods: gdk.ModifierType = .{};
    if (gtk.acceleratorParse(owned, &key, &mods) == 0) return null;
    // Shift+Tab is its own keyval to X, and the key event side spells it as
    // the Tab key with shift held.
    if (key == gdk.KEY_ISO_Left_Tab) return plainAccel(mods.control_mask, mods.alt_mask, true, "tab");
    const name = gdk.keyvalName(gdk.keyvalToLower(key)) orelse return null;
    return plainAccel(mods.control_mask, mods.alt_mask, mods.shift_mask, std.mem.span(name));
}

/// A non-alphanumeric key by its Windows key code, as the GTK keyval name of
/// its unshifted and shifted symbol on a US layout. Chromium names keys this
/// way on every platform, and the menu declared them by keyval: "primary+plus"
/// is the equal key with shift held, "primary+shift+comma" the same key as
/// "primary+less".
const NamedKey = struct { vk: c_int, plain: []const u8, shifted: ?[]const u8 = null };
const named_keys = [_]NamedKey{
    .{ .vk = 0x08, .plain = "backspace" },
    .{ .vk = 0x09, .plain = "tab" },
    .{ .vk = 0x0D, .plain = "return" },
    .{ .vk = 0x20, .plain = "space" },
    .{ .vk = 0x21, .plain = "page_up" },
    .{ .vk = 0x22, .plain = "page_down" },
    .{ .vk = 0x23, .plain = "end" },
    .{ .vk = 0x24, .plain = "home" },
    .{ .vk = 0x25, .plain = "left" },
    .{ .vk = 0x26, .plain = "up" },
    .{ .vk = 0x27, .plain = "right" },
    .{ .vk = 0x28, .plain = "down" },
    .{ .vk = 0x2E, .plain = "delete" },
    .{ .vk = 0x6B, .plain = "kp_add" },
    .{ .vk = 0x6D, .plain = "kp_subtract" },
    .{ .vk = 0xBA, .plain = "semicolon", .shifted = "colon" },
    .{ .vk = 0xBB, .plain = "equal", .shifted = "plus" },
    .{ .vk = 0xBC, .plain = "comma", .shifted = "less" },
    .{ .vk = 0xBD, .plain = "minus", .shifted = "underscore" },
    .{ .vk = 0xBE, .plain = "period", .shifted = "greater" },
    .{ .vk = 0xBF, .plain = "slash", .shifted = "question" },
    .{ .vk = 0xC0, .plain = "grave", .shifted = "asciitilde" },
    .{ .vk = 0xDB, .plain = "bracketleft", .shifted = "braceleft" },
    .{ .vk = 0xDC, .plain = "backslash", .shifted = "bar" },
    .{ .vk = 0xDD, .plain = "bracketright", .shifted = "braceright" },
    .{ .vk = 0xDE, .plain = "apostrophe", .shifted = "quotedbl" },
};

/// The accelerators a key event can spell, or none for anything that is not
/// one: at least one of control or alt held, or a function key. A shifted
/// symbol spells two, the way GTK matches it either by the symbol it types
/// ("ctrl+plus") or by the key with shift held ("ctrl+shift+equal").
fn accelsOf(event: *const c.cef_key_event_t) [2]?[]u8 {
    const none: [2]?[]u8 = .{ null, null };
    const ctrl = (event.modifiers & c.EVENTFLAG_CONTROL_DOWN) != 0;
    const alt = (event.modifiers & c.EVENTFLAG_ALT_DOWN) != 0;
    const shift = (event.modifiers & c.EVENTFLAG_SHIFT_DOWN) != 0;
    const vk = event.windows_key_code;
    const is_fkey = vk >= 0x70 and vk <= 0x7B; // VK_F1..VK_F12
    if (!ctrl and !alt and !is_fkey) return none;
    if (is_fkey) {
        var num: [4]u8 = undefined;
        const text = std.fmt.bufPrint(&num, "f{d}", .{vk - 0x6F}) catch return none;
        return .{ plainAccel(ctrl, alt, shift, text), null };
    }
    if ((vk >= 'A' and vk <= 'Z') or (vk >= '0' and vk <= '9')) {
        const ch = [_]u8{@intCast(vk)};
        return .{ plainAccel(ctrl, alt, shift, &ch), null };
    }
    for (named_keys) |k| {
        if (k.vk != vk) continue;
        const as_symbol = if (shift) if (k.shifted) |sym| plainAccel(ctrl, alt, false, sym) else null else null;
        return .{ plainAccel(ctrl, alt, shift, k.plain), as_symbol };
    }
    return none;
}

const AccelTask = struct { action: []u8 };
const AccelObj = ref.Counted(c.cef_task_t, AccelTask);

/// CEF UI thread. The page keeps every key but the ones the app declared as
/// menu accelerators, which is the rule the AppKit host already follows: a
/// browser whose page has the keyboard still answers ctrl+t.
fn onPreKeyEvent(
    self: [*c]c.cef_keyboard_handler_t,
    browser: [*c]c.cef_browser_t,
    event: [*c]const c.cef_key_event_t,
    _: c.cef_event_handle_t,
    _: [*c]c_int,
) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    if (event == null) return 0;
    if (event.*.type != c.KEYEVENT_RAWKEYDOWN) return 0;
    // An extension popup closes on Escape in Chrome. The popover's own key
    // binding never sees it: the page holds the keyboard.
    const held = c.EVENTFLAG_SHIFT_DOWN | c.EVENTFLAG_CONTROL_DOWN | c.EVENTFLAG_ALT_DOWN | c.EVENTFLAG_COMMAND_DOWN;
    if (event.*.windows_key_code == 0x1B and event.*.modifiers & held == 0) {
        const view = KeyboardObj.of(self).payload;
        if (view.in_popover.load(.acquire)) {
            post(.{ .view = view, .name = "", .popdown = true });
            return 1;
        }
    }
    const spelled = accelsOf(event);
    defer for (spelled) |a| if (a) |x| alloc.free(x);
    const accel = spelled[0] orelse return 0;
    accel_lock.lock();
    var action: ?[]u8 = null;
    for (spelled) |a| {
        const name = a orelse continue;
        if (accel_actions.get(name)) |found| {
            action = alloc.dupe(u8, found) catch null;
            break;
        }
    }
    accel_lock.unlock();
    const owned = action orelse {
        tr("keyToPage {s}", .{accel});
        return 0;
    };
    tr("appAccel {s} -> {s}", .{ accel, owned });
    marker.lat("cef.accel", "{s}", .{accel});
    const task = AccelObj.create(.{ .action = owned }) orelse {
        alloc.free(owned);
        return 0;
    };
    task.cef.execute = &runAccelTask;
    postAccel(task);
    return 1;
}

fn postAccel(task: *AccelObj) void {
    // The activation itself belongs on the GTK thread; `post` is this engine's
    // only hop onto it, and it carries a view, so the task rides the idle
    // queue through glib instead, at input's priority (see `post`).
    _ = glib.idleAddFull(glib.PRIORITY_DEFAULT, &runAccelIdle, task, null);
}

fn runAccelTask(_: [*c]c.cef_task_t) callconv(.c) void {}

fn runAccelIdle(data: ?*anyopaque) callconv(.c) c_int {
    const task: *AccelObj = @ptrCast(@alignCast(data.?));
    defer {
        alloc.free(task.payload.action);
        task.drop();
    }
    const app = gio.Application.getDefault() orelse return 0;
    // Every accelerator this engine matches came from a menu item, whose
    // action is "app.nd-menu-<node>" with no target, so the prefix is all
    // there is to strip.
    const detailed = task.payload.action;
    const name = if (std.mem.startsWith(u8, detailed, "app.")) detailed[4..] else detailed;
    const owned = alloc.dupeZ(u8, name) catch return 0;
    defer alloc.free(owned);
    marker.lat("cef.accelRun", "{s}", .{name});
    gio.ActionGroup.activateAction(app.as(gio.ActionGroup), owned, null);
    return 0;
}

fn onChromeCommand(
    self: [*c]c.cef_command_handler_t,
    browser: [*c]c.cef_browser_t,
    command_id: c_int,
    _: c.cef_window_open_disposition_t,
) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    tr("chromeCommand node={d} id={d}", .{ CommandObj.of(self).payload.node_id, command_id });
    // F12 and ctrl+shift+I arrive as IDC_DEV_TOOLS_TOGGLE, which only ever
    // opens: Chrome toggles a DevToolsWindow of its own, and the inspector on
    // this path is a second browser docked in the view instead. Closing it is
    // this engine's to do, and doing it here is what makes the shortcut the
    // toggle the docs describe.
    const view = CommandObj.of(self).payload;
    if (serveZoomCommand(view, command_id)) return 1;
    if (isDevToolsToggle(command_id) and view.devtools_window.load(.acquire) != 0) {
        closeDockedDevTools(view);
        return 1;
    }
    if (chromeCommandAllowed(command_id)) return 0;
    // Everything else would put Chromium's browser UI on screen: a window, a
    // tab strip, the app menu, a bubble anchored to a toolbar this browser does
    // not have. The ones an app has a meaning for reach it instead, so
    // ctrl+shift+N from a page is the app's private window or nothing.
    const routed = routedChromeCommand(command_id);
    tr("chromeCommand node={d} id={d} refused routed={?s}", .{ view.node_id, command_id, routed });
    if (routed) |name| {
        if (alloc.dupe(u8, name)) |text| {
            post(.{ .view = view, .name = "browserCommand", .text = text });
        } else |_| {}
    }
    return 1;
}

/// The Chrome commands that act on the page in this browser and nothing else,
/// plus the devtools ones, which this engine docks in the view. Every other
/// command id is refused by `onChromeCommand`; the peer of
/// `ndCefAllowedChromeCommands` on AppKit.
const allowed_chrome_commands = [_][*:0]const u8{
    "IDC_BACK",
    "IDC_FORWARD",
    "IDC_RELOAD",
    "IDC_RELOAD_BYPASSING_CACHE",
    "IDC_RELOAD_CLEARING_CACHE",
    "IDC_STOP",
    // Escape in a page: stops a load, or closes a find bar this browser never
    // shows.
    "IDC_CLOSE_FIND_OR_STOP",
    "IDC_ZOOM_PLUS",
    "IDC_ZOOM_NORMAL",
    "IDC_ZOOM_MINUS",
    "IDC_CUT",
    "IDC_COPY",
    "IDC_PASTE",
    "IDC_DEV_TOOLS",
    "IDC_DEV_TOOLS_TOGGLE",
    "IDC_DEV_TOOLS_CONSOLE",
    "IDC_DEV_TOOLS_INSPECT",
    "IDC_DEV_TOOLS_DEVICES",
};

var allowed_chrome_ids: ?[allowed_chrome_commands.len]c_int = null;

fn chromeCommandAllowed(command_id: c_int) bool {
    if (allowed_chrome_ids == null) {
        const api = loader.loaded() orelse return false;
        var ids: [allowed_chrome_commands.len]c_int = undefined;
        for (allowed_chrome_commands, 0..) |name, i| ids[i] = api.id_for_command_id_name(name);
        allowed_chrome_ids = ids;
    }
    for (allowed_chrome_ids.?) |id| {
        if (id >= 0 and id == command_id) return true;
    }
    return false;
}

/// Refused Chrome commands an app has its own meaning for, by the name the app
/// receives as `browserCommand`. The names are the framework's and identical on
/// AppKit (`ndCefRoutedChromeCommands`), so an app handles them once.
const routed_chrome_commands = [_]struct { idc: [*:0]const u8, name: []const u8 }{
    .{ .idc = "IDC_NEW_WINDOW", .name = "newWindow" },
    .{ .idc = "IDC_NEW_INCOGNITO_WINDOW", .name = "newPrivateWindow" },
    .{ .idc = "IDC_NEW_TAB", .name = "newTab" },
    .{ .idc = "IDC_NEW_TAB_TO_RIGHT", .name = "newTab" },
    .{ .idc = "IDC_RESTORE_TAB", .name = "reopenClosedTab" },
    .{ .idc = "IDC_CLOSE_TAB", .name = "closeTab" },
    .{ .idc = "IDC_CLOSE_WINDOW", .name = "closeWindow" },
    .{ .idc = "IDC_SELECT_NEXT_TAB", .name = "nextTab" },
    .{ .idc = "IDC_SELECT_PREVIOUS_TAB", .name = "previousTab" },
    .{ .idc = "IDC_SHOW_HISTORY", .name = "history" },
    .{ .idc = "IDC_SHOW_DOWNLOADS", .name = "downloads" },
    .{ .idc = "IDC_SHOW_BOOKMARK_MANAGER", .name = "bookmarks" },
    .{ .idc = "IDC_BOOKMARK_THIS_TAB", .name = "bookmarkPage" },
    .{ .idc = "IDC_OPTIONS", .name = "settings" },
    .{ .idc = "IDC_MANAGE_EXTENSIONS", .name = "extensions" },
    .{ .idc = "IDC_CLEAR_BROWSING_DATA", .name = "clearBrowsingData" },
    .{ .idc = "IDC_PRINT", .name = "print" },
    .{ .idc = "IDC_BASIC_PRINT", .name = "print" },
    .{ .idc = "IDC_SAVE_PAGE", .name = "savePage" },
    .{ .idc = "IDC_VIEW_SOURCE", .name = "viewSource" },
    .{ .idc = "IDC_OPEN_FILE", .name = "openFile" },
    .{ .idc = "IDC_FIND", .name = "find" },
    .{ .idc = "IDC_FIND_NEXT", .name = "findNext" },
    .{ .idc = "IDC_FIND_PREVIOUS", .name = "findPrevious" },
    .{ .idc = "IDC_FOCUS_LOCATION", .name = "focusAddress" },
    .{ .idc = "IDC_FOCUS_SEARCH", .name = "focusAddress" },
    .{ .idc = "IDC_FULLSCREEN", .name = "fullscreen" },
    .{ .idc = "IDC_HOME", .name = "home" },
    .{ .idc = "IDC_TASK_MANAGER", .name = "taskManager" },
    .{ .idc = "IDC_TASK_MANAGER_SHORTCUT", .name = "taskManager" },
    .{ .idc = "IDC_EXIT", .name = "quit" },
};

var routed_chrome_ids: ?[routed_chrome_commands.len]c_int = null;

fn routedChromeCommand(command_id: c_int) ?[]const u8 {
    if (routed_chrome_ids == null) {
        const api = loader.loaded() orelse return null;
        var ids: [routed_chrome_commands.len]c_int = undefined;
        for (routed_chrome_commands, 0..) |entry, i| ids[i] = api.id_for_command_id_name(entry.idc);
        routed_chrome_ids = ids;
    }
    for (routed_chrome_ids.?, 0..) |id, i| {
        if (id >= 0 and id == command_id) return routed_chrome_commands[i].name;
    }
    return null;
}

fn isDevToolsToggle(command_id: c_int) bool {
    const api = loader.loaded() orelse return false;
    return command_id == api.id_for_command_id_name("IDC_DEV_TOOLS") or
        command_id == api.id_for_command_id_name("IDC_DEV_TOOLS_TOGGLE");
}

/// CEF UI thread only. `close_dev_tools` on the PAGE browser's host is what
/// takes the docked inspector away; it is the same call `closeBrowsersInOrder`
/// makes at quit, and on_before_close then undocks the panel.
fn closeDockedDevTools(view: *View) void {
    const host = hostOf(view) orelse return;
    if (host.close_dev_tools) |close| close(host);
}

/// The app menu, the page action icons and the toolbar buttons all belong to
/// chrome the app never asked for: this engine's browser is a widget inside the
/// host's own window, and the host draws its own chrome.
fn isChromeAppMenuItemVisible(
    _: [*c]c.cef_command_handler_t,
    browser: [*c]c.cef_browser_t,
    _: c_int,
) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    return 0;
}

fn isChromePageActionIconVisible(
    _: [*c]c.cef_command_handler_t,
    _: c.cef_chrome_page_action_icon_type_t,
) callconv(.c) c_int {
    return 0;
}

fn isChromeToolbarButtonVisible(
    _: [*c]c.cef_command_handler_t,
    _: c.cef_chrome_toolbar_button_type_t,
) callconv(.c) c_int {
    return 0;
}

// ============================================================================
// Page zoom reported to the app
// ============================================================================
//
// Every zoom change in Chrome style raises Chrome's zoom bubble, anchored to
// the location bar's zoom icon. This browser has no location bar, so the bubble
// comes up anchored to nothing, over the page. The app draws its own indicator
// from `zoomChanged`, so every zoom change goes through this engine: the chords
// and ctrl+wheel arrive as IDC_ZOOM_* commands and are served here, and the
// level is read back and reported.
//
// On Linux the bubble is not a window of its own: Views draws it inside the
// browser's X window, at the view's top right, where no window watch sees it.
// `chrome_zoom_bubble` off in the browser settings keeps Chrome from showing
// it at all (CEF hands it to `ZoomController::SetShowsNotificationBubble`
// when the browser is created). On macOS NDCefZoom.swift closes it instead.

const ZoomTask = struct {
    view: *View,
    host: *c.cef_browser_host_t,
    command: ?c.cef_zoom_command_t = null,
    level: ?f64 = null,
    source: []const u8,
};
const ZoomObj = ref.Counted(c.cef_task_t, ZoomTask);

const zoom_commands = [_]struct { idc: [*:0]const u8, command: c.cef_zoom_command_t }{
    .{ .idc = "IDC_ZOOM_PLUS", .command = c.CEF_ZOOM_COMMAND_IN },
    .{ .idc = "IDC_ZOOM_MINUS", .command = c.CEF_ZOOM_COMMAND_OUT },
    .{ .idc = "IDC_ZOOM_NORMAL", .command = c.CEF_ZOOM_COMMAND_RESET },
};
var zoom_command_ids: ?[zoom_commands.len]c_int = null;

/// CEF UI thread. `host.zoom` steps through the preset levels Chrome's own
/// command uses, so a chord lands where it would in Chrome.
fn serveZoomCommand(view: *View, command_id: c_int) bool {
    if (zoom_command_ids == null) {
        const api = loader.loaded() orelse return false;
        var ids: [zoom_commands.len]c_int = undefined;
        for (zoom_commands, 0..) |entry, i| ids[i] = api.id_for_command_id_name(entry.idc);
        zoom_command_ids = ids;
    }
    for (zoom_command_ids.?, 0..) |id, i| {
        if (id < 0 or id != command_id) continue;
        const host = hostOf(view) orelse return true;
        changeZoom(.{ .view = view, .host = host, .command = zoom_commands[i].command, .source = "page" });
        return true;
    }
    return false;
}

/// The zoom calls run on the CEF UI thread so the level can be read back in
/// the same step; `setZoom` arrives on the GTK one.
fn postZoomChange(task: ZoomTask) void {
    const api = loader.loaded() orelse return;
    if (api.currently_on(c.TID_UI) != 0) return changeZoom(task);
    const obj = ZoomObj.create(task) orelse return;
    obj.cef.execute = &runZoomTask;
    if (api.post_task(c.TID_UI, obj.handOut()) == 0) obj.drop();
    obj.drop();
}

fn runZoomTask(self: [*c]c.cef_task_t) callconv(.c) void {
    changeZoom(ZoomObj.of(self).payload);
}

/// CEF UI thread.
fn changeZoom(task: ZoomTask) void {
    if (task.command) |step| {
        if (task.host.zoom) |zoom| zoom(task.host, step);
    } else if (task.level) |level| {
        if (task.host.set_zoom_level) |set| set(task.host, level);
    }
    reportZoom(task.view, task.source);
}

/// CEF UI thread. Emits `zoomChanged` when the level differs from the one last
/// reported.
fn reportZoom(view: *View, source: []const u8) void {
    const host = hostOf(view) orelse return;
    const get = host.get_zoom_level orelse return;
    const level = get(host);
    if (@abs(level - view.reported_zoom) < 1e-6) return;
    view.reported_zoom = level;
    const factor = @round(std.math.pow(f64, 1.2, level) * 1000) / 1000;
    tr("zoomChanged node={d} factor={d} source={s}", .{ view.node_id, factor, source });
    post(.{ .view = view, .name = "zoomChanged", .number = factor, .text = alloc.dupe(u8, source) catch null });
}

// ============================================================================
// cef_request_handler_t
// ============================================================================

/// The navigations Chromium would answer by putting the page in a different
/// browser: a middle-click, a ctrl-click, a cross-origin file:// hop. Alloy
/// never reaches this for a windowed embed, Chrome style does, and the answer
/// is the same one on_before_popup gives: the app decides, this engine opens
/// nothing.
fn onOpenUrlFromTab(
    self: [*c]c.cef_request_handler_t,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    target_url: [*c]const c.cef_string_t,
    disposition: c.cef_window_open_disposition_t,
    user_gesture: c_int,
) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(frame);
    const url = dupeStr(target_url);
    if (url) |u| {
        if (externalScheme(u)) {
            handOutside(u, user_gesture != 0);
            return 1;
        }
    }
    postNewWindow(RequestHandlerObj.of(self).payload, url, dispositionName(disposition), user_gesture != 0);
    return 1;
}

/// A navigation to a scheme no browser draws (mailto:, tel:, zoommtg:, …).
/// Chrome style answers it with its own "Open …?" dialog or a blank tab; here
/// it goes to the desktop's handler for the scheme and the page stays put.
fn onBeforeBrowse(
    _: [*c]c.cef_request_handler_t,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    request: [*c]c.cef_request_t,
    user_gesture: c_int,
    _: c_int,
) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(frame);
    defer ref.releaseParam(request);
    if (request == null) return 0;
    const get_url = request.*.get_url orelse return 0;
    const raw = get_url(request);
    if (raw == null) return 0;
    defer freeUserfree(raw);
    const url = dupeStr(raw) orelse return 0;
    if (!externalScheme(url)) {
        alloc.free(url);
        return 0;
    }
    handOutside(url, user_gesture != 0);
    return 1;
}

/// Schemes a browser view renders itself. Anything else is another
/// application's to open.
const internal_schemes = [_][]const u8{
    "http",     "https",     "file",       "data",     "blob",   "about",
    "javascript", "chrome",  "chrome-extension", "chrome-untrusted", "devtools",
    "view-source", "filesystem", "ws",    "wss",
};

fn externalScheme(url: []const u8) bool {
    const colon = std.mem.indexOfScalar(u8, url, ':') orelse return false;
    const scheme = url[0..colon];
    if (scheme.len == 0) return false;
    for (scheme) |ch| {
        if (!std.ascii.isAlphanumeric(ch) and ch != '+' and ch != '-' and ch != '.') return false;
    }
    for (internal_schemes) |known| {
        if (std.ascii.eqlIgnoreCase(scheme, known)) return false;
    }
    for (custom_schemes.items) |spec| {
        if (std.ascii.eqlIgnoreCase(scheme, spec.name)) return false;
    }
    return true;
}

/// Adopts `url`. Only a navigation the user asked for launches anything: a page
/// cannot start another application on its own, which is Chrome's rule too.
fn handOutside(url: []u8, user_gesture: bool) void {
    tr("externalScheme url={s} gesture={}", .{ url, user_gesture });
    if (!user_gesture) {
        std.debug.print("ND_WARN WebView engine=chromium: {s} not opened, no user gesture\n", .{url});
        alloc.free(url);
        return;
    }
    const owned = alloc.dupeZ(u8, url) catch {
        alloc.free(url);
        return;
    };
    alloc.free(url);
    _ = glib.idleAddFull(glib.PRIORITY_DEFAULT, &launchOutside, owned.ptr, null);
}

fn launchOutside(data: ?*anyopaque) callconv(.c) c_int {
    const uri: [*:0]u8 = @ptrCast(data.?);
    defer alloc.free(std.mem.span(uri));
    gio.AppInfo.launchDefaultForUriAsync(uri, null, null, null, null);
    return 0;
}

/// Chrome style answers an HTTP auth challenge with Chromium's own login
/// window, which this embedding may not have. The framework gives an app no way
/// to supply credentials, so the challenge is refused: 0 cancels the request and
/// the page gets the 401 it would have got if the user pressed Cancel.
fn onGetAuthCredentials(
    _: [*c]c.cef_request_handler_t,
    browser: [*c]c.cef_browser_t,
    origin_url: [*c]const c.cef_string_t,
    _: c_int,
    _: [*c]const c.cef_string_t,
    _: c_int,
    _: [*c]const c.cef_string_t,
    _: [*c]const c.cef_string_t,
    callback: [*c]c.cef_auth_callback_t,
) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(callback);
    if (dupeStr(origin_url)) |origin| {
        defer alloc.free(origin);
        std.debug.print("ND_WARN WebView engine=chromium: HTTP authentication refused for {s}; no credential surface in this engine\n", .{origin});
    }
    return 0;
}

// ============================================================================
// Content blocking (src/adblock.zig holds the engine; this is the GTK host's
// half, the peer of swift/Sources/NDShell/NDCefAdblock.swift)
// ============================================================================
//
// Network rules are answered on CEF's IO thread from the resource request
// handlers below. Cosmetic rules and scriptlets reach every frame through the
// render process handler: each new main-world context fetches its own script
// from the `nd-adblock` scheme, which the same request handlers serve.

const AdblockBlockObj = ref.Counted(c.cef_resource_request_handler_t, void);
const AdblockRedirectObj = ref.Counted(c.cef_resource_request_handler_t, AdblockBody);
const AdblockBodyObj = ref.Counted(c.cef_resource_handler_t, AdblockBody);

/// Shared by every view and by the profile contexts' service-worker requests.
/// Created on the GTK thread before any browser or context exists, read on the
/// IO thread afterwards.
var adblock_block: ?*AdblockBlockObj = null;

fn ensureAdblockBlock() void {
    if (adblock_block != null) return;
    const h = AdblockBlockObj.create({}) orelse return;
    h.cef.on_before_resource_load = &onAdblockBlockLoad;
    adblock_block = h;
}

fn onAdblockBlockLoad(
    _: [*c]c.cef_resource_request_handler_t,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    request: [*c]c.cef_request_t,
    callback: [*c]c.cef_callback_t,
) callconv(.c) c.cef_return_value_t {
    ref.releaseParam(browser);
    ref.releaseParam(frame);
    ref.releaseParam(request);
    ref.releaseParam(callback);
    return c.RV_CANCEL;
}

fn requestUrl(request: [*c]c.cef_request_t) ?[]u8 {
    const get_url = request.*.get_url orelse return null;
    const raw = get_url(request);
    if (raw == null) return null;
    defer freeUserfree(raw);
    return dupeStr(raw);
}

/// The view's `get_resource_request_handler`. IO thread.
fn onGetResourceRequestHandler(
    self: [*c]c.cef_request_handler_t,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    request: [*c]c.cef_request_t,
    _: c_int,
    _: c_int,
    request_initiator: [*c]const c.cef_string_t,
    disable_default_handling: [*c]c_int,
) callconv(.c) [*c]c.cef_resource_request_handler_t {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(frame);
    defer ref.releaseParam(request);
    if (request == null) return null;
    const view = RequestHandlerObj.of(self).payload;
    const url = requestUrl(request) orelse return null;
    defer alloc.free(url);
    if (adblock.isSchemeUrl(url)) {
        if (disable_default_handling != null) disable_default_handling.* = 1;
        const top = if (browser != null) mainFrameUrl(browser) else null;
        defer if (top) |t| alloc.free(t);
        return adblockServe(url, top orelse "");
    }
    const initiator = dupeStr(request_initiator);
    defer if (initiator) |i| alloc.free(i);
    if (needsOriginFix(request, initiator)) return (origin_fix orelse return null).handOut();
    if (!adblock.nd_adblock_active()) return null;
    const get_type = request.*.get_resource_type orelse return null;
    const kind: c_int = @intCast(get_type(request));
    if (kind == c.RT_MAIN_FRAME) {
        // The docked inspector rides this client, and its navigation is not
        // the page's.
        if (std.mem.startsWith(u8, url, "devtools://")) return null;
        view.adblock_count.store(0, .release);
        adblockReport(view);
        return null;
    }
    if (extensionInitiated(initiator)) return null;
    const top = if (browser != null) mainFrameUrl(browser) else null;
    defer if (top) |t| alloc.free(t);
    return adblockNetwork(view, url, kind, initiator orelse "", top orelse "");
}

/// The contexts' `get_resource_request_handler`, which is where a request with
/// no browser (a service worker's) is seen. A request that has one was already
/// answered by its view's handler, or comes from a browser Chrome made for
/// itself, and only gets the Origin fix. IO thread.
fn onContextGetResourceRequestHandler(
    _: [*c]c.cef_request_context_handler_t,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    request: [*c]c.cef_request_t,
    _: c_int,
    _: c_int,
    request_initiator: [*c]const c.cef_string_t,
    _: [*c]c_int,
) callconv(.c) [*c]c.cef_resource_request_handler_t {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(frame);
    defer ref.releaseParam(request);
    if (request == null) return null;
    const initiator = dupeStr(request_initiator);
    defer if (initiator) |i| alloc.free(i);
    if (needsOriginFix(request, initiator)) return (origin_fix orelse return null).handOut();
    if (browser != null or !adblock.nd_adblock_active() or extensionInitiated(initiator)) return null;
    const get_type = request.*.get_resource_type orelse return null;
    const url = requestUrl(request) orelse return null;
    defer alloc.free(url);
    return adblockNetwork(null, url, @intCast(get_type(request)), initiator orelse "", initiator orelse "");
}

/// Chromium gives no extension a view of another's requests, and the lists
/// here are written against pages, so an extension's own traffic is left alone.
fn extensionInitiated(initiator: ?[]const u8) bool {
    const from = initiator orelse return false;
    return std.mem.startsWith(u8, from, "chrome-extension://");
}

// CEF's URL loader proxy adds the Origin of a cross-origin CORS request the way
// a navigation's is built (proxy_url_loader_factory.cc, InterceptedRequest::
// Restart), and that sanitizer keeps only http(s) initiators, so an extension's
// fetch leaves with `Origin: null`. Chromium's network service sets the
// extension's own origin wherever Chrome sends one (any non-GET, and a GET the
// extension has no host permission for) and sends none on a permitted GET, so
// the header CEF added is dropped and the network service decides. 1Password's
// servers answer `Origin: null` with 500 and 503, and its extension then cannot
// add the account the desktop app hands it.
const OriginFixObj = ref.Counted(c.cef_resource_request_handler_t, void);
var origin_fix: ?*OriginFixObj = null;

fn ensureOriginFix() void {
    if (origin_fix != null) return;
    const h = OriginFixObj.create({}) orelse return;
    h.cef.on_before_resource_load = &onOriginFixLoad;
    origin_fix = h;
}

fn needsOriginFix(request: [*c]c.cef_request_t, initiator: ?[]const u8) bool {
    if (!extensionInitiated(initiator)) return false;
    const get = request.*.get_header_by_name orelse return false;
    var name = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&name);
    if (!setStr(&name, "Origin")) return false;
    const raw = get(request, &name);
    if (raw == null) return false;
    defer freeUserfree(raw);
    const value = dupeStr(raw) orelse return false;
    defer alloc.free(value);
    return std.mem.eql(u8, value, "null");
}

fn onOriginFixLoad(
    _: [*c]c.cef_resource_request_handler_t,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    request: [*c]c.cef_request_t,
    callback: [*c]c.cef_callback_t,
) callconv(.c) c.cef_return_value_t {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(frame);
    defer ref.releaseParam(request);
    defer ref.releaseParam(callback);
    if (request != null) dropHeader(request, "Origin");
    return c.RV_CONTINUE;
}

/// cef_request_t has no remove; set_header_by_name with an empty value would
/// still send the header.
fn dropHeader(request: [*c]c.cef_request_t, name: []const u8) void {
    const api = loader.loaded() orelse return;
    const get_map = request.*.get_header_map orelse return;
    const set_map = request.*.set_header_map orelse return;
    const all = api.string_multimap_alloc();
    if (all == null) return;
    defer api.string_multimap_free(all);
    const kept = api.string_multimap_alloc();
    if (kept == null) return;
    defer api.string_multimap_free(kept);
    get_map(request, all);
    for (0..api.string_multimap_size(all)) |i| {
        var key = std.mem.zeroes(c.cef_string_t);
        defer clearStr(&key);
        var value = std.mem.zeroes(c.cef_string_t);
        defer clearStr(&value);
        if (api.string_multimap_key(all, i, &key) == 0) continue;
        if (api.string_multimap_value(all, i, &value) == 0) continue;
        const k = dupeStr(&key) orelse continue;
        defer alloc.free(k);
        if (std.ascii.eqlIgnoreCase(k, name)) continue;
        _ = api.string_multimap_append(kept, &key, &value);
    }
    set_map(request, kept);
}

fn adblockNetwork(view: ?*View, url: []const u8, kind: c_int, initiator: []const u8, top: []const u8) [*c]c.cef_resource_request_handler_t {
    var decision = adblock.decide(url, initiator, top, @enumFromInt(kind));
    defer adblock.freeDecision(&decision);
    switch (decision.action) {
        .allow => return null,
        .block => {
            tr("adblock block kind={d} {s}", .{ kind, url });
            if (view) |v| adblockBump(v);
            return (adblock_block orelse return null).handOut();
        },
        .redirect => {
            tr("adblock redirect kind={d} {s}", .{ kind, url });
            if (view) |v| adblockBump(v);
            const body = decision.body orelse return null;
            const mime = decision.mime orelse return null;
            return adblockRespond(body[0..decision.body_len], std.mem.span(mime));
        },
    }
}

/// A request on the `nd-adblock` scheme: a frame asking for its document-start
/// script, or the cosmetic agent asking about what it found.
fn adblockServe(url: []const u8, top: []const u8) [*c]c.cef_resource_request_handler_t {
    var served = adblock.serve(url, top);
    defer adblock.freeServed(&served);
    const body: []const u8 = if (served.body) |b| b[0..served.body_len] else "";
    return adblockRespond(body, std.mem.span(served.mime));
}

fn adblockRespond(body: []const u8, mime: []const u8) [*c]c.cef_resource_request_handler_t {
    const payload = AdblockBody.init(body, mime) orelse return null;
    const h = AdblockRedirectObj.create(payload) orelse {
        var p = payload;
        p.deinit();
        return null;
    };
    h.cef.get_resource_handler = &onAdblockRedirectHandler;
    // The one reference `create` handed back is the caller's.
    return h.cptr();
}

/// A body served in place of a network response: a `redirect=` rule's
/// resource, or an `nd-adblock` scheme answer.
const AdblockBody = struct {
    body: []u8,
    mime: []u8,
    /// IO thread only: CEF reads one resource from one thread.
    offset: usize = 0,

    fn init(body: []const u8, mime: []const u8) ?AdblockBody {
        const b = alloc.dupe(u8, body) catch return null;
        const m = alloc.dupe(u8, mime) catch {
            alloc.free(b);
            return null;
        };
        return .{ .body = b, .mime = m };
    }

    pub fn deinit(self: *AdblockBody) void {
        alloc.free(self.body);
        alloc.free(self.mime);
    }
};

fn onAdblockRedirectHandler(
    self: [*c]c.cef_resource_request_handler_t,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    request: [*c]c.cef_request_t,
) callconv(.c) [*c]c.cef_resource_handler_t {
    ref.releaseParam(browser);
    ref.releaseParam(frame);
    ref.releaseParam(request);
    const src = AdblockRedirectObj.of(self).payload;
    const payload = AdblockBody.init(src.body, src.mime) orelse return null;
    const h = AdblockBodyObj.create(payload) orelse {
        var p = payload;
        p.deinit();
        return null;
    };
    h.cef.open = &onAdblockBodyOpen;
    h.cef.get_response_headers = &onAdblockBodyHeaders;
    h.cef.read = &onAdblockBodyRead;
    h.cef.cancel = &onAdblockBodyCancel;
    return h.cptr();
}

fn onAdblockBodyOpen(
    _: [*c]c.cef_resource_handler_t,
    request: [*c]c.cef_request_t,
    handle_request: [*c]c_int,
    callback: [*c]c.cef_callback_t,
) callconv(.c) c_int {
    ref.releaseParam(request);
    ref.releaseParam(callback);
    if (handle_request != null) handle_request.* = 1;
    return 1;
}

fn onAdblockBodyHeaders(
    self: [*c]c.cef_resource_handler_t,
    response: [*c]c.cef_response_t,
    response_length: [*c]i64,
    _: [*c]c.cef_string_t,
) callconv(.c) void {
    const res = &AdblockBodyObj.of(self).payload;
    if (response != null) {
        if (response.*.set_status) |set| set(response, 200);
        if (response.*.set_mime_type) |set| {
            var s = std.mem.zeroes(c.cef_string_t);
            defer clearStr(&s);
            if (setStr(&s, res.mime)) set(response, &s);
        }
        // A redirected script or XHR is usually cross-origin to the page, and
        // every `nd-adblock` request is.
        if (response.*.set_header_by_name) |set| {
            var name = std.mem.zeroes(c.cef_string_t);
            var value = std.mem.zeroes(c.cef_string_t);
            defer clearStr(&name);
            defer clearStr(&value);
            if (setStr(&name, "Access-Control-Allow-Origin") and setStr(&value, "*")) set(response, &name, &value, 1);
        }
    }
    if (response_length != null) response_length.* = @intCast(res.body.len);
}

fn onAdblockBodyRead(
    self: [*c]c.cef_resource_handler_t,
    data_out: ?*anyopaque,
    bytes_to_read: c_int,
    bytes_read: [*c]c_int,
    callback: [*c]c.cef_resource_read_callback_t,
) callconv(.c) c_int {
    ref.releaseParam(callback);
    const res = &AdblockBodyObj.of(self).payload;
    if (bytes_read != null) bytes_read.* = 0;
    if (res.offset >= res.body.len) return 0;
    const out = data_out orelse return 0;
    const n = @min(@as(usize, @intCast(@max(bytes_to_read, 0))), res.body.len - res.offset);
    if (n == 0) return 0;
    @memcpy(@as([*]u8, @ptrCast(out))[0..n], res.body[res.offset..][0..n]);
    res.offset += n;
    if (bytes_read != null) bytes_read.* = @intCast(n);
    return 1;
}

fn onAdblockBodyCancel(_: [*c]c.cef_resource_handler_t) callconv(.c) void {}

/// Blocked requests since the last main-frame navigation, counted on the IO
/// thread and reported as `contentBlocked` with at most one report in flight.
fn adblockBump(view: *View) void {
    _ = view.adblock_count.fetchAdd(1, .monotonic);
    adblockReport(view);
}

fn adblockReport(view: *View) void {
    if (view.adblock_report_queued.swap(true, .acq_rel)) return;
    post(.{ .view = view, .name = "", .adblock_report = true });
}

fn emitContentBlocked(view: *View) void {
    view.adblock_report_queued.store(false, .release);
    const count = view.adblock_count.load(.acquire);
    const f = emit orelse return;
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    payload.put(alloc, "count", .{ .integer = count }) catch return;
    f(view.node_id, "contentBlocked", .{ .data = .{ .object = payload } });
}

// ---- Renderer side ----------------------------------------------------------
//
// Runs in every renderer subprocess (this binary re-exec'd by CEF), never in
// the browser.

const RenderProcessObj = ref.Counted(c.cef_render_process_handler_t, void);
var render_process_obj: ?*RenderProcessObj = null;

fn appGetRenderProcessHandler(_: [*c]c.cef_app_t) callconv(.c) [*c]c.cef_render_process_handler_t {
    if (render_process_obj == null) {
        const h = RenderProcessObj.create({}) orelse return null;
        h.cef.on_context_created = &onRendererContextCreated;
        render_process_obj = h;
    }
    return render_process_obj.?.handOut();
}

/// A frame's main world exists and none of its scripts has run: fetch and run
/// its content-blocking script (src/adblock.zig `renderer_bootstrap`).
fn onRendererContextCreated(
    _: [*c]c.cef_render_process_handler_t,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    context: [*c]c.cef_v8_context_t,
) callconv(.c) void {
    ref.releaseParam(browser);
    defer ref.releaseParam(frame);
    defer ref.releaseParam(context);
    if (context == null) return;
    // CEF reports every context a frame gets, an extension's content-script
    // world included. The filters are the page's, and a scriptlet run in
    // 1Password's world rewrites that world's globals.
    if (!isMainWorld(frame, context)) return;
    const script = v8Eval(context, adblock.renderer_bootstrap) orelse return;
    defer alloc.free(script);
    if (script.len == 0) return;
    if (v8Eval(context, script)) |rest| alloc.free(rest);
}

fn isMainWorld(frame: [*c]c.cef_frame_t, context: [*c]c.cef_v8_context_t) bool {
    if (frame == null) return false;
    const get_context = frame.*.get_v8_context orelse return false;
    const same = context.*.is_same orelse return false;
    const main = get_context(frame);
    if (main == null) return false;
    // is_same takes the reference get_v8_context handed out.
    return same(context, main) != 0;
}

/// Runs `code` in `context` and returns its value when that is a string.
/// CEF's eval compiles directly, so a page's CSP has no say in it.
fn v8Eval(context: [*c]c.cef_v8_context_t, code: []const u8) ?[]u8 {
    const eval = context.*.eval orelse return null;
    var source = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&source);
    if (!setStr(&source, code)) return null;
    var name = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&name);
    _ = setStr(&name, "nd-adblock://frame/");
    var retval: [*c]c.cef_v8_value_t = null;
    var exception: [*c]c.cef_v8_exception_t = null;
    const ok = eval(context, &source, &name, 1, &retval, &exception);
    defer ref.releaseParam(retval);
    defer ref.releaseParam(exception);
    if (ok == 0 or retval == null) return null;
    const is_string = retval.*.is_string orelse return null;
    if (is_string(retval) == 0) return null;
    const get = retval.*.get_string_value orelse return null;
    const raw = get(retval);
    if (raw == null) return alloc.dupe(u8, "") catch null;
    defer freeUserfree(raw);
    return dupeStr(raw);
}

// ============================================================================
// cef_display_handler_t
// ============================================================================

/// True for the docked inspector, which shares this view's handlers: its
/// address and title are its own, and reporting them as the view's is what put
/// a devtools:// URL in the app's address bar and "DevTools" in the window
/// title.
fn isDevToolsBrowser(view: *View, browser: [*c]c.cef_browser_t) bool {
    const devtools = view.devtools_window.load(.acquire);
    if (devtools == 0 or browser == null) return false;
    const get_host = browser.*.get_host orelse return false;
    const host = get_host(browser);
    if (host == null) return false;
    defer ref.releaseParam(host);
    const get_window = host.*.get_window_handle orelse return false;
    return @as(usize, @intCast(get_window(host))) == devtools;
}

fn onAddressChange(
    self: [*c]c.cef_display_handler_t,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    url: [*c]const c.cef_string_t,
) callconv(.c) void {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(frame);
    if (isDevToolsBrowser(DisplayObj.of(self).payload, browser)) return;
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
    if (isDevToolsBrowser(DisplayObj.of(self).payload, browser)) return;
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

/// The inspector's own close button and dock-side menu are drawn only when the
/// frontend was told it can dock, and that is `can_dock` on the frontend URL.
/// CEF builds that URL inside `show_dev_tools` and takes no argument for it, so
/// the flag is added by re-pointing the frontend at its own address once the
/// document is up. Clicking the button then reaches CEF as `closeWindow`, which
/// closes the devtools browser: the end state `closeDockedDevTools` reaches,
/// through `onBeforeClose`.
fn dockFrontend(view: *View, frame: [*c]c.cef_frame_t) void {
    if (!chromeStyle() or frame == null) return;
    if (frame.*.is_main) |is_main| {
        if (is_main(frame) == 0) return;
    }
    const get_url = frame.*.get_url orelse return;
    const raw = get_url(frame);
    if (raw == null) return;
    defer freeUserfree(raw);
    const url = dupeStr(raw) orelse return;
    defer alloc.free(url);
    if (!std.mem.startsWith(u8, url, "devtools://")) return;
    if (std.mem.indexOf(u8, url, "can_dock=") != null) {
        // The document that carries the placeholder is this one: the frontend
        // told it cannot dock lays out no hole and announces nothing, so the
        // hook goes in here rather than on the frontend's first load.
        installDockHook(view);
        return;
    }
    const sep: []const u8 = if (std.mem.indexOfScalar(u8, url, '?') == null) "?" else "&";
    const docked = std.fmt.allocPrint(alloc, "{s}{s}can_dock=true", .{ url, sep }) catch return;
    defer alloc.free(docked);
    const load = frame.*.load_url orelse return;
    var s = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&s);
    if (!setStr(&s, docked)) return;
    load(frame, &s);
}

fn onLoadStart(
    self: [*c]c.cef_load_handler_t,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    _: c.cef_transition_type_t,
) callconv(.c) void {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(frame);
    dockFrontend(LoadObj.of(self).payload, frame);
}

fn onLoadingStateChange(
    self: [*c]c.cef_load_handler_t,
    browser: [*c]c.cef_browser_t,
    is_loading: c_int,
    can_go_back: c_int,
    can_go_forward: c_int,
) callconv(.c) void {
    defer ref.releaseParam(browser);
    const view = LoadObj.of(self).payload;
    // Chromium restores a host's saved zoom level when a page of it commits.
    if (is_loading == 0) reportZoom(view, "navigation");
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

/// Chrome's popup policy. A window the page opened without a user gesture is
/// blocked unless the site is allowed pop-ups, and the app hears it as
/// `popupBlocked`. With `adoptPopups` the popup's browser is created here and
/// waits for the app to mount it (`openForAdoption`), which keeps
/// `window.opener` and the opener's handle on the window. Otherwise it is
/// cancelled and the app opens a tab off the emitted URL, exactly as it does on
/// WebKitGTK's `create` signal.
fn onBeforePopup(
    self: [*c]c.cef_life_span_handler_t,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    _: c_int,
    target_url: [*c]const c.cef_string_t,
    target_name: [*c]const c.cef_string_t,
    disposition: c.cef_window_open_disposition_t,
    user_gesture: c_int,
    features: [*c]const c.cef_popup_features_t,
    window_info: [*c]c.cef_window_info_t,
    client: [*c][*c]c.cef_client_t,
    _: [*c]c.cef_browser_settings_t,
    _: [*c][*c]c.cef_dictionary_value_t,
    _: [*c]c_int,
) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(frame);
    // Document picture-in-picture (documentPictureInPicture.requestWindow) is
    // Chrome's own browser of TYPE_PICTURE_IN_PICTURE: CEF builds it whatever
    // this returns, and refusing it only rejects the page's promise. It goes
    // through without a client of ours, so it never reaches on_after_created as
    // one of this view's browsers. The client arrived with a reference CEF only
    // drops when it gets the same pointer back. Alloy has no such browser, and
    // there the popup stays refused.
    if (disposition == c.CEF_WOD_NEW_PICTURE_IN_PICTURE and chromeStyle()) {
        if (client != null) {
            ref.releaseParam(client.*);
            client.* = null;
        }
        pending_doc_pip_until.store(glib.getMonotonicTime() + pending_doc_pip_us, .release);
        pending_doc_pip_view.store(@intFromPtr(LifeObj.of(self).payload), .release);
        return 0;
    }
    // A popup let through without a parent window of ours becomes Chrome's own
    // tabbed top-level, at no size the page asked for (measured on 151.3.23,
    // and `use_views_default_popup` gives a bare 800x600 window instead), so
    // one that is allowed always gets a parked container (`openForAdoption`).
    // Without `adoptPopups` it is denied, and window.open answers null:
    // `window.open("about:blank")` followed by `w.location = …` then reaches
    // the app as about:blank and the opener's next statement throws.
    const view = LifeObj.of(self).payload;
    const url = dupeStr(target_url);
    if (url) |u| {
        if (externalScheme(u)) {
            handOutside(u, user_gesture != 0);
            return 1;
        }
    }
    if (user_gesture == 0 and !popupsAllowed(browser, frame)) {
        tr("popupBlocked node={d} url={?s}", .{ view.node_id, url });
        post(.{
            .view = view,
            .name = "popupBlocked",
            .text = url,
            .extra = alloc.dupe(u8, dispositionName(disposition)) catch null,
            .target_name = dupeStr(target_name),
            .features = popupFeatures(features),
        });
        return 1;
    }
    if (view.adopt_popups.load(.acquire) and chromeStyle()) {
        if (openForAdoption(view, url, disposition, user_gesture != 0, features, window_info, client)) return 0;
    }
    postNewWindow(view, url, dispositionName(disposition), user_gesture != 0);
    return 1;
}

/// Chrome's own popup blocker is off (see `disable-popup-blocking`), so its
/// decision is made here against the same content setting, which is what
/// chrome://settings/content/popups and `allowPopups` write. Pages of an
/// extension and of the browser itself are never blocked, as in Chrome, where
/// the blocker only watches web pages in tabs.
fn popupsAllowed(browser: [*c]c.cef_browser_t, frame: [*c]c.cef_frame_t) bool {
    if (frame == null) return false;
    const get_url = frame.*.get_url orelse return false;
    const raw = get_url(frame);
    if (raw == null) return false;
    defer freeUserfree(raw);
    const opener = dupeStr(raw) orelse return false;
    defer alloc.free(opener);
    for ([_][]const u8{ "chrome-extension:", "chrome:", "devtools:" }) |scheme| {
        if (std.mem.startsWith(u8, opener, scheme)) return true;
    }
    const get_host = browser.*.get_host orelse return false;
    const host = get_host(browser);
    if (host == null) return false;
    defer ref.releaseParam(host);
    const get_ctx = host.*.get_request_context orelse return false;
    const ctx = get_ctx(host);
    if (ctx == null) return false;
    defer ref.releaseParam(ctx);
    const get_setting = ctx.*.get_content_setting orelse return false;
    return get_setting(ctx, raw, raw, c.CEF_CONTENT_SETTING_TYPE_POPUPS) == c.CEF_CONTENT_SETTING_VALUE_ALLOW;
}

// ============================================================================
// Popups the app adopts
// ============================================================================
//
// A page's window.open answers with the new window's handle synchronously, so
// a popup that is to keep `window.opener` must have its browser created inside
// on_before_popup, long before the app can say where it goes. With
// `adoptPopups` on the opener, the browser is created into a parked container
// of a view that has no widget yet, the app hears `newWindow` with a popup id,
// and the `<webview popup={id}>` it mounts takes that view over instead of
// creating a browser. Events the browser produced meanwhile are held and
// replayed to the adopting node.

const PopupFeatures = struct { x: ?i64 = null, y: ?i64 = null, width: ?i64 = null, height: ?i64 = null };

/// How long a popup's browser waits for the app before it is closed.
const popup_adopt_wait_ms: c_uint = 10_000;

var next_popup_id: std.atomic.Value(u32) = .init(1);

/// GTK thread only. Filled through the event hop (`popup_register`), which
/// runs before any event of the popup's own.
var pending_popups: std.AutoHashMapUnmanaged(u32, *View) = .empty;

fn popupFeatures(features: [*c]const c.cef_popup_features_t) ?PopupFeatures {
    if (features == null) return null;
    const f = features.*;
    var out: PopupFeatures = .{};
    if (f.xSet != 0) out.x = f.x;
    if (f.ySet != 0) out.y = f.y;
    if (f.widthSet != 0) out.width = f.width;
    if (f.heightSet != 0) out.height = f.height;
    if (out.x == null and out.y == null and out.width == null and out.height == null) return null;
    return out;
}

/// CEF UI thread. Builds the popup's view and container and points CEF at
/// them; false leaves the popup to the cancel-and-report path. `url` is
/// adopted either way.
fn openForAdoption(
    opener: *View,
    url: ?[]u8,
    disposition: c.cef_window_open_disposition_t,
    user_gesture: bool,
    features: [*c]const c.cef_popup_features_t,
    window_info: [*c]c.cef_window_info_t,
    client: [*c][*c]c.cef_client_t,
) bool {
    if (window_info == null or client == null) return false;
    // The opener's own GTK toplevel, not the root's child above its browser:
    // under a reparenting window manager that is the manager's frame. Read off
    // the GTK thread; it changes only when the opener moves windows, and a
    // stale one is corrected by the adopting view's first layout.
    const parent = opener.container_parent;
    if (parent == 0) return false;
    const view = newView() orelse return false;
    const w = @max(opener.size_w.load(.acquire), 1);
    const h = @max(opener.size_h.load(.acquire), 1);
    const container = x11.createChild(parent, park_origin, park_origin, w, h);
    if (container == 0) return false;
    x11.show(container);

    view.container = container;
    view.container_parent = parent;
    view.created = true;
    view.created_url = dupeOwned(url orelse "about:blank");
    view.size_w.store(w, .release);
    view.size_h.store(h, .release);
    view.adopt_popups.store(true, .release);
    view.waiting_popup.store(true, .release);
    view.suppress_menu.store(opener.suppress_menu.load(.acquire), .release);
    if (opener.context) |ctx| {
        ref.addRefParam(ctx);
        view.context = ctx;
    }
    view.popup_id = next_popup_id.fetchAdd(1, .monotonic);

    window_info.*.parent_window = container;
    window_info.*.bounds = .{ .x = 0, .y = 0, .width = @intCast(w), .height = @intCast(h) };
    window_info.*.runtime_style = @intCast(c.CEF_RUNTIME_STYLE_CHROME);
    ref.releaseParam(client.*);
    client.* = view.client.handOut();

    tr("popupForAdoption node={d} popup={d} url={?s} container=0x{x}", .{ opener.node_id, view.popup_id, url, container });
    post(.{ .view = view, .name = "", .popup_register = true });
    post(.{
        .view = opener,
        .name = "newWindow",
        .text = url,
        .extra = alloc.dupe(u8, dispositionName(disposition)) catch null,
        .flag = user_gesture,
        .popup = view.popup_id,
        .features = popupFeatures(features),
    });
    return true;
}

fn registerPendingPopup(view: *View) void {
    if (!view.waiting_popup.load(.acquire)) return;
    pending_popups.put(alloc, view.popup_id, view) catch return;
    view.popup_timer = glib.timeoutAdd(popup_adopt_wait_ms, &onPopupTimeout, view);
}

/// An event of a popup's view that is still waiting is kept for the node that
/// adopts it.
fn holdForPopup(box: *Emission) bool {
    const view = box.view;
    if (!view.waiting_popup.load(.acquire)) {
        // The app gave up on it: its container goes once the browser has.
        if (box.browser_closed and view.popup_abandoned.load(.acquire) and view.container != 0) {
            x11.destroy(view.container);
            view.container = 0;
        }
        return false;
    }
    view.popup_backlog.append(alloc, box) catch return false;
    return true;
}

fn takePendingPopup(id_text: []const u8) ?*View {
    const id = std.fmt.parseInt(u32, id_text, 10) catch return null;
    const entry = pending_popups.fetchRemove(id) orelse return null;
    const view = entry.value;
    if (view.popup_timer != 0) {
        _ = glib.Source.remove(view.popup_timer);
        view.popup_timer = 0;
    }
    view.waiting_popup.store(false, .release);
    tr("popupAdopted popup={d}", .{id});
    return view;
}

fn replayPopupBacklog(data: ?*anyopaque) callconv(.c) c_int {
    const view: *View = @ptrCast(@alignCast(data.?));
    // The lines createBrowser and on_after_created trace for every other
    // view, now that this one has a node id to put in them.
    tr("embed node={d} parent=0x{x} container=0x{x} adopted popup={d}", .{ view.node_id, view.container_parent, view.container, view.popup_id });
    const cef_window = view.cef_window.load(.acquire);
    if (cef_window != 0) tr("created node={d} cefWindow=0x{x}", .{ view.node_id, cef_window });
    var backlog = view.popup_backlog;
    view.popup_backlog = .empty;
    defer backlog.deinit(alloc);
    for (backlog.items) |box| _ = deliver(box);
    return 0;
}

fn onPopupTimeout(data: ?*anyopaque) callconv(.c) c_int {
    const view: *View = @ptrCast(@alignCast(data.?));
    view.popup_timer = 0;
    _ = pending_popups.remove(view.popup_id);
    view.popup_abandoned.store(true, .release);
    view.waiting_popup.store(false, .release);
    tr("popupAbandoned popup={d}", .{view.popup_id});
    // Answered as they would be for a view that is gone.
    var backlog = view.popup_backlog;
    view.popup_backlog = .empty;
    defer backlog.deinit(alloc);
    for (backlog.items) |box| _ = deliver(box);
    if (hostOf(view)) |host| {
        if (host.close_browser) |close| close(host, 1);
    }
    return 0;
}

pub fn setAdoptPopups(widget: *gtk.Widget, on: bool) void {
    const view = viewOf(widget) orelse return;
    view.adopt_popups.store(on, .release);
}

/// Under Alloy, devtools opens as CEF's own top-level window and only when the
/// app asked for it through the openDevTools command; anything else (a page
/// trying to conjure one) is refused. Parenting the window into GTK instead is
/// not an option there: CEF #3165 makes that a crash.
///
/// Under Chrome style there is no separate window at all. Chrome's devtools is
/// still a browser of its own (CEF has no docked inspector for a browser it did
/// not put in a Views window), so it is docked by hand: a second X child inside
/// this view's container, on the right, with the page browser narrowed to make
/// room. That is the shape a user reads as docked, and it keeps the
/// no-top-level invariant that F12 would otherwise break.
fn onBeforeDevToolsPopup(
    self: [*c]c.cef_life_span_handler_t,
    browser: [*c]c.cef_browser_t,
    window_info: [*c]c.cef_window_info_t,
    client: [*c][*c]c.cef_client_t,
    _: [*c]c.cef_browser_settings_t,
    _: [*c][*c]c.cef_dictionary_value_t,
    use_default_window: [*c]c_int,
) callconv(.c) void {
    defer ref.releaseParam(browser);
    const view = LifeObj.of(self).payload;
    const wanted = view.devtools_requested.swap(false, .acq_rel);
    if (!chromeStyle()) {
        if (use_default_window != null) use_default_window.* = @intFromBool(wanted);
        return;
    }
    if (use_default_window != null) use_default_window.* = 0;
    if (window_info == null or view.container == 0) return;
    const w = view.size_w.load(.acquire);
    const h = view.size_h.load(.acquire);
    if (w == 0 or h == 0) return;
    // The dock container outlives every devtools browser that sits in it.
    // Destroying it when devtools closes takes CEF's own window with it while
    // CEF is still tearing that window down, and the host dies on the way out.
    var dock = view.dock_container;
    if (dock == 0) {
        dock = x11.createChild(view.container, 0, 0, w, h);
        if (dock == 0) return;
        view.dock_container = dock;
    }
    x11.show(dock);
    view.devtools_container.store(dock, .release);
    layoutContents(view, w, h);

    window_info.*.size = @sizeOf(c.cef_window_info_t);
    window_info.*.parent_window = dock;
    window_info.*.bounds = .{ .x = 0, .y = 0, .width = @intCast(w), .height = @intCast(h) };
    // Alloy, not Chrome: a Chrome-style devtools browser comes with a Chrome
    // Browser and its tab strip, and the pair are torn down after the browser
    // they belonged to, which crashes on the way out of the process.
    window_info.*.runtime_style = @intCast(c.CEF_RUNTIME_STYLE_ALLOY);
    // The client is left at CEF's default, which is the source browser's own:
    // handing the devtools popup a second client of ours leaves CEF's browser
    // bookkeeping inconsistent and the process dies on the way out. The view's
    // handlers tell the two browsers apart by which one arrived first.
    _ = client;
}

/// The reference this parameter arrives with is deliberately kept: it is the
/// view's handle on its browser until on_before_close gives it back.
fn onAfterCreated(self: [*c]c.cef_life_span_handler_t, browser: [*c]c.cef_browser_t) callconv(.c) void {
    if (browser == null) return;
    const view = LifeObj.of(self).payload;
    // The docked devtools browser shares this client, and it is always the
    // second to arrive: the page browser keeps the view.
    if (view.browser.load(.acquire) != 0) {
        if (browser.*.get_host) |get_host| {
            const host = get_host(browser);
            if (host != null) {
                if (host.*.get_window_handle) |get_window| {
                    view.devtools_window.store(@intCast(get_window(host)), .release);
                }
                // The reference is kept, like the page's: the session that
                // reads the frontend's page rectangle sends through it, and
                // onBeforeClose gives it back.
                view.devtools_host.store(@intFromPtr(host), .release);
                view.dock_session = cdp.attach(host, dockTag(view));
                startDockSession(view);
            }
        }
        layoutContents(view, view.size_w.load(.acquire), view.size_h.load(.acquire));
        return;
    }
    view.browser.store(@intFromPtr(browser), .release);
    if (focused_view == null and !view.waiting_popup.load(.acquire)) focused_view = view;
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
                if (chromeStyle()) x11.nameChromiumWindowsFrom(@intCast(get_handle(host)));
            }
            // Written here rather than on the GTK thread because the observer
            // has to exist before the first protocol message; the settle hop
            // below is the first GTK-side read of it.
            view.session = cdp.attach(host, @intFromPtr(view));
        }
    }
    tr("created node={d} cefWindow=0x{x}", .{ view.node_id, view.cef_window.load(.acquire) });
    post(.{ .view = view, .name = "", .settle = true });
    if (view.popup_abandoned.load(.acquire)) {
        if (hostOf(view)) |host| {
            if (host.close_browser) |close| close(host, 1);
        }
    }
}

/// A view's host reference, released on the GTK thread. That thread reads the
/// pointer and posts calls on it (cdp.send, executeChromeCommand), which take a
/// reference of their own; releasing ours from here, on the CEF UI thread,
/// could free the host between that read and that reference.
fn releaseHostOnGtk(raw: usize) void {
    _ = glib.idleAddFull(glib.PRIORITY_DEFAULT, &releaseHostIdle, @ptrFromInt(raw), null);
}

fn releaseHostIdle(data: ?*anyopaque) callconv(.c) c_int {
    ref.releaseParam(@as([*c]c.cef_browser_host_t, @ptrCast(@alignCast(data.?))));
    return 0;
}

fn onDoClose(_: [*c]c.cef_life_span_handler_t, browser: [*c]c.cef_browser_t) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    return 0;
}

fn onBeforeClose(self: [*c]c.cef_life_span_handler_t, browser: [*c]c.cef_browser_t) callconv(.c) void {
    const view = LifeObj.of(self).payload;
    // The docked devtools browser rides the same client. Closing it undocks
    // the panel and leaves the page browser alone; the container itself is
    // kept, because destroying it takes CEF's window with it mid-teardown.
    const devtools = view.devtools_window.load(.acquire);
    if (devtools != 0 and browser != null) {
        if (browser.*.get_host) |get_host| {
            const host = get_host(browser);
            if (host != null) {
                defer ref.releaseParam(host);
                if (host.*.get_window_handle) |get_window| {
                    if (@as(usize, @intCast(get_window(host))) == devtools) {
                        view.devtools_window.store(0, .release);
                        cdp.detach(&view.dock_session);
                        const devtools_host = view.devtools_host.swap(0, .acq_rel);
                        if (devtools_host != 0) releaseHostOnGtk(devtools_host);
                        view.page_w.store(0, .release);
                        const dock = view.devtools_container.swap(0, .acq_rel);
                        if (dock != 0) x11.hide(@intCast(dock));
                        layoutContents(view, view.size_w.load(.acquire), view.size_h.load(.acquire));
                        tr("devtoolsClosed node={d}", .{view.node_id});
                        ref.releaseParam(browser);
                        return;
                    }
                }
            }
        }
    }
    view.cef_window.store(0, .release);
    // Before the host goes, so a GTK-side send that finds no host knows why.
    view.browser_gone.store(true, .release);
    cdp.detach(&view.session);
    const host = view.host.swap(0, .acq_rel);
    if (host != 0) releaseHostOnGtk(host);
    const held = view.browser.swap(0, .acq_rel);
    if (held != 0) ref.releaseParam(@as([*c]c.cef_browser_t, @ptrFromInt(held)));
    ref.releaseParam(browser);
    post(.{ .view = view, .name = "", .browser_closed = true });
    releaseViews(view);
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
    /// `newWindow` for a tab Chrome made on its own rather than one a page in
    /// this view asked for.
    unrelated: bool = false,
    settle: bool = false,
    /// Devtools traffic rides the same hop: a protocol result arrives on the
    /// CEF UI thread, and everything that interprets it (the pending-call
    /// table, the world map, the script registry) lives on the GTK one.
    cdp_result: bool = false,
    cdp_event: bool = false,
    message_id: c_int = 0,
    ok: bool = false,
    /// Chromium tabbed past its last focusable and is handing the keyboard
    /// back; the GTK-side hop returns X input focus to the toplevel.
    take_focus: bool = false,
    grab_focus: bool = false,
    /// Escape in a page that sits in an autohide popover.
    popdown: bool = false,
    relayout: bool = false,
    browser_closed: bool = false,
    /// The hop that files a popup's view as waiting for the app to adopt it.
    popup_register: bool = false,
    /// `newWindow` for a popup whose browser waits for `<webview popup=…>`.
    popup: u32 = 0,
    /// What window.open asked for in its features, in CSS pixels.
    features: ?PopupFeatures = null,
    /// The window name a blocked window.open asked for, so the app can open it
    /// again as asked.
    target_name: ?[]u8 = null,
    /// A parked scheme request being handed from the IO thread to the GTK one.
    scheme_obj: ?*ResourceObj = null,
    /// Non-zero on the hop that records a new browser's identifier.
    browser_id: c_int = 0,
    /// Context-menu payloads, owned by the emission because the params they
    /// were read from are gone by the time the GTK loop runs.
    menu_hit: ?*MenuHit = null,
    menu_click: ?*MenuClick = null,
    /// A native menu waiting to be drawn. It holds the run_context_menu
    /// callback, so the GTK side owns answering it from here on.
    menu_request: ?*MenuRequest = null,
    /// A permission prompt Chromium would otherwise have drawn itself. It
    /// holds the callback, so the GTK side owns answering it from here on.
    permission: ?*PermissionRequest = null,
    /// Chromium's own id for a prompt it has finished with.
    permission_dismissed: ?u64 = null,
    /// A download waiting for `respondDownload`. It holds the callback, so the
    /// GTK side owns continuing or cancelling it from here on.
    download: ?*DownloadRequest = null,
    download_update: ?DownloadUpdate = null,
    /// The view's blocked count changed (`contentBlocked`).
    adblock_report: bool = false,
};

fn post(e: Emission) void {
    const box = alloc.create(Emission) catch {
        if (e.text) |t| alloc.free(t);
        if (e.extra) |t| alloc.free(t);
        return;
    };
    box.* = e;
    // g_idle_add is the one glib entry point safe to call from a foreign
    // thread; everything downstream of `deliver` runs on the GTK loop. At
    // G_PRIORITY_DEFAULT, the priority GDK gives input, not g_idle_add's own:
    // that one is below GTK's redraw, and on a slow machine with a frame
    // always due, a ctrl+tab typed into a page waited seconds behind them.
    _ = glib.idleAddFull(glib.PRIORITY_DEFAULT, &deliver, box, null);
}

fn freeEmission(box: *Emission) void {
    if (box.text) |t| alloc.free(t);
    if (box.extra) |t| alloc.free(t);
    if (box.target_name) |t| alloc.free(t);
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

fn deliver(data: ?*anyopaque) callconv(.c) c_int {
    const box: *Emission = @ptrCast(@alignCast(data.?));
    var kept_box = false;
    defer if (!kept_box) freeEmission(box);
    if (box.popup_register) {
        registerPendingPopup(box.view);
        return 0;
    }
    // A scheme request is keyed by browser id, not by view pointer: the
    // factory runs on the IO thread with no view in hand.
    if (box.scheme_obj) |obj| {
        announceSchemeRequest(obj);
        return 0;
    }
    // Keyed by Chromium's prompt id rather than by view, so a prompt dismissed
    // after its tab went away is still forgotten.
    if (box.permission_dismissed) |prompt_id| {
        dropPermissionRequest(box.view, prompt_id);
        return 0;
    }
    // Not keyed by the view: the key went to a page in a popover, and every
    // such popover closes, whichever view the event is filed under.
    if (box.popdown) {
        popdownPagePopovers(null);
        return 0;
    }
    // The tab this came from can have been closed while the event was in
    // flight; the widget, and with it the container window, is already gone.
    if (!live_views.contains(@intFromPtr(box.view))) {
        if (holdForPopup(box)) {
            kept_box = true;
            return 0;
        }
        if (box.menu_request) |req| cancelMenuRequest(req);
        if (box.permission) |req| answerPermissionRequest(req, .dismiss);
        if (box.download) |req| answerDownload(req, null);
        if (box.download_update) |u| {
            // A download outlives the view that started it for as long as
            // its browser runs; the report then goes to any view still alive.
            var it = live_views.keyIterator();
            if (it.next()) |other| {
                box.view = @ptrFromInt(other.*);
            } else {
                releaseItemCallback(u.callback);
                return 0;
            }
        } else return 0;
    }
    const view = box.view;

    if (box.browser_closed) {
        failAllCalls(view);
        x11.destroy(view.closing_container);
        view.closing_container = 0;
        // Still in the tree, so the page closed itself (window.close() in a
        // window a script opened) rather than the app removing the view.
        if (emit) |f| f(view.node_id, "windowClosed", .{});
        return 0;
    }

    if (box.menu_request) |req| {
        openNativeMenu(view, req);
        return 0;
    }

    if (box.permission) |req| {
        announcePermissionRequest(view, req);
        return 0;
    }

    if (box.adblock_report) {
        emitContentBlocked(view);
        return 0;
    }

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
        // The popover handed this view the keyboard before its browser
        // existed, and something else in the window may have taken GTK's focus
        // since. An Alloy-style popup does not ask for it on its own.
        if (view.in_popover.load(.acquire)) _ = gtk.Widget.grabFocus(view.widget);
        syncBrowserFocus(view);
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

    if (box.relayout) {
        layoutContents(view, view.size_w.load(.acquire), view.size_h.load(.acquire));
        return 0;
    }

    if (box.grab_focus) {
        // A click into another page closes a page popover, as GDK's press
        // check would have; focus this engine handed back itself does not.
        if (box.flag and autohidePopoverOf(view) == null) popdownPagePopovers(view);
        // Re-asserted rather than left to the property notification: the
        // widget can already BE the focus widget (a tab coming back from
        // hidden brings its own focus with it), and then nothing would tell
        // the browser that the keyboard is its again.
        _ = gtk.Widget.grabFocus(view.widget);
        syncBrowserFocus(view);
        return 0;
    }

    if (box.take_focus) {
        // GTK's focus widget and X input focus both come home: grabFocus
        // alone leaves the keyboard on CEF's X window, and the app
        // (accelerators included) stays deaf until something else moves it.
        view.page_released = true;
        defer view.page_released = false;
        if (gtk.Widget.getRoot(view.widget)) |root| {
            const root_widget: *gtk.Widget = @ptrCast(@alignCast(root));
            // childFocus, not grabFocus: grabbing on the root hands focus
            // straight back to the window's default widget, which is this view
            // again, and tabbing out of the page never leaves it. It can take
            // nothing, though, and a window with no focus widget at all is
            // worse than one focused on the view.
            _ = gtk.Widget.childFocus(root_widget, if (box.flag) .tab_forward else .tab_backward);
            // childFocus can report success and leave the window with no focus
            // widget at all, which is worse than leaving it on the view: a
            // window with none drops every key.
            if (gobject.ext.cast(gtk.Window, root)) |window| {
                if (gtk.Window.getFocus(window) == null) _ = gtk.Widget.grabFocus(view.widget);
            }
        }
        syncBrowserFocus(view);
        return 0;
    }

    const f = emit orelse return 0;
    if (std.mem.eql(u8, box.name, "navigate")) {
        const text = box.text orelse return 0;
        // The page the menu was opened over is gone, and so are the commands
        // Chromium queued against it.
        closeNativeMenu(view);
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
        const how = box.extra orelse {
            f(view.node_id, "newWindow", .{ .text = text });
            return 0;
        };
        var payload: std.json.ObjectMap = .empty;
        defer payload.deinit(alloc);
        payload.put(alloc, "disposition", .{ .string = how }) catch return 0;
        payload.put(alloc, "userGesture", .{ .bool = box.flag }) catch return 0;
        if (box.unrelated) payload.put(alloc, "fromExtension", .{ .bool = true }) catch return 0;
        var id_buf: [16]u8 = undefined;
        if (box.popup != 0) payload.put(alloc, "popup", .{ .string = std.fmt.bufPrint(&id_buf, "{d}", .{box.popup}) catch return 0 }) catch return 0;
        var bounds: std.json.ObjectMap = .empty;
        defer bounds.deinit(alloc);
        if (box.features) |want| {
            if (want.x) |v| bounds.put(alloc, "x", .{ .integer = v }) catch {};
            if (want.y) |v| bounds.put(alloc, "y", .{ .integer = v }) catch {};
            if (want.width) |v| bounds.put(alloc, "width", .{ .integer = v }) catch {};
            if (want.height) |v| bounds.put(alloc, "height", .{ .integer = v }) catch {};
            payload.put(alloc, "features", .{ .object = bounds }) catch {};
        }
        f(view.node_id, "newWindow", .{ .text = text, .data = .{ .object = payload } });
    } else if (std.mem.eql(u8, box.name, "popupBlocked")) {
        var payload: std.json.ObjectMap = .empty;
        defer payload.deinit(alloc);
        payload.put(alloc, "url", .{ .string = box.text orelse "" }) catch return 0;
        payload.put(alloc, "disposition", .{ .string = box.extra orelse "foregroundTab" }) catch return 0;
        if (box.target_name) |name| {
            if (name.len != 0) payload.put(alloc, "target", .{ .string = name }) catch return 0;
        }
        var bounds: std.json.ObjectMap = .empty;
        defer bounds.deinit(alloc);
        if (box.features) |want| {
            if (want.x) |v| bounds.put(alloc, "x", .{ .integer = v }) catch {};
            if (want.y) |v| bounds.put(alloc, "y", .{ .integer = v }) catch {};
            if (want.width) |v| bounds.put(alloc, "width", .{ .integer = v }) catch {};
            if (want.height) |v| bounds.put(alloc, "height", .{ .integer = v }) catch {};
            payload.put(alloc, "features", .{ .object = bounds }) catch {};
        }
        f(view.node_id, "popupBlocked", .{ .data = .{ .object = payload } });
    } else if (std.mem.eql(u8, box.name, "browserCommand")) {
        const text = box.text orelse return 0;
        f(view.node_id, "browserCommand", .{ .text = text });
    } else if (std.mem.eql(u8, box.name, "pictureInPicture")) {
        var payload: std.json.ObjectMap = .empty;
        defer payload.deinit(alloc);
        payload.put(alloc, "state", .{ .string = box.text orelse return 0 }) catch return 0;
        payload.put(alloc, "kind", .{ .string = box.extra orelse return 0 }) catch return 0;
        f(view.node_id, "pictureInPicture", .{ .data = .{ .object = payload } });
    } else if (std.mem.eql(u8, box.name, "zoomChanged")) {
        var payload: std.json.ObjectMap = .empty;
        defer payload.deinit(alloc);
        payload.put(alloc, "factor", .{ .float = box.number }) catch return 0;
        payload.put(alloc, "source", .{ .string = box.text orelse "app" }) catch return 0;
        f(view.node_id, "zoomChanged", .{ .data = .{ .object = payload } });
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
        const req = box.download orelse return 0;
        const id = std.fmt.allocPrint(alloc, "cefdownload-{d}", .{req.item_id}) catch {
            answerDownload(req, null);
            return 0;
        };
        if (pending_downloads.fetchRemove(id)) |stale| {
            alloc.free(stale.key);
            answerDownload(stale.value, null);
        }
        pending_downloads.put(alloc, id, req) catch {
            alloc.free(id);
            answerDownload(req, null);
            return 0;
        };
        tr("downloadRequested node={d} id={s}", .{ view.node_id, id });
        var payload: std.json.ObjectMap = .empty;
        defer payload.deinit(alloc);
        payload.put(alloc, "id", .{ .string = id }) catch return 0;
        payload.put(alloc, "url", .{ .string = if (box.text) |t| t else "" }) catch return 0;
        if (box.extra) |name| payload.put(alloc, "suggestedFilename", .{ .string = name }) catch return 0;
        f(view.node_id, "downloadRequested", .{ .data = .{ .object = payload } });
    } else if (std.mem.eql(u8, box.name, "downloadUpdated")) {
        const u = box.download_update orelse return 0;
        var key_buf: [40]u8 = undefined;
        const id = std.fmt.bufPrint(&key_buf, "cefdownload-{d}", .{u.item_id}) catch return 0;
        const entry = running_downloads.getEntry(id) orelse {
            releaseItemCallback(u.callback);
            return 0;
        };
        if (u.callback != 0) {
            if (download_controls.getPtr(id)) |held| {
                releaseItemCallback(held.*);
                held.* = u.callback;
            } else {
                download_controls.put(alloc, entry.key_ptr.*, u.callback) catch releaseItemCallback(u.callback);
            }
        }
        const signature = (@as(u64, @intFromEnum(u.state)) << 60) | (@as(u64, @intFromBool(u.paused)) << 59) | (@as(u64, @bitCast(u.received)) & ((1 << 59) - 1));
        if (entry.value_ptr.* == signature) return 0;
        entry.value_ptr.* = signature;
        const final = u.state == .done or u.state == .cancelled;
        if (final) {
            if (download_controls.fetchRemove(id)) |held| releaseItemCallback(held.value);
        }
        tr("downloadUpdated node={d} id={s} state={s} received={d}", .{ view.node_id, id, @tagName(u.state), u.received });
        var payload: std.json.ObjectMap = .empty;
        defer payload.deinit(alloc);
        payload.put(alloc, "id", .{ .string = id }) catch return 0;
        payload.put(alloc, "state", .{ .string = @tagName(u.state) }) catch return 0;
        payload.put(alloc, "received", .{ .integer = u.received }) catch return 0;
        payload.put(alloc, "total", .{ .integer = u.total }) catch return 0;
        payload.put(alloc, "path", .{ .string = if (box.text) |t| t else "" }) catch return 0;
        payload.put(alloc, "speed", .{ .integer = u.speed }) catch return 0;
        payload.put(alloc, "paused", .{ .bool = u.paused }) catch return 0;
        f(view.node_id, "downloadUpdated", .{ .data = .{ .object = payload } });
        if (final) {
            if (running_downloads.fetchRemove(id)) |gone| alloc.free(gone.key);
        }
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

/// Which of a view's two protocol sessions a callback belongs to. A View is
/// pointer-aligned, so the low bit is free to carry it: the page's session and
/// the docked inspector's both land in the same sink.
fn dockTag(view: *View) usize {
    return @intFromPtr(view) | 1;
}

/// The devtools sink, both arms on the CEF UI thread. `tag` is the View
/// pointer cdp.attach was handed.
fn cdpResultSink(tag: usize, message_id: c_int, ok: bool, json: []const u8) void {
    const view: *View = @ptrFromInt(tag & ~@as(usize, 1));
    if (tag & 1 != 0) {
        // Nothing carrying parameters may go out before the agent has answered
        // one that does not, so the frontend's hook waits on that answer.
        if (ok and message_id == view.dock_handshake.load(.acquire)) installDockHook(view);
        return;
    }
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
    const view: *View = @ptrFromInt(tag & ~@as(usize, 1));
    if (tag & 1 != 0) {
        dockEvent(view, method, json);
        return;
    }
    // Only the three events this engine acts on are worth a hop; the Page and
    // Runtime domains are chatty enough that forwarding everything would put a
    // GTK idle source behind every DOM mutation.
    if (!std.mem.eql(u8, method, "Runtime.executionContextCreated") and
        !std.mem.eql(u8, method, "Runtime.executionContextsCleared") and
        !std.mem.eql(u8, method, "Runtime.executionContextDestroyed") and
        !std.mem.eql(u8, method, "Page.frameNavigated") and
        !std.mem.eql(u8, method, "Runtime.bindingCalled") and
        !std.mem.eql(u8, method, "Network.responseReceived") and
        !std.mem.eql(u8, method, cdp.agent_attached) and
        !std.mem.eql(u8, method, cdp.agent_detached)) return;
    // Every subresource's response arrives here too, and only the document's
    // is read (onCdpEvent).
    if (std.mem.eql(u8, method, "Network.responseReceived") and
        std.mem.indexOf(u8, json, "\"type\":\"Document\"") == null) return;
    post(.{
        .view = view,
        .name = "",
        .cdp_event = true,
        .extra = alloc.dupe(u8, method) catch null,
        .text = alloc.dupe(u8, json) catch null,
    });
}

// ============================================================================
// The docked inspector's frontend
// ============================================================================
//
// Chrome's inspector does not sit beside the page: the frontend fills the
// browser's whole contents area and keeps a hole in its own layout for the
// page, which the browser draws into that rectangle. The frontend announces
// the hole with `setInspectedPageBounds` every time it moves: the splitter is
// dragged, the dock side changes, device mode turns on (the rectangle is then
// the device's, which is how the phone is drawn inside the page area).
//
// CEF has no seam for that message, and the inspector's browser carries CEF's
// own client, so no handler here ever sees it. The frontend is asked for it
// instead, over its own protocol session: a patch on its channel to the
// embedder forwards every announcement into a Runtime binding, which comes
// back as `Runtime.bindingCalled`.

const dock_binding = "ndDockBounds";

/// Installed at document start and once on the live document. `DevToolsHost` is
/// Chromium's own injection into a devtools:// page and is there before any
/// frontend script runs; the retry covers that ordering not being contractual.
const dock_hook =
    \\(() => {
    \\  const send = (rect, tries) => {
    \\    if (typeof window.ndDockBounds === 'function') {
    \\      window.ndDockBounds(rect.x + ',' + rect.y + ',' + rect.width + ',' + rect.height);
    \\      return;
    \\    }
    \\    if (tries < 100) setTimeout(() => send(rect, tries + 1), 50);
    \\  };
    \\  const install = (tries) => {
    \\    const host = window.DevToolsHost;
    \\    if (!host || !host.sendMessageToEmbedder) {
    \\      if (tries < 100) setTimeout(() => install(tries + 1), 50);
    \\      return;
    \\    }
    \\    if (host.__ndDockHook) return;
    \\    const original = host.sendMessageToEmbedder.bind(host);
    \\    host.sendMessageToEmbedder = (message) => {
    \\      try {
    \\        const parsed = typeof message === 'string' ? JSON.parse(message) : message;
    \\        if (parsed && parsed.method === 'setInspectedPageBounds') send(parsed.params[0], 0);
    \\      } catch (error) {}
    \\      return original(message);
    \\    };
    \\    host.__ndDockHook = true;
    \\    window.dispatchEvent(new Event('resize'));
    \\  };
    \\  install(0);
    \\})()
;

/// The agent attaches in answer to the first message, so the handshake is what
/// starts it rather than something that waits for it. Nothing carrying
/// parameters may go out until this one has been answered.
fn startDockSession(view: *View) void {
    const host = view.devtools_host.load(.acquire);
    if (host == 0) return;
    const id = cdp.send(@ptrFromInt(host), "Runtime.enable", "") orelse return;
    view.dock_handshake.store(id, .release);
}

/// CEF UI thread, both of these: the dock session never touches GTK.
fn dockEvent(view: *View, method: []const u8, json: []const u8) void {
    if (std.mem.eql(u8, method, cdp.agent_attached)) {
        installDockHook(view);
        return;
    }
    if (!std.mem.eql(u8, method, "Runtime.bindingCalled")) return;
    const payload = jsonStringField(json, "payload") orelse return;
    const rect = dockRect(payload) orelse return;
    if (view.page_w.load(.acquire) == rect[2] and view.page_h.load(.acquire) == rect[3] and
        view.page_x.load(.acquire) == rect[0] and view.page_y.load(.acquire) == rect[1]) return;
    view.page_x.store(rect[0], .release);
    view.page_y.store(rect[1], .release);
    view.page_w.store(rect[2], .release);
    view.page_h.store(rect[3], .release);
    tr("dockPageRect node={d} {d}x{d}@{d},{d}", .{ view.node_id, rect[2], rect[3], rect[0], rect[1] });
    layoutContents(view, view.size_w.load(.acquire), view.size_h.load(.acquire));
}

fn installDockHook(view: *View) void {
    const raw = view.devtools_host.load(.acquire);
    if (raw == 0) return;
    const host: *c.cef_browser_host_t = @ptrFromInt(raw);
    var params: std.ArrayList(u8) = .empty;
    defer params.deinit(alloc);
    params.appendSlice(alloc, "{\"name\":\"" ++ dock_binding ++ "\"}") catch return;
    _ = cdp.send(host, "Runtime.addBinding", params.items);
    _ = cdp.send(host, "Page.enable", "");
    var script: std.ArrayList(u8) = .empty;
    defer script.deinit(alloc);
    script.appendSlice(alloc, "{\"source\":") catch return;
    cdp.quote(&script, dock_hook);
    script.appendSlice(alloc, "}") catch return;
    _ = cdp.send(host, "Page.addScriptToEvaluateOnNewDocument", script.items);
    var now: std.ArrayList(u8) = .empty;
    defer now.deinit(alloc);
    now.appendSlice(alloc, "{\"expression\":") catch return;
    cdp.quote(&now, dock_hook);
    now.appendSlice(alloc, "}") catch return;
    _ = cdp.send(host, "Runtime.evaluate", now.items);
    tr("dockHook node={d}", .{view.node_id});
}

/// "x,y,width,height", which is what the hook sends: a binding payload is a
/// string, and four numbers in one avoid unescaping JSON inside JSON.
fn dockRect(payload: []const u8) ?[4]u32 {
    var out: [4]u32 = .{ 0, 0, 0, 0 };
    var it = std.mem.splitScalar(u8, payload, ',');
    for (&out) |*slot| {
        const field = it.next() orelse return null;
        const value = std.fmt.parseInt(i64, std.mem.trim(u8, field, " "), 10) catch return null;
        slot.* = @intCast(@max(value, 0));
    }
    if (out[2] == 0 or out[3] == 0) return null;
    return out;
}

/// The one string field this file reads out of a protocol event, without
/// pulling a JSON parser onto the UI thread for it. Escapes inside the value
/// are left alone: the only reader is `dockRect`, whose payload is digits and
/// commas.
fn jsonStringField(json: []const u8, name: []const u8) ?[]const u8 {
    var needle_buf: [32]u8 = undefined;
    const needle = std.fmt.bufPrint(&needle_buf, "\"{s}\":\"", .{name}) catch return null;
    const at = std.mem.indexOf(u8, json, needle) orelse return null;
    const rest = json[at + needle.len ..];
    const end = std.mem.indexOfScalar(u8, rest, '"') orelse return null;
    return rest[0..end];
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

/// An isolated world's execution context, keyed by world name AND frame.
///
/// One name is many contexts: every frame of the document gets its own, and a
/// page with an iframe creates the child's second. Keying on the name alone
/// meant the last context created won, so a world-scoped evaluation aimed at
/// "this view" landed inside the iframe: the whole of an extension broker's
/// delivery path went to the wrong document, and every message it sent carried
/// the subframe's id.
fn worldKey(world: []const u8, frame: []const u8) ?[]u8 {
    return std.fmt.allocPrint(alloc, "{s}\x00{s}", .{ world, frame }) catch null;
}

const ScriptEntry = struct { identifier: []u8, world: []u8 };
const Channel = struct { world: []u8, script: ?[]u8 = null };

/// A call that cannot be issued until its world has an execution context.
const Deferred = union(enum) {
    eval: struct { sink: EvalSink, code: []u8, world: []u8, retried: bool = false, deadline_us: i64 = 0 },
};

/// How long a call may wait for its world to get an execution context. A world
/// whose document never arrives would otherwise park the call for the process's
/// life, and a caller waiting on an answer that never comes reads as the whole
/// feature being dead: an extension's background page reported exactly that.
const world_wait_us: i64 = 5 * std.time.us_per_s;

/// `event` and `key` are static strings; `id` and `what` are owned. `what` is
/// what a timeout calls the command in its error, and `budget_us` is how long
/// the command may take: a round trip for most of them, a person's answer for
/// the one that raises Chrome's own confirmation.
const JsonResult = struct {
    id: []u8,
    event: []const u8,
    key: []const u8,
    what: []u8,
    budget_us: i64,
};

/// Where the string form of one evaluation goes.
const EvalSink = union(enum) {
    /// `executeJavaScript`: emits `javaScriptResult` with this correlation id.
    app: []u8,
    /// `executeJavaScript` with `userGesture`: the same answer, from code run
    /// as if the user had just clicked the page.
    app_gesture: []u8,
    /// `webviewEval`: settles this entry in `pending_evals`.
    auto: u64,
    /// The `pageTextContains` cache.
    page_text: u32,
    /// `listExtensions`: emits `extensionsList` with this correlation id.
    extensions: []u8,
    /// The three commands that mutate the registry and answer with the list it
    /// became.
    json_result: JsonResult,
    /// `listExtensionActions`: emits `extensionActions` with this correlation
    /// id, after the manifests are read.
    extension_actions: []u8,
    /// Fire and forget: the result is not wanted, only the side effect.
    discard,
};

/// An evaluation in flight, with everything a retry needs. A world-scoped call
/// names an execution context that can die under it: the document commits
/// between the send and the answer, and the id CEF was given is gone by the
/// time it looks. That is a race no caller can avoid, so it is absorbed here.
const EvalCall = struct {
    sink: EvalSink,
    code: []u8,
    world: []u8,
    /// The context this call actually named, so a retry can tell "the world has
    /// already moved on" (re-issue against the new one) from "the id we still
    /// hold is the dead one" (forget it and wait).
    context_id: i64 = 0,
    retried: bool = false,

    fn deinit(self: *const EvalCall) void {
        alloc.free(self.code);
        alloc.free(self.world);
    }
};

const Call = union(enum) {
    ignore,
    eval: EvalCall,
    /// The second hop for a non-primitive result: String(value) computed in the
    /// page rather than approximated from the RemoteObject's description.
    stringify: struct { sink: EvalSink, object_id: []u8 },
    add_user_script: struct { id: []u8, world: []u8, gen: u64 },
    add_channel_script: struct { name: []u8 },
    /// A getCookies correlation id.
    cookies: []u8,
    /// Page.enable's own reply: the gate every other call waits behind.
    agent_ready,
    /// Page.getFrameTree's reply, which names the main frame.
    frame_tree,
    /// Target.getTargetInfo's reply for `triggerExtensionAction`: the page
    /// target whose tab the click is for.
    trigger: struct { id: []u8, extension: []u8 },
    /// `probePaint`'s answer: the page painted at the size it was probed for.
    painted: u32,
};

const Queued = struct { method: []u8, params: []u8, call: Call };

/// A devtools call that has been sent and not yet answered.
///
/// CEF does not promise a reply. A Runtime.evaluate issued while a navigation
/// is committing can simply never be answered, and with no deadline the caller
/// waits for it forever: an app polling an evaluation reads that as the whole
/// view being dead. An expired call is reported as a failure, which a caller
/// can retry, rather than as silence it cannot.
const PendingCall = struct { view: *View, call: Call, deadline_us: i64 };

/// Evaluations are answered inside a caller's own budget or not at all, so
/// they expire fast enough for the caller to try again; everything else is a
/// registration whose only failure mode is a wedged agent.
const eval_call_timeout_us: i64 = 4 * std.time.us_per_s;
const other_call_timeout_us: i64 = 15 * std.time.us_per_s;
/// `installExtension` waits on nobody: the directory chooser
/// `developerPrivate.loadUnpacked` opens is answered by this engine's own
/// dialog handler. What it does wait on is Chromium unpacking and validating
/// the directory, which a large extension makes slow. Past this the command
/// fails with the path it was given, rather than leaving the app's promise
/// unsettled for the process's life.
const install_call_timeout_us: i64 = 90 * std.time.us_per_s;
/// The registry reads and the enable/disable writes: a promise around one
/// chrome.* callback, with no dialog and no unpacking behind it.
const registry_call_timeout_us: i64 = 30 * std.time.us_per_s;

var pending_sweep_timer: c_uint = 0;

fn armPendingSweep() void {
    if (pending_sweep_timer != 0) return;
    pending_sweep_timer = glib.timeoutAdd(250, &onPendingSweep, null);
}

fn onPendingSweep(_: ?*anyopaque) callconv(.c) c_int {
    pending_sweep_timer = 0;
    const now = glib.getMonotonicTime();
    var expired: std.ArrayList(c_int) = .empty;
    defer expired.deinit(alloc);
    var it = pending_calls.iterator();
    while (it.next()) |entry| {
        if (now > entry.value_ptr.deadline_us) expired.append(alloc, entry.key_ptr.*) catch {};
    }
    for (expired.items) |id| {
        const entry = pending_calls.fetchRemove(id) orelse continue;
        tr("cdpExpired node={d} id={d}", .{ entry.value.view.node_id, id });
        var buf: [256]u8 = undefined;
        failCall(entry.value.view, entry.value.call, expiryMessage(&buf, entry.value.call));
    }
    if (pending_calls.count() > 0) armPendingSweep();
    return 0;
}

/// What a call nobody answered tells its caller. Naming the command matters:
/// an app whose install promise never settles reads the whole feature as dead,
/// and the one thing it cannot do is work out which step stalled.
fn expiryMessage(buf: []u8, call: Call) []const u8 {
    const fallback = "the engine never answered this devtools call";
    const what = switch (call) {
        .eval => |e| switch (e.sink) {
            .json_result => |r| r.what,
            else => return fallback,
        },
        else => return fallback,
    };
    return std.fmt.bufPrint(buf, "{s}: the engine never answered", .{what}) catch fallback;
}

/// GTK thread only: every CDP result is marshaled before it is looked up here.
var pending_calls: std.AutoHashMapUnmanaged(c_int, PendingCall) = .empty;

fn sinkFree(sink: EvalSink) void {
    switch (sink) {
        .app, .app_gesture => |id| alloc.free(id),
        .extensions => |id| alloc.free(id),
        .extension_actions => |id| alloc.free(id),
        .json_result => |r| {
            alloc.free(r.id);
            alloc.free(r.what);
        },
        else => {},
    }
}

fn callFree(call: Call) void {
    switch (call) {
        .ignore => {},
        .eval => |e| {
            sinkFree(e.sink);
            e.deinit();
        },
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
        .agent_ready, .frame_tree, .painted => {},
        .trigger => |t| {
            alloc.free(t.id);
            alloc.free(t.extension);
        },
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
    if (view.browser_gone.load(.acquire)) {
        failCall(view, call, browser_gone_message);
        return true;
    }
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
    const budget: i64 = switch (call) {
        .eval => |e| switch (e.sink) {
            .json_result => |r| r.budget_us,
            else => eval_call_timeout_us,
        },
        .stringify => eval_call_timeout_us,
        else => other_call_timeout_us,
    };
    pending_calls.put(alloc, id, .{
        .view = view,
        .call = call,
        .deadline_us = glib.getMonotonicTime() + budget,
    }) catch {
        callFree(call);
        return false;
    };
    armPendingSweep();
    return true;
}

/// Queues one CDP call behind the agent handshake and records what to do with
/// its answer. Everything above this line in the file goes through here.
fn cdpSend(view: *View, method: []const u8, params_json: []const u8, call: Call) bool {
    if (view.cdp_ready or view.browser_gone.load(.acquire)) return cdpSendRaw(view, method, params_json, call);
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

const browser_gone_message = "the page's browser has closed";

/// GTK thread, once the browser is gone for good: everything parked behind the
/// agent and everything sent and not yet answered fails now.
fn failAllCalls(view: *View) void {
    view.cdp_ready = false;
    const items = view.queued.toOwnedSlice(alloc) catch &.{};
    defer alloc.free(items);
    for (items) |q| {
        alloc.free(q.method);
        alloc.free(q.params);
        failCall(view, q.call, browser_gone_message);
    }
    var mine: std.ArrayList(c_int) = .empty;
    defer mine.deinit(alloc);
    var it = pending_calls.iterator();
    while (it.next()) |entry| {
        if (entry.value_ptr.view == view) mine.append(alloc, entry.key_ptr.*) catch {};
    }
    for (mine.items) |id| {
        const entry = pending_calls.fetchRemove(id) orelse continue;
        failCall(view, entry.value.call, browser_gone_message);
    }
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
    // No buffers: by default the renderer keeps up to 100 MB of every page's
    // response bodies for Network.getResponseBody, which nothing here asks for.
    _ = cdpSendRaw(view, "Network.enable", "{\"maxTotalBufferSize\":0,\"maxResourceBufferSize\":0,\"maxPostDataSize\":0}", .ignore);
    // The main frame has to be known before any world-scoped call can be aimed;
    // frameNavigated keeps it current from here on.
    _ = cdpSendRaw(view, "Page.getFrameTree", "", .frame_tree);
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

/// Makes sure `world` exists now and after every future navigation. The
/// mechanism is a new-document script carrying the world name: CDP creates the
/// isolated world to run it in, on every load, which is exactly the lifetime an
/// isolated world needs. Page.createIsolatedWorld would answer with an id
/// sooner but only for the current document, and the id it returns is dead
/// after the next navigation.
fn ensureWorld(view: *View, world: []const u8) void {
    if (world.len == 0) return;
    if (view.worlds_requested.contains(world)) return;
    const key = alloc.dupe(u8, world) catch return;
    view.worlds_requested.put(alloc, key, {}) catch {
        alloc.free(key);
        return;
    };

    var params: std.ArrayList(u8) = .empty;
    defer params.deinit(alloc);
    params.appendSlice(alloc, "{\"source\":\"\",\"runImmediately\":true,\"worldName\":") catch return;
    cdp.quote(&params, world);
    params.appendSlice(alloc, "}") catch return;
    _ = cdpSend(view, "Page.addScriptToEvaluateOnNewDocument", params.items, .ignore);
}

fn setWorldContext(view: *View, world: []const u8, frame: []const u8, context_id: i64) void {
    const key = worldKey(world, frame) orelse return;
    const gop = view.world_contexts.getOrPut(alloc, key) catch {
        alloc.free(key);
        return;
    };
    if (gop.found_existing) alloc.free(key) else gop.key_ptr.* = key;
    gop.value_ptr.* = context_id;
    tr("worldContext node={d} world={s} frame={s} id={d} main={?s}", .{
        view.node_id, world, frame, context_id, view.main_frame,
    });
    drainDeferred(view);
}

/// The context for `world` in the frame an app means by "this view". Zero when
/// the world has no document there yet, or when the main frame is still
/// unknown, both of which park the call rather than aiming it at a guess.
fn worldContextId(view: *View, world: []const u8) i64 {
    if (world.len == 0) return 0;
    const frame = view.main_frame orelse return 0;
    const key = worldKey(world, frame) orelse return 0;
    defer alloc.free(key);
    return view.world_contexts.get(key) orelse 0;
}

fn clearWorldContexts(view: *View) void {
    tr("worldContextsCleared node={d}", .{view.node_id});
    var it = view.world_contexts.keyIterator();
    while (it.next()) |k| alloc.free(k.*);
    view.world_contexts.clearRetainingCapacity();
}

/// One context died. Every frame's copy of every world is a separate entry, so
/// this evicts by id rather than by name.
fn forgetContext(view: *View, context_id: i64) void {
    var doomed: ?[]const u8 = null;
    var it = view.world_contexts.iterator();
    while (it.next()) |entry| {
        if (entry.value_ptr.* == context_id) {
            doomed = entry.key_ptr.*;
            break;
        }
    }
    const key = doomed orelse return;
    if (view.world_contexts.fetchRemove(key)) |removed| alloc.free(removed.key);
}

/// The main frame, which every world-scoped call is aimed at unless the caller
/// says otherwise. Read once when the agent comes up and kept current from
/// Page.frameNavigated, because a cross-document navigation can replace it.
fn setMainFrame(view: *View, frame: []const u8) void {
    if (view.main_frame) |old| {
        if (std.mem.eql(u8, old, frame)) return;
        alloc.free(old);
    }
    view.main_frame = alloc.dupe(u8, frame) catch null;
    tr("mainFrame node={d} frame={s}", .{ view.node_id, frame });
    drainDeferred(view);
}

fn drainDeferred(view: *View) void {
    if (view.deferred.items.len == 0) return;
    var still: std.ArrayList(Deferred) = .empty;
    const now = glib.getMonotonicTime();
    // Take the list first: issuing a call can defer again, and appending to a
    // list being iterated is how that turns into a loop.
    const items = view.deferred.toOwnedSlice(alloc) catch return;
    defer alloc.free(items);
    for (items) |item| switch (item) {
        .eval => |e| {
            defer alloc.free(e.code);
            defer alloc.free(e.world);
            if (worldContextId(view, e.world) == 0) {
                if (e.deadline_us != 0 and now > e.deadline_us) {
                    tr("evalExpired node={d} world={s}", .{ view.node_id, e.world });
                    finishEval(view, e.sink, false, "the isolated world has no execution context");
                    continue;
                }
                const code_copy = alloc.dupe(u8, e.code) catch {
                    sinkFree(e.sink);
                    continue;
                };
                const world_copy = alloc.dupe(u8, e.world) catch {
                    alloc.free(code_copy);
                    sinkFree(e.sink);
                    continue;
                };
                still.append(alloc, .{ .eval = .{
                    .sink = e.sink,
                    .code = code_copy,
                    .world = world_copy,
                    .retried = e.retried,
                    .deadline_us = e.deadline_us,
                } }) catch {
                    alloc.free(code_copy);
                    alloc.free(world_copy);
                    sinkFree(e.sink);
                };
                continue;
            }
            _ = issueEval(view, e.sink, e.code, e.world, e.retried);
        },
    };
    view.deferred = still;
    if (view.deferred.items.len > 0) armDeferredTimer(view);
}

/// Nothing else wakes a deferred call whose world never turns up, so the
/// deadline needs a clock of its own.
fn armDeferredTimer(view: *View) void {
    if (view.deferred_timer != 0) return;
    view.deferred_timer = glib.timeoutAdd(250, &onDeferredTimer, view);
}

fn onDeferredTimer(data: ?*anyopaque) callconv(.c) c_int {
    const view: *View = @ptrCast(@alignCast(data.?));
    view.deferred_timer = 0;
    drainDeferred(view);
    return 0;
}

// ============================================================================
// Evaluation
// ============================================================================

/// One evaluation, world-aware. A world with no execution context yet parks the
/// call rather than running it in the wrong one.
fn startEval(view: *View, sink: EvalSink, code: []const u8, world: []const u8) bool {
    return startEvalRetry(view, sink, code, world, false);
}

fn startEvalRetry(view: *View, sink: EvalSink, code: []const u8, world: []const u8, retried: bool) bool {
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
                .retried = retried,
                .deadline_us = glib.getMonotonicTime() + world_wait_us,
            } }) catch {
                alloc.free(code_copy);
                alloc.free(world_copy);
                return false;
            };
            armDeferredTimer(view);
            return true;
        }
    }
    return issueEval(view, sink, code, world, retried);
}

fn issueEval(view: *View, sink: EvalSink, code: []const u8, world: []const u8, retried: bool) bool {
    var params: std.ArrayList(u8) = .empty;
    defer params.deinit(alloc);
    params.appendSlice(alloc, "{\"expression\":") catch return false;
    cdp.quote(&params, code);
    // returnByValue stays false so an object comes back as a handle this can
    // stringify in the page; awaitPromise stays false because WebKitGTK's
    // evaluate_javascript does not await either, and a Promise has to
    // stringify as "[object Promise]" on both engines. The extension commands
    // are the exception: they query Chromium's own callback APIs and there is
    // no app code on the other side of them to be consistent with. So is a
    // gesture call: WebKit has no gesture to be consistent about, and what it
    // exists for (requestPictureInPicture, requestFullscreen) answers with a
    // promise whose outcome is the whole point of the call.
    params.appendSlice(alloc, ",\"returnByValue\":false,\"awaitPromise\":") catch return false;
    const await_promise = sink == .extensions or sink == .json_result or sink == .extension_actions or
        sink == .app_gesture;
    params.appendSlice(alloc, if (await_promise) "true" else "false") catch return false;
    // `chrome.management.uninstall` refuses without one, and so do
    // requestPictureInPicture and requestFullscreen; the gesture that stands
    // behind these calls is the app's own button or shortcut.
    if (sink == .json_result or sink == .app_gesture) params.appendSlice(alloc, ",\"userGesture\":true") catch return false;
    params.appendSlice(alloc, ",\"objectGroup\":\"nd\"") catch return false;
    const context_id = worldContextId(view, world);
    if (context_id != 0) {
        var buf: [32]u8 = undefined;
        const n = std.fmt.bufPrint(&buf, ",\"contextId\":{d}", .{context_id}) catch return false;
        params.appendSlice(alloc, n) catch return false;
    }
    params.appendSlice(alloc, "}") catch return false;
    const code_copy = alloc.dupe(u8, code) catch return false;
    const world_copy = alloc.dupe(u8, world) catch {
        alloc.free(code_copy);
        return false;
    };
    return cdpSend(view, "Runtime.evaluate", params.items, .{ .eval = .{
        .sink = sink,
        .code = code_copy,
        .world = world_copy,
        .context_id = context_id,
        .retried = retried,
    } });
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
        .app, .app_gesture => |id| {
            defer alloc.free(id);
            const f = emit orelse return;
            var payload: std.json.ObjectMap = .empty;
            defer payload.deinit(alloc);
            payload.put(alloc, "id", .{ .string = id }) catch return;
            payload.put(alloc, "ok", .{ .bool = ok }) catch return;
            payload.put(alloc, if (ok) "value" else "error", .{ .string = text }) catch return;
            f(view.node_id, "javaScriptResult", .{ .data = .{ .object = payload } });
        },
        .extensions => |id| {
            defer alloc.free(id);
            const f = emit orelse return;
            var payload: std.json.ObjectMap = .empty;
            defer payload.deinit(alloc);
            payload.put(alloc, "id", .{ .string = id }) catch return;
            payload.put(alloc, "ok", .{ .bool = ok }) catch return;
            if (!ok) {
                payload.put(alloc, "error", .{ .string = text }) catch return;
                f(view.node_id, "extensionsList", .{ .data = .{ .object = payload } });
                return;
            }
            var parsed = std.json.parseFromSlice(std.json.Value, alloc, text, .{}) catch {
                payload.put(alloc, "ok", .{ .bool = false }) catch return;
                payload.put(alloc, "error", .{ .string = "listExtensions: unreadable answer" }) catch return;
                f(view.node_id, "extensionsList", .{ .data = .{ .object = payload } });
                return;
            };
            defer parsed.deinit();
            payload.put(alloc, "extensions", parsed.value) catch return;
            f(view.node_id, "extensionsList", .{ .data = .{ .object = payload } });
        },
        .extension_actions => |id| {
            defer alloc.free(id);
            const f = emit orelse return;
            var payload: std.json.ObjectMap = .empty;
            defer payload.deinit(alloc);
            payload.put(alloc, "id", .{ .string = id }) catch return;
            payload.put(alloc, "ok", .{ .bool = ok }) catch return;
            if (!ok) {
                payload.put(alloc, "error", .{ .string = text }) catch return;
                f(view.node_id, "extensionActions", .{ .data = .{ .object = payload } });
                return;
            }
            var parsed = std.json.parseFromSlice(std.json.Value, alloc, text, .{}) catch {
                payload.put(alloc, "ok", .{ .bool = false }) catch return;
                payload.put(alloc, "error", .{ .string = "listExtensionActions: unreadable answer" }) catch return;
                f(view.node_id, "extensionActions", .{ .data = .{ .object = payload } });
                return;
            };
            defer parsed.deinit();
            var actions = buildExtensionActions(parsed.value) orelse return;
            defer actions.deinit();
            payload.put(alloc, "actions", .{ .array = actions.items }) catch return;
            f(view.node_id, "extensionActions", .{ .data = .{ .object = payload } });
        },
        .json_result => |result| {
            defer alloc.free(result.id);
            defer alloc.free(result.what);
            if (std.mem.startsWith(u8, result.what, "installExtension")) {
                view.dialog_lock.lock();
                view.install_in_flight = false;
                view.dialog_lock.unlock();
            }
            const f = emit orelse return;
            var payload: std.json.ObjectMap = .empty;
            defer payload.deinit(alloc);
            payload.put(alloc, "id", .{ .string = result.id }) catch return;
            payload.put(alloc, "ok", .{ .bool = ok }) catch return;
            if (!ok) {
                payload.put(alloc, "error", .{ .string = text }) catch return;
                f(view.node_id, result.event, .{ .data = .{ .object = payload } });
                return;
            }
            var parsed = std.json.parseFromSlice(std.json.Value, alloc, text, .{}) catch {
                payload.put(alloc, "ok", .{ .bool = false }) catch return;
                payload.put(alloc, "error", .{ .string = "unreadable answer" }) catch return;
                f(view.node_id, result.event, .{ .data = .{ .object = payload } });
                return;
            };
            defer parsed.deinit();
            payload.put(alloc, result.key, parsed.value) catch return;
            f(view.node_id, result.event, .{ .data = .{ .object = payload } });
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
        const message = cdpErrorText(root);
        // A world-scoped call whose context died under it is not a failure to
        // report: the document committed between the send and the answer, and
        // the world already has (or is about to have) a new context. Forget the
        // dead id and let the call wait for the new one.
        if (call == .eval and !call.eval.retried and isMissingContext(message)) {
            const e = call.eval;
            defer e.deinit();
            // The new context usually lands BEFORE the failure does, which is
            // the whole race: re-issue against it. If it has not, this fails
            // like any other error rather than waiting for a context that may
            // never come, because a caller can retry and a hung call cannot.
            const current = worldContextId(view, e.world);
            tr("evalRetry node={d} world={s} dead={d} now={d}", .{ view.node_id, e.world, e.context_id, current });
            if (current != 0 and current != e.context_id) {
                if (issueEval(view, e.sink, e.code, e.world, true)) return;
            }
            finishEval(view, e.sink, false, message);
            return;
        }
        failCall(view, call, message);
        return;
    }

    switch (call) {
        .ignore, .agent_ready => {},
        .painted => |gen| motionPainted(view, gen),
        .trigger => |t| {
            const target = switch (root) {
                .object => |o| o.get("targetInfo") orelse .null,
                else => .null,
            };
            const page = stringField(target, "targetId") orelse {
                defer callFree(call);
                return emitTriggered(view, t.id, "triggerExtensionAction: page target unknown");
            };
            startTrigger(view, t.id, t.extension, page, stringField(target, "url") orelse "");
        },
        .frame_tree => {
            const tree = switch (root) {
                .object => |o| o.get("frameTree") orelse return,
                else => return,
            };
            if (tree != .object) return;
            const frame = tree.object.get("frame") orelse return;
            if (frame != .object) return;
            const id = switch (frame.object.get("id") orelse return) {
                .string => |str| str,
                else => return,
            };
            setMainFrame(view, id);
        },
        .eval => |e| {
            defer e.deinit();
            const sink = e.sink;
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
        .eval => |e| {
            defer e.deinit();
            finishEval(view, e.sink, false, message);
        },
        .stringify => |s| {
            alloc.free(s.object_id);
            finishEval(view, s.sink, false, message);
        },
        .trigger => |t| {
            defer callFree(call);
            emitTriggered(view, t.id, message);
        },
        // A page that cannot answer is not waited on.
        .painted => |gen| motionPainted(view, gen),
        else => callFree(call),
    }
}

fn isMissingContext(message: []const u8) bool {
    return std.mem.indexOf(u8, message, "Cannot find context") != null or
        std.mem.indexOf(u8, message, "Execution context was destroyed") != null;
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
            .string => |str| str,
            else => "",
        };
        if (name.len == 0) return; // the page's own world needs no id
        const aux = context.object.get("auxData") orelse return;
        if (aux != .object) return;
        // An isolated world only. The default context carries the same frame
        // and an evaluation with no contextId already lands there.
        if (aux.object.get("isDefault")) |is_default| {
            if (is_default == .bool and is_default.bool) return;
        }
        const frame = switch (aux.object.get("frameId") orelse return) {
            .string => |str| str,
            else => return,
        };
        setWorldContext(view, name, frame, id);
        return;
    }
    if (std.mem.eql(u8, method, "Runtime.executionContextsCleared")) {
        clearWorldContexts(view);
        return;
    }
    if (std.mem.eql(u8, method, "Runtime.executionContextDestroyed")) {
        // The per-context event, which is what an ordinary navigation or
        // reload actually sends; executionContextsCleared only arrives on the
        // transitions that reset the whole agent.
        const id = switch (root) {
            .object => |o| switch (o.get("executionContextId") orelse return) {
                .integer => |i| i,
                .float => |fv| @as(i64, @intFromFloat(fv)),
                else => return,
            },
            else => return,
        };
        forgetContext(view, id);
        return;
    }
    if (std.mem.eql(u8, method, "Page.frameNavigated")) {
        const frame = switch (root) {
            .object => |o| o.get("frame") orelse return,
            else => return,
        };
        if (frame != .object) return;
        // A subframe navigating is not this view moving; only the frame with
        // no parent is what an app calls "the page".
        if (frame.object.get("parentId") != null) return;
        const id = switch (frame.object.get("id") orelse return) {
            .string => |str| str,
            else => return,
        };
        setMainFrame(view, id);
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

/// Which world a binding name belongs to. Bindings are per world, not per
/// frame, so this walks the registered world names rather than the contexts.
fn worldForBinding(view: *View, binding: []const u8) ?[]const u8 {
    if (std.mem.eql(u8, binding, page_binding)) return "";
    var it = view.worlds_requested.keyIterator();
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
    if (std.mem.eql(u8, binding, extensions_changed_binding)) {
        var parsed = std.json.parseFromSlice(std.json.Value, alloc, payload_text, .{}) catch return;
        defer parsed.deinit();
        const reason = stringField(parsed.value, "reason") orelse "";
        emitExtensionsChanged(view, reason, stringField(parsed.value, "id") orelse "");
        return;
    }
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
    // runImmediately, matching the AppKit engine. A registration that lands
    // after the document has already committed would otherwise apply to the
    // NEXT document only, and there is a window where that is guaranteed: every
    // call is parked until the devtools agent attaches, and the browser is
    // loading its first page throughout. An extension's background page has no
    // next document, so its bootstrap simply never ran.
    //
    // WebKitGTK's add_script does not touch the live document, so this is a
    // deliberate divergence: a script may run once in the initial empty
    // document and again in the real one, which is what `match_about_blank`
    // exists for and what the mac engine already does.
    addNewDocumentScript(view, body, world, true, .{ .add_user_script = .{ .id = id_copy, .world = world_copy, .gen = gen } });
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
    const gesture = if (obj.get("userGesture")) |g| g == .bool and g.bool else false;
    const id_copy = alloc.dupe(u8, id) catch return;
    const sink: EvalSink = if (gesture) .{ .app_gesture = id_copy } else .{ .app = id_copy };
    if (!startEval(view, sink, code, world)) alloc.free(id_copy);
}

/// Chromium keeps its extension registry behind chrome.developerPrivate, which
/// only chrome://extensions has, so this runs there: an app lists extensions by
/// pointing a view (a hidden one will do) at chrome://extensions and sending
/// this command to it. Anywhere else the answer is an error rather than a
/// silent empty list.
const list_extensions_js =
    \\(async () => {
    \\  if (typeof chrome === "undefined" || !chrome.developerPrivate) {
    \\    throw new Error("listExtensions needs a view showing chrome://extensions");
    \\  }
    \\  const list = await new Promise((resolve) => chrome.developerPrivate.getExtensionsInfo(
    \\    { includeDisabled: true, includeTerminated: true }, resolve));
    \\  return JSON.stringify(list.filter((e) => e.type === "EXTENSION" && e.id !== "pfbmaghgajhpjaobhbamhamgbcelckhd").map((e) => ({
    \\    id: e.id,
    \\    name: e.name,
    \\    version: e.version,
    \\    enabled: e.state === "ENABLED",
    \\    iconUrl: e.iconUrl || "",
    \\    optionsUrl: (e.optionsPage && e.optionsPage.url) || "",
    \\  })));
    \\})()
;

/// The binding `watchExtensions` subscribes the registry's own events to. One
/// name for the view, in the page's own world: chrome://extensions runs no user
/// scripts and has no isolated world of this engine's to put it in.
const extensions_changed_binding = "__ndExtensionsChanged";

/// Which of Chromium's registry events this build actually exposes to
/// chrome://extensions is not something the host can know from a header, so
/// nothing is assumed: every candidate is feature-detected in the page and the
/// command answers with the ones it attached to. An answer with an empty list
/// is a failure, not a silent no-op.
///
/// `developerPrivate.onItemStateChanged` is the event the real Extensions page
/// listens to, so it carries the whole vocabulary (installed, uninstalled,
/// loaded, unloaded, prefs changed); `chrome.management`'s four are the public
/// spelling of the same thing and are attached beside it so a build that binds
/// only one of the two still reports.
const watch_extensions_js =
    \\(() => {
    \\  if (typeof chrome === "undefined" || !chrome.developerPrivate) {
    \\    throw new Error("watchExtensions needs a view showing chrome://extensions");
    \\  }
    \\  if (globalThis.__ndExtensionsWatch) return JSON.stringify(globalThis.__ndExtensionsWatch);
    \\  const send = (reason, id) => {
    \\    try { globalThis.__ndExtensionsChanged(JSON.stringify({ reason: String(reason), id: id ? String(id) : "" })); } catch (e) {}
    \\  };
    \\  const sources = [];
    \\  const item = chrome.developerPrivate.onItemStateChanged;
    \\  if (item && typeof item.addListener === "function") {
    \\    item.addListener((e) => send((e && e.event_type) || "itemStateChanged", e && e.item_id));
    \\    sources.push("developerPrivate.onItemStateChanged");
    \\  }
    \\  for (const name of ["onInstalled", "onUninstalled", "onEnabled", "onDisabled"]) {
    \\    const ev = chrome.management && chrome.management[name];
    \\    if (ev && typeof ev.addListener === "function") {
    \\      ev.addListener((info) => send(name, typeof info === "string" ? info : info && info.id));
    \\      sources.push("management." + name);
    \\    }
    \\  }
    \\  if (sources.length === 0) throw new Error("watchExtensions: this page exposes no registry events");
    \\  globalThis.__ndExtensionsWatch = sources;
    \\  return JSON.stringify(sources);
    \\})()
;

/// Subscribes the view to the registry's own change events. Without it an app
/// has no way to learn that an extension was installed, removed, enabled,
/// disabled or updated: a Web Store install happens entirely inside Chromium
/// and reaches no <webview> callback, so the app is left polling.
fn cmdWatchExtensions(view: *View, arg: ?std.json.Value) void {
    const id = extensionCommandId(arg, "watchExtensions") orelse return;
    if (!view.extensions_watched) {
        var params: std.ArrayList(u8) = .empty;
        defer params.deinit(alloc);
        params.appendSlice(alloc, "{\"name\":") catch return;
        cdp.quote(&params, extensions_changed_binding);
        params.appendSlice(alloc, "}") catch return;
        // Re-adding a binding is a protocol error rather than a no-op, and the
        // app is expected to call this again after the view reloads.
        _ = cdpSend(view, "Runtime.addBinding", params.items, .ignore);
        view.extensions_watched = true;
    }
    const what = alloc.dupe(u8, "watchExtensions") catch return;
    startJsonCommand(view, id, "extensionsChanged", "sources", what, registry_call_timeout_us, watch_extensions_js);
}

/// A registry change reported by the page's own subscription. No correlation
/// id: the app's listener is the whole audience.
fn emitExtensionsChanged(view: *View, reason: []const u8, extension_id: []const u8) void {
    const f = emit orelse return;
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    payload.put(alloc, "reason", .{ .string = reason }) catch return;
    payload.put(alloc, "extensionId", .{ .string = extension_id }) catch return;
    tr("extensionsChanged node={d} reason={s}", .{ view.node_id, reason });
    f(view.node_id, "extensionsChanged", .{ .data = .{ .object = payload } });
}

fn cmdListExtensions(view: *View, arg: ?std.json.Value) void {
    const obj = argObject(arg) orelse {
        std.debug.print("ND_WARN WebView listExtensions: malformed arg (expected {{id}})\n", .{});
        return;
    };
    const id = objStr(obj, "id") orelse {
        std.debug.print("ND_WARN WebView listExtensions: malformed arg (expected {{id}})\n", .{});
        return;
    };
    const id_copy = alloc.dupe(u8, id) catch return;
    if (!startEval(view, .{ .extensions = id_copy }, list_extensions_js, "")) alloc.free(id_copy);
}

/// The registry rows an action is built from. The action itself is declared in
/// the manifest and nowhere Chromium will hand it over: chrome://extensions
/// cannot fetch `chrome-extension://<id>/manifest.json` (it is not a
/// web-accessible resource and the WebUI origin is not the extension's), and
/// `developerPrivate` reports commands and pinning but not the action's popup,
/// title or icon. So the page answers with where each extension lives and the
/// host reads the manifest off disk.
const list_extension_actions_js =
    \\(async () => {
    \\  if (typeof chrome === "undefined" || !chrome.developerPrivate) {
    \\    throw new Error("listExtensionActions needs a view showing chrome://extensions");
    \\  }
    \\  const list = await new Promise((resolve) => chrome.developerPrivate.getExtensionsInfo(
    \\    { includeDisabled: true, includeTerminated: true }, resolve));
    \\  return JSON.stringify(list.filter((e) => e.type === "EXTENSION" && e.id !== "pfbmaghgajhpjaobhbamhamgbcelckhd").map((e) => ({
    \\    id: e.id,
    \\    name: e.name,
    \\    version: e.version,
    \\    enabled: e.state === "ENABLED",
    \\    iconUrl: e.iconUrl || "",
    \\    path: e.path || "",
    \\  })));
    \\})()
;

fn cmdListExtensionActions(view: *View, arg: ?std.json.Value) void {
    const id = extensionCommandId(arg, "listExtensionActions") orelse return;
    const id_copy = alloc.dupe(u8, id) catch return;
    if (!startEval(view, .{ .extension_actions = id_copy }, list_extension_actions_js, "")) {
        alloc.free(id_copy);
    }
}

/// An action's state as Chromium holds it right now, which is not what the
/// manifest says. `chrome.action.setPopup`, `setBadgeText`, `setIcon` and
/// `setTitle` are answered to the extension alone, and `chrome://extensions` is
/// told none of it: `developerPrivate` has no action field and the WebUI has no
/// `chrome.action` at all (both checked on 151). A page of the extension does
/// have the API, so this command is sent to a view showing one, which for an
/// app drawing its own toolbar is the popup it mounts for a click.
///
/// An extension that clears its popup means it: 1Password sets it to "" while
/// no account is configured so that a toolbar click opens its onboarding
/// instead, and an app that opens the manifest's popup anyway shows a document
/// the extension never meant to be on screen.
///
/// Which tab the state is read for is Chromium's own answer, not this engine's.
/// `cef_browser_t::get_identifier` says in the header that it "is also used as
/// the tabId for extension APIs", and under Chrome style it is not: a view's
/// identifier is CEF's own small counter and Chromium answers `No tab with id:
/// 1` for it. `{ active: true, lastFocusedWindow: true }` is what an extension
/// itself uses, and it resolves to the page the app last had focus in.
const read_action_js =
    \\(async () => {
    \\  if (typeof chrome === "undefined" || !chrome.action) {
    \\    throw new Error("readExtensionAction needs a view showing a page of the extension");
    \\  }
    \\  const active = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
    \\  const tab = active.length ? active[0] : null;
    \\  const where = tab ? { tabId: tab.id } : {};
    \\  const color = await chrome.action.getBadgeBackgroundColor(where);
    \\  return JSON.stringify({
    \\    id: chrome.runtime.id,
    \\    tabId: tab ? tab.id : 0,
    \\    tabUrl: tab ? (tab.url || "") : "",
    \\    popupUrl: await chrome.action.getPopup(where),
    \\    badgeText: await chrome.action.getBadgeText(where),
    \\    badgeColor: Array.isArray(color) ? color : [],
    \\    title: await chrome.action.getTitle(where),
    \\    enabled: tab ? await chrome.action.isEnabled(tab.id) : true,
    \\  });
    \\})()
;

fn cmdReadExtensionAction(view: *View, arg: ?std.json.Value) void {
    const id = extensionCommandId(arg, "readExtensionAction") orelse return;
    const what = alloc.dupe(u8, "readExtensionAction") catch return;
    startJsonCommand(view, id, "extensionActions", "action", what, registry_call_timeout_us, read_action_js);
}

/// What Chrome does when its toolbar button is clicked: the popup when the
/// action has one for this tab, otherwise `action.onClicked` with the tab and
/// an `activeTab` grant on it. `Extensions.triggerAction` runs that same
/// `ExecuteUserAction`, over the browser target (browser_pipe.zig). It needs
/// Chromium's ExtensionsContainer, which only a browser with a toolbar has: a
/// `parent_window` browser gets CEF's ChildBrowserViewDelegate, whose toolbar
/// type is fixed at CEF_CTT_NONE (libcef/browser/chrome/views/
/// chrome_child_window.cc), and the command dereferences null there. So it is
/// sent only for the Views-hosted embedding.
fn cmdTriggerExtensionAction(view: *View, arg: ?std.json.Value) void {
    const id = extensionCommandId(arg, "triggerExtensionAction") orelse return;
    const obj = argObject(arg) orelse return;
    const extension = objStr(obj, "extensionId") orelse {
        std.debug.print("ND_WARN WebView triggerExtensionAction: malformed arg (expected {{id, extensionId}})\n", .{});
        return;
    };
    if (!view.views_hosted) return emitTriggered(view, id, "triggerExtensionAction: a parent_window browser has no Chrome toolbar to click through");
    if (!browser_pipe.available()) return emitTriggered(view, id, "triggerExtensionAction: no browser protocol pipe in this process");
    const id_copy = alloc.dupe(u8, id) catch return;
    const extension_copy = alloc.dupe(u8, extension) catch {
        alloc.free(id_copy);
        return;
    };
    _ = cdpSend(view, "Target.getTargetInfo", "{}", .{ .trigger = .{ .id = id_copy, .extension = extension_copy } });
}

/// GTK thread. `error_text` null is success.
fn emitTriggered(view: *View, id: []const u8, error_text: ?[]const u8) void {
    const f = emit orelse return;
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    payload.put(alloc, "id", .{ .string = id }) catch return;
    payload.put(alloc, "ok", .{ .bool = error_text == null }) catch return;
    if (error_text) |e| payload.put(alloc, "error", .{ .string = e }) catch return;
    tr("triggerExtensionAction node={d} {s}", .{ view.node_id, error_text orelse "ok" });
    f(view.node_id, "extensionActions", .{ .data = .{ .object = payload } });
}

/// One click in flight, walked on the pipe's reader thread. A top-level page
/// target reports no parent (render_frame_devtools_agent_host.cc GetParentId),
/// so the tab is found the way a tab-mode client finds it: a tab session with
/// auto-attach names its page child. Tabs on the page's address go first.
const Trigger = struct {
    view: *View,
    id: []u8,
    extension: []u8,
    page: []u8,
    url: []u8,
    tabs: std.ArrayList([]u8) = .empty,
    next: usize = 0,
    session: ?[]u8 = null,
    owns: bool = false,
    error_text: ?[]u8 = null,

    fn deinit(t: *Trigger) void {
        alloc.free(t.id);
        alloc.free(t.extension);
        alloc.free(t.page);
        alloc.free(t.url);
        for (t.tabs.items) |tab| alloc.free(tab);
        t.tabs.deinit(alloc);
        if (t.session) |s| alloc.free(s);
        if (t.error_text) |e| alloc.free(e);
        alloc.destroy(t);
    }
};

/// Takes `id` and `extension`.
fn startTrigger(view: *View, id: []u8, extension: []u8, page: []const u8, url: []const u8) void {
    const t = alloc.create(Trigger) catch return;
    t.* = .{
        .view = view,
        .id = id,
        .extension = extension,
        .page = alloc.dupe(u8, page) catch return,
        .url = alloc.dupe(u8, url) catch return,
    };
    if (!browser_pipe.call("Target.getTargets", "{\"filter\":[{\"type\":\"tab\"}]}", null, &onTabs, t)) {
        finishTrigger(t, "Target.getTargets could not be sent");
    }
}

fn onTabs(ctx: *anyopaque, ok: bool, result: std.json.Value) void {
    const t: *Trigger = @ptrCast(@alignCast(ctx));
    if (!ok) return finishTrigger(t, cdpErrorText(result));
    const infos = switch (result) {
        .object => |o| o.get("targetInfos") orelse .null,
        else => .null,
    };
    if (infos == .array) {
        for ([_]bool{ true, false }) |same_url| {
            for (infos.array.items) |entry| {
                const tab = stringField(entry, "targetId") orelse continue;
                const url = stringField(entry, "url") orelse "";
                if (std.mem.eql(u8, url, t.url) != same_url) continue;
                const copy = alloc.dupe(u8, tab) catch continue;
                t.tabs.append(alloc, copy) catch alloc.free(copy);
            }
        }
    }
    nextTab(t);
}

fn nextTab(t: *Trigger) void {
    if (t.next >= t.tabs.items.len) return finishTrigger(t, "no tab target owns this page");
    const tab = t.tabs.items[t.next];
    t.next += 1;
    const params = std.fmt.allocPrint(alloc, "{{\"targetId\":\"{s}\",\"flatten\":true}}", .{tab}) catch return finishTrigger(t, "out of memory");
    defer alloc.free(params);
    if (!browser_pipe.call("Target.attachToTarget", params, null, &onAttached, t)) finishTrigger(t, "Target.attachToTarget could not be sent");
}

fn onAttached(ctx: *anyopaque, ok: bool, result: std.json.Value) void {
    const t: *Trigger = @ptrCast(@alignCast(ctx));
    const session = if (ok) stringField(result, "sessionId") else null;
    const s = session orelse return nextTab(t);
    t.session = alloc.dupe(u8, s) catch return finishTrigger(t, "out of memory");
    t.owns = false;
    browser_pipe.listen(s, browser_pipe.listener(&onTabEvent, t));
    // Existing children are announced before the reply to setAutoAttach.
    if (!browser_pipe.call("Target.setAutoAttach", "{\"autoAttach\":true,\"waitForDebuggerOnStart\":false,\"flatten\":true}", s, &onAutoAttached, t)) {
        finishTrigger(t, "Target.setAutoAttach could not be sent");
    }
}

fn onTabEvent(ctx: *anyopaque, method: []const u8, params: std.json.Value) void {
    const t: *Trigger = @ptrCast(@alignCast(ctx));
    if (!std.mem.eql(u8, method, "Target.attachedToTarget")) return;
    const target = switch (params) {
        .object => |o| o.get("targetInfo") orelse return,
        else => return,
    };
    const child = stringField(target, "targetId") orelse return;
    if (std.mem.eql(u8, child, t.page)) t.owns = true;
}

fn onAutoAttached(ctx: *anyopaque, _: bool, _: std.json.Value) void {
    const t: *Trigger = @ptrCast(@alignCast(ctx));
    const session = t.session orelse return nextTab(t);
    browser_pipe.listen(session, null);
    const params = std.fmt.allocPrint(alloc, "{{\"sessionId\":\"{s}\"}}", .{session}) catch return finishTrigger(t, "out of memory");
    defer alloc.free(params);
    _ = browser_pipe.call("Target.detachFromTarget", params, null, &ignoreReply, t);
    alloc.free(session);
    t.session = null;
    if (!t.owns) return nextTab(t);
    const tab = t.tabs.items[t.next - 1];
    const action = std.fmt.allocPrint(alloc, "{{\"id\":\"{s}\",\"targetId\":\"{s}\"}}", .{ t.extension, tab }) catch return finishTrigger(t, "out of memory");
    defer alloc.free(action);
    if (!browser_pipe.call("Extensions.triggerAction", action, null, &onTriggered, t)) finishTrigger(t, "Extensions.triggerAction could not be sent");
}

fn ignoreReply(_: *anyopaque, _: bool, _: std.json.Value) void {}

fn onTriggered(ctx: *anyopaque, ok: bool, result: std.json.Value) void {
    const t: *Trigger = @ptrCast(@alignCast(ctx));
    finishTrigger(t, if (ok) null else cdpErrorText(result));
}

/// Reader thread to GTK thread. The detach above may still be in flight, and
/// its reply names this object as its context, so the object outlives it by
/// being freed on the GTK side after the answer has gone out.
fn finishTrigger(t: *Trigger, error_text: ?[]const u8) void {
    if (error_text) |e| t.error_text = std.fmt.allocPrint(alloc, "triggerExtensionAction: {s}", .{e}) catch null;
    if (error_text != null and t.error_text == null) t.error_text = alloc.dupe(u8, "triggerExtensionAction failed") catch null;
    _ = glib.idleAddFull(glib.PRIORITY_DEFAULT, &deliverTrigger, t, null);
}

fn deliverTrigger(data: ?*anyopaque) callconv(.c) c_int {
    const t: *Trigger = @ptrCast(@alignCast(data.?));
    defer t.deinit();
    if (live_views.contains(@intFromPtr(t.view))) emitTriggered(t.view, t.id, t.error_text);
    return 0;
}

const manifest_limit: usize = 1 << 20;

/// An extension's manifest.json. `developerPrivate` reports `path` for an
/// unpacked extension and nothing for one from the store, which lives under the
/// profile at `Extensions/<id>/<version>_<n>`; `n` counts reinstalls of the
/// same version in place, so the first few are tried rather than the directory
/// being listed.
fn readExtensionManifest(id: []const u8, version: []const u8, path: []const u8) ?[]u8 {
    if (path.len > 0) return readFileUnder(path, "manifest.json");
    const root = defaultCacheRoot() orelse return null;
    defer alloc.free(root);
    for (0..4) |n| {
        const dir = std.fmt.allocPrint(alloc, "{s}/Default/Extensions/{s}/{s}_{d}", .{ root, id, version, n }) catch return null;
        defer alloc.free(dir);
        if (readFileUnder(dir, "manifest.json")) |manifest| return manifest;
    }
    return null;
}

fn readFileUnder(dir: []const u8, name: []const u8) ?[]u8 {
    const path = std.fmt.allocPrintSentinel(alloc, "{s}/{s}", .{ dir, name }, 0) catch return null;
    defer alloc.free(path);
    const file = std.c.fopen(path.ptr, "rb") orelse return null;
    defer _ = std.c.fclose(file);
    const buf = alloc.alloc(u8, manifest_limit) catch return null;
    const n = std.c.fread(buf.ptr, 1, buf.len, file);
    if (n == 0) {
        alloc.free(buf);
        return null;
    }
    return alloc.realloc(buf, n) catch buf[0..n];
}

/// The manifest's action block, whatever key it is under. MV2 spells it
/// `browser_action` or `page_action`; both still load.
fn manifestAction(manifest: std.json.Value) ?std.json.ObjectMap {
    const root = switch (manifest) {
        .object => |o| o,
        else => return null,
    };
    for ([_][]const u8{ "action", "browser_action", "page_action" }) |key| {
        const value = root.get(key) orelse continue;
        if (value == .object) return value.object;
    }
    return null;
}

/// The `extensionActions` payload plus everything allocated to build it. The
/// strings in the answer come out of manifests that are parsed and freed one at
/// a time, so they are copied here rather than borrowed.
const ActionList = struct {
    /// Managed, unlike the unmanaged list beside it: std.json.Array carries its
    /// own allocator.
    items: std.json.Array,
    strings: std.ArrayList([]u8) = .empty,

    fn own(self: *ActionList, value: []const u8) ?[]const u8 {
        const copy = alloc.dupe(u8, value) catch return null;
        return self.adopt(copy);
    }

    fn adopt(self: *ActionList, value: []u8) ?[]const u8 {
        self.strings.append(alloc, value) catch {
            alloc.free(value);
            return null;
        };
        return value;
    }

    fn deinit(self: *ActionList) void {
        for (self.items.items) |value| {
            if (value == .object) {
                var obj = value.object;
                obj.deinit(alloc);
            }
        }
        self.items.deinit();
        for (self.strings.items) |s| alloc.free(s);
        self.strings.deinit(alloc);
    }
};

fn buildExtensionActions(list: std.json.Value) ?ActionList {
    var out: ActionList = .{ .items = .init(alloc) };
    const rows = switch (list) {
        .array => |a| a,
        else => return out,
    };
    for (rows.items) |row| {
        const obj = switch (row) {
            .object => |o| o,
            else => continue,
        };
        const id = objStr(obj, "id") orelse continue;
        const manifest_text = readExtensionManifest(id, objStr(obj, "version") orelse "", objStr(obj, "path") orelse "") orelse continue;
        defer alloc.free(manifest_text);
        var manifest = std.json.parseFromSlice(std.json.Value, alloc, manifest_text, .{}) catch continue;
        defer manifest.deinit();
        const action = manifestAction(manifest.value) orelse continue;

        const name = objStr(obj, "name") orelse "";
        const title = out.own(objStr(action, "default_title") orelse name) orelse continue;
        var icon_url: []const u8 = objStr(obj, "iconUrl") orelse "";
        if (actionIcon(action)) |icon| {
            icon_url = out.adopt(std.fmt.allocPrint(alloc, "chrome-extension://{s}/{s}", .{ id, icon }) catch continue) orelse continue;
        }
        var popup_url: []const u8 = "";
        if (objStr(action, "default_popup")) |popup| {
            popup_url = out.adopt(std.fmt.allocPrint(alloc, "chrome-extension://{s}/{s}", .{ id, popup }) catch continue) orelse continue;
        }

        var entry: std.json.ObjectMap = .empty;
        entry.put(alloc, "id", .{ .string = id }) catch continue;
        entry.put(alloc, "name", .{ .string = name }) catch continue;
        entry.put(alloc, "enabled", .{ .bool = objBool(obj, "enabled") orelse false }) catch continue;
        entry.put(alloc, "title", .{ .string = title }) catch continue;
        entry.put(alloc, "iconUrl", .{ .string = icon_url }) catch continue;
        entry.put(alloc, "popupUrl", .{ .string = popup_url }) catch continue;
        entry.put(alloc, "badgeText", .{ .string = "" }) catch continue;
        out.items.append(.{ .object = entry }) catch {
            entry.deinit(alloc);
            continue;
        };
    }
    return out;
}

/// `default_icon` is either one path or a size-keyed map; the largest size wins,
/// which is what a toolbar wants on a HiDPI display.
fn actionIcon(action: std.json.ObjectMap) ?[]const u8 {
    const value = action.get("default_icon") orelse return null;
    switch (value) {
        .string => |s| return s,
        .object => |sizes| {
            var best: ?[]const u8 = null;
            var best_size: i64 = -1;
            var it = sizes.iterator();
            while (it.next()) |entry| {
                if (entry.value_ptr.* != .string) continue;
                const size = std.fmt.parseInt(i64, entry.key_ptr.*, 10) catch 0;
                if (size > best_size) {
                    best_size = size;
                    best = entry.value_ptr.string;
                }
            }
            return best;
        },
        else => return null,
    }
}

/// The three commands that change the registry share a body: they run against
/// chrome://extensions, do their one thing, and answer with the list the
/// registry became, which is the document `listExtensions` produces. One
/// function expression, because Runtime.evaluate leaves a top-level `const` in
/// the page's global scope and the second call would redeclare it.
const extension_mutation_prefix =
    \\(async () => {
    \\  if (typeof chrome === "undefined" || !chrome.developerPrivate) {
    \\    throw new Error("this command needs a view showing chrome://extensions");
    \\  }
    \\  const listed = async () => {
    \\    const list = await new Promise((resolve) => chrome.developerPrivate.getExtensionsInfo(
    \\      { includeDisabled: true, includeTerminated: true }, resolve));
    \\    return JSON.stringify(list.filter((e) => e.type === "EXTENSION" && e.id !== "pfbmaghgajhpjaobhbamhamgbcelckhd").map((e) => ({
    \\      id: e.id,
    \\      name: e.name,
    \\      version: e.version,
    \\      enabled: e.state === "ENABLED",
    \\      iconUrl: e.iconUrl || "",
    \\      optionsUrl: (e.optionsPage && e.optionsPage.url) || "",
    \\    })));
    \\  };
    \\
;

const extension_mutation_suffix =
    \\
    \\  return await listed();
    \\})()
;

/// chrome://extensions has no way to load an unpacked extension from a path it
/// was handed: `developerPrivate.loadUnpacked` opens a directory chooser and
/// uses what comes back. The chooser is a CEF file dialog, so the path the app
/// asked for is parked on the view and `on_file_dialog` answers with it.
const install_extension_body =
    \\  await new Promise((resolve) => chrome.developerPrivate.updateProfileConfiguration(
    \\    { inDeveloperMode: true }, resolve));
    \\  const loaded = await new Promise((resolve, reject) => chrome.developerPrivate.loadUnpacked(
    \\    { failQuietly: true, populateError: true },
    \\    (result) => chrome.runtime.lastError
    \\      ? reject(new Error(chrome.runtime.lastError.message))
    \\      : resolve(result)));
    \\  if (loaded && loaded.error) throw new Error(loaded.error);
;

fn extensionCommandId(arg: ?std.json.Value, comptime name: []const u8) ?[]const u8 {
    const obj = argObject(arg) orelse {
        std.debug.print("ND_WARN WebView " ++ name ++ ": malformed arg (expected {{id}})\n", .{});
        return null;
    };
    return objStr(obj, "id") orelse {
        std.debug.print("ND_WARN WebView " ++ name ++ ": malformed arg (expected {{id}})\n", .{});
        return null;
    };
}

fn startJsonCommand(
    view: *View,
    id: []const u8,
    event: []const u8,
    key: []const u8,
    what: []u8,
    budget_us: i64,
    code: []const u8,
) void {
    const id_copy = alloc.dupe(u8, id) catch {
        alloc.free(what);
        return;
    };
    const sink: EvalSink = .{ .json_result = .{
        .id = id_copy,
        .event = event,
        .key = key,
        .what = what,
        .budget_us = budget_us,
    } };
    if (!startEval(view, sink, code, "")) sinkFree(sink);
}

fn cmdInstallExtension(view: *View, arg: ?std.json.Value) void {
    const obj = argObject(arg) orelse {
        std.debug.print("ND_WARN WebView installExtension: malformed arg (expected {{id, path}})\n", .{});
        return;
    };
    const id = objStr(obj, "id") orelse {
        std.debug.print("ND_WARN WebView installExtension: malformed arg (expected {{id, path}})\n", .{});
        return;
    };
    const path = objStr(obj, "path") orelse {
        std.debug.print("ND_WARN WebView installExtension: malformed arg (expected {{id, path}})\n", .{});
        return;
    };
    const parked = alloc.dupe(u8, path) catch return;
    view.dialog_lock.lock();
    if (view.pending_dialog_path) |old| alloc.free(old);
    view.pending_dialog_path = parked;
    view.install_in_flight = true;
    view.dialog_lock.unlock();

    var code: std.ArrayList(u8) = .empty;
    defer code.deinit(alloc);
    code.appendSlice(alloc, extension_mutation_prefix) catch return;
    code.appendSlice(alloc, install_extension_body) catch return;
    code.appendSlice(alloc, extension_mutation_suffix) catch return;
    const what = std.fmt.allocPrint(alloc, "installExtension {s}", .{path}) catch return;
    startJsonCommand(view, id, "extensionsList", "extensions", what, install_call_timeout_us, code.items);
}

/// `chrome.management.uninstall` from anything but the extension itself always
/// puts up Chrome's "Remove ...?" dialog (management_api.cc forces it unless
/// the caller is the target), and that dialog hangs off a toolbar no embedded
/// browser shows. So the extension removes itself: `uninstallSelf` in a hidden
/// target on its manifest.json, over the browser pipe. Every extension has
/// that document, and a hidden target is in no tab strip and no window. The
/// app asks the person first. The answer is the registry once the extension
/// has left it.
fn cmdUninstallExtension(view: *View, arg: ?std.json.Value) void {
    const obj = argObject(arg) orelse return;
    const id = objStr(obj, "id") orelse return;
    const target = objStr(obj, "extensionId") orelse {
        std.debug.print("ND_WARN WebView uninstallExtension: malformed arg (expected {{id, extensionId}})\n", .{});
        return;
    };
    for (target) |ch| {
        if (ch < 'a' or ch > 'p') return emitRemovalError(view, id, "uninstallExtension: not an extension id");
    }
    if (!browser_pipe.available()) return emitRemovalError(view, id, "uninstallExtension: no browser protocol pipe in this process");
    const r = alloc.create(Removal) catch return;
    r.* = .{
        .view = view,
        .id = alloc.dupe(u8, id) catch return alloc.destroy(r),
        .extension = alloc.dupe(u8, target) catch {
            alloc.free(r.id);
            return alloc.destroy(r);
        },
    };
    const params = std.fmt.allocPrint(alloc, "{{\"url\":\"chrome-extension://{s}/manifest.json\",\"hidden\":true,\"background\":true}}", .{r.extension}) catch return finishRemoval(r, "out of memory");
    defer alloc.free(params);
    if (!browser_pipe.call("Target.createTarget", params, null, &onRemovalCreated, r)) finishRemoval(r, "Target.createTarget could not be sent");
}

/// Walked on the pipe's reader thread, answered on the GTK thread.
const Removal = struct {
    view: *View,
    id: []u8,
    extension: []u8,
    target: ?[]u8 = null,
    session: ?[]u8 = null,
    error_text: ?[]u8 = null,

    fn deinit(r: *Removal) void {
        alloc.free(r.id);
        alloc.free(r.extension);
        if (r.target) |t| alloc.free(t);
        if (r.session) |t| alloc.free(t);
        if (r.error_text) |e| alloc.free(e);
        alloc.destroy(r);
    }
};

fn onRemovalCreated(ctx: *anyopaque, ok: bool, result: std.json.Value) void {
    const r: *Removal = @ptrCast(@alignCast(ctx));
    if (!ok) return finishRemoval(r, cdpErrorText(result));
    const t = stringField(result, "targetId") orelse return finishRemoval(r, "no context of the extension to remove it from");
    r.target = alloc.dupe(u8, t) catch return finishRemoval(r, "out of memory");
    attachRemoval(r);
}

fn attachRemoval(r: *Removal) void {
    const params = std.fmt.allocPrint(alloc, "{{\"targetId\":\"{s}\",\"flatten\":true}}", .{r.target.?}) catch return finishRemoval(r, "out of memory");
    defer alloc.free(params);
    if (!browser_pipe.call("Target.attachToTarget", params, null, &onRemovalAttached, r)) finishRemoval(r, "Target.attachToTarget could not be sent");
}

/// A new target starts on about:blank, where `chrome.management` does not
/// exist, so the call waits for the extension's own document to announce its
/// context and runs there. The context goes away with the extension, so the
/// uninstall itself is not awaited: the registry, read next, is what says it
/// worked.
fn onRemovalAttached(ctx: *anyopaque, ok: bool, result: std.json.Value) void {
    const r: *Removal = @ptrCast(@alignCast(ctx));
    if (!ok) return finishRemoval(r, cdpErrorText(result));
    const session = stringField(result, "sessionId") orelse return finishRemoval(r, "attach gave no session");
    r.session = alloc.dupe(u8, session) catch return finishRemoval(r, "out of memory");
    browser_pipe.listen(r.session.?, browser_pipe.listener(&onRemovalEvent, r));
    if (!browser_pipe.call("Runtime.enable", "{}", r.session.?, &ignoreReply, r)) finishRemoval(r, "Runtime.enable could not be sent");
}

fn onRemovalEvent(ctx: *anyopaque, method: []const u8, params: std.json.Value) void {
    const r: *Removal = @ptrCast(@alignCast(ctx));
    if (!std.mem.eql(u8, method, "Runtime.executionContextCreated")) return;
    const context = switch (params) {
        .object => |o| o.get("context") orelse return,
        else => return,
    };
    const origin = stringField(context, "origin") orelse return;
    if (!std.mem.startsWith(u8, origin, "chrome-extension://") or !std.mem.eql(u8, origin["chrome-extension://".len..], r.extension)) return;
    const context_id = switch (context) {
        .object => |o| switch (o.get("id") orelse return) {
            .integer => |i| i,
            else => return,
        },
        else => return,
    };
    browser_pipe.listen(r.session.?, null);
    const call = std.fmt.allocPrint(alloc,
        \\{{"expression":"chrome.management.uninstallSelf({{ showConfirmDialog: false }}).catch(() => {{}}), 'sent'","contextId":{d},"returnByValue":true,"userGesture":true}}
    , .{context_id}) catch return finishRemoval(r, "out of memory");
    defer alloc.free(call);
    if (!browser_pipe.call("Runtime.evaluate", call, r.session.?, &onRemovalSent, r)) finishRemoval(r, "Runtime.evaluate could not be sent");
}

fn onRemovalSent(ctx: *anyopaque, ok: bool, result: std.json.Value) void {
    const r: *Removal = @ptrCast(@alignCast(ctx));
    if (std.fmt.allocPrint(alloc, "{{\"targetId\":\"{s}\"}}", .{r.target.?})) |params| {
        defer alloc.free(params);
        _ = browser_pipe.call("Target.closeTarget", params, null, &ignoreReply, r);
    } else |_| {}
    // A protocol error is most often the context going away with the
    // extension; only an exception the page threw is an answer.
    if (ok and result == .object) {
        if (result.object.get("exceptionDetails")) |details| {
            const thrown = switch (details) {
                .object => |o| if (o.get("exception")) |e| stringField(e, "description") else null,
                else => null,
            };
            return finishRemoval(r, thrown orelse stringField(details, "text") orelse "uninstallSelf threw");
        }
    }
    finishRemoval(r, null);
}

/// Reader thread to GTK thread; a closeTarget reply may still name `r`, so it
/// is freed on the GTK side after the answer, like a Trigger.
fn finishRemoval(r: *Removal, error_text: ?[]const u8) void {
    if (error_text) |e| r.error_text = std.fmt.allocPrint(alloc, "uninstallExtension: {s}", .{e}) catch null;
    if (error_text != null and r.error_text == null) r.error_text = alloc.dupe(u8, "uninstallExtension failed") catch null;
    _ = glib.idleAddFull(glib.PRIORITY_DEFAULT, &deliverRemoval, r, null);
}

fn deliverRemoval(data: ?*anyopaque) callconv(.c) c_int {
    const r: *Removal = @ptrCast(@alignCast(data.?));
    defer r.deinit();
    if (!live_views.contains(@intFromPtr(r.view))) return 0;
    if (r.error_text) |e| {
        emitRemovalError(r.view, r.id, e);
        return 0;
    }
    var code: std.ArrayList(u8) = .empty;
    defer code.deinit(alloc);
    code.appendSlice(alloc, extension_mutation_prefix) catch return 0;
    code.appendSlice(alloc, "  const target = ") catch return 0;
    appendJsString(&code, r.extension) catch return 0;
    code.appendSlice(alloc,
        \\;
        \\  for (let i = 0; JSON.parse(await listed()).some((e) => e.id === target); i++) {
        \\    if (i >= 100) throw new Error(target + " is still installed");
        \\    await new Promise((resolve) => setTimeout(resolve, 200));
        \\  }
    ) catch return 0;
    code.appendSlice(alloc, extension_mutation_suffix) catch return 0;
    const what = alloc.dupe(u8, "uninstallExtension") catch return 0;
    startJsonCommand(r.view, r.id, "extensionsList", "extensions", what, registry_call_timeout_us, code.items);
    return 0;
}

fn emitRemovalError(view: *View, id: []const u8, error_text: []const u8) void {
    const f = emit orelse return;
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    payload.put(alloc, "id", .{ .string = id }) catch return;
    payload.put(alloc, "ok", .{ .bool = false }) catch return;
    payload.put(alloc, "error", .{ .string = error_text }) catch return;
    f(view.node_id, "extensionsList", .{ .data = .{ .object = payload } });
}

fn cmdSetExtensionEnabled(view: *View, arg: ?std.json.Value) void {
    const obj = argObject(arg) orelse return;
    const id = objStr(obj, "id") orelse return;
    const target = objStr(obj, "extensionId") orelse {
        std.debug.print("ND_WARN WebView setExtensionEnabled: malformed arg (expected {{id, extensionId, enabled}})\n", .{});
        return;
    };
    const enabled = objBool(obj, "enabled") orelse true;
    var code: std.ArrayList(u8) = .empty;
    defer code.deinit(alloc);
    code.appendSlice(alloc, extension_mutation_prefix) catch return;
    code.appendSlice(alloc, "  await new Promise((resolve, reject) => chrome.management.setEnabled(") catch return;
    appendJsString(&code, target) catch return;
    code.appendSlice(alloc, if (enabled) ", true," else ", false,") catch return;
    code.appendSlice(alloc,
        \\
        \\    () => chrome.runtime.lastError ? reject(new Error(chrome.runtime.lastError.message)) : resolve()));
    ) catch return;
    code.appendSlice(alloc, extension_mutation_suffix) catch return;
    const what = alloc.dupe(u8, "setExtensionEnabled") catch return;
    startJsonCommand(view, id, "extensionsList", "extensions", what, registry_call_timeout_us, code.items);
}

fn appendJsString(out: *std.ArrayList(u8), value: []const u8) !void {
    try out.append(alloc, '"');
    for (value) |ch| {
        switch (ch) {
            '"', '\\' => {
                try out.append(alloc, '\\');
                try out.append(alloc, ch);
            },
            '\n' => try out.appendSlice(alloc, "\\n"),
            '\r' => try out.appendSlice(alloc, "\\r"),
            else => try out.append(alloc, ch),
        }
    }
    try out.append(alloc, '"');
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
    postZoomChange(.{ .view = view, .host = host, .level = std.math.log2(factor) / std.math.log2(1.2), .source = "app" });
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

/// The app's half of the toggle. Posted to the CEF UI thread because that is
/// where a browser host may be asked to close anything.
fn cmdCloseDevTools(view: *View) void {
    if (view.devtools_window.load(.acquire) == 0) return;
    const api = loader.loaded() orelse return;
    if (api.currently_on(c.TID_UI) != 0) {
        closeDockedDevTools(view);
        return;
    }
    const host = hostOf(view) orelse return;
    const task = ChromeCommandObj.create(.{ .host = host, .command_id = 0 }) orelse return;
    task.cef.execute = &runCloseDevToolsTask;
    if (api.post_task(c.TID_UI, task.handOut()) == 0) task.drop();
    task.drop();
}

fn runCloseDevToolsTask(self: [*c]c.cef_task_t) callconv(.c) void {
    const call = ChromeCommandObj.of(self).payload;
    if (call.host.close_dev_tools) |close| close(call.host);
}

fn cmdFocus(view: *View) void {
    _ = gtk.Widget.grabFocus(view.widget);
    // The widget can already BE the focus widget, in which case grabFocus
    // notifies nothing and only this call tells the browser.
    syncBrowserFocus(view);
}

fn jsonToInt(v: std.json.Value) i64 {
    return switch (v) {
        .integer => |n| n,
        .float => |f| @intFromFloat(f),
        else => 0,
    };
}

/// Opens Chromium's devtools. Under Chrome style that is Chrome's own docked
/// inspector inside the browser's contents container, which is both what the
/// user expects and the only devtools this engine can have without a top-level
/// window. Under Alloy it is CEF's separate devtools window (parenting it into
/// a GTK window crashes, CEF #3165). Optional arg `{x, y}` (view-relative CSS
/// pixels, the contextMenu event's coordinates) starts it with that element
/// inspected, which is what an app's "Inspect Element" menu item wants.
fn cmdOpenDevTools(view: *View, arg: ?std.json.Value) void {
    const host = hostOf(view) orelse return;
    if (chromeStyle()) {
        const api = loader.loaded() orelse return;
        const id = api.id_for_command_id_name("IDC_DEV_TOOLS");
        if (id >= 0) executeChromeCommand(host, id);
        return;
    }
    const show = host.show_dev_tools orelse return;
    var point: c.cef_point_t = .{ .x = 0, .y = 0 };
    var have_point = false;
    if (arg) |a| {
        if (a == .object) {
            if (a.object.get("x")) |x| {
                if (a.object.get("y")) |y| {
                    point = .{ .x = @intCast(jsonToInt(x)), .y = @intCast(jsonToInt(y)) };
                    have_point = true;
                }
            }
        }
    }
    view.devtools_requested.store(true, .release);
    var window_info = std.mem.zeroes(c.cef_window_info_t);
    show(host, &window_info, null, null, if (have_point) &point else null);
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

/// Chrome style's default (returning 0) saves a copy of its own to
/// ~/Downloads and shows its download bubble. The download is claimed instead
/// and parked until the app answers `respondDownload`: a path lets Chromium
/// run the transfer there (so blob:, data:, POST and cookie-bound downloads
/// work, which the app could not fetch again), no path cancels it.
const DownloadRequest = struct {
    item_id: u32,
    callback: usize,
};

const DownloadUpdate = struct {
    item_id: u32,
    state: enum { running, done, failed, cancelled },
    received: i64,
    total: i64,
    speed: i64,
    paused: bool,
    /// The item callback, retained, for pause, resume and cancel.
    callback: usize,
};

/// GTK thread only. Keys are owned by the maps.
var pending_downloads: std.StringHashMapUnmanaged(*DownloadRequest) = .empty;
/// Downloads the app gave a path, with the last state reported, so an update
/// that changes nothing is not sent again.
var running_downloads: std.StringHashMapUnmanaged(u64) = .empty;
/// The latest item callback per running download, retained, keyed like
/// running_downloads. A failed download keeps its own: resume is how an
/// interrupted download comes back.
var download_controls: std.StringHashMapUnmanaged(usize) = .empty;

fn releaseItemCallback(token: usize) void {
    if (token == 0) return;
    const cb: [*c]c.cef_download_item_callback_t = @ptrFromInt(token);
    ref.releaseParam(cb);
}

/// `pauseDownload`, `resumeDownload`, `cancelDownload`: `{id}`. A download
/// still waiting for `respondDownload` is cancelled by never continuing it.
fn cmdControlDownload(cmd: []const u8, arg: ?std.json.Value) void {
    const obj_arg = argObject(arg) orelse return;
    const id = objStr(obj_arg, "id") orelse {
        std.debug.print("ND_WARN WebView {s}: missing id\n", .{cmd});
        return;
    };
    if (std.mem.eql(u8, cmd, "cancelDownload")) {
        if (pending_downloads.fetchRemove(id)) |entry| {
            alloc.free(entry.key);
            answerDownload(entry.value, null);
            return;
        }
    }
    const token = download_controls.get(id) orelse {
        std.debug.print("ND_WARN WebView {s}: unknown download id {s}\n", .{ cmd, id });
        return;
    };
    const cb: [*c]c.cef_download_item_callback_t = @ptrFromInt(token);
    if (std.mem.eql(u8, cmd, "pauseDownload")) {
        if (cb.*.pause) |f| f(cb);
    } else if (std.mem.eql(u8, cmd, "resumeDownload")) {
        if (cb.*.@"resume") |f| f(cb);
    } else if (cb.*.cancel) |f| f(cb);
}

fn cmdStartDownload(view: *View, arg: ?std.json.Value) void {
    const obj_arg = argObject(arg) orelse return;
    const url = objStr(obj_arg, "url") orelse {
        std.debug.print("ND_WARN WebView startDownload: missing url\n", .{});
        return;
    };
    const host = hostOf(view) orelse return;
    const start = host.start_download orelse return;
    var target = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&target);
    if (!setStr(&target, url)) return;
    start(host, &target);
}

fn onBeforeDownload(
    self: [*c]c.cef_download_handler_t,
    browser: [*c]c.cef_browser_t,
    download_item: [*c]c.cef_download_item_t,
    suggested_name: [*c]const c.cef_string_t,
    callback: [*c]c.cef_before_download_callback_t,
) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(download_item);
    if (callback == null) return 0;
    if (extensionPackage(download_item, suggested_name)) {
        tr("downloadToChrome extension package", .{});
        ref.releaseParam(callback);
        return 0;
    }
    const view = DownloadObj.of(self).payload;
    var url: ?[]u8 = null;
    var item_id: u32 = 0;
    if (download_item != null) {
        if (download_item.*.get_url) |get_url| {
            const raw = get_url(download_item);
            if (raw != null) {
                defer freeUserfree(raw);
                url = dupeStr(raw);
            }
        }
        if (download_item.*.get_id) |get_id| item_id = get_id(download_item);
    }
    const req = alloc.create(DownloadRequest) catch {
        ref.releaseParam(callback);
        if (url) |u| alloc.free(u);
        return 1;
    };
    req.* = .{ .item_id = item_id, .callback = @intFromPtr(callback) };
    post(.{
        .view = view,
        .name = "downloadRequested",
        .text = url,
        .extra = dupeStr(suggested_name),
        .download = req,
    });
    return 1;
}

/// A Web Store install fetches the extension's CRX as a download, and
/// Chrome's installer is waiting on it at a path of its own. Handed to the
/// app, it became a row in the app's downloads, and its file went where the
/// app saves downloads; Chrome's own handling keeps it out of sight, as in
/// Chrome. A .crx from anywhere else gets Chrome's answer too, which is to
/// refuse it.
fn extensionPackage(item: [*c]c.cef_download_item_t, suggested_name: [*c]const c.cef_string_t) bool {
    if (item != null) {
        if (item.*.get_mime_type) |get_mime| {
            const raw = get_mime(item);
            if (raw != null) {
                defer freeUserfree(raw);
                const mime = dupeStr(raw) orelse return false;
                defer alloc.free(mime);
                if (std.ascii.eqlIgnoreCase(mime, "application/x-chrome-extension")) return true;
            }
        }
    }
    const name = dupeStr(suggested_name) orelse return false;
    defer alloc.free(name);
    return std.ascii.endsWithIgnoreCase(name, ".crx");
}

/// Continues the download to `path`, or cancels it without one (a callback
/// released without cont() is a cancelled download), and frees the request.
fn answerDownload(req: *DownloadRequest, path: ?[]const u8) void {
    defer alloc.destroy(req);
    const cb: [*c]c.cef_before_download_callback_t = @ptrFromInt(req.callback);
    defer ref.releaseParam(cb);
    const p = path orelse return;
    var target = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&target);
    if (!setStr(&target, p)) return;
    if (cb.*.cont) |cont| cont(cb, &target, 0);
}

fn cmdRespondDownload(arg: ?std.json.Value) void {
    const obj_arg = argObject(arg) orelse return;
    const id = objStr(obj_arg, "id") orelse {
        std.debug.print("ND_WARN WebView respondDownload: missing id\n", .{});
        return;
    };
    const entry = pending_downloads.fetchRemove(id) orelse {
        std.debug.print("ND_WARN WebView respondDownload: unknown download id {s}\n", .{id});
        return;
    };
    const path = objStr(obj_arg, "path");
    if (path != null and path.?.len > 0) {
        running_downloads.put(alloc, entry.key, 0) catch alloc.free(entry.key);
        watchDownloadAnimation();
        answerDownload(entry.value, path);
    } else {
        alloc.free(entry.key);
        answerDownload(entry.value, null);
    }
}

fn onDownloadUpdated(
    self: [*c]c.cef_download_handler_t,
    browser: [*c]c.cef_browser_t,
    item: [*c]c.cef_download_item_t,
    callback: [*c]c.cef_download_item_callback_t,
) callconv(.c) void {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(item);
    if (item == null) {
        ref.releaseParam(callback);
        return;
    }
    const view = DownloadObj.of(self).payload;
    const is = struct {
        fn flag(f: ?*const fn ([*c]c.cef_download_item_t) callconv(.c) c_int, it: [*c]c.cef_download_item_t) bool {
            return if (f) |g| g(it) != 0 else false;
        }
    };
    var path: ?[]u8 = null;
    if (item.*.get_full_path) |get_path| {
        const raw = get_path(item);
        if (raw != null) {
            defer freeUserfree(raw);
            path = dupeStr(raw);
        }
    }
    post(.{
        .view = view,
        .name = "downloadUpdated",
        .text = path,
        .download_update = .{
            .item_id = if (item.*.get_id) |g| g(item) else 0,
            .state = if (is.flag(item.*.is_complete, item)) .done else if (is.flag(item.*.is_canceled, item)) .cancelled else if (is.flag(item.*.is_interrupted, item)) .failed else .running,
            .received = if (item.*.get_received_bytes) |g| g(item) else 0,
            .total = if (item.*.get_total_bytes) |g| g(item) else -1,
            .speed = if (item.*.get_current_speed) |g| g(item) else 0,
            .paused = is.flag(item.*.is_paused, item),
            .callback = @intFromPtr(callback),
        },
    });
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
// request context: "" shares the global one's storage, a named profile is a persistent
// context under the shared root_cache_path, and "private…" is a context with no
// cache path at all, which is CEF's spelling of in-memory.

/// Named profiles are shared: two views asking for the same name must see the
/// same jar. Ephemeral ones are not, by construction.
var profile_contexts: std.StringHashMapUnmanaged(*c.cef_request_context_t) = .empty;

/// The default profile's views share the global context's storage through a
/// context of their own, because the global one has no handler (cef_initialize
/// takes none) and CEF 151 finds a handler for a service worker's request only
/// through a frame in the worker's process whose context has one
/// (CefRequestContextHandlerMap::GetHandler). With it, an extension's worker is
/// seen through any page of the extension a view shows.
var default_context: ?*c.cef_request_context_t = null;

fn requestContext(profile: []const u8) ?*c.cef_request_context_t {
    const api = loader.loaded() orelse return null;
    if (profile.len == 0) {
        if (default_context) |ctx| return ctx;
        const global = api.request_context_get_global_context();
        if (global == null) return null;
        const ctx = api.request_context_create_context_shared(global, startupPrefsHandler());
        if (ctx == null) return null;
        default_context = @ptrCast(ctx);
        return default_context;
    }
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

    const ctx = api.request_context_create_context(&settings, startupPrefsHandler());
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

fn clientGetPermissionHandler(self: [*c]c.cef_client_t) callconv(.c) [*c]c.cef_permission_handler_t {
    return ClientObj.of(self).payload.permission_handler.handOut();
}

fn clientGetJsDialogHandler(self: [*c]c.cef_client_t) callconv(.c) [*c]c.cef_jsdialog_handler_t {
    return ClientObj.of(self).payload.jsdialog_handler.handOut();
}

fn clientGetDialogHandler(self: [*c]c.cef_client_t) callconv(.c) [*c]c.cef_dialog_handler_t {
    return ClientObj.of(self).payload.dialog_handler.handOut();
}

/// The chooser `installExtension` armed is answered from the parked path.
/// Every other one (a page's file input, Save Page As) is the host's own
/// GtkFileDialog, transient for the window the view is in: Chrome style has no
/// platform dialog of its own on Linux and cancels the request, and a dialog
/// that is not the app window's child would be a free toplevel of its own.
fn onFileDialog(
    self: [*c]c.cef_dialog_handler_t,
    browser: [*c]c.cef_browser_t,
    mode: c.cef_file_dialog_mode_t,
    title: [*c]const c.cef_string_t,
    default_file_path: [*c]const c.cef_string_t,
    accept_filters: c.cef_string_list_t,
    _: c.cef_string_list_t,
    _: c.cef_string_list_t,
    callback: [*c]c.cef_file_dialog_callback_t,
) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(callback);
    const view = DialogHandlerObj.of(self).payload;
    view.dialog_lock.lock();
    const path = view.pending_dialog_path;
    view.pending_dialog_path = null;
    const installing = view.install_in_flight;
    view.dialog_lock.unlock();
    const answer = path orelse {
        if (!installing) return pageFileDialog(view, mode, title, default_file_path, accept_filters, callback);
        // Cancelled rather than left to CEF: `loadUnpacked` answers a cancelled
        // chooser with an error the app's promise carries, and opens a real
        // directory chooser over the app if this returns 0.
        tr("installDialogUnarmed node={d}", .{view.node_id});
        if (callback.*.cancel) |cancel| cancel(callback);
        return 1;
    };
    defer alloc.free(answer);
    tr("installDialogAnswered node={d} path={s}", .{ view.node_id, answer });

    const api = loader.loaded() orelse return 0;
    const list = api.string_list_alloc();
    defer api.string_list_free(list);
    var entry = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&entry);
    if (!setStr(&entry, answer)) return 0;
    api.string_list_append(list, &entry);
    if (callback.*.cont) |cont| cont(callback, list);
    return 1;
}

const PageFileDialog = struct {
    view: *View,
    mode: c.cef_file_dialog_mode_t,
    title: ?[]u8,
    default_path: ?[]u8,
    filters: std.ArrayList([]u8),
    callback: [*c]c.cef_file_dialog_callback_t,
    dialog: ?*gtk.FileDialog = null,
};

/// CEF UI thread: copies what the request carries and hands it to the GTK
/// thread, holding a reference on the callback until it is answered.
fn pageFileDialog(
    view: *View,
    mode: c.cef_file_dialog_mode_t,
    title: [*c]const c.cef_string_t,
    default_file_path: [*c]const c.cef_string_t,
    accept_filters: c.cef_string_list_t,
    callback: [*c]c.cef_file_dialog_callback_t,
) c_int {
    const api = loader.loaded() orelse return 0;
    const job = alloc.create(PageFileDialog) catch return 0;
    job.* = .{
        .view = view,
        .mode = mode,
        .title = dupeStr(title),
        .default_path = dupeStr(default_file_path),
        .filters = .empty,
        .callback = callback,
    };
    if (accept_filters != null) {
        var i: usize = 0;
        while (i < api.string_list_size(accept_filters)) : (i += 1) {
            var entry = std.mem.zeroes(c.cef_string_t);
            defer api.string_utf16_clear(&entry);
            if (api.string_list_value(accept_filters, i, &entry) == 0) continue;
            const f = dupeStr(&entry) orelse continue;
            job.filters.append(alloc, f) catch alloc.free(f);
        }
    }
    ref.addRefParam(callback);
    tr("pageFileDialog node={d} mode={d}", .{ view.node_id, mode });
    _ = glib.idleAddFull(glib.PRIORITY_DEFAULT, &showPageFileDialog, job, null);
    return 1;
}

fn freePageFileDialog(job: *PageFileDialog) void {
    ref.releaseParam(job.callback);
    if (job.title) |t| alloc.free(t);
    if (job.default_path) |d| alloc.free(d);
    for (job.filters.items) |f| alloc.free(f);
    job.filters.deinit(alloc);
    if (job.dialog) |d| gobject.Object.unref(d.as(gobject.Object));
    alloc.destroy(job);
}

/// Answers the page with `paths`, or cancels it on an empty list. Any thread:
/// CEF's callback hops to its own UI thread.
fn answerPageFileDialog(job: *PageFileDialog, paths: []const []const u8) void {
    const cb = job.callback;
    if (paths.len == 0) {
        if (cb.*.cancel) |cancel| cancel(cb);
        return;
    }
    const api = loader.loaded() orelse {
        if (cb.*.cancel) |cancel| cancel(cb);
        return;
    };
    const list = api.string_list_alloc();
    defer api.string_list_free(list);
    for (paths) |p| {
        var entry = std.mem.zeroes(c.cef_string_t);
        defer clearStr(&entry);
        if (setStr(&entry, p)) api.string_list_append(list, &entry);
    }
    if (cb.*.cont) |cont| cont(cb, list);
}

/// GTK thread.
fn showPageFileDialog(data: ?*anyopaque) callconv(.c) c_int {
    const job: *PageFileDialog = @ptrCast(@alignCast(data.?));
    switch (automation_dialogs.take("webview.fileDialog")) {
        .unscripted => {},
        .exhausted => {
            std.debug.print("ND_WARN WebView fileDialog: the automation dialog script ran out of answers; cancelling\n", .{});
            answerPageFileDialog(job, &.{});
            freePageFileDialog(job);
            return 0;
        },
        .response => |raw| {
            const P = struct { paths: []const []const u8 = &.{} };
            const parsed = std.json.parseFromSlice(P, alloc, raw, .{ .ignore_unknown_fields = true }) catch null;
            defer if (parsed) |pp| pp.deinit();
            answerPageFileDialog(job, if (parsed) |pp| pp.value.paths else &.{});
            freePageFileDialog(job);
            return 0;
        },
    }
    if (!live_views.contains(@intFromPtr(job.view))) {
        answerPageFileDialog(job, &.{});
        freePageFileDialog(job);
        return 0;
    }
    const parent: ?*gtk.Window = blk: {
        const root = gtk.Widget.getRoot(job.view.widget) orelse break :blk null;
        break :blk gobject.ext.cast(gtk.Window, root);
    };

    const dialog = gtk.FileDialog.new();
    job.dialog = dialog;
    gtk.FileDialog.setModal(dialog, 1);
    if (job.title) |t| {
        if (alloc.dupeZ(u8, t)) |z| {
            defer alloc.free(z);
            gtk.FileDialog.setTitle(dialog, z.ptr);
        } else |_| {}
    }
    if (job.default_path) |d| {
        if (alloc.dupeZ(u8, d)) |z| {
            defer alloc.free(z);
            if (job.mode == c.FILE_DIALOG_SAVE) {
                const base = std.fs.path.basename(z);
                if (alloc.dupeZ(u8, base)) |bz| {
                    defer alloc.free(bz);
                    gtk.FileDialog.setInitialName(dialog, bz.ptr);
                } else |_| {}
            } else {
                const file = gio.File.newForPath(z.ptr);
                defer gobject.Object.unref(file.as(gobject.Object));
                gtk.FileDialog.setInitialFile(dialog, file);
            }
        } else |_| {}
    }
    applyPageFilters(dialog, job.filters.items);

    switch (job.mode) {
        c.FILE_DIALOG_OPEN_MULTIPLE => gtk.FileDialog.openMultiple(dialog, parent, null, &onPageFileDialogDone, job),
        c.FILE_DIALOG_OPEN_FOLDER => gtk.FileDialog.selectFolder(dialog, parent, null, &onPageFileDialogDone, job),
        c.FILE_DIALOG_SAVE => gtk.FileDialog.save(dialog, parent, null, &onPageFileDialogDone, job),
        else => gtk.FileDialog.open(dialog, parent, null, &onPageFileDialogDone, job),
    }
    return 0;
}

/// A page's accept list is extensions (".png") and MIME types ("image/*"),
/// which is exactly what a GtkFileFilter matches on. One filter carries them
/// all, the way Chrome's own dialog offers "the types this page asked for".
fn applyPageFilters(dialog: *gtk.FileDialog, filters: []const []u8) void {
    if (filters.len == 0) return;
    const filter = gtk.FileFilter.new();
    defer gobject.Object.unref(filter.as(gobject.Object));
    for (filters) |f| {
        const z = alloc.dupeZ(u8, f) catch continue;
        defer alloc.free(z);
        if (f.len > 1 and f[0] == '.') {
            const pattern = std.fmt.allocPrintSentinel(alloc, "*{s}", .{f}, 0) catch continue;
            defer alloc.free(pattern);
            gtk.FileFilter.addPattern(filter, pattern.ptr);
        } else if (std.mem.indexOfScalar(u8, f, '/') != null) {
            gtk.FileFilter.addMimeType(filter, z.ptr);
        }
    }
    gtk.FileDialog.setDefaultFilter(dialog, filter);
}

fn onPageFileDialogDone(_: ?*gobject.Object, res: *gio.AsyncResult, user_data: ?*anyopaque) callconv(.c) void {
    const job: *PageFileDialog = @ptrCast(@alignCast(user_data.?));
    defer freePageFileDialog(job);
    const dialog = job.dialog.?;
    var err: ?*glib.Error = null;
    defer if (err) |e| e.free();
    var paths: std.ArrayList([]const u8) = .empty;
    defer {
        for (paths.items) |p| alloc.free(p);
        paths.deinit(alloc);
    }
    const one: ?*gio.File = switch (job.mode) {
        c.FILE_DIALOG_OPEN_MULTIPLE => null,
        c.FILE_DIALOG_OPEN_FOLDER => gtk.FileDialog.selectFolderFinish(dialog, res, &err),
        c.FILE_DIALOG_SAVE => gtk.FileDialog.saveFinish(dialog, res, &err),
        else => gtk.FileDialog.openFinish(dialog, res, &err),
    };
    if (one) |f| {
        defer gobject.Object.unref(f.as(gobject.Object));
        if (gio.File.getPath(f)) |pz| {
            defer glib.free(pz);
            if (alloc.dupe(u8, std.mem.span(pz))) |d| paths.append(alloc, d) catch alloc.free(d) else |_| {}
        }
    } else if (job.mode == c.FILE_DIALOG_OPEN_MULTIPLE) {
        if (gtk.FileDialog.openMultipleFinish(dialog, res, &err)) |lm| {
            defer gobject.Object.unref(lm.as(gobject.Object));
            var i: c_uint = 0;
            while (i < gio.ListModel.getNItems(lm)) : (i += 1) {
                const item = gio.ListModel.getItem(lm, i) orelse continue;
                const file: *gio.File = @ptrCast(@alignCast(item));
                defer gobject.Object.unref(@ptrCast(@alignCast(item)));
                const pz = gio.File.getPath(file) orelse continue;
                defer glib.free(pz);
                if (alloc.dupe(u8, std.mem.span(pz))) |d| paths.append(alloc, d) catch alloc.free(d) else |_| {}
            }
        }
    }
    tr("pageFileDialogDone node={d} paths={d}", .{ job.view.node_id, paths.items.len });
    answerPageFileDialog(job, paths.items);
}

// ============================================================================
// Permission prompts
// ============================================================================
//
// Chrome style answers a permission request with Chromium's own prompt, a Views
// surface anchored to the toolbar this embedding does not have. On GTK it is
// drawn inside the browser's X window; on AppKit it becomes a window of its
// own, following the anchor. Either way it is Chrome's UI in a native app and
// the app has no say in it, so `cef_permission_handler_t` takes both routes
// (`on_show_permission_prompt` and the getUserMedia one) and hands the request
// to the app as `permissionRequest`, answered with `respondPermission`. An
// unanswered id leaves the page waiting, as `schemeRequest` does.
//
// The app owns persistence, so Chromium must not also own it. ACCEPT and DENY
// are both explicit user actions to Chromium and are written into the profile's
// content settings, after which that origin never asks again and the app's own
// store stops being consulted. CEF 151 has no one-time grant to answer with
// (`cef_permission_request_result_t` is accept/deny/dismiss/ignore), so the
// answer is put back to the profile default through
// `cef_request_context_t::set_content_setting` as soon as CEF says it is done
// with the prompt. `resetPermissions` is the same clearing on demand.

const PermissionResult = enum { allow, deny, dismiss };

const PermissionRequest = struct {
    id: []const u8,
    origin: []const u8,
    types: []const u8,
    /// The URL of the frame CEF named and whether it is the main one. CEF hands
    /// a frame to the getUserMedia route only; the prompt route names nothing
    /// below the browser, so `frame_url` is null there.
    frame_url: ?[]const u8,
    is_main_frame: bool,
    main_frame_url: []const u8,
    /// Chromium's own id for the prompt, so `on_dismiss_permission_prompt` can
    /// drop a request whose callback CEF has already torn down. Zero on the
    /// getUserMedia route, which has no dismissal callback.
    prompt_id: u64,
    /// `cef_permission_prompt_callback_t` or, for getUserMedia,
    /// `cef_media_access_callback_t`; `media_mask` tells them apart by being
    /// non-zero on the media route.
    callback: usize,
    media_mask: u32,
    /// The bits CEF asked for, whichever route they came in on. Recorded
    /// against the origin when the app answers, so `resetPermissions` clears
    /// only what Chromium has actually prompted for.
    request_mask: u32,
    /// The media route has no dismissal callback to clear the answer from, so
    /// it carries the context and the mask it has to clear itself. One owned
    /// reference, released with the request.
    context: ?*c.cef_request_context_t,
};

var pending_permission_requests: std.StringHashMapUnmanaged(*PermissionRequest) = .empty;
var permission_seq: u64 = 0;

/// What each origin has been asked for, so `resetPermissions` has something to
/// clear: CEF 151 can remove one origin's setting, but has no clear-all for a
/// content type. Recording the bits rather than sweeping the whole table is
/// also what keeps the sweep safe, see `clearPermissionSettings`.
const PermissionMasks = struct { prompt: u32 = 0, media: u32 = 0 };
var answered_permission_origins: std.StringHashMapUnmanaged(PermissionMasks) = .empty;

/// The permission names the app sees. Chromium's own enum is a bitmask, so a
/// request carrying two of them reports both.
const permission_names = [_]struct { bit: u32, name: []const u8 }{
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_AR_SESSION), .name = "arSession" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_CAMERA_PAN_TILT_ZOOM), .name = "cameraPanTiltZoom" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_CAMERA_STREAM), .name = "camera" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_CAPTURED_SURFACE_CONTROL), .name = "capturedSurfaceControl" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_CLIPBOARD), .name = "clipboard" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_TOP_LEVEL_STORAGE_ACCESS), .name = "topLevelStorageAccess" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_DISK_QUOTA), .name = "diskQuota" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_LOCAL_FONTS), .name = "localFonts" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_GEOLOCATION), .name = "geolocation" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_HAND_TRACKING), .name = "handTracking" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_IDENTITY_PROVIDER), .name = "identityProvider" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_IDLE_DETECTION), .name = "idleDetection" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_MIC_STREAM), .name = "microphone" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_MIDI_SYSEX), .name = "midiSysex" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_MULTIPLE_DOWNLOADS), .name = "multipleDownloads" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_NOTIFICATIONS), .name = "notifications" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_KEYBOARD_LOCK), .name = "keyboardLock" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_POINTER_LOCK), .name = "pointerLock" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_PROTECTED_MEDIA_IDENTIFIER), .name = "protectedMediaIdentifier" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_REGISTER_PROTOCOL_HANDLER), .name = "registerProtocolHandler" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_STORAGE_ACCESS), .name = "storageAccess" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_VR_SESSION), .name = "vrSession" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_WEB_APP_INSTALLATION), .name = "webAppInstallation" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_WINDOW_MANAGEMENT), .name = "windowManagement" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_FILE_SYSTEM_ACCESS), .name = "fileSystemAccess" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_LOCAL_NETWORK_ACCESS_DEPRECATED), .name = "localNetworkAccess" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_LOCAL_NETWORK), .name = "localNetwork" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_LOOPBACK_NETWORK), .name = "loopbackNetwork" },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_SENSORS), .name = "sensors" },
};

const media_permission_names = [_]struct { bit: u32, name: []const u8 }{
    .{ .bit = @intCast(c.CEF_MEDIA_PERMISSION_DEVICE_AUDIO_CAPTURE), .name = "microphone" },
    .{ .bit = @intCast(c.CEF_MEDIA_PERMISSION_DEVICE_VIDEO_CAPTURE), .name = "camera" },
    .{ .bit = @intCast(c.CEF_MEDIA_PERMISSION_DESKTOP_AUDIO_CAPTURE), .name = "desktopAudio" },
    .{ .bit = @intCast(c.CEF_MEDIA_PERMISSION_DESKTOP_VIDEO_CAPTURE), .name = "desktopVideo" },
};

/// The content settings an answered permission can be written into, so it can
/// be written back out. A permission type CEF 151 names no content setting for
/// is left alone rather than guessed at; geolocation has two because Chromium
/// splits the precise/approximate choice into its own setting.
const PermissionSettings = struct { bit: u32, settings: []const c_uint };

const permission_content_settings = [_]PermissionSettings{
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_AR_SESSION), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_AR} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_CAMERA_PAN_TILT_ZOOM), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_CAMERA_PAN_TILT_ZOOM} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_CAMERA_STREAM), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_MEDIASTREAM_CAMERA} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_CAPTURED_SURFACE_CONTROL), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_CAPTURED_SURFACE_CONTROL} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_CLIPBOARD), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_CLIPBOARD_READ_WRITE} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_TOP_LEVEL_STORAGE_ACCESS), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_TOP_LEVEL_STORAGE_ACCESS} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_DISK_QUOTA), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_PERSISTENT_STORAGE} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_LOCAL_FONTS), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_LOCAL_FONTS} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_GEOLOCATION), .settings = &.{ c.CEF_CONTENT_SETTING_TYPE_GEOLOCATION, c.CEF_CONTENT_SETTING_TYPE_GEOLOCATION_WITH_OPTIONS } },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_HAND_TRACKING), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_HAND_TRACKING} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_IDLE_DETECTION), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_IDLE_DETECTION} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_MIC_STREAM), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_MEDIASTREAM_MIC} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_MIDI_SYSEX), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_MIDI_SYSEX} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_MULTIPLE_DOWNLOADS), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_AUTOMATIC_DOWNLOADS} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_NOTIFICATIONS), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_NOTIFICATIONS} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_KEYBOARD_LOCK), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_KEYBOARD_LOCK} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_POINTER_LOCK), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_POINTER_LOCK} },
    // CEF_CONTENT_SETTING_TYPE_PROTECTED_MEDIA_IDENTIFIER is deliberately
    // absent: desktop Chromium does not register it, and asking for its pattern
    // scope aborts the browser process. Measured on 151.3.23 against a Linux
    // build, which trapped in HostContentSettingsMap on exactly that type.
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_REGISTER_PROTOCOL_HANDLER), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_PROTOCOL_HANDLERS} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_STORAGE_ACCESS), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_STORAGE_ACCESS} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_VR_SESSION), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_VR} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_WEB_APP_INSTALLATION), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_WEB_APP_INSTALLATION} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_WINDOW_MANAGEMENT), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_WINDOW_MANAGEMENT} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_LOCAL_NETWORK_ACCESS_DEPRECATED), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_LOCAL_NETWORK_ACCESS} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_LOCAL_NETWORK), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_LOCAL_NETWORK} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_LOOPBACK_NETWORK), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_LOOPBACK_NETWORK} },
    .{ .bit = @intCast(c.CEF_PERMISSION_TYPE_SENSORS), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_SENSORS} },
};

const media_content_settings = [_]PermissionSettings{
    .{ .bit = @intCast(c.CEF_MEDIA_PERMISSION_DEVICE_AUDIO_CAPTURE), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_MEDIASTREAM_MIC} },
    .{ .bit = @intCast(c.CEF_MEDIA_PERMISSION_DEVICE_VIDEO_CAPTURE), .settings = &.{c.CEF_CONTENT_SETTING_TYPE_MEDIASTREAM_CAMERA} },
};

fn permissionTypeList(mask: u32, media: bool) []const u8 {
    var out: std.ArrayList(u8) = .empty;
    if (media) {
        for (media_permission_names) |entry| {
            if (mask & entry.bit == 0) continue;
            if (out.items.len > 0) out.append(alloc, ',') catch {};
            out.appendSlice(alloc, entry.name) catch {};
        }
    } else {
        for (permission_names) |entry| {
            if (mask & entry.bit == 0) continue;
            if (out.items.len > 0) out.append(alloc, ',') catch {};
            out.appendSlice(alloc, entry.name) catch {};
        }
    }
    return out.toOwnedSlice(alloc) catch &.{};
}

/// The bits a list of permission names stands for, so `resetPermissions` takes
/// the same vocabulary `permissionRequest` reports. An unknown name is ignored.
fn permissionTypeMask(names: []const []const u8, media: bool) u32 {
    var mask: u32 = 0;
    for (names) |name| {
        if (media) {
            for (media_permission_names) |entry| {
                if (std.mem.eql(u8, entry.name, name)) mask |= entry.bit;
            }
        } else {
            for (permission_names) |entry| {
                if (std.mem.eql(u8, entry.name, name)) mask |= entry.bit;
            }
        }
    }
    return mask;
}

/// `URL.origin` form: scheme, host and a non-default port, with no trailing
/// slash. CEF serializes an origin as a GURL, which always ends in one, and an
/// app comparing it against `location.origin` would never match.
fn originForm(raw: []const u8) []const u8 {
    const sep = std.mem.indexOf(u8, raw, "://") orelse {
        const trimmed = std.mem.trimEnd(u8, raw, "/");
        return alloc.dupe(u8, trimmed) catch &.{};
    };
    const scheme = raw[0..sep];
    const rest = raw[sep + 3 ..];
    const end = std.mem.indexOfScalar(u8, rest, '/') orelse rest.len;
    var authority = rest[0..end];
    const default_port: ?[]const u8 = if (std.mem.eql(u8, scheme, "http"))
        ":80"
    else if (std.mem.eql(u8, scheme, "https") or std.mem.eql(u8, scheme, "wss"))
        ":443"
    else
        null;
    if (default_port) |port| {
        if (std.mem.endsWith(u8, authority, port)) authority = authority[0 .. authority.len - port.len];
    }
    return std.fmt.allocPrint(alloc, "{s}://{s}", .{ scheme, authority }) catch &.{};
}

/// CEF UI thread: removes the settings an answer may have written, so the same
/// origin asks again and the app's own store stays the only record of the
/// decision.
///
/// `set_website_setting` with a null value, not `set_content_setting` with
/// CEF_CONTENT_SETTING_VALUE_DEFAULT: the latter reaches
/// `HostContentSettingsMap::SetContentSettingDefaultScope`, which aborts the
/// browser process for a type Chromium keeps as a website setting rather than a
/// content setting (geolocation's precise/approximate choice is one). The
/// website-setting entry point serves both kinds and removes the rule either
/// way. Measured against 151.3.23: the content-setting call trapped in
/// `SetContentSettingDefaultScope` on the first answered geolocation prompt.
///
/// Both URLs are passed, never a null top level: CEF derives the rule's pattern
/// pair from them, and a type scoped to the requesting origin alone ignores the
/// second one.
///
/// `mask` only ever carries bits Chromium itself raised a prompt for. That is
/// the safety property this depends on: a type Chromium has not registered a
/// pattern scope for aborts the browser process, and a type it just prompted
/// for necessarily has one.
fn clearPermissionSettings(ctx: *c.cef_request_context_t, origin: []const u8, mask: u32, media: bool) void {
    if (origin.len == 0) return;
    const set = ctx.set_website_setting orelse return;
    var url = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&url);
    if (!setStr(&url, origin)) return;
    const table: []const PermissionSettings = if (media) &media_content_settings else &permission_content_settings;
    for (table) |entry| {
        if (mask & entry.bit == 0) continue;
        for (entry.settings) |setting| {
            set(ctx, &url, &url, @intCast(setting), null);
        }
    }
    tr("permissionSettingsCleared origin={s} mask={x} media={}", .{ origin, mask, media });
}

/// What an outstanding prompt asked for. Read back in
/// `on_dismiss_permission_prompt`. Touched on the CEF UI thread only, where
/// both permission callbacks run.
const PromptRecord = struct { origin: []const u8, mask: u32 };
var prompt_records: std.AutoHashMapUnmanaged(u64, PromptRecord) = .empty;

const ClearSettingsCall = struct { ctx: *c.cef_request_context_t, origin: []u8, mask: u32, media: bool };
const ClearSettingsObj = ref.Counted(c.cef_task_t, ClearSettingsCall);

/// Always a task, never an inline call, even when this is already the CEF UI
/// thread: `on_dismiss_permission_prompt` runs while Chromium is still
/// finishing the decision, and a clear made there is overwritten by the content
/// setting the decision then writes. A task runs on the next UI turn, after
/// that write. Measured on 151.3.23: clearing inline from the dismissal left
/// the origin blocked and it never asked again.
fn postClearPermissionSettings(ctx: *c.cef_request_context_t, origin: []const u8, mask: u32, media: bool) void {
    if (mask == 0 or origin.len == 0) return;
    const api = loader.loaded() orelse return;
    const copy = alloc.dupe(u8, origin) catch return;
    const task = ClearSettingsObj.create(.{ .ctx = ctx, .origin = copy, .mask = mask, .media = media }) orelse {
        alloc.free(copy);
        return;
    };
    task.cef.execute = &runClearSettingsTask;
    ref.addRefParam(ctx);
    if (api.post_task(c.TID_UI, task.handOut()) == 0) {
        ref.releaseParam(ctx);
        alloc.free(copy);
        task.drop();
    }
    task.drop();
}

fn runClearSettingsTask(self: [*c]c.cef_task_t) callconv(.c) void {
    const call = ClearSettingsObj.of(self).payload;
    clearPermissionSettings(call.ctx, call.origin, call.mask, call.media);
    ref.releaseParam(call.ctx);
    alloc.free(call.origin);
}

fn postPermissionRequest(
    view: *View,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    prompt_id: u64,
    origin: [*c]const c.cef_string_t,
    mask: u32,
    callback: usize,
    media: bool,
) void {
    const req = alloc.create(PermissionRequest) catch return;
    const raw_origin = dupeStr(origin) orelse (alloc.dupe(u8, "") catch &.{});
    defer alloc.free(raw_origin);
    var frame_url: ?[]const u8 = null;
    var is_main = false;
    if (frame != null) {
        if (frame.*.is_main) |f| is_main = f(frame) != 0;
        if (frame.*.get_url) |get_url| {
            const raw = get_url(frame);
            if (raw != null) {
                defer freeUserfree(raw);
                frame_url = dupeStr(raw);
            }
        }
    }
    req.* = .{
        .id = &.{},
        .origin = originForm(raw_origin),
        .types = permissionTypeList(mask, media),
        .frame_url = frame_url,
        .is_main_frame = is_main,
        .main_frame_url = mainFrameUrl(browser) orelse (alloc.dupe(u8, "") catch &.{}),
        .prompt_id = prompt_id,
        .callback = callback,
        .media_mask = if (media) mask else 0,
        .request_mask = mask,
        .context = if (media) browserContext(browser) else null,
    };
    if (!media) rememberPrompt(prompt_id, req.origin, mask);
    post(.{ .view = view, .name = "permissionRequest", .permission = req });
}

/// One owned reference to the browser's own request context, which is where its
/// content settings live whether it was created on a named profile or the
/// global one.
fn browserContext(browser: [*c]c.cef_browser_t) ?*c.cef_request_context_t {
    if (browser == null) return null;
    const get_host = browser.*.get_host orelse return null;
    const host = get_host(browser);
    if (host == null) return null;
    defer ref.releaseOwned(host);
    const get_ctx = host.*.get_request_context orelse return null;
    const ctx = get_ctx(host);
    if (ctx == null) return null;
    return @ptrCast(ctx);
}

fn rememberPrompt(prompt_id: u64, origin: []const u8, mask: u32) void {
    const copy = alloc.dupe(u8, origin) catch return;
    prompt_records.put(alloc, prompt_id, .{ .origin = copy, .mask = mask }) catch alloc.free(copy);
}

/// GTK thread: parks the request and raises the event the app answers with
/// `respondPermission`.
fn announcePermissionRequest(view: *View, req: *PermissionRequest) void {
    permission_seq += 1;
    req.id = std.fmt.allocPrint(alloc, "cefpermission-{d}", .{permission_seq}) catch {
        answerPermissionRequest(req, .deny);
        return;
    };
    pending_permission_requests.put(alloc, req.id, req) catch {
        answerPermissionRequest(req, .deny);
        return;
    };
    tr("permissionRequest node={d} id={s} origin={s} types={s} frame={?s} main={}", .{ view.node_id, req.id, req.origin, req.types, req.frame_url, req.is_main_frame });

    const f = emit orelse return;
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    payload.put(alloc, "id", .{ .string = req.id }) catch return;
    payload.put(alloc, "origin", .{ .string = req.origin }) catch return;
    payload.put(alloc, "types", .{ .string = req.types }) catch return;
    payload.put(alloc, "mainFrameUrl", .{ .string = req.main_frame_url }) catch return;
    if (req.frame_url) |url| {
        payload.put(alloc, "frameUrl", .{ .string = url }) catch return;
        payload.put(alloc, "isMainFrame", .{ .bool = req.is_main_frame }) catch return;
    }
    f(view.node_id, "permissionRequest", .{ .data = .{ .object = payload } });
}

/// Answers CEF and frees the request. Safe to call with a callback that is
/// already spent: `on_dismiss_permission_prompt` clears it first.
fn answerPermissionRequest(req: *PermissionRequest, result: PermissionResult) void {
    defer {
        if (req.id.len > 0) alloc.free(req.id);
        if (req.frame_url) |url| alloc.free(url);
        alloc.free(req.main_frame_url);
        alloc.free(req.origin);
        alloc.free(req.types);
        if (req.context) |ctx| ref.releaseParam(ctx);
        alloc.destroy(req);
    }
    if (req.callback == 0) return;
    if (req.media_mask != 0) {
        const cb: [*c]c.cef_media_access_callback_t = @ptrFromInt(req.callback);
        defer ref.releaseParam(cb);
        // The mask has to come back exactly as it went out, or CEF rejects it.
        // The media callback has no dismiss of its own: cancelling is the only
        // way to end the request without granting it.
        if (result == .allow) {
            if (cb.*.cont) |cont| cont(cb, req.media_mask);
        } else if (cb.*.cancel) |cancel| {
            cancel(cb);
        }
        if (result != .dismiss) {
            if (req.context) |ctx| {
                postClearPermissionSettings(ctx, req.origin, req.media_mask, true);
                rememberAnsweredOrigin(req.origin, .{ .media = req.media_mask });
            }
        }
        return;
    }
    const cb: [*c]c.cef_permission_prompt_callback_t = @ptrFromInt(req.callback);
    defer ref.releaseParam(cb);
    if (cb.*.cont) |cont| {
        cont(cb, @intCast(switch (result) {
            .allow => c.CEF_PERMISSION_RESULT_ACCEPT,
            .deny => c.CEF_PERMISSION_RESULT_DENY,
            .dismiss => c.CEF_PERMISSION_RESULT_DISMISS,
        }));
    }
    if (result != .dismiss) rememberAnsweredOrigin(req.origin, .{ .prompt = req.request_mask });
}

fn rememberAnsweredOrigin(origin: []const u8, add: PermissionMasks) void {
    if (origin.len == 0) return;
    if (answered_permission_origins.getPtr(origin)) |seen| {
        seen.prompt |= add.prompt;
        seen.media |= add.media;
        return;
    }
    const copy = alloc.dupe(u8, origin) catch return;
    answered_permission_origins.put(alloc, copy, add) catch alloc.free(copy);
}

fn onShowPermissionPrompt(
    self: [*c]c.cef_permission_handler_t,
    browser: [*c]c.cef_browser_t,
    prompt_id: u64,
    requesting_origin: [*c]const c.cef_string_t,
    requested_permissions: u32,
    callback: [*c]c.cef_permission_prompt_callback_t,
) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    const view = PermissionObj.of(self).payload;
    postPermissionRequest(view, browser, null, prompt_id, requesting_origin, requested_permissions, @intFromPtr(callback), false);
    return 1;
}

fn onRequestMediaAccessPermission(
    self: [*c]c.cef_permission_handler_t,
    browser: [*c]c.cef_browser_t,
    frame: [*c]c.cef_frame_t,
    requesting_origin: [*c]const c.cef_string_t,
    requested_permissions: u32,
    callback: [*c]c.cef_media_access_callback_t,
) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    defer ref.releaseParam(frame);
    const view = PermissionObj.of(self).payload;
    postPermissionRequest(view, browser, frame, 0, requesting_origin, requested_permissions, @intFromPtr(callback), true);
    return 1;
}

/// CEF is done with the prompt: either the app answered it, or Chromium retired
/// it on its own (a navigation, a closed browser). Chromium has written its
/// content setting by now, so this is where it is written back out; the request
/// still being parked means nobody answered, and the app is told the id is dead.
fn onDismissPermissionPrompt(
    self: [*c]c.cef_permission_handler_t,
    browser: [*c]c.cef_browser_t,
    prompt_id: u64,
    result: c.cef_permission_request_result_t,
) callconv(.c) void {
    defer ref.releaseParam(browser);
    const view = PermissionObj.of(self).payload;
    if (prompt_records.fetchRemove(prompt_id)) |entry| {
        defer alloc.free(entry.value.origin);
        // A dismissal records no content setting, so there is nothing to undo.
        if (result != c.CEF_PERMISSION_RESULT_DISMISS) {
            if (browserContext(browser)) |ctx| {
                defer ref.releaseParam(ctx);
                postClearPermissionSettings(ctx, entry.value.origin, entry.value.mask, false);
            }
        }
    }
    post(.{ .view = view, .name = "permissionDismissed", .permission_dismissed = prompt_id });
}

/// GTK thread: forgets a prompt CEF has finished with. The callback is spent,
/// so it is released without being called. A request still parked here is one
/// the app never answered, so it also hears that the id is dead.
fn dropPermissionRequest(view: *View, prompt_id: u64) void {
    var it = pending_permission_requests.iterator();
    while (it.next()) |entry| {
        const req = entry.value_ptr.*;
        if (req.prompt_id == 0 or req.prompt_id != prompt_id) continue;
        _ = pending_permission_requests.remove(entry.key_ptr.*);
        const cb: [*c]c.cef_permission_prompt_callback_t = @ptrFromInt(req.callback);
        req.callback = 0;
        ref.releaseParam(cb);
        if (live_views.contains(@intFromPtr(view))) {
            tr("permissionRequestDismissed node={d} id={s}", .{ view.node_id, req.id });
            if (emit) |f| {
                var payload: std.json.ObjectMap = .empty;
                defer payload.deinit(alloc);
                payload.put(alloc, "id", .{ .string = req.id }) catch {};
                f(view.node_id, "permissionRequestDismissed", .{ .data = .{ .object = payload } });
            }
        }
        answerPermissionRequest(req, .dismiss);
        return;
    }
}

/// An id this engine handed out but no longer has parked: the app answered it
/// already, or Chromium retired it and the app's answer was in flight. Neither
/// is a mistake worth a warning; an id that was never handed out is.
fn permissionIdWasIssued(id: []const u8) bool {
    const prefix = "cefpermission-";
    if (!std.mem.startsWith(u8, id, prefix)) return false;
    const n = std.fmt.parseInt(u64, id[prefix.len..], 10) catch return false;
    return n >= 1 and n <= permission_seq;
}

fn cmdRespondPermission(arg: ?std.json.Value) void {
    const obj_arg = argObject(arg) orelse return;
    const id = objStr(obj_arg, "id") orelse {
        std.debug.print("ND_WARN WebView respondPermission: missing id\n", .{});
        return;
    };
    const entry = pending_permission_requests.fetchRemove(id) orelse {
        if (permissionIdWasIssued(id)) return;
        std.debug.print("ND_WARN WebView respondPermission: unknown request id {s}\n", .{id});
        return;
    };
    const result: PermissionResult = blk: {
        if (objStr(obj_arg, "result")) |name| {
            if (std.mem.eql(u8, name, "allow")) break :blk .allow;
            if (std.mem.eql(u8, name, "deny")) break :blk .deny;
            if (std.mem.eql(u8, name, "dismiss")) break :blk .dismiss;
            std.debug.print("ND_WARN WebView respondPermission: unknown result {s}\n", .{name});
        }
        break :blk if (objBool(obj_arg, "allow") orelse false) .allow else .deny;
    };
    answerPermissionRequest(entry.value, result);
}

/// `resetPermissions`: removes Chromium's stored decisions, so a site the app
/// blocked earlier asks again. It clears what this process has recorded being
/// asked, per origin, narrowed by `origin` and `types` when they are given.
/// Nothing wider is possible or safe: CEF 151 has no clear-all for a content
/// type, and a type Chromium has not registered aborts the browser process, so
/// only bits Chromium itself raised a prompt for are ever handed back to it.
fn cmdResetPermissions(view: *View, arg: ?std.json.Value) void {
    const obj_arg = argObject(arg);
    var filter: u32 = std.math.maxInt(u32);
    var media_filter: u32 = std.math.maxInt(u32);
    if (obj_arg) |obj| {
        if (objStrList(obj, "types")) |list| {
            var names: std.ArrayList([]const u8) = .empty;
            defer names.deinit(alloc);
            for (list.items) |item| {
                if (item == .string) names.append(alloc, item.string) catch {};
            }
            filter = permissionTypeMask(names.items, false);
            media_filter = permissionTypeMask(names.items, true);
        }
    }
    const wanted: ?[]const u8 = if (obj_arg) |obj| objStr(obj, "origin") else null;
    const form: ?[]const u8 = if (wanted) |origin| originForm(origin) else null;
    defer if (form) |f| alloc.free(f);

    const host = hostOf(view) orelse return;
    const get_ctx = host.get_request_context orelse return;
    const raw = get_ctx(host);
    if (raw == null) return;
    const ctx: *c.cef_request_context_t = @ptrCast(raw);
    defer ref.releaseOwned(ctx);

    var cleared: usize = 0;
    var it = answered_permission_origins.iterator();
    while (it.next()) |entry| {
        const origin = entry.key_ptr.*;
        if (form) |f| {
            if (!std.mem.eql(u8, f, origin)) continue;
        }
        postClearPermissionSettings(ctx, origin, entry.value_ptr.prompt & filter, false);
        postClearPermissionSettings(ctx, origin, entry.value_ptr.media & media_filter, true);
        cleared += 1;
    }
    tr("resetPermissions node={d} origins={d} of={d}", .{ view.node_id, cleared, answered_permission_origins.count() });
}

/// Chrome's "Always allow pop-ups and redirects from" this site: the popups
/// content setting for `origin`, or for the page on show when none is named.
fn cmdAllowPopups(view: *View, arg: ?std.json.Value) void {
    const api = loader.loaded() orelse return;
    const named: ?[]const u8 = if (argObject(arg)) |obj| objStr(obj, "origin") else null;
    const site = named orelse view.url orelse return;
    const origin = originForm(site);
    if (origin.len == 0) return;
    const host = hostOf(view) orelse {
        alloc.free(origin);
        return;
    };
    const get_ctx = host.get_request_context orelse {
        alloc.free(origin);
        return;
    };
    const raw = get_ctx(host);
    if (raw == null) {
        alloc.free(origin);
        return;
    }
    const ctx: *c.cef_request_context_t = @ptrCast(raw);
    const task = AllowPopupsObj.create(.{ .ctx = ctx, .origin = origin }) orelse {
        ref.releaseOwned(ctx);
        alloc.free(origin);
        return;
    };
    task.cef.execute = &runAllowPopupsTask;
    if (api.post_task(c.TID_UI, task.handOut()) == 0) {
        ref.releaseOwned(ctx);
        alloc.free(origin);
    }
    task.drop();
    tr("allowPopups node={d}", .{view.node_id});
}

const AllowPopupsCall = struct { ctx: *c.cef_request_context_t, origin: []const u8 };
const AllowPopupsObj = ref.Counted(c.cef_task_t, AllowPopupsCall);

fn runAllowPopupsTask(self: [*c]c.cef_task_t) callconv(.c) void {
    const call = AllowPopupsObj.of(self).payload;
    defer ref.releaseOwned(call.ctx);
    defer alloc.free(call.origin);
    const set = call.ctx.set_content_setting orelse return;
    var url = std.mem.zeroes(c.cef_string_t);
    defer clearStr(&url);
    if (!setStr(&url, call.origin)) return;
    set(call.ctx, &url, &url, c.CEF_CONTENT_SETTING_TYPE_POPUPS, c.CEF_CONTENT_SETTING_VALUE_ALLOW);
    tr("popupsAllowed origin={s}", .{call.origin});
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
const CEF_SCHEME_OPTION_CSP_BYPASSING: c_int = 1 << 5;

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
    // Content blocking's own scheme (src/adblock.zig `serve`), in every
    // process and for every app: the renderers fetch from it from inside
    // pages whose CSP would refuse anything else.
    addCustomScheme(registrar, add, adblock.scheme, CEF_SCHEME_OPTION_STANDARD | CEF_SCHEME_OPTION_SECURE |
        CEF_SCHEME_OPTION_CORS_ENABLED | CEF_SCHEME_OPTION_CSP_BYPASSING);
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
    /// How many times the GTK side has looked for this request's view. The
    /// browser-id hop and the request itself are posted from DIFFERENT threads
    /// (the CEF UI thread and the IO thread), so the request can arrive first
    /// and the view is simply not known yet.
    attempts: u32 = 0,
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
/// How long a scheme request waits for its browser to be known before it is
/// answered with a failure. The first request a browser makes is often its own
/// initial navigation, which is exactly when the mapping is still in flight:
/// an extension page served over a custom scheme is the whole of its own
/// startup, so failing it early is failing the extension.
const scheme_view_attempts: u32 = 80;

fn announceSchemeRequest(obj: *ResourceObj) void {
    const res = obj.payload;
    const view = browsers_by_id.get(res.browser_id);
    if (view == null) {
        res.attempts += 1;
        if (res.attempts <= scheme_view_attempts) {
            // Re-posted rather than failed: the browser-id hop is already on
            // the GTK loop behind this one.
            _ = glib.timeoutAdd(25, &onSchemeRetry, obj);
            return;
        }
        tr("announceSchemeRequest browser={d} view=none url={s}", .{ res.browser_id, res.url });
        scheme_seq += 1;
        res.id = std.fmt.allocPrint(alloc, "cefscheme-{d}", .{scheme_seq}) catch return;
        failSchemeRequest(obj, "no view for this browser");
        return;
    }
    tr("announceSchemeRequest browser={d} view={d} url={s}", .{ res.browser_id, view.?.node_id, res.url });
    scheme_seq += 1;
    const id = std.fmt.allocPrint(alloc, "cefscheme-{d}", .{scheme_seq}) catch return;
    res.id = id;
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

fn onSchemeRetry(data: ?*anyopaque) callconv(.c) c_int {
    announceSchemeRequest(@ptrCast(@alignCast(data.?)));
    return 0;
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

fn clientGetFocusHandler(self: [*c]c.cef_client_t) callconv(.c) [*c]c.cef_focus_handler_t {
    return ClientObj.of(self).payload.focus_handler.handOut();
}

/// Chromium asks for real X input focus on every navigation. Granted, the GTK
/// toplevel stops receiving key events and every accelerator in the app goes
/// dead until the user clicks native chrome again; on an XWayland session a
/// grab Chromium fails to release can make that permanent. Refusing
/// navigation-sourced focus is what Chrome's own browser UI does: the page
/// only gets the keyboard when the user gives it (FOCUS_SOURCE_SYSTEM).
fn onSetFocus(self: [*c]c.cef_focus_handler_t, browser: [*c]c.cef_browser_t, source: c.cef_focus_source_t) callconv(.c) c_int {
    defer ref.releaseParam(browser);
    const view = FocusObj.of(self).payload;
    focused_view = view;
    // A view too small to click into is refused whatever the source. An app
    // that keeps a functional-but-invisible browser (the hidden view Chromium
    // makes an extension registry readable from) had two browsers taking
    // focus from each other tens of thousands of times in a single run: every
    // grant moved X input focus, the browser that lost it asked again, and
    // the loop pinned GTK's focus widget on a webview, undid whatever the app
    // had just focused, and buried the GTK idle queue deep enough that
    // automation stopped answering.
    if (!focusEligible(view)) return 1;
    return @intFromBool(source == c.FOCUS_SOURCE_NAVIGATION);
}

/// A click inside the page never reaches GTK: the browser's X window swallows
/// it. This is the only notice that the user moved the keyboard from the app's
/// native chrome to the page, so it is what moves GTK's focus widget to match.
fn onGotFocus(self: [*c]c.cef_focus_handler_t, browser: [*c]c.cef_browser_t) callconv(.c) void {
    defer ref.releaseParam(browser);
    const view = FocusObj.of(self).payload;
    if (!focusEligible(view)) return;
    const asked = glib.getMonotonicTime() - view.focus_set_us.load(.acquire) < own_focus_window_us;
    post(.{ .view = view, .name = "", .grab_focus = true, .flag = !asked });
}

/// How long after this engine's own `set_focus` an `on_got_focus` is taken to
/// be its answer rather than a click.
const own_focus_window_us: i64 = 500 * std.time.us_per_ms;

fn onTakeFocus(self: [*c]c.cef_focus_handler_t, browser: [*c]c.cef_browser_t, next: c_int) callconv(.c) void {
    defer ref.releaseParam(browser);
    post(.{ .view = FocusObj.of(self).payload, .name = "", .take_focus = true, .flag = next != 0 });
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

/// One trace line per item of the model the engine was handed, submenus
/// included. Chrome style's model is Chrome's own, extension items and
/// spell-check suggestions among them, and the only way to see what an app
/// would have to render is to read it here.
fn traceMenuModel(model: [*c]c.cef_menu_model_t, depth: u32) void {
    if (model == null or depth > 4) return;
    const get_count = model.*.get_count orelse return;
    const count = get_count(model);
    var i: usize = 0;
    while (i < count) : (i += 1) {
        var label: []u8 = &.{};
        defer alloc.free(label);
        if (model.*.get_label_at) |get_label| {
            const raw = get_label(model, i);
            if (raw != null) {
                defer freeUserfree(raw);
                label = dupeStr(raw) orelse &.{};
            }
        }
        const kind: c_int = if (model.*.get_type_at) |f| @intCast(f(model, i)) else -1;
        const command_id: c_int = if (model.*.get_command_id_at) |f| f(model, i) else -1;
        const enabled: c_int = if (model.*.is_enabled_at) |f| f(model, i) else -1;
        const checked: c_int = if (model.*.is_checked_at) |f| f(model, i) else -1;
        tr("menuItem depth={d} index={d} id={d} type={d} enabled={d} checked={d} label={s}", .{
            depth, i, command_id, kind, enabled, checked, label,
        });
        if (model.*.get_sub_menu_at) |get_sub| {
            const sub = get_sub(model, i);
            if (sub != null) {
                defer ref.releaseOwned(sub);
                traceMenuModel(sub, depth + 1);
            }
        }
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
    const view = ContextMenuObj.of(self).payload;

    // Emitted in both modes, exactly as the WebKitGTK backend does: an app that
    // wants to decorate the native menu and an app that wants to replace it
    // both need to know where the click landed.
    var hit = readMenuHit(params);
    const at_x = hit.x;
    const at_y = hit.y;
    const boxed = alloc.create(MenuHit) catch {
        hit.deinit();
        ref.releaseParam(callback);
        return 0;
    };
    boxed.* = hit;
    post(.{ .view = view, .name = "contextMenu", .menu_hit = boxed });

    traceMenuModel(model, 0);
    if (view.suppress_menu.load(.acquire)) {
        if (callback != null) {
            if (callback.*.cancel) |cancel| cancel(callback);
        }
        ref.releaseParam(callback);
        return 1;
    }
    if (callback == null) return 0;

    // Chromium's model is copied here because it may not be referenced once
    // this returns, and the menu it becomes is drawn on the GTK thread.
    const items = copyMenuModel(model, 0);
    if (items.len == 0) {
        gtkmenu.freeItems(items);
        ref.releaseParam(callback);
        return 0;
    }
    const req = alloc.create(MenuRequest) catch {
        gtkmenu.freeItems(items);
        ref.releaseParam(callback);
        return 0;
    };
    // The callback parameter's reference is deliberately kept: it is what
    // `answerMenu` consumes when the user picks or dismisses.
    req.* = .{ .callback = callback, .items = items, .x = at_x, .y = at_y };
    post(.{ .view = view, .name = "", .menu_request = req });
    return 1;
}

/// One native menu in flight, from `run_context_menu` to the pick or the
/// dismissal that answers it.
const MenuRequest = struct {
    callback: [*c]c.cef_run_context_menu_callback_t,
    items: []gtkmenu.Item,
    x: c_int,
    y: c_int,
};

fn cancelMenuRequest(req: *MenuRequest) void {
    answerMenu(req.callback, 0);
    gtkmenu.freeItems(req.items);
    alloc.destroy(req);
}

/// How long a pending menu waits for the pointer button to come up, and how
/// often it looks. The wait is bounded because a button the X server believes
/// is still down after the user has let go (a release swallowed by a grab that
/// went away with its client) would otherwise park the menu forever.
const menu_button_wait_us: i64 = 2 * std.time.us_per_s;
const menu_button_poll_ms: c_uint = 16;

fn openNativeMenu(view: *View, req: *MenuRequest) void {
    closeNativeMenu(view);
    // Chromium raises the context menu on the press, not the release, so this
    // routinely arrives with the right button still down. A GtkPopover grabs
    // the seat as it pops up, and while a button is held the X server holds an
    // automatic pointer grab for the client that owns the window the press
    // landed in, which here is Chromium's own X connection and not GDK's.
    // XGrabPointer answers AlreadyGrabbed, and GTK hides a popover whose grab
    // failed before it is ever mapped: no menu and no error. So the menu waits
    // out the rest of the user's click.
    if (x11.pointerButtonsDown()) {
        view.menu_pending = req;
        view.menu_open_deadline_us = glib.getMonotonicTime() + menu_button_wait_us;
        view.menu_open_source = glib.timeoutAdd(menu_button_poll_ms, &onMenuButtonWatch, view);
        return;
    }
    presentNativeMenu(view, req);
}

fn onMenuButtonWatch(data: ?*anyopaque) callconv(.c) c_int {
    const view: *View = @ptrCast(@alignCast(data.?));
    if (x11.pointerButtonsDown() and glib.getMonotonicTime() < view.menu_open_deadline_us) return 1; // G_SOURCE_CONTINUE
    view.menu_open_source = 0;
    const req = view.menu_pending orelse return 0;
    view.menu_pending = null;
    presentNativeMenu(view, req);
    return 0; // G_SOURCE_REMOVE
}

fn presentNativeMenu(view: *View, req: *MenuRequest) void {
    // The popover takes a keyboard grab, and X input focus is on CEF's own
    // window after any interaction with the page; without this the menu opens
    // with no keyboard.
    x11.focusToplevel(view.widget);
    const popup = gtkmenu.open(view.widget, req.items, req.x, req.y, &onNativeMenuAnswer, view) orelse {
        cancelMenuRequest(req);
        return;
    };
    view.menu_popup = popup;
    view.menu_request = req;
    traceRenderedMenu(req.items, 0);
    tr("menuOpen node={d} at={d},{d} items={d}", .{ view.node_id, req.x, req.y, req.items.len });
}

fn closeNativeMenu(view: *View) void {
    if (view.menu_open_source != 0) {
        _ = glib.Source.remove(view.menu_open_source);
        view.menu_open_source = 0;
    }
    if (view.menu_pending) |req| {
        view.menu_pending = null;
        cancelMenuRequest(req);
    }
    const popup = view.menu_popup orelse return;
    gtkmenu.close(popup);
}

fn onNativeMenuAnswer(ctx: ?*anyopaque, command_id: c_int) void {
    const view: *View = @ptrCast(@alignCast(ctx orelse return));
    view.menu_popup = null;
    const req = view.menu_request orelse return;
    view.menu_request = null;
    tr("menuAnswer node={d} command={d}", .{ view.node_id, command_id });
    answerMenu(req.callback, command_id);
    gtkmenu.freeItems(req.items);
    alloc.destroy(req);
}

const MenuAnswer = struct {
    callback: [*c]c.cef_run_context_menu_callback_t,
    command_id: c_int,
};
const MenuAnswerObj = ref.Counted(c.cef_task_t, MenuAnswer);

/// Answers the menu callback exactly once and gives back the reference
/// `run_context_menu` kept. `command_id` 0 is the dismissal.
fn answerMenu(callback: [*c]c.cef_run_context_menu_callback_t, command_id: c_int) void {
    if (callback == null) return;
    const api = loader.loaded() orelse return ref.releaseOwned(callback);
    if (api.currently_on(c.TID_UI) != 0) return finishMenu(callback, command_id);
    const task = MenuAnswerObj.create(.{ .callback = callback, .command_id = command_id }) orelse
        return ref.releaseOwned(callback);
    task.cef.execute = &runMenuAnswerTask;
    if (api.post_task(c.TID_UI, task.handOut()) == 0) {
        task.drop();
        task.drop();
        // CEF is on its way down; leaving the menu unanswered is nothing the
        // process will still be around to notice, and the reference is not.
        ref.releaseOwned(callback);
        return;
    }
    task.drop();
}

fn runMenuAnswerTask(self: [*c]c.cef_task_t) callconv(.c) void {
    const answer = MenuAnswerObj.of(self).payload;
    finishMenu(answer.callback, answer.command_id);
}

fn finishMenu(callback: [*c]c.cef_run_context_menu_callback_t, command_id: c_int) void {
    if (command_id == 0) {
        if (callback.*.cancel) |cancel| cancel(callback);
    } else if (callback.*.cont) |cont| {
        cont(callback, command_id, c.EVENTFLAG_NONE);
    }
    ref.releaseOwned(callback);
}

/// Items whose command the engine refuses are dropped rather than drawn: an
/// entry that does nothing when picked is worse than no entry. The open-link
/// commands stay, because `on_context_menu_command` reroutes them to the app's
/// `newWindow` before the deny list is consulted.
fn menuItemDenied(command_id: c_int) bool {
    if (command_id < 0) return false;
    if (!chromeStyle()) return false;
    if (openLinkCommand(command_id)) return false;
    return chromeCommandBlocked(command_id);
}

/// Chromium's model, copied into the owned tree the GTK side draws. Runs on the
/// CEF UI thread, where the model is only valid for the length of the callback.
fn copyMenuModel(model: [*c]c.cef_menu_model_t, depth: u32) []gtkmenu.Item {
    if (model == null or depth > 6) return &.{};
    const get_count = model.*.get_count orelse return &.{};
    const count = get_count(model);
    var out: std.ArrayList(gtkmenu.Item) = .empty;
    var i: usize = 0;
    while (i < count) : (i += 1) {
        if (model.*.is_visible_at) |visible| {
            if (visible(model, i) == 0) continue;
        }
        const raw_type: c.cef_menu_item_type_t = if (model.*.get_type_at) |f| f(model, i) else c.MENUITEMTYPE_COMMAND;
        if (raw_type == c.MENUITEMTYPE_SEPARATOR) {
            // Filtering leaves rules with nothing on one side of them, so one
            // is only kept when something drawable precedes it.
            if (out.items.len == 0 or out.items[out.items.len - 1].kind == .separator) continue;
            const rule = alloc.dupeZ(u8, "") catch break;
            out.append(alloc, .{ .label = rule, .kind = .separator }) catch {
                gtkmenu.freeLabel(rule);
                break;
            };
            continue;
        }
        const command_id: c_int = if (model.*.get_command_id_at) |f| f(model, i) else -1;
        if (menuItemDenied(command_id)) continue;

        var children: []gtkmenu.Item = &.{};
        if (raw_type == c.MENUITEMTYPE_SUBMENU) {
            const get_sub = model.*.get_sub_menu_at orelse continue;
            const sub = get_sub(model, i);
            if (sub == null) continue;
            defer ref.releaseOwned(sub);
            children = copyMenuModel(sub, depth + 1);
            // A submenu whose every child was denied is a dead label.
            if (!anyDrawable(children)) {
                gtkmenu.freeItems(children);
                continue;
            }
        }

        const label = menuLabelAt(model, i) orelse continue;
        const item: gtkmenu.Item = .{
            .label = label,
            .command_id = command_id,
            .kind = switch (raw_type) {
                c.MENUITEMTYPE_SUBMENU => .submenu,
                c.MENUITEMTYPE_CHECK => .check,
                c.MENUITEMTYPE_RADIO => .radio,
                else => .command,
            },
            .enabled = if (model.*.is_enabled_at) |f| f(model, i) != 0 else true,
            .checked = if (model.*.is_checked_at) |f| f(model, i) != 0 else false,
            .group_id = if (model.*.get_group_id_at) |f| f(model, i) else -1,
            .children = children,
            .accel = menuAccelAt(model, i),
        };
        out.append(alloc, item) catch {
            gtkmenu.freeItems(item.children);
            gtkmenu.freeLabel(item.label);
            if (item.accel) |a| gtkmenu.freeLabel(a);
            break;
        };
    }
    while (out.items.len > 0 and out.items[out.items.len - 1].kind == .separator) {
        if (out.pop()) |last| gtkmenu.freeLabel(last.label);
    }
    return out.toOwnedSlice(alloc) catch &.{};
}

fn anyDrawable(items: []const gtkmenu.Item) bool {
    for (items) |item| {
        if (item.kind != .separator) return true;
    }
    return false;
}

/// Chromium's labels carry Windows-style `&` mnemonics, which GTK4's menu
/// models do not draw at all. `&&` is the escaped ampersand.
fn menuLabelAt(model: [*c]c.cef_menu_model_t, index: usize) ?[:0]u8 {
    const get_label = model.*.get_label_at orelse return null;
    const raw = get_label(model, index);
    if (raw == null) return alloc.dupeZ(u8, "") catch null;
    defer freeUserfree(raw);
    const text = dupeStr(raw) orelse return null;
    defer alloc.free(text);
    var out: std.ArrayList(u8) = .empty;
    defer out.deinit(alloc);
    var i: usize = 0;
    while (i < text.len) : (i += 1) {
        if (text[i] == '&') {
            if (i + 1 < text.len and text[i + 1] == '&') i += 1 else continue;
        }
        out.append(alloc, text[i]) catch return null;
    }
    return alloc.dupeZ(u8, out.items) catch null;
}

/// The accelerator as gtk_accelerator_parse spells it, which is what a menu
/// model's "accel" attribute takes. Key codes Chromium reports are Windows
/// virtual keys; anything outside the set a context menu actually uses is
/// reported as no accelerator rather than guessed at.
fn menuAccelAt(model: [*c]c.cef_menu_model_t, index: usize) ?[:0]u8 {
    const has = model.*.has_accelerator_at orelse return null;
    if (has(model, index) == 0) return null;
    const get = model.*.get_accelerator_at orelse return null;
    var key_code: c_int = 0;
    var shift: c_int = 0;
    var ctrl: c_int = 0;
    var alt: c_int = 0;
    if (get(model, index, &key_code, &shift, &ctrl, &alt) == 0) return null;

    var key_buf: [8]u8 = undefined;
    const key: []const u8 = switch (key_code) {
        0x08 => "BackSpace",
        0x0D => "Return",
        0x1B => "Escape",
        0x20 => "space",
        0x25 => "Left",
        0x26 => "Up",
        0x27 => "Right",
        0x28 => "Down",
        0x2E => "Delete",
        0x30...0x39 => blk: {
            key_buf[0] = @intCast(key_code);
            break :blk key_buf[0..1];
        },
        0x41...0x5A => blk: {
            key_buf[0] = std.ascii.toLower(@intCast(key_code));
            break :blk key_buf[0..1];
        },
        0x70...0x7B => std.fmt.bufPrint(&key_buf, "F{d}", .{key_code - 0x6F}) catch return null,
        else => return null,
    };
    var out: [48]u8 = undefined;
    const text = std.fmt.bufPrintZ(&out, "{s}{s}{s}{s}", .{
        if (ctrl != 0) "<Control>" else "",
        if (alt != 0) "<Alt>" else "",
        if (shift != 0) "<Shift>" else "",
        key,
    }) catch return null;
    return alloc.dupeZ(u8, text) catch null;
}

/// One trace line per item of the menu the user is actually looking at, which
/// is what the gate asserts against: `menuItem` is the model the engine was
/// handed, this is what survived the filter.
fn traceRenderedMenu(items: []const gtkmenu.Item, depth: u32) void {
    for (items, 0..) |item, i| {
        const accel: []const u8 = if (item.accel) |a| a else "";
        tr("menuShown depth={d} index={d} id={d} kind={s} enabled={d} checked={d} accel={s} label={s}", .{
            depth,
            i,
            item.command_id,
            @tagName(item.kind),
            @intFromBool(item.enabled),
            @intFromBool(item.checked),
            accel,
            item.label,
        });
        if (item.children.len > 0) traceRenderedMenu(item.children, depth + 1);
    }
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

    // Chrome style's default menu carries the "open link in …" items, which
    // Chrome answers with a window of its own. They run before
    // cef_command_handler_t sees them and they are the only place the link URL
    // is still in hand, so the routing to `newWindow` happens here and the
    // command deny list is only the backstop.
    if (chromeStyle() and hit.link.len > 0) {
        if (openLinkIndex(command_id)) |which| {
            // Chrome opens "Open link in new tab" behind the page.
            postNewWindow(view, dupeOwned(hit.link), if (which == 0) "backgroundTab" else "window", true);
            return 1;
        }
    }

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
