// Framework base CSS install, split from style.zig so tabs.zig can import it
// without pulling style.zig into the "generated" module: style.zig is the
// ROOT of its own dedicated test module (build.zig style_tests), and Zig 0.16
// forbids one file belonging to two modules, so nothing reachable from
// src/generated/widgets.zig may import style.zig. This file must stay
// self-contained (gtk/gdk only).
const gtk = @import("gtk");
const gdk = @import("gdk");

// Framework base CSS for the ND-owned classes the generated arms add outside
// the per-node style pipeline: Button.badge's capsule suffix, Button.size
// metrics (AppKit peer: NSControl.controlSize), and Window.density=compact
// (AppKit peer: prefersCompactControlSizeMetrics). Installed once at display
// level; providers restyle retroactively, so a lazy install is safe.
var base_installed = false;
const nd_base_css =
    \\.nd-badge { background: alpha(currentColor, 0.12); border-radius: 99px; padding: 1px 7px; font-size: 0.85em; font-weight: bold; }
    \\button.compact { min-height: 24px; padding: 0 8px; }
    \\button.large { min-height: 40px; padding: 0 18px; }
    \\.nd-compact button { min-height: 26px; }
    \\.nd-compact entry { min-height: 26px; }
;

/// Called from tabs.zig's createWindow (a live display is guaranteed there);
/// a no-op after the first successful install.
pub fn ensureBaseCss() void {
    if (base_installed) return;
    const display = gdk.Display.getDefault() orelse return;
    const p = gtk.CssProvider.new();
    gtk.CssProvider.loadFromString(p, nd_base_css);
    gtk.StyleContext.addProviderForDisplay(display, p.as(gtk.StyleProvider), 600); // STYLE_PROVIDER_PRIORITY_APPLICATION
    base_installed = true;
}
