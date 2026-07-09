const std = @import("std");
const glib = @import("glib");
const gobject = @import("gobject");
const gio = @import("gio");
const gtk = @import("gtk");

pub const app_id = "dev.nativedesktop.hello";

var smoke = false;
var global_app: ?*gtk.Application = null;

pub fn main(init: std.process.Init) void {
    for (init.minimal.args.vector) |arg| {
        if (std.mem.eql(u8, std.mem.span(arg), "--smoke")) smoke = true;
    }

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
    const window = gtk.ApplicationWindow.new(app);
    const win = window.as(gtk.Window);
    gtk.Window.setTitle(win, "NativeDesktop M1");
    gtk.Window.setDefaultSize(win, 480, 320);

    const button = gtk.Button.newWithLabel("Click me");
    _ = gtk.Button.signals.clicked.connect(button, ?*anyopaque, &onClicked, null, .{});
    gtk.Window.setChild(win, button.as(gtk.Widget));

    if (smoke) {
        _ = gtk.Widget.signals.map.connect(window.as(gtk.Widget), ?*anyopaque, &onMapped, null, .{});
    }

    gtk.Window.present(win);
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
