// GTK4 rendering/input surface for the <terminal> widget. Drives the terminal
// core (src/core/terminal.zig, the `ndterm` C ABI — see include/ndterm.h) and
// paints its viewport grid onto a GtkDrawingArea with Pango (harfbuzz+
// fontconfig fallback — see the addendum on the cairo-toy-API predecessor in
// docs/plans/mac-design-polish.md issue 4). The core owns the PTY +
// libghostty-vt behind a mutex; this file only snapshots+draws and maps key/
// mouse/scroll events to the ndterm input ABI. One State per widget:
// ndterm_open_ex() at create, ndterm_close() + free at unrealize.
const std = @import("std");
const gtk = @import("gtk");
const gdk = @import("gdk");
const gobject = @import("gobject");
const cairo = @import("cairo");
const pango = @import("pango");
const pangocairo = @import("pangocairo");
const ndt = @import("../core/terminal.zig");
const ndremote = @import("../core/remote_terminal.zig");
const protocol = @import("../protocol.zig");

/// Peer of the generated widgets.zig EmitFn (same shape, same protocol module
/// instance) — handed over once by the generated connectEvents Terminal arm.
pub const EmitFn = *const fn (node_id: u32, name: []const u8, payload: protocol.EventPayload) void;
var emit: ?EmitFn = null;

// ---- ndterm C-ABI structs, re-declared locally to stay decoupled from the
// core's Zig type exports (layout matches include/ndterm.h exactly). The core's
// export fns are called by their Zig names on an opaque handle; struct-pointer
// args go through @ptrCast so this file is robust to whatever concrete type the
// core declares for the identical layout. ----

const FLAG_BOLD: u8 = 1 << 0;
const FLAG_UNDERLINE: u8 = 1 << 1;
const FLAG_INVERSE: u8 = 1 << 2;
const FLAG_WIDE: u8 = 1 << 3;
const FLAG_WIDE_TAIL: u8 = 1 << 4;

const Cell = extern struct {
    utf8: [16]u8,
    fg: [3]u8,
    bg: [3]u8,
    flags: u8,
};

const Cursor = extern struct {
    x: u16,
    y: u16,
    visible: u8,
    style: u8,
};

// include/ndterm.h NDTERM_MOUSE_* — bitmask returned by ndterm_mouse_mode.
const NDTERM_MOUSE_BUTTON: u32 = 1 << 2;
const NDTERM_MOUSE_ANY: u32 = 1 << 3;

const STATE_KEY: [*:0]const u8 = "nd-terminal-state";

/// Per-widget heap state, stashed on the DrawingArea via g_object_set_data and
/// retrieved in the draw callback; the tick/signal callbacks receive it as their
/// typed closure data. Allocated with the c_allocator (this lives in a
/// long-running shared lib, not tied to any arena's lifetime).
const State = struct {
    term: *align(8) anyopaque, // ndterm handle (opaque nd_terminal*, core Terminal is 8-aligned)
    widget: *gtk.Widget, // the DrawingArea, for grabFocus from the click gesture
    font_size: f64,
    cell_w: f64,
    cell_h: f64,
    cols: u16,
    rows: u16,
    node_id: u32 = 0, // set by connectEvents; effect emits gate on it (0 = unwired)
    is_remote: bool = false,
    rt: ?*align(8) anyopaque = null, // nd_remote_terminal handle (Channel is 8-aligned)
    // WP polish-1 deliverable 4: pre-built once at create (never per-cell —
    // see the addendum's "selectFontFace called per cell" finding), reused by
    // every draw. `font_family` unset -> "monospace" (cairo toy API's old
    // default), so the fallback stays behavior-identical when a caller hasn't
    // opted into a Nerd Font.
    font_regular: *pango.FontDescription,
    font_bold: *pango.FontDescription,
    // WP polish-1 deliverable 6: tracks whether a button is currently held so
    // motion reports can be gated on NDTERM_MOUSE_BUTTON (report only while
    // dragging) vs NDTERM_MOUSE_ANY (report every move).
    mouse_button_down: bool = false,
    mouse_last_button: c_uint = 0,
};

fn stateFrom(widget: *gtk.Widget) ?*State {
    const raw = gobject.Object.getData(widget.as(gobject.Object), STATE_KEY) orelse return null;
    return @ptrCast(@alignCast(raw));
}

// ---------------------------------------------------------------------------
// WP polish-1 deliverable 7: palette/fg/bg parsing (docs/widgets.md: `palette`
// is a comma-separated list of `#rrggbb`, 16 or 256 entries in ANSI index
// order; `foreground`/`background` are single `#rrggbb` strings). Shared by
// create/createRemote.
// ---------------------------------------------------------------------------

const PALETTE_COLORS: usize = 256;

fn parseHex2(s: []const u8) ?u8 {
    if (s.len != 2) return null;
    return std.fmt.parseInt(u8, s, 16) catch null;
}

fn parseHexColor(s: []const u8) ?[3]u8 {
    if (s.len != 7 or s[0] != '#') return null;
    const r = parseHex2(s[1..3]) orelse return null;
    const g = parseHex2(s[3..5]) orelse return null;
    const b = parseHex2(s[5..7]) orelse return null;
    return .{ r, g, b };
}

/// The standard xterm 256-color cube (16-231) + grayscale ramp (232-255),
/// used to fill the entries a caller-supplied 16-color palette doesn't cover
/// — ndterm's open-time palette override is all-or-nothing (exactly
/// NDTERM_PALETTE_COLORS*3 bytes, see include/ndterm.h `nd_term_open_opts`).
fn xtermStandardColor(index: u8) [3]u8 {
    if (index < 16) return .{ 0, 0, 0 }; // caller always supplies 0-15
    if (index < 232) {
        const n = index - 16;
        const levels = [6]u8{ 0, 95, 135, 175, 215, 255 };
        return .{ levels[n / 36], levels[(n / 6) % 6], levels[n % 6] };
    }
    const gray: u8 = 8 + 10 * (index - 232);
    return .{ gray, gray, gray };
}

/// Parses the `palette` prop into `out` (NDTERM_PALETTE_COLORS*3 bytes). A
/// 16-entry palette gets the standard xterm cube/grayscale for indices
/// 16-255. Returns false (out untouched) on a malformed entry or a count
/// that's neither 16 nor 256.
fn parsePalette(s: []const u8, out: *[PALETTE_COLORS * 3]u8) bool {
    var colors: [PALETTE_COLORS][3]u8 = undefined;
    var n: usize = 0;
    var it = std.mem.splitScalar(u8, s, ',');
    while (it.next()) |tok| {
        if (n >= PALETTE_COLORS) return false;
        colors[n] = parseHexColor(std.mem.trim(u8, tok, " ")) orelse return false;
        n += 1;
    }
    if (n != 16 and n != PALETTE_COLORS) return false;
    for (0..PALETTE_COLORS) |i| {
        const c = if (i < n) colors[i] else xtermStandardColor(@intCast(i));
        out[i * 3 + 0] = c[0];
        out[i * 3 + 1] = c[1];
        out[i * 3 + 2] = c[2];
    }
    return true;
}

/// Owns the palette byte buffer `opts.palette_rgb` borrows — must outlive the
/// ndterm_open_ex/ndrt_open_ex call it's passed to.
const OpenOpts = struct {
    palette_buf: [PALETTE_COLORS * 3]u8 = undefined,
    opts: ndt.nd_term_open_opts = undefined,
};

/// Builds the palette-carrying open options (WP polish-1 deliverable 7) from
/// the create-time fg/bg/palette props.
fn buildOpenOpts(out: *OpenOpts, palette: ?[*:0]const u8, foreground: [*:0]const u8, background: [*:0]const u8) *const ndt.nd_term_open_opts {
    const has_palette = if (palette) |p| parsePalette(std.mem.span(p), &out.palette_buf) else false;
    const fg = parseHexColor(std.mem.span(foreground)) orelse [3]u8{ 0xcc, 0xcc, 0xcc };
    const bg = parseHexColor(std.mem.span(background)) orelse [3]u8{ 0x00, 0x00, 0x00 };
    out.opts = .{
        .palette_rgb = if (has_palette) &out.palette_buf else null,
        .palette_len = if (has_palette) out.palette_buf.len else 0,
        .has_fg = 1,
        .fg = fg,
        .has_bg = 1,
        .bg = bg,
    };
    return &out.opts;
}

pub fn create(command: ?[*:0]const u8, cwd: ?[*:0]const u8, font_size: c_int, font_family: ?[*:0]const u8, palette: ?[*:0]const u8, foreground: [*:0]const u8, background: [*:0]const u8, cols: u16, rows: u16) *gtk.Widget {
    const area = gtk.DrawingArea.new();
    const widget = area.as(gtk.Widget);
    const state = newState(widget, area, font_size, font_family, cols, rows);

    var open_opts: OpenOpts = .{};
    const opts_ptr = buildOpenOpts(&open_opts, palette, foreground, background);

    const term = ndt.ndterm_open_ex(cols, rows, command, cwd, opts_ptr, &effectTramp, state) orelse {
        // Core failed to spawn the PTY: return the bare DrawingArea (blank).
        std.debug.print("ND_WARN ndterm_open failed\n", .{});
        freeState(state);
        return widget;
    };
    state.term = @ptrCast(term);

    wireSurface(area, widget, state);
    return widget;
}

/// Remote variant: the grid is fed by the byte-plane transport (ndremote)
/// instead of a local PTY. Same rendering/input surface; effect + connection
/// state route to NDP events via effectTramp/stateTramp.
pub fn createRemote(host: [*:0]const u8, port: u16, session_id: [*:0]const u8, ticket: [*:0]const u8, font_size: c_int, font_family: ?[*:0]const u8, palette: ?[*:0]const u8, foreground: [*:0]const u8, background: [*:0]const u8, cols: u16, rows: u16) *gtk.Widget {
    const area = gtk.DrawingArea.new();
    const widget = area.as(gtk.Widget);
    const state = newState(widget, area, font_size, font_family, cols, rows);
    state.is_remote = true;

    var open_opts: OpenOpts = .{};
    const opts_ptr = buildOpenOpts(&open_opts, palette, foreground, background);

    const rt = ndremote.ndrt_open_ex(host, port, session_id, ticket, cols, rows, opts_ptr, &effectTramp, &stateTramp, state) orelse {
        std.debug.print("ND_WARN ndrt_open failed\n", .{});
        freeState(state);
        return widget;
    };
    state.rt = @ptrCast(rt);
    const term = ndremote.ndrt_terminal(rt) orelse {
        ndremote.ndrt_close(rt);
        freeState(state);
        return widget;
    };
    state.term = @ptrCast(@alignCast(term));

    wireSurface(area, widget, state);
    return widget;
}

/// Allocate + init per-widget State (term set by the caller after open).
/// Monospace cell metric from the point size (measuring via pango would be more
/// precise, but this keeps the grid internally consistent because the draw func
/// reuses these same numbers).
fn newState(widget: *gtk.Widget, area: *gtk.DrawingArea, font_size: c_int, font_family: ?[*:0]const u8, cols: u16, rows: u16) *State {
    const fs: f64 = @floatFromInt(font_size);
    const cell_w = @round(fs * 0.6);
    const cell_h = @round(fs * 1.2);

    gtk.DrawingArea.setContentWidth(area, @intFromFloat(cell_w * @as(f64, @floatFromInt(cols))));
    gtk.DrawingArea.setContentHeight(area, @intFromFloat(cell_h * @as(f64, @floatFromInt(rows))));

    const family: [*:0]const u8 = font_family orelse "monospace";
    const font_regular = makeFontDesc(family, fs, .normal);
    const font_bold = makeFontDesc(family, fs, .bold);

    const state = std.heap.c_allocator.create(State) catch @panic("OOM allocating terminal State");
    state.* = .{
        .term = undefined,
        .widget = widget,
        .font_size = fs,
        .cell_w = cell_w,
        .cell_h = cell_h,
        .cols = cols,
        .rows = rows,
        .font_regular = font_regular,
        .font_bold = font_bold,
    };
    return state;
}

fn makeFontDesc(family: [*:0]const u8, size_px: f64, weight: pango.Weight) *pango.FontDescription {
    const d = pango.FontDescription.new();
    pango.FontDescription.setFamily(d, family);
    pango.FontDescription.setAbsoluteSize(d, size_px * pango.SCALE);
    pango.FontDescription.setWeight(d, weight);
    return d;
}

fn freeState(state: *State) void {
    pango.FontDescription.free(state.font_regular);
    pango.FontDescription.free(state.font_bold);
    std.heap.c_allocator.destroy(state);
}

/// Shared surface wiring for both the local and remote paths.
fn wireSurface(area: *gtk.DrawingArea, widget: *gtk.Widget, state: *State) void {
    gobject.Object.setData(widget.as(gobject.Object), STATE_KEY, state);
    gtk.DrawingArea.setDrawFunc(area, &drawCb, state, null);

    // Focusable + key controller for input, click gesture to grab focus + send
    // SGR mouse press/release, motion controller for drag/hover reporting,
    // scroll controller for scrollback (WP polish-1 deliverables 5/6).
    gtk.Widget.setFocusable(widget, 1);
    const key_ctrl = gtk.EventControllerKey.new();
    _ = gtk.EventControllerKey.signals.key_pressed.connect(key_ctrl, *State, &onKeyPressed, state, .{});
    gtk.Widget.addController(widget, key_ctrl.as(gtk.EventController));

    const click = gtk.GestureClick.new();
    gtk.GestureSingle.setButton(click.as(gtk.GestureSingle), 0); // any button, not just left
    _ = gtk.GestureClick.signals.pressed.connect(click, *State, &onPressed, state, .{});
    _ = gtk.GestureClick.signals.released.connect(click, *State, &onReleased, state, .{});
    gtk.Widget.addController(widget, click.as(gtk.EventController));

    const motion_ctrl = gtk.EventControllerMotion.new();
    _ = gtk.EventControllerMotion.signals.motion.connect(motion_ctrl, *State, &onMotion, state, .{});
    gtk.Widget.addController(widget, motion_ctrl.as(gtk.EventController));

    const scroll_ctrl = gtk.EventControllerScroll.new(gtk.EventControllerScrollFlags.flags_vertical);
    _ = gtk.EventControllerScroll.signals.scroll.connect(scroll_ctrl, *State, &onScroll, state, .{});
    gtk.Widget.addController(widget, scroll_ctrl.as(gtk.EventController));

    // Track the drawing area's actual allocation onto the grid (peer of the
    // AppKit setFrameSize path). GtkDrawingArea::resize is the correct
    // observation point — cheaper and more precise than polling in tickCb.
    _ = gtk.DrawingArea.signals.resize.connect(area, *State, &onResize, state, .{});

    // Repaint every frame so output from the core's reader thread shows up
    // without a per-cell dirty channel (correct first, fast later).
    _ = gtk.Widget.addTickCallback(widget, &tickCb, null, null);

    // Tear down the core + free state when the widget goes away.
    _ = gtk.Widget.signals.unrealize.connect(widget, *State, &onUnrealize, state, .{});
}

/// Generated connectEvents Terminal arm: stash node_id on the State and save
/// the module emit sink. Both trampolines gate on node_id != 0. Effect/state
/// callbacks fire on a reader thread — emitting an NDP event there is safe
/// (writeFrameOpts holds writer_mutex); they never touch a GTK widget.
pub fn connectEvents(widget: *gtk.Widget, node_id: u32, emit_fn: EmitFn) void {
    emit = emit_fn;
    const state = stateFrom(widget) orelse return;
    state.node_id = node_id;
}

fn effectTramp(ud: ?*anyopaque, kind: c_int, text: ?[*:0]const u8, code: c_int) callconv(.c) void {
    const state: *State = @ptrCast(@alignCast(ud orelse return));
    if (state.node_id == 0) return;
    const f = emit orelse return;
    switch (kind) {
        0 => { // title changed
            const t = text orelse return;
            f(state.node_id, "titleChanged", .{ .text = std.mem.span(t) });
        },
        1 => f(state.node_id, "bell", .{}), // bell
        2 => { // child exited
            var obj: std.json.ObjectMap = .empty;
            defer obj.deinit(std.heap.page_allocator);
            obj.put(std.heap.page_allocator, "code", .{ .integer = @as(i64, code) }) catch return;
            f(state.node_id, "exited", .{ .data = .{ .object = obj } });
        },
        else => {},
    }
}

fn stateTramp(ud: ?*anyopaque, state_val: c_int, detail: ?[*:0]const u8) callconv(.c) void {
    const state: *State = @ptrCast(@alignCast(ud orelse return));
    if (state.node_id == 0) return;
    const f = emit orelse return;
    var obj: std.json.ObjectMap = .empty;
    defer obj.deinit(std.heap.page_allocator);
    obj.put(std.heap.page_allocator, "state", .{ .integer = @as(i64, state_val) }) catch return;
    if (detail) |d| obj.put(std.heap.page_allocator, "detail", .{ .string = std.mem.span(d) }) catch return;
    f(state.node_id, "connectionState", .{ .data = .{ .object = obj } });
}

const G_SOURCE_CONTINUE: c_int = 1;

fn tickCb(widget: *gtk.Widget, _: *gdk.FrameClock, _: ?*anyopaque) callconv(.c) c_int {
    gtk.Widget.queueDraw(widget);
    return G_SOURCE_CONTINUE;
}

fn onPressed(gesture: *gtk.GestureClick, _: c_int, x: f64, y: f64, state: *State) callconv(.c) void {
    _ = gtk.Widget.grabFocus(state.widget);
    const btn = gtk.GestureSingle.getCurrentButton(gesture.as(gtk.GestureSingle));
    state.mouse_button_down = true;
    state.mouse_last_button = btn;
    if (ndt.ndterm_mouse_mode(@ptrCast(state.term)) == 0) return;
    sendSgrMouse(state, sgrBaseButton(btn), x, y, true);
}

fn onReleased(_: *gtk.GestureClick, _: c_int, x: f64, y: f64, state: *State) callconv(.c) void {
    const btn = state.mouse_last_button;
    state.mouse_button_down = false;
    if (ndt.ndterm_mouse_mode(@ptrCast(state.term)) == 0) return;
    sendSgrMouse(state, sgrBaseButton(btn), x, y, false);
}

/// WP polish-1 deliverable 6: hover/drag reporting, gated on the VT's active
/// mouse modes — NDTERM_MOUSE_ANY reports every move, NDTERM_MOUSE_BUTTON
/// reports only while a button is held (xterm 1002/1003 semantics). Always
/// SGR-encoded (1006 format) regardless of which format bit the app actually
/// asked for — a deliberate v1 simplification (see NDTerminalView.swift's
/// mirror of this same choice), safe in practice because apps that enable
/// tracking overwhelmingly also request SGR.
fn onMotion(_: *gtk.EventControllerMotion, x: f64, y: f64, state: *State) callconv(.c) void {
    const mode = ndt.ndterm_mouse_mode(@ptrCast(state.term));
    const any_motion = (mode & NDTERM_MOUSE_ANY) != 0;
    const button_motion = state.mouse_button_down and (mode & NDTERM_MOUSE_BUTTON) != 0;
    if (!any_motion and !button_motion) return;
    const base: u8 = if (state.mouse_button_down) sgrBaseButton(state.mouse_last_button) else 3; // 3 = no button held
    sendSgrMouse(state, base | 32, x, y, true); // +32 = motion
}

/// WP polish-1 deliverable 6: scroll wheel -> client-local scrollback viewport
/// (ndterm_scroll_viewport, wrapping libghostty-vt's scroll_viewport — the
/// client VT already saw every byte, so no round-trip to the daemon is
/// needed). `dy` is GTK's vertical scroll delta (negative = up, matching
/// ndterm_scroll_viewport's "up is negative" convention); scaled to a few
/// lines per wheel step, matching common terminal-emulator convention.
const SCROLL_LINES_PER_UNIT: f64 = 3.0;

fn onScroll(_: *gtk.EventControllerScroll, _: f64, dy: f64, state: *State) callconv(.c) c_int {
    const delta: c_int = @intFromFloat(@round(dy * SCROLL_LINES_PER_UNIT));
    if (delta == 0) return 0;
    ndt.ndterm_scroll_viewport(@ptrCast(state.term), delta);
    gtk.Widget.queueDraw(state.widget);
    return 1; // GDK_EVENT_STOP
}

/// Map the drawing area's pixel allocation onto the cell grid. Local:
/// ndterm_resize (grid only). Remote: ndrt_resize (grid + a RESIZE frame; the
/// server replies RESIZED and the reader applies the min-box result). Both take
/// the ndterm mutex, so this is safe against the reader's feed.
fn onResize(_: *gtk.DrawingArea, width: c_int, height: c_int, state: *State) callconv(.c) void {
    if (width <= 0 or height <= 0) return;
    const cols_i: i64 = @intFromFloat(@as(f64, @floatFromInt(width)) / state.cell_w);
    const rows_i: i64 = @intFromFloat(@as(f64, @floatFromInt(height)) / state.cell_h);
    const cols: u16 = @intCast(std.math.clamp(cols_i, 1, 65535));
    const rows: u16 = @intCast(std.math.clamp(rows_i, 1, 65535));
    if (cols == state.cols and rows == state.rows) return;
    state.cols = cols;
    state.rows = rows;
    if (state.is_remote) {
        if (state.rt) |rt| ndremote.ndrt_resize(@ptrCast(rt), cols, rows);
    } else {
        ndt.ndterm_resize(@ptrCast(state.term), cols, rows);
    }
    gtk.Widget.queueDraw(state.widget);
}

fn onUnrealize(widget: *gtk.Widget, state: *State) callconv(.c) void {
    if (state.is_remote) {
        if (state.rt) |rt| ndremote.ndrt_close(@ptrCast(rt)); // closes the virtual ndterm too
    } else {
        ndt.ndterm_close(@ptrCast(state.term));
    }
    gobject.Object.setData(widget.as(gobject.Object), STATE_KEY, null);
    freeState(state);
}

// ---- rendering ----

fn setRgb(cr: *cairo.Context, rgb: [3]u8) void {
    cairo.Context.setSourceRgb(
        cr,
        @as(f64, @floatFromInt(rgb[0])) / 255.0,
        @as(f64, @floatFromInt(rgb[1])) / 255.0,
        @as(f64, @floatFromInt(rgb[2])) / 255.0,
    );
}

/// Draws one cell's grapheme at (px, py), clipped to its owned cell rect
/// (`own_w` x cell_h) so a fontconfig fallback face wider than the fixed
/// cw/ch metric box (PUA/emoji substitutes) can't smear into the neighbor.
fn drawGlyph(cr: *cairo.Context, layout: *pango.Layout, desc: *pango.FontDescription, txt: [*:0]const u8, px: f64, py: f64, own_w: f64, ch: f64, rgb: [3]u8) void {
    pango.Layout.setFontDescription(layout, desc);
    pango.Layout.setText(layout, txt, -1);
    setRgb(cr, rgb);
    cairo.Context.save(cr);
    cairo.Context.rectangle(cr, px, py, own_w, ch);
    cairo.Context.clip(cr);
    cairo.Context.moveTo(cr, px, py);
    pangocairo.showLayout(cr, layout);
    cairo.Context.restore(cr);
}

fn drawCb(area: *gtk.DrawingArea, cr: *cairo.Context, width: c_int, height: c_int, _: ?*anyopaque) callconv(.c) void {
    const state = stateFrom(area.as(gtk.Widget)) orelse return;
    const term = state.term;
    const cw = state.cell_w;
    const ch = state.cell_h;

    var cols: u16 = 0;
    var rows: u16 = 0;
    ndt.ndterm_render_lock(@ptrCast(term), &cols, &rows);
    defer ndt.ndterm_render_unlock(@ptrCast(term));

    var def_fg: [3]u8 = .{ 0, 0, 0 };
    var def_bg: [3]u8 = .{ 0, 0, 0 };
    ndt.ndterm_default_colors(@ptrCast(term), @ptrCast(&def_fg), @ptrCast(&def_bg));

    // Whole-widget background in the default bg.
    setRgb(cr, def_bg);
    cairo.Context.rectangle(cr, 0, 0, @floatFromInt(width), @floatFromInt(height));
    cairo.Context.fill(cr);

    // One PangoLayout for the whole frame (not one per cell — the addendum's
    // flagged bug was cairo_select_font_face called PER CELL; the analogous
    // Pango mistake would be allocating a layout per cell). set_text per cell
    // reshapes against the same layout/font-map/context instead of
    // reallocating; harfbuzz+fontconfig substitute a Nerd Font automatically
    // for PUA glyphs the primary face lacks (Pango's fallback cascade, unlike
    // the cairo toy API this replaces).
    const layout = pangocairo.createLayout(cr);
    defer gobject.Object.unref(layout.as(gobject.Object));
    pango.Layout.setSingleParagraphMode(layout, 1);

    var y: u16 = 0;
    while (y < rows) : (y += 1) {
        var x: u16 = 0;
        while (x < cols) : (x += 1) {
            var cell: Cell = undefined;
            ndt.ndterm_cell(@ptrCast(term), x, y, @ptrCast(&cell));

            // The trailing half of a wide glyph carries no text of its own;
            // the wide cell's grapheme already overdraws into it.
            if ((cell.flags & FLAG_WIDE_TAIL) != 0) continue;
            const wide = (cell.flags & FLAG_WIDE) != 0;

            const inverse = (cell.flags & FLAG_INVERSE) != 0;
            const fg = if (inverse) cell.bg else cell.fg;
            const bg = if (inverse) cell.fg else cell.bg;

            const px = @as(f64, @floatFromInt(x)) * cw;
            const py = @as(f64, @floatFromInt(y)) * ch;
            const own_w = if (wide) cw * 2 else cw;

            // Cell background (spans both cells for a wide lead).
            setRgb(cr, bg);
            cairo.Context.rectangle(cr, px, py, own_w, ch);
            cairo.Context.fill(cr);

            if (cell.utf8[0] != 0) {
                const desc = if ((cell.flags & FLAG_BOLD) != 0) state.font_bold else state.font_regular;
                const txt: [*:0]const u8 = @ptrCast(&cell.utf8);
                drawGlyph(cr, layout, desc, txt, px, py, own_w, ch, fg);
            }

            if ((cell.flags & FLAG_UNDERLINE) != 0) {
                setRgb(cr, fg);
                cairo.Context.rectangle(cr, px, py + ch - 1, own_w, 1);
                cairo.Context.fill(cr);
            }

            if (wide) x += 1; // consumed the paired WIDE_TAIL cell above.
        }
    }

    // Cursor: a filled block using the underlying cell's fg, with the glyph
    // repainted in that cell's bg (block-inverse). Style is ignored for v1.
    var cur: Cursor = undefined;
    ndt.ndterm_cursor(@ptrCast(term), @ptrCast(&cur));
    if (cur.visible != 0 and cur.x < cols and cur.y < rows) {
        var cell: Cell = undefined;
        ndt.ndterm_cell(@ptrCast(term), cur.x, cur.y, @ptrCast(&cell));
        const cursor_wide = (cell.flags & FLAG_WIDE) != 0;
        const px = @as(f64, @floatFromInt(cur.x)) * cw;
        const py = @as(f64, @floatFromInt(cur.y)) * ch;
        const own_w = if (cursor_wide) cw * 2 else cw;
        setRgb(cr, cell.fg);
        cairo.Context.rectangle(cr, px, py, own_w, ch);
        cairo.Context.fill(cr);
        if (cell.utf8[0] != 0) {
            const desc = if ((cell.flags & FLAG_BOLD) != 0) state.font_bold else state.font_regular;
            const txt: [*:0]const u8 = @ptrCast(&cell.utf8);
            drawGlyph(cr, layout, desc, txt, px, py, own_w, ch, cell.bg);
        }
    }
}

// ---- input ----

fn send(state: *State, bytes: []const u8) void {
    ndt.ndterm_write_input(@ptrCast(state.term), bytes.ptr, bytes.len);
}

/// GDK button number (1=left, 2=middle, 3=right) -> SGR base button code.
fn sgrBaseButton(gdk_button: c_uint) u8 {
    return switch (gdk_button) {
        1 => 0,
        2 => 1,
        3 => 2,
        else => 0,
    };
}

fn cellCol(state: *State, x: f64) u16 {
    const c: i64 = @intFromFloat(x / state.cell_w);
    return @intCast(std.math.clamp(c, 0, @as(i64, state.cols) - 1));
}

fn cellRow(state: *State, y: f64) u16 {
    const r: i64 = @intFromFloat(y / state.cell_h);
    return @intCast(std.math.clamp(r, 0, @as(i64, state.rows) - 1));
}

/// SGR (1006) mouse sequence: ESC [ < Cb ; Cx ; Cy M|m — M on press/motion, m
/// on release. Cx/Cy are 1-based cell coordinates.
fn sendSgrMouse(state: *State, cb: u8, x: f64, y: f64, press: bool) void {
    const col = cellCol(state, x);
    const row = cellRow(state, y);
    var buf: [32]u8 = undefined;
    const final: u8 = if (press) 'M' else 'm';
    const msg = std.fmt.bufPrint(&buf, "\x1b[<{d};{d};{d}{c}", .{ cb, col + 1, row + 1, final }) catch return;
    send(state, msg);
}

fn onKeyPressed(_: *gtk.EventControllerKey, keyval: c_uint, _: c_uint, mods: gdk.ModifierType, state: *State) callconv(.c) c_int {
    // Ctrl+letter -> the corresponding control byte (a..z / A..Z -> 0x01..0x1a).
    if (mods.control_mask) {
        if ((keyval >= 'a' and keyval <= 'z') or (keyval >= 'A' and keyval <= 'Z')) {
            const b: u8 = @intCast(keyval & 0x1f);
            send(state, &[_]u8{b});
            return 1;
        }
    }

    // Named keys -> their terminal byte sequences.
    const seq: ?[]const u8 = switch (keyval) {
        gdk.KEY_Return => "\r",
        gdk.KEY_BackSpace => "\x7f",
        gdk.KEY_Tab => "\t",
        gdk.KEY_Escape => "\x1b",
        gdk.KEY_Up => "\x1b[A",
        gdk.KEY_Down => "\x1b[B",
        gdk.KEY_Right => "\x1b[C",
        gdk.KEY_Left => "\x1b[D",
        gdk.KEY_Home => "\x1b[H",
        gdk.KEY_End => "\x1b[F",
        gdk.KEY_Page_Up => "\x1b[5~",
        gdk.KEY_Page_Down => "\x1b[6~",
        else => null,
    };
    if (seq) |s| {
        send(state, s);
        return 1;
    }

    // Printable Unicode -> UTF-8 bytes.
    const cp = gdk.keyvalToUnicode(keyval);
    if (cp >= 0x20 and cp != 0x7f and cp <= 0x10ffff) {
        var buf: [4]u8 = undefined;
        const n = std.unicode.utf8Encode(@intCast(cp), &buf) catch return 0;
        send(state, buf[0..n]);
        return 1;
    }

    return 0;
}
