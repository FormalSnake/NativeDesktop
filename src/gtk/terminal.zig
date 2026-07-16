// GTK4 rendering/input surface for the <terminal> widget. Drives the terminal
// core (src/core/terminal.zig, the `ndterm` C ABI — see include/ndterm.h) and
// paints its viewport grid onto a GtkDrawingArea with cairo. The core owns the
// PTY + libghostty-vt behind a mutex; this file only snapshots+draws and maps
// key events to bytes. One State per widget: ndterm_open() at create,
// ndterm_close() + free at unrealize.
const std = @import("std");
const gtk = @import("gtk");
const gdk = @import("gdk");
const gobject = @import("gobject");
const cairo = @import("cairo");
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
};

fn stateFrom(widget: *gtk.Widget) ?*State {
    const raw = gobject.Object.getData(widget.as(gobject.Object), STATE_KEY) orelse return null;
    return @ptrCast(@alignCast(raw));
}

pub fn create(command: ?[*:0]const u8, cwd: ?[*:0]const u8, font_size: c_int, cols: u16, rows: u16) *gtk.Widget {
    const area = gtk.DrawingArea.new();
    const widget = area.as(gtk.Widget);
    const state = newState(widget, area, font_size, cols, rows);

    // Pass the State as the effect userdata (title/bell/child-exit) so the
    // generated connectEvents arm can wire it to NDP events.
    const term = ndt.ndterm_open(cols, rows, command, cwd, &effectTramp, state) orelse {
        // Core failed to spawn the PTY: return the bare DrawingArea (blank).
        std.debug.print("ND_WARN ndterm_open failed\n", .{});
        std.heap.c_allocator.destroy(state);
        return widget;
    };
    state.term = @ptrCast(term);

    wireSurface(area, widget, state);
    return widget;
}

/// Remote variant: the grid is fed by the byte-plane transport (ndremote)
/// instead of a local PTY. Same rendering/input surface; effect + connection
/// state route to NDP events via effectTramp/stateTramp.
pub fn createRemote(host: [*:0]const u8, port: u16, session_id: [*:0]const u8, ticket: [*:0]const u8, font_size: c_int, cols: u16, rows: u16) *gtk.Widget {
    const area = gtk.DrawingArea.new();
    const widget = area.as(gtk.Widget);
    const state = newState(widget, area, font_size, cols, rows);
    state.is_remote = true;

    const rt = ndremote.ndrt_open(host, port, session_id, ticket, cols, rows, &effectTramp, &stateTramp, state) orelse {
        std.debug.print("ND_WARN ndrt_open failed\n", .{});
        std.heap.c_allocator.destroy(state);
        return widget;
    };
    state.rt = @ptrCast(rt);
    const term = ndremote.ndrt_terminal(rt) orelse {
        ndremote.ndrt_close(rt);
        std.heap.c_allocator.destroy(state);
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
fn newState(widget: *gtk.Widget, area: *gtk.DrawingArea, font_size: c_int, cols: u16, rows: u16) *State {
    const fs: f64 = @floatFromInt(font_size);
    const cell_w = @round(fs * 0.6);
    const cell_h = @round(fs * 1.2);

    gtk.DrawingArea.setContentWidth(area, @intFromFloat(cell_w * @as(f64, @floatFromInt(cols))));
    gtk.DrawingArea.setContentHeight(area, @intFromFloat(cell_h * @as(f64, @floatFromInt(rows))));

    const state = std.heap.c_allocator.create(State) catch @panic("OOM allocating terminal State");
    state.* = .{
        .term = undefined,
        .widget = widget,
        .font_size = fs,
        .cell_w = cell_w,
        .cell_h = cell_h,
        .cols = cols,
        .rows = rows,
    };
    return state;
}

/// Shared surface wiring for both the local and remote paths.
fn wireSurface(area: *gtk.DrawingArea, widget: *gtk.Widget, state: *State) void {
    gobject.Object.setData(widget.as(gobject.Object), STATE_KEY, state);
    gtk.DrawingArea.setDrawFunc(area, &drawCb, state, null);

    // Focusable + key controller for input, click gesture to grab focus.
    gtk.Widget.setFocusable(widget, 1);
    const key_ctrl = gtk.EventControllerKey.new();
    _ = gtk.EventControllerKey.signals.key_pressed.connect(key_ctrl, *State, &onKeyPressed, state, .{});
    gtk.Widget.addController(widget, key_ctrl.as(gtk.EventController));

    const click = gtk.GestureClick.new();
    _ = gtk.GestureClick.signals.pressed.connect(click, *State, &onPressed, state, .{});
    gtk.Widget.addController(widget, click.as(gtk.EventController));

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

fn onPressed(_: *gtk.GestureClick, _: c_int, _: f64, _: f64, state: *State) callconv(.c) void {
    _ = gtk.Widget.grabFocus(state.widget);
}

fn onUnrealize(widget: *gtk.Widget, state: *State) callconv(.c) void {
    if (state.is_remote) {
        if (state.rt) |rt| ndremote.ndrt_close(@ptrCast(rt)); // closes the virtual ndterm too
    } else {
        ndt.ndterm_close(@ptrCast(state.term));
    }
    gobject.Object.setData(widget.as(gobject.Object), STATE_KEY, null);
    std.heap.c_allocator.destroy(state);
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

    cairo.Context.selectFontFace(cr, "monospace", .normal, .normal);
    cairo.Context.setFontSize(cr, state.font_size);
    var fe: cairo.FontExtents = undefined;
    cairo.Context.fontExtents(cr, &fe);
    const ascent = fe.ascent;

    var y: u16 = 0;
    while (y < rows) : (y += 1) {
        var x: u16 = 0;
        while (x < cols) : (x += 1) {
            var cell: Cell = undefined;
            ndt.ndterm_cell(@ptrCast(term), x, y, @ptrCast(&cell));

            const inverse = (cell.flags & FLAG_INVERSE) != 0;
            const fg = if (inverse) cell.bg else cell.fg;
            const bg = if (inverse) cell.fg else cell.bg;

            const px = @as(f64, @floatFromInt(x)) * cw;
            const py = @as(f64, @floatFromInt(y)) * ch;

            // Cell background.
            setRgb(cr, bg);
            cairo.Context.rectangle(cr, px, py, cw, ch);
            cairo.Context.fill(cr);

            // Glyph (skip blank cells: utf8 == "").
            if (cell.utf8[0] != 0) {
                if ((cell.flags & FLAG_BOLD) != 0)
                    cairo.Context.selectFontFace(cr, "monospace", .normal, .bold)
                else
                    cairo.Context.selectFontFace(cr, "monospace", .normal, .normal);
                setRgb(cr, fg);
                cairo.Context.moveTo(cr, px, py + ascent);
                const txt: [*:0]const u8 = @ptrCast(&cell.utf8);
                cairo.Context.showText(cr, txt);
            }

            if ((cell.flags & FLAG_UNDERLINE) != 0) {
                setRgb(cr, fg);
                cairo.Context.rectangle(cr, px, py + ch - 1, cw, 1);
                cairo.Context.fill(cr);
            }
        }
    }

    // Cursor: a filled block using the underlying cell's fg, with the glyph
    // repainted in that cell's bg (block-inverse). Style is ignored for v1.
    var cur: Cursor = undefined;
    ndt.ndterm_cursor(@ptrCast(term), @ptrCast(&cur));
    if (cur.visible != 0 and cur.x < cols and cur.y < rows) {
        var cell: Cell = undefined;
        ndt.ndterm_cell(@ptrCast(term), cur.x, cur.y, @ptrCast(&cell));
        const px = @as(f64, @floatFromInt(cur.x)) * cw;
        const py = @as(f64, @floatFromInt(cur.y)) * ch;
        setRgb(cr, cell.fg);
        cairo.Context.rectangle(cr, px, py, cw, ch);
        cairo.Context.fill(cr);
        if (cell.utf8[0] != 0) {
            cairo.Context.selectFontFace(cr, "monospace", .normal, .normal);
            setRgb(cr, cell.bg);
            cairo.Context.moveTo(cr, px, py + ascent);
            const txt: [*:0]const u8 = @ptrCast(&cell.utf8);
            cairo.Context.showText(cr, txt);
        }
    }
}

// ---- input ----

fn send(state: *State, bytes: []const u8) void {
    ndt.ndterm_write_input(@ptrCast(state.term), bytes.ptr, bytes.len);
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
