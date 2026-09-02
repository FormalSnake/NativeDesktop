const std = @import("std");
const gtk = @import("gtk");
const gdk = @import("gdk");
const gsk = @import("gsk");
const gobject = @import("gobject");
const gio = @import("gio");
const glib = @import("glib");
const graphene = @import("graphene");
const protocol = @import("../protocol.zig");
// Named import, not a relative path: `style.zig`'s dedicated
// test root can't reach `src/generated/widgets.zig` via `../` (see that
// file's import + build.zig for the full rationale), so it uses the named
// module "generated" instead. `style.zig` and `backend.zig` share one
// widget-tree instance, so both must resolve "generated" to the same
// module here too — a relative-path import here alongside style.zig's
// named one would instantiate `generated/widgets.zig` twice as two
// distinct (type-incompatible) modules.
const generated = @import("generated");
const style = @import("style.zig");
const overlay = @import("overlay.zig");
const abi = @import("../abi.zig");
const tree_mod = @import("../tree.zig");
const system = @import("system.zig");
const adw = @import("adw");
const ndtabs_gtk = @import("tabs.zig");
const ndpalette_gtk = @import("commandpalette.zig");
const ndsourcetree_gtk = @import("sourcetree.zig");
const ndwebview_gtk = @import("webview.zig");
const pango = @import("pango");

pub const Widget = gtk.Widget;

var arena_state = std.heap.ArenaAllocator.init(std.heap.page_allocator);
const arena = arena_state.allocator();
var the_window: ?*gtk.Window = null;
// The window the NEXT `snapshot` renders (multi-window). Set by
// `vtResolveWindow` (which automation.zig's selectSnapshotWindow drives before
// each screenshot) and consumed one-shot by `vtSnapshot`. Null falls back to
// the primary `the_window`, so single-window behavior is unchanged.
var snapshot_target_window: ?*gtk.Window = null;

/// Set by `main.zig` immediately after `nd_init()` returns, before
/// `nd_register_backend`/`nd_start_runtime` — every embedder->core call
/// (`nd_emit_event`, `show_overlay`'s bookkeeping) needs the context handle.
var the_ctx: ?*abi.NdContext = null;
pub fn setCtx(ctx: *abi.NdContext) void {
    the_ctx = ctx;
    system.setCtx(ctx);
}

/// Adapter from the generated dispatcher's typed `EmitFn` to the ABI's
/// embedder->core event channel: stringifies `payload` and calls
/// `nd_emit_event` directly — there is no core-installed sink function
/// pointer.
fn emitEventAdapter(node_id: u32, name: []const u8, payload: protocol.EventPayload) void {
    const ctx = the_ctx orelse return; // no context registered yet (shouldn't happen post-init)
    const name_z = arena.dupeZ(u8, name) catch return;
    const json = std.json.Stringify.valueAlloc(arena, payload, .{ .emit_null_optional_fields = false }) catch return;
    defer arena.free(json);
    const json_z = arena.dupeZ(u8, json) catch return;
    abi.nd_emit_event(ctx, node_id, name_z, json_z);
}

/// Wires the generated dispatcher's signal handlers + the style-error
/// reporter to the ABI event channel. Called once by `main.zig` before
/// `nd_start_runtime`, so it precedes the first `create` — the ordering
/// `generated.zig`'s `events_ready` assert relies on.
pub fn initEventsAndStyle() void {
    generated.initEvents(arena, &emitEventAdapter);
    style.init(arena, &styleErrorAdapter);
}

fn styleErrorAdapter(node_id: u32, key: []const u8) void {
    emitEventAdapter(node_id, "styleError", .{ .key = key });
}

pub fn connectEvents(widget: *gtk.Widget, kind: []const u8, node_id: u32) void {
    generated.connectEvents(widget, kind, node_id);
}

pub fn getWindow() ?*gtk.Window {
    return the_window;
}

fn dupeZ(s: []const u8) [:0]const u8 {
    return arena.dupeZ(u8, s) catch @panic("OOM in gtk_backend arena");
}

/// Menu bar: menu nodes (Menubar/Menu/MenuItem) ride the ordinary
/// create/append vtable ops but their handles are GMenu/GMenuItem GObjects,
/// not GtkWidgets. Every generic vtable op that would cast a stored handle to
/// *gtk.Widget must guard with this: `gobject.ext.isA` is safe on any valid
/// GObject and cleanly returns false for the non-Widget menu GObjects, so the
/// op no-ops instead of reinterpreting a GMenu as a GtkWidget.
fn isRealWidget(widget: *gtk.Widget) bool {
    return gobject.ext.isA(widget, gtk.Widget);
}

pub fn createWidget(app: *gtk.Application, kind: []const u8, props: ?std.json.Value) !*gtk.Widget {
    return generated.create(app, kind, props, &dupeZ, &the_window);
}

pub fn appendChild(parent: *gtk.Widget, parent_kind: []const u8, child: *gtk.Widget, attached: protocol.Attached) void {
    generated.appendChild(parent, parent_kind, child, attached, &dupeZ);
}

pub fn setText(widget: *gtk.Widget, text: []const u8) void {
    if (!isRealWidget(widget)) return; // menu node: no GtkLabel to set
    const label: *gtk.Label = @ptrCast(@alignCast(widget));
    gtk.Label.setText(label, dupeZ(text));
}

pub fn removeChild(parent: *gtk.Widget, parent_kind: []const u8, child: *gtk.Widget) void {
    generated.removeChild(parent, parent_kind, child);
}

pub fn insertBefore(parent: *gtk.Widget, parent_kind: []const u8, child: *gtk.Widget, before: ?*gtk.Widget, attached: protocol.Attached) void {
    generated.insertBefore(parent, parent_kind, child, before, attached, &dupeZ);
}

pub fn setVisible(widget: *gtk.Widget, visible: bool) void {
    if (!isRealWidget(widget)) return; // menu node: not a GtkWidget
    gtk.Widget.setVisible(widget, @intFromBool(visible));
}

pub fn applyProps(widget: *gtk.Widget, kind: []const u8, props: ?std.json.Value) void {
    // A NativeView's nested plugin props are routed by the core straight to
    // the plugin manager (tree.zig's update arm, via the retained view_kind);
    // the generated NativeView arm would forward them a second time through
    // its nd-view-kind stash, so skip it — that arm carries nothing else, and
    // host-level cssClasses still apply via vtApplyProps below.
    if (std.mem.eql(u8, kind, "NativeView")) return;
    generated.applyProps(widget, kind, props, &dupeZ);
}

pub fn widgetCommand(widget: *gtk.Widget, kind: []const u8, command: []const u8, arg: ?std.json.Value) void {
    if (!isRealWidget(widget)) return; // menu node: commands target real widgets only
    generated.widgetCommand(widget, kind, command, arg);
}

pub fn initStyle(sink_err: style.StyleErrorFn) void {
    style.init(arena, sink_err);
}

pub fn applyStyle(widget: *gtk.Widget, node_id: u32, style_value: std.json.Value) void {
    if (!isRealWidget(widget)) return; // menu node: no GtkWidget CSS surface
    // `padding` lands as CSS, which the Window content path cannot read back
    // off the widget when it decides whether the app already inset its root.
    if (style_value == .object and style_value.object.get("padding") != null) ndtabs_gtk.markAppPadding(widget);
    style.applyStyle(widget, node_id, style_value);
}

/// Routes `props.cssClasses` (if present) to `style.applyCssClasses`.
/// cssClasses rides in the ordinary props JSON, not a dedicated vtable
/// field — called from both `vtCreate` (right after widget creation) and
/// `vtApplyProps`.
fn applyCssClassesIfPresent(widget: *gtk.Widget, props: ?std.json.Value) void {
    const v = props orelse return;
    if (v != .object) return;
    if (v.object.get("cssClasses")) |cls| style.applyCssClasses(widget, cls);
}

/// Generation GC helpers: detach a swept widget from its parent
/// without destroying the parent or siblings.
pub fn hasParent(widget: *gtk.Widget) bool {
    if (!isRealWidget(widget)) return false; // menu node: never parented into a GtkWidget tree
    return gtk.Widget.getParent(widget) != null;
}

pub fn unparentWidget(widget: *gtk.Widget) void {
    if (!isRealWidget(widget)) return; // menu node: nothing to unparent
    // dev-mode GC sweep path: a doomed <paned> needs the same settle-timer
    // cancel here as the ordinary removeChild dispatch (AppKit peer:
    // Backend.swift's vt.unparent calling ndPanedTeardown).
    generated.ndPanedStructuralTeardown(widget);
    gtk.Widget.unparent(widget);
}

// ============================================================================
// C-ABI vtable fill: every `abi.NdBackend` field, wrapping the
// Zig-level functions above. `main.zig` calls `ndBackend()` once at startup
// and passes the result to `nd_register_backend`. Widget handles cross the
// ABI as `?*anyopaque`; every wrapper here casts back to `*gtk.Widget`
// (never any narrower concrete type — the generated dispatcher owns the
// per-kind casts internally).
// ============================================================================

var global_app: *gtk.Application = undefined;

/// Set once by `main.zig` before `nd_register_backend` — `create`'s vtable
/// signature carries no app handle (structural ops are widget/kind/props
/// only), so the GTK embedder keeps its own app reference here,
/// exactly like `the_window`.
pub fn setApp(app: *gtk.Application) void {
    global_app = app;
    system.setApp(app);
}

fn parseJson(json: [*:0]const u8) ?std.json.Parsed(std.json.Value) {
    const s = std.mem.span(json);
    if (s.len == 0) return null;
    return std.json.parseFromSlice(std.json.Value, arena, s, .{}) catch null;
}

fn vtCreate(_: *abi.NdContext, kind: [*:0]const u8, props_json: [*:0]const u8) callconv(.c) ?*anyopaque {
    const parsed = parseJson(props_json);
    const props: ?std.json.Value = if (parsed) |p| p.value else null;
    const widget = createWidget(global_app, std.mem.span(kind), props) catch return null;
    // The core's handle table OWNS one reference per node (dropped by
    // vtReleaseNode when the id leaves the tree). Without it the table was
    // unowned: a single-child container swap (ScrolledWindow.setChild,
    // Paned.setStartChild, ...) dropped the previous child's only sunk ref
    // with no `remove` op, and the next getTree's node_visible/a11y probes
    // read a freed GObject — the ~1-in-3 first-getTree segfault. ref_sink is
    // correct for both floating widgets and the non-floating GMenu/GMenuItem
    // menu handles (a plain ref there).
    _ = gobject.Object.refSink(widget.as(gobject.Object));
    applyCssClassesIfPresent(widget, props);
    if (std.mem.eql(u8, std.mem.span(kind), "Window")) system.attachWindowDropTarget(widget);
    return widget;
}

/// `vtable.release_node`: the balancing unref of vtCreate's ref_sink. The
/// widget object stays alive while a native parent still references it.
fn vtReleaseNode(_: *abi.NdContext, widget: ?*anyopaque) callconv(.c) void {
    const w: *gtk.Widget = @ptrCast(@alignCast(widget orelse return));
    gobject.Object.unref(w.as(gobject.Object));
}

fn vtApplyProps(_: *abi.NdContext, widget: ?*anyopaque, kind: [*:0]const u8, props_json: [*:0]const u8) callconv(.c) void {
    const parsed = parseJson(props_json);
    const props: ?std.json.Value = if (parsed) |p| p.value else null;
    const w: *gtk.Widget = @ptrCast(@alignCast(widget));
    applyProps(w, std.mem.span(kind), props);
    applyCssClassesIfPresent(w, props);
}

fn parseAttached(json: [*:0]const u8) protocol.Attached {
    const parsed = parseJson(json) orelse return .{};
    return protocol.Attached.fromProps(parsed.value);
}

fn vtAppendChild(_: *abi.NdContext, parent: ?*anyopaque, parent_kind: [*:0]const u8, child: ?*anyopaque, attached_json: [*:0]const u8) callconv(.c) void {
    appendChild(@ptrCast(@alignCast(parent)), std.mem.span(parent_kind), @ptrCast(@alignCast(child)), parseAttached(attached_json));
}

fn vtInsertBefore(_: *abi.NdContext, parent: ?*anyopaque, parent_kind: [*:0]const u8, child: ?*anyopaque, before: ?*anyopaque, attached_json: [*:0]const u8) callconv(.c) void {
    const before_widget: ?*gtk.Widget = if (before) |b| @ptrCast(@alignCast(b)) else null;
    insertBefore(@ptrCast(@alignCast(parent)), std.mem.span(parent_kind), @ptrCast(@alignCast(child)), before_widget, parseAttached(attached_json));
}

fn vtRemoveChild(_: *abi.NdContext, parent: ?*anyopaque, parent_kind: [*:0]const u8, child: ?*anyopaque) callconv(.c) void {
    removeChild(@ptrCast(@alignCast(parent)), std.mem.span(parent_kind), @ptrCast(@alignCast(child)));
}

fn vtSetText(_: *abi.NdContext, widget: ?*anyopaque, text: [*:0]const u8) callconv(.c) void {
    setText(@ptrCast(@alignCast(widget)), std.mem.span(text));
}

fn vtSetVisible(_: *abi.NdContext, widget: ?*anyopaque, visible: bool) callconv(.c) void {
    setVisible(@ptrCast(@alignCast(widget)), visible);
}

fn vtApplyStyle(_: *abi.NdContext, widget: ?*anyopaque, node_id: u32, style_json: [*:0]const u8) callconv(.c) void {
    const parsed = parseJson(style_json) orelse return;
    applyStyle(@ptrCast(@alignCast(widget)), node_id, parsed.value);
}

fn vtConnectEvents(_: *abi.NdContext, widget: ?*anyopaque, kind: [*:0]const u8, node_id: u32) callconv(.c) void {
    connectEvents(@ptrCast(@alignCast(widget)), std.mem.span(kind), node_id);
}

fn vtWidgetCommand(_: *abi.NdContext, widget: ?*anyopaque, kind: [*:0]const u8, command: [*:0]const u8, arg_json: [*:0]const u8) callconv(.c) void {
    const parsed = parseJson(arg_json);
    const arg: ?std.json.Value = if (parsed) |p| p.value else null;
    widgetCommand(@ptrCast(@alignCast(widget)), std.mem.span(kind), std.mem.span(command), arg);
}

fn vtHasParent(_: *abi.NdContext, widget: ?*anyopaque) callconv(.c) bool {
    return hasParent(@ptrCast(@alignCast(widget)));
}

fn vtUnparent(_: *abi.NdContext, widget: ?*anyopaque) callconv(.c) void {
    unparentWidget(@ptrCast(@alignCast(widget)));
}

fn vtGetWindow(_: *abi.NdContext) callconv(.c) ?*anyopaque {
    const w = getWindow() orelse return null;
    return w.as(gtk.Widget);
}

/// `vtable.resolve_window`: a plain GTK Window node's handle is the stable
/// gtk.Window widget (never orphaned the way AppKit's create-time content
/// view is once a SplitView takes over), so reconstruction rebinds to it
/// unchanged. A tab-member node's handle is its page bin (M17) — resolve
/// walks to the CURRENT owning scaffold window, which also keeps screenshot
/// targeting correct after a cross-window tab drag. Also records the
/// resolved window as the current snapshot target (multi-window):
/// automation.zig's selectSnapshotWindow calls this before each screenshot so
/// `vtSnapshot` renders the requested window rather than the global one.
fn vtResolveWindow(_: *abi.NdContext, handle: ?*anyopaque) callconv(.c) ?*anyopaque {
    const h = handle orelse return null;
    // Return the handle UNCHANGED — tree.zig rebinds the respawned node to
    // exactly this pointer, and for a tab member that must stay the page bin.
    // Only the snapshot target wants the real window under it.
    const w: *gtk.Widget = @ptrCast(@alignCast(h));
    snapshot_target_window = ndtabs_gtk.owningWindow(w) orelse @ptrCast(@alignCast(h));
    return handle;
}

/// `vtable.reparent_child`: relocate a live GtkWidget from `old_parent` to
/// `new_parent` WITHOUT destroying it. GTK containers own the single ref of
/// their child, so `gtk_widget_unparent` on the old container would drop the
/// last ref and finalize the widget mid-move — the loaded page of a WebKitGTK
/// view would be gone. Bracket the move in an explicit `g_object_ref`/`unref`
/// so the handle survives the gap, then reuse the ordinary per-kind remove +
/// insert paths so the target attaches correctly whatever the parent is. `old`
/// may be null (a still-detached pool widget shown in a window for the first
/// time).
fn vtReparentChild(
    _: *abi.NdContext,
    child: ?*anyopaque,
    old_parent: ?*anyopaque,
    old_parent_kind: [*:0]const u8,
    new_parent: ?*anyopaque,
    new_parent_kind: [*:0]const u8,
    before: ?*anyopaque,
    attached_json: [*:0]const u8,
) callconv(.c) void {
    const child_w: *gtk.Widget = @ptrCast(@alignCast(child));
    const new_parent_w: *gtk.Widget = @ptrCast(@alignCast(new_parent));
    const before_w: ?*gtk.Widget = if (before) |b| @ptrCast(@alignCast(b)) else null;
    const attached = parseAttached(attached_json);

    _ = gobject.Object.ref(child_w.as(gobject.Object));
    defer gobject.Object.unref(child_w.as(gobject.Object));
    if (old_parent) |op| {
        removeChild(@ptrCast(@alignCast(op)), std.mem.span(old_parent_kind), child_w);
    } else if (gtk.Widget.getParent(child_w) != null) {
        gtk.Widget.unparent(child_w);
    }
    insertBefore(new_parent_w, std.mem.span(new_parent_kind), child_w, before_w, attached);
}

const G_SOURCE_REMOVE: c_int = 0;

const MarshalJob = struct { fn_ptr: *const fn (?*anyopaque) callconv(.c) void, data: ?*anyopaque };

// Marshal jobs are the ONE arena user that crosses threads: vtMarshalAsync runs
// on the core's reader/child-watcher threads (it IS the core's hop onto the UI
// thread) while marshalTrampoline frees on the UI thread, which also owns every
// other arena user (dupeZ, event stringify, commit-batch parsing). `arena` is a
// plain ArenaAllocator — not thread-safe — so allocating jobs from it raced the
// UI thread's arena writes and corrupted the heap, surfacing as intermittent
// GTK segfaults (e.g. inside a later g_object_notify during commit apply). Jobs
// ride a thread-safe allocator instead, keeping `arena` UI-thread-exclusive.
const marshal_alloc = std.heap.smp_allocator;

fn marshalTrampoline(data: ?*anyopaque) callconv(.c) c_int {
    const job: *MarshalJob = @ptrCast(@alignCast(data.?));
    defer marshal_alloc.destroy(job);
    job.fn_ptr(job.data);
    return G_SOURCE_REMOVE;
}

/// `vtable.marshal_async`: the core's commit-apply/child-exit
/// paths call this instead of touching glib directly. Fills with
/// `g_main_context_invoke_full`.
fn vtMarshalAsync(_: *abi.NdContext, fn_ptr: *const fn (?*anyopaque) callconv(.c) void, data: ?*anyopaque) callconv(.c) void {
    const job = marshal_alloc.create(MarshalJob) catch return;
    job.* = .{ .fn_ptr = fn_ptr, .data = data };
    _ = glib.MainContext.default().invokeFull(glib.PRIORITY_DEFAULT, &marshalTrampoline, job, null);
}

var dev_mode: ?bool = null;
fn isDevMode() bool {
    if (dev_mode) |d| return d;
    const v = std.c.getenv("ND_DEV");
    const d = if (v) |val| std.mem.eql(u8, std.mem.span(val), "1") else false;
    dev_mode = d;
    return d;
}

/// Restart click trampoline: defers off the click signal's call stack (the
/// same re-entrancy reason overlay.zig's own `onRestartClicked` already
/// documents) and emits the reserved `nd_emit_event(ctx, 0, "restart", "{}")`
/// sentinel so `abi.zig` routes it to `Runtime.restart` instead
/// of forwarding a normal NDP event.
fn onRestartIdle(_: ?*anyopaque) callconv(.c) c_int {
    abi.nd_emit_event(the_ctx.?, 0, "restart", "{}");
    return G_SOURCE_REMOVE;
}
fn restartTrampoline() void {
    _ = glib.idleAdd(&onRestartIdle, null);
}

var the_tree: ?*tree_mod.Tree = null;

/// `vtable.show_overlay`: empty `message` is the clear
/// sentinel (see runtime.zig's `respawn`); `dev` gates the Restart button
/// (read from the embedder's own `ND_DEV` env — the ABI's `show_overlay`
/// carries only the message, so the embedder decides for itself, same as
/// documented in include/nd.h).
fn vtShowOverlay(_: *abi.NdContext, message: [*:0]const u8) callconv(.c) void {
    const tree = the_tree orelse return;
    const msg = std.mem.span(message);
    // A JS crash kills the one Bun process, so every window loses its live UI —
    // paint (and clear) the overlay on ALL open windows, not just the global
    // `the_window`.
    if (msg.len == 0) {
        overlay.clearAll(tree);
        return;
    }
    overlay.showAll(tree, global_app, msg, isDevMode(), &restartTrampoline);
}

/// Set once by `main.zig` before the first commit (so `show_overlay` can
/// reach the tree for its overlay-node bookkeeping) — mirrors `setApp`.
pub fn setTree(tree: *tree_mod.Tree) void {
    the_tree = tree;
}

fn vtNodeVisible(_: *abi.NdContext, widget: ?*anyopaque) callconv(.c) bool {
    const w: *gtk.Widget = @ptrCast(@alignCast(widget));
    // The palette's tracked node is an invisible host box; its real entry/list
    // live in the separately-presented dialog. Report actionable exactly while
    // presented so automation can drive it (and only then).
    if (ndpalette_gtk.isPaletteHandle(w)) return ndpalette_gtk.isPresented(w);
    // Menu nodes have no GtkWidget mapped state; treat them as actionable so a
    // MenuItem ref survives checkActionable and reaches semanticClick.
    if (!isRealWidget(w)) return true;
    if (gtk.Widget.getVisible(w) == 0) return false;
    // "mapped" folds into node_visible's contract (documented in
    // automation.zig's checkActionable comment).
    if (gtk.Widget.getMapped(w) == 0) return false;
    return withinClips(w);
}

/// Whether `w`'s allocation still intersects every clip between it and its
/// window: each enclosing GtkScrolledWindow's viewport, then the window
/// itself. GTK keeps a scrolled-away row mapped and fully allocated, so
/// `get_mapped` alone reported every row of a long list visible in a window
/// showing a handful of them. A node the user would have to scroll to reach
/// is not actionable, and `scrollIntoView` is what makes it so. `vtNodeBounds`
/// keeps reporting the untransformed allocation, so a caller can still see
/// where the node would be.
///
/// A degenerate allocation is left to the bounds half of the actionability
/// predicate, which already rejects it.
fn withinClips(w: *gtk.Widget) bool {
    var rect: graphene.Rect = undefined;
    var ancestor = gtk.Widget.getParent(w);
    while (ancestor) |a| : (ancestor = gtk.Widget.getParent(a)) {
        if (!gobject.ext.isA(a, gtk.ScrolledWindow)) continue;
        if (gtk.Widget.computeBounds(w, a, &rect) == 0) continue;
        if (!intersectsBox(rect, gtk.Widget.getWidth(a), gtk.Widget.getHeight(a))) return false;
    }
    const root = gtk.Widget.getRoot(w) orelse return true;
    const root_widget = root.as(gtk.Widget);
    if (root_widget == w) return true;
    if (gtk.Widget.computeBounds(w, root_widget, &rect) == 0) return true;
    return intersectsBox(rect, gtk.Widget.getWidth(root_widget), gtk.Widget.getHeight(root_widget));
}

fn intersectsBox(rect: graphene.Rect, width: c_int, height: c_int) bool {
    if (rect.f_size.f_width <= 0 or rect.f_size.f_height <= 0) return true;
    const w: f32 = @floatFromInt(width);
    const h: f32 = @floatFromInt(height);
    if (w <= 0 or h <= 0) return true;
    if (rect.f_origin.f_x >= w or rect.f_origin.f_y >= h) return false;
    return rect.f_origin.f_x + rect.f_size.f_width > 0 and rect.f_origin.f_y + rect.f_size.f_height > 0;
}

fn vtNodeBounds(_: *abi.NdContext, widget: ?*anyopaque, out: *abi.NdRect) callconv(.c) bool {
    const w: *gtk.Widget = @ptrCast(@alignCast(widget));
    // Palette host box: a zero-size tracked node whose real surface is the
    // presented dialog. Report a nominal non-degenerate rect so checkActionable
    // admits it for the routed setValue/type/click actions.
    if (ndpalette_gtk.isPaletteHandle(w)) {
        out.* = .{ .x = 0, .y = 0, .w = 1, .h = 1 };
        return true;
    }
    // Menu nodes have no geometry; report a nominal non-degenerate rect so
    // checkActionable (w>0 ∧ h>0) admits a MenuItem ref for semanticClick.
    if (!isRealWidget(w)) {
        out.* = .{ .x = 0, .y = 0, .w = 1, .h = 1 };
        return true;
    }
    // Per-window (multi-window): convert relative to the widget's OWN root
    // window, not a single global. A widget living in window B must report its
    // bounds in window B's coordinate space, and `getRoot` is its GtkWindow
    // ancestor.
    //
    // An UNROOTED widget has no bounds in anyone's coordinate space, and
    // substituting the primary window is not a harmless degenerate answer:
    // `gtk_widget_compute_bounds` requires the target to be an ancestor, and
    // GTK 4.22 faults rather than returning FALSE when it is not. A collapsed
    // GtkExpander's child is unrooted, so any getTree over a window holding one
    // took the process down as soon as a second window made the primary the
    // wrong guess.
    const root = gtk.Widget.getRoot(w) orelse return false;
    const root_widget: *gtk.Widget = root.as(gtk.Widget);
    var rect: graphene.Rect = undefined;
    const has_bounds = gtk.Widget.computeBounds(w, root_widget, &rect) != 0;
    if (has_bounds) {
        out.* = .{
            .x = @intFromFloat(rect.f_origin.f_x),
            .y = @intFromFloat(rect.f_origin.f_y),
            .w = @intFromFloat(rect.f_size.f_width),
            .h = @intFromFloat(rect.f_size.f_height),
        };
        return true;
    }
    if (gtk.Widget.getVisible(w) != 0) {
        out.* = .{ .x = 0, .y = 0, .w = gtk.Widget.getWidth(w), .h = gtk.Widget.getHeight(w) };
        return true;
    }
    return false;
}

/// `vtable.snapshot`: the GTK-native WidgetPaintable render path. Live
/// WebKit content rasterizes fine, headless cairo included. When the
/// pixel-true pass still fails after the retries below and webviews are in
/// the window, the snapshot DEGRADES instead of erroring: each webview is
/// hidden for the pass (child_visible keeps its allocation, nothing reflows)
/// and a flat placeholder plate is painted over its rect, with
/// `ND_SNAPSHOT_DEGRADED webviews=N` on stderr as the machine-readable
/// marker (the frozen ABI's `snapshot` returns only a bool).
fn vtSnapshot(_: *abi.NdContext, png_path: [*:0]const u8) callconv(.c) bool {
    // One-shot: the target set by the preceding `resolve_window` (see
    // selectSnapshotWindow) wins; consumed here so a later stray snapshot falls
    // back to the primary window instead of a stale target.
    const win = snapshot_target_window orelse getWindow() orelse return false;
    snapshot_target_window = null;
    const win_widget = win.as(gtk.Widget);

    const native = gtk.Widget.getNative(win_widget) orelse return false;
    var owned_renderer: ?*gsk.CairoRenderer = null;
    const renderer: *gsk.Renderer = gtk.Native.getRenderer(native) orelse blk: {
        const cairo_renderer = gsk.CairoRenderer.new();
        owned_renderer = cairo_renderer;
        const surface = gtk.Native.getSurface(native);
        _ = gsk.Renderer.realize(cairo_renderer.as(gsk.Renderer), surface, null);
        break :blk cairo_renderer.as(gsk.Renderer);
    };
    defer if (owned_renderer) |r| {
        gsk.Renderer.unrealize(r.as(gsk.Renderer));
        r.as(gsk.Renderer).unref();
    };

    // A WidgetPaintable renders NOTHING while its widget still owes a layout
    // pass, so a snapshot landing between a resize request and the next frame
    // frees to a null node even though the window is realized, mapped and
    // fully sized. Retry behind a frame-clock pump before concluding the
    // window cannot be rendered pixel-true. Headless webview windows have
    // needed as many as five ticks, so the budget is deliberately generous;
    // nothing pays for it unless the first pass already failed.
    if (renderWindowPng(win_widget, renderer, png_path, &.{})) return true;
    for (0..8) |_| {
        settleFrames(win_widget, 250_000);
        if (renderWindowPng(win_widget, renderer, png_path, &.{})) return true;
    }

    var views: std.ArrayList(WebViewRect) = .empty;
    defer views.deinit(std.heap.page_allocator);
    collectWebViews(win_widget, win_widget, &views);
    if (views.items.len == 0) return false;
    // Hide AFTER the rects are captured: computeBounds on an unmapped widget
    // answers a degenerate full-window rect, which would plate over the
    // window's chrome too.
    for (views.items) |v| gtk.Widget.setChildVisible(v.widget, 0);
    defer for (views.items) |v| gtk.Widget.setChildVisible(v.widget, 1);
    settleFrames(win_widget, 250_000);
    const ok = renderWindowPng(win_widget, renderer, png_path, views.items);
    if (ok) std.debug.print("ND_SNAPSHOT_DEGRADED webviews={d}\n", .{views.items.len});
    return ok;
}

/// Iterate the main context, BLOCKING, for up to `budget_us` so a window with
/// a pending layout or an unpainted frame gets a real compositor frame tick.
/// The snapshot job already runs on the UI thread and automation serves one
/// RPC at a time, so blocking here cannot reorder anything.
fn settleFrames(win_widget: *gtk.Widget, budget_us: i64) void {
    gtk.Widget.queueDraw(win_widget);
    const deadline = glib.getMonotonicTime() + budget_us;
    while (glib.getMonotonicTime() < deadline) {
        // Non-blocking iteration + sleep, never a may_block wait: an idle
        // context would otherwise park past the deadline.
        while (glib.MainContext.iteration(null, 0) != 0) {}
        glib.usleep(10_000);
    }
}

const WebViewRect = struct { widget: *gtk.Widget, rect: graphene.Rect };

/// Depth-first live-webview collection (with window-relative bounds) for the
/// degraded snapshot pass; a webview's own subtree is WebKit internals,
/// never descended into.
fn collectWebViews(win_widget: *gtk.Widget, root: *gtk.Widget, out: *std.ArrayList(WebViewRect)) void {
    if (ndwebview_gtk.isRealWebView(root)) {
        var rect: graphene.Rect = undefined;
        if (gtk.Widget.computeBounds(root, win_widget, &rect) != 0) {
            out.append(std.heap.page_allocator, .{ .widget = root, .rect = rect }) catch {};
        }
        return;
    }
    var child = gtk.Widget.getFirstChild(root);
    while (child) |c| : (child = gtk.Widget.getNextSibling(c)) collectWebViews(win_widget, c, out);
}

/// One WidgetPaintable render of `win_widget` to `png_path`. Each
/// `placeholders` entry gets a flat plate + centered "WebView" label drawn
/// over its rect (the degraded webview pass); empty for the pixel-true pass.
fn renderWindowPng(win_widget: *gtk.Widget, renderer: *gsk.Renderer, png_path: [*:0]const u8, placeholders: []const WebViewRect) bool {
    const paintable = gtk.WidgetPaintable.new(win_widget);
    defer paintable.unref();

    const snapshot = gtk.Snapshot.new();
    const width = gtk.Widget.getWidth(win_widget);
    const height = gtk.Widget.getHeight(win_widget);
    gdk.Paintable.snapshot(paintable.as(gdk.Paintable), snapshot.as(gdk.Snapshot), @floatFromInt(width), @floatFromInt(height));

    for (placeholders) |v| appendPlaceholder(snapshot, win_widget, v);

    const node = gtk.Snapshot.freeToNode(snapshot) orelse return false;
    defer gsk.RenderNode.unref(node);

    const texture = gsk.Renderer.renderTexture(renderer, node, null);
    defer texture.unref();

    return gdk.Texture.saveToPng(texture, png_path) != 0;
}

fn appendPlaceholder(snapshot: *gtk.Snapshot, win_widget: *gtk.Widget, entry: WebViewRect) void {
    const rect = entry.rect;
    // Mid-gray plate + white label: legible under both light and dark
    // Adwaita without consulting the theme.
    const plate = gdk.RGBA{ .f_red = 0.53, .f_green = 0.55, .f_blue = 0.58, .f_alpha = 1 };
    gtk.Snapshot.appendColor(snapshot, &plate, &rect);
    const layout = gtk.Widget.createPangoLayout(win_widget, "WebView");
    defer gobject.Object.unref(@ptrCast(@alignCast(layout)));
    var tw: c_int = 0;
    var th: c_int = 0;
    pango.Layout.getPixelSize(layout, &tw, &th);
    const text_color = gdk.RGBA{ .f_red = 1, .f_green = 1, .f_blue = 1, .f_alpha = 1 };
    gtk.Snapshot.save(snapshot);
    gtk.Snapshot.translate(snapshot, &graphene.Point{
        .f_x = rect.f_origin.f_x + (rect.f_size.f_width - @as(f32, @floatFromInt(tw))) / 2,
        .f_y = rect.f_origin.f_y + (rect.f_size.f_height - @as(f32, @floatFromInt(th))) / 2,
    });
    gtk.Snapshot.appendLayout(snapshot, layout, &text_color);
    gtk.Snapshot.restore(snapshot);
}

/// `vtable.semantic_action`: dispatches on `action` to the
/// click/setValue/type/scroll bodies (including the "emit `clicked`
/// directly, not `activate`" quirk and the GtkEditable `insertText(-1)`
/// contract). `result_json_out`/`err_json_out` are malloc'd (libc `free`,
/// so `nd_free` can release them uniformly across languages).
fn vtSemanticAction(
    _: *abi.NdContext,
    widget: ?*anyopaque,
    node_id: u32,
    action: [*:0]const u8,
    arg_json: [*:0]const u8,
    result_json_out: *?[*:0]u8,
    err_json_out: *?[*:0]u8,
) callconv(.c) i32 {
    const w: *gtk.Widget = @ptrCast(@alignCast(widget));
    const action_s = std.mem.span(action);
    const parsed = parseJson(arg_json);
    const args: ?std.json.Value = if (parsed) |p| p.value else null;

    // Menu nodes support "click" (→ menu dispatch) and "a11y" (which reads the
    // GMenuItem's declared state, below); setValue/type/scroll would cast the
    // GMenu handle to a GtkWidget, so reject them here.
    if (!isRealWidget(w)) {
        if (std.mem.eql(u8, action_s, "a11y")) return semanticMenuA11y(node_id, result_json_out);
        if (!std.mem.eql(u8, action_s, "click")) {
            setErr(err_json_out, node_id);
            return -32602;
        }
    }

    if (std.mem.eql(u8, action_s, "window.close")) {
        // Window-root unmount (tree.zig remove arm): close the native
        // window/tab — tabs.zig no-ops if the user already closed it.
        ndtabs_gtk.closeNode(w);
        return 0;
    } else if (ndpalette_gtk.isPaletteHandle(w) and
        (std.mem.eql(u8, action_s, "setValue") or std.mem.eql(u8, action_s, "type") or std.mem.eql(u8, action_s, "click")))
    {
        // Palette: route setValue/type/click to the real entry/list; a11y and
        // the rest fall through to the generic host-box handling below.
        return ndpalette_gtk.automationAction(w, node_id, action_s, args, result_json_out, err_json_out);
    } else if (std.mem.eql(u8, action_s, "click")) {
        return semanticClick(w, node_id, result_json_out, err_json_out);
    } else if (std.mem.eql(u8, action_s, "setValue")) {
        return semanticSetValue(w, node_id, args, result_json_out, err_json_out);
    } else if (std.mem.eql(u8, action_s, "type")) {
        return semanticType(w, node_id, args, result_json_out, err_json_out);
    } else if (std.mem.eql(u8, action_s, "scroll")) {
        return semanticScroll(w, node_id, args, result_json_out);
    } else if (std.mem.eql(u8, action_s, "rowAction")) {
        return semanticRowAction(w, node_id, args, result_json_out, err_json_out);
    } else if (std.mem.eql(u8, action_s, "webviewInfo")) {
        return semanticWebViewInfo(w, node_id, result_json_out, err_json_out);
    } else if (std.mem.eql(u8, action_s, "webviewEvalStart")) {
        return semanticWebViewEvalStart(w, node_id, args, result_json_out, err_json_out);
    } else if (std.mem.eql(u8, action_s, "webviewEvalPoll")) {
        return semanticWebViewEvalPoll(node_id, args, result_json_out, err_json_out);
    } else if (std.mem.eql(u8, action_s, "webviewPageText")) {
        return semanticWebViewPageText(w, node_id, result_json_out, err_json_out);
    } else if (std.mem.eql(u8, action_s, "a11y")) {
        return semanticA11y(w, node_id, result_json_out);
    } else if (std.mem.eql(u8, action_s, "windowState")) {
        return semanticWindowState(w, result_json_out);
    } else if (std.mem.eql(u8, action_s, "window.setFrame")) {
        return semanticWindowSetFrame(w, node_id, args, result_json_out, err_json_out);
    } else if (std.mem.eql(u8, action_s, "focus")) {
        return semanticFocus(w, result_json_out);
    } else if (std.mem.eql(u8, action_s, "scrollIntoView")) {
        return semanticScrollIntoView(w, result_json_out);
    } else if (std.mem.eql(u8, action_s, "snapshotNode")) {
        return semanticSnapshotNode(w, node_id, args, result_json_out, err_json_out);
    } else if (std.mem.eql(u8, action_s, "pointer") or std.mem.eql(u8, action_s, "drag") or
        std.mem.eql(u8, action_s, "keys") or std.mem.eql(u8, action_s, "doubleClick") or
        std.mem.eql(u8, action_s, "rightClick") or std.mem.eql(u8, action_s, "hover"))
    {
        // GTK4 removed app-constructible GdkEvents, so real input synthesis
        // is impossible in-process — a documented platform gap (-32003, the
        // rpc.json inputUnsupported code), not a missing arm. Semantic
        // equivalents (click/setValue/type/scroll) remain the Linux path.
        setErr(err_json_out, node_id);
        return -32003;
    }
    setErr(err_json_out, node_id);
    return -32601;
}

/// "a11y" for a MENU node. The handle is a GMenuItem, not a GtkWidget, so the
/// widget probe below cannot run on it — and without this every menu item
/// reported the probe's `enabled: true` default, making a disabled Back or
/// Forward item indistinguishable from an enabled one in getTree.
fn semanticMenuA11y(node_id: u32, result_json_out: *?[*:0]u8) i32 {
    const enabled = generated.menuItemEnabled(node_id) orelse true;
    const json = std.fmt.allocPrint(arena, "{{\"enabled\":{},\"focused\":false,\"value\":null}}", .{enabled}) catch return -32603;
    defer arena.free(json);
    result_json_out.* = mallocZ(json);
    return 0;
}

/// "a11y" — the live per-node accessibility probe behind getTree's
/// enabled/focused/value fields. Value reads mirror
/// `semanticSetValue`'s kind dispatch so both sides of the round-trip agree
/// on what a widget's value is.
fn semanticA11y(widget: *gtk.Widget, node_id: u32, result_json_out: *?[*:0]u8) i32 {
    const enabled = gtk.Widget.isSensitive(widget) != 0;
    // is-focus as well as has-focus: has-focus additionally requires the
    // toplevel to be ACTIVE, which it never is under a headless compositor
    // with no seat. A drive means "this is the window's focus widget", which
    // is what is-focus answers — and it matches what AppKit reports, where the
    // probe compares against the window's own firstResponder. The delegate is
    // asked too, because that is where focusTarget put the focus.
    const focus_widget = focusTarget(widget);
    const focused = gtk.Widget.hasFocus(widget) != 0 or gtk.Widget.isFocus(widget) != 0 or
        gtk.Widget.hasFocus(focus_widget) != 0 or gtk.Widget.isFocus(focus_widget) != 0;
    const kind = widgetKind(widget);

    var value_json: []const u8 = "null";
    var owned = false;
    if (std.mem.eql(u8, kind, "TextInput") or std.mem.eql(u8, kind, "SearchInput")) {
        const editable: *gtk.Editable = @ptrCast(@alignCast(widget));
        const text = std.mem.span(gtk.Editable.getText(editable));
        value_json = std.json.Stringify.valueAlloc(arena, text, .{}) catch "null";
        owned = true;
    } else if (std.mem.eql(u8, kind, "Checkbox") or std.mem.eql(u8, kind, "Radio")) {
        value_json = if (gtk.CheckButton.getActive(@ptrCast(@alignCast(widget))) != 0) "true" else "false";
    } else if (std.mem.eql(u8, kind, "Switch")) {
        value_json = if (gtk.Switch.getActive(@ptrCast(@alignCast(widget))) != 0) "true" else "false";
    } else if (std.mem.eql(u8, kind, "SwitchRow")) {
        value_json = if (adw.SwitchRow.getActive(@ptrCast(@alignCast(widget))) != 0) "true" else "false";
    } else if (std.mem.eql(u8, kind, "Slider")) {
        const range = @as(*gtk.Scale, @ptrCast(@alignCast(widget))).as(gtk.Range);
        value_json = std.fmt.allocPrint(arena, "{d}", .{gtk.Range.getValue(range)}) catch "null";
        owned = true;
    } else if (std.mem.eql(u8, kind, "Select")) {
        value_json = std.fmt.allocPrint(arena, "{d}", .{gtk.DropDown.getSelected(@ptrCast(@alignCast(widget)))}) catch "null";
        owned = true;
    } else if (std.mem.eql(u8, kind, "SourceList")) {
        const sw: *gtk.ScrolledWindow = @ptrCast(@alignCast(widget));
        const box: *gtk.ListBox = @ptrCast(@alignCast(generated.scrolledWindowInner(sw).?));
        if (gtk.ListBox.getSelectedRow(box)) |row| {
            value_json = std.fmt.allocPrint(arena, "{d}", .{gtk.ListBoxRow.getIndex(row)}) catch "null";
            owned = true;
        }
    } else if (std.mem.eql(u8, kind, "SourceTree")) {
        // Value is the selected node ID (id-addressed widget, not indexed).
        if (ndsourcetree_gtk.selectedIdOf(widget)) |id| {
            value_json = std.json.Stringify.valueAlloc(arena, id, .{}) catch "null";
            owned = true;
        }
    }
    defer if (owned) arena.free(@constCast(value_json));

    const extras = a11yExtrasJson(widget, kind, value_json);
    defer arena.free(@constCast(extras));
    const json = std.fmt.allocPrint(arena, "{{\"enabled\":{},\"focused\":{},\"value\":{s}{s}}}", .{ enabled, focused, value_json, extras }) catch return -32603;
    defer arena.free(json);
    result_json_out.* = mallocZ(json);
    _ = node_id;
    return 0;
}

/// Kinds whose a11y value IS an on/off state, so `checked` can be read back
/// off it rather than re-derived per widget class.
fn isCheckableKind(kind: []const u8) bool {
    for ([_][]const u8{ "Checkbox", "Radio", "Switch", "SwitchRow", "ToggleButton" }) |k| {
        if (std.mem.eql(u8, k, kind)) return true;
    }
    return false;
}

/// The kind-shaped half of the a11y probe, appended to the enabled/focused/
/// value trio. A field the node cannot carry is OMITTED, never reported
/// false: the core maps a missing key to null, which is what lets a caller
/// tell "unchecked" apart from "not a checkable thing". Caller frees.
fn a11yExtrasJson(widget: *gtk.Widget, kind: []const u8, value_json: []const u8) []const u8 {
    var out: std.ArrayList(u8) = .empty;
    if (isCheckableKind(kind) and (std.mem.eql(u8, value_json, "true") or std.mem.eql(u8, value_json, "false"))) {
        out.appendSlice(arena, ",\"checked\":") catch {};
        out.appendSlice(arena, value_json) catch {};
    }
    if (a11ySelected(widget)) |sel| {
        out.appendSlice(arena, if (sel) ",\"selected\":true" else ",\"selected\":false") catch {};
    }
    if (gobject.ext.isA(widget, gtk.Expander)) {
        const open = gtk.Expander.getExpanded(@ptrCast(@alignCast(widget))) != 0;
        out.appendSlice(arena, if (open) ",\"expanded\":true" else ",\"expanded\":false") catch {};
    }
    if (a11yPlaceholder(widget)) |p| appendJsonField(&out, "placeholder", p);
    if (a11yLabel(widget)) |l| appendJsonField(&out, "label", l);
    if (gobject.ext.isA(widget, gtk.DropDown)) appendOptions(&out, @ptrCast(@alignCast(widget)));
    return out.items;
}

fn appendJsonField(out: *std.ArrayList(u8), name: []const u8, value: []const u8) void {
    if (value.len == 0) return;
    const encoded = std.json.Stringify.valueAlloc(arena, value, .{}) catch return;
    defer arena.free(encoded);
    out.appendSlice(arena, ",\"") catch {};
    out.appendSlice(arena, name) catch {};
    out.appendSlice(arena, "\":") catch {};
    out.appendSlice(arena, encoded) catch {};
}

/// The choices a Select-shaped node offers, in index order, so `setValue`'s
/// integer index can be aimed by name.
fn appendOptions(out: *std.ArrayList(u8), dd: *gtk.DropDown) void {
    const model = gtk.DropDown.getModel(dd) orelse return;
    const list: *gtk.StringList = @ptrCast(@alignCast(model));
    const n = gio.ListModel.getNItems(model);
    out.appendSlice(arena, ",\"options\":[") catch {};
    var i: c_uint = 0;
    while (i < n) : (i += 1) {
        if (i > 0) out.appendSlice(arena, ",") catch {};
        const s = gtk.StringList.getString(list, i) orelse {
            out.appendSlice(arena, "\"\"") catch {};
            continue;
        };
        const encoded = std.json.Stringify.valueAlloc(arena, std.mem.span(s), .{}) catch continue;
        defer arena.free(encoded);
        out.appendSlice(arena, encoded) catch {};
    }
    out.appendSlice(arena, "]") catch {};
}

/// `selected` for a node drawn as a list row: GTK keeps that state on the
/// enclosing GtkListBoxRow, never on the app's own widget.
fn a11ySelected(widget: *gtk.Widget) ?bool {
    var cur: ?*gtk.Widget = widget;
    while (cur) |c| : (cur = gtk.Widget.getParent(c)) {
        if (gobject.ext.isA(c, gtk.ListBoxRow)) {
            return gtk.ListBoxRow.isSelected(@ptrCast(@alignCast(c))) != 0;
        }
    }
    return null;
}

fn a11yPlaceholder(widget: *gtk.Widget) ?[]const u8 {
    if (gobject.ext.isA(widget, gtk.Entry)) {
        const p = gtk.Entry.getPlaceholderText(@ptrCast(@alignCast(widget))) orelse return null;
        return std.mem.span(p);
    }
    // GtkSearchEntry is not a GtkEntry subclass, so it needs its own arm.
    if (gobject.ext.isA(widget, gtk.SearchEntry)) {
        const p = gtk.SearchEntry.getPlaceholderText(@ptrCast(@alignCast(widget))) orelse return null;
        return std.mem.span(p);
    }
    return null;
}

/// The node's spoken label where it says something `text` does not: a boxed
/// row's title, or the tooltip an icon-only control carries.
fn a11yLabel(widget: *gtk.Widget) ?[]const u8 {
    if (gobject.ext.isA(widget, adw.PreferencesRow)) {
        return std.mem.span(adw.PreferencesRow.getTitle(@ptrCast(@alignCast(widget))));
    }
    const tip = gtk.Widget.getTooltipText(widget) orelse return null;
    return std.mem.span(tip);
}

/// "windowState" — the frontmost-window probe behind the automation
/// `windows`/`resolve` ranking. Walks to the handle's own GtkWindow root
/// (a tab member's page-bin handle resolves to its CURRENT scaffold
/// window). GTK draws no key/main distinction, so `is-active` fills both.
/// Tab members answer per PAGE, not per scaffold: a background tab is not
/// visible and never key, and its title is its AdwTabPage's. Otherwise
/// every tab of a group would report the scaffold's identical state and
/// `windows` could not tell them apart.
fn semanticWindowState(widget: *gtk.Widget, result_json_out: *?[*:0]u8) i32 {
    const root = gtk.Widget.getRoot(widget) orelse {
        result_json_out.* = mallocZ("{\"key\":false,\"main\":false,\"visible\":false,\"title\":null}");
        return 0;
    };
    const win: *gtk.Window = @ptrCast(@alignCast(root));
    var active = gtk.Window.isActive(win) != 0;
    var visible = gtk.Widget.getVisible(win.as(gtk.Widget)) != 0;
    var title: ?[*:0]const u8 = gtk.Window.getTitle(win);
    if (ndtabs_gtk.isTabBin(widget)) {
        const selected = ndtabs_gtk.tabIsSelected(widget);
        active = active and selected;
        visible = visible and selected;
        if (ndtabs_gtk.tabTitle(widget)) |t| title = t;
    }
    var title_json: []const u8 = "null";
    var owned = false;
    if (title) |t| {
        if (std.json.Stringify.valueAlloc(arena, std.mem.span(t), .{}) catch null) |tj| {
            title_json = tj;
            owned = true;
        }
    }
    defer if (owned) arena.free(@constCast(title_json));
    const win_widget = win.as(gtk.Widget);
    const json = std.fmt.allocPrint(
        arena,
        "{{\"key\":{},\"main\":{},\"visible\":{},\"title\":{s},\"geometry\":{{\"x\":0,\"y\":0,\"w\":{d},\"h\":{d}}}}}",
        .{ active, active, visible, title_json, gtk.Widget.getWidth(win_widget), gtk.Widget.getHeight(win_widget) },
    ) catch return -32603;
    defer arena.free(json);
    result_json_out.* = mallocZ(json);
    return 0;
}

/// "window.setFrame" (peer of "window.close": the action string carries it,
/// no new vtable op). GTK4 has no client-side window placement, which is a
/// Wayland protocol decision rather than a missing binding, so x/y are
/// accepted and ignored and only the size lands.
fn semanticWindowSetFrame(widget: *gtk.Widget, node_id: u32, args: ?std.json.Value, result_json_out: *?[*:0]u8, err_json_out: *?[*:0]u8) i32 {
    const root = gtk.Widget.getRoot(widget) orelse return invalidValue(err_json_out, node_id);
    const win: *gtk.Window = @ptrCast(@alignCast(root));
    const win_widget = win.as(gtk.Widget);
    const o = objectArg(args);
    const width = if (o) |obj| intArg(obj.get("width")) else null;
    const height = if (o) |obj| intArg(obj.get("height")) else null;
    var applied = true;
    if (width != null or height != null) {
        const want_w = width orelse gtk.Widget.getWidth(win_widget);
        const want_h = height orelse gtk.Widget.getHeight(win_widget);
        gtk.Window.setDefaultSize(win, want_w, want_h);
        applied = settleWindowSize(win_widget, want_w, want_h, 500_000);
    }
    setResult(result_json_out, .{
        .ok = true,
        .applied = applied,
        .width = gtk.Widget.getWidth(win_widget),
        .height = gtk.Widget.getHeight(win_widget),
    });
    return 0;
}

/// Pump the main context until the window's allocation matches the size just
/// requested. A Wayland resize is a round trip through the compositor, so
/// without this the caller (and the WindowInfo the core re-probes right after
/// this arm) reads the PRE-resize allocation. Bounded, because a compositor
/// that refuses the size must not hang the RPC; false means the allocation
/// never converged and whatever is reported is the real one.
fn settleWindowSize(win_widget: *gtk.Widget, want_w: c_int, want_h: c_int, budget_us: i64) bool {
    const deadline = glib.getMonotonicTime() + budget_us;
    while (true) {
        if (gtk.Widget.getWidth(win_widget) == want_w and gtk.Widget.getHeight(win_widget) == want_h) return true;
        if (glib.getMonotonicTime() >= deadline) return false;
        while (glib.MainContext.iteration(null, 0) != 0) {}
        glib.usleep(5_000);
    }
}

fn intArg(v: ?std.json.Value) ?c_int {
    return switch (v orelse return null) {
        .integer => |i| @intCast(i),
        .float => |f| @intFromFloat(f),
        else => null,
    };
}

/// "focus" is the same path the universal `focus` widget command takes
/// (generated `ndGrabFocus`), mirrored here rather than called: the generated
/// helper is file-private and dispatch there needs the tracked kind string,
/// which the vtable call does not carry.
fn semanticFocus(widget: *gtk.Widget, result_json_out: *?[*:0]u8) i32 {
    if (isRealWidget(widget)) _ = gtk.Widget.grabFocus(focusTarget(widget));
    setResult(result_json_out, .{ .ok = true });
    return 0;
}

/// The widget that ends up holding the keyboard focus for a tracked handle.
/// GtkEntry and GtkSearchEntry delegate to an internal GtkText, and a
/// GtkScrolledWindow to the view it wraps, so after grab_focus the root's
/// focus widget is one level BELOW the handle. The a11y probe has to look
/// where the arm aimed, or a focused text field reads back unfocused.
fn focusTarget(widget: *gtk.Widget) *gtk.Widget {
    if (gobject.ext.isA(widget, gtk.ScrolledWindow)) {
        if (generated.scrolledWindowInner(@ptrCast(@alignCast(widget)))) |inner| return inner;
    }
    if (gobject.ext.isA(widget, gtk.Editable)) {
        if (gtk.Editable.getDelegate(@ptrCast(@alignCast(widget)))) |d| return @ptrCast(@alignCast(d));
    }
    return widget;
}

/// "scrollIntoView" scrolls the nearest GtkScrolledWindow ancestor by the
/// smallest amount that brings the widget inside the viewport. Computing the
/// delta from `compute_bounds` (which already carries the viewport's scroll
/// translation) works for every scrolled widget, unlike the per-widget
/// scroll-to actions GtkListView and GtkColumnView expose.
fn semanticScrollIntoView(widget: *gtk.Widget, result_json_out: *?[*:0]u8) i32 {
    const sw = enclosingScrolledWindow(widget) orelse {
        setResult(result_json_out, .{ .ok = true, .scrolled = false });
        return 0;
    };
    const sw_widget = sw.as(gtk.Widget);
    var rect: graphene.Rect = undefined;
    if (gtk.Widget.computeBounds(widget, sw_widget, &rect) != 0) {
        scrollAxis(gtk.ScrolledWindow.getHadjustment(sw), rect.f_origin.f_x, rect.f_size.f_width, @floatFromInt(gtk.Widget.getWidth(sw_widget)));
        scrollAxis(gtk.ScrolledWindow.getVadjustment(sw), rect.f_origin.f_y, rect.f_size.f_height, @floatFromInt(gtk.Widget.getHeight(sw_widget)));
    }
    setResult(result_json_out, .{ .ok = true, .scrolled = true });
    return 0;
}

fn enclosingScrolledWindow(widget: *gtk.Widget) ?*gtk.ScrolledWindow {
    var cur = gtk.Widget.getParent(widget);
    while (cur) |c| : (cur = gtk.Widget.getParent(c)) {
        if (gobject.ext.isA(c, gtk.ScrolledWindow)) return @ptrCast(@alignCast(c));
    }
    return null;
}

/// One axis of scrollIntoView: `pos`/`size` are the widget's rect in the
/// viewport's own space and `viewport` its length. Scrolls by the shortfall
/// at whichever edge the widget overhangs, and not at all when it fits.
fn scrollAxis(adj: *gtk.Adjustment, pos: f32, size: f32, viewport: f32) void {
    if (size <= 0 or viewport <= 0) return;
    const value = gtk.Adjustment.getValue(adj);
    var delta: f64 = 0;
    if (pos < 0) {
        delta = pos;
    } else if (pos + size > viewport) {
        delta = @min(pos + size - viewport, pos);
    }
    if (delta == 0) return;
    gtk.Adjustment.setValue(adj, value + delta);
}

/// "snapshotNode" renders the node's OWN surface to a PNG through the same
/// WidgetPaintable path `vtSnapshot` uses for a window. Rendering the handle
/// rather than cropping a window capture ties the PNG's pixel dimensions to
/// the logical rect `getTree` reports.
fn semanticSnapshotNode(widget: *gtk.Widget, node_id: u32, args: ?std.json.Value, result_json_out: *?[*:0]u8, err_json_out: *?[*:0]u8) i32 {
    const o = objectArg(args) orelse return invalidValue(err_json_out, node_id);
    const path = switch (o.get("path") orelse return invalidValue(err_json_out, node_id)) {
        .string => |s| s,
        else => return invalidValue(err_json_out, node_id),
    };
    if (!isRealWidget(widget)) return invalidValue(err_json_out, node_id);
    if (gtk.Widget.getWidth(widget) <= 0 or gtk.Widget.getHeight(widget) <= 0) return invalidValue(err_json_out, node_id);
    const native = gtk.Widget.getNative(widget) orelse return invalidValue(err_json_out, node_id);
    const renderer = gtk.Native.getRenderer(native) orelse return invalidValue(err_json_out, node_id);
    const path_z = arena.dupeZ(u8, path) catch return -32603;
    defer arena.free(path_z);
    if (!renderWidgetPng(widget, renderer, path_z)) return -32603;
    setResult(result_json_out, .{ .ref = node_id });
    return 0;
}

/// One WidgetPaintable render of `widget` alone to `png_path`.
fn renderWidgetPng(widget: *gtk.Widget, renderer: *gsk.Renderer, png_path: [*:0]const u8) bool {
    const paintable = gtk.WidgetPaintable.new(widget);
    defer paintable.unref();
    const width = gtk.Widget.getWidth(widget);
    const height = gtk.Widget.getHeight(widget);
    const snapshot = gtk.Snapshot.new();
    gdk.Paintable.snapshot(
        paintable.as(gdk.Paintable),
        snapshot.as(gdk.Snapshot),
        @floatFromInt(width),
        @floatFromInt(height),
    );
    const node = gtk.Snapshot.freeToNode(snapshot) orelse return false;
    defer gsk.RenderNode.unref(node);
    // An explicit viewport, taken from the same compute_bounds rect
    // `vtNodeBounds` reports, so the PNG's pixel size IS the rect getTree
    // gives for the node. With a null viewport the texture takes the render
    // node's own bounds instead: the ink of whatever the widget drew, which
    // for a label was 71x15 inside its 608x28 row. That rect is the CSS
    // border box, wider than get_width by the padding, and its origin sits
    // outside the content origin, so it has to be honoured and not just
    // measured. A window gets away with null because its own background node
    // already spans the whole surface.
    const viewport = nodeViewport(widget, width, height);
    const texture = gsk.Renderer.renderTexture(renderer, node, &viewport);
    defer texture.unref();
    return gdk.Texture.saveToPng(texture, png_path) != 0;
}

/// The rect `snapshotNode` renders, matched to what `vtNodeBounds` reports.
fn nodeViewport(widget: *gtk.Widget, width: c_int, height: c_int) graphene.Rect {
    var rect: graphene.Rect = undefined;
    if (gtk.Widget.computeBounds(widget, widget, &rect) != 0 and
        rect.f_size.f_width > 0 and rect.f_size.f_height > 0) return rect;
    return .{
        .f_origin = .{ .f_x = 0, .f_y = 0 },
        .f_size = .{ .f_width = @floatFromInt(width), .f_height = @floatFromInt(height) },
    };
}

/// Mallocs a NUL-terminated copy of `json` for a `*_json_out` param (freed
/// via `nd_free`/libc `free` by the core).
fn mallocZ(json: []const u8) ?[*:0]u8 {
    const buf: [*]u8 = @ptrCast(std.c.malloc(json.len + 1) orelse return null);
    @memcpy(buf[0..json.len], json);
    buf[json.len] = 0;
    return @ptrCast(buf);
}

fn setResult(out: *?[*:0]u8, value: anytype) void {
    const json = std.json.Stringify.valueAlloc(arena, value, .{}) catch return;
    defer arena.free(json);
    out.* = mallocZ(json);
}

fn setErr(out: *?[*:0]u8, node_id: u32) void {
    const json = std.fmt.allocPrint(arena, "{{\"ref\":{d}}}", .{node_id}) catch return;
    defer arena.free(json);
    out.* = mallocZ(json);
}

fn semanticClick(widget: *gtk.Widget, node_id: u32, result_json_out: *?[*:0]u8, err_json_out: *?[*:0]u8) i32 {
    if (!isRealWidget(widget)) {
        // Menu node: dispatch the item's GAction (custom onSelect fires
        // "selected"; a disabled item's action is a no-op, so onSelect does
        // not fire and app state is unchanged).
        _ = generated.menuSemanticClick(node_id);
        setResult(result_json_out, .{ .ref = node_id, .dispatched = true });
        return 0;
    }
    if (std.mem.eql(u8, widgetKind(widget), "SourceList")) {
        // SourceList: "click" activates the
        // currently-selected row, or the first row if none is selected.
        // `gtk.Widget.activate` on the GtkListBoxRow was tried first but
        // does not reliably raise the ListBox's "row-activated" — same
        // class of quirk as Button's activate() below, fixed the same
        // way: emit the ListBox's signal directly, bypassing whatever
        // internal activation gating drops it.
        const sw: *gtk.ScrolledWindow = @ptrCast(@alignCast(widget));
        const box: *gtk.ListBox = @ptrCast(@alignCast(generated.scrolledWindowInner(sw).?));
        const row = gtk.ListBox.getSelectedRow(box) orelse gtk.ListBox.getRowAtIndex(box, 0);
        if (row) |r| gobject.signalEmitByName(@ptrCast(@alignCast(box)), "row-activated", r);
        setResult(result_json_out, .{ .ref = node_id, .dispatched = true });
        return 0;
    }
    if (std.mem.eql(u8, widgetKind(widget), "SourceTree")) {
        // "click" activates the selected row (first selectable row when
        // nothing is selected), emitting rowActivated {nodeId}.
        _ = ndsourcetree_gtk.semanticActivate(widget);
        setResult(result_json_out, .{ .ref = node_id, .dispatched = true });
        return 0;
    }
    const kind = widgetKind(widget);
    if (std.mem.eql(u8, kind, "SwitchRow")) {
        // The row's activatable-widget is its switch: activate toggles it,
        // firing notify::active exactly like a user click.
        _ = gtk.Widget.activate(widget);
        setResult(result_json_out, .{ .ref = node_id, .dispatched = true });
        return 0;
    }
    if (std.mem.eql(u8, kind, "Row")) {
        adw.ActionRow.activate(@ptrCast(@alignCast(widget))); // emits "activated"
        setResult(result_json_out, .{ .ref = node_id, .dispatched = true });
        return 0;
    }
    if (std.mem.eql(u8, kind, "Checkbox") or std.mem.eql(u8, kind, "Switch")) {
        // GtkCheckButton and GtkSwitch are not GtkButton subclasses in GTK4 —
        // they have no `clicked` signal, so the generic emit below is a silent
        // no-op for them. Widget.activate runs their real activation path:
        // toggles `active`, fires `toggled`/state change, and keeps
        // radio-group semantics (an already-active grouped radio stays
        // active, exactly like a user click).
        _ = gtk.Widget.activate(widget);
        setResult(result_json_out, .{ .ref = node_id, .dispatched = true });
        return 0;
    }
    // A widget without a `clicked` signal (GtkNotebook, GtkListBoxRow, plain
    // containers) must fail honestly: emitting anyway raises a GLib CRITICAL
    // and the client would get dispatched=true for a click that did nothing.
    const winstance: *gobject.TypeInstance = @ptrCast(@alignCast(widget));
    if (gobject.signalLookup("clicked", winstance.f_g_class.?.f_g_type) == 0) {
        setErr(err_json_out, node_id);
        return -32001;
    }
    // Semantic dispatch: emit `clicked` directly. `gtk.Widget.activate` was
    // tried first but only re-emits `clicked` on the first call per main-loop
    // settle under weston headless — rapid successive activate() calls
    // silently drop clicks 2+. Emitting the signal directly bypasses GTK's
    // activate/gesture state machine and fires reliably on every call.
    gobject.signalEmitByName(@ptrCast(@alignCast(widget)), "clicked");
    setResult(result_json_out, .{ .ref = node_id, .dispatched = true });
    return 0;
}

fn semanticSetValue(widget: *gtk.Widget, node_id: u32, args: ?std.json.Value, result_json_out: *?[*:0]u8, err_json_out: *?[*:0]u8) i32 {
    const v = if (args) |a| (if (a == .object) a.object.get("value") else null) else null;
    const value = v orelse {
        setErr(err_json_out, node_id);
        return -32602;
    };
    const kind = widgetKind(widget);

    if (std.mem.eql(u8, kind, "TextInput")) {
        if (value != .string) return invalidValue(err_json_out, node_id);
        const z = arena.dupeZ(u8, value.string) catch return -32602;
        const editable = @as(*gtk.Entry, @ptrCast(@alignCast(widget))).as(gtk.Editable);
        gtk.Editable.setText(editable, z); // fires "changed" -> Event -> React (by design)
    } else if (std.mem.eql(u8, kind, "SearchInput")) {
        // GtkSearchEntry is not a GtkEntry subclass (unlike TextInput) but
        // does implement gtk.Editable — same setText-fires-changed contract.
        if (value != .string) return invalidValue(err_json_out, node_id);
        const z = arena.dupeZ(u8, value.string) catch return -32602;
        const editable = @as(*gtk.SearchEntry, @ptrCast(@alignCast(widget))).as(gtk.Editable);
        gtk.Editable.setText(editable, z); // fires "changed" -> Event -> React (by design)
    } else if (std.mem.eql(u8, kind, "TextArea")) {
        if (value != .string) return invalidValue(err_json_out, node_id);
        const z = arena.dupeZ(u8, value.string) catch return -32602;
        // TextArea is tracked by its GtkScrolledWindow wrapper, not the view.
        const sw: *gtk.ScrolledWindow = @ptrCast(@alignCast(widget));
        const inner = generated.scrolledWindowInner(sw) orelse return invalidValue(err_json_out, node_id);
        const view: *gtk.TextView = @ptrCast(@alignCast(inner));
        gtk.TextBuffer.setText(gtk.TextView.getBuffer(view), z, -1);
    } else if (std.mem.eql(u8, kind, "Checkbox") or std.mem.eql(u8, kind, "Radio")) {
        if (value != .bool) return invalidValue(err_json_out, node_id);
        gtk.CheckButton.setActive(@ptrCast(@alignCast(widget)), @intFromBool(value.bool));
    } else if (std.mem.eql(u8, kind, "Switch")) {
        if (value != .bool) return invalidValue(err_json_out, node_id);
        gtk.Switch.setActive(@ptrCast(@alignCast(widget)), @intFromBool(value.bool));
    } else if (std.mem.eql(u8, kind, "SwitchRow")) {
        if (value != .bool) return invalidValue(err_json_out, node_id);
        adw.SwitchRow.setActive(@ptrCast(@alignCast(widget)), @intFromBool(value.bool)); // fires notify::active
    } else if (std.mem.eql(u8, kind, "Slider")) {
        const num: f64 = switch (value) {
            .float => value.float,
            .integer => @floatFromInt(value.integer),
            else => return invalidValue(err_json_out, node_id),
        };
        // GtkAdjustment-backed: Range.setValue drives the Scale's adjustment
        // and clamps to [min, max]; fires "value-changed".
        const range = @as(*gtk.Scale, @ptrCast(@alignCast(widget))).as(gtk.Range);
        gtk.Range.setValue(range, num);
    } else if (std.mem.eql(u8, kind, "Select")) {
        if (value != .integer) return invalidValue(err_json_out, node_id);
        gtk.DropDown.setSelected(@ptrCast(@alignCast(widget)), @intCast(value.integer)); // fires notify::selected
    } else if (std.mem.eql(u8, kind, "SourceList")) {
        if (value != .integer) return invalidValue(err_json_out, node_id);
        const sw: *gtk.ScrolledWindow = @ptrCast(@alignCast(widget));
        const box: *gtk.ListBox = @ptrCast(@alignCast(generated.scrolledWindowInner(sw).?));
        if (gtk.ListBox.getRowAtIndex(box, @intCast(value.integer))) |row| {
            gtk.ListBox.selectRow(box, row); // fires "row-selected" -> Event -> React (by design)
        } else {
            return invalidValue(err_json_out, node_id);
        }
    } else if (std.mem.eql(u8, kind, "SourceTree")) {
        // Id-addressed: value is a node ID string ("" deselects). The module
        // emits selectionChanged itself rather than riding "row-selected",
        // which GtkListBox raises only when the selection moves. See
        // sourcetree.zig's semanticSelect.
        if (value != .string) return invalidValue(err_json_out, node_id);
        if (!ndsourcetree_gtk.semanticSelect(widget, value.string)) return invalidValue(err_json_out, node_id);
    } else {
        setErr(err_json_out, node_id);
        return -32602;
    }
    setResult(result_json_out, .{ .ref = node_id, .applied = true });
    return 0;
}

fn invalidValue(err_json_out: *?[*:0]u8, node_id: u32) i32 {
    setErr(err_json_out, node_id);
    return -32602;
}

/// "rowAction" {actionId, testId?}, SourceTree only: dispatches a row's
/// trailing action as if its button were clicked (actionClicked
/// {nodeId, actionId}). testId picks the row by its per-node testID; absent,
/// the selected row is the target.
fn semanticRowAction(widget: *gtk.Widget, node_id: u32, args: ?std.json.Value, result_json_out: *?[*:0]u8, err_json_out: *?[*:0]u8) i32 {
    if (!std.mem.eql(u8, widgetKind(widget), "SourceTree")) return invalidValue(err_json_out, node_id);
    const obj: ?std.json.ObjectMap = if (args) |a| (if (a == .object) a.object else null) else null;
    const o = obj orelse return invalidValue(err_json_out, node_id);
    const action_id = switch (o.get("actionId") orelse return invalidValue(err_json_out, node_id)) {
        .string => |s| s,
        else => return invalidValue(err_json_out, node_id),
    };
    const test_id: ?[]const u8 = if (o.get("testId")) |t| (if (t == .string) t.string else null) else null;
    if (!ndsourcetree_gtk.semanticRowAction(widget, action_id, test_id)) return invalidValue(err_json_out, node_id);
    setResult(result_json_out, .{ .ref = node_id, .dispatched = true });
    return 0;
}

/// "webviewInfo" — the live {url, title, loading, canGoBack, canGoForward}
/// behind the automation `webviewInfo` RPC and waitFor's
/// urlContains/pageTitleContains predicates. Read off the engine, so a drive
/// never needs the app to forward navigate/titleChanged.
fn semanticWebViewInfo(widget: *gtk.Widget, node_id: u32, result_json_out: *?[*:0]u8, err_json_out: *?[*:0]u8) i32 {
    const i = ndwebview_gtk.info(widget) orelse return invalidValue(err_json_out, node_id);
    setResult(result_json_out, .{
        .ref = node_id,
        .url = i.url,
        .title = i.title,
        .loading = i.loading,
        .canGoBack = i.can_go_back,
        .canGoForward = i.can_go_forward,
    });
    return 0;
}

/// "webviewEvalStart" {code, world?} — kicks the engine's asynchronous
/// evaluation and answers its id. The automation thread polls
/// "webviewEvalPoll" until it settles; the vtable call itself cannot block,
/// since it runs ON the UI thread that has to drive the evaluation.
fn semanticWebViewEvalStart(widget: *gtk.Widget, node_id: u32, args: ?std.json.Value, result_json_out: *?[*:0]u8, err_json_out: *?[*:0]u8) i32 {
    const o = objectArg(args) orelse return invalidValue(err_json_out, node_id);
    const code = switch (o.get("code") orelse return invalidValue(err_json_out, node_id)) {
        .string => |s| s,
        else => return invalidValue(err_json_out, node_id),
    };
    const world: ?[]const u8 = if (o.get("world")) |w| (if (w == .string) w.string else null) else null;
    const id = ndwebview_gtk.evalStart(widget, code, world) orelse return invalidValue(err_json_out, node_id);
    setResult(result_json_out, .{ .evalId = id });
    return 0;
}

/// "webviewEvalPoll" {evalId} — one non-blocking look at an eval, releasing it
/// once it has settled so the caller must read the outcome exactly once.
fn semanticWebViewEvalPoll(node_id: u32, args: ?std.json.Value, result_json_out: *?[*:0]u8, err_json_out: *?[*:0]u8) i32 {
    const o = objectArg(args) orelse return invalidValue(err_json_out, node_id);
    const id: u64 = switch (o.get("evalId") orelse return invalidValue(err_json_out, node_id)) {
        .integer => |n| if (n > 0) @intCast(n) else return invalidValue(err_json_out, node_id),
        else => return invalidValue(err_json_out, node_id),
    };
    const state = ndwebview_gtk.evalPoll(id) orelse return invalidValue(err_json_out, node_id);
    setResult(result_json_out, .{ .done = state.done, .ok = state.ok, .value = state.value, .@"error" = state.err });
    if (state.done) ndwebview_gtk.evalRelease(id);
    return 0;
}

/// "webviewPageText" — the throttled `document.body.innerText` cache behind
/// waitFor's pageTextContains. Null text means "no probe has answered yet",
/// which the predicate treats as not-matching-yet rather than an error.
fn semanticWebViewPageText(widget: *gtk.Widget, node_id: u32, result_json_out: *?[*:0]u8, err_json_out: *?[*:0]u8) i32 {
    if (!ndwebview_gtk.isRealWebView(widget)) return invalidValue(err_json_out, node_id);
    setResult(result_json_out, .{ .text = ndwebview_gtk.pageText(widget) });
    return 0;
}

fn objectArg(args: ?std.json.Value) ?std.json.ObjectMap {
    const a = args orelse return null;
    return if (a == .object) a.object else null;
}

/// Best-effort widget-kind sniff for the semantic-action dispatch: the
/// vtable call carries only the widget handle + node_id, not the tracked
/// `widget_type` string (that lives in `Tree.meta`, core-owned) — so this
/// mirrors it structurally via GObject's runtime type name, matching the
/// exact widget classes `generated.create` constructs per kind.
fn widgetKind(widget: *gtk.Widget) []const u8 {
    const instance: *gobject.TypeInstance = @ptrCast(@alignCast(widget));
    const type_name = std.mem.span(gobject.typeNameFromInstance(instance));
    if (std.mem.eql(u8, type_name, "GtkEntry")) return "TextInput";
    if (std.mem.eql(u8, type_name, "GtkSearchEntry")) return "SearchInput";
    if (std.mem.eql(u8, type_name, "GtkTextView")) return "TextArea";
    if (std.mem.eql(u8, type_name, "GtkCheckButton")) return "Checkbox";
    if (std.mem.eql(u8, type_name, "GtkSwitch")) return "Switch";
    if (std.mem.eql(u8, type_name, "AdwSwitchRow")) return "SwitchRow";
    if (std.mem.eql(u8, type_name, "AdwActionRow")) return "Row";
    if (std.mem.eql(u8, type_name, "GtkScale")) return "Slider";
    if (std.mem.eql(u8, type_name, "GtkDropDown")) return "Select";
    if (std.mem.eql(u8, type_name, "GtkScrolledWindow")) {
        // ScrollView and SourceList are both tracked
        // by their GtkScrolledWindow wrapper — disambiguate by sniffing the
        // inner child's type (unwrapping the implicit GtkViewport GTK
        // inserts for SourceList's non-GtkScrollable GtkListBox, same as
        // `scrolledWindowInner` — mirrors the structural shape ListView uses
        // for its own ScrolledWindow-wrapped, natively-scrollable GtkListView).
        const sw: *gtk.ScrolledWindow = @ptrCast(@alignCast(widget));
        if (generated.scrolledWindowInner(sw)) |child| {
            const child_instance: *gobject.TypeInstance = @ptrCast(@alignCast(child));
            const child_type_name = std.mem.span(gobject.typeNameFromInstance(child_instance));
            if (std.mem.eql(u8, child_type_name, "GtkListBox")) {
                // SourceTree wraps the same ScrolledWindow>GtkListBox pair —
                // disambiguated by the flag its create arm sets on the box.
                const child_obj: *gobject.Object = @ptrCast(@alignCast(child));
                if (gobject.Object.getData(child_obj, "nd-sourcetree") != null) return "SourceTree";
                return "SourceList";
            }
            if (std.mem.eql(u8, child_type_name, "GtkColumnView")) return "Table"; // TreeView's GtkListView stays "" like ListView's
            if (std.mem.eql(u8, child_type_name, "GtkTextView")) return "TextArea";
        }
        return "ScrollView";
    }
    return "";
}

fn semanticType(widget: *gtk.Widget, node_id: u32, args: ?std.json.Value, result_json_out: *?[*:0]u8, err_json_out: *?[*:0]u8) i32 {
    const kind = widgetKind(widget);
    if (!std.mem.eql(u8, kind, "TextInput") and !std.mem.eql(u8, kind, "SearchInput")) return invalidValue(err_json_out, node_id);
    const text = if (args) |a| (if (a == .object) a.object.get("text") else null) else null;
    if (text == null or text.? != .string) return invalidValue(err_json_out, node_id);
    // GtkEntry and GtkSearchEntry both implement gtk.Editable (unrelated
    // otherwise — SearchEntry is not an Entry subclass), so go through the
    // interface directly rather than a kind-specific concrete-type cast.
    const editable: *gtk.Editable = @ptrCast(@alignCast(widget));
    const cur = std.mem.span(gtk.Editable.getText(editable));
    // insertText position is in CHARACTERS; append = current codepoint count.
    var pos: c_int = @intCast(std.unicode.utf8CountCodepoints(cur) catch cur.len);
    const z = arena.dupeZ(u8, text.?.string) catch return -32602;
    // Length param: bytes of `text` to insert. -1 (NUL-terminated) inserts
    // the full string correctly against a live GtkEntry.
    gtk.Editable.insertText(editable, z, -1, &pos); // fires "changed"
    const full = std.mem.span(gtk.Editable.getText(editable));
    setResult(result_json_out, .{ .ref = node_id, .text = full });
    return 0;
}

fn semanticScroll(widget: *gtk.Widget, node_id: u32, args: ?std.json.Value, result_json_out: *?[*:0]u8) i32 {
    const sw: *gtk.ScrolledWindow = @ptrCast(@alignCast(widget));
    if (args) |a| {
        if (a == .object) {
            if (a.object.get("dx")) |dx_v| {
                const dx: f64 = switch (dx_v) {
                    .float => dx_v.float,
                    .integer => @floatFromInt(dx_v.integer),
                    else => 0,
                };
                const h = gtk.ScrolledWindow.getHadjustment(sw);
                gtk.Adjustment.setValue(h, gtk.Adjustment.getValue(h) + dx);
            }
            if (a.object.get("dy")) |dy_v| {
                const dy: f64 = switch (dy_v) {
                    .float => dy_v.float,
                    .integer => @floatFromInt(dy_v.integer),
                    else => 0,
                };
                const v = gtk.ScrolledWindow.getVadjustment(sw);
                gtk.Adjustment.setValue(v, gtk.Adjustment.getValue(v) + dy);
            }
        }
    }
    setResult(result_json_out, .{
        .ref = node_id,
        .x = gtk.Adjustment.getValue(gtk.ScrolledWindow.getHadjustment(sw)),
        .y = gtk.Adjustment.getValue(gtk.ScrolledWindow.getVadjustment(sw)),
    });
    return 0;
}

/// `vtable.system_request`: dispatches host system-capability requests into
/// `system.zig` (dialogs, clipboard, notifications, recent files, credentials).
/// Runs on the UI thread; each request answers via `nd_system_response`.
fn vtSystemRequest(ctx: *abi.NdContext, id: u32, method: [*:0]const u8, params: [*:0]const u8) callconv(.c) void {
    system.handleRequest(ctx, id, method, params);
}

/// Builds the complete `NdBackend` vtable for `nd_register_backend`. Called
/// once at startup by `main.zig`.
pub fn ndBackend() abi.NdBackend {
    return .{
        .create = &vtCreate,
        .apply_props = &vtApplyProps,
        .append_child = &vtAppendChild,
        .insert_before = &vtInsertBefore,
        .remove_child = &vtRemoveChild,
        .set_text = &vtSetText,
        .set_visible = &vtSetVisible,
        .apply_style = &vtApplyStyle,
        .connect_events = &vtConnectEvents,
        .has_parent = &vtHasParent,
        .unparent = &vtUnparent,
        .get_window = &vtGetWindow,
        .marshal_async = &vtMarshalAsync,
        .show_overlay = &vtShowOverlay,
        .node_visible = &vtNodeVisible,
        .node_bounds = &vtNodeBounds,
        .snapshot = &vtSnapshot,
        .semantic_action = &vtSemanticAction,
        .widget_command = &vtWidgetCommand,
        .resolve_window = &vtResolveWindow,
        .reparent_child = &vtReparentChild,
        .system_request = &vtSystemRequest,
        .release_node = &vtReleaseNode,
    };
}
