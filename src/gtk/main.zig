const std = @import("std");
const glib = @import("glib");
const gobject = @import("gobject");
const gio = @import("gio");
const gtk = @import("gtk");
const adw = @import("adw");
const abi = @import("../abi.zig");
const backend = @import("backend.zig");

pub const app_id = "dev.nativedesktop.hello";

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
    for (init.minimal.args.vector) |arg| {
        if (std.mem.eql(u8, std.mem.span(arg), "--smoke")) smoke = true;
    }
    global_environ_map = init.environ_map;

    // AdwApplication's default startup handler runs adw_init() for us, which
    // loads the Adwaita stylesheet and starts AdwStyleManager's system
    // light/dark tracking from the first frame. The window class below stays
    // gtk.ApplicationWindow on purpose, so the default titlebar/window
    // controls keep working until a later task hands titlebar ownership to
    // a <headerbar> widget.
    var app = adw.Application.new(app_id, .{});
    defer app.unref();
    global_app = app.as(gtk.Application);

    _ = gio.Application.signals.activate.connect(app.as(gtk.Application), ?*anyopaque, &onActivate, null, .{});

    // Only forward argv[0] to GApplication so its GOptionContext doesn't choke
    // on --smoke; the flag is parsed ourselves above.
    const status = gio.Application.run(app.as(gio.Application), 0, null);
    std.process.exit(@intCast(status));
}

fn onActivate(app: *gtk.Application, _: ?*anyopaque) callconv(.c) void {
    if (smoke) {
        // Unchanged M1 pure-GTK smoke path.
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

    // M6a: the embedder crosses the C ABI instead of building the Tree/
    // Runtime/automation.Server directly (M2-M5c called those Zig types by
    // hand; the core now owns them behind nd_init/nd_register_backend/
    // nd_start_runtime/nd_start_automation).
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

    // M10: opt-in capability ACL + native plugin. Absent env = safe default
    // (core UI ops granted), byte-identical to pre-M10 behavior. Mirrors
    // swift/Sources/NDShell/main.swift's env wiring (env names + call order).
    if (global_environ_map.?.get("ND_ACL_GRANTS")) |grants| {
        if (std.heap.page_allocator.dupeZ(u8, grants)) |grants_z| {
            abi.nd_set_acl(ctx, grants_z.ptr);
        } else |_| {}
    }
    if (global_environ_map.?.get("ND_PLUGINS")) |v| {
        if (std.mem.eql(u8, v, "1")) {
            const paths = global_environ_map.?.get("ND_PLUGIN_PATHS") orelse global_environ_map.?.get("ND_PLUGIN_PATH") orelse "";
            var it = std.mem.splitScalar(u8, paths, ':');
            while (it.next()) |path| {
                if (path.len == 0) continue;
                const path_z = std.heap.page_allocator.dupeZ(u8, path) catch @panic("oom");
                defer std.heap.page_allocator.free(path_z);
                if (abi.nd_load_plugin(ctx, path_z.ptr) != 0) std.debug.print("ND_PLUGIN_LOAD_FAILED path={s}\n", .{path});
            }
        }
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

fn onWindowAdded(_: *gtk.Application, _: *gtk.Window, _: ?*anyopaque) callconv(.c) void {
    if (hold_released) return;
    hold_released = true;
    if (global_app) |app| gio.Application.release(app.as(gio.Application));
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
