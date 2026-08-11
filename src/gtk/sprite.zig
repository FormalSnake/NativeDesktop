// Geometric sprite rendering for box-drawing, block-element, braille and
// Powerline codepoints. These are drawn as cairo geometry (not font glyphs) so
// that borders tile seamlessly and prompt separators meet the neighbouring
// background with no seam, matching how Ghostty draws them
// (src/font/sprite/draw/{box,block,braille,powerline}.zig). The stub-weight
// model for U+2500..U+257F, the eighth/quadrant model for U+2580..U+259F, the
// 2x4 braille dot grid and the Powerline triangle/chevron/half-disc geometry
// are ports of that source, adapted to cell-local cairo coordinates.
const std = @import("std");
const cairo = @import("cairo");

/// True for any codepoint `draw` can render. The caller decodes a cell's first
/// codepoint and, when this is true, calls `draw` and skips the font path.
pub fn contains(cp: u32) bool {
    return (cp >= 0x2500 and cp <= 0x257f) or
        (cp >= 0x2580 and cp <= 0x259f) or
        (cp >= 0x2800 and cp <= 0x28ff) or
        (cp >= 0xe0b0 and cp <= 0xe0b7);
}

/// Draw the sprite for `cp` into the cell rect at (px, py) sized w x h, using
/// `rgb` as the fill colour and `base_thick` as the light line weight in px.
/// Everything is drawn in a saved+clipped cairo state so straight box rules stay
/// pixel-crisp (antialias off) while diagonals/arcs/triangles antialias, and no
/// ink escapes the cell. Returns false without touching the context when `cp`
/// isn't a sprite codepoint (caller falls back to the font path).
pub fn draw(
    cr: *cairo.Context,
    cp: u32,
    px: f64,
    py: f64,
    w: f64,
    h: f64,
    base_thick: u32,
    rgb: [3]u8,
) bool {
    if (!contains(cp)) return false;

    cairo.Context.save(cr);
    defer cairo.Context.restore(cr);
    cairo.Context.translate(cr, px, py);
    cairo.Context.rectangle(cr, 0, 0, w, h);
    cairo.Context.clip(cr);

    const cv = Canvas{
        .cr = cr,
        .cw = @intFromFloat(w),
        .ch = @intFromFloat(h),
        .base = @max(base_thick, 1),
        .r = @as(f64, @floatFromInt(rgb[0])) / 255.0,
        .g = @as(f64, @floatFromInt(rgb[1])) / 255.0,
        .b = @as(f64, @floatFromInt(rgb[2])) / 255.0,
    };
    cairo.Context.setSourceRgb(cr, cv.r, cv.g, cv.b);

    switch (cp) {
        0x2500...0x257f => cv.boxDrawing(cp),
        0x2580...0x259f => cv.blockElement(cp),
        0x2800...0x28ff => cv.braille(cp),
        0xe0b0...0xe0b7 => cv.powerline(cp),
        else => unreachable,
    }
    return true;
}

/// One box-drawing stub's line style, from a cell edge to the centre.
const Style = enum(u2) { none, light, heavy, double };

/// The four stubs of a traditional box/line-drawing character. Struct layout
/// and the per-codepoint mapping mirror Ghostty's `box.Lines`.
const Lines = packed struct(u8) {
    up: Style = .none,
    right: Style = .none,
    down: Style = .none,
    left: Style = .none,
};

const Corner = enum { tl, tr, bl, br };
const HAlign = enum { left, center, right };
const VAlign = enum { top, middle, bottom };

/// Cell-local drawing surface. All coordinates are relative to the cell's
/// top-left (the caller has already translated the context), and `cw`/`ch` are
/// the integer cell box (2x width for a wide cell).
const Canvas = struct {
    cr: *cairo.Context,
    cw: u32,
    ch: u32,
    base: u32,
    r: f64,
    g: f64,
    b: f64,

    // ---- primitives ----

    /// Crisp axis-aligned fill (antialias off) so rules land on the pixel grid
    /// and adjacent cells join without a seam.
    fn boxFill(self: *const Canvas, x0: i32, y0: i32, x1: i32, y1: i32) void {
        const xa = @min(x0, x1);
        const ya = @min(y0, y1);
        const xb = @max(x0, x1);
        const yb = @max(y0, y1);
        if (xb <= xa or yb <= ya) return;
        cairo.Context.setAntialias(self.cr, .none);
        cairo.Context.rectangle(
            self.cr,
            @floatFromInt(xa),
            @floatFromInt(ya),
            @floatFromInt(xb - xa),
            @floatFromInt(yb - ya),
        );
        cairo.Context.fill(self.cr);
    }

    fn boxU(self: *const Canvas, x0: u32, y0: u32, x1: u32, y1: u32) void {
        self.boxFill(@intCast(x0), @intCast(y0), @intCast(x1), @intCast(y1));
    }

    fn hlineI(self: *const Canvas, x1: i32, x2: i32, y: i32, thick: u32) void {
        self.boxFill(x1, y, x2, y + @as(i32, @intCast(thick)));
    }

    fn vlineI(self: *const Canvas, y1: i32, y2: i32, x: i32, thick: u32) void {
        self.boxFill(x, y1, x + @as(i32, @intCast(thick)), y2);
    }

    fn hlineMiddle(self: *const Canvas, thick: u32) void {
        const y: i32 = @intCast((self.ch -| thick) / 2);
        self.hlineI(0, @intCast(self.cw), y, thick);
    }

    fn vlineMiddle(self: *const Canvas, thick: u32) void {
        const x: i32 = @intCast((self.cw -| thick) / 2);
        self.vlineI(0, @intCast(self.ch), x, thick);
    }

    // ---- box drawing: U+2500..U+257F ----

    fn boxDrawing(self: *const Canvas, cp: u32) void {
        const base = self.base;
        switch (cp) {
            0x2500 => self.linesChar(.{ .left = .light, .right = .light }),
            0x2501 => self.linesChar(.{ .left = .heavy, .right = .heavy }),
            0x2502 => self.linesChar(.{ .up = .light, .down = .light }),
            0x2503 => self.linesChar(.{ .up = .heavy, .down = .heavy }),
            0x2504 => self.dashHorizontal(3, base, @max(4, base)),
            0x2505 => self.dashHorizontal(3, base * 2, @max(4, base)),
            0x2506 => self.dashVertical(3, base, @max(4, base)),
            0x2507 => self.dashVertical(3, base * 2, @max(4, base)),
            0x2508 => self.dashHorizontal(4, base, @max(4, base)),
            0x2509 => self.dashHorizontal(4, base * 2, @max(4, base)),
            0x250a => self.dashVertical(4, base, @max(4, base)),
            0x250b => self.dashVertical(4, base * 2, @max(4, base)),
            0x250c => self.linesChar(.{ .down = .light, .right = .light }),
            0x250d => self.linesChar(.{ .down = .light, .right = .heavy }),
            0x250e => self.linesChar(.{ .down = .heavy, .right = .light }),
            0x250f => self.linesChar(.{ .down = .heavy, .right = .heavy }),
            0x2510 => self.linesChar(.{ .down = .light, .left = .light }),
            0x2511 => self.linesChar(.{ .down = .light, .left = .heavy }),
            0x2512 => self.linesChar(.{ .down = .heavy, .left = .light }),
            0x2513 => self.linesChar(.{ .down = .heavy, .left = .heavy }),
            0x2514 => self.linesChar(.{ .up = .light, .right = .light }),
            0x2515 => self.linesChar(.{ .up = .light, .right = .heavy }),
            0x2516 => self.linesChar(.{ .up = .heavy, .right = .light }),
            0x2517 => self.linesChar(.{ .up = .heavy, .right = .heavy }),
            0x2518 => self.linesChar(.{ .up = .light, .left = .light }),
            0x2519 => self.linesChar(.{ .up = .light, .left = .heavy }),
            0x251a => self.linesChar(.{ .up = .heavy, .left = .light }),
            0x251b => self.linesChar(.{ .up = .heavy, .left = .heavy }),
            0x251c => self.linesChar(.{ .up = .light, .down = .light, .right = .light }),
            0x251d => self.linesChar(.{ .up = .light, .down = .light, .right = .heavy }),
            0x251e => self.linesChar(.{ .up = .heavy, .right = .light, .down = .light }),
            0x251f => self.linesChar(.{ .down = .heavy, .right = .light, .up = .light }),
            0x2520 => self.linesChar(.{ .up = .heavy, .down = .heavy, .right = .light }),
            0x2521 => self.linesChar(.{ .down = .light, .right = .heavy, .up = .heavy }),
            0x2522 => self.linesChar(.{ .up = .light, .right = .heavy, .down = .heavy }),
            0x2523 => self.linesChar(.{ .up = .heavy, .down = .heavy, .right = .heavy }),
            0x2524 => self.linesChar(.{ .up = .light, .down = .light, .left = .light }),
            0x2525 => self.linesChar(.{ .up = .light, .down = .light, .left = .heavy }),
            0x2526 => self.linesChar(.{ .up = .heavy, .left = .light, .down = .light }),
            0x2527 => self.linesChar(.{ .down = .heavy, .left = .light, .up = .light }),
            0x2528 => self.linesChar(.{ .up = .heavy, .down = .heavy, .left = .light }),
            0x2529 => self.linesChar(.{ .down = .light, .left = .heavy, .up = .heavy }),
            0x252a => self.linesChar(.{ .up = .light, .left = .heavy, .down = .heavy }),
            0x252b => self.linesChar(.{ .up = .heavy, .down = .heavy, .left = .heavy }),
            0x252c => self.linesChar(.{ .down = .light, .left = .light, .right = .light }),
            0x252d => self.linesChar(.{ .left = .heavy, .right = .light, .down = .light }),
            0x252e => self.linesChar(.{ .right = .heavy, .left = .light, .down = .light }),
            0x252f => self.linesChar(.{ .down = .light, .left = .heavy, .right = .heavy }),
            0x2530 => self.linesChar(.{ .down = .heavy, .left = .light, .right = .light }),
            0x2531 => self.linesChar(.{ .right = .light, .left = .heavy, .down = .heavy }),
            0x2532 => self.linesChar(.{ .left = .light, .right = .heavy, .down = .heavy }),
            0x2533 => self.linesChar(.{ .down = .heavy, .left = .heavy, .right = .heavy }),
            0x2534 => self.linesChar(.{ .up = .light, .left = .light, .right = .light }),
            0x2535 => self.linesChar(.{ .left = .heavy, .right = .light, .up = .light }),
            0x2536 => self.linesChar(.{ .right = .heavy, .left = .light, .up = .light }),
            0x2537 => self.linesChar(.{ .up = .light, .left = .heavy, .right = .heavy }),
            0x2538 => self.linesChar(.{ .up = .heavy, .left = .light, .right = .light }),
            0x2539 => self.linesChar(.{ .right = .light, .left = .heavy, .up = .heavy }),
            0x253a => self.linesChar(.{ .left = .light, .right = .heavy, .up = .heavy }),
            0x253b => self.linesChar(.{ .up = .heavy, .left = .heavy, .right = .heavy }),
            0x253c => self.linesChar(.{ .up = .light, .down = .light, .left = .light, .right = .light }),
            0x253d => self.linesChar(.{ .left = .heavy, .right = .light, .up = .light, .down = .light }),
            0x253e => self.linesChar(.{ .right = .heavy, .left = .light, .up = .light, .down = .light }),
            0x253f => self.linesChar(.{ .up = .light, .down = .light, .left = .heavy, .right = .heavy }),
            0x2540 => self.linesChar(.{ .up = .heavy, .down = .light, .left = .light, .right = .light }),
            0x2541 => self.linesChar(.{ .down = .heavy, .up = .light, .left = .light, .right = .light }),
            0x2542 => self.linesChar(.{ .up = .heavy, .down = .heavy, .left = .light, .right = .light }),
            0x2543 => self.linesChar(.{ .left = .heavy, .up = .heavy, .right = .light, .down = .light }),
            0x2544 => self.linesChar(.{ .right = .heavy, .up = .heavy, .left = .light, .down = .light }),
            0x2545 => self.linesChar(.{ .left = .heavy, .down = .heavy, .right = .light, .up = .light }),
            0x2546 => self.linesChar(.{ .right = .heavy, .down = .heavy, .left = .light, .up = .light }),
            0x2547 => self.linesChar(.{ .down = .light, .up = .heavy, .left = .heavy, .right = .heavy }),
            0x2548 => self.linesChar(.{ .up = .light, .down = .heavy, .left = .heavy, .right = .heavy }),
            0x2549 => self.linesChar(.{ .right = .light, .left = .heavy, .up = .heavy, .down = .heavy }),
            0x254a => self.linesChar(.{ .left = .light, .right = .heavy, .up = .heavy, .down = .heavy }),
            0x254b => self.linesChar(.{ .up = .heavy, .down = .heavy, .left = .heavy, .right = .heavy }),
            0x254c => self.dashHorizontal(2, base, base),
            0x254d => self.dashHorizontal(2, base * 2, base * 2),
            0x254e => self.dashVertical(2, base, base * 2),
            0x254f => self.dashVertical(2, base * 2, base * 2),
            0x2550 => self.linesChar(.{ .left = .double, .right = .double }),
            0x2551 => self.linesChar(.{ .up = .double, .down = .double }),
            0x2552 => self.linesChar(.{ .down = .light, .right = .double }),
            0x2553 => self.linesChar(.{ .down = .double, .right = .light }),
            0x2554 => self.linesChar(.{ .down = .double, .right = .double }),
            0x2555 => self.linesChar(.{ .down = .light, .left = .double }),
            0x2556 => self.linesChar(.{ .down = .double, .left = .light }),
            0x2557 => self.linesChar(.{ .down = .double, .left = .double }),
            0x2558 => self.linesChar(.{ .up = .light, .right = .double }),
            0x2559 => self.linesChar(.{ .up = .double, .right = .light }),
            0x255a => self.linesChar(.{ .up = .double, .right = .double }),
            0x255b => self.linesChar(.{ .up = .light, .left = .double }),
            0x255c => self.linesChar(.{ .up = .double, .left = .light }),
            0x255d => self.linesChar(.{ .up = .double, .left = .double }),
            0x255e => self.linesChar(.{ .up = .light, .down = .light, .right = .double }),
            0x255f => self.linesChar(.{ .up = .double, .down = .double, .right = .light }),
            0x2560 => self.linesChar(.{ .up = .double, .down = .double, .right = .double }),
            0x2561 => self.linesChar(.{ .up = .light, .down = .light, .left = .double }),
            0x2562 => self.linesChar(.{ .up = .double, .down = .double, .left = .light }),
            0x2563 => self.linesChar(.{ .up = .double, .down = .double, .left = .double }),
            0x2564 => self.linesChar(.{ .down = .light, .left = .double, .right = .double }),
            0x2565 => self.linesChar(.{ .down = .double, .left = .light, .right = .light }),
            0x2566 => self.linesChar(.{ .down = .double, .left = .double, .right = .double }),
            0x2567 => self.linesChar(.{ .up = .light, .left = .double, .right = .double }),
            0x2568 => self.linesChar(.{ .up = .double, .left = .light, .right = .light }),
            0x2569 => self.linesChar(.{ .up = .double, .left = .double, .right = .double }),
            0x256a => self.linesChar(.{ .up = .light, .down = .light, .left = .double, .right = .double }),
            0x256b => self.linesChar(.{ .up = .double, .down = .double, .left = .light, .right = .light }),
            0x256c => self.linesChar(.{ .up = .double, .down = .double, .left = .double, .right = .double }),
            0x256d => self.arc(.br),
            0x256e => self.arc(.bl),
            0x256f => self.arc(.tl),
            0x2570 => self.arc(.tr),
            0x2571 => self.diagUpperRightToLowerLeft(),
            0x2572 => self.diagUpperLeftToLowerRight(),
            0x2573 => {
                self.diagUpperRightToLowerLeft();
                self.diagUpperLeftToLowerRight();
            },
            0x2574 => self.linesChar(.{ .left = .light }),
            0x2575 => self.linesChar(.{ .up = .light }),
            0x2576 => self.linesChar(.{ .right = .light }),
            0x2577 => self.linesChar(.{ .down = .light }),
            0x2578 => self.linesChar(.{ .left = .heavy }),
            0x2579 => self.linesChar(.{ .up = .heavy }),
            0x257a => self.linesChar(.{ .right = .heavy }),
            0x257b => self.linesChar(.{ .down = .heavy }),
            0x257c => self.linesChar(.{ .left = .light, .right = .heavy }),
            0x257d => self.linesChar(.{ .up = .light, .down = .heavy }),
            0x257e => self.linesChar(.{ .left = .heavy, .right = .light }),
            0x257f => self.linesChar(.{ .up = .heavy, .down = .light }),
            else => unreachable,
        }
    }

    /// Port of Ghostty box.linesChar: each stub is a filled rect from a cell
    /// edge to the centre, so adjacent cells meet exactly. The join offsets
    /// (up_bottom/down_top/left_right/right_left) shift a stub's inner end when
    /// a perpendicular heavy/double stub is present, and the double-line arms
    /// notch around the crossing so the two rails stay separated.
    fn linesChar(self: *const Canvas, lines: Lines) void {
        const light_px = self.base;
        const heavy_px = self.base * 2;

        const h_light_top = (self.ch -| light_px) / 2;
        const h_light_bottom = h_light_top +| light_px;
        const h_heavy_top = (self.ch -| heavy_px) / 2;
        const h_heavy_bottom = h_heavy_top +| heavy_px;
        const h_double_top = h_light_top -| light_px;
        const h_double_bottom = h_light_bottom +| light_px;

        const v_light_left = (self.cw -| light_px) / 2;
        const v_light_right = v_light_left +| light_px;
        const v_heavy_left = (self.cw -| heavy_px) / 2;
        const v_heavy_right = v_heavy_left +| heavy_px;
        const v_double_left = v_light_left -| light_px;
        const v_double_right = v_light_right +| light_px;

        const up_bottom = if (lines.left == .heavy or lines.right == .heavy)
            h_heavy_bottom
        else if (lines.left != lines.right or lines.down == lines.up)
            if (lines.left == .double or lines.right == .double) h_double_bottom else h_light_bottom
        else if (lines.left == .none and lines.right == .none)
            h_light_bottom
        else
            h_light_top;

        const down_top = if (lines.left == .heavy or lines.right == .heavy)
            h_heavy_top
        else if (lines.left != lines.right or lines.up == lines.down)
            if (lines.left == .double or lines.right == .double) h_double_top else h_light_top
        else if (lines.left == .none and lines.right == .none)
            h_light_top
        else
            h_light_bottom;

        const left_right = if (lines.up == .heavy or lines.down == .heavy)
            v_heavy_right
        else if (lines.up != lines.down or lines.left == lines.right)
            if (lines.up == .double or lines.down == .double) v_double_right else v_light_right
        else if (lines.up == .none and lines.down == .none)
            v_light_right
        else
            v_light_left;

        const right_left = if (lines.up == .heavy or lines.down == .heavy)
            v_heavy_left
        else if (lines.up != lines.down or lines.right == lines.left)
            if (lines.up == .double or lines.down == .double) v_double_left else v_light_left
        else if (lines.up == .none and lines.down == .none)
            v_light_left
        else
            v_light_right;

        switch (lines.up) {
            .none => {},
            .light => self.boxU(v_light_left, 0, v_light_right, up_bottom),
            .heavy => self.boxU(v_heavy_left, 0, v_heavy_right, up_bottom),
            .double => {
                const left_bottom = if (lines.left == .double) h_light_top else up_bottom;
                const right_bottom = if (lines.right == .double) h_light_top else up_bottom;
                self.boxU(v_double_left, 0, v_light_left, left_bottom);
                self.boxU(v_light_right, 0, v_double_right, right_bottom);
            },
        }

        switch (lines.right) {
            .none => {},
            .light => self.boxU(right_left, h_light_top, self.cw, h_light_bottom),
            .heavy => self.boxU(right_left, h_heavy_top, self.cw, h_heavy_bottom),
            .double => {
                const top_left = if (lines.up == .double) v_light_right else right_left;
                const bottom_left = if (lines.down == .double) v_light_right else right_left;
                self.boxU(top_left, h_double_top, self.cw, h_light_top);
                self.boxU(bottom_left, h_light_bottom, self.cw, h_double_bottom);
            },
        }

        switch (lines.down) {
            .none => {},
            .light => self.boxU(v_light_left, down_top, v_light_right, self.ch),
            .heavy => self.boxU(v_heavy_left, down_top, v_heavy_right, self.ch),
            .double => {
                const left_top = if (lines.left == .double) h_light_bottom else down_top;
                const right_top = if (lines.right == .double) h_light_bottom else down_top;
                self.boxU(v_double_left, left_top, v_light_left, self.ch);
                self.boxU(v_light_right, right_top, v_double_right, self.ch);
            },
        }

        switch (lines.left) {
            .none => {},
            .light => self.boxU(0, h_light_top, left_right, h_light_bottom),
            .heavy => self.boxU(0, h_heavy_top, left_right, h_heavy_bottom),
            .double => {
                const top_right = if (lines.up == .double) v_light_left else left_right;
                const bottom_right = if (lines.down == .double) v_light_left else left_right;
                self.boxU(0, h_double_top, top_right, h_light_top);
                self.boxU(0, h_light_bottom, bottom_right, h_double_bottom);
            },
        }
    }

    fn dashHorizontal(self: *const Canvas, count: u32, thick_px: u32, desired_gap: u32) void {
        const gap_count = count;
        if (self.cw < count + gap_count) {
            self.hlineMiddle(self.base);
            return;
        }
        const gap_width: i32 = @intCast(@min(desired_gap, self.cw / (2 * count)));
        const total_gap_width: i32 = @as(i32, @intCast(gap_count)) * gap_width;
        const total_dash_width: i32 = @as(i32, @intCast(self.cw)) - total_gap_width;
        const dash_width: i32 = @divFloor(total_dash_width, @as(i32, @intCast(count)));
        var extra: i32 = @mod(total_dash_width, @as(i32, @intCast(count)));
        const y: i32 = @intCast((self.ch -| thick_px) / 2);
        var x: i32 = @divFloor(gap_width, 2);
        var i: u32 = 0;
        while (i < count) : (i += 1) {
            var x1 = x + dash_width;
            if (extra > 0) {
                extra -= 1;
                x1 += 1;
            }
            self.hlineI(x, x1, y, thick_px);
            x = x1 + gap_width;
        }
    }

    fn dashVertical(self: *const Canvas, count: u32, thick_px: u32, desired_gap: u32) void {
        const gap_count = count;
        if (self.ch < count + gap_count) {
            self.vlineMiddle(self.base);
            return;
        }
        const gap_height: i32 = @intCast(@min(desired_gap, self.ch / (2 * count)));
        const total_gap_height: i32 = @as(i32, @intCast(gap_count)) * gap_height;
        const total_dash_height: i32 = @as(i32, @intCast(self.ch)) - total_gap_height;
        const dash_height: i32 = @divFloor(total_dash_height, @as(i32, @intCast(count)));
        var extra: i32 = @mod(total_dash_height, @as(i32, @intCast(count)));
        const x: i32 = @intCast((self.cw -| thick_px) / 2);
        var y: i32 = 0;
        var i: u32 = 0;
        while (i < count) : (i += 1) {
            var y1 = y + dash_height;
            if (extra > 0) {
                extra -= 1;
                y1 += 1;
            }
            self.vlineI(y, y1, x, thick_px);
            y = y1 + gap_height;
        }
    }

    /// Rounded corner (U+256D..U+2570). A stroked quarter-circle joining the
    /// centred vertical and horizontal stubs, port of Ghostty box.arc.
    fn arc(self: *const Canvas, corner: Corner) void {
        const thick_px = self.base;
        const fw: f64 = @floatFromInt(self.cw);
        const fh: f64 = @floatFromInt(self.ch);
        const ft: f64 = @floatFromInt(thick_px);
        const cx: f64 = @as(f64, @floatFromInt((self.cw -| thick_px) / 2)) + ft / 2;
        const cy: f64 = @as(f64, @floatFromInt((self.ch -| thick_px) / 2)) + ft / 2;
        const r = @min(fw, fh) / 2;
        const s: f64 = 0.25;

        cairo.Context.newPath(self.cr);
        switch (corner) {
            .tl => {
                cairo.Context.moveTo(self.cr, cx, 0);
                cairo.Context.lineTo(self.cr, cx, cy - r);
                cairo.Context.curveTo(self.cr, cx, cy - s * r, cx - s * r, cy, cx - r, cy);
                cairo.Context.lineTo(self.cr, 0, cy);
            },
            .tr => {
                cairo.Context.moveTo(self.cr, cx, 0);
                cairo.Context.lineTo(self.cr, cx, cy - r);
                cairo.Context.curveTo(self.cr, cx, cy - s * r, cx + s * r, cy, cx + r, cy);
                cairo.Context.lineTo(self.cr, fw, cy);
            },
            .bl => {
                cairo.Context.moveTo(self.cr, cx, fh);
                cairo.Context.lineTo(self.cr, cx, cy + r);
                cairo.Context.curveTo(self.cr, cx, cy + s * r, cx - s * r, cy, cx - r, cy);
                cairo.Context.lineTo(self.cr, 0, cy);
            },
            .br => {
                cairo.Context.moveTo(self.cr, cx, fh);
                cairo.Context.lineTo(self.cr, cx, cy + r);
                cairo.Context.curveTo(self.cr, cx, cy + s * r, cx + s * r, cy, cx + r, cy);
                cairo.Context.lineTo(self.cr, fw, cy);
            },
        }
        cairo.Context.setAntialias(self.cr, .gray);
        cairo.Context.setLineWidth(self.cr, ft);
        cairo.Context.setLineCap(self.cr, .butt);
        cairo.Context.stroke(self.cr);
    }

    fn strokeLine(self: *const Canvas, x0: f64, y0: f64, x1: f64, y1: f64) void {
        cairo.Context.newPath(self.cr);
        cairo.Context.moveTo(self.cr, x0, y0);
        cairo.Context.lineTo(self.cr, x1, y1);
        cairo.Context.setAntialias(self.cr, .gray);
        cairo.Context.setLineWidth(self.cr, @floatFromInt(self.base));
        cairo.Context.setLineCap(self.cr, .butt);
        cairo.Context.stroke(self.cr);
    }

    fn diagUpperRightToLowerLeft(self: *const Canvas) void {
        const fw: f64 = @floatFromInt(self.cw);
        const fh: f64 = @floatFromInt(self.ch);
        const sx = @min(1.0, fw / fh);
        const sy = @min(1.0, fh / fw);
        self.strokeLine(fw + 0.5 * sx, -0.5 * sy, -0.5 * sx, fh + 0.5 * sy);
    }

    fn diagUpperLeftToLowerRight(self: *const Canvas) void {
        const fw: f64 = @floatFromInt(self.cw);
        const fh: f64 = @floatFromInt(self.ch);
        const sx = @min(1.0, fw / fh);
        const sy = @min(1.0, fh / fw);
        self.strokeLine(-0.5 * sx, -0.5 * sy, fw + 0.5 * sx, fh + 0.5 * sy);
    }

    // ---- block elements: U+2580..U+259F ----

    fn blockElement(self: *const Canvas, cp: u32) void {
        const e1: f64 = 0.125;
        const q1: f64 = 0.25;
        const e3: f64 = 0.375;
        const half: f64 = 0.5;
        const e5: f64 = 0.625;
        const q3: f64 = 0.75;
        const e7: f64 = 0.875;
        switch (cp) {
            0x2580 => self.alignedBlock(.center, .top, 1, half),
            0x2581 => self.alignedBlock(.center, .bottom, 1, e1),
            0x2582 => self.alignedBlock(.center, .bottom, 1, q1),
            0x2583 => self.alignedBlock(.center, .bottom, 1, e3),
            0x2584 => self.alignedBlock(.center, .bottom, 1, half),
            0x2585 => self.alignedBlock(.center, .bottom, 1, e5),
            0x2586 => self.alignedBlock(.center, .bottom, 1, q3),
            0x2587 => self.alignedBlock(.center, .bottom, 1, e7),
            0x2588 => self.boxFill(0, 0, @intCast(self.cw), @intCast(self.ch)),
            0x2589 => self.alignedBlock(.left, .middle, e7, 1),
            0x258a => self.alignedBlock(.left, .middle, q3, 1),
            0x258b => self.alignedBlock(.left, .middle, e5, 1),
            0x258c => self.alignedBlock(.left, .middle, half, 1),
            0x258d => self.alignedBlock(.left, .middle, e3, 1),
            0x258e => self.alignedBlock(.left, .middle, q1, 1),
            0x258f => self.alignedBlock(.left, .middle, e1, 1),
            0x2590 => self.alignedBlock(.right, .middle, half, 1),
            0x2591 => self.shadeFull(0.25),
            0x2592 => self.shadeFull(0.5),
            0x2593 => self.shadeFull(0.75),
            0x2594 => self.alignedBlock(.center, .top, 1, e1),
            0x2595 => self.alignedBlock(.right, .middle, e1, 1),
            0x2596 => self.quadrant(false, false, true, false),
            0x2597 => self.quadrant(false, false, false, true),
            0x2598 => self.quadrant(true, false, false, false),
            0x2599 => self.quadrant(true, false, true, true),
            0x259a => self.quadrant(true, false, false, true),
            0x259b => self.quadrant(true, true, true, false),
            0x259c => self.quadrant(true, true, false, true),
            0x259d => self.quadrant(false, true, false, false),
            0x259e => self.quadrant(false, true, true, false),
            0x259f => self.quadrant(false, true, true, true),
            else => unreachable,
        }
    }

    fn alignedBlock(self: *const Canvas, ha: HAlign, va: VAlign, wfrac: f64, hfrac: f64) void {
        const fw: f64 = @floatFromInt(self.cw);
        const fh: f64 = @floatFromInt(self.ch);
        const w: i32 = @intFromFloat(@round(fw * wfrac));
        const h: i32 = @intFromFloat(@round(fh * hfrac));
        const cwi: i32 = @intCast(self.cw);
        const chi: i32 = @intCast(self.ch);
        const x: i32 = switch (ha) {
            .left => 0,
            .right => cwi - w,
            .center => @divTrunc(cwi - w, 2),
        };
        const y: i32 = switch (va) {
            .top => 0,
            .bottom => chi - h,
            .middle => @divTrunc(chi - h, 2),
        };
        self.boxFill(x, y, x + w, y + h);
    }

    /// Fill any subset of the four quadrants. Uses complementary rounding
    /// (min from the low edge, max from the high edge) so opposing quadrants
    /// tile with no seam or overlap, matching Ghostty's Fraction model.
    fn quadrant(self: *const Canvas, tl: bool, tr: bool, bl: bool, br: bool) void {
        const cwi: i32 = @intCast(self.cw);
        const chi: i32 = @intCast(self.ch);
        const xm = self.cw - @as(u32, @intFromFloat(@round(0.5 * @as(f64, @floatFromInt(self.cw)))));
        const ym = self.ch - @as(u32, @intFromFloat(@round(0.5 * @as(f64, @floatFromInt(self.ch)))));
        const xh: i32 = @intCast(xm);
        const yh: i32 = @intCast(ym);
        if (tl) self.boxFill(0, 0, xh, yh);
        if (tr) self.boxFill(xh, 0, cwi, yh);
        if (bl) self.boxFill(0, yh, xh, chi);
        if (br) self.boxFill(xh, yh, cwi, chi);
    }

    fn shadeFull(self: *const Canvas, alpha: f64) void {
        cairo.Context.setAntialias(self.cr, .none);
        cairo.Context.setSourceRgba(self.cr, self.r, self.g, self.b, alpha);
        cairo.Context.rectangle(self.cr, 0, 0, @floatFromInt(self.cw), @floatFromInt(self.ch));
        cairo.Context.fill(self.cr);
    }

    // ---- braille: U+2800..U+28FF ----

    /// Algorithmic 2x4 dot grid. The low 8 bits select dots; bit order follows
    /// the Unicode braille layout (Ghostty braille.Pattern). Dot size/spacing is
    /// fit to the cell the same way Ghostty does.
    fn braille(self: *const Canvas, cp: u32) void {
        const width = self.cw;
        const height = self.ch;

        var w: i32 = @intCast(@min(width / 4, height / 8));
        var x_spacing: i32 = @intCast(width / 4);
        var y_spacing: i32 = @intCast(height / 8);
        var x_margin: i32 = @divFloor(x_spacing, 2);
        var y_margin: i32 = @divFloor(y_spacing, 2);

        var x_px_left: i32 = @as(i32, @intCast(width)) - 2 * x_margin - x_spacing - 2 * w;
        var y_px_left: i32 = @as(i32, @intCast(height)) - 2 * y_margin - 3 * y_spacing - 4 * w;

        if (x_px_left >= 2 and y_px_left >= 4 and w == 0) {
            w += 1;
            x_px_left -= 2;
            y_px_left -= 4;
        }
        if (x_px_left >= 2 and x_margin == 0) {
            x_margin = 1;
            x_px_left -= 2;
        }
        if (y_px_left >= 2 and y_margin == 0) {
            y_margin = 1;
            y_px_left -= 2;
        }
        if (x_px_left >= 1) {
            x_spacing += 1;
            x_px_left -= 1;
        }
        if (y_px_left >= 3) {
            y_spacing += 1;
            y_px_left -= 3;
        }
        if (x_px_left >= 2) {
            x_margin += 1;
            x_px_left -= 2;
        }
        if (y_px_left >= 2) {
            y_margin += 1;
            y_px_left -= 2;
        }
        if (x_px_left >= 2 and y_px_left >= 4) {
            w += 1;
            x_px_left -= 2;
            y_px_left -= 4;
        }
        if (w <= 0) return;

        const xs = [2]i32{ x_margin, x_margin + w + x_spacing };
        var ys: [4]i32 = undefined;
        ys[0] = y_margin;
        ys[1] = ys[0] + w + y_spacing;
        ys[2] = ys[1] + w + y_spacing;
        ys[3] = ys[2] + w + y_spacing;

        const bits: u8 = @truncate(cp);
        // Bit -> (column, row): dots 1-3 then 7 in the left column, 4-6 then 8
        // in the right column (Unicode braille layout).
        const dots = [8]struct { c: u1, r: u2 }{
            .{ .c = 0, .r = 0 }, // tl (bit 0)
            .{ .c = 0, .r = 1 }, // ul (bit 1)
            .{ .c = 0, .r = 2 }, // ll (bit 2)
            .{ .c = 1, .r = 0 }, // tr (bit 3)
            .{ .c = 1, .r = 1 }, // ur (bit 4)
            .{ .c = 1, .r = 2 }, // lr (bit 5)
            .{ .c = 0, .r = 3 }, // bl (bit 6)
            .{ .c = 1, .r = 3 }, // br (bit 7)
        };
        inline for (dots, 0..) |d, i| {
            if (bits & (@as(u8, 1) << i) != 0) {
                const x = xs[d.c];
                const y = ys[d.r];
                self.boxFill(x, y, x + w, y + w);
            }
        }
    }

    // ---- Powerline: U+E0B0..U+E0B7 ----

    fn powerline(self: *const Canvas, cp: u32) void {
        const fw: f64 = @floatFromInt(self.cw);
        const fh: f64 = @floatFromInt(self.ch);
        switch (cp) {
            0xe0b0 => self.fillTriangle(0, 0, fw, fh / 2, 0, fh),
            0xe0b1 => self.chevron(false),
            0xe0b2 => self.fillTriangle(fw, 0, 0, fh / 2, fw, fh),
            0xe0b3 => self.chevron(true),
            0xe0b4 => self.halfDiscFill(false),
            0xe0b5 => self.halfDiscStroke(false),
            0xe0b6 => self.halfDiscFill(true),
            0xe0b7 => self.halfDiscStroke(true),
            else => unreachable,
        }
    }

    fn fillTriangle(self: *const Canvas, x0: f64, y0: f64, x1: f64, y1: f64, x2: f64, y2: f64) void {
        cairo.Context.newPath(self.cr);
        cairo.Context.moveTo(self.cr, x0, y0);
        cairo.Context.lineTo(self.cr, x1, y1);
        cairo.Context.lineTo(self.cr, x2, y2);
        cairo.Context.closePath(self.cr);
        cairo.Context.setAntialias(self.cr, .gray);
        cairo.Context.fill(self.cr);
    }

    fn chevron(self: *const Canvas, flip: bool) void {
        const fw: f64 = @floatFromInt(self.cw);
        const fh: f64 = @floatFromInt(self.ch);
        const x0: f64 = if (flip) fw else 0;
        const x1: f64 = if (flip) 0 else fw;
        cairo.Context.newPath(self.cr);
        cairo.Context.moveTo(self.cr, x0, 0);
        cairo.Context.lineTo(self.cr, x1, fh / 2);
        cairo.Context.lineTo(self.cr, x0, fh);
        cairo.Context.setAntialias(self.cr, .gray);
        cairo.Context.setLineWidth(self.cr, @floatFromInt(self.base));
        cairo.Context.setLineCap(self.cr, .butt);
        cairo.Context.stroke(self.cr);
    }

    fn halfDiscFill(self: *const Canvas, flip: bool) void {
        cairo.Context.save(self.cr);
        defer cairo.Context.restore(self.cr);
        const fw: f64 = @floatFromInt(self.cw);
        const fh: f64 = @floatFromInt(self.ch);
        if (flip) {
            cairo.Context.translate(self.cr, fw, 0);
            cairo.Context.scale(self.cr, -1, 1);
        }
        const c: f64 = (std.math.sqrt2 - 1.0) * 4.0 / 3.0;
        const radius: f64 = @min(fw, fh / 2);
        cairo.Context.newPath(self.cr);
        cairo.Context.moveTo(self.cr, 0, 0);
        cairo.Context.curveTo(self.cr, radius * c, 0, radius, radius - radius * c, radius, radius);
        cairo.Context.lineTo(self.cr, radius, fh - radius);
        cairo.Context.curveTo(self.cr, radius, fh - radius + radius * c, radius * c, fh, 0, fh);
        cairo.Context.closePath(self.cr);
        cairo.Context.setAntialias(self.cr, .gray);
        cairo.Context.fill(self.cr);
    }

    fn halfDiscStroke(self: *const Canvas, flip: bool) void {
        cairo.Context.save(self.cr);
        defer cairo.Context.restore(self.cr);
        const fw: f64 = @floatFromInt(self.cw);
        const fh: f64 = @floatFromInt(self.ch);
        if (flip) {
            cairo.Context.translate(self.cr, fw, 0);
            cairo.Context.scale(self.cr, -1, 1);
        }
        const c: f64 = (std.math.sqrt2 - 1.0) * 4.0 / 3.0;
        const radius: f64 = @min(fw, fh / 2);
        cairo.Context.newPath(self.cr);
        cairo.Context.moveTo(self.cr, 0, 0);
        cairo.Context.curveTo(self.cr, radius * c, 0, radius, radius - radius * c, radius, radius);
        cairo.Context.lineTo(self.cr, radius, fh - radius);
        cairo.Context.curveTo(self.cr, radius, fh - radius + radius * c, radius * c, fh, 0, fh);
        cairo.Context.setAntialias(self.cr, .gray);
        cairo.Context.setLineWidth(self.cr, @floatFromInt(self.base));
        cairo.Context.setLineCap(self.cr, .butt);
        cairo.Context.stroke(self.cr);
    }
};
