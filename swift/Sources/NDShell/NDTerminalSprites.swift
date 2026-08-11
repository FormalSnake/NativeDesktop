import AppKit
import CoreGraphics

/// Geometric sprite rendering for the ranges Ghostty draws as geometry rather
/// than font glyphs: box drawing (U+2500-257F), block elements (U+2580-259F),
/// Braille (U+2800-28FF), and Powerline separators (U+E0B0-E0B7). Drawing them
/// this way makes borders tile with no seams and Powerline separators fill
/// edge to edge, which a Nerd Font's own glyphs never manage across cells.
///
/// Geometry mirrors `ghostty/src/font/sprite/draw/{box,block,braille,powerline}.zig`.
/// Axis-aligned box/block strokes are computed in device pixels and filled with
/// antialiasing off so rules stay crisp on Retina; arcs, diagonals, Braille
/// dots, and Powerline curves are drawn in point space with antialiasing on.
enum NDSprite {
    /// Codepoints handled here. Every value in these ranges is drawn; a value
    /// outside them falls through to the font path.
    static func isSprite(_ cp: UInt32) -> Bool {
        (cp >= 0x2500 && cp <= 0x259F) ||
            (cp >= 0x2800 && cp <= 0x28FF) ||
            (cp >= 0xE0B0 && cp <= 0xE0B7)
    }

    /// Draw the sprite for `cp` into the cell rect (`x`,`y`,`w`,`h` in points),
    /// filled/stroked in `color`. `thickness` is the light line weight in
    /// points (heavy is 2x); `scale` is the backing scale factor so strokes
    /// land on whole device pixels.
    static func draw(cp: UInt32, ctx: CGContext,
                     x: CGFloat, y: CGFloat, w: CGFloat, h: CGFloat,
                     color: NSColor, thickness: CGFloat, scale: CGFloat) {
        let pen = Pen(ctx: ctx, x0: x, y0: y, w: w, h: h, s: scale,
                      color: color.cgColor, thickness: thickness)
        ctx.saveGState()
        ctx.setFillColor(color.cgColor)
        ctx.setStrokeColor(color.cgColor)
        switch cp {
        case 0x2500...0x257F: pen.box(Int(cp))
        case 0x2580...0x259F: pen.block(Int(cp))
        case 0x2800...0x28FF: pen.braille(Int(cp))
        default: pen.powerline(Int(cp))
        }
        ctx.restoreGState()
    }
}

private enum Corner { case tl, tr, bl, br }

/// One cell's worth of drawing state. All the `*Px` values are device pixels
/// (integer, matching Ghostty's atlas math); `fillPx` maps them back to points
/// through the scale factor so the rects land on the device pixel grid.
private struct Pen {
    let ctx: CGContext
    let x0: CGFloat
    let y0: CGFloat
    let w: CGFloat
    let h: CGFloat
    let s: CGFloat
    let color: CGColor
    let thickness: CGFloat

    var cwPx: Int { Int((w * s).rounded()) }
    var chPx: Int { Int((h * s).rounded()) }
    var lightPx: Int { max(Int((thickness * s).rounded()), 1) }
    var heavyPx: Int { lightPx * 2 }

    /// Fill a device-pixel rect (edges snap to whole device pixels).
    func fillPx(_ ax: Int, _ ay: Int, _ bx: Int, _ by: Int) {
        guard bx > ax, by > ay else { return }
        ctx.fill(CGRect(x: x0 + CGFloat(ax) / s, y: y0 + CGFloat(ay) / s,
                        width: CGFloat(bx - ax) / s, height: CGFloat(by - ay) / s))
    }

    private func pt(_ rx: CGFloat, _ ry: CGFloat) -> CGPoint { CGPoint(x: x0 + rx, y: y0 + ry) }

    // MARK: box drawing

    func box(_ cp: Int) {
        ctx.setShouldAntialias(false)
        let lp = lightPx, hp = heavyPx
        switch cp {
        case 0x2504: dashH(3, lp, max(4, lp))
        case 0x2505: dashH(3, hp, max(4, lp))
        case 0x2506: dashV(3, lp, max(4, lp))
        case 0x2507: dashV(3, hp, max(4, lp))
        case 0x2508: dashH(4, lp, max(4, lp))
        case 0x2509: dashH(4, hp, max(4, lp))
        case 0x250A: dashV(4, lp, max(4, lp))
        case 0x250B: dashV(4, hp, max(4, lp))
        case 0x254C: dashH(2, lp, lp)
        case 0x254D: dashH(2, hp, hp)
        case 0x254E: dashV(2, lp, hp)
        case 0x254F: dashV(2, hp, hp)
        case 0x256D: arc(.br)
        case 0x256E: arc(.bl)
        case 0x256F: arc(.tl)
        case 0x2570: arc(.tr)
        case 0x2571: diagonals(ne: true, se: false)
        case 0x2572: diagonals(ne: false, se: true)
        case 0x2573: diagonals(ne: true, se: true)
        default:
            let l = Self.lines[cp - 0x2500]
            lines(up: Int(l >> 6) & 3, right: Int(l >> 4) & 3, down: Int(l >> 2) & 3, left: Int(l) & 3)
        }
    }

    /// The 128 intersection glyphs, packed one byte each as `up|right|down|left`
    /// (2 bits per edge: 0 none, 1 light, 2 heavy, 3 double). Dash, arc, and
    /// diagonal codepoints are handled above and their slots here are unused.
    private static let lines: [UInt8] = {
        func e(_ up: Int, _ right: Int, _ down: Int, _ left: Int) -> UInt8 {
            UInt8((up << 6) | (right << 4) | (down << 2) | left)
        }
        var t = [UInt8](repeating: 0, count: 128)
        t[0x00] = e(0, 1, 0, 1); t[0x01] = e(0, 2, 0, 2); t[0x02] = e(1, 0, 1, 0); t[0x03] = e(2, 0, 2, 0)
        t[0x0C] = e(0, 1, 1, 0); t[0x0D] = e(0, 2, 1, 0); t[0x0E] = e(0, 1, 2, 0); t[0x0F] = e(0, 2, 2, 0)
        t[0x10] = e(0, 0, 1, 1); t[0x11] = e(0, 0, 1, 2); t[0x12] = e(0, 0, 2, 1); t[0x13] = e(0, 0, 2, 2)
        t[0x14] = e(1, 1, 0, 0); t[0x15] = e(1, 2, 0, 0); t[0x16] = e(2, 1, 0, 0); t[0x17] = e(2, 2, 0, 0)
        t[0x18] = e(1, 0, 0, 1); t[0x19] = e(1, 0, 0, 2); t[0x1A] = e(2, 0, 0, 1); t[0x1B] = e(2, 0, 0, 2)
        t[0x1C] = e(1, 1, 1, 0); t[0x1D] = e(1, 2, 1, 0); t[0x1E] = e(2, 1, 1, 0); t[0x1F] = e(1, 1, 2, 0)
        t[0x20] = e(2, 1, 2, 0); t[0x21] = e(2, 2, 1, 0); t[0x22] = e(1, 2, 2, 0); t[0x23] = e(2, 2, 2, 0)
        t[0x24] = e(1, 0, 1, 1); t[0x25] = e(1, 0, 1, 2); t[0x26] = e(2, 0, 1, 1); t[0x27] = e(1, 0, 2, 1)
        t[0x28] = e(2, 0, 2, 1); t[0x29] = e(2, 0, 1, 2); t[0x2A] = e(1, 0, 2, 2); t[0x2B] = e(2, 0, 2, 2)
        t[0x2C] = e(0, 1, 1, 1); t[0x2D] = e(0, 1, 1, 2); t[0x2E] = e(0, 2, 1, 1); t[0x2F] = e(0, 2, 1, 2)
        t[0x30] = e(0, 1, 2, 1); t[0x31] = e(0, 1, 2, 2); t[0x32] = e(0, 2, 2, 1); t[0x33] = e(0, 2, 2, 2)
        t[0x34] = e(1, 1, 0, 1); t[0x35] = e(1, 1, 0, 2); t[0x36] = e(1, 2, 0, 1); t[0x37] = e(1, 2, 0, 2)
        t[0x38] = e(2, 1, 0, 1); t[0x39] = e(2, 1, 0, 2); t[0x3A] = e(2, 2, 0, 1); t[0x3B] = e(2, 2, 0, 2)
        t[0x3C] = e(1, 1, 1, 1); t[0x3D] = e(1, 1, 1, 2); t[0x3E] = e(1, 2, 1, 1); t[0x3F] = e(1, 2, 1, 2)
        t[0x40] = e(2, 1, 1, 1); t[0x41] = e(1, 1, 2, 1); t[0x42] = e(2, 1, 2, 1); t[0x43] = e(2, 1, 1, 2)
        t[0x44] = e(2, 2, 1, 1); t[0x45] = e(1, 1, 2, 2); t[0x46] = e(1, 2, 2, 1); t[0x47] = e(2, 2, 1, 2)
        t[0x48] = e(1, 2, 2, 2); t[0x49] = e(2, 1, 2, 2); t[0x4A] = e(2, 2, 2, 1); t[0x4B] = e(2, 2, 2, 2)
        t[0x50] = e(0, 3, 0, 3); t[0x51] = e(3, 0, 3, 0); t[0x52] = e(0, 3, 1, 0); t[0x53] = e(0, 1, 3, 0)
        t[0x54] = e(0, 3, 3, 0); t[0x55] = e(0, 0, 1, 3); t[0x56] = e(0, 0, 3, 1); t[0x57] = e(0, 0, 3, 3)
        t[0x58] = e(1, 3, 0, 0); t[0x59] = e(3, 1, 0, 0); t[0x5A] = e(3, 3, 0, 0); t[0x5B] = e(1, 0, 0, 3)
        t[0x5C] = e(3, 0, 0, 1); t[0x5D] = e(3, 0, 0, 3); t[0x5E] = e(1, 3, 1, 0); t[0x5F] = e(3, 1, 3, 0)
        t[0x60] = e(3, 3, 3, 0); t[0x61] = e(1, 0, 1, 3); t[0x62] = e(3, 0, 3, 1); t[0x63] = e(3, 0, 3, 3)
        t[0x64] = e(0, 3, 1, 3); t[0x65] = e(0, 1, 3, 1); t[0x66] = e(0, 3, 3, 3); t[0x67] = e(1, 3, 0, 3)
        t[0x68] = e(3, 1, 0, 1); t[0x69] = e(3, 3, 0, 3); t[0x6A] = e(1, 3, 1, 3); t[0x6B] = e(3, 1, 3, 1)
        t[0x6C] = e(3, 3, 3, 3)
        t[0x74] = e(0, 0, 0, 1); t[0x75] = e(1, 0, 0, 0); t[0x76] = e(0, 1, 0, 0); t[0x77] = e(0, 0, 1, 0)
        t[0x78] = e(0, 0, 0, 2); t[0x79] = e(2, 0, 0, 0); t[0x7A] = e(0, 2, 0, 0); t[0x7B] = e(0, 0, 2, 0)
        t[0x7C] = e(0, 2, 0, 1); t[0x7D] = e(1, 0, 2, 0); t[0x7E] = e(0, 1, 0, 2); t[0x7F] = e(2, 0, 1, 0)
        return t
    }()

    /// Port of Ghostty's `linesChar`: each edge stub is a filled rect running
    /// from a shared center band out to its cell edge, so adjacent cells' stubs
    /// meet exactly. Heavy is a thicker band, double is two light rails.
    private func lines(up: Int, right: Int, down: Int, left: Int) {
        let lp = lightPx, hp = heavyPx, cw = cwPx, ch = chPx

        let hLightTop = max(0, ch - lp) / 2
        let hLightBottom = hLightTop + lp
        let hHeavyTop = max(0, ch - hp) / 2
        let hHeavyBottom = hHeavyTop + hp
        let hDoubleTop = max(0, hLightTop - lp)
        let hDoubleBottom = hLightBottom + lp

        let vLightLeft = max(0, cw - lp) / 2
        let vLightRight = vLightLeft + lp
        let vHeavyLeft = max(0, cw - hp) / 2
        let vHeavyRight = vHeavyLeft + hp
        let vDoubleLeft = max(0, vLightLeft - lp)
        let vDoubleRight = vLightRight + lp

        let upBottom: Int
        if left == 2 || right == 2 { upBottom = hHeavyBottom }
        else if left != right || down == up { upBottom = (left == 3 || right == 3) ? hDoubleBottom : hLightBottom }
        else if left == 0 && right == 0 { upBottom = hLightBottom }
        else { upBottom = hLightTop }

        let downTop: Int
        if left == 2 || right == 2 { downTop = hHeavyTop }
        else if left != right || up == down { downTop = (left == 3 || right == 3) ? hDoubleTop : hLightTop }
        else if left == 0 && right == 0 { downTop = hLightTop }
        else { downTop = hLightBottom }

        let leftRight: Int
        if up == 2 || down == 2 { leftRight = vHeavyRight }
        else if up != down || left == right { leftRight = (up == 3 || down == 3) ? vDoubleRight : vLightRight }
        else if up == 0 && down == 0 { leftRight = vLightRight }
        else { leftRight = vLightLeft }

        let rightLeft: Int
        if up == 2 || down == 2 { rightLeft = vHeavyLeft }
        else if up != down || right == left { rightLeft = (up == 3 || down == 3) ? vDoubleLeft : vLightLeft }
        else if up == 0 && down == 0 { rightLeft = vLightLeft }
        else { rightLeft = vLightRight }

        switch up {
        case 1: fillPx(vLightLeft, 0, vLightRight, upBottom)
        case 2: fillPx(vHeavyLeft, 0, vHeavyRight, upBottom)
        case 3:
            let lb = left == 3 ? hLightTop : upBottom
            let rb = right == 3 ? hLightTop : upBottom
            fillPx(vDoubleLeft, 0, vLightLeft, lb)
            fillPx(vLightRight, 0, vDoubleRight, rb)
        default: break
        }

        switch right {
        case 1: fillPx(rightLeft, hLightTop, cw, hLightBottom)
        case 2: fillPx(rightLeft, hHeavyTop, cw, hHeavyBottom)
        case 3:
            let tl = up == 3 ? vLightRight : rightLeft
            let bl = down == 3 ? vLightRight : rightLeft
            fillPx(tl, hDoubleTop, cw, hLightTop)
            fillPx(bl, hLightBottom, cw, hDoubleBottom)
        default: break
        }

        switch down {
        case 1: fillPx(vLightLeft, downTop, vLightRight, ch)
        case 2: fillPx(vHeavyLeft, downTop, vHeavyRight, ch)
        case 3:
            let lt = left == 3 ? hLightBottom : downTop
            let rt = right == 3 ? hLightBottom : downTop
            fillPx(vDoubleLeft, lt, vLightLeft, ch)
            fillPx(vLightRight, rt, vDoubleRight, ch)
        default: break
        }

        switch left {
        case 1: fillPx(0, hLightTop, leftRight, hLightBottom)
        case 2: fillPx(0, hHeavyTop, leftRight, hHeavyBottom)
        case 3:
            let tr = up == 3 ? vLightLeft : leftRight
            let br = down == 3 ? vLightLeft : leftRight
            fillPx(0, hDoubleTop, tr, hLightTop)
            fillPx(0, hLightBottom, br, hDoubleBottom)
        default: break
        }
    }

    private func dashH(_ count: Int, _ thickPx: Int, _ gap: Int) {
        let y = max(0, chPx - thickPx) / 2
        guard cwPx >= count * 2 else { fillPx(0, y, cwPx, y + thickPx); return }
        let gapWidth = min(gap, cwPx / (2 * count))
        let totalDash = cwPx - count * gapWidth
        let dashWidth = totalDash / count
        var remaining = totalDash % count
        var x = gapWidth / 2
        for _ in 0..<count {
            var x1 = x + dashWidth
            if remaining > 0 { remaining -= 1; x1 += 1 }
            fillPx(x, y, x1, y + thickPx)
            x = x1 + gapWidth
        }
    }

    private func dashV(_ count: Int, _ thickPx: Int, _ gap: Int) {
        let x = max(0, cwPx - thickPx) / 2
        guard chPx >= count * 2 else { fillPx(x, 0, x + thickPx, chPx); return }
        let gapHeight = min(gap, chPx / (2 * count))
        let totalDash = chPx - count * gapHeight
        let dashHeight = totalDash / count
        var remaining = totalDash % count
        var y = 0
        for _ in 0..<count {
            var y1 = y + dashHeight
            if remaining > 0 { remaining -= 1; y1 += 1 }
            fillPx(x, y, x + thickPx, y1)
            y = y1 + gapHeight
        }
    }

    private func arc(_ corner: Corner) {
        ctx.setShouldAntialias(true)
        let lp = lightPx
        let cx = x0 + (CGFloat(max(0, cwPx - lp) / 2) + CGFloat(lp) / 2) / s
        let cy = y0 + (CGFloat(max(0, chPx - lp) / 2) + CGFloat(lp) / 2) / s
        let r = CGFloat(min(cwPx, chPx)) / 2 / s
        let sr = 0.25 * r
        let path = CGMutablePath()
        switch corner {
        case .tl:
            path.move(to: CGPoint(x: cx, y: y0))
            path.addLine(to: CGPoint(x: cx, y: cy - r))
            path.addCurve(to: CGPoint(x: cx - r, y: cy), control1: CGPoint(x: cx, y: cy - sr), control2: CGPoint(x: cx - sr, y: cy))
            path.addLine(to: CGPoint(x: x0, y: cy))
        case .tr:
            path.move(to: CGPoint(x: cx, y: y0))
            path.addLine(to: CGPoint(x: cx, y: cy - r))
            path.addCurve(to: CGPoint(x: cx + r, y: cy), control1: CGPoint(x: cx, y: cy - sr), control2: CGPoint(x: cx + sr, y: cy))
            path.addLine(to: CGPoint(x: x0 + w, y: cy))
        case .bl:
            path.move(to: CGPoint(x: cx, y: y0 + h))
            path.addLine(to: CGPoint(x: cx, y: cy + r))
            path.addCurve(to: CGPoint(x: cx - r, y: cy), control1: CGPoint(x: cx, y: cy + sr), control2: CGPoint(x: cx - sr, y: cy))
            path.addLine(to: CGPoint(x: x0, y: cy))
        case .br:
            path.move(to: CGPoint(x: cx, y: y0 + h))
            path.addLine(to: CGPoint(x: cx, y: cy + r))
            path.addCurve(to: CGPoint(x: cx + r, y: cy), control1: CGPoint(x: cx, y: cy + sr), control2: CGPoint(x: cx + sr, y: cy))
            path.addLine(to: CGPoint(x: x0 + w, y: cy))
        }
        ctx.addPath(path)
        ctx.setLineWidth(CGFloat(lp) / s)
        ctx.setLineCap(.butt)
        ctx.strokePath()
    }

    private func diagonals(ne: Bool, se: Bool) {
        ctx.setShouldAntialias(true)
        let sx = min(1, w / h)
        let sy = min(1, h / w)
        ctx.setLineWidth(thickness)
        ctx.setLineCap(.butt)
        if ne {
            let p = CGMutablePath()
            p.move(to: pt(w + 0.5 * sx, -0.5 * sy))
            p.addLine(to: pt(-0.5 * sx, h + 0.5 * sy))
            ctx.addPath(p); ctx.strokePath()
        }
        if se {
            let p = CGMutablePath()
            p.move(to: pt(-0.5 * sx, -0.5 * sy))
            p.addLine(to: pt(w + 0.5 * sx, h + 0.5 * sy))
            ctx.addPath(p); ctx.strokePath()
        }
    }

    // MARK: block elements

    func block(_ cp: Int) {
        ctx.setShouldAntialias(false)
        switch cp {
        case 0x2580: rect(.center, .top, 1, 1.0 / 2)
        case 0x2581: rect(.center, .bottom, 1, 1.0 / 8)
        case 0x2582: rect(.center, .bottom, 1, 1.0 / 4)
        case 0x2583: rect(.center, .bottom, 1, 3.0 / 8)
        case 0x2584: rect(.center, .bottom, 1, 1.0 / 2)
        case 0x2585: rect(.center, .bottom, 1, 5.0 / 8)
        case 0x2586: rect(.center, .bottom, 1, 3.0 / 4)
        case 0x2587: rect(.center, .bottom, 1, 7.0 / 8)
        case 0x2588: rect(.left, .top, 1, 1)
        case 0x2589: rect(.left, .middle, 7.0 / 8, 1)
        case 0x258A: rect(.left, .middle, 3.0 / 4, 1)
        case 0x258B: rect(.left, .middle, 5.0 / 8, 1)
        case 0x258C: rect(.left, .middle, 1.0 / 2, 1)
        case 0x258D: rect(.left, .middle, 3.0 / 8, 1)
        case 0x258E: rect(.left, .middle, 1.0 / 4, 1)
        case 0x258F: rect(.left, .middle, 1.0 / 8, 1)
        case 0x2590: rect(.right, .middle, 1.0 / 2, 1)
        case 0x2591: rect(.left, .top, 1, 1, alpha: 0.25)
        case 0x2592: rect(.left, .top, 1, 1, alpha: 0.50)
        case 0x2593: rect(.left, .top, 1, 1, alpha: 0.75)
        case 0x2594: rect(.center, .top, 1, 1.0 / 8)
        case 0x2595: rect(.right, .middle, 1.0 / 8, 1)
        case 0x2596: quad(bl: true)
        case 0x2597: quad(br: true)
        case 0x2598: quad(tl: true)
        case 0x2599: quad(tl: true, bl: true, br: true)
        case 0x259A: quad(tl: true, br: true)
        case 0x259B: quad(tl: true, tr: true, bl: true)
        case 0x259C: quad(tl: true, tr: true, br: true)
        case 0x259D: quad(tr: true)
        case 0x259E: quad(tr: true, bl: true)
        case 0x259F: quad(tr: true, bl: true, br: true)
        default: break
        }
    }

    private enum HAlign { case left, center, right }
    private enum VAlign { case top, middle, bottom }

    private func rect(_ ha: HAlign, _ va: VAlign, _ wf: CGFloat, _ hf: CGFloat, alpha: CGFloat = 1) {
        let bw = Int((CGFloat(cwPx) * wf).rounded())
        let bh = Int((CGFloat(chPx) * hf).rounded())
        let bx: Int
        switch ha { case .left: bx = 0; case .center: bx = (cwPx - bw) / 2; case .right: bx = cwPx - bw }
        let by: Int
        switch va { case .top: by = 0; case .middle: by = (chPx - bh) / 2; case .bottom: by = chPx - bh }
        if alpha < 1, let faded = color.copy(alpha: alpha) { ctx.setFillColor(faded) }
        fillPx(bx, by, bx + bw, by + bh)
        if alpha < 1 { ctx.setFillColor(color) }
    }

    private func quad(tl: Bool = false, tr: Bool = false, bl: Bool = false, br: Bool = false) {
        let hw = Int((CGFloat(cwPx) / 2).rounded())
        let hh = Int((CGFloat(chPx) / 2).rounded())
        if tl { fillPx(0, 0, hw, hh) }
        if tr { fillPx(hw, 0, cwPx, hh) }
        if bl { fillPx(0, hh, hw, chPx) }
        if br { fillPx(hw, hh, cwPx, chPx) }
    }

    // MARK: braille

    /// The low byte selects dots in a 2x4 grid; bit order matches Unicode
    /// (dots 1-3 left column, 4-6 right column, then 7/8 on the bottom row).
    func braille(_ cp: Int) {
        ctx.setShouldAntialias(true)
        let bits = UInt8(truncatingIfNeeded: cp)
        let d = CGFloat(max(min(cwPx / 4, chPx / 8), 1)) / s
        let colX: [CGFloat] = [w * 0.25, w * 0.75]
        let rowY: [CGFloat] = [h * 0.125, h * 0.375, h * 0.625, h * 0.875]
        let dots: [(bit: Int, col: Int, row: Int)] = [
            (0, 0, 0), (1, 0, 1), (2, 0, 2), (3, 1, 0), (4, 1, 1), (5, 1, 2), (6, 0, 3), (7, 1, 3),
        ]
        for dot in dots where bits & (1 << dot.bit) != 0 {
            let c = pt(colX[dot.col], rowY[dot.row])
            ctx.fillEllipse(in: CGRect(x: c.x - d / 2, y: c.y - d / 2, width: d, height: d))
        }
    }

    // MARK: powerline

    func powerline(_ cp: Int) {
        ctx.setShouldAntialias(true)
        switch cp {
        case 0xE0B0: fillTriangle((0, 0), (w, h / 2), (0, h))
        case 0xE0B1: strokeChevron(pointsRight: true)
        case 0xE0B2: fillTriangle((w, 0), (0, h / 2), (w, h))
        case 0xE0B3: strokeChevron(pointsRight: false)
        case 0xE0B4: ctx.addPath(roundedPath(right: true)); ctx.fillPath()
        case 0xE0B5: strokeRounded(right: true)
        case 0xE0B6: ctx.addPath(roundedPath(right: false)); ctx.fillPath()
        case 0xE0B7: strokeRounded(right: false)
        default: break
        }
    }

    private func fillTriangle(_ a: (CGFloat, CGFloat), _ b: (CGFloat, CGFloat), _ c: (CGFloat, CGFloat)) {
        let p = CGMutablePath()
        p.move(to: pt(a.0, a.1))
        p.addLine(to: pt(b.0, b.1))
        p.addLine(to: pt(c.0, c.1))
        p.closeSubpath()
        ctx.addPath(p)
        ctx.fillPath()
    }

    private func strokeChevron(pointsRight: Bool) {
        let p = CGMutablePath()
        if pointsRight {
            p.move(to: pt(0, 0)); p.addLine(to: pt(w, h / 2)); p.addLine(to: pt(0, h))
        } else {
            p.move(to: pt(w, 0)); p.addLine(to: pt(0, h / 2)); p.addLine(to: pt(w, h))
        }
        ctx.addPath(p)
        ctx.setLineWidth(thickness)
        ctx.setLineCap(.butt)
        ctx.setLineJoin(.miter)
        ctx.strokePath()
    }

    /// Ghostty's E0B4 half-disc: flat side on the left, bulge to the right;
    /// `right == false` mirrors it (E0B6).
    private func roundedPath(right: Bool) -> CGPath {
        let c = (CGFloat(2).squareRoot() - 1) * 4 / 3
        let radius = min(w, h / 2)
        func p(_ rx: CGFloat, _ ry: CGFloat) -> CGPoint { pt(right ? rx : w - rx, ry) }
        let path = CGMutablePath()
        path.move(to: p(0, 0))
        path.addCurve(to: p(radius, radius), control1: p(radius * c, 0), control2: p(radius, radius - radius * c))
        path.addLine(to: p(radius, h - radius))
        path.addCurve(to: p(0, h), control1: p(radius, h - radius + radius * c), control2: p(radius * c, h))
        path.closeSubpath()
        return path
    }

    private func strokeRounded(right: Bool) {
        ctx.addPath(roundedPath(right: right))
        ctx.setLineWidth(thickness)
        ctx.setLineCap(.butt)
        ctx.strokePath()
    }
}
