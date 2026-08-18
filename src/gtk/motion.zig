// Where the GTK backend asks the desktop whether motion is welcome. Every
// widget that animates (<skeleton>'s shimmer, <chart>'s grow-in) reads this
// one answer, so the OS switch is honoured identically everywhere.
const gtk = @import("gtk");
const gobject = @import("gobject");

/// The OS switch outranks any `animated` prop: `gtk-enable-animations` off
/// means the widget draws its resting state and never starts a tick callback
/// (AppKit peer: accessibilityDisplayShouldReduceMotion).
pub fn animationsEnabled() bool {
    const settings = gtk.Settings.getDefault() orelse return true;
    var v = gobject.ext.Value.newFrom(true);
    defer gobject.Value.unset(&v);
    gobject.Object.getProperty(@ptrCast(@alignCast(settings)), "gtk-enable-animations", &v);
    return gobject.Value.getBoolean(&v) != 0;
}
