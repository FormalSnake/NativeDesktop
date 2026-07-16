// Native system tabs for the <window> widget (GTK/libadwaita side).
//
// The app model is tabs-as-windows: every `<window tabGroup="x">` React root
// is one tab. On this backend the group owns the real OS window — an
// AdwApplicationWindow whose content is AdwTabOverview{view} > AdwTabView
// (Ghostty's exact hierarchy) — and each member Window NODE's handle is an
// AdwBin that lives as an AdwTabPage child. A group can span several such
// scaffold windows (Chrome-style: dragging a tab to the desktop rides
// AdwTabView::create-window into a fresh scaffold; the page and its child
// widget transfer intact, so a <webview> tab never reloads).
//
// Tab chrome is framework-injected so the SAME app tree stays native here:
// an AdwTabBar (autohide, with a new-tab button as its end action widget)
// slots under the app's own <headerbar> inside each page, and an
// AdwTabButton ("overview.open", page-count badge) packs at the end of that
// headerbar — the GNOME tab-overview affordance. Apps without a <toolbarview>
// root get a framework AdwToolbarView wrapper carrying the tab bar.
//
// Close protocol is deferred BY DESIGN (AdwTabView's async close-page
// contract): user close -> hold the page pending, emit the node's `closed`
// event -> JS unmounts the <window> -> the remove op's `window.close`
// semantic action lands back here -> close_page_finish(TRUE). JS-initiated
// unmount without a prior close-page takes the same path via closePage with
// a confirmed marker. Page bins carry a framework g_object_ref so the node
// handle outlives the page (mirrors reparent_child's ref bracket).
const std = @import("std");
const gtk = @import("gtk");
const gdk = @import("gdk");
const glib = @import("glib");
const gobject = @import("gobject");
const adw = @import("adw");
const protocol = @import("../protocol.zig");

pub const EmitFn = *const fn (node_id: u32, name: []const u8, payload: protocol.EventPayload) void;

var emit: ?EmitFn = null;
const alloc = std.heap.page_allocator;

// g_object data keys. On page bins: node id, chrome pointers, close-protocol
// flags. On scaffold windows: overview/view/group backrefs. Keys are distinct
// from dialogs.zig's "nd-window-node-id" so the two modules stay uncoupled.
const K_TAB_BIN = "nd-tab-bin";
const K_NODE_ID = "nd-tabs-node-id";
const K_TAB_BAR = "nd-tab-bar";
const K_TAB_BTN = "nd-tab-btn";
const K_WRAPPER = "nd-tab-wrapper";
const K_PENDING = "nd-tab-pending-close";
const K_CONFIRMED = "nd-tab-close-confirmed";
const K_OVERVIEW = "nd-tab-overview";
const K_VIEW = "nd-tab-view";
const K_GROUP = "nd-tab-group";
const K_WIN_CLOSED = "nd-window-closed";
const K_BTN_DONE = "nd-tab-btn-injected";

const Group = struct {
    name: []u8,
    windows: std.ArrayListUnmanaged(*gtk.Window) = .empty,
    last_active: ?*gtk.Window = null,
};

var groups: std.StringHashMapUnmanaged(*Group) = .empty;

fn setData(obj: anytype, key: [*:0]const u8, val: ?*anyopaque) void {
    gobject.Object.setData(@ptrCast(@alignCast(obj)), key, val);
}
fn getData(obj: anytype, key: [*:0]const u8) ?*anyopaque {
    return gobject.Object.getData(@ptrCast(@alignCast(obj)), key);
}
fn setFlag(obj: anytype, key: [*:0]const u8, on: bool) void {
    setData(obj, key, if (on) @ptrFromInt(@as(usize, 1)) else null);
}
fn hasFlag(obj: anytype, key: [*:0]const u8) bool {
    return getData(obj, key) != null;
}

pub fn isTabBin(widget: *gtk.Widget) bool {
    return hasFlag(widget, K_TAB_BIN);
}

fn binNodeId(bin: *gtk.Widget) ?u32 {
    const raw = getData(bin, K_NODE_ID) orelse return null;
    return @intCast(@intFromPtr(raw));
}

/// The AdwTabView a page bin currently lives under (changes on cross-window
/// drag — always resolve, never cache).
fn owningView(bin: *gtk.Widget) ?*adw.TabView {
    const w = gtk.Widget.getAncestor(bin, adw.TabView.getGObjectType()) orelse return null;
    return @ptrCast(@alignCast(w));
}

/// The real gtk.Window above any node handle (plain windows are their own).
pub fn owningWindow(widget: *gtk.Widget) ?*gtk.Window {
    if (gobject.ext.isA(widget, gtk.Window)) return @ptrCast(@alignCast(widget));
    const root = gtk.Widget.getRoot(widget) orelse return null;
    return @ptrCast(@alignCast(root));
}

fn emitEmpty(node_id: u32, name: []const u8) void {
    if (emit) |f| f(node_id, name, .{ .data = .{ .object = .empty } });
}

// ---- creation --------------------------------------------------------------

/// Generated create-arm entry. Plain window (no tabGroup) keeps the previous
/// AdwApplicationWindow behavior verbatim; a group member becomes a page bin
/// in the group's most recently active scaffold (created on demand).
pub fn createWindow(
    app: *gtk.Application,
    tab_group: ?[]const u8,
    title: ?[]const u8,
    width: c_int,
    height: c_int,
    the_window: *?*gtk.Window,
    dupeZ: *const fn ([]const u8) [:0]const u8,
) !*gtk.Widget {
    const group_name = tab_group orelse {
        const window = adw.ApplicationWindow.new(app);
        const win = window.as(gtk.Window);
        the_window.* = win;
        if (title) |t| gtk.Window.setTitle(win, dupeZ(t));
        gtk.Window.setDefaultSize(win, width, height);
        gtk.Window.present(win);
        return window.as(gtk.Widget);
    };

    const group = try ensureGroup(group_name);
    const win = group.last_active orelse if (group.windows.items.len > 0)
        group.windows.items[group.windows.items.len - 1]
    else
        try createScaffold(app, group, width, height);
    the_window.* = win;

    const view: *adw.TabView = @ptrCast(@alignCast(getData(win, K_VIEW).?));
    const bin = adw.Bin.new();
    const bin_w = bin.as(gtk.Widget);
    // Framework ref: the node handle must stay a valid object after the page
    // closes (the remove op still addresses it). Dropped in cleanupBin.
    _ = gobject.Object.ref(@ptrCast(@alignCast(bin_w)));
    setFlag(bin_w, K_TAB_BIN, true);
    const page = adw.TabView.append(view, bin_w);
    if (title) |t| adw.TabPage.setTitle(page, dupeZ(t));
    adw.TabView.setSelectedPage(view, page);
    return bin_w;
}

fn ensureGroup(name: []const u8) !*Group {
    if (groups.get(name)) |g| return g;
    const g = try alloc.create(Group);
    g.* = .{ .name = try alloc.dupe(u8, name) };
    try groups.put(alloc, g.name, g);
    return g;
}

/// One scaffold OS window: AdwApplicationWindow > AdwTabOverview{view} >
/// AdwTabView. The overview's own new-tab button stays disabled — its
/// create-tab signal demands a synchronously returned AdwTabPage, which an
/// async JS round-trip can't produce; the tab bar's injected "+" fires
/// `newTabRequested` instead.
fn createScaffold(app: *gtk.Application, group: *Group, width: c_int, height: c_int) !*gtk.Window {
    const window = adw.ApplicationWindow.new(app);
    const win = window.as(gtk.Window);
    gtk.Window.setDefaultSize(win, width, height);

    const overview = adw.TabOverview.new();
    const view = adw.TabView.new();
    adw.TabOverview.setChild(overview, view.as(gtk.Widget));
    adw.TabOverview.setView(overview, view);
    adw.TabOverview.setEnableNewTab(overview, 0);
    adw.ApplicationWindow.setContent(window, overview.as(gtk.Widget));

    setData(win, K_OVERVIEW, overview);
    setData(win, K_VIEW, view);
    setData(win, K_GROUP, group);
    // The view needs the window for create-window/title sync; data instead of
    // getRoot because signal callbacks can fire mid-reparent.
    setData(view, K_GROUP, group);

    _ = adw.TabView.signals.close_page.connect(view, ?*anyopaque, &onClosePage, null, .{});
    _ = adw.TabView.signals.create_window.connect(view, ?*anyopaque, &onCreateWindow, null, .{});
    _ = adw.TabView.signals.page_attached.connect(view, ?*anyopaque, &onPageAttached, null, .{});
    _ = adw.TabView.signals.page_detached.connect(view, ?*anyopaque, &onPageDetached, null, .{});
    _ = gobject.signalConnectData(@ptrCast(@alignCast(view)), "notify::selected-page", @ptrCast(&onSelectedPage), win, null, .{});
    _ = gobject.signalConnectData(@ptrCast(@alignCast(win)), "notify::is-active", @ptrCast(&onWindowActive), null, null, .{});
    _ = gtk.Window.signals.close_request.connect(win, ?*anyopaque, &onScaffoldCloseRequest, null, .{});
    _ = gtk.Widget.signals.destroy.connect(win.as(gtk.Widget), ?*anyopaque, &onScaffoldDestroy, null, .{});

    // Ctrl+W closes the active tab. It's a native tab-system keybinding (GNOME
    // AdwTabView apps bind it), so the tab window owns it rather than leaning on
    // the app's optional menu. Capture phase intercepts before the focused
    // webview can swallow the accelerator.
    const keys = gtk.EventControllerKey.new();
    gtk.EventController.setPropagationPhase(keys.as(gtk.EventController), .capture);
    _ = gtk.EventControllerKey.signals.key_pressed.connect(keys, *gtk.Window, &onWindowKey, win, .{});
    gtk.Widget.addController(win.as(gtk.Widget), keys.as(gtk.EventController));

    try group.windows.append(alloc, win);
    group.last_active = win;
    gtk.Window.present(win);
    return win;
}

// ---- structural attach (generated Window arm delegates here) ---------------

/// Window-node content attach. Plain window: AdwApplicationWindow.setContent.
/// Page bin: mount the child and inject tab chrome — into the app's own
/// AdwToolbarView when it has one (tab bar lands BELOW its headerbar, the
/// Epiphany layout), else under a framework AdwToolbarView wrapper.
pub fn appendToWindow(parent: *gtk.Widget, child: *gtk.Widget) void {
    if (!isTabBin(parent)) {
        adw.ApplicationWindow.setContent(@ptrCast(@alignCast(parent)), child);
        return;
    }
    const bin: *adw.Bin = @ptrCast(@alignCast(parent));
    if (gobject.ext.isA(child, adw.ToolbarView)) {
        setData(parent, K_WRAPPER, null);
        adw.Bin.setChild(bin, child);
        const tv: *adw.ToolbarView = @ptrCast(@alignCast(child));
        injectTabBar(parent, tv);
        injectTabButtonInto(parent, tv);
    } else {
        const wrapper = adw.ToolbarView.new();
        setData(parent, K_WRAPPER, wrapper);
        adw.ToolbarView.setContent(wrapper, child);
        adw.Bin.setChild(bin, wrapper.as(gtk.Widget));
        injectTabBar(parent, wrapper);
    }
}

pub fn removeFromWindow(parent: *gtk.Widget, child: *gtk.Widget) void {
    if (!isTabBin(parent)) {
        adw.ApplicationWindow.setContent(@ptrCast(@alignCast(parent)), null);
        return;
    }
    const bin: *adw.Bin = @ptrCast(@alignCast(parent));
    if (getData(parent, K_WRAPPER)) |w| {
        const wrapper: *adw.ToolbarView = @ptrCast(@alignCast(w));
        if (gtk.Widget.getParent(child) == wrapper.as(gtk.Widget)) {
            adw.ToolbarView.setContent(wrapper, null);
            return;
        }
    }
    // Direct-mounted app toolbarview: reclaim the injected tab bar before the
    // child (and the bar with it) is torn down, so re-append re-injects fresh.
    if (getData(parent, K_TAB_BAR)) |tb| {
        const bar: *adw.TabBar = @ptrCast(@alignCast(tb));
        if (gobject.ext.isA(child, adw.ToolbarView)) adw.ToolbarView.remove(@ptrCast(@alignCast(child)), bar.as(gtk.Widget));
        setData(parent, K_TAB_BAR, null);
    }
    adw.Bin.setChild(bin, null);
}

/// One AdwTabBar per page, bound to the CURRENT owning view (rebound on
/// page-attached after a cross-window drag). End action widget is the
/// new-tab "+" that fires the node's newTabRequested.
fn injectTabBar(bin: *gtk.Widget, tv: *adw.ToolbarView) void {
    if (getData(bin, K_TAB_BAR) != null) return;
    const bar = adw.TabBar.new();
    adw.TabBar.setAutohide(bar, 1);
    if (owningView(bin)) |view| adw.TabBar.setView(bar, view);
    const plus = gtk.Button.newFromIconName("tab-new-symbolic");
    gtk.Widget.addCssClass(plus.as(gtk.Widget), "flat");
    _ = gtk.Button.signals.clicked.connect(plus, *gtk.Widget, &onNewTabClicked, bin, .{});
    adw.TabBar.setEndActionWidget(bar, plus.as(gtk.Widget));
    adw.ToolbarView.addTopBar(tv, bar.as(gtk.Widget));
    setData(bin, K_TAB_BAR, bar);
}

/// Generated ToolbarView arm hook: a <headerbar> just attached. If this
/// toolbarview lives in a tab page, (a) our tab bar must sit BELOW the new
/// headerbar (AdwToolbarView stacks top bars in add order — re-adding moves
/// ours to the end) and (b) the headerbar gains the AdwTabButton.
pub fn onHeaderBarAttached(tv: *adw.ToolbarView, header: *gtk.Widget) void {
    const tvw = tv.as(gtk.Widget);
    const bin = gtk.Widget.getAncestor(tvw, adw.Bin.getGObjectType()) orelse return;
    if (!isTabBin(bin)) return;
    if (getData(bin, K_TAB_BAR)) |tb| {
        const bar: *adw.TabBar = @ptrCast(@alignCast(tb));
        if (gtk.Widget.getParent(bar.as(gtk.Widget)) == tvw) {
            adw.ToolbarView.remove(tv, bar.as(gtk.Widget));
            adw.ToolbarView.addTopBar(tv, bar.as(gtk.Widget));
        }
    }
    injectTabButton(bin, @ptrCast(@alignCast(header)));
}

fn injectTabButtonInto(bin: *gtk.Widget, tv: *adw.ToolbarView) void {
    // Walk the toolbarview's direct children for an already-attached
    // headerbar (child-before-parent commit order).
    var it = gtk.Widget.getFirstChild(tv.as(gtk.Widget));
    while (it) |w| : (it = gtk.Widget.getNextSibling(w)) {
        if (findHeaderBar(w)) |hb| {
            injectTabButton(bin, hb);
            return;
        }
    }
}

fn findHeaderBar(w: *gtk.Widget) ?*adw.HeaderBar {
    if (gobject.ext.isA(w, adw.HeaderBar)) return @ptrCast(@alignCast(w));
    var it = gtk.Widget.getFirstChild(w);
    while (it) |c| : (it = gtk.Widget.getNextSibling(c)) {
        if (findHeaderBar(c)) |hb| return hb;
    }
    return null;
}

fn injectTabButton(bin: *gtk.Widget, header: *adw.HeaderBar) void {
    if (hasFlag(header, K_BTN_DONE)) return;
    const btn = adw.TabButton.new();
    const btn_w: *gtk.Widget = @ptrCast(@alignCast(btn));
    if (owningView(bin)) |view| adw.TabButton.setView(btn, view);
    // overview.open resolves through the widget-tree action muxer — the
    // header lives inside the AdwTabOverview subtree, so no manual handler.
    gtk.Actionable.setActionName(@ptrCast(@alignCast(btn_w)), "overview.open");
    adw.HeaderBar.packEnd(header, btn_w);
    setFlag(header, K_BTN_DONE, true);
    setData(bin, K_TAB_BTN, btn);
}

// ---- props / events / commands ---------------------------------------------

/// Title routing: page bins retitle their AdwTabPage (and the scaffold window
/// when selected — GNOME windows title after the active tab); plain windows
/// keep gtk_window_set_title.
pub fn setTitle(widget: *gtk.Widget, title: [:0]const u8) void {
    if (!isTabBin(widget)) {
        gtk.Window.setTitle(@ptrCast(@alignCast(widget)), title);
        return;
    }
    const view = owningView(widget) orelse return;
    const page = adw.TabView.getPage(view, widget);
    adw.TabPage.setTitle(page, title);
    if (adw.TabView.getSelectedPage(view) == page) {
        if (owningWindow(widget)) |win| gtk.Window.setTitle(win, title);
    }
}

/// Generated connectEvents Window arm: record the node id on the handle so
/// close-page / the "+" button / scaffold close can address events. Plain
/// windows also report user closes (informational — close proceeds natively).
pub fn connectEvents(widget: *gtk.Widget, node_id: u32, emit_fn: EmitFn) void {
    emit = emit_fn;
    setData(widget, K_NODE_ID, @ptrFromInt(@as(usize, node_id)));
    if (!isTabBin(widget)) {
        // Ref + destroy marker: the node handle must stay addressable for the
        // remove op that follows the app's reaction to `closed`.
        _ = gobject.Object.ref(@ptrCast(@alignCast(widget)));
        _ = gtk.Window.signals.close_request.connect(@as(*gtk.Window, @ptrCast(@alignCast(widget))), ?*anyopaque, &onPlainWindowClose, null, .{});
        _ = gtk.Widget.signals.destroy.connect(widget, ?*anyopaque, &onPlainWindowDestroy, null, .{});
    }
}

pub fn command(widget: *gtk.Widget, cmd: []const u8, arg: ?std.json.Value) void {
    _ = arg;
    if (std.mem.eql(u8, cmd, "showTabOverview")) {
        const win = owningWindow(widget) orelse return;
        const raw = getData(win, K_OVERVIEW) orelse {
            std.debug.print("ND_WARN showTabOverview on a window with no tab group\n", .{});
            return;
        };
        adw.TabOverview.setOpen(@ptrCast(@alignCast(raw)), 1);
        return;
    }
    std.debug.print("ND_WARN unknown tabs command {s}\n", .{cmd});
}

/// The `window.close` semantic action (tree.zig remove arm): finish whatever
/// close is in flight, or start a confirmed one for a JS-initiated unmount.
/// Safe on already-closed handles — the framework ref keeps them valid.
pub fn closeNode(widget: *gtk.Widget) void {
    if (isTabBin(widget)) {
        defer cleanupBin(widget);
        const view = owningView(widget) orelse return; // page already gone
        const page = adw.TabView.getPage(view, widget);
        if (hasFlag(widget, K_PENDING)) {
            setFlag(widget, K_PENDING, false);
            adw.TabView.closePageFinish(view, page, 1);
        } else {
            setFlag(widget, K_CONFIRMED, true);
            adw.TabView.closePage(view, page);
        }
        return;
    }
    defer _ = gobject.Object.unref(@as(*gobject.Object, @ptrCast(@alignCast(widget))));
    if (hasFlag(widget, K_WIN_CLOSED)) return;
    gtk.Window.close(@ptrCast(@alignCast(widget)));
}

fn cleanupBin(bin: *gtk.Widget) void {
    setData(bin, K_TAB_BAR, null);
    setData(bin, K_TAB_BTN, null);
    _ = gobject.Object.unref(@as(*gobject.Object, @ptrCast(@alignCast(bin))));
}

// ---- signal handlers ---------------------------------------------------------

/// AdwTabView::close-page. JS-initiated closes (K_CONFIRMED) finish at once;
/// user closes hold the page pending and hand the decision to the app via
/// `closed` — the page visibly lingers only for the commit round-trip.
fn onClosePage(view: *adw.TabView, page: *adw.TabPage, _: ?*anyopaque) callconv(.c) c_int {
    const bin = adw.TabPage.getChild(page);
    if (hasFlag(bin, K_CONFIRMED)) {
        adw.TabView.closePageFinish(view, page, 1);
        reapIfEmptySoon(view);
        return 1; // GDK_EVENT_STOP
    }
    setFlag(bin, K_PENDING, true);
    if (binNodeId(bin)) |id| emitEmpty(id, "closed");
    return 1;
}

/// AdwTabView::create-window — a tab was dropped on the desktop. Spawn a
/// sibling scaffold in the same group and let AdwTabView transfer the page.
fn onCreateWindow(view: *adw.TabView, _: ?*anyopaque) callconv(.c) ?*adw.TabView {
    const group: *Group = @ptrCast(@alignCast(getData(view, K_GROUP) orelse return null));
    const src_win = owningWindow(view.as(gtk.Widget)) orelse return null;
    var w: c_int = 0;
    var h: c_int = 0;
    gtk.Window.getDefaultSize(src_win, &w, &h);
    const app: *gtk.Application = @ptrCast(@alignCast(gtk.Window.getApplication(src_win) orelse return null));
    const win = createScaffold(app, group, w, h) catch return null;
    return @ptrCast(@alignCast(getData(win, K_VIEW).?));
}

/// Page arrived (fresh append OR cross-window transfer): rebind the page's
/// injected chrome to the new owning view. Never touch page children here —
/// the libadwaita contract for transfers.
fn onPageAttached(view: *adw.TabView, page: *adw.TabPage, _: c_int, _: ?*anyopaque) callconv(.c) void {
    const bin = adw.TabPage.getChild(page);
    if (!isTabBin(bin)) return;
    if (getData(bin, K_TAB_BAR)) |tb| adw.TabBar.setView(@ptrCast(@alignCast(tb)), view);
    if (getData(bin, K_TAB_BTN)) |btn| adw.TabButton.setView(@ptrCast(@alignCast(btn)), view);
}

fn onPageDetached(view: *adw.TabView, _: *adw.TabPage, _: c_int, _: ?*anyopaque) callconv(.c) void {
    reapIfEmptySoon(view);
}

/// Close an emptied scaffold — but from idle, never mid-signal: during a
/// drag the detach fires while AdwTabView is still transferring, and closing
/// the source window under it crashes the gesture.
fn reapIfEmptySoon(view: *adw.TabView) void {
    _ = glib.idleAdd(&reapIdle, view);
}

fn reapIdle(data: ?*anyopaque) callconv(.c) c_int {
    const view: *adw.TabView = @ptrCast(@alignCast(data orelse return 0));
    if (adw.TabView.getNPages(view) == 0 and adw.TabView.getIsTransferringPage(view) == 0) {
        if (owningWindow(view.as(gtk.Widget))) |win| gtk.Window.close(win);
    }
    return 0; // G_SOURCE_REMOVE
}

fn onSelectedPage(view: *adw.TabView, _: ?*anyopaque, win: *gtk.Window) callconv(.c) void {
    const page = adw.TabView.getSelectedPage(view) orelse return;
    gtk.Window.setTitle(win, adw.TabPage.getTitle(page));
}

fn onWindowActive(win: *gtk.Window, _: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
    if (gtk.Window.isActive(win) == 0) return;
    const group: *Group = @ptrCast(@alignCast(getData(win, K_GROUP) orelse return));
    group.last_active = win;
}

fn onNewTabClicked(_: *gtk.Button, bin: *gtk.Widget) callconv(.c) void {
    const id = binNodeId(bin) orelse return;
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    if (owningWindow(bin)) |win| {
        if (getData(win, K_GROUP)) |g| {
            const group: *Group = @ptrCast(@alignCast(g));
            payload.put(alloc, "tabGroup", .{ .string = group.name }) catch {};
        }
    }
    if (emit) |f| f(id, "newTabRequested", .{ .data = .{ .object = payload } });
}

/// Scaffold close (user X / Alt+F4): report every member tab closed so the
/// app unmounts them, then let the close proceed — main.zig's last-window
/// shutdown logic runs unchanged after us.
/// Ctrl+W on a scaffold window: close the selected tab through the same
/// deferred close-page path as its X button (closePage -> onClosePage holds the
/// page pending and emits `closed` -> the app unmounts the <window>).
fn onWindowKey(_: *gtk.EventControllerKey, keyval: c_uint, _: c_uint, mods: gdk.ModifierType, win: *gtk.Window) callconv(.c) c_int {
    if (!mods.control_mask) return 0;
    if (keyval != 'w' and keyval != 'W') return 0;
    const raw = getData(win, K_VIEW) orelse return 0;
    const view: *adw.TabView = @ptrCast(@alignCast(raw));
    const page = adw.TabView.getSelectedPage(view) orelse return 0;
    adw.TabView.closePage(view, page);
    return 1; // GDK_EVENT_STOP
}

fn onScaffoldCloseRequest(win: *gtk.Window, _: ?*anyopaque) callconv(.c) c_int {
    const raw = getData(win, K_VIEW) orelse return 0;
    const view: *adw.TabView = @ptrCast(@alignCast(raw));
    const n = adw.TabView.getNPages(view);
    var i: c_int = 0;
    while (i < n) : (i += 1) {
        const page = adw.TabView.getNthPage(view, i);
        const bin = adw.TabPage.getChild(page);
        setFlag(bin, K_CONFIRMED, true); // window teardown destroys the page natively
        if (binNodeId(bin)) |id| emitEmpty(id, "closed");
    }
    return 0; // GDK_EVENT_PROPAGATE
}

fn onScaffoldDestroy(w: *gtk.Widget, _: ?*anyopaque) callconv(.c) void {
    const group: *Group = @ptrCast(@alignCast(getData(w, K_GROUP) orelse return));
    const win: *gtk.Window = @ptrCast(@alignCast(w));
    for (group.windows.items, 0..) |gw, i| {
        if (gw == win) {
            _ = group.windows.orderedRemove(i);
            break;
        }
    }
    if (group.last_active == win) group.last_active = if (group.windows.items.len > 0) group.windows.items[group.windows.items.len - 1] else null;
}

/// Plain (ungrouped) window user close: informational `closed`, native close
/// proceeds. The destroy marker keeps the later remove-op close a no-op.
fn onPlainWindowClose(win: *gtk.Window, _: ?*anyopaque) callconv(.c) c_int {
    const w = win.as(gtk.Widget);
    if (binNodeId(w)) |id| emitEmpty(id, "closed");
    return 0;
}

fn onPlainWindowDestroy(w: *gtk.Widget, _: ?*anyopaque) callconv(.c) void {
    setFlag(w, K_WIN_CLOSED, true);
}
