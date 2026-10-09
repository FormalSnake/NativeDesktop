// The universal `contextMenu` prop and its `contextMenuSelected` event for the
// GTK backend. Driven from one arm above the generated kind dispatch, like the
// drag/drop trio (tools/codegen.ts, UNIVERSAL_PROP_OWNERS): a secondary-button
// GtkGestureClick on the node's widget opens the entries as the same
// GtkPopoverMenu the page's context menu uses (src/cef/gtkmenu.zig).
//
// Ordering: the props arm runs at create time, before the core calls
// connectEvents, so the node id is read back off the widget when an item is
// picked rather than captured when the gesture is installed.
const std = @import("std");
const gtk = @import("gtk");
const gio = @import("gio");
const gobject = @import("gobject");
const protocol = @import("../protocol.zig");
const gtkmenu = @import("../cef/gtkmenu.zig");

pub const EmitFn = *const fn (node_id: u32, name: []const u8, payload: protocol.EventPayload) void;

/// gtkmenu frees what it is handed with the C allocator, so the entries are
/// kept in it too.
const alloc = std.heap.c_allocator;

var emit: ?EmitFn = null;

/// The context menu on screen, if any.
var current: ?*gtkmenu.Popup = null;

const NODE_ID_KEY = "nd-ctx-node-id";
const MENU_KEY = "nd-ctx-menu";
const GESTURE_KEY = "nd-ctx-gesture";
const POPUP_KEY = "nd-ctx-popup";

/// One node's entries: gtkmenu's items, command id = index + 1, and the app's
/// id for each, which is what the event carries back.
const Menu = struct {
    items: []gtkmenu.Item,
    ids: [][:0]u8,

    fn destroy(data: ?*anyopaque) callconv(.c) void {
        const menu: *Menu = @ptrCast(@alignCast(data orelse return));
        gtkmenu.freeItems(menu.items);
        for (menu.ids) |id| gtkmenu.freeLabel(id);
        if (menu.ids.len > 0) alloc.free(menu.ids);
        alloc.destroy(menu);
    }
};

fn asObject(widget: *gtk.Widget) *gobject.Object {
    return @ptrCast(@alignCast(widget));
}

fn nodeIdOf(widget: *gtk.Widget) u32 {
    const raw = gobject.Object.getData(asObject(widget), NODE_ID_KEY) orelse return 0;
    return @intCast(@intFromPtr(raw));
}

fn menuOf(widget: *gtk.Widget) ?*Menu {
    const raw = gobject.Object.getData(asObject(widget), MENU_KEY) orelse return null;
    return @ptrCast(@alignCast(raw));
}

fn field(entry: std.json.Value, key: []const u8) ?std.json.Value {
    if (entry != .object) return null;
    return entry.object.get(key);
}

fn fieldStr(entry: std.json.Value, key: []const u8) ?[]const u8 {
    return switch (field(entry, key) orelse return null) {
        .string => |s| s,
        else => null,
    };
}

fn fieldBool(entry: std.json.Value, key: []const u8) ?bool {
    return switch (field(entry, key) orelse return null) {
        .bool => |b| b,
        else => null,
    };
}

fn dupe(text: []const u8) ?[:0]u8 {
    return alloc.dupeZ(u8, text) catch null;
}

/// The entries as GTK gets them: a separator only between two commands, so an
/// app that builds its list from optional groups never shows a stray rule.
fn build(list: []const std.json.Value) ?*Menu {
    var items: std.ArrayList(gtkmenu.Item) = .empty;
    var ids: std.ArrayList([:0]u8) = .empty;
    for (list) |entry| {
        if (fieldBool(entry, "separator") orelse false) {
            if (items.items.len > 0 and items.items[items.items.len - 1].kind != .separator) {
                const label = dupe("") orelse continue;
                items.append(alloc, .{ .label = label, .kind = .separator }) catch gtkmenu.freeLabel(label);
            }
            continue;
        }
        const id = fieldStr(entry, "id") orelse continue;
        const text = fieldStr(entry, "label") orelse continue;
        const label = dupe(text) orelse continue;
        const owned_id = dupe(id) orelse {
            gtkmenu.freeLabel(label);
            continue;
        };
        ids.append(alloc, owned_id) catch {
            gtkmenu.freeLabel(label);
            gtkmenu.freeLabel(owned_id);
            continue;
        };
        items.append(alloc, .{
            .label = label,
            .command_id = @intCast(ids.items.len),
            .enabled = fieldBool(entry, "enabled") orelse true,
            .accel = if (fieldStr(entry, "accelerator")) |a| gtkAccel(a) else null,
        }) catch {
            gtkmenu.freeLabel(label);
            continue;
        };
    }
    if (items.items.len > 0 and items.items[items.items.len - 1].kind == .separator) {
        gtkmenu.freeLabel(items.pop().?.label);
    }
    if (items.items.len == 0) {
        for (ids.items) |id| gtkmenu.freeLabel(id);
        ids.deinit(alloc);
        items.deinit(alloc);
        return null;
    }
    const menu = alloc.create(Menu) catch return null;
    menu.* = .{
        .items = items.toOwnedSlice(alloc) catch &.{},
        .ids = ids.toOwnedSlice(alloc) catch &.{},
    };
    return menu;
}

/// "primary+shift+n" as GTK's accelerator syntax, "<Control><Shift>n". Shown
/// beside the label only: a popover menu is not where the binding lives.
fn gtkAccel(spec: []const u8) ?[:0]u8 {
    var buf: std.ArrayList(u8) = .empty;
    defer buf.deinit(alloc);
    var key: []const u8 = "";
    var parts = std.mem.splitScalar(u8, spec, '+');
    while (parts.next()) |part| {
        if (std.mem.eql(u8, part, "primary") or std.mem.eql(u8, part, "ctrl")) {
            buf.appendSlice(alloc, "<Control>") catch return null;
        } else if (std.mem.eql(u8, part, "shift")) {
            buf.appendSlice(alloc, "<Shift>") catch return null;
        } else if (std.mem.eql(u8, part, "alt")) {
            buf.appendSlice(alloc, "<Alt>") catch return null;
        } else key = part;
    }
    if (key.len == 0) return null;
    buf.appendSlice(alloc, key) catch return null;
    return alloc.dupeZ(u8, buf.items) catch null;
}

fn cbPressed(gesture: *gtk.GestureClick, _: c_int, x: f64, y: f64, _: ?*anyopaque) callconv(.c) void {
    const widget = gtk.EventController.getWidget(gesture.as(gtk.EventController)) orelse return;
    // The gesture runs in the capture phase, so an ancestor with a menu hears
    // the press before a row inside it with its own. The innermost one wins.
    if (innermostMenu(widget, x, y) != widget) return;
    if (open(widget, @intFromFloat(x), @intFromFloat(y))) {
        _ = gtk.Gesture.setState(gesture.as(gtk.Gesture), .claimed);
    }
}

fn innermostMenu(widget: *gtk.Widget, x: f64, y: f64) *gtk.Widget {
    var at = gtk.Widget.pick(widget, x, y, .{}) orelse return widget;
    while (at != widget) {
        if (menuOf(at) != null) return at;
        at = gtk.Widget.getParent(at) orelse return widget;
    }
    return widget;
}

fn onAnswer(ctx: ?*anyopaque, command_id: c_int) void {
    const widget: *gtk.Widget = @ptrCast(@alignCast(ctx orelse return));
    if (gobject.Object.getData(asObject(widget), POPUP_KEY)) |raw| {
        if (current) |open_one| {
            if (@intFromPtr(open_one) == @intFromPtr(raw)) current = null;
        }
    }
    gobject.Object.setData(asObject(widget), POPUP_KEY, null);
    if (command_id <= 0) return;
    const menu = menuOf(widget) orelse return;
    const index: usize = @intCast(command_id - 1);
    if (index >= menu.ids.len) return;
    const node_id = nodeIdOf(widget);
    if (node_id == 0) return;
    if (emit) |f| f(node_id, "contextMenuSelected", .{ .text = menu.ids[index] });
}

/// Pops the node's menu up at `x`,`y` in the widget's own coordinates. False
/// when the widget carries no menu. Also the automation rightClick's route on
/// GTK, which cannot synthesise the press itself.
pub fn open(widget: *gtk.Widget, x: c_int, y: c_int) bool {
    const menu = menuOf(widget) orelse return false;
    if (gobject.Object.getData(asObject(widget), POPUP_KEY) != null) return true;
    // One context menu at a time. A real press outside a menu closes it on
    // its own; this is for one opened without a press, by the automation.
    if (current) |other| {
        current = null;
        gtkmenu.close(other);
    }
    const popup = gtkmenu.open(widget, menu.items, x, y, &onAnswer, widget) orelse return false;
    gobject.Object.setData(asObject(widget), POPUP_KEY, popup);
    current = popup;
    trace(widget, menu);
    return true;
}

/// The open menu, one line per item, for a drive: the same trace the AppKit
/// host writes (swift/Sources/NDShell/ContextMenu.swift), so one drive reads
/// both.
fn trace(widget: *gtk.Widget, menu: *const Menu) void {
    const on = std.c.getenv("NATIVE_AUTOMATION") orelse return;
    if (!std.mem.eql(u8, std.mem.span(on), "1")) return;
    const node_id = nodeIdOf(widget);
    for (menu.items, 0..) |item, index| {
        const kind = if (item.kind == .separator) "separator" else "item";
        std.debug.print("ND_CONTEXT_MENU node={d} index={d} kind={s} enabled={d} label={s}\n", .{ node_id, index, kind, @intFromBool(item.enabled), item.label });
    }
}

/// The open menu's live model, for the menuModel RPC.
pub fn openModel(widget: *gtk.Widget) ?*gio.MenuModel {
    const raw = gobject.Object.getData(asObject(widget), POPUP_KEY) orelse return null;
    const popup: *gtkmenu.Popup = @ptrCast(@alignCast(raw));
    return gtk.PopoverMenu.getMenuModel(popup.popover);
}

fn ensureGesture(widget: *gtk.Widget) void {
    if (gobject.Object.getData(asObject(widget), GESTURE_KEY) != null) return;
    const click = gtk.GestureClick.new();
    gtk.GestureSingle.setButton(click.as(gtk.GestureSingle), 3);
    // Ahead of the widget's own handlers, so a button that also answers the
    // secondary button (a GtkMenuButton does) does not open two menus.
    gtk.EventController.setPropagationPhase(click.as(gtk.EventController), .capture);
    _ = gtk.GestureClick.signals.pressed.connect(click, ?*anyopaque, &cbPressed, null, .{});
    gtk.Widget.addController(widget, click.as(gtk.EventController));
    gobject.Object.setData(asObject(widget), GESTURE_KEY, click);
}

fn removeGesture(widget: *gtk.Widget) void {
    const raw = gobject.Object.getData(asObject(widget), GESTURE_KEY) orelse return;
    gobject.Object.setData(asObject(widget), GESTURE_KEY, null);
    const click: *gtk.GestureClick = @ptrCast(@alignCast(raw));
    gtk.Widget.removeController(widget, click.as(gtk.EventController));
}

// ============================================================================
// Generated-dispatcher seam
// ============================================================================

/// Universal props arm, called from both `create` and `applyProps`. An empty
/// list takes the menu away. Menu nodes hand back GMenu objects rather than
/// widgets, hence the type guard.
pub fn applyProps(widget: *gtk.Widget, props: ?std.json.Value) void {
    if (!gobject.ext.isA(widget, gtk.Widget)) return;
    const v = props orelse return;
    if (v != .object) return;
    const list = switch (v.object.get("contextMenu") orelse return) {
        .array => |a| a.items,
        else => return,
    };
    if (build(list)) |menu| {
        gobject.Object.setDataFull(asObject(widget), MENU_KEY, menu, &Menu.destroy);
        ensureGesture(widget);
    } else {
        gobject.Object.setData(asObject(widget), MENU_KEY, null);
        removeGesture(widget);
    }
}

/// Universal connect arm. Records the node id the event reports against and
/// installs the emit sink once.
pub fn connectEvents(widget: *gtk.Widget, node_id: u32, emit_fn: EmitFn) void {
    emit = emit_fn;
    if (!gobject.ext.isA(widget, gtk.Widget)) return;
    gobject.Object.setData(asObject(widget), NODE_ID_KEY, @ptrFromInt(@as(usize, node_id)));
}
