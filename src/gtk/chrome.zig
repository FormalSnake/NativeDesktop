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
//   <toolbarview topBarsAutoHide>  the top bars slide away over the content and
//                                  back when the pointer reaches the top edge
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
    return (pointerIn(sv) orelse return null).f_x;
}

/// The pointer in `w`'s coordinates, read off the device rather than from
/// motion events: an embedded engine's native child window takes those.
fn pointerIn(sv: *gtk.Widget) ?graphene.Point {
    return pointerAt(sv, false);
}

/// As pointerIn, and null unless the pointer is over the window's own surface,
/// its frame included: an undecorated window's top edge is its resize border
/// (a few px above the content) as much as its first row.
fn pointerOnWindow(w: *gtk.Widget) ?graphene.Point {
    return pointerAt(w, true);
}

fn pointerAt(sv: *gtk.Widget, on_surface: bool) ?graphene.Point {
    const native = gtk.Widget.getNative(sv) orelse return null;
    const surface = gtk.Native.getSurface(native) orelse return null;
    const display = gtk.Widget.getDisplay(sv);
    const seat = gdk.Display.getDefaultSeat(display) orelse return null;
    const pointer = gdk.Seat.getPointer(seat) orelse return null;
    var x: f64 = 0;
    var y: f64 = 0;
    if (gdk.Surface.getDevicePosition(surface, pointer, &x, &y, null) == 0) return null;
    if (on_surface and (x < 0 or y < 0 or x > @as(f64, @floatFromInt(gdk.Surface.getWidth(surface))) or
        y > @as(f64, @floatFromInt(gdk.Surface.getHeight(surface))))) return null;
    var sx: f64 = 0;
    var sy: f64 = 0;
    gtk.Native.getSurfaceTransform(native, &sx, &sy);
    var in = graphene.Point{ .f_x = @floatCast(x - sx), .f_y = @floatCast(y - sy) };
    var out: graphene.Point = undefined;
    if (gtk.Widget.computePoint(native.as(gtk.Widget), sv, &in, &out) == 0) return null;
    return out;
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

/// Collapsed, the content fills the window edge to edge, so no GTK-drawn
/// margin is left for the motion controller to see the pointer arrive at the
/// leading edge over an engine's page; the device is read instead.
const K_EDGE_POLL = "nd-edge-reveal-edge-poll";

fn cbEdgePoll(data: ?*anyopaque) callconv(.c) c_int {
    const sv: *gtk.Widget = @ptrCast(@alignCast(data.?));
    const s = splitOf(sv);
    if (!getFlag(sv, K_REVEAL) or adw.OverlaySplitView.getCollapsed(s) == 0 or gtk.Widget.getRoot(sv) == null) {
        gobject.Object.setData(asObject(sv), K_EDGE_POLL, null);
        gobject.Object.unref(asObject(sv));
        return 0;
    }
    if (!isRevealed(sv)) {
        if (pointerOnWindow(sv)) |pt| {
            const w: f64 = @floatFromInt(gtk.Widget.getWidth(sv));
            const h: f64 = @floatFromInt(gtk.Widget.getHeight(sv));
            const rtl = gtk.Widget.getDirection(sv) == .rtl;
            const from_edge = if (rtl) w - pt.f_x else pt.f_x;
            if (from_edge <= reveal_edge_px and pt.f_y >= 0 and pt.f_y <= h) reveal(sv, true);
        }
    }
    return 1;
}

fn startEdgePoll(sv: *gtk.Widget) void {
    if (gobject.Object.getData(asObject(sv), K_EDGE_POLL) != null) return;
    const id = glib.timeoutAdd(reveal_poll_ms, &cbEdgePoll, gobject.Object.ref(asObject(sv)));
    gobject.Object.setData(asObject(sv), K_EDGE_POLL, @ptrFromInt(@as(usize, id)));
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
    // A sidebar sliding out on its way to collapsing is the app's own doing.
    if (adw.OverlaySplitView.getCollapsed(s) == 0 and adw.OverlaySplitView.getSidebar(s) != null and !getFlag(sv, K_WANT_HIDDEN)) {
        _ = glib.idleAdd(&cbShowSidebarIdle, gobject.Object.ref(obj));
        // libadwaita hides the sidebar's pane when its hide slide ends. A show
        // that starts before that end lands (a conceal right before an
        // uncollapse) is overtaken by it: show-sidebar true, the pane hidden
        // and off screen. Checked once the slide has had time to end.
        _ = glib.timeoutAdd(450, &cbShowSidebarHeal, gobject.Object.ref(obj));
    }
    if (adw.OverlaySplitView.getCollapsed(s) != 0 and getFlag(sv, K_REVEAL)) startEdgePoll(sv);
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
        adw.OverlaySplitView.getSidebar(s) != null and !getFlag(obj, K_WANT_HIDDEN)) adw.OverlaySplitView.setShowSidebar(s, 1);
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
    if (on and adw.OverlaySplitView.getCollapsed(splitOf(sv)) != 0) startEdgePoll(sv);
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

// ---- <splitview collapsed>: the sidebar slides -------------------------------
// Collapsing an AdwOverlaySplitView swaps its layout in one frame. The slide is
// the split's own uncollapsed show-sidebar spring instead (critically damped,
// about 210 ms to within a pixel, skipped when animations are off), with the
// collapse itself applied once the sidebar is out of sight, and undone before
// it slides back in. `pin-sidebar` is held only around those two
// set_collapsed calls: unpinned, set_collapsed snaps show-sidebar to match.
//
// An embedded engine's page is a native child window that cannot follow a
// GTK transform, and re-laying a page out on every frame is what a slow
// machine cannot keep up with, so the page reads `pageMotionShift` and
// stretches a still of itself over the slide instead (src/cef/engine.zig).

/// The app's own `collapsed`, which the slide is heading for.
const K_WANT_HIDDEN = "nd-split-want-hidden";
const K_MOTION = "nd-split-motion";

const Motion = struct {
    tick: c_uint = 0,
    /// Frame clock times (µs) of every frame the slide ran, for ND_MOTION_TRACE.
    frames: [96]i64 = undefined,
    frame_n: usize = 0,
    refresh_us: i64 = 0,
    reversals: u32 = 0,
    /// The page has been told the slide is over: it swaps its still for the
    /// live page as soon as that has painted at its new size.
    page_released: bool = false,
};

const motion_trace = struct {
    var checked = false;
    var on = false;
    fn enabled() bool {
        if (!checked) {
            checked = true;
            on = std.c.getenv("ND_MOTION_TRACE") != null;
        }
        return on;
    }
};

fn motionOf(sv: *gtk.Widget) ?*Motion {
    const raw = gobject.Object.getData(asObject(sv), K_MOTION) orelse return null;
    return @ptrCast(@alignCast(raw));
}

fn setCollapsedPinned(s: *adw.OverlaySplitView, collapsed: bool) void {
    const pinned = adw.OverlaySplitView.getPinSidebar(s);
    adw.OverlaySplitView.setPinSidebar(s, 1);
    adw.OverlaySplitView.setCollapsed(s, @intFromBool(collapsed));
    adw.OverlaySplitView.setPinSidebar(s, pinned);
}

/// The `collapsed` prop. Not on screen, or with no sidebar, it lands at once.
pub fn setCollapsed(sv: *gtk.Widget, collapsed: bool) void {
    const s = splitOf(sv);
    setFlag(sv, K_WANT_HIDDEN, collapsed);
    if (adw.OverlaySplitView.getSidebar(s) == null or gtk.Widget.getMapped(sv) == 0) {
        adw.OverlaySplitView.setCollapsed(s, @intFromBool(collapsed));
        return;
    }
    wireReveal(sv);
    const is_collapsed = adw.OverlaySplitView.getCollapsed(s) != 0;
    const shown = adw.OverlaySplitView.getShowSidebar(s) != 0;
    if (collapsed) {
        // Already collapsed, peeking or not: the app's state is already this.
        if (is_collapsed or !shown) return;
        startMotion(sv);
        adw.OverlaySplitView.setShowSidebar(s, 0);
    } else if (!is_collapsed) {
        // A slide out still running turns round where it is.
        if (shown) return;
        startMotion(sv);
        adw.OverlaySplitView.setShowSidebar(s, 1);
    } else {
        // A peeking sidebar is already in place: only the page moves.
        startMotion(sv);
        setCollapsedPinned(s, false);
        if (!shown) adw.OverlaySplitView.setShowSidebar(s, 1);
    }
}

fn startMotion(sv: *gtk.Widget) void {
    if (motionOf(sv)) |m| {
        m.reversals += 1;
        m.page_released = false;
        return;
    }
    const m = std.heap.c_allocator.create(Motion) catch return;
    m.* = .{};
    gobject.Object.setData(asObject(sv), K_MOTION, m);
    m.tick = gtk.Widget.addTickCallback(sv, &cbMotionTick, null, null);
    // The page reads the slide on the next layout; this is what makes one.
    gtk.Widget.queueAllocate(sv);
}

/// How far the slide still has to move the content's leading edge, in logical
/// pixels, while `w` is inside a split that is sliding; null otherwise. The
/// content sits at the sidebar's offset, a whole pixel, as libadwaita
/// allocates it (adw-overlay-split-view.c, allocate_uncollapsed).
pub fn pageMotionShift(w: *gtk.Widget) ?f64 {
    var it: ?*gtk.Widget = gtk.Widget.getParent(w);
    while (it) |cur| : (it = gtk.Widget.getParent(cur)) {
        if (!gobject.ext.isA(cur, adw.OverlaySplitView)) continue;
        const m = motionOf(cur) orelse continue;
        if (m.page_released) return null;
        const s = splitOf(cur);
        const sidebar = adw.OverlaySplitView.getSidebar(s) orelse return null;
        if (gtk.Widget.isAncestor(w, sidebar) != 0) return null;
        const width: f64 = @floatFromInt(gtk.Widget.getWidth(gtk.Widget.getParent(sidebar) orelse sidebar));
        const progress = adw.Swipeable.getProgress(@ptrCast(@alignCast(cur)));
        const target: f64 = if (adw.OverlaySplitView.getShowSidebar(s) != 0) 1 else 0;
        return @trunc(width * target) - @trunc(width * @max(0, @min(1, progress)));
    }
    return null;
}

fn cbMotionTick(w: *gtk.Widget, clock: *gdk.FrameClock, _: ?*anyopaque) callconv(.c) c_int {
    const m = motionOf(w) orelse return 0;
    if (m.frame_n < m.frames.len) {
        m.frames[m.frame_n] = gdk.FrameClock.getFrameTime(clock);
        m.frame_n += 1;
    }
    if (m.refresh_us == 0) {
        var refresh: i64 = 0;
        var presentation: i64 = 0;
        gdk.FrameClock.getRefreshInfo(clock, gdk.FrameClock.getFrameTime(clock), &refresh, &presentation);
        m.refresh_us = refresh;
    }
    const s = splitOf(w);
    const target: f64 = if (adw.OverlaySplitView.getShowSidebar(s) != 0) 1 else 0;
    const progress = adw.Swipeable.getProgress(@ptrCast(@alignCast(w)));
    const sidebar = adw.OverlaySplitView.getSidebar(s);
    const width: f64 = if (sidebar) |sb| @floatFromInt(gtk.Widget.getWidth(gtk.Widget.getParent(sb) orelse sb)) else 0;
    // Within a pixel the allocation no longer changes: the page can take its
    // place while the spring's tail runs out.
    if (!m.page_released and @abs(progress - target) * width < 0.5) {
        m.page_released = true;
        gtk.Widget.queueAllocate(w);
        traceMotion(m, target);
    }
    if (@abs(progress - target) > 1e-6) return 1;
    if (target == 0 and getFlag(w, K_WANT_HIDDEN) and adw.OverlaySplitView.getCollapsed(s) == 0) {
        setCollapsedPinned(s, true);
    }
    if (!m.page_released) gtk.Widget.queueAllocate(w);
    gobject.Object.setData(asObject(w), K_MOTION, null);
    std.heap.c_allocator.destroy(m);
    return 0;
}

/// ND_MOTION_TRACE: one line per slide with its frame pacing, from the frame
/// clock the slide itself ran on.
fn traceMotion(m: *Motion, target: f64) void {
    if (!motion_trace.enabled() or m.frame_n < 2) return;
    const refresh: i64 = if (m.refresh_us > 0) m.refresh_us else 16_667;
    var worst: i64 = 0;
    var late: usize = 0;
    var i: usize = 1;
    while (i < m.frame_n) : (i += 1) {
        const d = m.frames[i] - m.frames[i - 1];
        worst = @max(worst, d);
        if (d * 2 > refresh * 3) late += 1;
    }
    std.debug.print("ND_SPLIT_MOTION dir={s} frames={d} span_ms={d:.1} worst_ms={d:.1} late={d} refresh_ms={d:.2} reversals={d}\n", .{
        if (target > 0.5) "show" else "hide",
        m.frame_n,
        @as(f64, @floatFromInt(m.frames[m.frame_n - 1] - m.frames[0])) / 1000,
        @as(f64, @floatFromInt(worst)) / 1000,
        late,
        @as(f64, @floatFromInt(refresh)) / 1000,
        m.reversals,
    });
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
            gtk.Widget.removeCssClass(old, "nd-card-immersive");
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
    // With the sidebar away the page is immersive: edge to edge, no frame.
    if (beside) {
        gtk.Widget.removeCssClass(child, "nd-card-immersive");
        setMargins(child, 0, top, card_margin, card_margin);
    } else {
        gtk.Widget.addCssClass(child, "nd-card-immersive");
        setMargins(child, 0, 0, 0, 0);
    }
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
        // A toast floats over the page the overlay holds, and AdwToastOverlay
        // is not a GtkOverlay.
        if (gobject.ext.isA(cur, adw.ToastOverlay)) {
            const base = adw.ToastOverlay.getChild(@ptrCast(@alignCast(cur)));
            var child = gtk.Widget.getFirstChild(cur);
            while (child) |c| : (child = gtk.Widget.getNextSibling(c)) {
                if (base != null and c == base.?) continue;
                if (gtk.Widget.getMapped(c) == 0 or gtk.Widget.getOpacity(c) <= 0) continue;
                if (n < out.len) {
                    out[n] = c;
                    n += 1;
                }
            }
        }
        if (gobject.ext.isA(cur, adw.ToolbarView) and getFlag(cur, K_AUTOHIDE)) {
            var child = gtk.Widget.getFirstChild(cur);
            while (child) |c| : (child = gtk.Widget.getNextSibling(c)) {
                if (gtk.Widget.hasCssClass(c, "top-bar") == 0 or gtk.Widget.getMapped(c) == 0) continue;
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

// ---- <toolbarview topBarsAutoHide> -------------------------------------------
// Zen's hidden title bar: AdwToolbarView's own reveal-top-bars slide, with the
// content extended under the bars so it never moves when they come and go.
// The trigger is the pointer at the top edge and the leave is the pointer off
// the bars for 250 ms, both read off the device (see pointerIn).

const K_AUTOHIDE = "nd-top-autohide";
const K_AUTOHIDE_POLL = "nd-top-autohide-poll";
const K_AUTOHIDE_AWAY = "nd-top-autohide-away";
/// Revealed by a command rather than the pointer: stays until concealed.
const K_AUTOHIDE_HELD = "nd-top-autohide-held";
const top_edge_px: f64 = 6;

fn tvOf(w: *gtk.Widget) *adw.ToolbarView {
    return @ptrCast(@alignCast(w));
}

fn cbAutoHidePoll(data: ?*anyopaque) callconv(.c) c_int {
    const w: *gtk.Widget = @ptrCast(@alignCast(data.?));
    if (!getFlag(w, K_AUTOHIDE) or gtk.Widget.getRoot(w) == null) {
        gobject.Object.setData(asObject(w), K_AUTOHIDE_POLL, null);
        gobject.Object.unref(asObject(w));
        return 0;
    }
    const tv = tvOf(w);
    const shown = adw.ToolbarView.getRevealTopBars(tv) != 0;
    const zone: f64 = if (shown) @as(f64, @floatFromInt(adw.ToolbarView.getTopBarHeight(tv))) + reveal_slack_px else top_edge_px;
    const inside = if (pointerOnWindow(w)) |pt| pt.f_y <= zone else false;
    if (inside) {
        gobject.Object.setData(asObject(w), K_AUTOHIDE_AWAY, null);
        if (!shown) adw.ToolbarView.setRevealTopBars(tv, 1);
    } else if (shown and !getFlag(w, K_AUTOHIDE_HELD)) {
        const away = @intFromPtr(gobject.Object.getData(asObject(w), K_AUTOHIDE_AWAY)) + 1;
        gobject.Object.setData(asObject(w), K_AUTOHIDE_AWAY, @ptrFromInt(away));
        if (away >= reveal_away_ticks) {
            gobject.Object.setData(asObject(w), K_AUTOHIDE_AWAY, null);
            adw.ToolbarView.setRevealTopBars(tv, 0);
        }
    }
    return 1;
}

pub fn setTopBarsAutoHide(w: *gtk.Widget, on: bool) void {
    if (getFlag(w, K_AUTOHIDE) == on) return;
    setFlag(w, K_AUTOHIDE, on);
    setFlag(w, K_AUTOHIDE_HELD, false);
    const tv = tvOf(w);
    adw.ToolbarView.setExtendContentToTopEdge(tv, @intFromBool(on));
    adw.ToolbarView.setRevealTopBars(tv, @intFromBool(!on));
    if (on) gtk.Widget.addCssClass(w, "nd-top-autohide") else gtk.Widget.removeCssClass(w, "nd-top-autohide");
    if (on and gobject.Object.getData(asObject(w), K_AUTOHIDE_POLL) == null) {
        const id = glib.timeoutAdd(reveal_poll_ms, &cbAutoHidePoll, gobject.Object.ref(asObject(w)));
        gobject.Object.setData(asObject(w), K_AUTOHIDE_POLL, @ptrFromInt(@as(usize, id)));
    }
}

pub fn toolbarCommand(w: *gtk.Widget, command: []const u8) void {
    if (!getFlag(w, K_AUTOHIDE)) return;
    const reveal_bars = std.mem.eql(u8, command, "revealTopBars");
    if (!reveal_bars and !std.mem.eql(u8, command, "concealTopBars")) return;
    setFlag(w, K_AUTOHIDE_HELD, reveal_bars);
    adw.ToolbarView.setRevealTopBars(tvOf(w), @intFromBool(reveal_bars));
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
    _ = adw.Animation.signals.done.connect(fade.as(adw.Animation), ?*anyopaque, &cbBarFadeDone, w, .{});
    gobject.Object.setDataFull(asObject(w), K_BAR_FADE, fade, @ptrCast(&gobject.Object.unref));
    adw.Animation.play(fade.as(adw.Animation));
}

/// A faded bar is hidden, not just transparent: GTK keeps restyling the
/// finished bar's progress node every frame while it is shown, which held the
/// frame clock (and an X round trip per frame) running at idle.
fn cbBarFadeDone(anim: *adw.Animation, data: ?*anyopaque) callconv(.c) void {
    const w: *gtk.Widget = @ptrCast(@alignCast(data.?));
    if (adw.Animation.getValue(anim) > 0) return;
    gtk.Widget.setVisible(w, 0);
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
        gtk.Widget.setVisible(w, 1);
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
        if (c == .string and std.mem.eql(u8, c.string, "osd")) {
            gtk.Widget.setOpacity(w, 0);
            gtk.Widget.setVisible(w, 0);
        }
    }
}
