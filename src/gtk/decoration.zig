// Window-control layout on the x11 backend.
//
// GTK sources `gtk-decoration-layout` per display backend. The Wayland backend
// reads the desktop's own value from the settings portal; the x11 backend reads
// XSettings and nothing else. A compositor that ships no XSettings manager
// (wlroots, Hyprland) therefore leaves an XWayland client on GTK's compiled-in
// `menu:close`, so the app draws a close button on a desktop where every native
// client correctly draws none. Read the same portal key the Wayland backend
// reads, and apply it when, and only when, the display itself had no answer.
const std = @import("std");
const glib = @import("glib");
const gobject = @import("gobject");
const gio = @import("gio");
const gdk = @import("gdk");
const gtk = @import("gtk");

const G_TYPE_STRING: usize = 16 << 2;

const portal_schema = "org.gnome.desktop.wm.preferences";
const portal_key = "button-layout";

var resolved = false;

/// Called once per process from the first window creation, where a display is
/// guaranteed. A no-op on any backend that answers for the setting itself.
pub fn ensureLayoutFromPortal() void {
    if (resolved) return;
    resolved = true;

    const display = gdk.Display.getDefault() orelse return;
    var probe: gobject.Value = std.mem.zeroes(gobject.Value);
    _ = gobject.Value.init(&probe, G_TYPE_STRING);
    defer gobject.Value.unset(&probe);
    if (gdk.Display.getSetting(display, "gtk-decoration-layout", &probe) != 0) {
        const have = gobject.Value.getString(&probe) orelse "";
        std.debug.print("ND_DECORATION_LAYOUT source=display value={s}\n", .{have});
        return;
    }

    var buf: [256]u8 = undefined;
    const layout = readPortal(&buf) orelse {
        std.debug.print("ND_DECORATION_LAYOUT source=builtin value=(no portal answer)\n", .{});
        return;
    };
    const settings = gtk.Settings.getDefault() orelse return;
    var v: gobject.Value = std.mem.zeroes(gobject.Value);
    _ = gobject.Value.init(&v, G_TYPE_STRING);
    defer gobject.Value.unset(&v);
    gobject.Value.setString(&v, layout.ptr);
    gobject.Object.setProperty(@ptrCast(@alignCast(settings)), "gtk-decoration-layout", &v);
    std.debug.print("ND_DECORATION_LAYOUT source=portal value={s}\n", .{layout});
}

/// The portal's answer, normalized into a NUL-terminated slice of `buf`.
/// `org.gnome.desktop.wm.preferences` names the app-menu slot `appmenu`, which
/// is GTK's `menu`; every other token is spelled the same in both.
fn readPortal(buf: []u8) ?[:0]const u8 {
    const bus = gio.busGetSync(.session, null, null) orelse return null;
    defer _ = gobject.Object.unref(@ptrCast(@alignCast(bus)));

    const args = [_]*glib.Variant{
        glib.Variant.newString(portal_schema),
        glib.Variant.newString(portal_key),
    };
    const params = glib.Variant.refSink(glib.Variant.newTuple(&args, args.len));
    defer glib.Variant.unref(params);
    // ReadOne is the current call; Read is the pre-1.16 portal's, and it wraps
    // the same string in one more variant layer.
    const reply = callPortal(bus, "ReadOne", params) orelse
        callPortal(bus, "Read", params) orelse return null;
    defer glib.Variant.unref(reply);

    var cursor = glib.Variant.getChildValue(reply, 0);
    defer glib.Variant.unref(cursor);
    while (std.mem.orderZ(u8, glib.Variant.getTypeString(cursor), "v") == .eq) {
        const inner = glib.Variant.getVariant(cursor);
        glib.Variant.unref(cursor);
        cursor = inner;
    }
    if (std.mem.orderZ(u8, glib.Variant.getTypeString(cursor), "s") != .eq) return null;

    var len: usize = 0;
    const raw = glib.Variant.getString(cursor, &len);
    return rewriteAppMenu(raw[0..len], buf);
}

fn rewriteAppMenu(src: []const u8, buf: []u8) ?[:0]const u8 {
    var out: usize = 0;
    var at: usize = 0;
    while (at <= src.len) {
        const end = std.mem.indexOfAnyPos(u8, src, at, ":,") orelse src.len;
        const token = src[at..end];
        const mapped = if (std.mem.eql(u8, token, "appmenu")) "menu" else token;
        if (out + mapped.len + 2 > buf.len) return null;
        @memcpy(buf[out..][0..mapped.len], mapped);
        out += mapped.len;
        if (end == src.len) break;
        buf[out] = src[end];
        out += 1;
        at = end + 1;
    }
    buf[out] = 0;
    return buf[0..out :0];
}

fn callPortal(bus: *gio.DBusConnection, method: [*:0]const u8, params: *glib.Variant) ?*glib.Variant {
    return gio.DBusConnection.callSync(
        bus,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
        method,
        params,
        null,
        .{},
        1000,
        null,
        null,
    );
}

test "appmenu maps to gtk's menu slot, other tokens pass through" {
    var buf: [64]u8 = undefined;
    try std.testing.expectEqualStrings("menu:close", rewriteAppMenu("appmenu:close", &buf).?);
    try std.testing.expectEqualStrings(":", rewriteAppMenu(":", &buf).?);
    try std.testing.expectEqualStrings(
        "menu:minimize,maximize,close",
        rewriteAppMenu("appmenu:minimize,maximize,close", &buf).?,
    );
    try std.testing.expectEqualStrings("close:", rewriteAppMenu("close:", &buf).?);
}
