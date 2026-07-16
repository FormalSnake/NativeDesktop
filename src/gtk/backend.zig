const std = @import("std");
const gtk = @import("gtk");
const gdk = @import("gdk");
const gsk = @import("gsk");
const gobject = @import("gobject");
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
const ndtabs_gtk = @import("tabs.zig");

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
    applyCssClassesIfPresent(widget, props);
    if (std.mem.eql(u8, std.mem.span(kind), "Window")) system.attachWindowDropTarget(widget);
    return widget;
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
    // Menu nodes have no GtkWidget mapped state; treat them as actionable so a
    // MenuItem ref survives checkActionable and reaches semanticClick.
    if (!isRealWidget(w)) return true;
    if (gtk.Widget.getVisible(w) == 0) return false;
    // "mapped" folds into node_visible's contract (documented in
    // automation.zig's checkActionable comment).
    return gtk.Widget.getMapped(w) != 0;
}

fn vtNodeBounds(_: *abi.NdContext, widget: ?*anyopaque, out: *abi.NdRect) callconv(.c) bool {
    const w: *gtk.Widget = @ptrCast(@alignCast(widget));
    // Menu nodes have no geometry; report a nominal non-degenerate rect so
    // checkActionable (w>0 ∧ h>0) admits a MenuItem ref for semanticClick.
    if (!isRealWidget(w)) {
        out.* = .{ .x = 0, .y = 0, .w = 1, .h = 1 };
        return true;
    }
    // Per-window (multi-window): convert relative to the widget's OWN root
    // window, not a single global — a widget living in window B must report
    // its bounds in window B's coordinate space. `getRoot` is the widget's
    // GtkWindow ancestor (the same object the pre-multi-window code assumed was
    // the only window); an unrooted widget falls back to the primary window,
    // where its bounds are degenerate anyway.
    const root_widget: *gtk.Widget = if (gtk.Widget.getRoot(w)) |r|
        r.as(gtk.Widget)
    else
        (getWindow() orelse return false).as(gtk.Widget);
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

/// `vtable.snapshot`: the GTK-native WidgetPaintable render path.
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

    const paintable = gtk.WidgetPaintable.new(win_widget);
    defer paintable.unref();

    const snapshot = gtk.Snapshot.new();
    const width = gtk.Widget.getWidth(win_widget);
    const height = gtk.Widget.getHeight(win_widget);
    gdk.Paintable.snapshot(paintable.as(gdk.Paintable), snapshot.as(gdk.Snapshot), @floatFromInt(width), @floatFromInt(height));

    const node = gtk.Snapshot.freeToNode(snapshot) orelse return false;
    defer gsk.RenderNode.unref(node);

    const texture = gsk.Renderer.renderTexture(renderer, node, null);
    defer texture.unref();

    return gdk.Texture.saveToPng(texture, png_path) != 0;
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

    // Menu nodes only support "click" (→ menu dispatch); setValue/type/scroll
    // would cast the GMenu handle to a GtkWidget, so reject them here.
    if (!isRealWidget(w) and !std.mem.eql(u8, action_s, "click")) {
        setErr(err_json_out, node_id);
        return -32602;
    }

    if (std.mem.eql(u8, action_s, "window.close")) {
        // Window-root unmount (tree.zig remove arm): close the native
        // window/tab — tabs.zig no-ops if the user already closed it.
        ndtabs_gtk.closeNode(w);
        return 0;
    } else if (std.mem.eql(u8, action_s, "click")) {
        return semanticClick(w, node_id, result_json_out, err_json_out);
    } else if (std.mem.eql(u8, action_s, "setValue")) {
        return semanticSetValue(w, node_id, args, result_json_out, err_json_out);
    } else if (std.mem.eql(u8, action_s, "type")) {
        return semanticType(w, node_id, args, result_json_out, err_json_out);
    } else if (std.mem.eql(u8, action_s, "scroll")) {
        return semanticScroll(w, node_id, args, result_json_out);
    } else if (std.mem.eql(u8, action_s, "a11y")) {
        return semanticA11y(w, node_id, result_json_out);
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

/// "a11y" — the live per-node accessibility probe behind getTree's
/// enabled/focused/value fields. Value reads mirror
/// `semanticSetValue`'s kind dispatch so both sides of the round-trip agree
/// on what a widget's value is.
fn semanticA11y(widget: *gtk.Widget, node_id: u32, result_json_out: *?[*:0]u8) i32 {
    const enabled = gtk.Widget.isSensitive(widget) != 0;
    const focused = gtk.Widget.hasFocus(widget) != 0;
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
    }
    defer if (owned) arena.free(@constCast(value_json));

    const json = std.fmt.allocPrint(arena, "{{\"enabled\":{},\"focused\":{},\"value\":{s}}}", .{ enabled, focused, value_json }) catch return -32603;
    defer arena.free(json);
    result_json_out.* = mallocZ(json);
    _ = node_id;
    return 0;
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
    const kind = widgetKind(widget);
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
        const view: *gtk.TextView = @ptrCast(@alignCast(widget));
        gtk.TextBuffer.setText(gtk.TextView.getBuffer(view), z, -1);
    } else if (std.mem.eql(u8, kind, "Checkbox") or std.mem.eql(u8, kind, "Radio")) {
        if (value != .bool) return invalidValue(err_json_out, node_id);
        gtk.CheckButton.setActive(@ptrCast(@alignCast(widget)), @intFromBool(value.bool));
    } else if (std.mem.eql(u8, kind, "Switch")) {
        if (value != .bool) return invalidValue(err_json_out, node_id);
        gtk.Switch.setActive(@ptrCast(@alignCast(widget)), @intFromBool(value.bool));
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
            if (std.mem.eql(u8, child_type_name, "GtkListBox")) return "SourceList";
            if (std.mem.eql(u8, child_type_name, "GtkColumnView")) return "Table"; // TreeView's GtkListView stays "" like ListView's
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
    };
}
