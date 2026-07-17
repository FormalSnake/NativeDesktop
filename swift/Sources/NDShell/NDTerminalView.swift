import AppKit
import CoreText
import Foundation
import CNd

/// AppKit rendering + input surface for the `<terminal>` widget (peer of the
/// GTK surface in `src/gtk`). It owns nothing terminal-shaped itself: the PTY,
/// libghostty-vt instance, and grid state all live behind the `ndterm` C ABI
/// (`include/ndterm.h`, imported via `CNd`). This view opens a handle at
/// create, drives `ndterm_render_lock`/`_cell`/`_cursor` inside `draw`, maps
/// key/mouse/scroll events to the ndterm input ABI, tracks pixel size onto the
/// grid via `ndterm_resize`, and closes the handle on `deinit`.
///
/// The effect callback (title/bell/child-exit) is registered with `self` as
/// userdata; a remote view additionally registers a connection-state callback.
/// Both fire on a reader thread and marshal to the main queue before emitting.
///
/// Flipped like the rest of the shell (`FlippedView`, `NDPaneHostView`): the
/// grid is top-origin (row 0 is the top row), so a top-left y-down coordinate
/// space lets cell (x, y) draw at `(x*cellW, y*cellH)` directly, and AppKit's
/// string drawing stays right-side-up in a flipped view.
final class NDTerminalView: NSView {
    /// `nd_terminal *` (opaque to Swift). Nil only if the open failed. For a
    /// remote view it is `ndrt_terminal(rt)` — the same render handle the local
    /// path uses, so draw/input are backend-agnostic.
    /// nonisolated(unsafe): the @MainActor view's nonisolated `deinit` closes the
    /// handle; teardown is single-owner so the unchecked access is safe.
    nonisolated(unsafe) private var term: OpaquePointer?
    /// `nd_remote_terminal *` for a remote view (nil for a local PTY view).
    nonisolated(unsafe) private var rt: OpaquePointer?
    private let isRemote: Bool
    private let font: NSFont
    private let boldFont: NSFont
    private let cellW: CGFloat
    private let cellH: CGFloat
    /// Current grid dimensions. Start from the create-time props, then track
    /// the view's pixel size in `setFrameSize`/`layout` (→ `ndterm_resize`/
    /// `ndrt_resize`).
    private var cols: Int
    private var rows: Int
    /// Node id recorded by `ndTerminalConnect` (generated ndConnectEvents arm).
    /// 0 = not yet wired; the effect/state trampolines gate on it. Read from the
    /// transport reader thread, hence nonisolated(unsafe) (set-once on main).
    nonisolated(unsafe) var ndNodeID: UInt32 = 0
    nonisolated(unsafe) private var repaintTimer: Timer?
    /// Last dirty generation the repaint timer marked for display. Sentinel
    /// `.max` forces the first tick to paint even though a fresh terminal's
    /// `ndterm_dirty_seq` is 0. Read/written only on the main thread (the
    /// timer callback).
    private var lastDrawnSeq: UInt64 = .max

    /// Memoized `NSColor` by packed 0xRRGGBB (perf: the render hot loop
    /// otherwise allocates 2 NSColors per cell per frame — ≥90k/sec on a full
    /// grid at 30Hz). There are ≤256 palette colors + a default, so this
    /// saturates quickly and never grows unbounded.
    private var colorCache: [UInt32: NSColor] = [:]
    /// Memoized `CGGlyph` per ASCII codepoint for the regular/bold primary
    /// faces, so a coalesced run draws with `CTFontDrawGlyphs` (explicit
    /// per-cell positions, bypassing the per-cell `NSStringDrawingEngine`
    /// layout the perf scout measured as the dominant cost).
    private var regularGlyphs: [UInt8: CGGlyph] = [:]
    private var boldGlyphs: [UInt8: CGGlyph] = [:]

    /// WP polish-1 deliverable 3: per-glyph fallback for codepoints the
    /// primary font can't render (PUA Powerline separators/devicons, symbols
    /// outside the primary's coverage). Both memoized by codepoint so a
    /// full-grid redraw at ~30Hz never re-probes CTFontGetGlyphsForCharacters
    /// for a codepoint it's already resolved. ASCII never reaches either —
    /// see `resolvedFont`.
    private var fallbackFonts: [UInt32: NSFont] = [:]
    private var primaryCovers: Set<UInt32> = []

    /// WP polish-1 deliverable 6: mouse-button state for gating drag/hover
    /// motion reports (NDTERM_MOUSE_BUTTON vs NDTERM_MOUSE_ANY).
    private var mouseButtonDown = false
    private var mouseLastButton: Int32 = 0

    /// WP-A1/A3 selection: `selecting` is true while a left drag builds a
    /// selection; `hasSelection` mirrors the last emitted onSelectionChanged so
    /// we only fire on a real transition. `pasteCounter` uniquifies temp PNGs.
    private var selecting = false
    private var hasSelection = false
    private static var pasteCounter = 0

    /// Cell width is the advance of a representative ASCII glyph, NOT
    /// `NSFont.maximumAdvancement` — that's the widest glyph in the WHOLE
    /// font, and a Nerd Font's huge Powerline/devicon glyph set can push it
    /// many times wider than an actual monospace character (verified: a
    /// GeistMono Nerd Font cell otherwise blows up to ~90pt). `defaultLineHeight`
    /// is the font's natural row pitch. Ceil both to keep cell rects on
    /// integral pixels. `fontFamily` unset (or not resolvable by name) falls
    /// back to the system monospace face — never hardcodes Menlo.
    private static func metrics(_ fontSize: Int, _ fontFamily: String?) -> (NSFont, NSFont, CGFloat, CGFloat) {
        let f = fontFamily.flatMap { NSFont(name: $0, size: CGFloat(fontSize)) }
            ?? NSFont.monospacedSystemFont(ofSize: CGFloat(fontSize), weight: .regular)
        let bold = NSFontManager.shared.convert(f, toHaveTrait: .boldFontMask)
        let cellWidth = ("M" as NSString).size(withAttributes: [.font: f]).width
        return (f, bold, ceil(cellWidth), ceil(NSLayoutManager().defaultLineHeight(for: f)))
    }

    init(command: String?, cwd: String?, fontSize: Int, fontFamily: String?, palette: String?, foreground: String, background: String, cols: Int, rows: Int) {
        (self.font, self.boldFont, self.cellW, self.cellH) = Self.metrics(fontSize, fontFamily)
        self.cols = max(1, cols)
        self.rows = max(1, rows)
        self.isRemote = false
        self.rt = nil
        self.term = nil
        super.init(frame: NSRect(x: 0, y: 0,
                                 width: CGFloat(self.cols) * cellW,
                                 height: CGFloat(self.rows) * cellH))

        // Open after super.init so `self` can be the effect userdata (title/bell/
        // child-exit). `command`/`cwd` are optional; a nil pointer tells the core
        // to use $SHELL / inherit cwd. Transient C strings outlive the sync call.
        let ud = Unmanaged.passUnretained(self).toOpaque()
        let c = UInt16(self.cols)
        let r = UInt16(self.rows)
        switch (command, cwd) {
        case let (cmd?, wd?):
            self.term = withTerminalOpenOpts(palette: palette, foreground: foreground, background: background) { opts in
                cmd.withCString { cp in wd.withCString { wp in ndterm_open_ex(c, r, cp, wp, opts, ndTerminalEffectCb, ud) } }
            }
        case let (cmd?, nil):
            self.term = withTerminalOpenOpts(palette: palette, foreground: foreground, background: background) { opts in
                cmd.withCString { cp in ndterm_open_ex(c, r, cp, nil, opts, ndTerminalEffectCb, ud) }
            }
        case let (nil, wd?):
            self.term = withTerminalOpenOpts(palette: palette, foreground: foreground, background: background) { opts in
                wd.withCString { wp in ndterm_open_ex(c, r, nil, wp, opts, ndTerminalEffectCb, ud) }
            }
        case (nil, nil):
            self.term = withTerminalOpenOpts(palette: palette, foreground: foreground, background: background) { opts in
                ndterm_open_ex(c, r, nil, nil, opts, ndTerminalEffectCb, ud)
            }
        }

        registerForDraggedTypes([.png, .tiff, .fileURL])
        startRepaint()
    }

    /// Remote view: the grid is fed by the byte-plane transport (ndremote).
    /// `remote` is the overload disambiguator (always true here).
    init(remote: Bool, host: String?, port: Int, sessionId: String?, ticket: String?, fontSize: Int, fontFamily: String?, palette: String?, foreground: String, background: String, cols: Int, rows: Int) {
        (self.font, self.boldFont, self.cellW, self.cellH) = Self.metrics(fontSize, fontFamily)
        self.cols = max(1, cols)
        self.rows = max(1, rows)
        self.isRemote = true
        self.rt = nil
        self.term = nil
        super.init(frame: NSRect(x: 0, y: 0,
                                 width: CGFloat(self.cols) * cellW,
                                 height: CGFloat(self.rows) * cellH))

        let ud = Unmanaged.passUnretained(self).toOpaque()
        let c = UInt16(self.cols)
        let r = UInt16(self.rows)
        let handle = withTerminalOpenOpts(palette: palette, foreground: foreground, background: background) { opts in
            (host ?? "127.0.0.1").withCString { h in
                (sessionId ?? "").withCString { s in
                    (ticket ?? "").withCString { t in
                        ndrt_open_ex(h, UInt16(truncatingIfNeeded: port), s, t, c, r, opts, ndTerminalEffectCb, ndTerminalStateCb, ud)
                    }
                }
            }
        }
        self.rt = handle
        self.term = handle.map { ndrt_terminal($0) } ?? nil

        registerForDraggedTypes([.png, .tiff, .fileURL])
        startRepaint()
    }

    // The core mutates the grid on its own reader thread; there's no push
    // signal, so poll at 30 Hz — but only invalidate when the core's dirty
    // generation actually advanced (perf: the old unconditional needsDisplay
    // burned ~50-65% of a core redrawing an unchanged idle prompt). The tick
    // itself is just a lock-free atomic read + compare, so an idle terminal
    // sits at ~0% CPU. Weak self ⇒ the timer doesn't keep the view alive;
    // `deinit` invalidates it.
    private func startRepaint() {
        repaintTimer = Timer.scheduledTimer(withTimeInterval: 1.0 / 30.0, repeats: true) { [weak self] _ in
            guard let self, let t = self.term else { return }
            let seq = ndterm_dirty_seq(t)
            guard seq != self.lastDrawnSeq else { return }
            self.lastDrawnSeq = seq
            self.needsDisplay = true
        }
    }

    required init?(coder: NSCoder) { fatalError("NDTerminalView is not NSCoding-decodable") }

    deinit {
        repaintTimer?.invalidate()
        if isRemote {
            if let r = rt { ndrt_close(r) } // closes the virtual ndterm too
        } else if let t = term {
            ndterm_close(t)
        }
    }

    // Top-origin grid; opaque (every pixel is painted by the bg fill below);
    // key input needs focus.
    override var isFlipped: Bool { true }
    override var isOpaque: Bool { true }
    override var acceptsFirstResponder: Bool { true }

    override var intrinsicContentSize: NSSize {
        NSSize(width: CGFloat(cols) * cellW, height: CGFloat(rows) * cellH)
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        // mouseMoved: only reaches a view when its window opts in.
        window?.acceptsMouseMovedEvents = true
    }

    // MARK: - tuple bridging

    /// `nd_term_cell.utf8` imports as a 16-tuple of `CChar`; reinterpret its
    /// bytes as UTF-8 up to the first NUL. Empty tuple ⇒ blank cell ⇒ "".
    private func graphemeString(_ cell: nd_term_cell) -> String {
        withUnsafeBytes(of: cell.utf8) { raw in
            let bytes = raw.bindMemory(to: UInt8.self)
            var n = 0
            while n < bytes.count && bytes[n] != 0 { n += 1 }
            return n == 0 ? "" : String(decoding: bytes[0..<n], as: UTF8.self)
        }
    }

    /// `nd_term_cell.fg`/`.bg` import as 3-tuples of `UInt8` (resolved sRGB),
    /// memoized by packed 0xRRGGBB so a redraw doesn't re-allocate the same few
    /// palette colors thousands of times.
    private func nsColor(_ rgb: (UInt8, UInt8, UInt8)) -> NSColor {
        let key = UInt32(rgb.0) << 16 | UInt32(rgb.1) << 8 | UInt32(rgb.2)
        if let c = colorCache[key] { return c }
        let c = NSColor(srgbRed: CGFloat(rgb.0) / 255.0,
                        green: CGFloat(rgb.1) / 255.0,
                        blue: CGFloat(rgb.2) / 255.0,
                        alpha: 1.0)
        colorCache[key] = c
        return c
    }

    /// CGGlyph for an ASCII byte on the given primary face, memoized. Returns
    /// nil when the face has no glyph for the codepoint (shouldn't happen for
    /// printable ASCII on a monospace face, but keeps the run path honest — a
    /// miss falls back to the per-cell substitute draw).
    private func asciiGlyph(_ byte: UInt8, bold: Bool) -> CGGlyph? {
        if bold, let g = boldGlyphs[byte] { return g }
        if !bold, let g = regularGlyphs[byte] { return g }
        let face = bold ? boldFont : font
        var unichar: [UniChar] = [UniChar(byte)]
        var glyph = CGGlyph(0)
        let ok = CTFontGetGlyphsForCharacters(face, &unichar, &glyph, 1)
        guard ok, glyph != 0 else { return nil }
        if bold { boldGlyphs[byte] = glyph } else { regularGlyphs[byte] = glyph }
        return glyph
    }

    /// Resolves the font to draw `grapheme` with, given the primary/bold face
    /// already chosen for this cell's SGR bold flag. Probes
    /// CTFontGetGlyphsForCharacters only for non-ASCII codepoints not already
    /// memoized; on a missing glyph, CTFontCreateForString searches every
    /// installed font (including Nerd Fonts for PUA codepoints — unlike the
    /// automatic script-based cascade plain `NSString.draw` would use, which
    /// can't route PUA codepoints anywhere) and the result is cached by
    /// codepoint. Returns (font, isSubstitute); the caller clips a substitute
    /// draw to the owned cell rect (see `draw`) since a mis-metric'd
    /// fallback face could otherwise smear into the next cell.
    private func resolvedFont(for grapheme: String, primary: NSFont) -> (NSFont, Bool) {
        guard let scalar = grapheme.unicodeScalars.first, scalar.value >= 0x80 else { return (primary, false) }
        let cp = scalar.value
        if primaryCovers.contains(cp) { return (primary, false) }
        if let cached = fallbackFonts[cp] { return (cached, true) }

        let utf16 = Array(grapheme.utf16)
        guard !utf16.isEmpty else { return (primary, false) }
        var glyphs = [CGGlyph](repeating: 0, count: utf16.count)
        let covered = utf16.withUnsafeBufferPointer { u in
            glyphs.withUnsafeMutableBufferPointer { g in
                CTFontGetGlyphsForCharacters(font, u.baseAddress!, g.baseAddress!, u.count)
            }
        }
        if covered {
            primaryCovers.insert(cp)
            return (primary, false)
        }
        let substitute = CTFontCreateForString(font, grapheme as CFString, CFRange(location: 0, length: utf16.count)) as NSFont
        fallbackFonts[cp] = substitute
        return (substitute, true)
    }

    // MARK: - render

    override func draw(_ dirtyRect: NSRect) {
        guard let t = term else {
            NSColor.black.setFill()
            bounds.fill()
            return
        }

        // Scrollback indicator state, queried BEFORE the render lock:
        // ndterm_scrollback_state takes the terminal mutex itself, and
        // render_lock keeps that same (non-recursive) mutex held until
        // render_unlock — calling it inside the locked region self-deadlocks
        // the UI thread. One frame of staleness in an advisory thumb is fine.
        var sbTotal = 0, sbOffset = 0
        let sbPinned = ndterm_scrollback_state(t, &sbTotal, nil, &sbOffset)

        // Snapshot the viewport under the core's mutex; every read below is
        // valid only until `ndterm_render_unlock`.
        var lcols: UInt16 = 0
        var lrows: UInt16 = 0
        ndterm_render_lock(t, &lcols, &lrows)

        var defFg: [UInt8] = [0, 0, 0]
        var defBg: [UInt8] = [0, 0, 0]
        ndterm_default_colors(t, &defFg, &defBg)
        let defaultFg = nsColor((defFg[0], defFg[1], defFg[2]))
        let defaultBg = nsColor((defBg[0], defBg[1], defBg[2]))

        defaultBg.setFill()
        bounds.fill()

        let ctx = NSGraphicsContext.current?.cgContext
        let defBgKey = UInt32(defBg[0]) << 16 | UInt32(defBg[1]) << 8 | UInt32(defBg[2])

        var cell = nd_term_cell()
        for y in 0..<Int(lrows) {
            // Pass 1 — backgrounds, before any glyph so a coalesced fill can't
            // overpaint the text on top of it. Contiguous cells sharing a
            // non-default bg collapse into one fill; a default-bg cell needs no
            // fill at all (the whole-view fill above already covers it).
            var bgRunStart = 0
            var bgRunCols = 0
            var bgRunColor: NSColor = defaultBg
            var bgRunKey: UInt32 = defBgKey
            var x = 0
            while x < Int(lcols) {
                ndterm_cell(t, UInt16(x), UInt16(y), &cell)
                let flags = UInt32(cell.flags)
                if (flags & NDTERM_FLAG_WIDE_TAIL) != 0 { x += 1; continue }
                let span = (flags & NDTERM_FLAG_WIDE) != 0 ? 2 : 1
                // A selected cell swaps fg/bg like INVERSE; the two compose by XOR.
                let inverse = ((flags & NDTERM_FLAG_INVERSE) != 0) != ((flags & NDTERM_FLAG_SELECTED) != 0)
                let bgTuple = inverse ? cell.fg : cell.bg
                let bgKey = UInt32(bgTuple.0) << 16 | UInt32(bgTuple.1) << 8 | UInt32(bgTuple.2)
                if bgKey == defBgKey {
                    if bgRunCols != 0 { fillBgRun(bgRunStart, y, bgRunCols, bgRunColor); bgRunCols = 0 }
                } else if bgRunCols != 0, bgKey == bgRunKey {
                    bgRunCols += span
                } else {
                    if bgRunCols != 0 { fillBgRun(bgRunStart, y, bgRunCols, bgRunColor) }
                    bgRunStart = x
                    bgRunCols = span
                    bgRunKey = bgKey
                    bgRunColor = nsColor(bgTuple)
                }
                x += span
            }
            if bgRunCols != 0 { fillBgRun(bgRunStart, y, bgRunCols, bgRunColor) }

            // Pass 2 — foreground. Coalesce contiguous same-attr printable-ASCII
            // cells into one `CTFontDrawGlyphs` run with explicit per-cell
            // positions (no per-cell NSStringDrawing layout, no advance drift);
            // non-ASCII / wide / substitute cells keep the a45f481 per-cell
            // fallback + clipping path exactly.
            var runGlyphs: [CGGlyph] = []
            var runStart = 0
            var runBold = false
            var runUnderline = false
            var runFg: NSColor = defaultFg
            var runFgKey: UInt32 = 0
            func flushRun() {
                guard !runGlyphs.isEmpty else { return }
                drawGlyphRun(ctx, glyphs: runGlyphs, startCol: runStart, row: y,
                             bold: runBold, color: runFg, underline: runUnderline)
                runGlyphs.removeAll(keepingCapacity: true)
            }
            x = 0
            while x < Int(lcols) {
                ndterm_cell(t, UInt16(x), UInt16(y), &cell)
                let flags = UInt32(cell.flags)
                if (flags & NDTERM_FLAG_WIDE_TAIL) != 0 { x += 1; continue }
                let isWide = (flags & NDTERM_FLAG_WIDE) != 0
                let span = isWide ? 2 : 1
                let bold = (flags & NDTERM_FLAG_BOLD) != 0
                let underline = (flags & NDTERM_FLAG_UNDERLINE) != 0
                let inverse = ((flags & NDTERM_FLAG_INVERSE) != 0) != ((flags & NDTERM_FLAG_SELECTED) != 0)
                let fgTuple = inverse ? cell.bg : cell.fg
                let fgKey = UInt32(fgTuple.0) << 16 | UInt32(fgTuple.1) << 8 | UInt32(fgTuple.2)

                let b0 = UInt8(bitPattern: cell.utf8.0)
                let b1 = UInt8(bitPattern: cell.utf8.1)
                let isAscii = b1 == 0 && b0 >= 0x20 && b0 <= 0x7e && !isWide

                if isAscii, let glyph = asciiGlyph(b0, bold: bold) {
                    if !runGlyphs.isEmpty,
                       bold != runBold || underline != runUnderline || fgKey != runFgKey || x != runStart + runGlyphs.count {
                        flushRun()
                    }
                    if runGlyphs.isEmpty {
                        runStart = x
                        runBold = bold
                        runUnderline = underline
                        runFg = nsColor(fgTuple)
                        runFgKey = fgKey
                    }
                    runGlyphs.append(glyph)
                } else {
                    flushRun()
                    let s = graphemeString(cell)
                    if !s.isEmpty {
                        drawCellIndividual(ctx, grapheme: s, x: x, y: y, isWide: isWide,
                                           bold: bold, underline: underline, fg: nsColor(fgTuple))
                    }
                }
                x += span
            }
            flushRun()
        }

        var cursor = nd_term_cursor()
        ndterm_cursor(t, &cursor)
        if cursor.visible != 0, Int(cursor.x) < Int(lcols), Int(cursor.y) < Int(lrows) {
            let cx = CGFloat(cursor.x) * cellW
            let cy = CGFloat(cursor.y) * cellH
            let curRect = NSRect(x: cx, y: cy, width: cellW, height: cellH)
            // Solid block: paint the cell in the default fg, then re-stamp its
            // glyph in the default bg so the character reads through the block.
            defaultFg.setFill()
            curRect.fill()
            ndterm_cell(t, cursor.x, cursor.y, &cell)
            let s = graphemeString(cell)
            if !s.isEmpty {
                (s as NSString).draw(at: NSPoint(x: cx, y: cy),
                                     withAttributes: [.font: font, .foregroundColor: defaultBg])
            }
        }

        // Scrollback indicator: a thin right-edge thumb, shown only while
        // scrolled into history (pinned == 0). Height/position track the
        // viewport's share of the total scrollable area. (State queried above,
        // before the render lock.)
        if sbPinned == 0, sbTotal > Int(lrows) {
            let trackH = bounds.height
            let totalF = CGFloat(sbTotal)
            let thumbH = max(trackH * CGFloat(Int(lrows)) / totalF, 16)
            let thumbY = (trackH - thumbH) * CGFloat(sbOffset) / max(totalF - CGFloat(Int(lrows)), 1)
            NSColor(white: 0.53, alpha: 1).setFill()
            NSRect(x: bounds.width - 4, y: thumbY, width: 3, height: thumbH).fill()
        }

        ndterm_render_unlock(t)
    }

    /// Fill one coalesced background run (`cols` cells wide) at (startCol, row).
    private func fillBgRun(_ startCol: Int, _ row: Int, _ cols: Int, _ color: NSColor) {
        color.setFill()
        NSRect(x: CGFloat(startCol) * cellW, y: CGFloat(row) * cellH,
               width: CGFloat(cols) * cellW, height: cellH).fill()
    }

    /// Draw a coalesced ASCII run as glyphs at fixed per-cell positions. The
    /// view is flipped (y-down), so the text matrix is y-flipped to keep glyphs
    /// upright and each glyph is placed at its exact `col*cellW` origin — no
    /// natural-advance drift off the grid. Underline spans the whole run.
    private func drawGlyphRun(_ ctx: CGContext?, glyphs: [CGGlyph], startCol: Int, row: Int,
                              bold: Bool, color: NSColor, underline: Bool) {
        guard let ctx, !glyphs.isEmpty else { return }
        let face = bold ? boldFont : font
        let baseline = CGFloat(row) * cellH + face.ascender
        var positions = [CGPoint]()
        positions.reserveCapacity(glyphs.count)
        for i in 0..<glyphs.count {
            positions.append(CGPoint(x: CGFloat(startCol + i) * cellW, y: baseline))
        }
        ctx.saveGState()
        ctx.setFillColor(color.cgColor)
        ctx.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
        CTFontDrawGlyphs(face as CTFont, glyphs, positions, glyphs.count, ctx)
        ctx.restoreGState()
        if underline {
            color.setFill()
            NSRect(x: CGFloat(startCol) * cellW, y: CGFloat(row) * cellH + cellH - 1,
                   width: CGFloat(glyphs.count) * cellW, height: 1).fill()
        }
    }

    /// Per-cell draw for a non-ASCII / wide / fallback-face grapheme: the exact
    /// a45f481 path — resolve a substitute font for uncovered codepoints and
    /// clip a substitute draw to the owned cell rect so a wide fallback face
    /// can't smear into the neighbor.
    private func drawCellIndividual(_ ctx: CGContext?, grapheme s: String, x: Int, y: Int,
                                    isWide: Bool, bold: Bool, underline: Bool, fg: NSColor) {
        let primaryFace = bold ? boldFont : font
        let (drawFont, isSubstitute) = resolvedFont(for: s, primary: primaryFace)
        var attrs: [NSAttributedString.Key: Any] = [.font: drawFont, .foregroundColor: fg]
        if underline { attrs[.underlineStyle] = NSUnderlineStyle.single.rawValue }
        let rect = NSRect(x: CGFloat(x) * cellW, y: CGFloat(y) * cellH,
                          width: isWide ? cellW * 2 : cellW, height: cellH)
        if isSubstitute {
            ctx?.saveGState()
            ctx?.clip(to: rect)
            (s as NSString).draw(at: NSPoint(x: rect.minX, y: rect.minY), withAttributes: attrs)
            ctx?.restoreGState()
        } else {
            (s as NSString).draw(at: NSPoint(x: rect.minX, y: rect.minY), withAttributes: attrs)
        }
    }

    // MARK: - resize

    /// Reconciles the grid to the view's CURRENT frame size (cols/rows +
    /// ndterm_resize/ndrt_resize) if it differs from what the core already
    /// thinks the grid is. Shared by `setFrameSize` (fires per-resize) and
    /// `layout` (fires on every Auto Layout pass, including the first — the
    /// backstop that makes sure the post-construction "grow to fill the split
    /// pane" resize the container's fill/expand constraints produce actually
    /// reaches ndterm instead of leaving the grid stuck at the session-create
    /// default). `invalidateIntrinsicContentSize` keeps Auto Layout's cached
    /// understanding of this view's natural size in sync — without it, a
    /// state-dependent `intrinsicContentSize` like this one goes stale the
    /// moment cols/rows change underneath it.
    private func syncGridToViewSize() {
        guard let t = term else { return }
        let newCols = max(1, Int(bounds.width / cellW))
        let newRows = max(1, Int(bounds.height / cellH))
        guard newCols != cols || newRows != rows else { return }
        cols = newCols
        rows = newRows
        if isRemote, let r = rt {
            ndrt_resize(r, UInt16(cols), UInt16(rows)) // local grid + RESIZE frame
        } else {
            ndterm_resize(t, UInt16(cols), UInt16(rows))
        }
        invalidateIntrinsicContentSize()
        needsDisplay = true
    }

    /// Autoresizing/Auto Layout (the view is pinned to fill its slot) drives
    /// this; the create-time frame produces the same cols/rows, so no
    /// spurious resize fires at construction.
    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        syncGridToViewSize()
    }

    /// Backstop for the first real Auto Layout pass (see `syncGridToViewSize`
    /// doc): `layout()` fires with `bounds` already resolved to this pass's
    /// final geometry, regardless of what intermediate size(s) `setFrameSize`
    /// may have been called with while the constraint solver converged.
    override func layout() {
        super.layout()
        syncGridToViewSize()
    }

    // MARK: - input

    override func keyDown(with event: NSEvent) {
        guard let t = term else { return }

        // Cmd+C/V/A: copy the selection / paste the clipboard / select all.
        if event.modifierFlags.contains(.command) {
            switch event.charactersIgnoringModifiers {
            case "c": copySelection(); return
            case "v": pasteClipboard(); return
            case "a": ndterm_selection_all(t); needsDisplay = true; emitSelectionChanged(); return
            default: break
            }
        }

        // Shift+PageUp/Down/Home/End: move the scrollback viewport. Home/End jump
        // to the extremes via a large delta the core clamps.
        if event.modifierFlags.contains(.shift), let sp = event.specialKey {
            switch sp {
            case .pageUp: ndterm_scroll_viewport(t, -Int32(rows)); needsDisplay = true; return
            case .pageDown: ndterm_scroll_viewport(t, Int32(rows)); needsDisplay = true; return
            case .home: ndterm_scroll_viewport(t, -1_000_000); needsDisplay = true; return
            case .end: ndterm_scroll_viewport(t, 1_000_000); needsDisplay = true; return
            default: break
            }
        }

        let bytes = keyBytes(for: event)
        guard !bytes.isEmpty else { return }
        bytes.withUnsafeBufferPointer { buf in
            if let base = buf.baseAddress {
                ndterm_write_input(t, base, buf.count)
            }
        }
    }

    /// Map an NSEvent to the byte sequence a PTY expects. Special keys and
    /// control combinations resolve to their control/escape sequences; anything
    /// else falls through to the event's own UTF-8 characters (letters, digits,
    /// symbols, Option-composed glyphs).
    private func keyBytes(for event: NSEvent) -> [UInt8] {
        if let sp = event.specialKey {
            switch sp {
            case .carriageReturn, .enter: return [0x0d]
            case .delete: return [0x7f]                        // Backspace
            case .deleteForward: return [0x1b, 0x5b, 0x33, 0x7e]
            case .tab: return [0x09]
            case .backTab: return [0x1b, 0x5b, 0x5a]           // Shift-Tab (CBT)
            case .upArrow: return [0x1b, 0x5b, 0x41]
            case .downArrow: return [0x1b, 0x5b, 0x42]
            case .rightArrow: return [0x1b, 0x5b, 0x43]
            case .leftArrow: return [0x1b, 0x5b, 0x44]
            case .home: return [0x1b, 0x5b, 0x48]
            case .end: return [0x1b, 0x5b, 0x46]
            case .pageUp: return [0x1b, 0x5b, 0x35, 0x7e]
            case .pageDown: return [0x1b, 0x5b, 0x36, 0x7e]
            default: break
            }
        }

        if event.keyCode == 53 { return [0x1b] }               // Escape (no specialKey case)

        // Ctrl+<key> → the low-5-bits control byte (Ctrl-C = 0x03, Ctrl-Space =
        // NUL, etc.). Uses characters-ignoring-modifiers so the base key, not
        // its shifted form, drives the mapping.
        if event.modifierFlags.contains(.control),
           let base = event.charactersIgnoringModifiers,
           let scalar = base.unicodeScalars.first, scalar.value < 0x80 {
            return [UInt8(scalar.value) & 0x1f]
        }

        if let chars = event.characters, !chars.isEmpty {
            return Array(chars.utf8)
        }
        return []
    }

    // MARK: - mouse / scroll (WP polish-1 deliverable 6)

    /// Cell coordinate under `event`, clamped to the grid.
    private func cellCoord(for event: NSEvent) -> (col: UInt16, row: UInt16) {
        let p = convert(event.locationInWindow, from: nil)
        let col = max(0, min(cols - 1, Int(p.x / cellW)))
        let row = max(0, min(rows - 1, Int(p.y / cellH)))
        return (UInt16(col), UInt16(row))
    }

    /// SGR (1006) mouse sequence: ESC [ < Cb ; Cx ; Cy M|m — M on press/
    /// motion, m on release. Cx/Cy are 1-based cell coordinates. Fed through
    /// the existing input path (ndterm_write_input), same as key bytes.
    private func sendSgrMouse(button: Int32, event: NSEvent, press: Bool) {
        guard let t = term else { return }
        let (col, row) = cellCoord(for: event)
        let final = press ? "M" : "m"
        let bytes = Array("\u{1b}[<\(button);\(col + 1);\(row + 1)\(final)".utf8)
        bytes.withUnsafeBufferPointer { buf in
            if let base = buf.baseAddress { ndterm_write_input(t, base, buf.count) }
        }
    }

    private func beginMouseButton(_ sgrButton: Int32, event: NSEvent) {
        window?.makeFirstResponder(self)
        mouseButtonDown = true
        mouseLastButton = sgrButton
        guard let t = term, ndterm_mouse_mode(t) != 0 else { return }
        sendSgrMouse(button: sgrButton, event: event, press: true)
    }

    private func endMouseButton(event: NSEvent) {
        let btn = mouseLastButton
        mouseButtonDown = false
        guard let t = term, ndterm_mouse_mode(t) != 0 else { return }
        sendSgrMouse(button: btn, event: event, press: false)
    }

    /// Hover/drag reporting, gated on the VT's active mouse modes —
    /// NDTERM_MOUSE_ANY reports every move, NDTERM_MOUSE_BUTTON reports only
    /// while a button is held (xterm 1002/1003 semantics). Always SGR-encoded
    /// regardless of which specific format bit the app asked for — a
    /// deliberate v1 simplification (mirrored on the GTK surface), safe in
    /// practice because apps that enable tracking overwhelmingly also
    /// request SGR. Selection/copy-paste are out of scope (phase 2).
    private func reportMotion(event: NSEvent) {
        guard let t = term else { return }
        let mode = ndterm_mouse_mode(t)
        let anyMotion = (mode & NDTERM_MOUSE_ANY) != 0
        let buttonMotion = mouseButtonDown && (mode & NDTERM_MOUSE_BUTTON) != 0
        guard anyMotion || buttonMotion else { return }
        let base: Int32 = mouseButtonDown ? mouseLastButton : 3 // 3 = no button held
        sendSgrMouse(button: base | 32, event: event, press: true) // +32 = motion
    }

    /// Left button drives text selection when the app isn't grabbing the mouse
    /// (mode 0), or when Shift overrides an active mouse-reporting mode — mirrors
    /// the GTK surface. clickCount drives word (2) / line (3) vs char (1).
    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        if let t = term, ndterm_mouse_mode(t) == 0 || event.modifierFlags.contains(.shift) {
            let (col, row) = cellCoord(for: event)
            if event.clickCount >= 3 {
                ndterm_selection_line(t, col, row)
            } else if event.clickCount == 2 {
                ndterm_selection_word(t, col, row)
            } else {
                ndterm_selection_begin(t, col, row)
            }
            selecting = true
            needsDisplay = true
            emitSelectionChanged()
            return
        }
        beginMouseButton(0, event: event) // SGR 0 = left
    }
    override func mouseUp(with event: NSEvent) {
        if selecting { selecting = false; emitSelectionChanged(); return }
        endMouseButton(event: event)
    }
    override func mouseDragged(with event: NSEvent) {
        if selecting, let t = term {
            let (col, row) = cellCoord(for: event)
            ndterm_selection_extend(t, col, row)
            needsDisplay = true
            emitSelectionChanged()
            return
        }
        reportMotion(event: event)
    }
    override func rightMouseDown(with event: NSEvent) { beginMouseButton(2, event: event) } // SGR 2 = right
    override func rightMouseUp(with event: NSEvent) { endMouseButton(event: event) }
    override func rightMouseDragged(with event: NSEvent) { reportMotion(event: event) }
    override func otherMouseDown(with event: NSEvent) { beginMouseButton(1, event: event) } // SGR 1 = middle
    override func otherMouseUp(with event: NSEvent) { endMouseButton(event: event) }
    override func otherMouseDragged(with event: NSEvent) { reportMotion(event: event) }
    override func mouseMoved(with event: NSEvent) { reportMotion(event: event) }

    /// Scroll wheel -> client-local scrollback viewport (ndterm_scroll_viewport,
    /// wrapping libghostty-vt's scroll_viewport — the client VT already saw
    /// every byte, so no round-trip to the daemon is needed). `scrollingDeltaY`
    /// positive = user scrolled up (reveal older content), matching
    /// ndterm_scroll_viewport's "up is negative" convention under negation.
    override func scrollWheel(with event: NSEvent) {
        guard let t = term else { return }
        let delta = Int32((-event.scrollingDeltaY / cellH).rounded())
        guard delta != 0 else { return }
        ndterm_scroll_viewport(t, delta)
        needsDisplay = true
    }

    // MARK: - selection / clipboard / image paste (WP-A3 / WP-A4 / WP-B2)

    /// Fire onSelectionChanged only when the has-selection state actually flips.
    private func emitSelectionChanged() {
        guard let t = term, ndNodeID != 0 else { return }
        let has = ndterm_selection_text(t, nil, 0) > 0
        guard has != hasSelection else { return }
        hasSelection = has
        ndEmitEvent(ndNodeID, "selectionChanged", "{\"checked\":\(has)}")
    }

    private func copySelection() {
        guard let t = term else { return }
        let need = ndterm_selection_text(t, nil, 0)
        guard need > 0 else { return }
        var buf = [UInt8](repeating: 0, count: need)
        // ndterm.h: the return may exceed buf_len if output grew the selection
        // between the size query and the read — clamp before slicing.
        let n = min(buf.withUnsafeMutableBufferPointer { ndterm_selection_text(t, $0.baseAddress, need) }, need)
        let s = String(decoding: buf[0..<n], as: UTF8.self)
        let pb = NSPasteboard.general
        pb.clearContents()
        pb.setString(s, forType: .string)
    }

    /// Probe the clipboard for an image first (WP-B2): if it holds one, save a
    /// local temp PNG and emit onImagePaste{path} instead of typing; otherwise
    /// paste text via ndterm_write_paste (bracketed-paste aware).
    private func pasteClipboard() {
        guard let t = term else { return }
        let pb = NSPasteboard.general
        if let png = pasteboardImagePng(pb), let path = writeTempPng(png) {
            emitImagePaste(path)
            return
        }
        guard let s = pb.string(forType: .string) else { return }
        let bytes = Array(s.utf8)
        bytes.withUnsafeBufferPointer { if let b = $0.baseAddress { ndterm_write_paste(t, b, bytes.count) } }
    }

    /// PNG bytes for whatever image the pasteboard holds (native PNG, or a TIFF
    /// re-encoded to PNG), or nil when it has no bitmap image.
    private func pasteboardImagePng(_ pb: NSPasteboard) -> Data? {
        if let png = pb.data(forType: .png) { return png }
        if let tiff = pb.data(forType: .tiff), let rep = NSBitmapImageRep(data: tiff) {
            return rep.representation(using: .png, properties: [:])
        }
        return nil
    }

    private func writeTempPng(_ data: Data) -> String? {
        let name = "nd-clip-\(ProcessInfo.processInfo.processIdentifier)-\(Self.pasteCounter).png"
        Self.pasteCounter += 1
        let path = (NSTemporaryDirectory() as NSString).appendingPathComponent(name)
        do {
            try data.write(to: URL(fileURLWithPath: path))
            return path
        } catch {
            return nil
        }
    }

    private func emitImagePaste(_ path: String) {
        guard ndNodeID != 0 else { return }
        ndEmitEvent(ndNodeID, "imagePaste", "{\"data\":{\"path\":\(ndJsonString(path))}}")
    }

    /// Generated widgetCommand Terminal arm (WP-A4): copy/paste/selectAll/
    /// clearSelection. Internal so the free `ndTerminalCommand` shim can reach it.
    func runWidgetCommand(_ command: String) {
        guard let t = term else { return }
        switch command {
        case "copy": copySelection()
        case "paste": pasteClipboard()
        case "selectAll": ndterm_selection_all(t); needsDisplay = true; emitSelectionChanged()
        case "clearSelection": ndterm_selection_clear(t); needsDisplay = true; emitSelectionChanged()
        default: break
        }
    }

    // MARK: - drag & drop image paste (WP-B2)

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        return dropHasImage(sender) ? .copy : []
    }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        let pb = sender.draggingPasteboard
        // A dropped image file: emit its own path (no copy needed).
        if let urls = pb.readObjects(forClasses: [NSURL.self],
                                     options: [.urlReadingFileURLsOnly: true]) as? [URL],
           let url = urls.first(where: { isImagePath($0.path) }) {
            emitImagePaste(url.path)
            return true
        }
        // Dropped raw image bytes: save a temp PNG and emit its path.
        if let png = pasteboardImagePng(pb), let path = writeTempPng(png) {
            emitImagePaste(path)
            return true
        }
        return false
    }

    private func dropHasImage(_ sender: NSDraggingInfo) -> Bool {
        let pb = sender.draggingPasteboard
        if pb.data(forType: .png) != nil || pb.data(forType: .tiff) != nil { return true }
        if let urls = pb.readObjects(forClasses: [NSURL.self],
                                     options: [.urlReadingFileURLsOnly: true]) as? [URL] {
            return urls.contains { isImagePath($0.path) }
        }
        return false
    }

    private func isImagePath(_ path: String) -> Bool {
        let ext = (path as NSString).pathExtension.lowercased()
        return ["png", "jpg", "jpeg", "gif", "bmp", "tiff", "tif", "webp"].contains(ext)
    }
}

// MARK: - WP polish-1 deliverable 7: palette/fg/bg open-time options

/// `#rrggbb` -> (r, g, b), nil on any malformed input.
private func parseHexColor(_ s: String) -> (UInt8, UInt8, UInt8)? {
    guard s.count == 7, s.first == "#", let v = UInt32(s.dropFirst(), radix: 16) else { return nil }
    return (UInt8((v >> 16) & 0xff), UInt8((v >> 8) & 0xff), UInt8(v & 0xff))
}

private let ndPaletteColors = 256

/// The standard xterm 256-color cube (16-231) + grayscale ramp (232-255),
/// used to fill the entries a caller-supplied 16-color palette doesn't cover
/// — ndterm's open-time palette override is all-or-nothing (exactly
/// ndPaletteColors*3 bytes, see include/ndterm.h `nd_term_open_opts`).
private func xtermStandardColor(_ index: Int) -> (UInt8, UInt8, UInt8) {
    if index < 16 { return (0, 0, 0) } // caller always supplies 0-15
    if index < 232 {
        let n = index - 16
        let levels: [UInt8] = [0, 95, 135, 175, 215, 255]
        return (levels[n / 36], levels[(n / 6) % 6], levels[n % 6])
    }
    let gray = UInt8(8 + 10 * (index - 232))
    return (gray, gray, gray)
}

/// Parses the `palette` prop (docs/widgets.md: comma-separated `#rrggbb`, 16
/// or 256 entries, ANSI index order) into exactly ndPaletteColors*3 packed
/// rgb bytes. A 16-entry palette gets the standard xterm cube/grayscale for
/// indices 16-255. Returns nil on a malformed entry or a count that's
/// neither 16 nor 256.
private func parsePalette(_ s: String) -> [UInt8]? {
    let tokens = s.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }
    guard tokens.count == 16 || tokens.count == ndPaletteColors else { return nil }
    var colors: [(UInt8, UInt8, UInt8)] = []
    colors.reserveCapacity(tokens.count)
    for t in tokens {
        guard let c = parseHexColor(t) else { return nil }
        colors.append(c)
    }
    var out = [UInt8](repeating: 0, count: ndPaletteColors * 3)
    for i in 0..<ndPaletteColors {
        let c = i < colors.count ? colors[i] : xtermStandardColor(i)
        out[i * 3 + 0] = c.0
        out[i * 3 + 1] = c.1
        out[i * 3 + 2] = c.2
    }
    return out
}

/// Threads a `palette`/`foreground`/`background`-derived `nd_term_open_opts`
/// through to a C ABI open call. The pointer is valid only inside `body` —
/// ndterm_open_ex/ndrt_open_ex only read `opts` during the call, never
/// retain it (include/ndterm.h `nd_term_open_opts`).
private func withTerminalOpenOpts<R>(palette: String?, foreground: String, background: String, _ body: (UnsafePointer<nd_term_open_opts>?) -> R) -> R {
    let fg = parseHexColor(foreground) ?? (0xcc, 0xcc, 0xcc)
    let bg = parseHexColor(background) ?? (0x00, 0x00, 0x00)
    if let bytes = palette.flatMap(parsePalette) {
        return bytes.withUnsafeBufferPointer { buf in
            var opts = nd_term_open_opts(palette_rgb: buf.baseAddress, palette_len: buf.count, has_fg: 1, fg: fg, has_bg: 1, bg: bg)
            return withUnsafePointer(to: &opts) { body($0) }
        }
    }
    var opts = nd_term_open_opts(palette_rgb: nil, palette_len: 0, has_fg: 1, fg: fg, has_bg: 1, bg: bg)
    return withUnsafePointer(to: &opts) { body($0) }
}

// MARK: - effect / connection-state trampolines (C ABI)

/// Records the node id so the effect/state trampolines can emit (generated
/// `ndConnectEvents` Terminal arm). Peer of `ndWebViewConnect`.
func ndTerminalConnect(_ view: NSView, nodeID: UInt32) {
    guard let tv = view as? NDTerminalView else { return }
    tv.ndNodeID = nodeID
}

/// Generated `ndWidgetCommand` Terminal arm (WP-A4): copy/paste/selectAll/
/// clearSelection. `argJson` is unused (these commands take no argument).
func ndTerminalCommand(_ view: NSView, _ command: String, _ argJson: String) {
    guard let tv = view as? NDTerminalView else { return }
    tv.runWidgetCommand(command)
}

/// Emit an NDP event for a terminal view. `udata` is the unretained view passed
/// as the ndterm/ndremote userdata. Fires on a reader thread; the actual emit is
/// marshaled to the main queue, capturing only value types (never the view).
private func ndTerminalEmit(_ udata: UnsafeMutableRawPointer?, _ name: String, _ json: String) {
    guard let udata = udata else { return }
    let nodeID = Unmanaged<NDTerminalView>.fromOpaque(udata).takeUnretainedValue().ndNodeID
    guard nodeID != 0 else { return }
    DispatchQueue.main.async { ndEmitEvent(nodeID, name, json) }
}

/// `nd_term_effect_cb` — kind: 0 title, 1 bell, 2 child-exit (code).
func ndTerminalEffectCb(_ udata: UnsafeMutableRawPointer?, _ kind: Int32, _ text: UnsafePointer<CChar>?, _ code: Int32) {
    switch kind {
    case 0:
        let title = text.map { String(cString: $0) } ?? ""
        ndTerminalEmit(udata, "titleChanged", "{\"text\":\(ndJsonString(title))}")
    case 1:
        ndTerminalEmit(udata, "bell", "{}")
    case 2:
        ndTerminalEmit(udata, "exited", "{\"data\":{\"code\":\(code)}}")
    default:
        break
    }
}

/// `nd_rt_state_cb` — connection-state transitions (nd_rt_state + optional detail).
func ndTerminalStateCb(_ udata: UnsafeMutableRawPointer?, _ state: Int32, _ detail: UnsafePointer<CChar>?) {
    let detailJson = detail.map { ",\"detail\":\(ndJsonString(String(cString: $0)))" } ?? ""
    ndTerminalEmit(udata, "connectionState", "{\"data\":{\"state\":\(state)\(detailJson)}}")
}
