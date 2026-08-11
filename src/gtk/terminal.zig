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
const glib = @import("glib");
const gio = @import("gio");
const gobject = @import("gobject");
const cairo = @import("cairo");
const pango = @import("pango");
const pangocairo = @import("pangocairo");
const ndt = @import("../core/terminal.zig");
const ndremote = @import("../core/remote_terminal.zig");
const protocol = @import("../protocol.zig");
const sprite = @import("sprite.zig");

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
const FLAG_SELECTED: u8 = 1 << 5;

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
    cell_baseline: f64, // cell top to text baseline (Pango ascent), see measureCell
    box_thickness: u32, // light line weight for geometric sprites, see newState
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
    // WP-A1/A2 selection: `selecting` is true while a left-button drag is
    // building a selection; `has_selection` mirrors the last emitted
    // onSelectionChanged so we only fire on a real transition.
    selecting: bool = false,
    has_selection: bool = false,
    // Mirrors the last emitted onFocusChanged so enter/leave only fire on a
    // real transition.
    focused: bool = false,
    // Click-artifact fix (cmux-parity-polish-2 deliverable 3): `sel_pending`
    // is true from a plain left press until either real drag motion crosses
    // into a different cell (promotes to `selecting`, see onMotion) or
    // release with zero motion (never promotes — no selection is created).
    // `sel_anchor_col/row` is the press-time cell, held until that decision.
    sel_pending: bool = false,
    sel_anchor_col: u16 = 0,
    sel_anchor_row: u16 = 0,
    // Last-known pointer cell (from onPressed/onMotion), used to place the
    // SGR coordinate on a wheel-forwarded mouse event (cmux-parity-polish-2
    // deliverable 4) — GtkEventControllerScroll's `scroll` signal carries no
    // position of its own.
    mouse_x: f64 = 0,
    mouse_y: f64 = 0,
    // Last dirty generation the tick callback queued a draw for (perf repaint
    // gate). Sentinel maxInt forces the first tick to paint the initial frame
    // even though a fresh terminal's dirty_seq is 0.
    last_drawn_seq: u64 = std.math.maxInt(u64),
    // Latest nd_rt_state seen by stateTramp (-1 = none yet). The transport
    // starts emitting at createRemote time, BEFORE connectEvents has assigned
    // node_id, so early transitions (always CONNECTING, and — on a reused,
    // already-authed connection — potentially ATTACHED) would be silently
    // dropped by the node_id gate. connectEvents re-emits this so the app's
    // onConnectionState always observes the current state once subscribed.
    last_conn_state: std.atomic.Value(i32) = std.atomic.Value(i32).init(-1),
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
pub fn createRemote(host: [*:0]const u8, port: u16, session_id: [*:0]const u8, ticket: [*:0]const u8, restore_scrollback: bool, font_size: c_int, font_family: ?[*:0]const u8, palette: ?[*:0]const u8, foreground: [*:0]const u8, background: [*:0]const u8, cols: u16, rows: u16) *gtk.Widget {
    const area = gtk.DrawingArea.new();
    const widget = area.as(gtk.Widget);
    const state = newState(widget, area, font_size, font_family, cols, rows);
    state.is_remote = true;

    var open_opts: OpenOpts = .{};
    const opts_ptr = buildOpenOpts(&open_opts, palette, foreground, background);

    const open = if (restore_scrollback) &ndremote.ndrt_open_history else &ndremote.ndrt_open_ex;
    const rt = open(host, port, session_id, ticket, cols, rows, opts_ptr, &effectTramp, &stateTramp, state) orelse {
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

/// Allocate + init per-widget State (term set by the caller after open). Cell
/// metrics are measured from the font (measureCell), not derived from the point
/// size, so glyphs sit on a real baseline and the grid matches the font's box.
fn newState(widget: *gtk.Widget, area: *gtk.DrawingArea, font_size: c_int, font_family: ?[*:0]const u8, cols: u16, rows: u16) *State {
    const fs: f64 = @floatFromInt(font_size);
    const family: [*:0]const u8 = font_family orelse "monospace";
    const font_regular = makeFontDesc(family, fs, .normal);
    const font_bold = makeFontDesc(family, fs, .bold);

    const m = measureCell(font_regular, fs);

    gtk.DrawingArea.setContentWidth(area, @intFromFloat(m.cell_w * @as(f64, @floatFromInt(cols))));
    gtk.DrawingArea.setContentHeight(area, @intFromFloat(m.cell_h * @as(f64, @floatFromInt(rows))));

    // Light line weight for box/block/Powerline sprites, from the cell height
    // (Ghostty derives box_thickness from the font's underline thickness; we
    // don't measure that here, so use its documented fallback). Heavy = 2x.
    const box_thickness: u32 = @max(1, @as(u32, @intFromFloat(@round(m.cell_h * 0.09))));

    const state = std.heap.c_allocator.create(State) catch @panic("OOM allocating terminal State");
    state.* = .{
        .term = undefined,
        .widget = widget,
        .font_size = fs,
        .cell_w = m.cell_w,
        .cell_h = m.cell_h,
        .cell_baseline = m.cell_baseline,
        .box_thickness = box_thickness,
        .cols = cols,
        .rows = rows,
        .font_regular = font_regular,
        .font_bold = font_bold,
    };
    return state;
}

const CellMetrics = struct { cell_w: f64, cell_h: f64, cell_baseline: f64 };

/// Grayscale AA + slight hinting (Ghostty's default look), applied to both the
/// metric-measuring context and the per-frame draw layout so the numbers agree.
fn makeFontOptions() *cairo.FontOptions {
    const opts = cairo.FontOptions.create();
    cairo.FontOptions.setAntialias(opts, .gray);
    cairo.FontOptions.setHintStyle(opts, .slight);
    cairo.FontOptions.setHintMetrics(opts, .on);
    return opts;
}

/// Measure the monospace cell box from the actual font via Pango. cell_w is the
/// max advance over a few wide ASCII glyphs; cell_h is ascent+descent (Pango's
/// line height, no separate line gap); cell_baseline is the ascent, i.e. the
/// distance from the cell top down to the text baseline. Falls back to the old
/// point-size heuristic if the font map yields no usable metrics (headless /
/// missing font).
fn measureCell(font_regular: *pango.FontDescription, fs: f64) CellMetrics {
    const fontmap = pangocairo.FontMap.getDefault();
    const ctx = pango.FontMap.createContext(fontmap);
    defer gobject.Object.unref(ctx.as(gobject.Object));

    const fopts = makeFontOptions();
    defer cairo.FontOptions.destroy(fopts);
    pangocairo.contextSetFontOptions(ctx, fopts);

    const metrics = pango.Context.getMetrics(ctx, font_regular, pango.Language.getDefault());
    defer pango.FontMetrics.unref(metrics);
    const ascent = @as(f64, @floatFromInt(pango.FontMetrics.getAscent(metrics))) / pango.SCALE;
    const descent = @as(f64, @floatFromInt(pango.FontMetrics.getDescent(metrics))) / pango.SCALE;

    const layout = pango.Layout.new(ctx);
    defer gobject.Object.unref(layout.as(gobject.Object));
    pango.Layout.setFontDescription(layout, font_regular);
    var max_w: c_int = 0;
    for ([_][*:0]const u8{ "M", "W", "@", "0" }) |s| {
        pango.Layout.setText(layout, s, -1);
        var w: c_int = 0;
        pango.Layout.getPixelSize(layout, &w, null);
        if (w > max_w) max_w = w;
    }

    const line_h = ascent + descent;
    const cw: f64 = @floatFromInt(max_w);
    return .{
        .cell_w = if (cw > 0) @round(cw) else @round(fs * 0.6),
        .cell_h = if (line_h > 0) @round(line_h) else @round(fs * 1.2),
        .cell_baseline = if (line_h > 0) ascent else @round(fs * 1.2) * 0.8,
    };
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

    // Keyboard-focus tracking for onFocusChanged. The callbacks resolve State
    // through the controller's widget (not a State user-data pointer): a leave
    // fired mid-destruction after onUnrealize freed State must find nothing.
    const focus_ctrl = gtk.EventControllerFocus.new();
    _ = gtk.EventControllerFocus.signals.enter.connect(focus_ctrl, ?*anyopaque, &onFocusEnter, null, .{});
    _ = gtk.EventControllerFocus.signals.leave.connect(focus_ctrl, ?*anyopaque, &onFocusLeave, null, .{});
    gtk.Widget.addController(widget, focus_ctrl.as(gtk.EventController));

    const scroll_ctrl = gtk.EventControllerScroll.new(gtk.EventControllerScrollFlags.flags_vertical);
    _ = gtk.EventControllerScroll.signals.scroll.connect(scroll_ctrl, *State, &onScroll, state, .{});
    gtk.Widget.addController(widget, scroll_ctrl.as(gtk.EventController));

    // WP-B2 image paste: accept dropped images (a raw GdkTexture, e.g. from a
    // browser/screenshot tool) and dropped image files (a GFile). Either yields
    // a local temp PNG path emitted via onImagePaste — the app uploads it and
    // types the returned path. Text/other drops are ignored here.
    const drop = gtk.DropTarget.new(gdk.Texture.getGObjectType(), .{ .copy = true });
    var drop_types = [_]usize{ gdk.Texture.getGObjectType(), gio.File.getGObjectType() };
    gtk.DropTarget.setGtypes(drop, &drop_types, drop_types.len);
    _ = gtk.DropTarget.signals.drop.connect(drop, *State, &onDrop, state, .{});
    gtk.Widget.addController(widget, drop.as(gtk.EventController));

    // Track the drawing area's actual allocation onto the grid (peer of the
    // AppKit setFrameSize path). GtkDrawingArea::resize is the correct
    // observation point — cheaper and more precise than polling in tickCb.
    _ = gtk.DrawingArea.signals.resize.connect(area, *State, &onResize, state, .{});

    // Frame-clock tick gates repaint on the core's dirty generation (see
    // tickCb) so an idle terminal doesn't burn CPU on unchanged redraws.
    _ = gtk.Widget.addTickCallback(widget, &tickCb, state, null);

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
    // Replay the newest connection state that arrived before the subscription
    // existed (see State.last_conn_state). Harmless if stateTramp emits the
    // same state again concurrently — the payload is idempotent app-side.
    const last = state.last_conn_state.load(.seq_cst);
    if (last >= 0) postEmit(node_id, .conn_state, last, null);
}

// effectTramp/stateTramp fire on the ndremote reader thread (and, for a local
// PTY terminal, the core's reader thread). The emit sink (`emit` -> backend.zig
// `emitEventAdapter`) allocates from the process-global `arena`, which is NOT
// thread-safe and is otherwise UI-thread-exclusive (see backend.zig's
// marshal-job comment: crossing threads on it corrupts the heap and surfaces as
// intermittent GTK segfaults / dropped events — e.g. the remote terminal's
// `connectionState` never reaching 'attached' on real Linux GTK). So the
// trampolines only COPY the payload into a thread-safe job here and hop onto the
// GTK main loop via g_main_context_invoke_full, mirroring the AppKit surface's
// `DispatchQueue.main.async`. The main-loop trampoline then does the actual
// emit, keeping every `emit`/`arena` touch on the UI thread.
const emit_alloc = std.heap.smp_allocator;

const EmitJob = struct {
    node_id: u32,
    kind: enum { title, bell, exited, conn_state },
    ival: c_int = 0, // exit code (exited) or state value (conn_state)
    text: ?[:0]u8 = null, // owned title (title) or detail (conn_state); freed by the trampoline
};

fn postEmit(node_id: u32, kind: @FieldType(EmitJob, "kind"), ival: c_int, text: ?[*:0]const u8) void {
    const job = emit_alloc.create(EmitJob) catch return;
    job.* = .{
        .node_id = node_id,
        .kind = kind,
        .ival = ival,
        .text = if (text) |t| emit_alloc.dupeZ(u8, std.mem.span(t)) catch null else null,
    };
    _ = glib.MainContext.default().invokeFull(glib.PRIORITY_DEFAULT, &emitTrampoline, job, null);
}

fn emitTrampoline(data: ?*anyopaque) callconv(.c) c_int {
    const job: *EmitJob = @ptrCast(@alignCast(data.?));
    defer {
        if (job.text) |t| emit_alloc.free(t);
        emit_alloc.destroy(job);
    }
    const f = emit orelse return G_SOURCE_REMOVE;
    switch (job.kind) {
        .title => f(job.node_id, "titleChanged", .{ .text = if (job.text) |t| @as([]const u8, t) else "" }),
        .bell => f(job.node_id, "bell", .{}),
        .exited => {
            var obj: std.json.ObjectMap = .empty;
            defer obj.deinit(std.heap.page_allocator);
            obj.put(std.heap.page_allocator, "code", .{ .integer = @as(i64, job.ival) }) catch return G_SOURCE_REMOVE;
            f(job.node_id, "exited", .{ .data = .{ .object = obj } });
        },
        .conn_state => {
            var obj: std.json.ObjectMap = .empty;
            defer obj.deinit(std.heap.page_allocator);
            obj.put(std.heap.page_allocator, "state", .{ .integer = @as(i64, job.ival) }) catch return G_SOURCE_REMOVE;
            if (job.text) |d| obj.put(std.heap.page_allocator, "detail", .{ .string = d }) catch return G_SOURCE_REMOVE;
            f(job.node_id, "connectionState", .{ .data = .{ .object = obj } });
        },
    }
    return G_SOURCE_REMOVE;
}

fn effectTramp(ud: ?*anyopaque, kind: c_int, text: ?[*:0]const u8, code: c_int) callconv(.c) void {
    const state: *State = @ptrCast(@alignCast(ud orelse return));
    if (state.node_id == 0) return;
    switch (kind) {
        0 => postEmit(state.node_id, .title, 0, text),
        1 => postEmit(state.node_id, .bell, 0, null),
        2 => postEmit(state.node_id, .exited, code, null),
        else => {},
    }
}

fn stateTramp(ud: ?*anyopaque, state_val: c_int, detail: ?[*:0]const u8) callconv(.c) void {
    const state: *State = @ptrCast(@alignCast(ud orelse return));
    state.last_conn_state.store(state_val, .seq_cst);
    if (state.node_id == 0) return;
    postEmit(state.node_id, .conn_state, state_val, detail);
}

const G_SOURCE_CONTINUE: c_int = 1;
const G_SOURCE_REMOVE: c_int = 0;

/// Repaint gate (perf: real-hardware scout measured ~35% idle CPU from this
/// callback's unconditional per-frame redraw). Only queue a draw when the core's
/// dirty generation advanced since the last one — an idle terminal (no output,
/// no scroll/resize) produces no bumps and so never repaints, dropping idle CPU
/// to ~0. The frame-clock tick itself is cheap (one atomic load + compare).
fn tickCb(widget: *gtk.Widget, _: *gdk.FrameClock, ud: ?*anyopaque) callconv(.c) c_int {
    const state: *State = @ptrCast(@alignCast(ud orelse return G_SOURCE_CONTINUE));
    const seq = ndt.ndterm_dirty_seq(@ptrCast(state.term));
    if (seq == state.last_drawn_seq) return G_SOURCE_CONTINUE;
    state.last_drawn_seq = seq;
    gtk.Widget.queueDraw(widget);
    return G_SOURCE_CONTINUE;
}

fn onPressed(gesture: *gtk.GestureClick, n_press: c_int, x: f64, y: f64, state: *State) callconv(.c) void {
    _ = gtk.Widget.grabFocus(state.widget);
    const btn = gtk.GestureSingle.getCurrentButton(gesture.as(gtk.GestureSingle));
    state.mouse_button_down = true;
    state.mouse_last_button = btn;
    state.mouse_x = x;
    state.mouse_y = y;

    const mode = ndt.ndterm_mouse_mode(@ptrCast(state.term));
    const shift = gtk.EventController.getCurrentEventState(gesture.as(gtk.EventController)).shift_mask;
    // Left-button selection when the app isn't grabbing the mouse (mode 0), or
    // when Shift overrides an active mouse-reporting mode — mirrors the AppKit
    // surface's gate. click-count drives word (2) / line (3) vs char (1).
    if (btn == 1 and (mode == 0 or shift)) {
        const col = cellCol(state, x);
        const row = cellRow(state, y);
        if (n_press >= 3) {
            ndt.ndterm_selection_line(@ptrCast(state.term), col, row);
            state.selecting = true;
        } else if (n_press == 2) {
            ndt.ndterm_selection_word(@ptrCast(state.term), col, row);
            state.selecting = true;
        } else {
            // Click-artifact fix: don't start a real selection on a plain
            // press — that used to leave a persistent one-cell phantom
            // highlight (rendered like a second cursor) on every click. Drop
            // any prior selection and just remember the anchor; onMotion
            // promotes it to a real selection only once the drag crosses
            // into a different cell (see onMotion / onReleased below).
            ndt.ndterm_selection_clear(@ptrCast(state.term));
            state.sel_pending = true;
            state.sel_anchor_col = col;
            state.sel_anchor_row = row;
        }
        gtk.Widget.queueDraw(state.widget);
        emitSelectionChanged(state);
        return;
    }

    if (mode == 0) return;
    sendSgrMouse(state, sgrBaseButton(btn), x, y, true);
}

fn onReleased(_: *gtk.GestureClick, _: c_int, x: f64, y: f64, state: *State) callconv(.c) void {
    const btn = state.mouse_last_button;
    state.mouse_button_down = false;
    if (state.selecting) {
        state.selecting = false;
        emitSelectionChanged(state);
        return;
    }
    if (state.sel_pending) {
        // Press+release with zero drag: onPressed never called
        // ndterm_selection_begin, so there is nothing to clear.
        state.sel_pending = false;
        return;
    }
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
    state.mouse_x = x;
    state.mouse_y = y;
    if (state.selecting) {
        ndt.ndterm_selection_extend(@ptrCast(state.term), cellCol(state, x), cellRow(state, y));
        gtk.Widget.queueDraw(state.widget);
        emitSelectionChanged(state);
        return;
    }
    if (state.sel_pending) {
        const col = cellCol(state, x);
        const row = cellRow(state, y);
        if (col == state.sel_anchor_col and row == state.sel_anchor_row) return; // still the same cell: just a click so far
        ndt.ndterm_selection_begin(@ptrCast(state.term), state.sel_anchor_col, state.sel_anchor_row);
        ndt.ndterm_selection_extend(@ptrCast(state.term), col, row);
        state.sel_pending = false;
        state.selecting = true;
        gtk.Widget.queueDraw(state.widget);
        emitSelectionChanged(state);
        return;
    }
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
///
/// cmux-parity-polish-2 deliverable 4: this only helps when the client owns
/// the scrollback view. An app that's grabbed the mouse (ndterm_mouse_mode !=
/// 0, mirroring onPressed's own gate) — including any alt-screen TUI, since
/// those enable mouse reporting to get scroll at all — never sees a local
/// viewport move. Forward the wheel as an SGR mouse-wheel report instead
/// (button 64 = up, 65 = down), one report per callback, the same way
/// onPressed forwards a mouse-mode click.
const SCROLL_LINES_PER_UNIT: f64 = 3.0;
const SGR_WHEEL_UP: u8 = 64;
const SGR_WHEEL_DOWN: u8 = 65;

fn onScroll(_: *gtk.EventControllerScroll, _: f64, dy: f64, state: *State) callconv(.c) c_int {
    if (dy == 0) return 0;
    if (ndt.ndterm_mouse_mode(@ptrCast(state.term)) != 0) {
        const cb: u8 = if (dy < 0) SGR_WHEEL_UP else SGR_WHEEL_DOWN;
        sendSgrMouse(state, cb, state.mouse_x, state.mouse_y, true);
        return 1;
    }
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

/// A grapheme up to this multiple of the cell is drawn at natural size (clipped
/// to the cell); larger than this it is scaled to fit. Box-drawing and Powerline
/// glyphs overhang the advance by design to tile (measured ~1.15-1.22x), so the
/// slack keeps them tiling while genuinely oversized icons/emoji (~1.5-2x) still
/// scale instead of being sliced. Interim heuristic until box-drawing/Powerline
/// get procedural sprites.
const FIT_SLACK: f64 = 1.3;

/// First codepoint of a cell's grapheme, but only when it is a geometric
/// sprite (box-drawing/block/braille/Powerline). Those are drawn as cairo
/// geometry so borders tile and Powerline separators meet the neighbour with
/// no seam; null falls through to the font path.
fn spriteCodepoint(utf8: *const [16]u8) ?u32 {
    if (utf8[0] == 0) return null;
    const n = std.unicode.utf8ByteSequenceLength(utf8[0]) catch return null;
    if (n > utf8.len) return null;
    const cp = std.unicode.utf8Decode(utf8[0..n]) catch return null;
    return if (sprite.contains(cp)) cp else null;
}

/// Draws one cell's grapheme in the cell box at (px, py) sized own_w x ch, on
/// the text baseline at py + baseline. A grapheme whose ink fits (within
/// FIT_SLACK) is drawn at natural size; an oversized one (Nerd Font PUA icon,
/// emoji substitute) is uniformly scaled to contain within the box (never
/// enlarged) and centered, instead of being sliced. The clip to the cell box is
/// a safety bound against spill into the neighbor, never a tighter cut.
fn drawGlyph(cr: *cairo.Context, layout: *pango.Layout, desc: *pango.FontDescription, txt: [*:0]const u8, px: f64, py: f64, own_w: f64, ch: f64, baseline: f64, rgb: [3]u8) void {
    pango.Layout.setFontDescription(layout, desc);
    pango.Layout.setText(layout, txt, -1);
    setRgb(cr, rgb);

    var ink: pango.Rectangle = undefined;
    pango.Layout.getPixelExtents(layout, &ink, null);
    const ink_w: f64 = @floatFromInt(ink.f_width);
    const ink_h: f64 = @floatFromInt(ink.f_height);

    cairo.Context.save(cr);
    cairo.Context.rectangle(cr, px, py, own_w, ch);
    cairo.Context.clip(cr);
    if (ink_w <= own_w * FIT_SLACK and ink_h <= ch * FIT_SLACK) {
        // Natural size on the baseline. The cell-box clip turns a glyph that
        // overhangs the advance (box-drawing/powerline are cut to overhang by
        // design so neighbours tile) into an edge-to-edge fill; a plain ASCII
        // glyph sits well inside it.
        const layout_baseline = @as(f64, @floatFromInt(pango.Layout.getBaseline(layout))) / pango.SCALE;
        cairo.Context.moveTo(cr, px, py + baseline - layout_baseline);
        pangocairo.showLayout(cr, layout);
    } else {
        const sx = if (ink_w > 0) own_w / ink_w else 1.0;
        const sy = if (ink_h > 0) ch / ink_h else 1.0;
        const scale = @min(@as(f64, 1.0), @min(sx, sy));
        const ox = px + (own_w - ink_w * scale) / 2.0;
        const oy = py + (ch - ink_h * scale) / 2.0;
        cairo.Context.translate(cr, ox, oy);
        cairo.Context.scale(cr, scale, scale);
        cairo.Context.translate(cr, -@as(f64, @floatFromInt(ink.f_x)), -@as(f64, @floatFromInt(ink.f_y)));
        cairo.Context.moveTo(cr, 0, 0);
        pangocairo.showLayout(cr, layout);
    }
    cairo.Context.restore(cr);
}

/// Fill one coalesced background run of `w` px at (px, row*ch). No-op for an
/// empty run (w == 0), which is how the caller skips default-bg gaps.
fn flushBgRun(cr: *cairo.Context, px: f64, row: u16, w: f64, ch: f64, rgb: [3]u8) void {
    if (w == 0) return;
    setRgb(cr, rgb);
    cairo.Context.rectangle(cr, px, @as(f64, @floatFromInt(row)) * ch, w, ch);
    cairo.Context.fill(cr);
}

fn drawCb(area: *gtk.DrawingArea, cr: *cairo.Context, width: c_int, height: c_int, _: ?*anyopaque) callconv(.c) void {
    const state = stateFrom(area.as(gtk.Widget)) orelse return;
    const term = state.term;
    const cw = state.cell_w;
    const ch = state.cell_h;

    // Scrollback indicator state, queried BEFORE the render lock:
    // ndterm_scrollback_state takes the terminal mutex itself, and render_lock
    // keeps that same (non-recursive) mutex held until render_unlock — calling
    // it inside the locked region self-deadlocks the UI thread on the first
    // draw. One frame of staleness in an advisory thumb is invisible.
    var sb_total: usize = 0;
    var sb_offset: usize = 0;
    const sb_pinned = ndt.ndterm_scrollback_state(@ptrCast(term), &sb_total, null, &sb_offset);

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
    const fopts = makeFontOptions();
    defer cairo.FontOptions.destroy(fopts);
    pangocairo.contextSetFontOptions(pango.Layout.getContext(layout), fopts);
    pango.Layout.contextChanged(layout);

    var y: u16 = 0;
    while (y < rows) : (y += 1) {
        // Pass 1 — backgrounds, before any glyph so a coalesced run can't
        // overpaint the text drawn on top of it. Contiguous cells sharing a
        // non-default bg collapse into one cairo fill (perf: turns a per-cell
        // fill into ~runs-per-row); a default-bg cell needs no fill at all, the
        // whole-widget fill above already covers it.
        var run_start_px: f64 = 0;
        var run_w: f64 = 0;
        var run_bg: [3]u8 = def_bg;
        var x: u16 = 0;
        while (x < cols) : (x += 1) {
            var cell: Cell = undefined;
            ndt.ndterm_cell(@ptrCast(term), x, y, @ptrCast(&cell));
            if ((cell.flags & FLAG_WIDE_TAIL) != 0) continue;
            const wide = (cell.flags & FLAG_WIDE) != 0;
            // A selected cell swaps fg/bg like INVERSE does; the two compose by
            // XOR (selection over inverse text stays legible).
            const inverse = ((cell.flags & FLAG_INVERSE) != 0) != ((cell.flags & FLAG_SELECTED) != 0);
            const bg = if (inverse) cell.fg else cell.bg;
            const px = @as(f64, @floatFromInt(x)) * cw;
            const own_w = if (wide) cw * 2 else cw;

            if (std.mem.eql(u8, &bg, &def_bg)) {
                flushBgRun(cr, run_start_px, y, run_w, ch, run_bg);
                run_w = 0;
            } else if (run_w != 0 and std.mem.eql(u8, &bg, &run_bg)) {
                run_w += own_w;
            } else {
                flushBgRun(cr, run_start_px, y, run_w, ch, run_bg);
                run_start_px = px;
                run_w = own_w;
                run_bg = bg;
            }
            if (wide) x += 1;
        }
        flushBgRun(cr, run_start_px, y, run_w, ch, run_bg);

        // Pass 2 — glyphs + underline, per cell (cell_w is a rounded metric,
        // not the font's measured advance, so a concatenated Pango run would
        // drift off the grid; the explicit per-cell origin is load-bearing).
        x = 0;
        while (x < cols) : (x += 1) {
            var cell: Cell = undefined;
            ndt.ndterm_cell(@ptrCast(term), x, y, @ptrCast(&cell));
            if ((cell.flags & FLAG_WIDE_TAIL) != 0) continue;
            const wide = (cell.flags & FLAG_WIDE) != 0;
            const inverse = ((cell.flags & FLAG_INVERSE) != 0) != ((cell.flags & FLAG_SELECTED) != 0);
            const fg = if (inverse) cell.bg else cell.fg;
            const px = @as(f64, @floatFromInt(x)) * cw;
            const py = @as(f64, @floatFromInt(y)) * ch;
            const own_w = if (wide) cw * 2 else cw;

            if (cell.utf8[0] != 0) {
                if (spriteCodepoint(&cell.utf8)) |cp| {
                    _ = sprite.draw(cr, cp, px, py, own_w, ch, state.box_thickness, fg);
                } else {
                    const desc = if ((cell.flags & FLAG_BOLD) != 0) state.font_bold else state.font_regular;
                    const txt: [*:0]const u8 = @ptrCast(&cell.utf8);
                    drawGlyph(cr, layout, desc, txt, px, py, own_w, ch, state.cell_baseline, fg);
                }
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
            if (spriteCodepoint(&cell.utf8)) |cp| {
                _ = sprite.draw(cr, cp, px, py, own_w, ch, state.box_thickness, cell.bg);
            } else {
                const desc = if ((cell.flags & FLAG_BOLD) != 0) state.font_bold else state.font_regular;
                const txt: [*:0]const u8 = @ptrCast(&cell.utf8);
                drawGlyph(cr, layout, desc, txt, px, py, own_w, ch, state.cell_baseline, cell.bg);
            }
        }
    }

    // Scrollback indicator: a thin right-edge thumb, shown only while scrolled
    // into history (pinned == 0). Height/position track the viewport's share of
    // the total scrollable area. (State queried above, before the render lock.)
    if (sb_pinned == 0 and sb_total > @as(usize, rows)) {
        const track_h: f64 = @floatFromInt(height);
        const total_f: f64 = @floatFromInt(sb_total);
        const thumb_h = @max(track_h * @as(f64, @floatFromInt(rows)) / total_f, 16.0);
        const thumb_y = (track_h - thumb_h) * @as(f64, @floatFromInt(sb_offset)) / @max(total_f - @as(f64, @floatFromInt(rows)), 1.0);
        setRgb(cr, .{ 0x88, 0x88, 0x88 });
        cairo.Context.rectangle(cr, @as(f64, @floatFromInt(width)) - 4.0, thumb_y, 3.0, thumb_h);
        cairo.Context.fill(cr);
    }
}

// ---- selection / clipboard / commands (WP-A2 / WP-A4 / WP-B2) ----

/// Fire onSelectionChanged only when the has-selection state actually flips.
/// Called on the UI thread (gestures/keys/commands), so it emits directly —
/// unlike the reader-thread effect callbacks, no main-loop marshaling needed.
fn emitSelectionChanged(state: *State) void {
    if (state.node_id == 0) return;
    const f = emit orelse return;
    const has = ndt.ndterm_selection_text(@ptrCast(state.term), null, 0) != 0;
    if (has == state.has_selection) return;
    state.has_selection = has;
    f(state.node_id, "selectionChanged", .{ .checked = has });
}

fn onFocusEnter(ctrl: *gtk.EventControllerFocus, _: ?*anyopaque) callconv(.c) void {
    emitFocusChanged(ctrl, true);
}

fn onFocusLeave(ctrl: *gtk.EventControllerFocus, _: ?*anyopaque) callconv(.c) void {
    emitFocusChanged(ctrl, false);
}

/// Fire onFocusChanged only on a real transition, on the UI thread (focus
/// signals fire on the main loop, like the gestures). State is resolved via
/// stateFrom so a signal firing during widget destruction, after onUnrealize
/// cleared the qdata and freed State, is a no-op instead of a use-after-free.
fn emitFocusChanged(ctrl: *gtk.EventControllerFocus, focused: bool) void {
    const widget = gtk.EventController.getWidget(ctrl.as(gtk.EventController)) orelse return;
    const state = stateFrom(widget) orelse return;
    if (state.node_id == 0) return;
    const f = emit orelse return;
    if (state.focused == focused) return;
    state.focused = focused;
    f(state.node_id, "focusChanged", .{ .checked = focused });
}

fn displayClipboard() ?*gdk.Clipboard {
    const display = gdk.Display.getDefault() orelse return null;
    return gdk.Display.getClipboard(display);
}

fn copyToClipboard(state: *State) void {
    const need = ndt.ndterm_selection_text(@ptrCast(state.term), null, 0);
    if (need == 0) return;
    const buf = std.heap.c_allocator.allocSentinel(u8, need, 0) catch return;
    defer std.heap.c_allocator.free(buf);
    // The reader thread can grow the selection between the size query and the
    // read; ndterm.h documents the return may exceed buf_len — clamp it.
    const n = @min(ndt.ndterm_selection_text(@ptrCast(state.term), buf.ptr, need), need);
    buf[n] = 0;
    const clip = displayClipboard() orelse return;
    gdk.Clipboard.setText(clip, buf.ptr);
}

const PasteJob = struct { state: *State };

/// Ctrl+Shift+V / the `paste` command. Probes the clipboard for an image first
/// (WP-B2): if it holds image/png, read it as a texture, save a local temp PNG,
/// and emit onImagePaste{path} instead of typing text; otherwise paste text.
fn pasteFromClipboard(state: *State) void {
    const clip = displayClipboard() orelse return;
    const formats = gdk.Clipboard.getFormats(clip);
    const job = std.heap.c_allocator.create(PasteJob) catch return;
    job.* = .{ .state = state };
    if (gdk.ContentFormats.containMimeType(formats, "image/png") != 0) {
        gdk.Clipboard.readTextureAsync(clip, null, &cbPasteImage, job);
    } else {
        gdk.Clipboard.readTextAsync(clip, null, &cbPasteText, job);
    }
}

fn cbPasteText(source: ?*gobject.Object, res: *gio.AsyncResult, user_data: ?*anyopaque) callconv(.c) void {
    const job: *PasteJob = @ptrCast(@alignCast(user_data.?));
    defer std.heap.c_allocator.destroy(job);
    const clip: *gdk.Clipboard = @ptrCast(@alignCast(source.?));
    var err: ?*glib.Error = null;
    const text = gdk.Clipboard.readTextFinish(clip, res, &err);
    if (text) |t| {
        defer glib.free(t);
        const s = std.mem.span(t);
        ndt.ndterm_write_paste(@ptrCast(job.state.term), s.ptr, s.len);
    } else if (err) |e| e.free();
}

fn cbPasteImage(source: ?*gobject.Object, res: *gio.AsyncResult, user_data: ?*anyopaque) callconv(.c) void {
    const job: *PasteJob = @ptrCast(@alignCast(user_data.?));
    defer std.heap.c_allocator.destroy(job);
    const clip: *gdk.Clipboard = @ptrCast(@alignCast(source.?));
    var err: ?*glib.Error = null;
    const texture = gdk.Clipboard.readTextureFinish(clip, res, &err);
    if (texture) |tex| {
        defer gobject.Object.unref(tex.as(gobject.Object));
        saveTextureAndEmit(job.state, tex);
    } else if (err) |e| e.free();
}

/// Drop handler: a dropped GdkTexture is saved to a temp PNG; a dropped GFile
/// image is emitted by its own path (no copy). Returns whether the drop was
/// accepted.
fn onDrop(_: *gtk.DropTarget, value: *gobject.Value, _: f64, _: f64, state: *State) callconv(.c) c_int {
    const obj = gobject.Value.getObject(value) orelse return 0;
    if (gobject.ext.cast(gdk.Texture, obj)) |tex| {
        saveTextureAndEmit(state, tex);
        return 1;
    }
    if (gobject.ext.cast(gio.File, obj)) |file| {
        const path = gio.File.getPath(file) orelse return 0;
        defer glib.free(path);
        emitImagePaste(state, std.mem.span(path));
        return 1;
    }
    return 0;
}

/// Write `tex` to a fresh local temp PNG and emit its path via onImagePaste.
fn saveTextureAndEmit(state: *State, tex: *gdk.Texture) void {
    var buf: [256]u8 = undefined;
    const path = tempPngPath(&buf) orelse return;
    if (gdk.Texture.saveToPng(tex, path.ptr) == 0) return;
    emitImagePaste(state, path);
}

var paste_counter: std.atomic.Value(u64) = std.atomic.Value(u64).init(0);

/// Build a unique `<tmp>/nd-clip-<pid>-<n>.png` path into `buf`.
fn tempPngPath(buf: []u8) ?[:0]const u8 {
    const dir_z = std.c.getenv("TMPDIR");
    const dir: []const u8 = if (dir_z) |d| std.mem.span(d) else "/tmp";
    const n = paste_counter.fetchAdd(1, .monotonic);
    const pid = std.c.getpid();
    return std.fmt.bufPrintZ(buf, "{s}/nd-clip-{d}-{d}.png", .{ dir, pid, n }) catch null;
}

fn emitImagePaste(state: *State, path: []const u8) void {
    if (state.node_id == 0) return;
    const f = emit orelse return;
    var obj: std.json.ObjectMap = .empty;
    defer obj.deinit(std.heap.page_allocator);
    obj.put(std.heap.page_allocator, "path", .{ .string = path }) catch return;
    f(state.node_id, "imagePaste", .{ .data = .{ .object = obj } });
}

/// Generated widgetCommand Terminal arm (WP-A4): copy/paste/selectAll/
/// clearSelection/focus. `widget` is the DrawingArea carrying the State. Named
/// `runCommand` (not `command`) to avoid shadowing create()'s `command` param.
pub fn runCommand(widget: *gtk.Widget, cmd: []const u8, _: ?std.json.Value) void {
    const state = stateFrom(widget) orelse return;
    if (std.mem.eql(u8, cmd, "focus")) {
        _ = gtk.Widget.grabFocus(state.widget);
    } else if (std.mem.eql(u8, cmd, "copy")) {
        copyToClipboard(state);
    } else if (std.mem.eql(u8, cmd, "paste")) {
        pasteFromClipboard(state);
    } else if (std.mem.eql(u8, cmd, "selectAll")) {
        ndt.ndterm_selection_all(@ptrCast(state.term));
        gtk.Widget.queueDraw(state.widget);
        emitSelectionChanged(state);
    } else if (std.mem.eql(u8, cmd, "clearSelection")) {
        ndt.ndterm_selection_clear(@ptrCast(state.term));
        gtk.Widget.queueDraw(state.widget);
        emitSelectionChanged(state);
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
    // Ctrl+Shift+C/V: copy the selection / paste the clipboard. Intercepted
    // before the generic Ctrl+letter -> control-byte mapping below (which would
    // otherwise send 0x03/0x16 into the PTY).
    if (mods.control_mask and mods.shift_mask) {
        if (keyval == 'C' or keyval == 'c') {
            copyToClipboard(state);
            return 1;
        }
        if (keyval == 'V' or keyval == 'v') {
            pasteFromClipboard(state);
            return 1;
        }
    }

    // Shift+PageUp/Down/Home/End: move the scrollback viewport (client-local,
    // no PTY round-trip). Home/End jump to the extremes via a large delta the
    // core clamps to the scrollback bounds.
    if (mods.shift_mask) {
        const rows: c_int = @intCast(state.rows);
        const scroll_delta: ?c_int = switch (keyval) {
            gdk.KEY_Page_Up => -rows,
            gdk.KEY_Page_Down => rows,
            gdk.KEY_Home => -1_000_000,
            gdk.KEY_End => 1_000_000,
            else => null,
        };
        if (scroll_delta) |d| {
            ndt.ndterm_scroll_viewport(@ptrCast(state.term), d);
            gtk.Widget.queueDraw(state.widget);
            return 1;
        }
    }

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
