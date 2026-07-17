//! Terminal core — the `ndterm` C ABI (include/ndterm.h).
//!
//! Owns one PTY (via `forkpty`) plus one libghostty-vt terminal instance behind
//! an internal mutex, with a dedicated reader thread pumping PTY output into the
//! VT parser. Both backends (the GTK Zig surface, the AppKit Swift surface)
//! consume ONLY the `ndterm_*` exports below; neither touches libghostty-vt
//! directly. `src/core/root.zig` force-retains these `export fn` symbols in
//! `libnd.a`.
//!
//! GTK-free: this file imports only `std`/`builtin` and hand-declared
//! `extern "c"` symbols (libghostty-vt is Zig 0.16 where `@cImport` is gone, so
//! every libghostty-vt / libc entry point is declared by hand below from the
//! vendored headers in vendor/libghostty-vt/include/ghostty/vt/).
//!
//! Threading model:
//!   - The reader thread loops `read(amaster)` and, under `mutex`, feeds bytes to
//!     `ghostty_terminal_vt_write`. libghostty-vt effect callbacks (write_pty,
//!     title, bell) fire synchronously inside that call, i.e. on the reader
//!     thread while `mutex` is held — they must not re-enter `vt_write` nor
//!     re-lock `mutex`.
//!   - `ndterm_render_lock` takes `mutex`, runs the render-state update, snapshots
//!     the whole viewport into an owned `[]nd_term_cell`, and KEEPS the mutex held
//!     until `ndterm_render_unlock`. `ndterm_cell`/`ndterm_cursor`/
//!     `ndterm_default_colors` read that snapshot with no additional locking
//!     (they run between lock and unlock).
//!   - `ndterm_write_input` locks and writes already-encoded bytes to the PTY.

const std = @import("std");
const builtin = @import("builtin");

const gpa = std.heap.c_allocator;

// ---------------------------------------------------------------------------
// ndterm.h ABI structs (byte-for-byte with include/ndterm.h).
// ---------------------------------------------------------------------------

/// include/ndterm.h `nd_term_cell` — a flattened, render-ready cell.
/// `utf8` is a NUL-terminated grapheme cluster ("" = blank).
const nd_term_cell = extern struct {
    utf8: [16]u8,
    fg: [3]u8,
    bg: [3]u8,
    flags: u8,
};

/// include/ndterm.h `nd_term_cursor`.
const nd_term_cursor = extern struct {
    x: u16,
    y: u16,
    visible: u8,
    style: u8,
};

/// include/ndterm.h `nd_term_open_opts` — open-time default fg/bg + 256-color
/// palette (WP polish-1 deliverable 7). Shared with remote_terminal.zig's
/// `ndrt_open_ex`, so it must stay `pub`. `palette_rgb`/`palette_len` are a
/// borrowed buffer only read during the `*_ex` open call, never retained.
pub const nd_term_open_opts = extern struct {
    palette_rgb: ?[*]const u8, // NULL, or exactly PALETTE_COLORS*3 packed r,g,b bytes
    palette_len: usize, // must equal PALETTE_COLORS*3 when palette_rgb != null
    has_fg: u8, // 0/1: apply fg[3] as the default foreground
    fg: [3]u8,
    has_bg: u8, // 0/1: apply bg[3] as the default background
    bg: [3]u8,
};

const NDTERM_FLAG_BOLD: u8 = 1 << 0;
const NDTERM_FLAG_UNDERLINE: u8 = 1 << 1;
const NDTERM_FLAG_INVERSE: u8 = 1 << 2;
const NDTERM_FLAG_WIDE: u8 = 1 << 3;
const NDTERM_FLAG_WIDE_TAIL: u8 = 1 << 4;

/// include/ndterm.h `nd_term_effect_cb`. kind: 0 title (`text`=new title),
/// 1 bell, 2 child-exit (`code`=exit status).
const EffectCb = *const fn (userdata: ?*anyopaque, kind: c_int, text: ?[*:0]const u8, code: c_int) callconv(.c) void;

/// include/ndterm.h `nd_term_output_cb`. In virtual mode this replaces the
/// write(amaster) path: both keystrokes and VT query responses route here.
const OutputCb = *const fn (userdata: ?*anyopaque, bytes: [*]const u8, len: usize) callconv(.c) void;

// ---------------------------------------------------------------------------
// libghostty-vt types (from vendor/libghostty-vt/include/ghostty/vt/*.h).
// Opaque handles are `?*anyopaque`. GhosttyResult is c_int (GHOSTTY_SUCCESS=0).
// ---------------------------------------------------------------------------

const GHOSTTY_SUCCESS: c_int = 0;

/// terminal.h `GhosttyTerminalOptions`.
const GhosttyTerminalOptions = extern struct {
    cols: u16,
    rows: u16,
    max_scrollback: usize,
};

/// color.h `GhosttyColorRgb`.
const GhosttyColorRgb = extern struct {
    r: u8,
    g: u8,
    b: u8,
};

/// types.h `GhosttyString` (borrowed ptr+len).
const GhosttyString = extern struct {
    ptr: ?[*]const u8,
    len: usize,
};

/// types.h `GhosttyBuffer` (caller-provided out buffer).
const GhosttyBuffer = extern struct {
    ptr: ?[*]u8,
    cap: usize,
    len: usize,
};

/// style.h `GhosttyStyleColorValue`.
const GhosttyStyleColorValue = extern union {
    palette: u8,
    rgb: GhosttyColorRgb,
    _padding: u64,
};

/// style.h `GhosttyStyleColor`.
const GhosttyStyleColor = extern struct {
    tag: c_int,
    value: GhosttyStyleColorValue,
};

/// style.h `GhosttyStyle` (sized struct — init `size` = @sizeOf).
const GhosttyStyle = extern struct {
    size: usize,
    fg_color: GhosttyStyleColor,
    bg_color: GhosttyStyleColor,
    underline_color: GhosttyStyleColor,
    bold: bool,
    italic: bool,
    faint: bool,
    blink: bool,
    inverse: bool,
    invisible: bool,
    strikethrough: bool,
    overline: bool,
    underline: c_int,
};

/// terminal.h `GhosttyTerminalScrollViewportValue` (tagged union value).
const GhosttyTerminalScrollViewportValue = extern union {
    delta: isize,
    _padding: [2]u64,
};

/// terminal.h `GhosttyTerminalScrollViewport` (tagged union for
/// ghostty_terminal_scroll_viewport). Only the DELTA tag is used here.
const GhosttyTerminalScrollViewport = extern struct {
    tag: c_int,
    value: GhosttyTerminalScrollViewportValue,
};

/// render.h `GhosttyRenderStateColors` (sized struct — init `size` = @sizeOf).
const GhosttyRenderStateColors = extern struct {
    size: usize,
    background: GhosttyColorRgb,
    foreground: GhosttyColorRgb,
    cursor: GhosttyColorRgb,
    cursor_has_value: bool,
    palette: [256]GhosttyColorRgb,
};

// terminal.h `GhosttyTerminalOption` values used here.
const OPT_USERDATA: c_int = 0;
const OPT_WRITE_PTY: c_int = 1;
const OPT_BELL: c_int = 2;
const OPT_TITLE_CHANGED: c_int = 5;
const OPT_COLOR_FOREGROUND: c_int = 11;
const OPT_COLOR_BACKGROUND: c_int = 12;
const OPT_COLOR_PALETTE: c_int = 14;

// terminal.h `GhosttyTerminalScrollViewportTag` values used here.
const SCROLL_VIEWPORT_DELTA: c_int = 2;

// modes.h `GHOSTTY_MODE_*` — DEC private mouse modes (ansi=false, so the
// packed `GhosttyMode` u16 equals the raw mode number below).
const MODE_X10_MOUSE: u16 = 9;
const MODE_NORMAL_MOUSE: u16 = 1000;
const MODE_BUTTON_MOUSE: u16 = 1002;
const MODE_ANY_MOUSE: u16 = 1003;
const MODE_UTF8_MOUSE: u16 = 1005;
const MODE_SGR_MOUSE: u16 = 1006;
const MODE_URXVT_MOUSE: u16 = 1015;
const MODE_SGR_PIXELS_MOUSE: u16 = 1016;

// include/ndterm.h `NDTERM_MOUSE_*` bitmask values returned by ndterm_mouse_mode.
const NDTERM_MOUSE_X10: u32 = 1 << 0;
const NDTERM_MOUSE_NORMAL: u32 = 1 << 1;
const NDTERM_MOUSE_BUTTON: u32 = 1 << 2;
const NDTERM_MOUSE_ANY: u32 = 1 << 3;
const NDTERM_MOUSE_UTF8: u32 = 1 << 4;
const NDTERM_MOUSE_SGR: u32 = 1 << 5;
const NDTERM_MOUSE_URXVT: u32 = 1 << 6;
const NDTERM_MOUSE_SGR_PIXELS: u32 = 1 << 7;

/// include/ndterm.h `NDTERM_PALETTE_COLORS` — palette_rgb must carry exactly
/// this many packed r,g,b triples.
const PALETTE_COLORS: usize = 256;

/// Client VT scrollback cap, both pty-backed and virtual (remote) terminals —
/// toward the daemon's 10k (was 1000).
const MAX_SCROLLBACK: usize = 10000;

// terminal.h `GhosttyTerminalData` values used here.
const TDATA_TITLE: c_int = 12;

// render.h `GhosttyRenderStateData` values used here.
const RDATA_COLS: c_int = 1;
const RDATA_ROWS: c_int = 2;
const RDATA_ROW_ITERATOR: c_int = 4;
const RDATA_CURSOR_VISUAL_STYLE: c_int = 10;
const RDATA_CURSOR_VISIBLE: c_int = 11;
const RDATA_CURSOR_VIEWPORT_HAS_VALUE: c_int = 14;
const RDATA_CURSOR_VIEWPORT_X: c_int = 15;
const RDATA_CURSOR_VIEWPORT_Y: c_int = 16;

// render.h `GhosttyRenderStateRowData` values used here.
const ROW_DATA_CELLS: c_int = 3;

// render.h `GhosttyRenderStateRowCellsData` values used here.
const CELLS_DATA_RAW: c_int = 1;
const CELLS_DATA_STYLE: c_int = 2;
const CELLS_DATA_BG_COLOR: c_int = 5;
const CELLS_DATA_FG_COLOR: c_int = 6;
const CELLS_DATA_GRAPHEMES_UTF8: c_int = 9;

// screen.h `GhosttyCellData` / `GhosttyCellWide` values used here.
const CELL_DATA_WIDE: c_int = 3;
const CELL_WIDE_WIDE: c_int = 1;
const CELL_WIDE_SPACER_TAIL: c_int = 2;

// ---------------------------------------------------------------------------
// libghostty-vt extern "c" decls (exact signatures from the vendored headers).
// ---------------------------------------------------------------------------

// terminal.h
extern "c" fn ghostty_terminal_new(allocator: ?*const anyopaque, terminal: *?*anyopaque, options: GhosttyTerminalOptions) c_int;
extern "c" fn ghostty_terminal_free(terminal: ?*anyopaque) void;
extern "c" fn ghostty_terminal_resize(terminal: ?*anyopaque, cols: u16, rows: u16, cell_width_px: u32, cell_height_px: u32) c_int;
extern "c" fn ghostty_terminal_set(terminal: ?*anyopaque, option: c_int, value: ?*const anyopaque) c_int;
extern "c" fn ghostty_terminal_get(terminal: ?*anyopaque, data: c_int, out: *anyopaque) c_int;
extern "c" fn ghostty_terminal_vt_write(terminal: ?*anyopaque, data: [*]const u8, len: usize) void;
extern "c" fn ghostty_terminal_scroll_viewport(terminal: ?*anyopaque, behavior: GhosttyTerminalScrollViewport) void;
extern "c" fn ghostty_terminal_mode_get(terminal: ?*anyopaque, mode: u16, out_value: *bool) c_int;

// screen.h — GhosttyCell is a uint64_t passed by value.
extern "c" fn ghostty_cell_get(cell: u64, data: c_int, out: *anyopaque) c_int;

// render.h
extern "c" fn ghostty_render_state_new(allocator: ?*const anyopaque, state: *?*anyopaque) c_int;
extern "c" fn ghostty_render_state_free(state: ?*anyopaque) void;
extern "c" fn ghostty_render_state_update(state: ?*anyopaque, terminal: ?*anyopaque) c_int;
extern "c" fn ghostty_render_state_get(state: ?*anyopaque, data: c_int, out: *anyopaque) c_int;
extern "c" fn ghostty_render_state_colors_get(state: ?*anyopaque, out_colors: *GhosttyRenderStateColors) c_int;
extern "c" fn ghostty_render_state_row_iterator_new(allocator: ?*const anyopaque, out_iterator: *?*anyopaque) c_int;
extern "c" fn ghostty_render_state_row_iterator_free(iterator: ?*anyopaque) void;
extern "c" fn ghostty_render_state_row_iterator_next(iterator: ?*anyopaque) bool;
extern "c" fn ghostty_render_state_row_get(iterator: ?*anyopaque, data: c_int, out: *anyopaque) c_int;
extern "c" fn ghostty_render_state_row_cells_new(allocator: ?*const anyopaque, out_cells: *?*anyopaque) c_int;
extern "c" fn ghostty_render_state_row_cells_free(cells: ?*anyopaque) void;
extern "c" fn ghostty_render_state_row_cells_select(cells: ?*anyopaque, x: u16) c_int;
extern "c" fn ghostty_render_state_row_cells_get(cells: ?*anyopaque, data: c_int, out: *anyopaque) c_int;

// ---------------------------------------------------------------------------
// libc / PTY extern "c" decls.
// ---------------------------------------------------------------------------

/// <sys/ioctl.h> struct winsize.
const winsize = extern struct {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
};

extern "c" fn forkpty(amaster: *c_int, name: ?[*]u8, termp: ?*const anyopaque, winp: ?*const winsize) c_int;
extern "c" fn ioctl(fd: c_int, request: c_ulong, ...) c_int;
extern "c" fn setenv(name: [*:0]const u8, value: [*:0]const u8, overwrite: c_int) c_int;
extern "c" fn chdir(path: [*:0]const u8) c_int;
extern "c" fn getenv(name: [*:0]const u8) ?[*:0]u8;
extern "c" fn execvp(file: [*:0]const u8, argv: [*:null]const ?[*:0]const u8) c_int;
extern "c" fn _exit(code: c_int) noreturn;
extern "c" fn kill(pid: c_int, sig: c_int) c_int;
extern "c" fn waitpid(pid: c_int, status: *c_int, options: c_int) c_int;
extern "c" fn read(fd: c_int, buf: [*]u8, nbyte: usize) isize;
extern "c" fn write(fd: c_int, buf: [*]const u8, nbyte: usize) isize;
extern "c" fn close(fd: c_int) c_int;

/// TIOCSWINSZ differs per platform (the two build targets are macOS + Linux).
const TIOCSWINSZ: c_ulong = switch (builtin.os.tag) {
    .linux => 0x5414,
    .macos, .ios, .tvos, .watchos => 0x80087467,
    else => @compileError("unsupported OS: no TIOCSWINSZ value"),
};

// ---------------------------------------------------------------------------
// Terminal instance (opaque `nd_terminal*` to callers).
// ---------------------------------------------------------------------------

const Terminal = struct {
    // Plain blocking mutex via libc pthread — this GTK-free core has no `std.Io`
    // (the only std Mutex in Zig 0.16 needs one), and its critical sections run
    // on raw OS threads (the reader thread + the surface's UI thread). The `.{}`
    // default is a valid static initializer on both targets (macOS sets the
    // required signature; Linux is all-zero), so no pthread_mutex_init is needed.
    mutex: std.c.pthread_mutex_t = .{},

    amaster: c_int,
    pid: c_int,

    // Virtual mode: no pty/fork/reader-thread (amaster == -1). Output bytes
    // (keystrokes + VT query responses) flow to `output_cb` instead of a PTY
    // master; input arrives via ndterm_feed. `cols`/`rows` track the logical
    // grid so ndterm_reset can rebuild the VT at the current size.
    is_virtual: bool = false,
    output_cb: ?OutputCb = null,
    cols: u16 = 0,
    rows: u16 = 0,

    term: ?*anyopaque = null, // GhosttyTerminal
    state: ?*anyopaque = null, // GhosttyRenderState
    row_iter: ?*anyopaque = null, // GhosttyRenderStateRowIterator
    row_cells: ?*anyopaque = null, // GhosttyRenderStateRowCells

    cb: ?EffectCb,
    userdata: ?*anyopaque,

    command_owned: ?[:0]u8,
    cwd_owned: ?[:0]u8,

    reader_thread: std.Thread = undefined,
    closing: std.atomic.Value(bool) = std.atomic.Value(bool).init(false),
    child_reaped: bool = false,

    snapshot: []nd_term_cell = &.{},
    snap_cols: u16 = 0,
    snap_rows: u16 = 0,
    cursor: nd_term_cursor = .{ .x = 0, .y = 0, .visible = 0, .style = 1 },
    default_fg: [3]u8 = .{ 0xcc, 0xcc, 0xcc },
    default_bg: [3]u8 = .{ 0x00, 0x00, 0x00 },
    // Open-time OPT_COLOR_PALETTE override (nd_term_open_opts.palette_rgb), if
    // any. Re-applied by registerVtOptions on every ndterm_reset, same as
    // default_fg/default_bg.
    palette: ?[256]GhosttyColorRgb = null,
};

fn lockMutex(t: *Terminal) void {
    _ = std.c.pthread_mutex_lock(&t.mutex);
}

fn unlockMutex(t: *Terminal) void {
    _ = std.c.pthread_mutex_unlock(&t.mutex);
}

// ---------------------------------------------------------------------------
// libghostty-vt effect callbacks (invoked on the reader thread under `mutex`
// during ghostty_terminal_vt_write). userdata is the *Terminal (set via
// OPT_USERDATA); the surface's own userdata is `t.userdata`.
// ---------------------------------------------------------------------------

fn writePtyCb(term: ?*anyopaque, userdata: ?*anyopaque, data: [*]const u8, len: usize) callconv(.c) void {
    _ = term;
    const t: *Terminal = @ptrCast(@alignCast(userdata orelse return));
    // Already under `mutex` (inside vt_write). Write query responses straight
    // back to the PTY; do NOT re-lock (non-recursive mutex) or re-enter vt_write.
    // In virtual mode there is no PTY — route responses out through output_cb
    // (the transport sends them as an INPUT frame; different lock, no deadlock).
    if (t.is_virtual) {
        if (t.output_cb) |o| o(t.userdata, data, len);
    } else {
        _ = write(t.amaster, data, len);
    }
}

fn titleChangedCb(term: ?*anyopaque, userdata: ?*anyopaque) callconv(.c) void {
    const t: *Terminal = @ptrCast(@alignCast(userdata orelse return));
    const cb = t.cb orelse return;
    var s: GhosttyString = .{ .ptr = null, .len = 0 };
    _ = ghostty_terminal_get(term, TDATA_TITLE, &s);
    var buf: [512]u8 = undefined;
    const n = @min(s.len, buf.len - 1);
    if (n > 0) {
        if (s.ptr) |p| @memcpy(buf[0..n], p[0..n]);
    }
    buf[n] = 0;
    cb(t.userdata, 0, @ptrCast(&buf), 0);
}

fn bellCb(term: ?*anyopaque, userdata: ?*anyopaque) callconv(.c) void {
    _ = term;
    const t: *Terminal = @ptrCast(@alignCast(userdata orelse return));
    const cb = t.cb orelse return;
    cb(t.userdata, 1, null, 0);
}

// ---------------------------------------------------------------------------
// Child process setup (runs in the forkpty child; never returns).
// ---------------------------------------------------------------------------

fn childExec(command: ?[:0]const u8, cwd: ?[:0]const u8) noreturn {
    _ = setenv("TERM", "xterm-256color", 1);
    if (cwd) |c| _ = chdir(c.ptr);
    const shell: [*:0]const u8 = if (command) |c|
        c.ptr
    else if (getenv("SHELL")) |s|
        s
    else
        "/bin/sh";
    var argv = [_:null]?[*:0]const u8{shell};
    _ = execvp(shell, &argv);
    _exit(127);
}

// ---------------------------------------------------------------------------
// Reader thread.
// ---------------------------------------------------------------------------

fn readerLoop(t: *Terminal) void {
    var buf: [65536]u8 = undefined;
    while (true) {
        const rc = read(t.amaster, &buf, buf.len);
        if (rc <= 0) break; // EOF or error: the child (or slave side) is gone.
        const n: usize = @intCast(rc);
        lockMutex(t);
        ghostty_terminal_vt_write(t.term, &buf, n);
        unlockMutex(t);
    }

    // The PTY hit EOF. If this is a genuine child exit (not a teardown from
    // ndterm_close), reap the child and report the exit code. Reaping is guarded
    // by `child_reaped` under `mutex` so ndterm_close never double-reaps.
    lockMutex(t);
    var report_code: ?c_int = null;
    if (!t.closing.load(.seq_cst) and !t.child_reaped) {
        var status: c_int = 0;
        _ = waitpid(t.pid, &status, 0);
        t.child_reaped = true;
        report_code = exitStatusToCode(status);
    }
    unlockMutex(t);

    if (report_code) |code| {
        if (t.cb) |cb| cb(t.userdata, 2, null, code);
    }
}

fn exitStatusToCode(status: c_int) c_int {
    // WIFEXITED: low 7 bits zero -> normal exit, code = bits 8..15.
    if ((status & 0x7f) == 0) return (status >> 8) & 0xff;
    // Otherwise terminated by signal N -> conventional 128+N.
    return 128 + (status & 0x7f);
}

// ---------------------------------------------------------------------------
// ndterm.h exports.
// ---------------------------------------------------------------------------

pub export fn ndterm_open(
    cols: u16,
    rows: u16,
    command: ?[*:0]const u8,
    cwd: ?[*:0]const u8,
    cb: ?EffectCb,
    userdata: ?*anyopaque,
) callconv(.c) ?*Terminal {
    return openImpl(cols, rows, command, cwd, null, cb, userdata);
}

/// WP polish-1 deliverable 7: same as ndterm_open, plus an optional open-time
/// default fg/bg + 256-color palette (see nd_term_open_opts). `opts == null`
/// behaves exactly like ndterm_open.
pub export fn ndterm_open_ex(
    cols: u16,
    rows: u16,
    command: ?[*:0]const u8,
    cwd: ?[*:0]const u8,
    opts: ?*const nd_term_open_opts,
    cb: ?EffectCb,
    userdata: ?*anyopaque,
) callconv(.c) ?*Terminal {
    return openImpl(cols, rows, command, cwd, opts, cb, userdata);
}

fn openImpl(
    cols: u16,
    rows: u16,
    command: ?[*:0]const u8,
    cwd: ?[*:0]const u8,
    opts: ?*const nd_term_open_opts,
    cb: ?EffectCb,
    userdata: ?*anyopaque,
) ?*Terminal {
    // Copy command/cwd into owned memory before forking — callers may free the
    // originals as soon as ndterm_open returns, but the child reads them at exec.
    const command_owned: ?[:0]u8 = if (command) |c|
        (gpa.dupeZ(u8, std.mem.span(c)) catch return null)
    else
        null;
    const cwd_owned: ?[:0]u8 = if (cwd) |c|
        (gpa.dupeZ(u8, std.mem.span(c)) catch {
            if (command_owned) |co| gpa.free(co);
            return null;
        })
    else
        null;

    var ws = winsize{ .ws_row = rows, .ws_col = cols, .ws_xpixel = 0, .ws_ypixel = 0 };
    var amaster: c_int = -1;
    const pid = forkpty(&amaster, null, null, &ws);
    if (pid < 0) {
        if (command_owned) |co| gpa.free(co);
        if (cwd_owned) |cw| gpa.free(cw);
        return null;
    }
    if (pid == 0) childExec(command_owned, cwd_owned); // never returns

    // --- Parent ---
    return parentSetup(cols, rows, amaster, pid, opts, cb, userdata, command_owned, cwd_owned) catch {
        // Setup failed after fork: tear the child down and release everything.
        _ = kill(pid, sigNum(std.posix.SIG.KILL));
        var status: c_int = 0;
        _ = waitpid(pid, &status, 0);
        _ = close(amaster);
        if (command_owned) |co| gpa.free(co);
        if (cwd_owned) |cw| gpa.free(cw);
        return null;
    };
}

fn parentSetup(
    cols: u16,
    rows: u16,
    amaster: c_int,
    pid: c_int,
    opts: ?*const nd_term_open_opts,
    cb: ?EffectCb,
    userdata: ?*anyopaque,
    command_owned: ?[:0]u8,
    cwd_owned: ?[:0]u8,
) !*Terminal {
    const t = try gpa.create(Terminal);
    errdefer gpa.destroy(t);
    t.* = .{
        .amaster = amaster,
        .pid = pid,
        .cb = cb,
        .userdata = userdata,
        .command_owned = command_owned,
        .cwd_owned = cwd_owned,
    };
    applyOpenOpts(t, opts);

    try setupVt(t, cols, rows);
    errdefer freeVt(t);

    t.reader_thread = try std.Thread.spawn(.{}, readerLoop, .{t});
    return t;
}

/// Register the 4 effect options + default colors on `t.term`. Shared by
/// setupVt and ndterm_reset (which rebuilds the VT). userdata is the
/// *Terminal so callbacks reach t.cb/t.userdata.
fn registerVtOptions(t: *Terminal) void {
    const def_fg = GhosttyColorRgb{ .r = t.default_fg[0], .g = t.default_fg[1], .b = t.default_fg[2] };
    const def_bg = GhosttyColorRgb{ .r = t.default_bg[0], .g = t.default_bg[1], .b = t.default_bg[2] };
    _ = ghostty_terminal_set(t.term, OPT_COLOR_FOREGROUND, @ptrCast(&def_fg));
    _ = ghostty_terminal_set(t.term, OPT_COLOR_BACKGROUND, @ptrCast(&def_bg));
    if (t.palette) |*p| _ = ghostty_terminal_set(t.term, OPT_COLOR_PALETTE, @ptrCast(p));
    _ = ghostty_terminal_set(t.term, OPT_USERDATA, @ptrCast(t));
    _ = ghostty_terminal_set(t.term, OPT_WRITE_PTY, @ptrCast(&writePtyCb));
    _ = ghostty_terminal_set(t.term, OPT_TITLE_CHANGED, @ptrCast(&titleChangedCb));
    _ = ghostty_terminal_set(t.term, OPT_BELL, @ptrCast(&bellCb));
}

/// Apply an optional nd_term_open_opts to a freshly-created `t`, before
/// setupVt registers the VT options. Shared by ndterm_open_ex,
/// ndterm_open_virtual_ex (and transitively ndrt_open_ex). A null `opts`
/// (or a malformed palette_len) leaves the struct's own defaults untouched.
fn applyOpenOpts(t: *Terminal, opts_opt: ?*const nd_term_open_opts) void {
    const opts = opts_opt orelse return;
    if (opts.has_fg != 0) t.default_fg = opts.fg;
    if (opts.has_bg != 0) t.default_bg = opts.bg;
    if (opts.palette_rgb) |bytes| {
        if (opts.palette_len == PALETTE_COLORS * 3) {
            var pal: [PALETTE_COLORS]GhosttyColorRgb = undefined;
            for (&pal, 0..) |*c, i| {
                c.* = .{ .r = bytes[i * 3], .g = bytes[i * 3 + 1], .b = bytes[i * 3 + 2] };
            }
            t.palette = pal;
        }
    }
}

/// Build the libghostty-vt terminal + render-state/iterator/cells on `t`.
/// Shared by the pty-backed (ndterm_open) and virtual (ndterm_open_virtual)
/// paths — neither touches the PTY. Frees any partial allocation on error.
fn setupVt(t: *Terminal, cols: u16, rows: u16) !void {
    t.cols = cols;
    t.rows = rows;

    var term: ?*anyopaque = null;
    const opts = GhosttyTerminalOptions{ .cols = cols, .rows = rows, .max_scrollback = MAX_SCROLLBACK };
    if (ghostty_terminal_new(null, &term, opts) != GHOSTTY_SUCCESS) return error.TerminalNew;
    t.term = term;
    errdefer {
        ghostty_terminal_free(t.term);
        t.term = null;
    }

    registerVtOptions(t);

    var state: ?*anyopaque = null;
    if (ghostty_render_state_new(null, &state) != GHOSTTY_SUCCESS) return error.RenderStateNew;
    t.state = state;
    errdefer {
        ghostty_render_state_free(t.state);
        t.state = null;
    }

    var iter: ?*anyopaque = null;
    if (ghostty_render_state_row_iterator_new(null, &iter) != GHOSTTY_SUCCESS) return error.RowIterNew;
    t.row_iter = iter;
    errdefer {
        ghostty_render_state_row_iterator_free(t.row_iter);
        t.row_iter = null;
    }

    var cells: ?*anyopaque = null;
    if (ghostty_render_state_row_cells_new(null, &cells) != GHOSTTY_SUCCESS) return error.RowCellsNew;
    t.row_cells = cells;
}

/// Free the libghostty-vt render objects + terminal (reverse of setupVt).
fn freeVt(t: *Terminal) void {
    ghostty_render_state_row_cells_free(t.row_cells);
    ghostty_render_state_row_iterator_free(t.row_iter);
    ghostty_render_state_free(t.state);
    ghostty_terminal_free(t.term);
}

pub export fn ndterm_close(t_opt: ?*Terminal) callconv(.c) void {
    const t = t_opt orelse return;

    // Virtual mode has no reader thread / child / PTY master (amaster == -1),
    // so skip all of the pty teardown — just free the VT + owned memory.
    if (!t.is_virtual) {
        // Signal teardown, then wake a reader blocked in read() by hanging up the
        // child (EOF on the PTY master). The reader sees `closing` and will not reap.
        t.closing.store(true, .seq_cst);
        lockMutex(t);
        if (!t.child_reaped) _ = kill(t.pid, sigNum(std.posix.SIG.HUP));
        unlockMutex(t);

        t.reader_thread.join();

        // Reader has exited; reap the child if it is still alive.
        lockMutex(t);
        if (!t.child_reaped) {
            _ = kill(t.pid, sigNum(std.posix.SIG.KILL));
            var status: c_int = 0;
            _ = waitpid(t.pid, &status, 0);
            t.child_reaped = true;
        }
        unlockMutex(t);

        _ = close(t.amaster);
    }

    freeVt(t);

    if (t.snapshot.len != 0) gpa.free(t.snapshot);
    if (t.command_owned) |c| gpa.free(c);
    if (t.cwd_owned) |c| gpa.free(c);
    gpa.destroy(t);
}

pub export fn ndterm_open_virtual(
    cols: u16,
    rows: u16,
    cb: ?EffectCb,
    output_cb: ?OutputCb,
    userdata: ?*anyopaque,
) callconv(.c) ?*Terminal {
    return openVirtualImpl(cols, rows, null, cb, output_cb, userdata);
}

/// WP polish-1 deliverable 7: same as ndterm_open_virtual, plus an optional
/// open-time default fg/bg + 256-color palette (see nd_term_open_opts).
/// `opts == null` behaves exactly like ndterm_open_virtual. Used by
/// ndrt_open_ex (src/core/remote_terminal.zig) for remote sessions.
pub export fn ndterm_open_virtual_ex(
    cols: u16,
    rows: u16,
    opts: ?*const nd_term_open_opts,
    cb: ?EffectCb,
    output_cb: ?OutputCb,
    userdata: ?*anyopaque,
) callconv(.c) ?*Terminal {
    return openVirtualImpl(cols, rows, opts, cb, output_cb, userdata);
}

fn openVirtualImpl(
    cols: u16,
    rows: u16,
    opts: ?*const nd_term_open_opts,
    cb: ?EffectCb,
    output_cb: ?OutputCb,
    userdata: ?*anyopaque,
) ?*Terminal {
    const t = gpa.create(Terminal) catch return null;
    t.* = .{
        .amaster = -1,
        .pid = -1,
        .is_virtual = true,
        .output_cb = output_cb,
        .cb = cb,
        .userdata = userdata,
        .command_owned = null,
        .cwd_owned = null,
    };
    applyOpenOpts(t, opts);
    setupVt(t, cols, rows) catch {
        gpa.destroy(t);
        return null;
    };
    return t;
}

pub export fn ndterm_feed(t_opt: ?*Terminal, bytes: [*]const u8, len: usize) callconv(.c) void {
    const t = t_opt orelse return;
    if (!t.is_virtual) return; // no-op on pty-backed terminals
    lockMutex(t);
    defer unlockMutex(t);
    ghostty_terminal_vt_write(t.term, bytes, len);
}

pub export fn ndterm_reset(t_opt: ?*Terminal) callconv(.c) void {
    const t = t_opt orelse return;
    if (!t.is_virtual) return;
    lockMutex(t);
    defer unlockMutex(t);
    // Rebuild the VT for a true power-on state: RIS (ESC c) alone would not
    // guarantee scrollback / alt-screen / DEC-mode reset to match a server
    // snapshot. free + new + re-register the effect options at the current size.
    ghostty_terminal_free(t.term);
    var term: ?*anyopaque = null;
    const opts = GhosttyTerminalOptions{ .cols = t.cols, .rows = t.rows, .max_scrollback = MAX_SCROLLBACK };
    if (ghostty_terminal_new(null, &term, opts) != GHOSTTY_SUCCESS) {
        t.term = null;
        return;
    }
    t.term = term;
    registerVtOptions(t);
}

pub export fn ndterm_resize(t_opt: ?*Terminal, cols: u16, rows: u16) callconv(.c) void {
    const t = t_opt orelse return;
    lockMutex(t);
    defer unlockMutex(t);
    _ = ghostty_terminal_resize(t.term, cols, rows, 0, 0);
    t.cols = cols;
    t.rows = rows;
    // Virtual mode has no PTY master to notify — the remote server owns the
    // real PTY winsize (driven by RESIZE frames from the transport).
    if (!t.is_virtual) {
        var ws = winsize{ .ws_row = rows, .ws_col = cols, .ws_xpixel = 0, .ws_ypixel = 0 };
        _ = ioctl(t.amaster, TIOCSWINSZ, &ws);
    }
}

/// WP polish-1 deliverable 6a: move the client-local scrollback viewport by
/// `delta` rows (negative = back into scrollback, positive = toward the live
/// output) and let the backend re-render via the normal
/// render_lock/cell/render_unlock path. Wraps libghostty-vt's
/// ghostty_terminal_scroll_viewport (terminal.h:1045).
pub export fn ndterm_scroll_viewport(t_opt: ?*Terminal, delta: c_int) callconv(.c) void {
    const t = t_opt orelse return;
    lockMutex(t);
    defer unlockMutex(t);
    const behavior = GhosttyTerminalScrollViewport{
        .tag = SCROLL_VIEWPORT_DELTA,
        .value = .{ .delta = delta },
    };
    ghostty_terminal_scroll_viewport(t.term, behavior);
}

/// WP polish-1 deliverable 6b: bitmask of the VT's currently-active mouse
/// reporting modes (NDTERM_MOUSE_* in include/ndterm.h), so a backend can gate
/// SGR mouse-event encoding on whether — and in which format — the app
/// enabled reporting. Returns 0 for a NULL terminal or when no mouse mode set.
pub export fn ndterm_mouse_mode(t_opt: ?*Terminal) callconv(.c) u32 {
    const t = t_opt orelse return 0;
    lockMutex(t);
    defer unlockMutex(t);
    var mask: u32 = 0;
    mask |= modeBit(t, MODE_X10_MOUSE, NDTERM_MOUSE_X10);
    mask |= modeBit(t, MODE_NORMAL_MOUSE, NDTERM_MOUSE_NORMAL);
    mask |= modeBit(t, MODE_BUTTON_MOUSE, NDTERM_MOUSE_BUTTON);
    mask |= modeBit(t, MODE_ANY_MOUSE, NDTERM_MOUSE_ANY);
    mask |= modeBit(t, MODE_UTF8_MOUSE, NDTERM_MOUSE_UTF8);
    mask |= modeBit(t, MODE_SGR_MOUSE, NDTERM_MOUSE_SGR);
    mask |= modeBit(t, MODE_URXVT_MOUSE, NDTERM_MOUSE_URXVT);
    mask |= modeBit(t, MODE_SGR_PIXELS_MOUSE, NDTERM_MOUSE_SGR_PIXELS);
    return mask;
}

/// `t.mutex` must already be held (called only from ndterm_mouse_mode).
fn modeBit(t: *Terminal, mode: u16, bit: u32) u32 {
    var v: bool = false;
    if (ghostty_terminal_mode_get(t.term, mode, &v) == GHOSTTY_SUCCESS and v) return bit;
    return 0;
}

pub export fn ndterm_write_input(t_opt: ?*Terminal, bytes: [*]const u8, len: usize) callconv(.c) void {
    const t = t_opt orelse return;
    lockMutex(t);
    defer unlockMutex(t);
    // Virtual mode: keystrokes leave through output_cb (the transport wraps
    // them in an INPUT frame) instead of a local PTY master.
    if (t.is_virtual) {
        if (t.output_cb) |o| o(t.userdata, bytes, len);
    } else {
        _ = write(t.amaster, bytes, len);
    }
}

pub export fn ndterm_render_lock(t_opt: ?*Terminal, out_cols: *u16, out_rows: *u16) callconv(.c) void {
    const t = t_opt orelse {
        out_cols.* = 0;
        out_rows.* = 0;
        return;
    };

    // Mutex intentionally acquired here and held until ndterm_render_unlock.
    lockMutex(t);

    _ = ghostty_render_state_update(t.state, t.term);

    var cols: u16 = 0;
    var rows: u16 = 0;
    _ = ghostty_render_state_get(t.state, RDATA_COLS, &cols);
    _ = ghostty_render_state_get(t.state, RDATA_ROWS, &rows);

    // Default fg/bg (used as the per-cell fallback when a cell has none).
    var colors: GhosttyRenderStateColors = std.mem.zeroes(GhosttyRenderStateColors);
    colors.size = @sizeOf(GhosttyRenderStateColors);
    if (ghostty_render_state_colors_get(t.state, &colors) == GHOSTTY_SUCCESS) {
        t.default_fg = .{ colors.foreground.r, colors.foreground.g, colors.foreground.b };
        t.default_bg = .{ colors.background.r, colors.background.g, colors.background.b };
    }

    const count: usize = @as(usize, cols) * @as(usize, rows);
    t.snap_cols = cols;
    t.snap_rows = rows;
    if (t.snapshot.len != count) {
        if (count == 0) {
            if (t.snapshot.len != 0) gpa.free(t.snapshot);
            t.snapshot = &.{};
        } else if (gpa.realloc(t.snapshot, count)) |buf| {
            t.snapshot = buf;
        } else |_| {
            // OOM: drop the buffer; cell reads degrade to blank via bounds check.
            if (t.snapshot.len != 0) gpa.free(t.snapshot);
            t.snapshot = &.{};
            t.snap_cols = 0;
            t.snap_rows = 0;
            snapshotCursor(t);
            out_cols.* = cols;
            out_rows.* = rows;
            return;
        }
    }

    const blank = blankCell(t.default_fg, t.default_bg);
    for (t.snapshot) |*c| c.* = blank;

    _ = ghostty_render_state_get(t.state, RDATA_ROW_ITERATOR, @ptrCast(&t.row_iter));
    var y: u16 = 0;
    while (ghostty_render_state_row_iterator_next(t.row_iter)) {
        if (y >= rows) break;
        defer y += 1;
        if (ghostty_render_state_row_get(t.row_iter, ROW_DATA_CELLS, @ptrCast(&t.row_cells)) != GHOSTTY_SUCCESS) continue;

        var x: u16 = 0;
        while (x < cols) : (x += 1) {
            if (ghostty_render_state_row_cells_select(t.row_cells, x) != GHOSTTY_SUCCESS) continue;
            t.snapshot[@as(usize, y) * @as(usize, cols) + x] = resolveCell(t, blank);
        }
    }

    snapshotCursor(t);
    out_cols.* = cols;
    out_rows.* = rows;
}

/// Resolve the currently-selected cell of `t.row_cells` into an nd_term_cell.
/// `blank` carries the resolved default fg/bg used when the cell has none.
fn resolveCell(t: *Terminal, blank: nd_term_cell) nd_term_cell {
    var cell = blank;

    // Grapheme UTF-8 (base codepoint + any combining marks), NUL-terminated.
    var textbuf: [64]u8 = undefined;
    var gb = GhosttyBuffer{ .ptr = &textbuf, .cap = textbuf.len, .len = 0 };
    cell.utf8 = std.mem.zeroes([16]u8);
    if (ghostty_render_state_row_cells_get(t.row_cells, CELLS_DATA_GRAPHEMES_UTF8, &gb) == GHOSTTY_SUCCESS) {
        const n = @min(gb.len, cell.utf8.len - 1);
        if (n > 0) @memcpy(cell.utf8[0..n], textbuf[0..n]);
        cell.utf8[n] = 0;
    }

    // Resolved bg/fg — INVALID_VALUE (non-success) means "use the default".
    var bg: GhosttyColorRgb = undefined;
    if (ghostty_render_state_row_cells_get(t.row_cells, CELLS_DATA_BG_COLOR, &bg) == GHOSTTY_SUCCESS) {
        cell.bg = .{ bg.r, bg.g, bg.b };
    } else {
        cell.bg = t.default_bg;
    }
    var fg: GhosttyColorRgb = undefined;
    if (ghostty_render_state_row_cells_get(t.row_cells, CELLS_DATA_FG_COLOR, &fg) == GHOSTTY_SUCCESS) {
        cell.fg = .{ fg.r, fg.g, fg.b };
    } else {
        cell.fg = t.default_fg;
    }

    var flags: u8 = 0;
    var style: GhosttyStyle = std.mem.zeroes(GhosttyStyle);
    style.size = @sizeOf(GhosttyStyle);
    if (ghostty_render_state_row_cells_get(t.row_cells, CELLS_DATA_STYLE, &style) == GHOSTTY_SUCCESS) {
        if (style.bold) flags |= NDTERM_FLAG_BOLD;
        if (style.underline != 0) flags |= NDTERM_FLAG_UNDERLINE;
        if (style.inverse) flags |= NDTERM_FLAG_INVERSE;
    }

    // Wide property lives on the raw cell (uint64_t) -> ghostty_cell_get(WIDE).
    var raw: u64 = 0;
    if (ghostty_render_state_row_cells_get(t.row_cells, CELLS_DATA_RAW, &raw) == GHOSTTY_SUCCESS) {
        var wide: c_int = 0;
        if (ghostty_cell_get(raw, CELL_DATA_WIDE, &wide) == GHOSTTY_SUCCESS) {
            if (wide == CELL_WIDE_WIDE) {
                flags |= NDTERM_FLAG_WIDE;
            } else if (wide == CELL_WIDE_SPACER_TAIL) {
                flags |= NDTERM_FLAG_WIDE_TAIL;
            }
        }
    }

    cell.flags = flags;
    return cell;
}

fn snapshotCursor(t: *Terminal) void {
    var cur = nd_term_cursor{ .x = 0, .y = 0, .visible = 0, .style = 1 };

    var has_vp: bool = false;
    _ = ghostty_render_state_get(t.state, RDATA_CURSOR_VIEWPORT_HAS_VALUE, &has_vp);
    var vis: bool = false;
    _ = ghostty_render_state_get(t.state, RDATA_CURSOR_VISIBLE, &vis);

    if (has_vp) {
        var cx: u16 = 0;
        var cy: u16 = 0;
        _ = ghostty_render_state_get(t.state, RDATA_CURSOR_VIEWPORT_X, &cx);
        _ = ghostty_render_state_get(t.state, RDATA_CURSOR_VIEWPORT_Y, &cy);
        cur.x = cx;
        cur.y = cy;
    }

    var vstyle: c_int = 1;
    _ = ghostty_render_state_get(t.state, RDATA_CURSOR_VISUAL_STYLE, &vstyle);
    cur.style = @intCast(vstyle & 0xff);
    cur.visible = if (vis and has_vp) 1 else 0;

    t.cursor = cur;
}

fn blankCell(fg: [3]u8, bg: [3]u8) nd_term_cell {
    return .{
        .utf8 = std.mem.zeroes([16]u8),
        .fg = fg,
        .bg = bg,
        .flags = 0,
    };
}

pub export fn ndterm_cell(t_opt: ?*Terminal, x: u16, y: u16, out: *nd_term_cell) callconv(.c) void {
    const t = t_opt orelse {
        out.* = blankCell(.{ 0xcc, 0xcc, 0xcc }, .{ 0, 0, 0 });
        return;
    };
    if (x >= t.snap_cols or y >= t.snap_rows or t.snapshot.len == 0) {
        out.* = blankCell(t.default_fg, t.default_bg);
        return;
    }
    out.* = t.snapshot[@as(usize, y) * @as(usize, t.snap_cols) + x];
}

pub export fn ndterm_cursor(t_opt: ?*Terminal, out: *nd_term_cursor) callconv(.c) void {
    const t = t_opt orelse {
        out.* = .{ .x = 0, .y = 0, .visible = 0, .style = 1 };
        return;
    };
    out.* = t.cursor;
}

pub export fn ndterm_default_colors(t_opt: ?*Terminal, fg: *[3]u8, bg: *[3]u8) callconv(.c) void {
    const t = t_opt orelse {
        fg.* = .{ 0xcc, 0xcc, 0xcc };
        bg.* = .{ 0, 0, 0 };
        return;
    };
    fg.* = t.default_fg;
    bg.* = t.default_bg;
}

pub export fn ndterm_render_unlock(t_opt: ?*Terminal) callconv(.c) void {
    const t = t_opt orelse return;
    unlockMutex(t);
}

/// std.posix.SIG.* are an enum(u32) in Zig 0.16; kill() wants a c_int.
inline fn sigNum(sig: anytype) c_int {
    return @intCast(@intFromEnum(sig));
}

// ---------------------------------------------------------------------------
// Virtual-mode unit tests (WP6). Exercise ndterm_open_virtual/feed/reset with a
// real libghostty-vt instance and no PTY. Framing tests live in
// remote_terminal.zig. Both are wired as their own addTest roots in build.zig.
// ---------------------------------------------------------------------------

var t_output: std.ArrayListUnmanaged(u8) = .empty;
var t_effect_kind: c_int = -1;
var t_effect_text: [256]u8 = undefined;
var t_effect_len: usize = 0;

fn testOutputCb(_: ?*anyopaque, bytes: [*]const u8, len: usize) callconv(.c) void {
    t_output.appendSlice(std.testing.allocator, bytes[0..len]) catch {};
}

fn testEffectCb(_: ?*anyopaque, kind: c_int, text: ?[*:0]const u8, code: c_int) callconv(.c) void {
    _ = code;
    t_effect_kind = kind;
    if (text) |tx| {
        const s = std.mem.span(tx);
        t_effect_len = @min(s.len, t_effect_text.len);
        @memcpy(t_effect_text[0..t_effect_len], s[0..t_effect_len]);
    }
}

fn cellChar(t: *Terminal, x: u16, y: u16) u8 {
    var cols: u16 = 0;
    var rows: u16 = 0;
    ndterm_render_lock(t, &cols, &rows);
    defer ndterm_render_unlock(t);
    var cell: nd_term_cell = undefined;
    ndterm_cell(t, x, y, &cell);
    return cell.utf8[0];
}

test "virtual mode: feed renders into the grid; input routes out with no echo" {
    t_output = .empty;
    defer t_output.deinit(std.testing.allocator);
    const t = ndterm_open_virtual(20, 5, testEffectCb, testOutputCb, null) orelse return error.OpenFailed;
    defer ndterm_close(t);

    ndterm_feed(t, "hi", 2);
    try std.testing.expectEqual(@as(u8, 'h'), cellChar(t, 0, 0));
    try std.testing.expectEqual(@as(u8, 'i'), cellChar(t, 1, 0));

    // Keystrokes leave via output_cb and are NOT echoed into the local grid.
    ndterm_write_input(t, "x", 1);
    try std.testing.expectEqualSlices(u8, "x", t_output.items);
    try std.testing.expectEqual(@as(u8, 0), cellChar(t, 2, 0));
}

test "virtual mode: OSC-2 title fires the effect callback" {
    t_output = .empty;
    defer t_output.deinit(std.testing.allocator);
    t_effect_kind = -1;
    t_effect_len = 0;
    const t = ndterm_open_virtual(20, 5, testEffectCb, testOutputCb, null) orelse return error.OpenFailed;
    defer ndterm_close(t);

    const osc = "\x1b]2;hello-title\x07";
    ndterm_feed(t, osc, osc.len);
    try std.testing.expectEqual(@as(c_int, 0), t_effect_kind);
    try std.testing.expectEqualStrings("hello-title", t_effect_text[0..t_effect_len]);
}

test "virtual mode: reset clears the grid" {
    t_output = .empty;
    defer t_output.deinit(std.testing.allocator);
    const t = ndterm_open_virtual(20, 5, testEffectCb, testOutputCb, null) orelse return error.OpenFailed;
    defer ndterm_close(t);

    ndterm_feed(t, "ABC", 3);
    try std.testing.expectEqual(@as(u8, 'A'), cellChar(t, 0, 0));
    ndterm_reset(t);
    try std.testing.expectEqual(@as(u8, 0), cellChar(t, 0, 0));
}

test "virtual mode: close is safe with no thread/child/fd" {
    const t = ndterm_open_virtual(10, 3, testEffectCb, testOutputCb, null) orelse return error.OpenFailed;
    ndterm_close(t); // must not join a thread, kill a pid, or close(-1)
}

test "virtual mode: mouse_mode reports DECSET state as a bitmask" {
    t_output = .empty;
    defer t_output.deinit(std.testing.allocator);
    const t = ndterm_open_virtual(10, 2, testEffectCb, testOutputCb, null) orelse return error.OpenFailed;
    defer ndterm_close(t);

    try std.testing.expectEqual(@as(u32, 0), ndterm_mouse_mode(t));

    // DECSET 1000 (normal tracking) + 1006 (SGR encoding).
    const enable = "\x1b[?1000h\x1b[?1006h";
    ndterm_feed(t, enable, enable.len);
    try std.testing.expectEqual(NDTERM_MOUSE_NORMAL | NDTERM_MOUSE_SGR, ndterm_mouse_mode(t));

    // DECRST 1000 drops normal tracking but leaves the SGR format bit.
    const disable = "\x1b[?1000l";
    ndterm_feed(t, disable, disable.len);
    try std.testing.expectEqual(NDTERM_MOUSE_SGR, ndterm_mouse_mode(t));
}

test "virtual mode: open_virtual_ex applies default fg/bg + palette at open" {
    t_output = .empty;
    defer t_output.deinit(std.testing.allocator);

    var palette: [PALETTE_COLORS * 3]u8 = @splat(0);
    palette[1 * 3 + 0] = 10;
    palette[1 * 3 + 1] = 20;
    palette[1 * 3 + 2] = 30;
    const opts = nd_term_open_opts{
        .palette_rgb = &palette,
        .palette_len = palette.len,
        .has_fg = 1,
        .fg = .{ 1, 2, 3 },
        .has_bg = 1,
        .bg = .{ 4, 5, 6 },
    };

    const t = ndterm_open_virtual_ex(10, 2, &opts, testEffectCb, testOutputCb, null) orelse return error.OpenFailed;
    defer ndterm_close(t);

    var fg: [3]u8 = undefined;
    var bg: [3]u8 = undefined;
    ndterm_default_colors(t, &fg, &bg);
    try std.testing.expectEqualSlices(u8, &.{ 1, 2, 3 }, &fg);
    try std.testing.expectEqualSlices(u8, &.{ 4, 5, 6 }, &bg);

    // SGR 38;5;1 selects palette index 1 -> our open-time override.
    const sgr = "\x1b[38;5;1mX";
    ndterm_feed(t, sgr, sgr.len);
    var cols: u16 = 0;
    var rows: u16 = 0;
    ndterm_render_lock(t, &cols, &rows);
    var cell: nd_term_cell = undefined;
    ndterm_cell(t, 0, 0, &cell);
    ndterm_render_unlock(t);
    try std.testing.expectEqualSlices(u8, &.{ 10, 20, 30 }, &cell.fg);
}

test "virtual mode: scroll_viewport moves the render window into scrollback" {
    t_output = .empty;
    defer t_output.deinit(std.testing.allocator);
    const t = ndterm_open_virtual(10, 2, testEffectCb, testOutputCb, null) orelse return error.OpenFailed;
    defer ndterm_close(t);

    // 2-row viewport: "AA" scrolls off into scrollback as "BB"/"CC" print.
    const lines = "AA\r\nBB\r\nCC";
    ndterm_feed(t, lines, lines.len);
    try std.testing.expectEqual(@as(u8, 'B'), cellChar(t, 0, 0));
    try std.testing.expectEqual(@as(u8, 'C'), cellChar(t, 0, 1));

    ndterm_scroll_viewport(t, -1);
    try std.testing.expectEqual(@as(u8, 'A'), cellChar(t, 0, 0));
    try std.testing.expectEqual(@as(u8, 'B'), cellChar(t, 0, 1));

    ndterm_scroll_viewport(t, 1);
    try std.testing.expectEqual(@as(u8, 'B'), cellChar(t, 0, 0));
    try std.testing.expectEqual(@as(u8, 'C'), cellChar(t, 0, 1));
}
