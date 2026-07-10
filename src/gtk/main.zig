const std = @import("std");
const glib = @import("glib");
const gobject = @import("gobject");
const gio = @import("gio");
const gtk = @import("gtk");
const abi = @import("../abi.zig");
const backend = @import("backend.zig");

pub const app_id = "dev.nativedesktop.hello";

var smoke = false;
var global_app: ?*gtk.Application = null;
var global_environ_map: ?*std.process.Environ.Map = null;
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

    var app = gtk.Application.new(app_id, .{});
    defer app.unref();
    global_app = app;

    _ = gio.Application.signals.activate.connect(app, ?*anyopaque, &onActivate, null, .{});

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
    backend.setCtx(ctx);
    backend.initEventsAndStyle();

    // Hold the app alive with no window until the first commit presents one.
    gio.Application.hold(app.as(gio.Application));

    the_vtable = backend.ndBackend();
    abi.nd_register_backend(ctx, &the_vtable);

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
