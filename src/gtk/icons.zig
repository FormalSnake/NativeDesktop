const std = @import("std");
const gtk = @import("gtk");
const gdk = @import("gdk");

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
