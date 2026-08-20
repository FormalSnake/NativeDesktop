const std = @import("std");
const glib = @import("glib");
const gobject = @import("gobject");
const gio = @import("gio");
const gtk = @import("gtk");
const adw = @import("adw");
const abi = @import("../abi.zig");
const backend = @import("backend.zig");
const system = @import("system.zig");
const cef = @import("../cef/backend.zig");

// Overridable per packaged app: `nd package` bakes ND_APP_ID=<app.id> into the
// AppRun so window grouping / StartupWMClass bind to the app's own identity.
pub var app_id: [*:0]const u8 = "dev.nativedesktop.hello";

var smoke = false;
var global_app: ?*gtk.Application = null;
var global_ctx: ?*abi.NdContext = null;
var global_environ_map: ?*std.process.Environ.Map = null;
// Balances the startup `hold()` (below) exactly once, when the child's first
// commit presents a window. After that GtkApplication keeps itself alive via
// the window, so closing it drops the use-count to zero and the app quits.
var hold_released = false;
// Must outlive `onActivate`'s stack frame — `nd_register_backend` stores a
// pointer to this, and the core calls through it for the rest of the
// process's life (every commit-apply/marshal_async/etc.). A stack-local
// here segfaults on the first vtable call after onActivate returns.
var the_vtable: abi.NdBackend = undefined;

pub fn main(init: std.process.Init) void {
    // Before anything else, including argument parsing: CEF re-execs this same
    // binary for its renderer/GPU/utility roles, and in those processes this
    // call runs the whole subprocess and returns its exit code. Doing any other
    // work first means doing it once per Chromium process.
    if (cef.earlyExecuteProcess(init.minimal.args.vector)) |code| std.process.exit(code);
    // Windowed CEF embedding is X11-only, so a Wayland session runs the app
    // through XWayland when (and only when) the Chromium engine is asked for.
    // Must precede GTK's display connection, which AdwApplication opens on
    // registration below.
    cef.pinDisplayBackend();
    for (init.minimal.args.vector) |arg| {
        if (std.mem.eql(u8, std.mem.span(arg), "--smoke")) smoke = true;
    }
    global_environ_map = init.environ_map;
    if (init.environ_map.get("ND_APP_ID")) |id| {
        if (std.heap.page_allocator.dupeZ(u8, id)) |id_z| {
            // A GApplication id is a D-Bus name: dot-separated elements of
            // [A-Za-z0-9_] only. GTK accepts an invalid one and then degrades
            // in ways that look like unrelated bugs (a hyphen cost an
            // afternoon), so say so loudly and keep the working default.
            if (gio.Application.idIsValid(id_z) != 0) {
                app_id = id_z;
            } else {
                std.debug.print("ND_WARN ND_APP_ID={s} is not a valid application id (dot-separated [A-Za-z0-9_] elements) — keeping {s}\n", .{ id_z, app_id });
            }
        } else |_| {}
    }

    // AdwApplication's default startup handler runs adw_init() for us, which
    // loads the Adwaita stylesheet and starts AdwStyleManager's system
    // light/dark tracking from the first frame. The window class below stays
    // gtk.ApplicationWindow on purpose, so the default titlebar/window
    // controls keep working until titlebar ownership moves to a
    // <headerbar> widget.
    // HANDLES_OPEN: the OS delivers file/URI launches through the `open`
    // signal (app.openFile / app.openUrl system events) instead of `activate`.
    var app = adw.Application.new(app_id, .{ .handles_open = true });
    defer app.unref();
    global_app = app.as(gtk.Application);

    _ = gio.Application.signals.activate.connect(app.as(gtk.Application), ?*anyopaque, &onActivate, null, .{});
    _ = gio.Application.signals.open.connect(app.as(gtk.Application), ?*anyopaque, &onOpen, null, .{});

    // GApplication is single-instance: a second launch registers as "remote",
    // forwards its activation to the already-running primary, and run() then
    // returns 0 with no output — which reads as the app silently failing to
    // start. Detect that case and say so instead of exiting mutely.
    _ = gio.Application.register(app.as(gio.Application), null, null);
    if (gio.Application.getIsRemote(app.as(gio.Application)) != 0) {
        std.debug.print("ND_ALREADY_RUNNING another instance of {s} is already running; activated it and exiting\n", .{std.mem.span(app_id)});
    }

    // Only forward argv[0] to GApplication so its GOptionContext doesn't choke
    // on --smoke; the flag is parsed ourselves above.
    const status = gio.Application.run(app.as(gio.Application), 0, null);
    // Without this the Chromium process tree outlives the host.
    cef.shutdown();
    std.process.exit(@intCast(status));
}

fn onActivate(app: *gtk.Application, _: ?*anyopaque) callconv(.c) void {
    if (smoke) {
        // Pure-GTK smoke path.
        const window = gtk.ApplicationWindow.new(app);
        const win = window.as(gtk.Window);
        gtk.Window.setTitle(win, "NativeDesktop M1");
        gtk.Window.setDefaultSize(win, 480, 320);
        const button = gtk.Button.newWithLabel("Click me");
        _ = gtk.Button.signals.clicked.connect(button, ?*anyopaque, &onClicked, null, .{});
        gtk.Window.setChild(win, button.as(gtk.Widget));
        _ = gtk.Widget.signals.map.connect(window.as(gtk.Widget), ?*anyopaque, &onMapped, null, .{});
        gtk.Window.present(win);
        return;
    }

    // The embedder crosses the C ABI instead of building the Tree/Runtime/
    // automation.Server directly; the core owns them behind nd_init/
    // nd_register_backend/nd_start_runtime/nd_start_automation.
    backend.setApp(app);
    const ctx = abi.nd_init() orelse {
        std.debug.print("ND_RUNTIME_ERROR nd_init failed\n", .{});
        gio.Application.quit(app.as(gio.Application));
        return;
    };
    global_ctx = ctx;
    backend.setCtx(ctx);
    backend.initEventsAndStyle();

    // Hold the app alive with no window until the first commit presents one;
    // `onWindowAdded` releases it so closing that window quits the app.
    gio.Application.hold(app.as(gio.Application));
    _ = gtk.Application.signals.window_added.connect(app, ?*anyopaque, &onWindowAdded, null, .{});
    // Kill the bun child when the app tears down, so it dies with the parent
    // instead of being orphaned.
    _ = gio.Application.signals.shutdown.connect(app.as(gio.Application), ?*anyopaque, &onShutdown, null, .{});

    the_vtable = backend.ndBackend();
    abi.nd_register_backend(ctx, &the_vtable);
    abi.nd_set_backend_name(ctx, "gtk");

    // Opt-in capability ACL + native plugin. Absent env = safe default
    // (core UI ops granted). Mirrors swift/Sources/NDShell/main.swift's
    // env wiring (env names + call order).
    if (global_environ_map.?.get("ND_ACL_GRANTS")) |grants| {
        if (std.heap.page_allocator.dupeZ(u8, grants)) |grants_z| {
            abi.nd_set_acl(ctx, grants_z.ptr);
        } else |_| {}
    }
    if (global_environ_map.?.get("ND_PLUGINS")) |v| {
        if (std.mem.eql(u8, v, "1")) abi.nd_load_plugins_from_env(ctx);
    }

    if (abi.nd_start_runtime(ctx) != 0) {
        std.debug.print("ND_RUNTIME_ERROR nd_start_runtime failed\n", .{});
        gio.Application.quit(app.as(gio.Application));
        return;
    }
    backend.setTree(&ctx.tree); // nd_start_runtime just initialized ctx.tree

    if (global_environ_map.?.get("NATIVE_AUTOMATION")) |v| {
        if (std.mem.eql(u8, v, "1")) {
            if (abi.nd_start_automation(ctx) != 0) {
                std.debug.print("ND_AUTOMATION_ERROR nd_start_automation failed\n", .{});
            }
        }
    }
}

fn onWindowAdded(_: *gtk.Application, window: *gtk.Window, _: ?*anyopaque) callconv(.c) void {
    // GApplication's "shutdown" signal fires only after a closed window has
    // already finalized its children, so nd_shutdown's plugin-view teardown
    // would touch freed widgets — run it from the last window's close-request
    // instead, while the widget tree is still alive. nd_shutdown is
    // idempotent; onShutdown stays as the fallback for quit() paths that
    // never emit close-request.
    _ = gtk.Window.signals.close_request.connect(window, ?*anyopaque, &onCloseRequest, null, .{});
    // Track per-window active state to derive whole-app activation (below).
    _ = gobject.signalConnectData(window.as(gobject.Object), "notify::is-active", @ptrCast(&onNotifyActive), null, null, .{});
    // Schedule the launch recompute: a background spawn starts with no window
    // active and no notify::is-active ever fires, yet the core still needs a
    // standing value to replay after HelloAck. recomputeActive emits
    // unconditionally the first time (app_active starts null).
    if (!active_recheck_scheduled) {
        active_recheck_scheduled = true;
        _ = glib.idleAdd(&recomputeActive, null);
    }
    if (hold_released) return;
    hold_released = true;
    if (global_app) |app| gio.Application.release(app.as(gio.Application));
}

// Whole-app activation: any window active => app active. A window's
// notify::is-active fires for both the losing and the gaining window when
// focus moves between two of this app's windows, so the recompute is deferred
// to idle where those paired notifications collapse into one net state — only
// a real app-level gain/loss of focus emits app.activate/app.deactivate.
var app_active: ?bool = null; // null until the launch recompute records a value
var active_recheck_scheduled = false;

fn onNotifyActive(_: *gobject.Object, _: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
    if (active_recheck_scheduled) return;
    active_recheck_scheduled = true;
    _ = glib.idleAdd(&recomputeActive, null);
}

fn recomputeActive(_: ?*anyopaque) callconv(.c) c_int {
    active_recheck_scheduled = false;
    const app = global_app orelse return 0; // G_SOURCE_REMOVE
    var any_active = false;
    var node: ?*glib.List = gtk.Application.getWindows(app);
    while (node) |nd| : (node = nd.f_next) {
        const win: *gtk.Window = @ptrCast(@alignCast(nd.f_data orelse continue));
        if (gtk.Window.isActive(win) != 0) {
            any_active = true;
            break;
        }
    }
    const changed = if (app_active) |prev| any_active != prev else true;
    if (changed) {
        app_active = any_active;
        if (global_ctx) |ctx| {
            abi.nd_system_event(ctx, if (any_active) "app.activate" else "app.deactivate", "{}");
        }
    }
    return 0; // G_SOURCE_REMOVE
}

/// GApplication `open` delivery (HANDLES_OPEN): the OS hands us launched
/// files/URIs. Routed into `system.handleOpen`, which emits app.openFile /
/// app.openUrl. A no-op before the runtime booted (nothing to deliver to).
fn onOpen(_: *gtk.Application, files: [*]*gio.File, n_files: c_int, _: [*:0]u8, _: ?*anyopaque) callconv(.c) void {
    const ctx = global_ctx orelse return;
    system.handleOpen(ctx, files, n_files);
}

fn onCloseRequest(_: *gtk.Window, _: ?*anyopaque) callconv(.c) c_int {
    const app = global_app orelse return 0;
    // Closing a non-last window must not tear the core down.
    const windows = gtk.Application.getWindows(app);
    if (windows.f_next == null) {
        if (global_ctx) |ctx| abi.nd_shutdown(ctx);
    }
    return 0; // run the default close handler (destroy the window)
}

fn onShutdown(app: *gio.Application, _: ?*anyopaque) callconv(.c) void {
    _ = app;
    if (global_ctx) |ctx| abi.nd_shutdown(ctx);
}

fn onClicked(_: *gtk.Button, _: ?*anyopaque) callconv(.c) void {
    std.debug.print("ND_CLICKED\n", .{});
}

fn onMapped(_: *gtk.Widget, _: ?*anyopaque) callconv(.c) void {
    std.debug.print("ND_SMOKE_MAPPED\n", .{});
    _ = glib.idleAdd(&quitIdle, null);
}

fn quitIdle(_: ?*anyopaque) callconv(.c) c_int {
    if (global_app) |app| gio.Application.quit(app.as(gio.Application));
    return 0; // G_SOURCE_REMOVE
}

test "toolchain pin matches .zigversion" {
    const builtin = @import("builtin");
    const pinned = std.mem.trim(u8, @embedFile(".zigversion"), " \n\r\t");
    const required = try std.SemanticVersion.parse(pinned);
    try std.testing.expect(builtin.zig_version.order(required) == .eq);
}
