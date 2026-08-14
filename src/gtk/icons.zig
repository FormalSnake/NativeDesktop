const std = @import("std");
const gtk = @import("gtk");
const gdk = @import("gdk");
const glib = @import("glib");
const gobject = @import("gobject");

// Icon names cross the wire as freedesktop names (the same names macOS maps to
// SF Symbols in swift/Sources/NDShell/Icons.swift). GTK renders a bare
// freedesktop name as a full-color, un-recolored icon, so on a dark theme it
// stays dark instead of tracking the widget foreground. GTK only recolors an
// icon to the foreground when the `-symbolic` variant is used — which is how a
// template SF Symbol tints on macOS. Buttons/menus therefore prefer the
// symbolic variant, matching platforms without any per-app tuning.

var arena_state = std.heap.ArenaAllocator.init(std.heap.page_allocator);
const arena = arena_state.allocator();

/// Returns `name`'s `-symbolic` variant when the active icon theme has one, so
/// the icon recolors to the widget foreground; otherwise returns `name`
/// unchanged (a themeless name must never become a missing icon). Already
/// `-symbolic` names pass through. The returned string outlives the call (arena
/// allocated, same leak-tolerant contract as the backend's `dupeZ`).
pub fn symbolic(name: [:0]const u8) [:0]const u8 {
    if (name.len == 0) return name;
    if (std.mem.endsWith(u8, name, "-symbolic")) return name;
    const display = gdk.Display.getDefault() orelse return name;
    const theme = gtk.IconTheme.getForDisplay(display);
    const candidate = std.fmt.allocPrintSentinel(arena, "{s}-symbolic", .{name}, 0) catch return name;
    if (gtk.IconTheme.hasIcon(theme, candidate) != 0) return candidate;
    return name;
}

/// Every `iconData` slot renders at this size: GTK's own icon-name paths
/// (gtk_button_set_icon_name, AdwButtonContent, an AdwActionRow prefix) all
/// resolve to a 16px image, so raw bytes have to match or a data icon and a
/// themed one differ in one row of widgets.
pub const data_pixel_size: c_int = 16;

/// A texture from raw image bytes — a `data:<mime>;base64,<payload>` URL or a
/// bare base64 payload, which is the shape `faviconChanged` hands the app on
/// GTK. Returns a full reference the caller owns. A payload GDK cannot decode
/// warns once (tagged with `what`) and returns null, so the widget renders
/// without an icon rather than failing.
pub fn textureFromData(data: []const u8, what: []const u8) ?*gdk.Texture {
    const comma = std.mem.indexOfScalar(u8, data, ',');
    const b64 = if (std.mem.startsWith(u8, data, "data:") and comma != null) data[comma.? + 1 ..] else data;
    const decoder = std.base64.standard.Decoder;
    const size = decoder.calcSizeForSlice(b64) catch {
        std.debug.print("ND_WARN {s} iconData: payload is not base64\n", .{what});
        return null;
    };
    const buf = std.heap.page_allocator.alloc(u8, @max(size, 1)) catch return null;
    defer std.heap.page_allocator.free(buf);
    decoder.decode(buf[0..size], b64) catch {
        std.debug.print("ND_WARN {s} iconData: payload is not base64\n", .{what});
        return null;
    };

    const bytes = glib.Bytes.new(buf.ptr, size);
    defer bytes.unref();
    var err: ?*glib.Error = null;
    return gdk.Texture.newFromBytes(bytes, &err) orelse {
        if (err) |e| {
            std.debug.print("ND_WARN {s} iconData: {s}\n", .{ what, if (e.f_message) |m| std.mem.span(m) else "undecodable image" });
            e.free();
        }
        return null;
    };
}

/// `textureFromData` wrapped in a GtkImage at `data_pixel_size`, for the slots
/// that hold a widget rather than a paintable.
pub fn imageFromData(data: []const u8, what: []const u8) ?*gtk.Image {
    const texture = textureFromData(data, what) orelse return null;
    defer gobject.Object.unref(texture.as(gobject.Object));
    const img = gtk.Image.newFromPaintable(texture.as(gdk.Paintable));
    gtk.Image.setPixelSize(img, data_pixel_size);
    return img;
}
