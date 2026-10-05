// AdwDialog surface for the <commandpalette> widget: a floating command bar
// over the window (a borderless GtkSearchEntry over a GtkListBox of one-line
// rows). Peer of swift/Sources/NDShell/CommandPalette.swift. CONTROLLED: the
// app owns `query` and `items`; the widget never filters or reorders. Every
// keystroke fires queryChanged with the text the user typed and the app feeds
// back the next result set. A present always starts from the last `query` the
// app set, never from what was typed into an earlier present. The tracked
// handle is a host-only GtkBox that lives in the tree only so
// gtk_widget_get_root resolves the window to present over; the AdwDialog
// itself is presented, never packed.
//
// Highlight is internal (Up/Down/Home/End clamp within the current results);
// onActivate carries the highlighted/clicked row's stable id, onSubmit the
// typed text (Enter on no highlight, or Ctrl+Enter regardless) so a directory
// picker can accept a typed path that matches no listed row.
//
// Inline autocompletion: the first row's `completion`, when it extends what
// was typed and the last edit inserted text, is shown selected after the
// caret. It never reaches queryChanged until Tab or Right accepts it.
//
// Placement: AdwDialog is still what presents it, because a visible dialog is
// what tells the engine to cut the Chromium page (an X11 child window no
// in-window widget can draw over) around the card, and it brings the modal
// focus handling. But its own sheet is made transparent and window-sized, and
// the card inside it is placed the way the AppKit card is: centred, its top
// edge at a fixed fraction of the window height, its height following its
// rows. The dialog's dimming layer, restyled, is the scrim over the whole
// window.
const std = @import("std");
const gtk = @import("gtk");
const gdk = @import("gdk");
const gio = @import("gio");
const glib = @import("glib");
const gobject = @import("gobject");
const adw = @import("adw");
const pango = @import("pango");
const graphene = @import("graphene");
const protocol = @import("../protocol.zig");
const ndicons = @import("icons.zig");
const dialogsurface = @import("dialogsurface.zig");
const typeahead = @import("typeahead.zig");

pub const EmitFn = *const fn (node_id: u32, name: []const u8, payload: protocol.EventPayload) void;

const STATE_KEY = "nd-command-palette-state";
const ROW_ICON_KEY = "nd-palette-icon";
const ROW_TITLE_KEY = "nd-palette-title";
const ROW_SUBTITLE_KEY = "nd-palette-subtitle";
const ROW_HINT_KEY = "nd-palette-hint";

/// Card geometry, the same numbers as NDPaletteMetrics on AppKit.
const panel_width: c_int = 640;
const row_height: c_int = 40;
const list_height: c_int = 400;
/// The card's top edge sits at this fraction of the window height, so the
/// field stays put while the list below it grows and shrinks.
const top_fraction: f64 = 0.18;
/// Clearance the card keeps from every window edge.
const margin: c_int = 20;
const icon_side: c_int = 16;
/// Characters a title may take before it ellipsizes, so a long page title
/// still leaves room to see which site it is on.
const title_max_chars: c_int = 44;

var emit: ?EmitFn = null;

const alloc = std.heap.page_allocator;

const State = struct {
    node_id: u32 = 0,
    handle: *gtk.Widget,
    dialog: *adw.Dialog,
    /// The transparent, window-sized layer inside the dialog's sheet.
    layer: *gtk.Widget,
    /// The visible panel.
    card: *gtk.Widget,
    tick: c_uint = 0,
    /// Follows the card while the bar is up, so the page the engine leaves
    /// under it is cut to the card's current size (`cbWatchTick`).
    watch: c_uint = 0,
    watched: [4]f32 = .{ 0, 0, 0, 0 },
    /// Resize notifications on `return_window`, dropped with it.
    resize_hids: [2]c_ulong = .{ 0, 0 },
    entry: *gtk.SearchEntry,
    separator: *gtk.Widget,
    list: *gtk.ListBox,
    scroller: *gtk.ScrolledWindow,
    ids: std.ArrayListUnmanaged([]u8) = .empty,
    // Content signature of the currently-rendered rows. A controlled app
    // re-renders on every state change and hands back a fresh `items` array
    // each time; without this, applyProps would tear down and rebuild the
    // ListBox on every render, dropping clicks onto rows being destroyed and
    // yanking keyboard focus off the search entry. Rebuild only when the
    // signature actually changes.
    items_sig: ?[]u8 = null,
    /// The first row's `completion`, if it has one.
    first_completion: ?[]u8 = null,
    /// The app's last `query`. Every present starts from it.
    controlled: []u8 = &.{},
    /// What the user typed, without any completion shown after it.
    typed: []u8 = &.{},
    /// The completion currently drawn selected after `typed`.
    suffix: []u8 = &.{},
    /// Set by an edit that inserted text, cleared by one that removed text:
    /// Backspace over a completion must not bring it straight back.
    may_complete: bool = false,
    /// Pending completion idle, removed if the handle dies first.
    completion_idle: c_uint = 0,
    /// What had focus when the palette presented, and its window.
    return_focus: ?*gtk.Widget = null,
    return_window: ?*gtk.Window = null,
    highlight: i32 = -1,
    presented: bool = false,
    pending_open: bool = false,
    programmatic_close: bool = false,
    search_changed_hid: c_ulong = 0,
};

fn asObj(p: anytype) *gobject.Object {
    return @ptrCast(@alignCast(p));
}

fn stateOf(widget: *gtk.Widget) ?*State {
    const raw = gobject.Object.getData(asObj(widget), STATE_KEY) orelse return null;
    return @ptrCast(@alignCast(raw));
}

fn propArray(props: ?std.json.Value, key: []const u8) ?std.json.Array {
    const v = props orelse return null;
    if (v != .object) return null;
    return switch (v.object.get(key) orelse return null) {
        .array => |a| a,
        else => null,
    };
}

fn propStr(props: ?std.json.Value, key: []const u8) ?[]const u8 {
    const v = props orelse return null;
    if (v != .object) return null;
    return switch (v.object.get(key) orelse return null) {
        .string => |s| s,
        else => null,
    };
}

fn propBool(props: ?std.json.Value, key: []const u8) ?bool {
    const v = props orelse return null;
    if (v != .object) return null;
    return switch (v.object.get(key) orelse return null) {
        .bool => |b| b,
        else => null,
    };
}

fn objStr(obj: std.json.ObjectMap, key: []const u8) ?[]const u8 {
    return switch (obj.get(key) orelse return null) {
        .string => |s| s,
        else => null,
    };
}

/// Replaces an owned string field with a copy of `value`.
fn setOwned(field: *[]u8, value: []const u8) void {
    const copy = alloc.dupe(u8, value) catch return;
    if (field.*.len > 0) alloc.free(field.*);
    field.* = copy;
}

fn entryText(state: *State) []const u8 {
    return std.mem.span(gtk.Editable.getText(state.entry.as(gtk.Editable)));
}

fn charCount(s: []const u8) c_int {
    return @intCast(std.unicode.utf8CountCodepoints(s) catch s.len);
}

/// UTF-16 length of the first `chars` code points of `s`: the unit the
/// automation surface reports selections in, matching AppKit's NSRange.
fn utf16Len(s: []const u8, chars: c_int) i32 {
    var view = std.unicode.Utf8View.init(s) catch return chars;
    var it = view.iterator();
    var n: i32 = 0;
    var i: c_int = 0;
    while (i < chars) : (i += 1) {
        const cp = it.nextCodepoint() orelse break;
        n += if (cp > 0xFFFF) 2 else 1;
    }
    return n;
}

/// Writes the entry without echoing it back as queryChanged.
fn writeEntry(state: *State, text: []const u8) void {
    const z = alloc.dupeZ(u8, text) catch return;
    defer alloc.free(z);
    if (state.search_changed_hid != 0) gobject.signalHandlerBlock(asObj(state.entry), state.search_changed_hid);
    gtk.Editable.setText(state.entry.as(gtk.Editable), z);
    if (state.search_changed_hid != 0) gobject.signalHandlerUnblock(asObj(state.entry), state.search_changed_hid);
}

fn emitQuery(state: *State, text: []const u8) void {
    if (emit) |f| f(state.node_id, "queryChanged", .{ .text = text });
}

// ---- rows ------------------------------------------------------------------

fn freeIds(state: *State) void {
    for (state.ids.items) |id| alloc.free(id);
    state.ids.clearRetainingCapacity();
}

/// Content fingerprint of `arr`, used to skip the destructive ListBox rebuild
/// when a re-render hands back rows that render identically. Caller owns the
/// returned slice.
fn itemsSignature(arr: ?std.json.Array) ?[]u8 {
    var buf: std.ArrayListUnmanaged(u8) = .empty;
    errdefer buf.deinit(alloc);
    if (arr) |items| {
        for (items.items) |it| {
            if (it != .object) continue;
            inline for (.{ "id", "title", "subtitle", "iconName", "iconData", "hint", "completion" }) |key| {
                buf.appendSlice(alloc, objStr(it.object, key) orelse "") catch return null;
                buf.append(alloc, 0x1f) catch return null;
            }
            buf.append(alloc, 0x1e) catch return null;
        }
    }
    return buf.toOwnedSlice(alloc) catch null;
}

/// Re-assert keyboard focus on the search entry after a rebuild, but only when
/// focus has actually drifted off it: a fresh row set (or a churn tick under a
/// sheet-mode dialog) moves the toplevel focus onto an internal list widget,
/// which starves the capture-phase key controller of Return. Skipping when the
/// entry already owns focus keeps normal typing (which re-grabs would disrupt
/// by reselecting the text) untouched.
fn refocusEntry(state: *State) void {
    const entry_w = state.entry.as(gtk.Widget);
    if (gtk.Widget.getRoot(entry_w)) |root| {
        const win: *gtk.Window = @ptrCast(@alignCast(root));
        if (gtk.Window.getFocus(win)) |fw| {
            if (fw == entry_w or gtk.Widget.isAncestor(fw, entry_w) != 0) return;
        }
    }
    _ = gtk.Widget.grabFocus(entry_w);
}

fn rowLabel(text: [:0]const u8, dimmed: bool, ellipsize: bool) *gtk.Label {
    const label = gtk.Label.new(text);
    gtk.Label.setXalign(label, 0.0);
    gtk.Label.setSingleLineMode(label, 1);
    // Data, not markup: a URL's & or < must render as itself.
    gtk.Label.setUseMarkup(label, 0);
    if (ellipsize) gtk.Label.setEllipsize(label, .end);
    gtk.Widget.setValign(label.as(gtk.Widget), .baseline_center);
    if (dimmed) gtk.Widget.addCssClass(label.as(gtk.Widget), "dimmed");
    return label;
}

/// One line: a 16px icon, the title, the subtitle dimmed after it, and the
/// hint dimmed at the trailing edge. The subtitle gives way first, then the
/// title; the hint never ellipsizes. An icon-less row keeps an empty 16px slot
/// so titles line up down the list.
fn buildRow(obj: std.json.ObjectMap, dupeZ: *const fn ([]const u8) [:0]const u8) *gtk.ListBoxRow {
    const row = gtk.ListBoxRow.new();
    gtk.Widget.setFocusable(row.as(gtk.Widget), 0); // keyboard stays on the entry; see the ListBox setup in create()
    const line = gtk.Box.new(.horizontal, 0);
    gtk.Widget.setValign(line.as(gtk.Widget), .center);

    var icon: ?*gtk.Image = null;
    if (objStr(obj, "iconData")) |d| {
        if (d.len > 0) icon = ndicons.imageFromData(d, "CommandPalette");
    }
    if (icon == null) {
        if (objStr(obj, "iconName")) |ic| {
            if (ic.len > 0) icon = gtk.Image.newFromIconName(ndicons.symbolic(dupeZ(ic)));
        }
    }
    const img = icon orelse gtk.Image.new();
    gtk.Image.setPixelSize(img, icon_side);
    gtk.Widget.setSizeRequest(img.as(gtk.Widget), icon_side, icon_side);
    gtk.Widget.setValign(img.as(gtk.Widget), .center);
    gtk.Widget.setHalign(img.as(gtk.Widget), .center);
    gtk.Widget.setMarginEnd(img.as(gtk.Widget), 10);
    if (icon == null) gtk.Widget.setVisible(img.as(gtk.Widget), 0);
    // A hidden image gives up its width; the placeholder keeps the column.
    const slot = gtk.Box.new(.horizontal, 0);
    gtk.Widget.setSizeRequest(slot.as(gtk.Widget), icon_side + 10, icon_side);
    gtk.Widget.setValign(slot.as(gtk.Widget), .center);
    gtk.Box.append(slot, img.as(gtk.Widget));
    gtk.Box.append(line, slot.as(gtk.Widget));

    const title = rowLabel(dupeZ(objStr(obj, "title") orelse ""), false, true);
    gtk.Label.setMaxWidthChars(title, title_max_chars);
    gtk.Box.append(line, title.as(gtk.Widget));

    const sub_text = objStr(obj, "subtitle") orelse "";
    const subtitle = rowLabel(dupeZ(sub_text), true, true);
    gtk.Widget.setHexpand(subtitle.as(gtk.Widget), 1);
    gtk.Widget.setMarginStart(subtitle.as(gtk.Widget), 8);
    if (sub_text.len == 0) gtk.Widget.setVisible(subtitle.as(gtk.Widget), 0);
    gtk.Box.append(line, subtitle.as(gtk.Widget));

    const hint_text = objStr(obj, "hint") orelse "";
    const hint = rowLabel(dupeZ(hint_text), true, false);
    gtk.Widget.setMarginStart(hint.as(gtk.Widget), 16);
    gtk.Widget.setHalign(hint.as(gtk.Widget), .end);
    if (hint_text.len == 0) gtk.Widget.setVisible(hint.as(gtk.Widget), 0);
    // With no subtitle nothing else expands, and the hint must still sit on
    // the trailing edge.
    if (sub_text.len == 0) gtk.Widget.setHexpand(hint.as(gtk.Widget), 1);
    gtk.Box.append(line, hint.as(gtk.Widget));

    gtk.ListBoxRow.setChild(row, line.as(gtk.Widget));
    gobject.Object.setData(asObj(row), ROW_ICON_KEY, if (icon != null) img else null);
    gobject.Object.setData(asObj(row), ROW_TITLE_KEY, title);
    gobject.Object.setData(asObj(row), ROW_SUBTITLE_KEY, if (sub_text.len > 0) subtitle else null);
    gobject.Object.setData(asObj(row), ROW_HINT_KEY, if (hint_text.len > 0) hint else null);

    var parts: [3][]const u8 = undefined;
    var n: usize = 0;
    for ([_][]const u8{ objStr(obj, "title") orelse "", sub_text, hint_text }) |p| {
        if (p.len == 0) continue;
        parts[n] = p;
        n += 1;
    }
    const joined = std.mem.join(alloc, ", ", parts[0..n]) catch null;
    if (joined) |j| {
        defer alloc.free(j);
        // Variadic: the -1 ends the property list. Without it GTK reads past
        // the label into garbage and wedges the main loop.
        gtk.Accessible.updateProperty(row.as(gtk.Accessible), .label, dupeZ(j).ptr, @as(c_int, -1));
    }
    return row;
}

fn rebuildRows(state: *State, arr: ?std.json.Array, dupeZ: *const fn ([]const u8) [:0]const u8) void {
    const new_sig = itemsSignature(arr);
    if (state.items_sig) |old| {
        if (new_sig) |ns| {
            if (std.mem.eql(u8, old, ns)) {
                alloc.free(ns);
                if (state.presented) scheduleCompletion(state);
                return; // rows render identically: no teardown, keep highlight/focus
            }
        }
    }
    if (state.items_sig) |old| alloc.free(old);
    state.items_sig = new_sig;
    if (state.first_completion) |c| alloc.free(c);
    state.first_completion = null;

    gtk.ListBox.removeAll(state.list);
    freeIds(state);
    if (arr) |items| {
        for (items.items) |it| {
            if (it != .object) continue;
            if (state.ids.items.len == 0) {
                if (objStr(it.object, "completion")) |c| {
                    if (c.len > 0) state.first_completion = alloc.dupe(u8, c) catch null;
                }
            }
            const row = buildRow(it.object, dupeZ);
            gtk.ListBox.append(state.list, row.as(gtk.Widget));
            const id_copy = alloc.dupe(u8, objStr(it.object, "id") orelse "") catch continue;
            state.ids.append(alloc, id_copy) catch alloc.free(id_copy);
        }
    }
    const empty = state.ids.items.len == 0;
    gtk.Widget.setVisible(state.separator, @intFromBool(!empty));
    gtk.Widget.setVisible(state.scroller.as(gtk.Widget), @intFromBool(!empty));
    // Fresh results: the top row is the highlighted default (Enter drills in).
    setHighlight(state, if (!empty) 0 else -1);
    if (state.presented) {
        refocusEntry(state);
        scheduleCompletion(state);
    }
}

fn scrollToRow(state: *State, row: *gtk.ListBoxRow) void {
    var rect: graphene.Rect = undefined;
    if (gtk.Widget.computeBounds(row.as(gtk.Widget), state.list.as(gtk.Widget), &rect) == 0) return;
    const adj = gtk.ScrolledWindow.getVadjustment(state.scroller);
    const top: f64 = rect.f_origin.f_y;
    gtk.Adjustment.clampPage(adj, top, top + rect.f_size.f_height);
}

fn setHighlight(state: *State, idx: i32) void {
    state.highlight = idx;
    if (idx >= 0) {
        if (gtk.ListBox.getRowAtIndex(state.list, idx)) |row| {
            gtk.ListBox.selectRow(state.list, row);
            scrollToRow(state, row);
        }
    } else {
        gtk.ListBox.unselectAll(state.list);
    }
}

// ---- inline completion -----------------------------------------------------

/// Deferred to an idle: writing the entry from inside its own `changed`
/// handler re-enters GtkText mid-insert.
fn scheduleCompletion(state: *State) void {
    if (state.completion_idle != 0) return;
    state.completion_idle = glib.idleAdd(&completionIdle, state);
}

fn completionIdle(data: ?*anyopaque) callconv(.c) c_int {
    const state: *State = @ptrCast(@alignCast(data.?));
    state.completion_idle = 0;
    if (state.presented) applyCompletion(state);
    return 0; // G_SOURCE_REMOVE
}

/// Draws the first row's completion after the typed text, or takes a stale
/// one away. Runs after every edit (against the rows already shown, so the
/// completion keeps up with fast typing) and again when new rows land.
fn applyCompletion(state: *State) void {
    var next: []const u8 = "";
    if (state.may_complete and state.typed.len > 0) {
        if (state.first_completion) |full| {
            if (full.len > state.typed.len and std.ascii.eqlIgnoreCase(full[0..state.typed.len], state.typed) and
                std.unicode.utf8ValidateSlice(full[state.typed.len..]))
            {
                next = full[state.typed.len..];
            }
        }
    }
    const shown = entryText(state);
    if (std.mem.eql(u8, next, state.suffix) and shown.len == state.typed.len + state.suffix.len and
        std.mem.startsWith(u8, shown, state.typed) and std.mem.endsWith(u8, shown, state.suffix)) return;
    setOwned(&state.suffix, next);
    const whole = std.mem.concat(alloc, u8, &.{ state.typed, state.suffix }) catch return;
    defer alloc.free(whole);
    writeEntry(state, whole);
    gtk.Editable.selectRegion(state.entry.as(gtk.Editable), charCount(state.typed), -1);
}

/// Tab or Right over a completion: it becomes typed text.
fn acceptCompletion(state: *State) bool {
    if (state.suffix.len == 0) return false;
    const whole = std.mem.concat(alloc, u8, &.{ state.typed, state.suffix }) catch return false;
    if (state.typed.len > 0) alloc.free(state.typed);
    state.typed = whole;
    setOwned(&state.suffix, "");
    state.may_complete = false;
    gtk.Editable.setPosition(state.entry.as(gtk.Editable), -1);
    emitQuery(state, state.typed);
    return true;
}

/// One path for every change of the typed text, from a keystroke or from
/// automation, so both see the same completion rules. An edit that only took
/// the completion away (Backspace over it, or the delete half of typing over
/// the selection) leaves the typed text as it was and is not re-announced.
fn userEdited(state: *State, text: []const u8) void {
    state.may_complete = charCount(text) > charCount(state.typed);
    const same = std.mem.eql(u8, text, state.typed);
    setOwned(&state.typed, text);
    setOwned(&state.suffix, "");
    if (!same) emitQuery(state, text);
    if (state.may_complete) scheduleCompletion(state);
}

// ---- present / dismiss -----------------------------------------------------

/// Runs `f` with libadwaita's own dialog animation skipped: its floating
/// sheet springs in from 0.8 around the window centre, and the palette opens
/// and closes without motion. An AdwAnimation started while
/// `gtk-enable-animations` is off jumps to its end, so the setting is off for
/// exactly the call that starts it.
fn withoutAdwAnimation(f: *const fn (*State) void, state: *State) void {
    const settings = gtk.Settings.getDefault() orelse return f(state);
    var was = gobject.ext.Value.newFrom(true);
    defer gobject.Value.unset(&was);
    gobject.Object.getProperty(asObj(settings), "gtk-enable-animations", &was);
    if (gobject.Value.getBoolean(&was) == 0) return f(state);
    var off = gobject.ext.Value.newFrom(false);
    defer gobject.Value.unset(&off);
    gobject.Object.setProperty(asObj(settings), "gtk-enable-animations", &off);
    f(state);
    gobject.Object.setProperty(asObj(settings), "gtk-enable-animations", &was);
}

fn presentDialog(state: *State) void {
    const win = state.return_window orelse return;
    dialogsurface.present(state.dialog, win.as(gtk.Widget));
}

fn closeDialog(state: *State) void {
    _ = adw.Dialog.forceClose(state.dialog);
}

fn present(state: *State) void {
    if (state.presented) return;
    const root = gtk.Widget.getRoot(state.handle) orelse {
        state.pending_open = true; // not rooted yet: cbHandleMapped presents
        return;
    };
    // Present over the application's active window (the visible window/tab),
    // not merely the handle's own root, so the overlay is modal over whatever
    // the user is looking at regardless of where the handle sits in the tree.
    // Fall back to the handle's root when there's no active window.
    const parent_win: *gtk.Window = blk: {
        const win: *gtk.Window = @ptrCast(@alignCast(root));
        if (gtk.Window.getApplication(win)) |app| {
            if (gtk.Application.getActiveWindow(app)) |active| break :blk active;
        }
        break :blk win;
    };
    {
        releaseReturnFocus(state);
        if (gtk.Window.getFocus(parent_win)) |fw| {
            _ = gobject.Object.ref(asObj(fw));
            state.return_focus = fw;
        }
        _ = gobject.Object.ref(asObj(parent_win));
        state.return_window = parent_win;
        state.resize_hids = .{
            gobject.signalConnectData(asObj(parent_win), "notify::default-width", @ptrCast(&cbWindowResized), state, null, .{}),
            gobject.signalConnectData(asObj(parent_win), "notify::default-height", @ptrCast(&cbWindowResized), state, null, .{}),
        };
    }

    setOwned(&state.typed, state.controlled);
    setOwned(&state.suffix, "");
    state.may_complete = false;
    writeEntry(state, state.controlled);
    adw.Dialog.setContentWidth(state.dialog, gtk.Widget.getWidth(parent_win.as(gtk.Widget)));
    adw.Dialog.setContentHeight(state.dialog, gtk.Widget.getHeight(parent_win.as(gtk.Widget)));
    withoutAdwAnimation(&presentDialog, state);
    _ = place(state);
    startPlacing(state);
    state.watched = .{ 0, 0, 0, 0 };
    if (state.watch == 0) state.watch = gtk.Widget.addTickCallback(state.card, &cbWatchTick, state, null);
    state.presented = true;
    state.pending_open = false;
    typeahead.wire(parent_win);
    // Grabbing focus selects the entry's whole text, which is what a seeded
    // open (the current address) wants.
    _ = gtk.Widget.grabFocus(state.entry.as(gtk.Widget));
    // The widget that now holds the focus (the entry's inner text) is realized
    // here rather than on the dialog's first frame: GTK swallows a key whose
    // path to the focus widget crosses an unrealized widget, and the keys typed
    // straight after the chord that opened the bar were lost.
    if (gtk.Window.getFocus(parent_win)) |focus| gtk.Widget.realize(focus);
    selectAllFromStart(state);
    setHighlight(state, if (state.ids.items.len > 0) 0 else -1);
}

/// Everything selected with the caret at the start, so a long seeded address
/// reads from its beginning (the entry scrolls to the caret, and a plain
/// select-all leaves it at the end). GtkText puts the caret on the second
/// bound.
fn selectAllFromStart(state: *State) void {
    gtk.Editable.selectRegion(state.entry.as(gtk.Editable), charCount(entryText(state)), 0);
}

/// Sizes the dialog to its window and puts the card where the AppKit card
/// sits. Returns whether anything had to move, so the tick that drives it
/// can stop once the layout has settled.
fn place(state: *State) bool {
    const root = gtk.Widget.getRoot(state.layer) orelse return true;
    const win = root.as(gtk.Widget);
    const w = gtk.Widget.getWidth(win);
    const h = gtk.Widget.getHeight(win);
    if (w <= 0 or h <= 0) return true;
    var moved = false;
    if (adw.Dialog.getContentWidth(state.dialog) != w) {
        adw.Dialog.setContentWidth(state.dialog, w);
        moved = true;
    }
    if (adw.Dialog.getContentHeight(state.dialog) != h) {
        adw.Dialog.setContentHeight(state.dialog, h);
        moved = true;
    }

    const card_w = @max(0, @min(panel_width, w - 2 * margin));
    var cur_w: c_int = 0;
    gtk.Widget.getSizeRequest(state.card, &cur_w, null);
    if (cur_w != card_w) {
        gtk.Widget.setSizeRequest(state.card, card_w, -1);
        moved = true;
    }

    // The layer may not start at the window's origin (the sheet keeps its
    // own inset); the card's margin is what is left to reach the target.
    var rect: graphene.Rect = undefined;
    const layer_y: c_int = if (gtk.Widget.computeBounds(state.layer, win, &rect) != 0) @intFromFloat(@round(rect.f_origin.f_y)) else 0;
    const target: c_int = @max(margin, @as(c_int, @intFromFloat(@round(@as(f64, @floatFromInt(h)) * top_fraction))));
    const top = @max(0, target - layer_y);
    if (gtk.Widget.getMarginTop(state.card) != top) {
        gtk.Widget.setMarginTop(state.card, top);
        moved = true;
    }
    return moved;
}

/// Runs from a present (or a window resize) until the card has stopped
/// moving: the layer's own offset is only known after a layout pass.
fn cbPlaceTick(_: *gtk.Widget, _: *gdk.FrameClock, data: ?*anyopaque) callconv(.c) c_int {
    const state: *State = @ptrCast(@alignCast(data.?));
    if (place(state)) return 1; // G_SOURCE_CONTINUE
    state.tick = 0;
    return 0; // G_SOURCE_REMOVE
}

fn startPlacing(state: *State) void {
    if (state.tick == 0) state.tick = gtk.Widget.addTickCallback(state.layer, &cbPlaceTick, state, null);
}

fn cbWindowResized(_: *gobject.Object, _: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
    const state: *State = @ptrCast(@alignCast(data.?));
    if (state.presented) startPlacing(state);
}

/// Every frame while the bar is up: the card grows and shrinks with its rows,
/// and the engine has to hear of each change to cut the page under it.
fn cbWatchTick(card: *gtk.Widget, _: *gdk.FrameClock, data: ?*anyopaque) callconv(.c) c_int {
    const state: *State = @ptrCast(@alignCast(data.?));
    const root = gtk.Widget.getRoot(card) orelse return 1;
    const win: *gtk.Widget = @ptrCast(@alignCast(root));
    var rect: graphene.Rect = undefined;
    if (gtk.Widget.computeBounds(card, win, &rect) == 0) return 1;
    const now: [4]f32 = .{ rect.f_origin.f_x, rect.f_origin.f_y, rect.f_size.f_width, rect.f_size.f_height };
    if (std.mem.eql(f32, &now, &state.watched)) return 1;
    state.watched = now;
    dialogsurface.refresh(win);
    return 1;
}

fn stopWatching(state: *State) void {
    if (state.watch != 0) gtk.Widget.removeTickCallback(state.card, state.watch);
    state.watch = 0;
}

fn stopPlacing(state: *State) void {
    if (state.tick != 0) gtk.Widget.removeTickCallback(state.layer, state.tick);
    state.tick = 0;
}

fn cbLayerPressed(gesture: *gtk.GestureClick, _: c_int, x: f64, y: f64, state: *State) callconv(.c) void {
    var rect: graphene.Rect = undefined;
    if (gtk.Widget.computeBounds(state.card, state.layer, &rect) != 0) {
        const inside = x >= rect.f_origin.f_x and x < rect.f_origin.f_x + rect.f_size.f_width and
            y >= rect.f_origin.f_y and y < rect.f_origin.f_y + rect.f_size.f_height;
        if (inside) return;
    }
    _ = gtk.Gesture.setState(gesture.as(gtk.Gesture), .claimed);
    dismiss(state, false);
}

fn releaseReturnFocus(state: *State) void {
    if (state.return_window) |w| {
        for (state.resize_hids) |hid| {
            if (hid != 0) gobject.signalHandlerDisconnect(asObj(w), hid);
        }
    }
    state.resize_hids = .{ 0, 0 };
    if (state.return_focus) |w| gobject.Object.unref(asObj(w));
    if (state.return_window) |w| gobject.Object.unref(asObj(w));
    state.return_focus = null;
    state.return_window = null;
}

/// Focus goes back where it was (the page, usually); a widget that left that
/// window meanwhile leaves focus with the window.
fn restoreFocus(state: *State) void {
    const w = state.return_focus orelse return releaseReturnFocus(state);
    const win = state.return_window orelse return releaseReturnFocus(state);
    if (gtk.Widget.getRoot(w)) |r| {
        if (@as(*gtk.Window, @ptrCast(@alignCast(r))) == win) _ = gtk.Widget.grabFocus(w);
    }
    releaseReturnFocus(state);
}

fn dismiss(state: *State, programmatic: bool) void {
    state.pending_open = false;
    if (!state.presented) return;
    state.programmatic_close = programmatic;
    withoutAdwAnimation(&closeDialog, state);
}

fn cbCloseAttempt(_: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const state: *State = @ptrCast(@alignCast(data.?));
    dismiss(state, false);
}

// ---- create / applyProps ---------------------------------------------------

pub fn create(props: ?std.json.Value, dupeZ: *const fn ([]const u8) [:0]const u8) *gtk.Widget {
    const state = alloc.create(State) catch @panic("OOM CommandPalette");

    const dialog = adw.Dialog.new();
    // Own a strong ref (sink the floating one): adw_dialog_close drops the
    // window's ref, so without ours the dialog would be destroyed on the first
    // close and a re-present (open flipping true again) would use freed memory.
    _ = gobject.Object.refSink(asObj(dialog));
    adw.Dialog.setPresentationMode(dialog, .floating); // scrim, never a bottom sheet
    adw.Dialog.setContentWidth(dialog, panel_width);
    gtk.Widget.addCssClass(dialog.as(gtk.Widget), "nd-palette");

    const card = gtk.Box.new(.vertical, 0);
    gtk.Widget.addCssClass(card.as(gtk.Widget), "nd-palette-card");
    gtk.Widget.setHalign(card.as(gtk.Widget), .center);
    gtk.Widget.setValign(card.as(gtk.Widget), .start);
    gtk.Widget.setOverflow(card.as(gtk.Widget), .hidden);

    const entry = gtk.SearchEntry.new();
    gtk.Widget.addCssClass(entry.as(gtk.Widget), "nd-palette-entry");
    gtk.Widget.setMarginTop(entry.as(gtk.Widget), 10);
    gtk.Widget.setMarginBottom(entry.as(gtk.Widget), 10);
    gtk.Widget.setMarginStart(entry.as(gtk.Widget), 12);
    gtk.Widget.setMarginEnd(entry.as(gtk.Widget), 12);
    if (propStr(props, "placeholder")) |ph| gtk.SearchEntry.setPlaceholderText(entry, dupeZ(ph));

    const separator = gtk.Separator.new(.horizontal);

    const scroller = gtk.ScrolledWindow.new();
    gtk.ScrolledWindow.setPolicy(scroller, .never, .automatic);
    // The list follows its rows up to ten of them, then scrolls.
    gtk.ScrolledWindow.setPropagateNaturalHeight(scroller, 1);
    gtk.ScrolledWindow.setMaxContentHeight(scroller, list_height);
    gtk.Widget.setMarginTop(scroller.as(gtk.Widget), 6);
    gtk.Widget.setMarginBottom(scroller.as(gtk.Widget), 6);

    const list = gtk.ListBox.new();
    gtk.ListBox.setSelectionMode(list, .browse);
    gtk.ListBox.setActivateOnSingleClick(list, 1);
    gtk.Widget.addCssClass(list.as(gtk.Widget), "navigation-sidebar");
    gtk.Widget.addCssClass(list.as(gtk.Widget), "nd-palette-list");
    gtk.Widget.setValign(list.as(gtk.Widget), .start);
    // Keyboard focus stays on the search entry, always: its capture-phase key
    // controller is the ONLY keyboard route to `activate` (onKeyPressed). If the
    // list (or a row) could hold focus, a pointer click that drills into a folder
    // would move focus onto the list, and the next Return would fire GtkListBox's
    // activate-cursor-row -> row-activated -> a SECOND `activate` alongside
    // onKeyPressed's: the add-project-twice bug. A non-focusable list can't be
    // that second path; mouse activation (activate-on-single-click) and the
    // automation row-activated emit are gesture/signal driven, not focus driven,
    // so both still fire exactly once. Rows are made non-focusable in buildRow.
    gtk.Widget.setFocusable(list.as(gtk.Widget), 0);
    gtk.ScrolledWindow.setChild(scroller, list.as(gtk.Widget));

    gtk.Box.append(card, entry.as(gtk.Widget));
    gtk.Box.append(card, separator.as(gtk.Widget));
    gtk.Box.append(card, scroller.as(gtk.Widget));

    const layer = gtk.Box.new(.vertical, 0);
    gtk.Widget.setHexpand(layer.as(gtk.Widget), 1);
    gtk.Widget.setVexpand(layer.as(gtk.Widget), 1);
    gtk.Box.append(layer, card.as(gtk.Widget));
    adw.Dialog.setChild(dialog, layer.as(gtk.Widget));

    const handle = gtk.Box.new(.vertical, 0);
    state.* = .{
        .handle = handle.as(gtk.Widget),
        .dialog = dialog,
        .layer = layer.as(gtk.Widget),
        .card = card.as(gtk.Widget),
        .entry = entry,
        .separator = separator.as(gtk.Widget),
        .list = list,
        .scroller = scroller,
    };
    if (propStr(props, "query")) |q| setOwned(&state.controlled, q);
    rebuildRows(state, propArray(props, "items"), dupeZ);
    if (propBool(props, "open") orelse false) state.pending_open = true;

    // A click on the layer outside the card is a click on the page behind it.
    const click = gtk.GestureClick.new();
    _ = gtk.GestureClick.signals.pressed.connect(click, *State, &cbLayerPressed, state, .{});
    gtk.Widget.addController(layer.as(gtk.Widget), click.as(gtk.EventController));

    gobject.Object.setData(asObj(handle), STATE_KEY, state);
    _ = gobject.signalConnectData(asObj(handle), "map", @ptrCast(&cbHandleMapped), state, null, .{});
    _ = gobject.signalConnectData(asObj(handle), "destroy", @ptrCast(&cbHandleDestroyed), state, null, .{});
    return handle.as(gtk.Widget);
}

pub fn applyProps(widget: *gtk.Widget, props: ?std.json.Value, dupeZ: *const fn ([]const u8) [:0]const u8) void {
    const state = stateOf(widget) orelse return;
    if (propStr(props, "placeholder")) |ph| gtk.SearchEntry.setPlaceholderText(state.entry, dupeZ(ph));
    if (propStr(props, "query")) |q| {
        setOwned(&state.controlled, q);
        if (state.presented) {
            // A reseed while open reads like a fresh open: the whole text selected.
            setOwned(&state.typed, q);
            setOwned(&state.suffix, "");
            state.may_complete = false;
            if (!std.mem.eql(u8, entryText(state), q)) writeEntry(state, q);
            selectAllFromStart(state);
        }
    }
    if (propArray(props, "items")) |arr| rebuildRows(state, arr, dupeZ);
    if (propBool(props, "open")) |o| {
        if (o) present(state) else dismiss(state, true);
    }
}

// ---- events ----------------------------------------------------------------

pub fn connectEvents(widget: *gtk.Widget, node_id: u32, emit_fn: EmitFn) void {
    emit = emit_fn;
    const state = stateOf(widget) orelse return;
    state.node_id = node_id;

    // "changed", not "search-changed": the latter is debounced ~150ms except on
    // an empty value, which splits gtk_editable_set_text's delete and insert
    // across two turns and collapses a controlled query. Peer of the
    // SearchInput.changed entry in tools/codegen.ts.
    state.search_changed_hid = gobject.signalConnectData(asObj(state.entry), "changed", @ptrCast(&cbSearchChanged), state, null, .{});
    _ = gobject.signalConnectData(asObj(state.list), "row-activated", @ptrCast(&cbRowActivated), state, null, .{});
    _ = gobject.signalConnectData(asObj(state.dialog), "closed", @ptrCast(&cbDialogClosed), state, null, .{});
    // A click on the scrim outside the sheet would close the dialog with
    // libadwaita's own animation; it asks instead, and closes without one.
    adw.Dialog.setCanClose(state.dialog, 0);
    _ = gobject.signalConnectData(asObj(state.dialog), "close-attempt", @ptrCast(&cbCloseAttempt), state, null, .{});

    // Capture-phase key controller on the entry: intercept navigation/commit
    // keys before GtkSearchEntry consumes them (its own Esc clears the text),
    // let plain typing fall through to drive the entry's changed signal.
    const key_ctrl = gtk.EventControllerKey.new();
    gtk.EventController.setPropagationPhase(key_ctrl.as(gtk.EventController), .capture);
    _ = gtk.EventControllerKey.signals.key_pressed.connect(key_ctrl, *State, &onKeyPressed, state, .{});
    gtk.Widget.addController(state.entry.as(gtk.Widget), key_ctrl.as(gtk.EventController));
}

fn emitActivate(state: *State, idx: i32) void {
    if (idx < 0 or idx >= @as(i32, @intCast(state.ids.items.len))) return;
    if (emit) |f| f(state.node_id, "activate", .{ .text = state.ids.items[@intCast(idx)] });
}

/// The typed text, never an unaccepted completion.
fn emitSubmit(state: *State) void {
    if (emit) |f| f(state.node_id, "submit", .{ .text = if (state.presented) state.typed else state.controlled });
}

/// Enter. Row 0 whose completion the user just took away (Backspace) is not
/// what they meant: with no completion shown and the typed text short of that
/// completion, Enter submits what was typed.
fn commitReturn(state: *State) void {
    const n: i32 = @intCast(state.ids.items.len);
    if (state.highlight < 0 or state.highlight >= n) return emitSubmit(state);
    if (state.highlight == 0 and state.suffix.len == 0) {
        if (state.first_completion) |c| {
            if (!std.ascii.eqlIgnoreCase(c, state.typed)) return emitSubmit(state);
        }
    }
    emitActivate(state, state.highlight);
}

// ---- automation ------------------------------------------------------------
// The tracked node is the host box; the real entry/list live in the separately
// presented dialog, so the generic setValue/type/click dispatch (which sniffs
// the host box as a plain GtkBox) can't reach them. The backend routes those
// actions here instead, driving the same paths a user would: query text fires
// queryChanged, an integer index fires the ListBox row-activated -> onActivate,
// a bool submits, click activates the current highlight, and paletteLayout
// reads the drawn geometry back.

/// True for the palette's host box (carries the palette state); lets the
/// backend's node_visible/node_bounds/semantic_action arms special-case it.
pub fn isPaletteHandle(widget: *gtk.Widget) bool {
    return gobject.Object.getData(asObj(widget), STATE_KEY) != null;
}

/// Actionable only while presented: a closed palette has no entry/list to act
/// on, so getTree/automation treats it as not-actionable.
pub fn isPresented(widget: *gtk.Widget) bool {
    const state = stateOf(widget) orelse return false;
    return state.presented;
}

fn cpMallocZ(json: []const u8) ?[*:0]u8 {
    const buf: [*]u8 = @ptrCast(std.c.malloc(json.len + 1) orelse return null);
    @memcpy(buf[0..json.len], json);
    buf[json.len] = 0;
    return @ptrCast(buf);
}

fn cpSetResult(out: *?[*:0]u8, value: anytype) void {
    const json = std.json.Stringify.valueAlloc(alloc, value, .{}) catch return;
    defer alloc.free(json);
    out.* = cpMallocZ(json);
}

fn cpSetErr(out: *?[*:0]u8, node_id: u32) i32 {
    var buf: [48]u8 = undefined;
    const json = std.fmt.bufPrint(&buf, "{{\"ref\":{d}}}", .{node_id}) catch return -32602;
    out.* = cpMallocZ(json);
    return -32602;
}

fn activateRowByIndex(state: *State, idx: i32) void {
    const row = gtk.ListBox.getRowAtIndex(state.list, idx) orelse return;
    gobject.signalEmitByName(asObj(state.list), "row-activated", row); // -> cbRowActivated -> emitActivate
}

const Geo = struct { x: i32, y: i32, w: i32, h: i32 };

const RowLayout = struct {
    row: Geo,
    icon: ?Geo,
    title: ?Geo,
    subtitle: ?Geo,
    hint: ?Geo,
    truncated: bool,
    highlighted: bool,
};

const Layout = struct {
    ref: u32,
    presented: bool,
    window: ?Geo = null,
    panel: ?Geo = null,
    field: ?Geo = null,
    fieldText: []const u8 = "",
    selectionStart: i32 = 0,
    selectionLength: i32 = 0,
    dimmed: bool = false,
    rows: []const RowLayout = &.{},
};

fn geoIn(w: *gtk.Widget, target: *gtk.Widget) ?Geo {
    if (gtk.Widget.getMapped(w) == 0) return null;
    var rect: graphene.Rect = undefined;
    if (gtk.Widget.computeBounds(w, target, &rect) == 0) return null;
    return .{
        .x = @intFromFloat(@round(rect.f_origin.f_x)),
        .y = @intFromFloat(@round(rect.f_origin.f_y)),
        .w = @intFromFloat(@round(rect.f_size.f_width)),
        .h = @intFromFloat(@round(rect.f_size.f_height)),
    };
}

fn rowPart(row: *gtk.ListBoxRow, key: [:0]const u8) ?*gtk.Widget {
    const raw = gobject.Object.getData(asObj(row), key) orelse return null;
    return @ptrCast(@alignCast(raw));
}

fn labelEllipsized(w: ?*gtk.Widget) bool {
    const label: *gtk.Label = @ptrCast(@alignCast(w orelse return false));
    return pango.Layout.isEllipsized(gtk.Label.getLayout(label)) != 0;
}

fn layoutJson(state: *State, out: *?[*:0]u8) void {
    if (!state.presented) return cpSetResult(out, Layout{ .ref = state.node_id, .presented = false });
    const root = gtk.Widget.getRoot(state.entry.as(gtk.Widget)) orelse
        return cpSetResult(out, Layout{ .ref = state.node_id, .presented = true });
    const win = root.as(gtk.Widget);

    var rows: std.ArrayListUnmanaged(RowLayout) = .empty;
    defer rows.deinit(alloc);
    const view = geoIn(state.scroller.as(gtk.Widget), win);
    var i: c_int = 0;
    while (gtk.ListBox.getRowAtIndex(state.list, i)) |row| : (i += 1) {
        const rg = geoIn(row.as(gtk.Widget), win) orelse continue;
        // A row built since the last frame has no allocation yet: nothing of
        // it is drawn, and its labels would read as ellipsized at width 0.
        if (rg.h == 0) continue;
        if (view) |v| {
            if (rg.y + rg.h <= v.y or rg.y >= v.y + v.h) continue; // scrolled out of the list
        }
        const title = rowPart(row, ROW_TITLE_KEY);
        const subtitle = rowPart(row, ROW_SUBTITLE_KEY);
        rows.append(alloc, .{
            .row = rg,
            .icon = if (rowPart(row, ROW_ICON_KEY)) |w| geoIn(w, win) else null,
            .title = if (title) |w| geoIn(w, win) else null,
            .subtitle = if (subtitle) |w| geoIn(w, win) else null,
            .hint = if (rowPart(row, ROW_HINT_KEY)) |w| geoIn(w, win) else null,
            .truncated = labelEllipsized(title) or labelEllipsized(subtitle),
            .highlighted = i == state.highlight,
        }) catch break;
    }

    const text = entryText(state);
    var start: c_int = 0;
    var end: c_int = 0;
    if (gtk.Editable.getSelectionBounds(state.entry.as(gtk.Editable), &start, &end) == 0) {
        start = gtk.Editable.getPosition(state.entry.as(gtk.Editable));
        end = start;
    }
    const s16 = utf16Len(text, start);
    cpSetResult(out, Layout{
        .ref = state.node_id,
        .presented = true,
        .window = .{ .x = 0, .y = 0, .w = gtk.Widget.getWidth(win), .h = gtk.Widget.getHeight(win) },
        .panel = geoIn(state.card, win),
        .field = geoIn(state.entry.as(gtk.Widget), win),
        .fieldText = text,
        .selectionStart = s16,
        .selectionLength = utf16Len(text, end) - s16,
        // The dialog's dimming layer, restyled in basecss, spans the window.
        .dimmed = true,
        .rows = rows.items,
    });
}

/// Routes the palette node's setValue/type/click/paletteLayout automation
/// actions to the real entry/list. Returns 0 ok / -32001 closed / -32602
/// invalid value.
pub fn automationAction(handle: *gtk.Widget, node_id: u32, action: []const u8, args: ?std.json.Value, result_out: *?[*:0]u8, err_out: *?[*:0]u8) i32 {
    const state = stateOf(handle) orelse return cpSetErr(err_out, node_id);
    if (std.mem.eql(u8, action, "paletteLayout")) {
        layoutJson(state, result_out);
        return 0;
    }
    if (!state.presented) {
        _ = cpSetErr(err_out, node_id);
        return -32001; // not actionable while closed
    }
    const obj: ?std.json.ObjectMap = if (args) |a| (if (a == .object) a.object else null) else null;

    if (std.mem.eql(u8, action, "setValue")) {
        const value = (if (obj) |o| o.get("value") else null) orelse return cpSetErr(err_out, node_id);
        switch (value) {
            .string => |s| {
                writeEntry(state, s);
                userEdited(state, s);
            },
            .integer => |i| {
                if (i < 0 or i >= @as(i64, @intCast(state.ids.items.len))) return cpSetErr(err_out, node_id);
                activateRowByIndex(state, @intCast(i));
            },
            .bool => |b| {
                if (!b) return cpSetErr(err_out, node_id);
                emitSubmit(state);
            },
            else => return cpSetErr(err_out, node_id),
        }
        cpSetResult(result_out, .{ .ref = node_id, .applied = true });
        return 0;
    } else if (std.mem.eql(u8, action, "type")) {
        const t = (if (obj) |o| o.get("text") else null) orelse return cpSetErr(err_out, node_id);
        if (t != .string) return cpSetErr(err_out, node_id);
        const full = std.mem.concat(alloc, u8, &.{ state.typed, t.string }) catch return cpSetErr(err_out, node_id);
        defer alloc.free(full);
        writeEntry(state, full);
        userEdited(state, full);
        cpSetResult(result_out, .{ .ref = node_id, .text = full });
        return 0;
    } else if (std.mem.eql(u8, action, "click")) {
        const n: i32 = @intCast(state.ids.items.len);
        const idx: i32 = if (state.highlight >= 0 and state.highlight < n) state.highlight else if (n > 0) 0 else -1;
        if (idx >= 0) activateRowByIndex(state, idx);
        cpSetResult(result_out, .{ .ref = node_id, .dispatched = true });
        return 0;
    }
    _ = cpSetErr(err_out, node_id);
    return -32601;
}

fn cbSearchChanged(obj: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const state: *State = @ptrCast(@alignCast(data.?));
    const editable = @as(*gtk.SearchEntry, @ptrCast(@alignCast(obj))).as(gtk.Editable);
    userEdited(state, std.mem.span(gtk.Editable.getText(editable)));
}

// GtkListBox "row-activated" passes (box, row, user_data); single-click
// activation is on, so a click lands here directly.
fn cbRowActivated(_: *gobject.Object, row: *gtk.ListBoxRow, data: ?*anyopaque) callconv(.c) void {
    const state: *State = @ptrCast(@alignCast(data.?));
    const idx = gtk.ListBoxRow.getIndex(row);
    state.highlight = idx;
    emitActivate(state, idx);
}

// AdwDialog "closed" fires for user dismissal (Esc / click-outside) AND
// programmatic close; only the former reaches the app as onCancel.
fn cbDialogClosed(_: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const state: *State = @ptrCast(@alignCast(data.?));
    state.presented = false;
    stopPlacing(state);
    stopWatching(state);
    // The page under the bar takes no focus while the bar is up; the engine
    // has to hear the bar went before the focus goes back to it.
    if (state.return_window) |w| dialogsurface.refresh(w.as(gtk.Widget));
    restoreFocus(state);
    if (state.programmatic_close) {
        state.programmatic_close = false;
        return;
    }
    if (emit) |f| f(state.node_id, "cancel", .{});
}

fn onKeyPressed(_: *gtk.EventControllerKey, keyval: c_uint, _: c_uint, mods: gdk.ModifierType, state: *State) callconv(.c) c_int {
    const n: i32 = @intCast(state.ids.items.len);
    switch (keyval) {
        gdk.KEY_Up => {
            if (n > 0) setHighlight(state, if (state.highlight <= 0) 0 else state.highlight - 1);
            return 1;
        },
        gdk.KEY_Down => {
            if (n > 0) setHighlight(state, if (state.highlight < 0) 0 else if (state.highlight >= n - 1) n - 1 else state.highlight + 1);
            return 1;
        },
        gdk.KEY_Home => {
            if (n > 0) setHighlight(state, 0);
            return 1;
        },
        gdk.KEY_End => {
            if (n > 0) setHighlight(state, n - 1);
            return 1;
        },
        // Tab never leaves the entry: it accepts a completion or does nothing.
        gdk.KEY_Tab, gdk.KEY_ISO_Left_Tab, gdk.KEY_KP_Tab => {
            _ = acceptCompletion(state);
            return 1;
        },
        gdk.KEY_Right, gdk.KEY_KP_Right => return @intFromBool(acceptCompletion(state)),
        gdk.KEY_Escape => {
            dismiss(state, false);
            return 1;
        },
        gdk.KEY_Return, gdk.KEY_KP_Enter => {
            if (mods.control_mask) emitSubmit(state) else commitReturn(state);
            return 1;
        },
        else => return 0, // typing drives the entry's changed signal
    }
}

fn cbHandleMapped(_: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const state: *State = @ptrCast(@alignCast(data.?));
    if (state.pending_open) present(state);
}

fn cbHandleDestroyed(_: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const state: *State = @ptrCast(@alignCast(data.?));
    // The dialog is presented over the window, not parented under this handle,
    // so destroying the handle never reaches it: close it explicitly (else an
    // unmount-while-open leaves an orphaned dialog on screen), then drop our
    // refSink so it (and its handlers) are destroyed before `state` is freed.
    if (state.presented) {
        state.programmatic_close = true;
        _ = adw.Dialog.forceClose(state.dialog);
    }
    if (state.completion_idle != 0) _ = glib.Source.remove(state.completion_idle);
    stopPlacing(state);
    stopWatching(state);
    releaseReturnFocus(state);
    gobject.Object.unref(asObj(state.dialog));
    freeIds(state);
    state.ids.deinit(alloc);
    if (state.items_sig) |sig| alloc.free(sig);
    if (state.first_completion) |c| alloc.free(c);
    for ([_][]u8{ state.controlled, state.typed, state.suffix }) |s| {
        if (s.len > 0) alloc.free(s);
    }
    alloc.destroy(state);
}
