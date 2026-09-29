//! Keys typed into a text field before it is on screen.
//!
//! A field that takes the keyboard as it opens (a command bar on ctrl+T) has
//! GTK's focus a frame before it is mapped, and a key typed into it then is
//! dropped. Printable keys that arrive at a window while its focus widget is a
//! text field that is not mapped yet are held at the surface, before GTK sees
//! them, and put into the field in the order they came as soon as it is.
const std = @import("std");
const gobject = @import("gobject");
const gdk = @import("gdk");
const gtk = @import("gtk");

const alloc = std.heap.c_allocator;

const WIRED_KEY: [*:0]const u8 = "nd-typeahead-wired";

var text: std.ArrayList(u8) = .empty;
var held_window: ?*gtk.Window = null;
var tick: c_uint = 0;

/// GTK thread. Watches `window`'s surface from now on.
pub fn wire(window: *gtk.Window) void {
    const obj = window.as(gobject.Object);
    if (gobject.Object.getData(obj, WIRED_KEY) != null) return;
    const surface = gtk.Native.getSurface(window.as(gtk.Native)) orelse return;
    gobject.Object.setData(obj, WIRED_KEY, @ptrFromInt(1));
    _ = gobject.signalConnectData(surface.as(gobject.Object), "event", @ptrCast(&onSurfaceEvent), window, null, .{});
}

/// The window's focus widget when it is a text field that is not on screen yet.
fn unmappedField(window: *gtk.Window) ?*gtk.Widget {
    const focus = gtk.Window.getFocus(window) orelse return null;
    if (gobject.ext.cast(gtk.Editable, focus) == null) return null;
    return if (gtk.Widget.getMapped(focus) == 0) focus else null;
}

fn onSurfaceEvent(_: *gdk.Surface, event: *gdk.Event, window: *gtk.Window) callconv(.c) c_int {
    const kind = gdk.Event.getEventType(event);
    if (kind != .key_press and kind != .key_release) return 0;
    const mods = gdk.Event.getModifierState(event);
    if (mods.control_mask or mods.alt_mask or mods.super_mask or mods.meta_mask) return 0;
    if (unmappedField(window) == null) {
        // On screen now: what was held goes in first, so this key lands behind it.
        if (text.items.len > 0 and held_window == window) flush(window);
        return 0;
    }
    const ch = gdk.keyvalToUnicode(gdk.KeyEvent.getKeyval(@ptrCast(event)));
    if (ch < 0x20 or ch == 0x7f) return 0;
    // The release of a held key goes with it.
    if (kind == .key_release) return 1;
    var utf8: [4]u8 = undefined;
    const n = std.unicode.utf8Encode(@intCast(ch), &utf8) catch return 0;
    text.appendSlice(alloc, utf8[0..n]) catch return 0;
    held_window = window;
    if (tick == 0) tick = gtk.Widget.addTickCallback(window.as(gtk.Widget), &onTick, null, null);
    return 1;
}

/// Every frame while something is held: the field is mapped on one of them.
fn onTick(widget: *gtk.Widget, _: *gdk.FrameClock, _: ?*anyopaque) callconv(.c) c_int {
    const window: *gtk.Window = @ptrCast(@alignCast(widget));
    if (text.items.len > 0 and unmappedField(window) != null) return 1;
    if (text.items.len > 0) flush(window);
    tick = 0;
    return 0;
}

fn flush(window: *gtk.Window) void {
    defer {
        text.clearRetainingCapacity();
        held_window = null;
    }
    const focus = gtk.Window.getFocus(window) orelse return;
    const field = gobject.ext.cast(gtk.Editable, focus) orelse return;
    const z = alloc.dupeZ(u8, text.items) catch return;
    defer alloc.free(z);
    gtk.Editable.deleteSelection(field);
    var pos: c_int = gtk.Editable.getPosition(field);
    gtk.Editable.insertText(field, z.ptr, @intCast(z.len), &pos);
    gtk.Editable.setPosition(field, pos);
}
