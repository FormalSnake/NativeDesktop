// Window chrome an app draws itself (the Arc window shape), GTK half. AppKit
// peers: NDShell/WindowControls.swift, SplitReveal.swift, ContentCard.swift,
// and NDBoxView's `windowHandle`.
//
//   <windowcontrols side>          GtkWindowControls for that side of the
//                                  desktop's decoration layout
//   <box windowHandle>             a press and drag on the box moves the
//                                  window, a double click maximizes
//   <splitview edgeReveal>         a collapsed sidebar slides in over the
//                                  content when the pointer touches the
//                                  leading edge (AdwOverlaySplitView's own
//                                  overlay), and out when it leaves
//   <splitview contentStyle=card>  the content pane is a rounded card inset on
//                                  the sidebar's surface
const std = @import("std");
const gtk = @import("gtk");
const gdk = @import("gdk");
const glib = @import("glib");
const gobject = @import("gobject");
const adw = @import("adw");
const graphene = @import("graphene");
const protocol = @import("../protocol.zig");

pub const EmitFn = *const fn (node_id: u32, name: []const u8, payload: protocol.EventPayload) void;

fn asObject(p: anytype) *gobject.Object {
    return @ptrCast(@alignCast(p));
}

fn getFlag(w: anytype, key: [*:0]const u8) bool {
    return gobject.Object.getData(asObject(w), key) != null;
}

fn setFlag(w: anytype, key: [*:0]const u8, on: bool) void {
    gobject.Object.setData(asObject(w), key, if (on) @ptrFromInt(1) else null);
}

// ---- <windowcontrols> --------------------------------------------------------

pub fn windowControlsNew(side: []const u8) *gtk.Widget {
    const pack: gtk.PackType = if (std.mem.eql(u8, side, "end")) .end else .start;
    const wc = gtk.WindowControls.new(pack);
    const w = wc.as(gtk.Widget);
    gtk.Widget.setValign(w, .center);
    return w;
}

const K_WC_NODE = "nd-window-controls-node";
var wc_emit: ?EmitFn = null;

fn wcReport(wc: *gtk.WindowControls) void {
    const raw = gobject.Object.getData(asObject(wc), K_WC_NODE) orelse return;
    const f = wc_emit orelse return;
    f(@intCast(@intFromPtr(raw)), "emptyChanged", .{ .checked = gtk.WindowControls.getEmpty(wc) != 0 });
}

fn cbWindowControlsEmpty(obj: *gobject.Object, _: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
    wcReport(@ptrCast(@alignCast(obj)));
}

/// `emptyChanged`: whether this side of the desktop's decoration layout holds
/// any buttons. Reported once at connect and again when the user changes the
/// layout, so an app can put the slot where the buttons are (Zen's rule: the
/// sidebar when they lead, a strip over the page when they trail).
pub fn connectWindowControls(w: *gtk.Widget, node_id: u32, emit_fn: EmitFn) void {
    wc_emit = emit_fn;
    gobject.Object.setData(asObject(w), K_WC_NODE, @ptrFromInt(@as(usize, node_id)));
    _ = gobject.signalConnectData(asObject(w), "notify::empty", @ptrCast(&cbWindowControlsEmpty), null, null, .{});
    wcReport(@ptrCast(@alignCast(w)));
}

/// Whether `w`'s subtree places its own window controls, which is a tree that
/// carries its own chrome (tabs.zig's `declaresOwnChrome`).
pub fn containsWindowControls(w: *gtk.Widget) bool {
    if (gobject.ext.isA(w, gtk.WindowControls)) return true;
    var it = gtk.Widget.getFirstChild(w);
    while (it) |c| : (it = gtk.Widget.getNextSibling(c)) {
        if (containsWindowControls(c)) return true;
    }
    return false;
}

// ---- <box windowHandle> ------------------------------------------------------
// GtkWindowHandle is a container, and a box's handle has to stay the GtkBox
// the tree placed, so the same two gestures GtkWindowHandle installs are put
// on the box itself. Both run in the bubble phase: a button inside the box
// claims its own press first.

const K_HANDLE = "nd-window-handle";
const K_HANDLE_WIRED = "nd-window-handle-wired";

fn toplevelOf(w: *gtk.Widget) ?*gdk.Toplevel {
    const native = gtk.Widget.getNative(w) orelse return null;
    const surface = gtk.Native.getSurface(native) orelse return null;
    if (!gobject.ext.isA(surface, gdk.Toplevel)) return null;
    return @ptrCast(@alignCast(surface));
}

/// `x`/`y` in `w`'s coordinates, as the surface coordinates begin_move takes.
fn surfacePoint(w: *gtk.Widget, x: f64, y: f64) ?struct { x: f64, y: f64 } {
    const native = gtk.Widget.getNative(w) orelse return null;
    var in = graphene.Point{ .f_x = @floatCast(x), .f_y = @floatCast(y) };
    var out: graphene.Point = undefined;
    if (gtk.Widget.computePoint(w, native.as(gtk.Widget), &in, &out) == 0) return null;
    var sx: f64 = 0;
    var sy: f64 = 0;
    gtk.Native.getSurfaceTransform(native, &sx, &sy);
    return .{ .x = out.f_x + sx, .y = out.f_y + sy };
}

fn cbHandleDragUpdate(g: *gtk.GestureDrag, dx: f64, dy: f64, data: ?*anyopaque) callconv(.c) void {
    const w: *gtk.Widget = @ptrCast(@alignCast(data.?));
    if (!getFlag(w, K_HANDLE)) return;
    // GTK's own drag threshold, so a click that wobbles a pixel stays a click.
    if (gtk.Widget.dragCheckThreshold(w, 0, 0, @intFromFloat(dx), @intFromFloat(dy)) == 0) return;
    var sx: f64 = 0;
    var sy: f64 = 0;
    _ = gtk.GestureDrag.getStartPoint(g, &sx, &sy);
    const top = toplevelOf(w) orelse return;
    const at = surfacePoint(w, sx, sy) orelse return;
    const gesture: *gtk.Gesture = @ptrCast(g);
    const device = gtk.Gesture.getDevice(gesture) orelse return;
    const button: c_int = @intCast(gtk.GestureSingle.getCurrentButton(@ptrCast(g)));
    const time = gtk.EventController.getCurrentEventTime(@ptrCast(g));
    _ = gtk.Gesture.setState(gesture, .claimed);
    gtk.EventController.reset(@ptrCast(g));
    gdk.Toplevel.beginMove(top, device, button, at.x, at.y, time);
}

fn cbHandlePressed(g: *gtk.GestureClick, n: c_int, _: f64, _: f64, data: ?*anyopaque) callconv(.c) void {
    const w: *gtk.Widget = @ptrCast(@alignCast(data.?));
    if (!getFlag(w, K_HANDLE) or n != 2) return;
    const root = gtk.Widget.getRoot(w) orelse return;
    if (!gobject.ext.isA(root, gtk.Window)) return;
    const win: *gtk.Window = @ptrCast(@alignCast(root));
    _ = gtk.Gesture.setState(@ptrCast(g), .claimed);
    if (gtk.Window.isMaximized(win) != 0) gtk.Window.unmaximize(win) else gtk.Window.maximize(win);
}

pub fn setWindowHandle(w: *gtk.Widget, on: bool) void {
    setFlag(w, K_HANDLE, on);
    if (!on or getFlag(w, K_HANDLE_WIRED)) return;
    setFlag(w, K_HANDLE_WIRED, true);
    const drag = gtk.GestureDrag.new();
    gtk.EventController.setPropagationPhase(drag.as(gtk.EventController), .bubble);
    _ = gtk.GestureDrag.signals.drag_update.connect(drag, ?*anyopaque, &cbHandleDragUpdate, w, .{});
    gtk.Widget.addController(w, drag.as(gtk.EventController));
    const click = gtk.GestureClick.new();
    gtk.EventController.setPropagationPhase(click.as(gtk.EventController), .bubble);
    _ = gtk.GestureClick.signals.pressed.connect(click, ?*anyopaque, &cbHandlePressed, w, .{});
    gtk.Widget.addController(w, click.as(gtk.EventController));
}

// ---- <splitview edgeReveal> --------------------------------------------------
// AdwOverlaySplitView already is the widget: collapsed, its sidebar is an
// overlay above the content, shown and hidden through `show-sidebar` with its
// own slide. What it lacks is the pointer trigger. The motion controller sees
// the leading edge, which the content card's margin keeps GTK-drawn; the
// leave is polled, because a pointer that goes over an embedded engine's
// native child window reports no motion to GTK at all.

const K_REVEAL = "nd-edge-reveal";
const K_REVEAL_WIRED = "nd-edge-reveal-wired";
const K_REVEAL_NODE = "nd-edge-reveal-node";
const K_REVEAL_POLL = "nd-edge-reveal-poll";
const K_REVEAL_AWAY = "nd-edge-reveal-away";

/// How near the leading edge the pointer has to be.
const reveal_edge_px: f64 = 6;
/// Slack past the sidebar's trailing edge before the pointer counts as gone.
const reveal_slack_px: f64 = 16;
/// Poll period, and how many periods away before the sidebar leaves (250 ms).
const reveal_poll_ms: c_uint = 50;
const reveal_away_ticks: usize = 5;

var reveal_emit: ?EmitFn = null;

fn splitOf(w: *gtk.Widget) *adw.OverlaySplitView {
    return @ptrCast(@alignCast(w));
}

fn revealEmit(sv: *gtk.Widget, on: bool) void {
    const raw = gobject.Object.getData(asObject(sv), K_REVEAL_NODE) orelse return;
    const f = reveal_emit orelse return;
    f(@intCast(@intFromPtr(raw)), "revealChanged", .{ .checked = on });
}

/// The sidebar is "revealed" when it is shown while collapsed: an overlay.
fn isRevealed(sv: *gtk.Widget) bool {
    const s = splitOf(sv);
    return adw.OverlaySplitView.getCollapsed(s) != 0 and adw.OverlaySplitView.getShowSidebar(s) != 0;
}

fn pointerX(sv: *gtk.Widget) ?f64 {
    const native = gtk.Widget.getNative(sv) orelse return null;
    const surface = gtk.Native.getSurface(native) orelse return null;
    const display = gtk.Widget.getDisplay(sv);
    const seat = gdk.Display.getDefaultSeat(display) orelse return null;
    const pointer = gdk.Seat.getPointer(seat) orelse return null;
    var x: f64 = 0;
    var y: f64 = 0;
    if (gdk.Surface.getDevicePosition(surface, pointer, &x, &y, null) == 0) return null;
    var sx: f64 = 0;
    var sy: f64 = 0;
    gtk.Native.getSurfaceTransform(native, &sx, &sy);
    var in = graphene.Point{ .f_x = @floatCast(x - sx), .f_y = @floatCast(y - sy) };
    var out: graphene.Point = undefined;
    if (gtk.Widget.computePoint(native.as(gtk.Widget), sv, &in, &out) == 0) return null;
    return out.f_x;
}

fn cbRevealPoll(data: ?*anyopaque) callconv(.c) c_int {
    const sv: *gtk.Widget = @ptrCast(@alignCast(data.?));
    if (!isRevealed(sv)) {
        gobject.Object.setData(asObject(sv), K_REVEAL_POLL, null);
        return 0;
    }
    const sidebar = adw.OverlaySplitView.getSidebar(splitOf(sv));
    const limit: f64 = @as(f64, @floatFromInt(if (sidebar) |sb| gtk.Widget.getWidth(sb) else 0)) + reveal_slack_px;
    const away_raw = @intFromPtr(gobject.Object.getData(asObject(sv), K_REVEAL_AWAY));
    const x = pointerX(sv) orelse limit + 1;
    const away: usize = if (x > limit) away_raw + 1 else 0;
    gobject.Object.setData(asObject(sv), K_REVEAL_AWAY, @ptrFromInt(away));
    if (away >= reveal_away_ticks) {
        adw.OverlaySplitView.setShowSidebar(splitOf(sv), 0);
        gobject.Object.setData(asObject(sv), K_REVEAL_POLL, null);
        return 0;
    }
    return 1;
}

fn startRevealPoll(sv: *gtk.Widget) void {
    if (gobject.Object.getData(asObject(sv), K_REVEAL_POLL) != null) return;
    gobject.Object.setData(asObject(sv), K_REVEAL_AWAY, null);
    const id = glib.timeoutAdd(reveal_poll_ms, &cbRevealPoll, sv);
    gobject.Object.setData(asObject(sv), K_REVEAL_POLL, @ptrFromInt(@as(usize, id)));
}

fn reveal(sv: *gtk.Widget, pointer_driven: bool) void {
    const s = splitOf(sv);
    if (adw.OverlaySplitView.getCollapsed(s) == 0) return;
    if (adw.OverlaySplitView.getSidebar(s) == null) return;
    adw.OverlaySplitView.setShowSidebar(s, 1);
    if (pointer_driven) startRevealPoll(sv);
}

fn cbRevealMotion(ctrl: *gtk.EventControllerMotion, x: f64, _: f64, _: ?*anyopaque) callconv(.c) void {
    const sv = gtk.EventController.getWidget(@ptrCast(ctrl)) orelse return;
    if (!getFlag(sv, K_REVEAL) or isRevealed(sv)) return;
    const rtl = gtk.Widget.getDirection(sv) == .rtl;
    const from_edge = if (rtl) @as(f64, @floatFromInt(gtk.Widget.getWidth(sv))) - x else x;
    if (from_edge <= reveal_edge_px) reveal(sv, true);
}

fn cbShowSidebar(obj: *gobject.Object, _: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
    const sv: *gtk.Widget = @ptrCast(@alignCast(obj));
    // Uncollapsing brings the sidebar back beside the content. libadwaita
    // only does that itself for a collapse its own breakpoint made, so a
    // sidebar the pointer revealed and then left stayed hidden once the app
    // uncollapsed the split. Deferred to idle: flipping show-sidebar from
    // inside libadwaita's own collapsed notify left its slide stuck at the
    // start, the sidebar allocated off screen with show-sidebar true.
    const s = splitOf(sv);
    if (adw.OverlaySplitView.getCollapsed(s) == 0 and adw.OverlaySplitView.getSidebar(s) != null) {
        _ = glib.idleAdd(&cbShowSidebarIdle, gobject.Object.ref(obj));
        // libadwaita hides the sidebar's pane when its hide slide ends. A show
        // that starts before that end lands (a conceal right before an
        // uncollapse) is overtaken by it: show-sidebar true, the pane hidden
        // and off screen. Checked once the slide has had time to end.
        _ = glib.timeoutAdd(450, &cbShowSidebarHeal, gobject.Object.ref(obj));
    }
    const was = getFlag(sv, "nd-revealed");
    const now = isRevealed(sv);
    applyContentCard(sv);
    if (was == now) return;
    setFlag(sv, "nd-revealed", now);
    revealEmit(sv, now);
}

fn cbShowSidebarIdle(data: ?*anyopaque) callconv(.c) c_int {
    const obj: *gobject.Object = @ptrCast(@alignCast(data.?));
    defer gobject.Object.unref(obj);
    const s: *adw.OverlaySplitView = @ptrCast(@alignCast(obj));
    if (adw.OverlaySplitView.getCollapsed(s) == 0 and adw.OverlaySplitView.getShowSidebar(s) == 0 and
        adw.OverlaySplitView.getSidebar(s) != null) adw.OverlaySplitView.setShowSidebar(s, 1);
    return 0;
}

fn cbShowSidebarHeal(data: ?*anyopaque) callconv(.c) c_int {
    const obj: *gobject.Object = @ptrCast(@alignCast(data.?));
    defer gobject.Object.unref(obj);
    const s: *adw.OverlaySplitView = @ptrCast(@alignCast(obj));
    if (adw.OverlaySplitView.getCollapsed(s) != 0 or adw.OverlaySplitView.getShowSidebar(s) == 0) return 0;
    const sb = adw.OverlaySplitView.getSidebar(s) orelse return 0;
    const pane = gtk.Widget.getParent(sb) orelse return 0;
    if (gtk.Widget.getChildVisible(pane) != 0) return 0;
    adw.OverlaySplitView.setShowSidebar(s, 0);
    adw.OverlaySplitView.setShowSidebar(s, 1);
    return 0;
}

fn wireReveal(sv: *gtk.Widget) void {
    if (getFlag(sv, K_REVEAL_WIRED)) return;
    setFlag(sv, K_REVEAL_WIRED, true);
    const motion = gtk.EventControllerMotion.new();
    gtk.EventController.setPropagationPhase(motion.as(gtk.EventController), .capture);
    _ = gtk.EventControllerMotion.signals.enter.connect(motion, ?*anyopaque, &cbRevealMotion, null, .{});
    _ = gtk.EventControllerMotion.signals.motion.connect(motion, ?*anyopaque, &cbRevealMotion, null, .{});
    gtk.Widget.addController(sv, motion.as(gtk.EventController));
    _ = gobject.signalConnectData(asObject(sv), "notify::show-sidebar", @ptrCast(&cbShowSidebar), null, null, .{});
    _ = gobject.signalConnectData(asObject(sv), "notify::collapsed", @ptrCast(&cbShowSidebar), null, null, .{});
    _ = gobject.signalConnectData(asObject(sv), "notify::content", @ptrCast(&cbShowSidebar), null, null, .{});
}

pub fn setEdgeReveal(sv: *gtk.Widget, on: bool) void {
    setFlag(sv, K_REVEAL, on);
    wireReveal(sv);
}

pub fn connectSplitReveal(sv: *gtk.Widget, node_id: u32, emit_fn: EmitFn) void {
    reveal_emit = emit_fn;
    gobject.Object.setData(asObject(sv), K_REVEAL_NODE, @ptrFromInt(@as(usize, node_id)));
    wireReveal(sv);
}

pub fn splitCommand(sv: *gtk.Widget, command: []const u8) void {
    if (std.mem.eql(u8, command, "revealSidebar")) {
        reveal(sv, false);
    } else if (std.mem.eql(u8, command, "concealSidebar")) {
        if (isRevealed(sv)) adw.OverlaySplitView.setShowSidebar(splitOf(sv), 0);
    }
}

// ---- <splitview contentStyle="card"> -----------------------------------------
// The split view paints the sidebar's colour under both panes, the sidebar
// pane drops its own fill and edge, and the content widget becomes a rounded
// card inset by the margin (src/gtk/basecss.zig carries the rules). Collapsed,
// the sidebar floats as a rounded panel over the card.

const K_CARD = "nd-content-card";
const K_CARD_CHILD = "nd-content-card-child";
pub const card_margin: c_int = 8;
/// libadwaita's card radius, which the CSS gives the content card too.
pub const card_radius: f64 = 12;

pub fn setContentStyle(sv: *gtk.Widget, style: []const u8) void {
    const on = std.mem.eql(u8, style, "card");
    setFlag(sv, K_CARD, on);
    if (on) gtk.Widget.addCssClass(sv, "nd-content-card") else gtk.Widget.removeCssClass(sv, "nd-content-card");
    wireReveal(sv);
    applyContentCard(sv);
}

fn setMargins(w: *gtk.Widget, start: c_int, top: c_int, end: c_int, bottom: c_int) void {
    gtk.Widget.setMarginStart(w, start);
    gtk.Widget.setMarginTop(w, top);
    gtk.Widget.setMarginEnd(w, end);
    gtk.Widget.setMarginBottom(w, bottom);
}

/// Re-derived whenever the split's content or sidebar state changes: the
/// leading margin is the sidebar's own padding while the sidebar is beside
/// the card, and the card's margin once it is not.
pub fn applyContentCard(sv: *gtk.Widget) void {
    const s = splitOf(sv);
    const on = getFlag(sv, K_CARD);
    const collapsed = adw.OverlaySplitView.getCollapsed(s) != 0;
    // Collapsed, the sidebar floats over the card as a rounded panel of its
    // own, the way the AppKit reveal panel does.
    if (adw.OverlaySplitView.getSidebar(s)) |sb| {
        if (on and collapsed) gtk.Widget.addCssClass(sb, "nd-floating-sidebar") else gtk.Widget.removeCssClass(sb, "nd-floating-sidebar");
        // The pane libadwaita wraps the sidebar in paints its own fill and
        // edge; its CSS node carries no class to select it by, so it gets one.
        if (gtk.Widget.getParent(sb)) |pane| {
            if (pane != sv) {
                if (on) gtk.Widget.addCssClass(pane, "nd-sidebar-pane") else gtk.Widget.removeCssClass(pane, "nd-sidebar-pane");
            }
        }
    }
    // A content pane that is a toolbar view keeps its bars on the sidebar's
    // surface, above and below the card: only its content is the card.
    const pane = adw.OverlaySplitView.getContent(s);
    const bars: ?*adw.ToolbarView = if (pane) |pw| (if (gobject.ext.isA(pw, adw.ToolbarView)) @ptrCast(@alignCast(pw)) else null) else null;
    const content = if (bars) |tv| adw.ToolbarView.getContent(tv) else pane;
    if (bars) |tv| {
        const tvw: *gtk.Widget = @ptrCast(@alignCast(tv));
        if (on) gtk.Widget.addCssClass(tvw, "nd-card-bars") else gtk.Widget.removeCssClass(tvw, "nd-card-bars");
        if (!getFlag(tvw, K_CARD_BARS_WIRED)) {
            setFlag(tvw, K_CARD_BARS_WIRED, true);
            gobject.Object.setData(asObject(tvw), K_CARD_SPLIT, sv);
            _ = gobject.signalConnectData(asObject(tvw), "notify::top-bar-height", @ptrCast(&cbCardBars), null, null, .{});
            _ = gobject.signalConnectData(asObject(tvw), "notify::content", @ptrCast(&cbCardBars), null, null, .{});
        }
    }
    // A child that was the card and no longer is loses the look.
    if (gobject.Object.getData(asObject(sv), K_CARD_CHILD)) |raw| {
        const old: *gtk.Widget = @ptrCast(@alignCast(raw));
        if (!on or content == null or content.? != old) {
            gtk.Widget.removeCssClass(old, "nd-card-content");
            gtk.Widget.setOverflow(old, .visible);
            setMargins(old, 0, 0, 0, 0);
            gobject.Object.setData(asObject(sv), K_CARD_CHILD, null);
        }
    }
    if (!on) return;
    const child = content orelse return;
    gobject.Object.setData(asObject(sv), K_CARD_CHILD, child);
    gtk.Widget.addCssClass(child, "nd-card-content");
    gtk.Widget.setOverflow(child, .hidden);
    const beside = !collapsed and adw.OverlaySplitView.getShowSidebar(s) != 0 and adw.OverlaySplitView.getSidebar(s) != null;
    // A top bar over the card is its own spacing: the card starts under it.
    const top: c_int = if (bars) |tv| (if (adw.ToolbarView.getTopBarHeight(tv) > 0) 0 else card_margin) else card_margin;
    setMargins(child, if (beside) 0 else card_margin, top, card_margin, card_margin);
}

const K_CARD_BARS_WIRED = "nd-card-bars-wired";
const K_CARD_SPLIT = "nd-card-split";

fn cbCardBars(obj: *gobject.Object, _: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
    const raw = gobject.Object.getData(obj, K_CARD_SPLIT) orelse return;
    applyContentCard(@ptrCast(@alignCast(raw)));
}

/// The rounded card an embedded engine's native child window has to be cut to
/// (src/cef), as the ancestor widget carrying the card: null when `w` is in no
/// card.
pub fn cardAncestor(w: *gtk.Widget) ?*gtk.Widget {
    var it: ?*gtk.Widget = w;
    while (it) |cur| : (it = gtk.Widget.getParent(cur)) {
        if (gtk.Widget.hasCssClass(cur, "nd-card-content") != 0) return cur;
        if (gtk.Widget.hasCssClass(cur, "card") != 0) return cur;
    }
    return null;
}

/// What GTK floats over the page `w` is (an embedded engine's native child
/// window, which is drawn above everything GTK paints): a split view's sidebar
/// while it slides over the content, and the layers of a GtkOverlay the page
/// sits in the base of. Written into `out`; returns how many.
pub fn coversOver(w: *gtk.Widget, out: []*gtk.Widget) usize {
    var n: usize = 0;
    var it: ?*gtk.Widget = gtk.Widget.getParent(w);
    while (it) |cur| : (it = gtk.Widget.getParent(cur)) {
        if (gobject.ext.isA(cur, gtk.Overlay)) {
            const base = gtk.Overlay.getChild(@ptrCast(@alignCast(cur)));
            var child = gtk.Widget.getFirstChild(cur);
            while (child) |c| : (child = gtk.Widget.getNextSibling(c)) {
                if (base != null and c == base.?) continue;
                if (gtk.Widget.getMapped(c) == 0 or gtk.Widget.getOpacity(c) <= 0) continue;
                // A layer that holds the page itself is not over it.
                if (gtk.Widget.isAncestor(w, c) != 0) continue;
                if (n < out.len) {
                    out[n] = c;
                    n += 1;
                }
            }
        }
        if (gobject.ext.isA(cur, adw.OverlaySplitView)) {
            const s = splitOf(cur);
            if (adw.OverlaySplitView.getCollapsed(s) == 0) continue;
            const sb = adw.OverlaySplitView.getSidebar(s) orelse continue;
            if (gtk.Widget.getMapped(sb) == 0) continue;
            if (n < out.len) {
                out[n] = sb;
                n += 1;
            }
        }
    }
    return n;
}

// ---- <progressbar cssClasses={["osd"]}> as a page-load bar -------------------
// Adwaita's `progressbar.osd` is already the thin bar GNOME Web loads pages
// with; GtkProgressBar jumps to each value, though. An osd bar slides to the
// new value and fades out once it reaches 1, the way the AppKit peer
// (NDShell/LoadBar.swift) does. Any other bar keeps setting its value.

const K_BAR_ANIM = "nd-load-bar-anim";
const K_BAR_FADE = "nd-load-bar-fade";
const load_bar_slide_ms: c_uint = 250;
const load_bar_fade_ms: c_uint = 300;

fn cbBarValue(value: f64, data: ?*anyopaque) callconv(.c) void {
    const bar: *gtk.ProgressBar = @ptrCast(@alignCast(data.?));
    gtk.ProgressBar.setFraction(bar, value);
}

fn cbBarSlideDone(anim: *adw.Animation, data: ?*anyopaque) callconv(.c) void {
    const w: *gtk.Widget = @ptrCast(@alignCast(data.?));
    if (adw.Animation.getValue(anim) < 1) return;
    const target = adw.PropertyAnimationTarget.new(asObject(w), "opacity");
    const fade = adw.TimedAnimation.new(w, gtk.Widget.getOpacity(w), 0, load_bar_fade_ms, target.as(adw.AnimationTarget));
    adw.TimedAnimation.setEasing(fade, .ease_out_cubic);
    stopAnim(w, K_BAR_FADE);
    gobject.Object.setDataFull(asObject(w), K_BAR_FADE, fade, @ptrCast(&gobject.Object.unref));
    adw.Animation.play(fade.as(adw.Animation));
}

fn stopAnim(w: *gtk.Widget, key: [*:0]const u8) void {
    if (gobject.Object.getData(asObject(w), key)) |raw| {
        adw.Animation.pause(@ptrCast(@alignCast(raw)));
        gobject.Object.setData(asObject(w), key, null);
    }
}

pub fn progressSetFraction(w: *gtk.Widget, fraction: f64) void {
    const bar: *gtk.ProgressBar = @ptrCast(@alignCast(w));
    const target_value = std.math.clamp(fraction, 0, 1);
    if (gtk.Widget.hasCssClass(w, "osd") == 0) {
        gtk.ProgressBar.setFraction(bar, target_value);
        return;
    }
    const current = gtk.ProgressBar.getFraction(bar);
    stopAnim(w, K_BAR_ANIM);
    // A value below the one shown is a new load: it jumps back, visible again.
    if (target_value < current or gtk.Widget.getOpacity(w) < 1) {
        stopAnim(w, K_BAR_FADE);
        gtk.Widget.setOpacity(w, 1);
        if (target_value < current) {
            gtk.ProgressBar.setFraction(bar, target_value);
            return;
        }
    }
    const cb = adw.CallbackAnimationTarget.new(&cbBarValue, bar, null);
    const slide = adw.TimedAnimation.new(w, current, target_value, load_bar_slide_ms, cb.as(adw.AnimationTarget));
    adw.TimedAnimation.setEasing(slide, .ease_out_cubic);
    _ = adw.Animation.signals.done.connect(slide.as(adw.Animation), ?*anyopaque, &cbBarSlideDone, w, .{});
    gobject.Object.setDataFull(asObject(w), K_BAR_ANIM, slide, @ptrCast(&gobject.Object.unref));
    adw.Animation.play(slide.as(adw.Animation));
}

/// Create arm: an osd bar that mounts already complete is a finished load,
/// and starts faded rather than flashing full width.
pub fn progressCreated(w: *gtk.Widget, fraction: f64, props: ?std.json.Value) void {
    gtk.ProgressBar.setFraction(@ptrCast(@alignCast(w)), std.math.clamp(fraction, 0, 1));
    if (fraction < 1) return;
    const p = props orelse return;
    if (p != .object) return;
    const classes = p.object.get("cssClasses") orelse return;
    if (classes != .array) return;
    for (classes.array.items) |c| {
        if (c == .string and std.mem.eql(u8, c.string, "osd")) gtk.Widget.setOpacity(w, 0);
    }
}
