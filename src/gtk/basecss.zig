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
// (AppKit peer: prefersCompactControlSizeMetrics).
//
// It also carries the two allowlisted cssClasses libadwaita scopes to widgets
// apps do not put them on, so the class means the same thing on both backends:
// `pill` is a button SIZE class there, never a label treatment (AppKit peer:
// ndApplyPillBadge), and `activatable` is scoped to `row` (AppKit peer:
// ndApplyActivatable, which tracks an NSStackView). The label capsule reuses
// .nd-badge's shape and fill and takes its typography from the cascade, like
// the AppKit peer; the box hover reuses libadwaita's own row values (9px
// radius, currentColor at 4%) but paints the fill as a background IMAGE, so a
// node's `style.background` keeps its background-color underneath. There is no
// :active half: GTK sets PRELIGHT along the pointer-focus chain, so a plain
// GtkBox gets :hover, but ACTIVE is set by the widget itself and a box never
// sets it.
//
// Installed once at display level; providers restyle retroactively, so a lazy
// install is safe.
var base_installed = false;
const nd_base_css =
    \\.nd-badge, label.pill { background: alpha(currentColor, 0.12); border-radius: 99px; padding: 1px 7px; }
    \\.nd-badge { font-size: 0.85em; font-weight: bold; }
    \\button.compact { min-height: 24px; padding: 0 8px; }
    \\button.large { min-height: 40px; padding: 0 18px; }
    \\.nd-compact button { min-height: 26px; }
    \\.nd-compact entry { min-height: 26px; }
    \\box.activatable { border-radius: 9px; }
    \\box.activatable:hover { background-image: image(alpha(currentColor, 0.04)); }
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
