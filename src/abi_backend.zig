const std = @import("std");
const protocol = @import("protocol.zig");
const abi = @import("abi.zig");

// The tree stores `*Widget`; for the abi backend `Widget == anyopaque` (an
// opaque handle riding as C `void*` — GtkWidget* on the GTK embedder,
// NSView* on the Mac shell). The core never dereferences it.
pub const Widget = anyopaque;

// Set by `bind` (called from `nd_register_backend`, Task 2) — the core's
// single instance. A real embedder registers exactly one backend per
// process; tests rebind for each fake-vtable case.
pub var ctx: *abi.NdContext = undefined;
pub var vtable: *const abi.NdBackend = undefined;
var gpa: std.mem.Allocator = undefined;

pub fn bind(allocator: std.mem.Allocator, c: *abi.NdContext, vt: *const abi.NdBackend) void {
    gpa = allocator;
    ctx = c;
    vtable = vt;
}

/// `std.json.Stringify` has no `valueAllocSentinel` in this Zig — stringify
/// then dupe with a NUL sentinel appended (one extra alloc, freed by the
/// caller of `jsonZ`/`attachedJsonZ` alongside everything else on this path).
fn allocZFromValue(v: anytype) [:0]const u8 {
    const json = std.json.Stringify.valueAlloc(gpa, v, .{}) catch return gpa.dupeZ(u8, "{}") catch @panic("OOM in abi_backend allocZFromValue");
    defer gpa.free(json);
    return gpa.dupeZ(u8, json) catch @panic("OOM in abi_backend allocZFromValue");
}

/// Stringifies a prop/style value to NUL-terminated JSON for the ABI
/// boundary (M6a-D2). Caller frees with `gpa.free`. `null` props serialize
/// as `"{}"`, matching how the GTK embedder already treats an absent props
/// object as "nothing to apply".
fn jsonZ(v: ?std.json.Value) [:0]const u8 {
    const val = v orelse return gpa.dupeZ(u8, "{}") catch @panic("OOM in abi_backend jsonZ");
    return allocZFromValue(val);
}

fn dupeZ(s: []const u8) [:0]const u8 {
    return gpa.dupeZ(u8, s) catch @panic("OOM in abi_backend dupeZ");
}

pub fn setEventSink(_: anytype) void {} // events flow embedder->core via nd_emit_event, not a sink here
pub fn initStyle(_: anytype) void {}

pub fn createWidget(_: *anyopaque, kind: []const u8, props: ?std.json.Value) !*Widget {
    const kz = dupeZ(kind);
    defer gpa.free(kz);
    const pz = jsonZ(props);
    defer gpa.free(pz);
    return @ptrCast(vtable.create(ctx, kz, pz) orelse return error.CreateFailed);
}

pub fn applyProps(widget: *Widget, kind: []const u8, props: ?std.json.Value) void {
    const kz = dupeZ(kind);
    defer gpa.free(kz);
    const pz = jsonZ(props);
    defer gpa.free(pz);
    vtable.apply_props(ctx, widget, kz, pz);
}

pub fn appendChild(parent: *Widget, parent_kind: []const u8, child: *Widget, attached: protocol.Attached) void {
    const kz = dupeZ(parent_kind);
    defer gpa.free(kz);
    const az = attachedJsonZ(attached);
    defer gpa.free(az);
    vtable.append_child(ctx, parent, kz, child, az);
}

pub fn insertBefore(parent: *Widget, parent_kind: []const u8, child: *Widget, before: ?*Widget, attached: protocol.Attached) void {
    const kz = dupeZ(parent_kind);
    defer gpa.free(kz);
    const az = attachedJsonZ(attached);
    defer gpa.free(az);
    vtable.insert_before(ctx, parent, kz, child, before, az);
}

pub fn removeChild(parent: *Widget, parent_kind: []const u8, child: *Widget) void {
    const kz = dupeZ(parent_kind);
    defer gpa.free(kz);
    vtable.remove_child(ctx, parent, kz, child);
}

pub fn setText(widget: *Widget, text: []const u8) void {
    const tz = dupeZ(text);
    defer gpa.free(tz);
    vtable.set_text(ctx, widget, tz);
}

pub fn setVisible(widget: *Widget, visible: bool) void {
    vtable.set_visible(ctx, widget, visible);
}

pub fn applyStyle(widget: *Widget, node_id: u32, style_value: std.json.Value) void {
    const sz = jsonZ(style_value);
    defer gpa.free(sz);
    vtable.apply_style(ctx, widget, node_id, sz);
}

pub fn connectEvents(widget: *Widget, kind: []const u8, node_id: u32) void {
    const kz = dupeZ(kind);
    defer gpa.free(kz);
    vtable.connect_events(ctx, widget, kz, node_id);
}

pub fn hasParent(widget: *Widget) bool {
    return vtable.has_parent(ctx, widget);
}

pub fn unparentWidget(widget: *Widget) void {
    vtable.unparent(ctx, widget);
}

pub fn getWindow() ?*Widget {
    return vtable.get_window(ctx);
}

/// Serializes `Attached` to the same JSON shape a create-op's props carry
/// (gridRow/gridColumn/gridRowSpan/gridColumnSpan/tabLabel/slot) — the embedder's
/// `append_child`/`insert_before` re-derive attach metadata the same way
/// `protocol.Attached.fromProps` does today, just crossing the ABI as JSON
/// per M6a-D2 instead of a Zig struct.
fn attachedJsonZ(attached: protocol.Attached) [:0]const u8 {
    const Shape = struct {
        gridRow: i64,
        gridColumn: i64,
        gridRowSpan: i64,
        gridColumnSpan: i64,
        tabLabel: ?[]const u8,
        slot: ?[]const u8,
    };
    const shape = Shape{
        .gridRow = attached.grid_row,
        .gridColumn = attached.grid_column,
        .gridRowSpan = attached.grid_row_span,
        .gridColumnSpan = attached.grid_column_span,
        .tabLabel = attached.tab_label,
        .slot = attached.slot,
    };
    return allocZFromValue(shape);
}

test "fake-vtable round-trip: create carries kind+props JSON losslessly" {
    const gpa_t = std.testing.allocator;

    const Recorded = struct {
        var kind: [64]u8 = undefined;
        var kind_len: usize = 0;
        var props: [256]u8 = undefined;
        var props_len: usize = 0;
        var widget_out: u8 = 0;

        fn create(_: *abi.NdContext, k: [*:0]const u8, p: [*:0]const u8) callconv(.c) ?*anyopaque {
            const ks = std.mem.span(k);
            const ps = std.mem.span(p);
            @memcpy(kind[0..ks.len], ks);
            kind_len = ks.len;
            @memcpy(props[0..ps.len], ps);
            props_len = ps.len;
            return &widget_out;
        }
    };

    var fake_vtable: abi.NdBackend = undefined;
    fake_vtable.create = &Recorded.create;

    var fake_ctx_storage: abi.NdContext = undefined;

    bind(gpa_t, &fake_ctx_storage, &fake_vtable);

    const parsed = try std.json.parseFromSlice(std.json.Value, gpa_t, "{\"title\":\"Hi\",\"defaultWidth\":480}", .{});
    defer parsed.deinit();

    const widget = try createWidget(undefined, "Window", parsed.value);
    try std.testing.expect(@intFromPtr(widget) == @intFromPtr(&Recorded.widget_out));
    try std.testing.expectEqualStrings("Window", Recorded.kind[0..Recorded.kind_len]);

    const recorded_props = Recorded.props[0..Recorded.props_len];
    const reparsed = try std.json.parseFromSlice(std.json.Value, gpa_t, recorded_props, .{});
    defer reparsed.deinit();
    try std.testing.expectEqualStrings("Hi", reparsed.value.object.get("title").?.string);
    try std.testing.expectEqual(@as(i64, 480), reparsed.value.object.get("defaultWidth").?.integer);
}
