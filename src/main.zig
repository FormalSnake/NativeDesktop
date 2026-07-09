const std = @import("std");
const glib = @import("glib");
const gobject = @import("gobject");
const gio = @import("gio");
const gtk = @import("gtk");

pub const app_id = "dev.nativedesktop.hello";

pub fn main(init: std.process.Init) void {
    var app = gtk.Application.new(app_id, .{});
    defer app.unref();

    _ = gio.Application.signals.activate.connect(app, ?*anyopaque, &onActivate, null, .{});

    const argv = init.minimal.args.vector;
    const status = gio.Application.run(app.as(gio.Application), @intCast(argv.len), @ptrCast(@constCast(argv.ptr)));
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

    gtk.Window.present(win);
}

fn onClicked(_: *gtk.Button, _: ?*anyopaque) callconv(.c) void {
    std.debug.print("ND_CLICKED\n", .{});
}

test "toolchain pin matches .zigversion" {
    const builtin = @import("builtin");
    const pinned = std.mem.trim(u8, @embedFile(".zigversion"), " \n\r\t");
    const required = try std.SemanticVersion.parse(pinned);
    try std.testing.expect(builtin.zig_version.order(required) == .eq);
}
