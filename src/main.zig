const std = @import("std");
const glib = @import("glib");
const gobject = @import("gobject");
const gio = @import("gio");
const gtk = @import("gtk");
const Tree = @import("tree.zig").Tree;
const Runtime = @import("runtime.zig").Runtime;
const automation = @import("automation.zig");

pub const app_id = "dev.nativedesktop.hello";

var smoke = false;
var global_app: ?*gtk.Application = null;
var global_environ_map: ?*std.process.Environ.Map = null;
var global_environ: std.process.Environ = undefined;
var global_gpa: std.mem.Allocator = undefined;
var tree: Tree = undefined;

pub fn main(init: std.process.Init) void {
    for (init.minimal.args.vector) |arg| {
        if (std.mem.eql(u8, std.mem.span(arg), "--smoke")) smoke = true;
    }
    global_environ_map = init.environ_map;
    global_environ = init.minimal.environ;
    global_gpa = init.gpa;

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
    // M2: the child builds the tree over NDP.
    const gpa = global_gpa;
    tree = Tree.init(gpa, app);
    // Hold the app alive with no window until the first commit presents one.
    gio.Application.hold(app.as(gio.Application));
    const rt = Runtime.start(gpa, app, &tree, global_environ_map.?, global_environ) catch |err| {
        std.debug.print("ND_RUNTIME_ERROR {any}\n", .{err});
        gio.Application.quit(app.as(gio.Application));
        return;
    };

    if (global_environ_map.?.get("NATIVE_AUTOMATION")) |v| {
        if (std.mem.eql(u8, v, "1")) {
            const runtime_dir = global_environ_map.?.get("XDG_RUNTIME_DIR") orelse "/tmp";
            _ = automation.Server.start(gpa, rt.getIo(), &tree, runtime_dir) catch |err| {
                std.debug.print("ND_AUTOMATION_ERROR {any}\n", .{err});
            };
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
