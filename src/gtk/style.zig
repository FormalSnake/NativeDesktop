const std = @import("std");
const gtk = @import("gtk");
const gdk = @import("gdk");
const gobject = @import("gobject");
const generated = @import("generated");

// ---- One GtkCssProvider per styled node, unique class `nd-<id>`,
// installed once at display level; update = replace provider content. ----
var gpa: std.mem.Allocator = undefined;
var providers: std.AutoHashMapUnmanaged(u32, *gtk.CssProvider) = .empty;
var size_requested: std.AutoHashMapUnmanaged(u32, void) = .empty;
pub const StyleErrorFn = *const fn (node_id: u32, key: []const u8) void;
var on_error: ?StyleErrorFn = null;
var ready = false;

pub fn init(allocator: std.mem.Allocator, err_fn: StyleErrorFn) void {
    gpa = allocator;
    on_error = err_fn;
    ready = true;
}

fn findKey(name: []const u8) ?generated.StyleKeyDef {
    for (generated.style_keys) |k| {
        if (std.mem.eql(u8, k.name, name)) return k;
    }
    return null;
}

fn jsonInt(v: std.json.Value) i64 {
    return switch (v) {
        .integer => |n| n,
        .float => |f| @intFromFloat(f),
        else => 0,
    };
}

/// int/float -> all four sides; per-side object -> only the given sides.
/// Margin is a GTK widget property, not CSS.
fn applyMarginSpacing(widget: *gtk.Widget, value: std.json.Value) void {
    switch (value) {
        .integer, .float => {
            const m: c_int = @intCast(jsonInt(value));
            gtk.Widget.setMarginStart(widget, m);
            gtk.Widget.setMarginEnd(widget, m);
            gtk.Widget.setMarginTop(widget, m);
            gtk.Widget.setMarginBottom(widget, m);
        },
        .object => |o| {
            if (o.get("left")) |v| gtk.Widget.setMarginStart(widget, @intCast(jsonInt(v)));
            if (o.get("right")) |v| gtk.Widget.setMarginEnd(widget, @intCast(jsonInt(v)));
            if (o.get("top")) |v| gtk.Widget.setMarginTop(widget, @intCast(jsonInt(v)));
            if (o.get("bottom")) |v| gtk.Widget.setMarginBottom(widget, @intCast(jsonInt(v)));
        },
        else => std.debug.print("ND_WARN unsupported margin value\n", .{}),
    }
}

/// halign/valign string -> GtkAlign. hexpand/vexpand/halign/valign are GTK
/// widget properties (like margin), not CSS — they drive the layout
/// engine, so a filled window comes from setting them, not from a stylesheet.
fn parseAlign(value: std.json.Value) ?gtk.Align {
    if (value != .string) return null;
    const s = value.string;
    if (std.mem.eql(u8, s, "fill")) return .fill;
    if (std.mem.eql(u8, s, "start")) return .start;
    if (std.mem.eql(u8, s, "end")) return .end;
    if (std.mem.eql(u8, s, "center")) return .center;
    std.debug.print("ND_WARN unknown align value \"{s}\" (fill|start|end|center)\n", .{s});
    return null;
}

/// `{css_name}: {value}{unit};` — the hex/rgb sanity check applies only to
/// `kind == "color"` keys (background/color/borderColor); other string-valued
/// keys (fontWeight enum, fontFamily string, …) pass through verbatim.
fn emitScalarCss(list: *std.ArrayList(u8), allocator: std.mem.Allocator, css_name: []const u8, value: std.json.Value, unit: ?[]const u8, kind: []const u8) void {
    switch (value) {
        .string => |s| {
            if (std.mem.eql(u8, kind, "color") and (s.len == 0 or !(s[0] == '#' or std.mem.startsWith(u8, s, "rgb")))) {
                std.debug.print("ND_WARN style color value \"{s}\" is not a hex/rgb string (GTK is not web CSS)\n", .{s});
                return;
            }
            list.print(allocator, "{s}: {s};", .{ css_name, s }) catch {};
        },
        .integer, .float => {
            if (unit) |u| list.print(allocator, "{s}: {d}{s};", .{ css_name, jsonInt(value), u }) catch {} else list.print(allocator, "{s}: {d};", .{ css_name, jsonInt(value) }) catch {};
        },
        else => std.debug.print("ND_WARN unsupported style value type for \"{s}\"\n", .{css_name}),
    }
}

/// int -> `{css_name}: Npx;` (all sides); per-side object -> four values.
fn emitSpacingCss(list: *std.ArrayList(u8), allocator: std.mem.Allocator, css_name: []const u8, value: std.json.Value) void {
    switch (value) {
        .integer, .float => list.print(allocator, "{s}: {d}px;", .{ css_name, jsonInt(value) }) catch {},
        .object => |o| {
            const top = jsonInt(o.get("top") orelse std.json.Value{ .integer = 0 });
            const right = jsonInt(o.get("right") orelse std.json.Value{ .integer = 0 });
            const bottom = jsonInt(o.get("bottom") orelse std.json.Value{ .integer = 0 });
            const left = jsonInt(o.get("left") orelse std.json.Value{ .integer = 0 });
            list.print(allocator, "{s}: {d}px {d}px {d}px {d}px;", .{ css_name, top, right, bottom, left }) catch {};
        },
        else => std.debug.print("ND_WARN unsupported spacing value for \"{s}\"\n", .{css_name}),
    }
}

/// Walks `generated.style_subkeys` for `parent == key`, emitting each present
/// sub-field as a CSS declaration. Returns true if any field was emitted
/// (used by the caller to decide whether `border-style: solid` is implied).
fn emitNested(list: *std.ArrayList(u8), allocator: std.mem.Allocator, value: std.json.Value, parent: []const u8) bool {
    if (value != .object) return false;
    var emitted = false;
    for (generated.style_subkeys) |sub| {
        if (!std.mem.eql(u8, sub.parent, parent)) continue;
        const field = value.object.get(sub.name) orelse continue;
        emitScalarCss(list, allocator, sub.css, field, sub.unit, sub.kind);
        emitted = true;
    }
    return emitted;
}

/// Pure CSS-body builder: walks the generated style tables, splitting margin
/// (a widget property) out of the CSS block. Unknown keys are simply
/// skipped here (the caller-facing `applyStyle` does the ND_WARN + error emit);
/// this half is unit-testable with no GTK display.
pub fn compileCss(allocator: std.mem.Allocator, node_id: u32, style: std.json.Value) ![]u8 {
    var list: std.ArrayList(u8) = .empty;
    errdefer list.deinit(allocator);
    try list.print(allocator, ".nd-{d} {{", .{node_id});
    if (style == .object) {
        var border_present = false;
        var it = style.object.iterator();
        while (it.next()) |entry| {
            const key = entry.key_ptr.*;
            const def = findKey(key) orelse continue; // unknown key: caller warns
            if (def.target == .widget) continue; // margin — not CSS
            if (std.mem.eql(u8, key, "border")) {
                border_present = emitNested(&list, allocator, entry.value_ptr.*, "border") or border_present;
            } else if (std.mem.eql(u8, key, "font")) {
                _ = emitNested(&list, allocator, entry.value_ptr.*, "font");
            } else if (std.mem.eql(u8, def.kind, "spacing")) {
                emitSpacingCss(&list, allocator, def.css.?, entry.value_ptr.*);
            } else if (def.css) |css_name| {
                emitScalarCss(&list, allocator, css_name, entry.value_ptr.*, def.unit, def.kind);
            }
        }
        if (border_present) try list.appendSlice(allocator, "border-style: solid;");
    }
    try list.appendSlice(allocator, "}");
    return list.toOwnedSlice(allocator);
}

/// Called from tree.apply at create AND update whenever `props.style` is
/// present. Splits the style object: unknown keys -> ND_WARN + styleError
/// event; `margin` -> widget properties; everything else -> the node's
/// scoped `.nd-<id>` CSS block.
pub fn applyStyle(widget: *gtk.Widget, node_id: u32, style: std.json.Value) void {
    if (style != .object) return;
    std.debug.assert(ready); // gtk_backend.initStyle runs before any create (mirrors initEvents)

    // 1. Reject unknown top-level keys loudly; apply margin directly to the widget.
    var it = style.object.iterator();
    while (it.next()) |entry| {
        const key = entry.key_ptr.*;
        if (findKey(key) == null) {
            std.debug.print("ND_WARN unknown style key \"{s}\" (GTK is not web CSS)\n", .{key});
            if (on_error) |f| f(node_id, key);
            continue;
        }
        if (std.mem.eql(u8, key, "margin")) {
            applyMarginSpacing(widget, entry.value_ptr.*);
        } else if (std.mem.eql(u8, key, "hexpand")) {
            if (entry.value_ptr.* == .bool) gtk.Widget.setHexpand(widget, @intFromBool(entry.value_ptr.*.bool));
        } else if (std.mem.eql(u8, key, "vexpand")) {
            if (entry.value_ptr.* == .bool) gtk.Widget.setVexpand(widget, @intFromBool(entry.value_ptr.*.bool));
        } else if (std.mem.eql(u8, key, "halign")) {
            if (parseAlign(entry.value_ptr.*)) |a| gtk.Widget.setHalign(widget, a);
        } else if (std.mem.eql(u8, key, "valign")) {
            if (parseAlign(entry.value_ptr.*)) |a| gtk.Widget.setValign(widget, a);
        }
    }

    // minWidth/minHeight share one gtk_widget_set_size_request call, so the
    // pair is read outside the loop: setting one axis must not clobber the
    // other, and -1 (GTK's "no request") is the baseline when a key drops out
    // of the style object. `size_requested` remembers which nodes carry a
    // request so dropping BOTH keys resets once instead of touching every
    // widget on every apply.
    if (style.object.get("minWidth") != null or style.object.get("minHeight") != null) {
        const min_w: c_int = if (style.object.get("minWidth")) |v| @intCast(jsonInt(v)) else -1;
        const min_h: c_int = if (style.object.get("minHeight")) |v| @intCast(jsonInt(v)) else -1;
        gtk.Widget.setSizeRequest(widget, min_w, min_h);
        size_requested.put(gpa, node_id, {}) catch {};
    } else if (size_requested.remove(node_id)) {
        gtk.Widget.setSizeRequest(widget, -1, -1);
    }

    // 2. Build the CSS half (margin excluded by compileCss's `.widget` skip).
    const css = compileCss(gpa, node_id, style) catch return;
    defer gpa.free(css);
    const css_z = gpa.dupeZ(u8, css) catch return;
    defer gpa.free(css_z);

    // 3. Install/replace this node's provider (display-level, class-scoped).
    const provider = providers.get(node_id) orelse blk: {
        const p = gtk.CssProvider.new();
        const display = gtk.Widget.getDisplay(widget);
        gtk.StyleContext.addProviderForDisplay(display, p.as(gtk.StyleProvider), 600); // STYLE_PROVIDER_PRIORITY_APPLICATION
        providers.put(gpa, node_id, p) catch {};
        var cls_buf: [32]u8 = undefined;
        const cls = std.fmt.bufPrintZ(&cls_buf, "nd-{d}", .{node_id}) catch "nd-x";
        gtk.Widget.addCssClass(widget, cls);
        break :blk p;
    };
    gtk.CssProvider.loadFromString(provider, css_z);
}

/// Reconciles `widget`'s Adwaita/GTK CSS classes against `value` (a JSON
/// array of class names, already validated by the React-side allowlist):
/// set-replace over `generated.css_class_spec`, the same allowlist the
/// React side validates `cssClasses` entries against. For each allowed
/// class, add it if present in `value`, else remove it — so classes no
/// longer requested are cleared instead of accumulating. Scoping the
/// reconcile to the allowlist means the internal `nd-<id>` class (added
/// once in `applyStyle`) and GTK's own default classes (e.g. `text-button`,
/// `toggle`) are never touched, since those aren't in the allowlist.
/// `gtk_widget_remove_css_class` on an absent class is a safe no-op.
pub fn applyCssClasses(widget: *gtk.Widget, value: std.json.Value) void {
    if (value != .array) return;
    for (generated.css_class_spec) |name| {
        var present = false;
        for (value.array.items) |item| {
            if (item == .string and std.mem.eql(u8, item.string, name)) {
                present = true;
                break;
            }
        }
        const z = gpa.dupeZ(u8, name) catch continue;
        defer gpa.free(z);
        if (present) {
            gtk.Widget.addCssClass(widget, z);
            warnMistargetedClass(widget, name);
        } else {
            gtk.Widget.removeCssClass(widget, z);
        }
    }
}

/// (widget ptr ^ class index) already warned about — one ND_WARN per
/// widget+class, not one per re-render.
var mistarget_warned: std.AutoHashMapUnmanaged(usize, void) = .empty;

/// Container-scoped libadwaita classes silently style NOTHING on the wrong
/// widget type (`.boxed-list` on a GtkBox is the canonical trap: the class
/// only targets GtkListBox). Warn once so the mistake fails loudly instead
/// of shipping as a no-op.
///
/// `navigation-sidebar` is deliberately NOT in this table. It used to be the
/// headline case, but basecss.zig now gives `box.navigation-sidebar > button`
/// libadwaita's own sidebar row chrome, so on a GtkBox the class carries real
/// semantics (AppKit peer: ndInstallSidebarTable) and warning would be wrong.
fn warnMistargetedClass(widget: *gtk.Widget, name: []const u8) void {
    const ok = if (std.mem.eql(u8, name, "boxed-list") or std.mem.eql(u8, name, "boxed-list-separate"))
        gobject.ext.isA(widget, gtk.ListBox)
    else if (std.mem.eql(u8, name, "menu"))
        gobject.ext.isA(widget, gtk.Popover)
    else if (std.mem.eql(u8, name, "inline"))
        gobject.ext.isA(widget, gtk.SearchBar)
    else
        return;
    if (ok) return;
    var idx: usize = 0;
    for (generated.css_class_spec, 0..) |n, i| {
        if (std.mem.eql(u8, n, name)) {
            idx = i;
            break;
        }
    }
    const key = @intFromPtr(widget) ^ idx;
    if (mistarget_warned.contains(key)) return;
    mistarget_warned.put(gpa, key, {}) catch {};
    const instance: *gobject.TypeInstance = @ptrCast(@alignCast(widget));
    const type_name = std.mem.span(gobject.typeNameFromInstance(instance));
    std.debug.print("ND_WARN cssClass \"{s}\" has no effect on {s} — libadwaita only styles list widgets with it (use <sourcelist>/<sourcetree>/<settingsgroup> for native list chrome)\n", .{ name, type_name });
}

test "compileCss emits scoped block, splits margin out, rejects unknown key" {
    const talloc = std.testing.allocator;
    const parsed = try std.json.parseFromSlice(std.json.Value, talloc, "{\"background\":\"#fff\",\"padding\":8,\"margin\":4,\"flex\":1}", .{});
    defer parsed.deinit();
    const css = try compileCss(talloc, 7, parsed.value);
    defer talloc.free(css);
    try std.testing.expect(std.mem.indexOf(u8, css, ".nd-7 {") != null);
    try std.testing.expect(std.mem.indexOf(u8, css, "background-color: #fff;") != null);
    try std.testing.expect(std.mem.indexOf(u8, css, "padding: 8px;") != null);
    try std.testing.expect(std.mem.indexOf(u8, css, "margin") == null); // margin is a widget prop, not CSS
}

test "compileCss: fontWeight bold emits font-weight: bold with no color-check rejection" {
    // Regression test: the hex/rgb color sanity check in emitScalarCss was
    // being applied to ALL string-valued style sub-keys, not just kind ==
    // "color" ones. fontWeight is kind "enum" ("normal"|"bold"), so "bold"
    // failed the "#"/"rgb" check and was silently dropped (plus a bogus
    // ND_WARN style color value "bold" is not a hex/rgb string).
    const talloc = std.testing.allocator;
    const parsed = try std.json.parseFromSlice(std.json.Value, talloc,
        \\{"font":{"fontWeight":"bold"}}
    , .{});
    defer parsed.deinit();
    const css = try compileCss(talloc, 9, parsed.value);
    defer talloc.free(css);
    try std.testing.expect(std.mem.indexOf(u8, css, "font-weight: bold;") != null);
}

test "compileCss emits nested font/border fields with implied border-style" {
    const talloc = std.testing.allocator;
    const parsed = try std.json.parseFromSlice(std.json.Value, talloc,
        \\{"font":{"fontSize":16,"fontWeight":"bold"},"border":{"borderWidth":2,"borderColor":"#003399","borderRadius":6}}
    , .{});
    defer parsed.deinit();
    const css = try compileCss(talloc, 3, parsed.value);
    defer talloc.free(css);
    try std.testing.expect(std.mem.indexOf(u8, css, "font-size: 16px;") != null);
    try std.testing.expect(std.mem.indexOf(u8, css, "font-weight: bold;") != null);
    try std.testing.expect(std.mem.indexOf(u8, css, "border-width: 2px;") != null);
    try std.testing.expect(std.mem.indexOf(u8, css, "border-color: #003399;") != null);
    try std.testing.expect(std.mem.indexOf(u8, css, "border-radius: 6px;") != null);
    try std.testing.expect(std.mem.indexOf(u8, css, "border-style: solid;") != null);
}
