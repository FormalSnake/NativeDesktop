// Chromium's context-menu model, drawn as a real GTK menu.
//
// `cef_menu_model_t` may not be referenced after `run_context_menu` returns and
// the model arrives on the CEF UI thread, so the engine copies it into the
// owned `Item` tree below and hands that to this file, which only ever runs on
// the GTK loop. Nothing here knows about CEF: the engine passes one `Answer`
// that fires exactly once, with the chosen command id or 0 for a dismissal.
//
// The menu is a GtkPopoverMenu because a GtkPopover is a GtkNative: it gets a
// GdkSurface of its own, which on X11 is an override-redirect window stacked
// above the foreign X child CEF renders into. A popover drawn inside the
// toplevel's own surface would be painted under it.
const std = @import("std");
const glib = @import("glib");
const gobject = @import("gobject");
const gio = @import("gio");
const gdk = @import("gdk");
const gtk = @import("gtk");

const alloc = std.heap.c_allocator;

/// The action-group prefix the menu model's detailed action names carry. The
/// group is inserted on the view widget the popover is parented to, which is
/// where GTK resolves them from.
const group_prefix = "ndcm";

pub const Kind = enum { command, check, radio, separator, submenu };

/// One item of the copied model. Every string is owned.
pub const Item = struct {
    label: [:0]u8,
    accel: ?[:0]u8 = null,
    command_id: c_int = -1,
    group_id: c_int = -1,
    kind: Kind = .command,
    enabled: bool = true,
    checked: bool = false,
    children: []Item = &.{},
};

/// Through the many-item pointer: a separator's label is the empty string, and
/// a zero-length slice cannot be re-sliced over its own sentinel.
pub fn freeLabel(label: [:0]u8) void {
    alloc.free(label.ptr[0 .. label.len + 1]);
}

pub fn freeItems(items: []Item) void {
    for (items) |item| {
        freeLabel(item.label);
        if (item.accel) |a| freeLabel(a);
        freeItems(item.children);
    }
    if (items.len > 0) alloc.free(items);
}

/// Called once per menu, on the GTK thread, with the chosen command id or 0.
pub const Answer = *const fn (ctx: ?*anyopaque, command_id: c_int) void;

pub const Popup = struct {
    popover: *gtk.PopoverMenu,
    /// The view the menu hangs off: where the action group lives and what the
    /// popover is positioned against.
    parent: *gtk.Widget,
    group: *gio.SimpleActionGroup,
    answer: Answer,
    ctx: ?*anyopaque,
    answered: bool = false,
    closing: bool = false,
    present_source: c_uint = 0,
};

/// Per-item activation context, freed by the closure's destroy notify when the
/// action goes with the group.
const Activation = struct {
    popup: *Popup,
    /// -1 for a radio item, whose command id rides the action target instead:
    /// one stateful action serves the whole group.
    command_id: c_int,
};

/// Builds the menu and pops it up at `x`,`y` in `parent`'s coordinate space.
/// `items` is borrowed; the caller frees it when this returns.
pub fn open(
    parent: *gtk.Widget,
    items: []const Item,
    x: c_int,
    y: c_int,
    answer: Answer,
    ctx: ?*anyopaque,
) ?*Popup {
    if (items.len == 0) return null;
    const popup = alloc.create(Popup) catch return null;
    const group = gio.SimpleActionGroup.new();
    popup.* = .{
        .popover = undefined,
        .parent = parent,
        .group = group,
        .answer = answer,
        .ctx = ctx,
    };

    const model = buildLevel(popup, items);
    const popover = gtk.PopoverMenu.newFromModelFull(model.as(gio.MenuModel), .{});
    gobject.Object.unref(model.as(gobject.Object));
    popup.popover = popover;

    gtk.Widget.insertActionGroup(parent, group_prefix, group.as(gio.ActionGroup));
    gtk.Widget.setParent(popover.as(gtk.Widget), parent);
    const popover_base = popover.as(gtk.Popover);
    gtk.Popover.setHasArrow(popover_base, 0);
    gtk.Popover.setPosition(popover_base, .bottom);
    // Chrome drops its menu from the click, not centred on it.
    gtk.Widget.setHalign(popover.as(gtk.Widget), .start);
    var rect: gdk.Rectangle = .{ .f_x = x, .f_y = y, .f_width = 1, .f_height = 1 };
    gtk.Popover.setPointingTo(popover_base, &rect);
    _ = gobject.signalConnectData(
        @ptrCast(@alignCast(popover)),
        "closed",
        @ptrCast(&cbClosed),
        popup,
        null,
        .{},
    );
    gtk.Popover.popup(popover_base);
    // A popover is not a GtkRoot: key events on its surface are dispatched to
    // the toplevel's focus widget, which is still the view, so arrow-key
    // navigation would go to the page unless the focus moves into the menu.
    _ = gtk.Widget.grabFocus(popover.as(gtk.Widget));
    // A GtkPopover is re-presented by whatever parent lays it out, and the
    // widget CEF is embedded in lays nothing out: the surface keeps the size
    // measured before the menu's own content was allocated, one section short.
    popup.present_source = glib.idleAdd(&cbPresent, popup);
    return popup;
}

fn cbPresent(data: ?*anyopaque) callconv(.c) c_int {
    const popup: *Popup = @ptrCast(@alignCast(data.?));
    popup.present_source = 0;
    if (popup.closing) return 0;
    sizeToContent(popup.popover.as(gtk.Widget));
    gtk.Popover.present(popup.popover.as(gtk.Popover));
    return 0; // G_SOURCE_REMOVE
}

/// GtkPopover presents its surface at the size it measured before the menu's
/// own content had been laid out, which leaves the last section clipped. The
/// natural size becomes the minimum so that measurement cannot come out short,
/// capped at the monitor so a long menu scrolls instead of running off it.
fn sizeToContent(popover: *gtk.Widget) void {
    var min_w: c_int = 0;
    var nat_w: c_int = 0;
    var min_h: c_int = 0;
    var nat_h: c_int = 0;
    var ignored: c_int = 0;
    gtk.Widget.measure(popover, .horizontal, -1, &min_w, &nat_w, &ignored, &ignored);
    gtk.Widget.measure(popover, .vertical, nat_w, &min_h, &nat_h, &ignored, &ignored);
    var area: gdk.Rectangle = .{ .f_x = 0, .f_y = 0, .f_width = 0, .f_height = 0 };
    if (gdk.Display.getDefault()) |display| {
        const monitors = gdk.Display.getMonitors(display);
        if (gio.ListModel.getItem(monitors, 0)) |first| {
            gdk.Monitor.getGeometry(@ptrCast(@alignCast(first)), &area);
        }
    }
    const cap: c_int = if (area.f_height > 80) area.f_height - 40 else nat_h;
    gtk.Widget.setSizeRequest(popover, nat_w, @min(nat_h, cap));
}

/// Dismisses a menu the user did not answer: the view went away, the page
/// navigated, or the host is quitting.
pub fn close(popup: *Popup) void {
    fire(popup, 0);
    gtk.Popover.popdown(popup.popover.as(gtk.Popover));
    scheduleDestroy(popup);
}

fn fire(popup: *Popup, command_id: c_int) void {
    if (popup.answered) return;
    popup.answered = true;
    popup.answer(popup.ctx, command_id);
}

fn scheduleDestroy(popup: *Popup) void {
    if (popup.closing) return;
    popup.closing = true;
    // Not inline: this runs from the popover's own "closed" handler, and
    // unparenting a widget while it is emitting is what GTK aborts on.
    _ = glib.idleAdd(&cbDestroy, popup);
}

/// The dismissal is not answered here. A picked item pops the menu down first
/// and activates its action afterwards, in the same iteration, so answering
/// "cancelled" from this handler would beat every pick to it.
fn cbClosed(_: *gtk.Popover, data: ?*anyopaque) callconv(.c) void {
    const popup: *Popup = @ptrCast(@alignCast(data orelse return));
    scheduleDestroy(popup);
}

fn cbDestroy(data: ?*anyopaque) callconv(.c) c_int {
    const popup: *Popup = @ptrCast(@alignCast(data.?));
    fire(popup, 0);
    if (popup.present_source != 0) {
        _ = glib.Source.remove(popup.present_source);
        popup.present_source = 0;
    }
    gtk.Widget.insertActionGroup(popup.parent, group_prefix, null);
    gtk.Widget.unparent(popup.popover.as(gtk.Widget));
    _ = gtk.Widget.grabFocus(popup.parent);
    gobject.Object.unref(popup.group.as(gobject.Object));
    alloc.destroy(popup);
    return 0; // G_SOURCE_REMOVE
}

// ------------------------------------------------------------------ build ----

/// One menu level. Separators become GMenu sections, which is how a GMenuModel
/// draws a rule, so a run with nothing in it disappears instead of leaving one.
fn buildLevel(popup: *Popup, items: []const Item) *gio.Menu {
    const root = gio.Menu.new();
    var section = gio.Menu.new();
    var filled: usize = 0;
    var i: usize = 0;
    while (i < items.len) {
        const item = items[i];
        if (item.kind == .separator) {
            if (filled > 0) {
                gio.Menu.appendSection(root, null, section.as(gio.MenuModel));
                gobject.Object.unref(section.as(gobject.Object));
                section = gio.Menu.new();
                filled = 0;
            }
            i += 1;
            continue;
        }
        if (item.kind == .radio) {
            i += appendRadioRun(popup, section, items[i..]);
            filled += 1;
            continue;
        }
        appendOne(popup, section, item);
        filled += 1;
        i += 1;
    }
    if (filled > 0) gio.Menu.appendSection(root, null, section.as(gio.MenuModel));
    gobject.Object.unref(section.as(gobject.Object));
    return root;
}

fn appendOne(popup: *Popup, menu: *gio.Menu, item: Item) void {
    const entry = gio.MenuItem.new(item.label.ptr, null);
    defer gobject.Object.unref(entry.as(gobject.Object));

    if (item.kind == .submenu) {
        const sub = buildLevel(popup, item.children);
        defer gobject.Object.unref(sub.as(gobject.Object));
        gio.MenuItem.setSubmenu(entry, sub.as(gio.MenuModel));
        gio.Menu.appendItem(menu, entry);
        return;
    }

    var name_buf: [32]u8 = undefined;
    const name = std.fmt.bufPrintZ(&name_buf, "i{d}", .{item.command_id}) catch return;
    const action = if (item.kind == .check)
        gio.SimpleAction.newStateful(name.ptr, null, glib.Variant.newBoolean(@intFromBool(item.checked)))
    else
        gio.SimpleAction.new(name.ptr, null);
    defer gobject.Object.unref(action.as(gobject.Object));
    if (!item.enabled) gio.SimpleAction.setEnabled(action, 0);
    connect(popup, action, item.command_id);
    gio.ActionMap.addAction(popup.group.as(gio.ActionMap), action.as(gio.Action));

    var detailed_buf: [48]u8 = undefined;
    const detailed = std.fmt.bufPrintZ(&detailed_buf, group_prefix ++ ".{s}", .{name}) catch return;
    gio.MenuItem.setActionAndTargetValue(entry, detailed.ptr, null);
    setAccel(entry, item);
    gio.Menu.appendItem(menu, entry);
}

/// A run of contiguous radio siblings is one stateful action with a string
/// target, which is what makes GTK draw radios rather than checkmarks. Returns
/// how many items of `items` the run consumed.
fn appendRadioRun(popup: *Popup, menu: *gio.Menu, items: []const Item) usize {
    var end: usize = 0;
    const group_id = items[0].group_id;
    while (end < items.len and items[end].kind == .radio and items[end].group_id == group_id) end += 1;

    var checked: c_int = items[0].command_id;
    for (items[0..end]) |item| {
        if (item.checked) {
            checked = item.command_id;
            break;
        }
    }

    var name_buf: [32]u8 = undefined;
    const name = std.fmt.bufPrintZ(&name_buf, "r{d}-{d}", .{ group_id, items[0].command_id }) catch return end;
    var state_buf: [16]u8 = undefined;
    const state = std.fmt.bufPrintZ(&state_buf, "{d}", .{checked}) catch return end;
    const type_s = glib.VariantType.new("s");
    defer glib.VariantType.free(type_s);
    const action = gio.SimpleAction.newStateful(name.ptr, type_s, glib.Variant.newString(state.ptr));
    defer gobject.Object.unref(action.as(gobject.Object));
    connect(popup, action, -1);
    gio.ActionMap.addAction(popup.group.as(gio.ActionMap), action.as(gio.Action));

    var detailed_buf: [48]u8 = undefined;
    const detailed = std.fmt.bufPrintZ(&detailed_buf, group_prefix ++ ".{s}", .{name}) catch return end;
    for (items[0..end]) |item| {
        const entry = gio.MenuItem.new(item.label.ptr, null);
        defer gobject.Object.unref(entry.as(gobject.Object));
        var target_buf: [16]u8 = undefined;
        const target = std.fmt.bufPrintZ(&target_buf, "{d}", .{item.command_id}) catch continue;
        gio.MenuItem.setActionAndTargetValue(entry, detailed.ptr, glib.Variant.newString(target.ptr));
        setAccel(entry, item);
        gio.Menu.appendItem(menu, entry);
    }
    // A disabled member would disable the whole run, so only an entirely
    // disabled group is reported insensitive.
    var any_enabled = false;
    for (items[0..end]) |item| {
        if (item.enabled) any_enabled = true;
    }
    if (!any_enabled) gio.SimpleAction.setEnabled(action, 0);
    return end;
}

fn setAccel(entry: *gio.MenuItem, item: Item) void {
    const accel = item.accel orelse return;
    gio.MenuItem.setAttributeValue(entry, "accel", glib.Variant.newString(accel.ptr));
}

fn connect(popup: *Popup, action: *gio.SimpleAction, command_id: c_int) void {
    const act = alloc.create(Activation) catch return;
    act.* = .{ .popup = popup, .command_id = command_id };
    _ = gobject.signalConnectData(
        @ptrCast(@alignCast(action)),
        "activate",
        @ptrCast(&cbActivate),
        act,
        &freeActivation,
        .{},
    );
}

fn freeActivation(data: ?*anyopaque, _: *anyopaque) callconv(.c) void {
    const act: *Activation = @ptrCast(@alignCast(data orelse return));
    alloc.destroy(act);
}

fn cbActivate(_: *gio.SimpleAction, param: ?*glib.Variant, data: ?*anyopaque) callconv(.c) void {
    const act: *Activation = @ptrCast(@alignCast(data orelse return));
    var command_id = act.command_id;
    if (command_id < 0) {
        const target = param orelse return;
        var len: usize = 0;
        const text = glib.Variant.getString(target, &len);
        command_id = std.fmt.parseInt(c_int, text[0..len], 10) catch return;
    }
    fire(act.popup, command_id);
    gtk.Popover.popdown(act.popup.popover.as(gtk.Popover));
}
