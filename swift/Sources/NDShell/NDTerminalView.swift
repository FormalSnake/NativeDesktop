import AppKit
import Foundation
import CNd

/// AppKit rendering + input surface for the `<terminal>` widget (peer of the
/// GTK surface in `src/gtk`). It owns nothing terminal-shaped itself: the PTY,
/// libghostty-vt instance, and grid state all live behind the `ndterm` C ABI
/// (`include/ndterm.h`, imported via `CNd`). This view opens a handle at
/// create, drives `ndterm_render_lock`/`_cell`/`_cursor` inside `draw`, maps
/// key events to bytes for `ndterm_write_input`, tracks pixel size onto the
/// grid via `ndterm_resize`, and closes the handle on `deinit`.
///
/// Phase A: no effect callback is registered (title/bell/child-exit events are
/// deferred), so `ndterm_open` gets `nil`/`nil` for `cb`/`userdata`.
///
/// Flipped like the rest of the shell (`FlippedView`, `NDPaneHostView`): the
/// grid is top-origin — row 0 is the top row — so a top-left y-down coordinate
/// space lets cell (x, y) draw at `(x*cellW, y*cellH)` directly, and AppKit's
/// string drawing stays right-side-up in a flipped view.
final class NDTerminalView: NSView {
    /// `nd_terminal *` (opaque to Swift). Nil only if `ndterm_open` failed.
    /// nonisolated(unsafe): the @MainActor view's nonisolated `deinit` closes the
    /// handle; teardown is single-owner so the unchecked access is safe.
    nonisolated(unsafe) private let term: OpaquePointer?
    private let font: NSFont
    private let boldFont: NSFont
    private let cellW: CGFloat
    private let cellH: CGFloat
    /// Current grid dimensions. Start from the create-time props, then track
    /// the view's pixel size in `setFrameSize` (→ `ndterm_resize`).
    private var cols: Int
    private var rows: Int
    nonisolated(unsafe) private var repaintTimer: Timer?

    init(command: String?, cwd: String?, fontSize: Int, cols: Int, rows: Int) {
        let f = NSFont(name: "Menlo", size: CGFloat(fontSize))
            ?? NSFont.monospacedSystemFont(ofSize: CGFloat(fontSize), weight: .regular)
        self.font = f
        self.boldFont = NSFontManager.shared.convert(f, toHaveTrait: .boldFontMask)
        // Monospace ⇒ every glyph shares one advance, so the max advance IS the
        // cell width; `defaultLineHeight` is the font's natural row pitch.
        // Ceil both to keep cell rects on integral pixels.
        self.cellW = ceil(f.maximumAdvancement.width)
        self.cellH = ceil(NSLayoutManager().defaultLineHeight(for: f))
        self.cols = max(1, cols)
        self.rows = max(1, rows)

        let c = UInt16(self.cols)
        let r = UInt16(self.rows)
        // `command`/`cwd` are optional; a nil pointer tells the core to use
        // $SHELL / inherit cwd. `ndterm_open` spawns synchronously, so the
        // transient C strings from `withCString` outlive the call.
        var handle: OpaquePointer?
        switch (command, cwd) {
        case let (cmd?, wd?):
            handle = cmd.withCString { cp in wd.withCString { wp in ndterm_open(c, r, cp, wp, nil, nil) } }
        case let (cmd?, nil):
            handle = cmd.withCString { cp in ndterm_open(c, r, cp, nil, nil, nil) }
        case let (nil, wd?):
            handle = wd.withCString { wp in ndterm_open(c, r, nil, wp, nil, nil) }
        case (nil, nil):
            handle = ndterm_open(c, r, nil, nil, nil, nil)
        }
        self.term = handle

        super.init(frame: NSRect(x: 0, y: 0,
                                 width: CGFloat(self.cols) * cellW,
                                 height: CGFloat(self.rows) * cellH))

        // The core mutates the grid on its own reader thread; there's no push
        // signal in Phase A, so poll-repaint at 30 Hz. Weak self ⇒ the timer
        // doesn't keep the view alive; `deinit` invalidates it.
        repaintTimer = Timer.scheduledTimer(withTimeInterval: 1.0 / 30.0, repeats: true) { [weak self] _ in
            self?.needsDisplay = true
        }
    }

    required init?(coder: NSCoder) { fatalError("NDTerminalView is not NSCoding-decodable") }

    deinit {
        repaintTimer?.invalidate()
        if let t = term { ndterm_close(t) }
    }

    // Top-origin grid; opaque (every pixel is painted by the bg fill below);
    // key input needs focus.
    override var isFlipped: Bool { true }
    override var isOpaque: Bool { true }
    override var acceptsFirstResponder: Bool { true }

    override var intrinsicContentSize: NSSize {
        NSSize(width: CGFloat(cols) * cellW, height: CGFloat(rows) * cellH)
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

    /// `nd_term_cell.fg`/`.bg` import as 3-tuples of `UInt8` (resolved sRGB).
    private func nsColor(_ rgb: (UInt8, UInt8, UInt8)) -> NSColor {
        NSColor(srgbRed: CGFloat(rgb.0) / 255.0,
                green: CGFloat(rgb.1) / 255.0,
                blue: CGFloat(rgb.2) / 255.0,
                alpha: 1.0)
    }

    // MARK: - render

    override func draw(_ dirtyRect: NSRect) {
        guard let t = term else {
            NSColor.black.setFill()
            bounds.fill()
            return
        }

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

        var cell = nd_term_cell()
        for y in 0..<Int(lrows) {
            for x in 0..<Int(lcols) {
                ndterm_cell(t, UInt16(x), UInt16(y), &cell)
                let flags = UInt32(cell.flags)
                // The trailing half of a wide glyph carries no text of its own;
                // the wide cell's grapheme already overdraws into it.
                if (flags & NDTERM_FLAG_WIDE_TAIL) != 0 { continue }

                var fg = nsColor(cell.fg)
                var bg = nsColor(cell.bg)
                if (flags & NDTERM_FLAG_INVERSE) != 0 { swap(&fg, &bg) }

                let rect = NSRect(x: CGFloat(x) * cellW, y: CGFloat(y) * cellH, width: cellW, height: cellH)
                bg.setFill()
                rect.fill()

                let s = graphemeString(cell)
                if !s.isEmpty {
                    var attrs: [NSAttributedString.Key: Any] = [
                        .font: (flags & NDTERM_FLAG_BOLD) != 0 ? boldFont : font,
                        .foregroundColor: fg,
                    ]
                    if (flags & NDTERM_FLAG_UNDERLINE) != 0 {
                        attrs[.underlineStyle] = NSUnderlineStyle.single.rawValue
                    }
                    (s as NSString).draw(at: NSPoint(x: rect.minX, y: rect.minY), withAttributes: attrs)
                }
            }
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

        ndterm_render_unlock(t)
    }

    // MARK: - resize

    /// Track the view's pixel size onto the PTY grid. Autoresizing (the view is
    /// pinned to fill its slot) drives this; the create-time frame produces the
    /// same cols/rows, so no spurious resize fires at construction.
    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        guard let t = term else { return }
        let newCols = max(1, Int(newSize.width / cellW))
        let newRows = max(1, Int(newSize.height / cellH))
        if newCols != cols || newRows != rows {
            cols = newCols
            rows = newRows
            ndterm_resize(t, UInt16(cols), UInt16(rows))
            needsDisplay = true
        }
    }

    // MARK: - input

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
    }

    override func keyDown(with event: NSEvent) {
        guard let t = term else { return }
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
}
