const std = @import("std");
const gtk = @import("gtk");
const gdk = @import("gdk");
const glib = @import("glib");
const gobject = @import("gobject");
const gsk = @import("gsk");
const cairo = @import("cairo");

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
    const display = gdk.Display.getDefault() orelse return name;
    const theme = gtk.IconTheme.getForDisplay(display);
    if (std.mem.endsWith(u8, name, "-symbolic")) {
        if (gtk.IconTheme.hasIcon(theme, name) == 0) warnMissing(name);
        return name;
    }
    const candidate = std.fmt.allocPrintSentinel(arena, "{s}-symbolic", .{name}, 0) catch return name;
    if (gtk.IconTheme.hasIcon(theme, candidate) != 0) return candidate;
    if (gtk.IconTheme.hasIcon(theme, name) == 0) warnMissing(name);
    return name;
}

/// Starts loading the icon theme on a worker thread. GTK loads it in a thread
/// at startup, but adw_init then adds libadwaita's icon resource path, which
/// throws that load away, and the first `gtk_icon_theme_has_icon` (the first
/// commit's first icon) rescanned every theme directory on the UI thread. The
/// theme serializes its own loading under its lock, and has_icon reaches no
/// main-thread-only path, so the UI thread's first lookup finds the theme
/// loaded or waits only for the rest of the load (gtkicontheme.c: "Public APIs
/// that never call _mainthread are threadsafe").
pub fn preloadTheme() void {
    const display = gdk.Display.getDefault() orelse return;
    const theme = gtk.IconTheme.getForDisplay(display);
    const thread = std.Thread.spawn(.{}, loadTheme, .{theme}) catch return;
    thread.detach();
}

fn loadTheme(theme: *gtk.IconTheme) void {
    _ = gtk.IconTheme.hasIcon(theme, "image-missing");
}

var warned: std.StringHashMapUnmanaged(void) = .empty;

/// A name the theme does not have draws GTK's missing-image glyph; said once
/// per name, so a drive (or a developer) can tell the icon is not there.
fn warnMissing(name: [:0]const u8) void {
    if (warned.contains(name)) return;
    const key = arena.dupe(u8, name) catch return;
    warned.put(arena, key, {}) catch return;
    std.debug.print("ND_WARN icon \"{s}\" is not in the icon theme\n", .{name});
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
    // The same favicon comes back on every tab row and every command bar
    // keystroke, and each decode is a base64 pass plus a PNG decode.
    const key = std.hash.Wyhash.hash(0, data);
    if (decoded.get(key)) |t| return @ptrCast(gobject.Object.ref(t.as(gobject.Object)));
    const texture = decodeTexture(data, what) orelse return null;
    if (decoded.count() >= decoded_max) {
        var it = decoded.valueIterator();
        while (it.next()) |t| gobject.Object.unref(t.*.as(gobject.Object));
        decoded.clearRetainingCapacity();
    }
    decoded.put(std.heap.smp_allocator, key, @ptrCast(gobject.Object.ref(texture.as(gobject.Object)))) catch {};
    return texture;
}

/// Decoded textures by a hash of their bytes. Textures are immutable, so one
/// can back any number of widgets. UI thread only.
var decoded: std.AutoHashMapUnmanaged(u64, *gdk.Texture) = .empty;
const decoded_max = 256;

fn decodeTexture(data: []const u8, what: []const u8) ?*gdk.Texture {
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
    // GDK decodes PNG, JPEG and TIFF itself and hands everything else to
    // gdk-pixbuf, whose SVG loader comes from librsvg and is often absent
    // (NixOS's system loaders.cache has none, and a packaged app uses the
    // host's), so an SVG favicon goes through GTK's own renderer instead.
    if (isSvg(buf[0..size])) {
        if (svgTexture(bytes)) |t| return t;
    }
    var err: ?*glib.Error = null;
    return gdk.Texture.newFromBytes(bytes, &err) orelse {
        if (err) |e| {
            std.debug.print("ND_WARN {s} iconData: {s}\n", .{ what, if (e.f_message) |m| std.mem.span(m) else "undecodable image" });
            e.free();
        }
        return null;
    };
}

fn isSvg(bytes: []const u8) bool {
    const head = bytes[0..@min(bytes.len, 1024)];
    return std.mem.indexOf(u8, head, "<svg") != null;
}

/// The square an SVG is rasterized into: four times `data_pixel_size`, so the
/// icon stays sharp at every scale factor GTK draws a 16px slot at.
const svg_raster_px: c_int = data_pixel_size * 4;

const SvgNewFromBytes = *const fn (*glib.Bytes) callconv(.c) ?*gdk.Paintable;
var svg_new_from_bytes: ?SvgNewFromBytes = null;
var svg_lookup_done = false;

/// `gtk_svg_new_from_bytes` arrived in GTK 4.22; resolved at runtime so an
/// older libgtk still loads the host and falls back to gdk-pixbuf.
fn svgNewFromBytes() ?SvgNewFromBytes {
    if (svg_lookup_done) return svg_new_from_bytes;
    svg_lookup_done = true;
    var lib = std.DynLib.open("libgtk-4.so.1") catch return null;
    svg_new_from_bytes = lib.lookup(SvgNewFromBytes, "gtk_svg_new_from_bytes");
    return svg_new_from_bytes;
}

fn svgTexture(bytes: *glib.Bytes) ?*gdk.Texture {
    const new_svg = svgNewFromBytes() orelse return null;
    const paintable = new_svg(bytes) orelse return null;
    defer gobject.Object.unref(@ptrCast(@alignCast(paintable)));

    var w: f64 = @floatFromInt(svg_raster_px);
    var h: f64 = w;
    const ratio = gdk.Paintable.getIntrinsicAspectRatio(paintable);
    if (ratio > 1) h = w / ratio else if (ratio > 0) w = h * ratio;

    const snapshot = gtk.Snapshot.new();
    gdk.Paintable.snapshot(paintable, snapshot.as(gdk.Snapshot), w, h);
    const node = gtk.Snapshot.freeToNode(snapshot) orelse return null;
    defer gsk.RenderNode.unref(node);

    const pw: c_int = @intFromFloat(@ceil(w));
    const ph: c_int = @intFromFloat(@ceil(h));
    const surface = cairo.Surface.imageCreate(.argb32, pw, ph);
    defer cairo.Surface.destroy(surface);
    const cr = cairo.Context.create(surface);
    gsk.RenderNode.draw(node, cr);
    cairo.Context.destroy(cr);
    cairo.Surface.flush(surface);

    const data = cairo.Surface.imageGetData(surface) orelse return null;
    const stride: usize = @intCast(cairo.Surface.imageGetStride(surface));
    const pixels = glib.Bytes.new(data, stride * @as(usize, @intCast(ph)));
    defer pixels.unref();
    // Cairo's ARGB32 is native-endian premultiplied, which on every target GTK
    // runs on here is GDK's B8G8R8A8_PREMULTIPLIED.
    return gdk.MemoryTexture.new(pw, ph, .b8g8r8a8_premultiplied, pixels, stride).as(gdk.Texture);
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
