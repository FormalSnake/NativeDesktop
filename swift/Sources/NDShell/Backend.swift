import AppKit
import CNd

// Must outlive main(): the core stores &gVTable and calls through it for the
// process's whole life (mirrors src/gtk/main.zig's module-level `the_vtable`
// and this file's T1 predecessor, buildStubVTable). `gWindow`/`gCtx` are
// declared in main.swift.

/// Rebuilds an `NSView` from a raw vtable handle. Handles ride the ABI as
/// retained `Unmanaged<NSView>` pointers (`create` calls `passRetained`;
/// every other op here calls `takeUnretainedValue` — the core owns the
/// retain for the node's lifetime, balanced by `unparent`/GC, same
/// contract as `src/gtk/backend.zig`'s widget handles).
@inline(__always) private func viewFrom(_ p: UnsafeMutableRawPointer?) -> NSView {
    Unmanaged<NSView>.fromOpaque(p!).takeUnretainedValue()
}
@inline(__always) private func cstr(_ p: UnsafePointer<CChar>?) -> String {
    p.map { String(cString: $0) } ?? ""
}

/// The real AppKit backend (T3), replacing T1's `buildStubVTable`. Every
/// `@convention(c)` closure decodes its raw-pointer/C-string args and calls
/// into the generated `NDGen.Widgets` dispatcher — never a narrower
/// concrete type at this layer (the generated dispatcher owns per-kind
/// casts, mirroring `src/gtk/backend.zig`'s `vt*` wrappers).
///
/// Swift 6 strict concurrency: raw pointers cross into `MainActor
/// .assumeIsolated` closures as `Int` bit patterns (capturing
/// `UnsafeMutableRawPointer` directly trips the sending-risk check even
/// though the ABI guarantees every call arrives on the UI thread already —
/// same pattern main.swift's T1 stub established). C strings are decoded to
/// Swift `String` *before* entering the isolated closure.
func buildVTable() -> nd_backend {
    var vt = nd_backend()

    vt.create = { _, kind, propsJson in
        let kindStr = cstr(kind)
        let propsStr = cstr(propsJson)
        let bits: Int? = MainActor.assumeIsolated {
            guard let v = ndCreate(kindStr, propsStr) else { return nil }
            ndApplyTestID(v, propsStr)
            ndApplyCssClassesIfPresent(v, propsStr)
            ndRecordButtonKind(v, kindStr)
            return Int(bitPattern: Unmanaged.passRetained(v).toOpaque())
        }
        guard let bits else { return nil }
        return UnsafeMutableRawPointer(bitPattern: bits)
    }

    vt.apply_props = { _, w, kind, propsJson in
        let widgetBits = Int(bitPattern: w)
        let kindStr = cstr(kind)
        let propsStr = cstr(propsJson)
        MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return }
            let view = viewFrom(widgetPtr)
            ndApplyProps(view, kindStr, propsStr)
            ndApplyTestID(view, propsStr)
            ndApplyCssClassesIfPresent(view, propsStr)
        }
    }

    vt.append_child = { _, parent, pkind, child, attachedJson in
        let parentBits = Int(bitPattern: parent)
        let childBits = Int(bitPattern: child)
        let pkindStr = cstr(pkind)
        let attachedStr = cstr(attachedJson)
        MainActor.assumeIsolated {
            guard let parentPtr = UnsafeMutableRawPointer(bitPattern: parentBits),
                  let childPtr = UnsafeMutableRawPointer(bitPattern: childBits) else { return }
            ndAppendChild(viewFrom(parentPtr), pkindStr, viewFrom(childPtr), attachedStr)
        }
    }

    vt.insert_before = { _, parent, pkind, child, before, attachedJson in
        let parentBits = Int(bitPattern: parent)
        let childBits = Int(bitPattern: child)
        let beforeBits = Int(bitPattern: before)
        let pkindStr = cstr(pkind)
        let attachedStr = cstr(attachedJson)
        MainActor.assumeIsolated {
            guard let parentPtr = UnsafeMutableRawPointer(bitPattern: parentBits),
                  let childPtr = UnsafeMutableRawPointer(bitPattern: childBits) else { return }
            let beforeView = UnsafeMutableRawPointer(bitPattern: beforeBits).map {
                Unmanaged<NSView>.fromOpaque($0).takeUnretainedValue()
            }
            ndInsertBefore(viewFrom(parentPtr), pkindStr, viewFrom(childPtr), beforeView, attachedStr)
        }
    }

    vt.remove_child = { _, parent, pkind, child in
        let parentBits = Int(bitPattern: parent)
        let childBits = Int(bitPattern: child)
        let pkindStr = cstr(pkind)
        MainActor.assumeIsolated {
            guard let parentPtr = UnsafeMutableRawPointer(bitPattern: parentBits),
                  let childPtr = UnsafeMutableRawPointer(bitPattern: childBits) else { return }
            ndRemoveChild(viewFrom(parentPtr), pkindStr, viewFrom(childPtr))
        }
    }

    vt.set_text = { _, w, text in
        let widgetBits = Int(bitPattern: w)
        let textStr = cstr(text)
        MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return }
            if let f = viewFrom(widgetPtr) as? NSTextField { f.stringValue = textStr }
        }
    }

    vt.set_visible = { _, w, visible in
        let widgetBits = Int(bitPattern: w)
        MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return }
            viewFrom(widgetPtr).isHidden = !visible
        }
    }

    vt.apply_style = { _, w, nodeID, styleJson in
        let widgetBits = Int(bitPattern: w)
        let styleStr = cstr(styleJson)
        MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return }
            ndApplyStyle(viewFrom(widgetPtr), nodeID, styleStr)
        }
    }

    vt.connect_events = { _, w, kind, nodeID in
        let widgetBits = Int(bitPattern: w)
        let kindStr = cstr(kind)
        MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return }
            ndConnectEvents(viewFrom(widgetPtr), kindStr, nodeID)
        }
    }

    vt.has_parent = { _, w in
        let widgetBits = Int(bitPattern: w)
        return MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return false }
            return viewFrom(widgetPtr).superview != nil
        }
    }

    vt.unparent = { _, w in
        let widgetBits = Int(bitPattern: w)
        MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return }
            viewFrom(widgetPtr).removeFromSuperview()
        }
    }

    vt.get_window = { _ in
        let bits: Int? = MainActor.assumeIsolated {
            guard let content = gWindow?.contentView else { return nil }
            return Int(bitPattern: Unmanaged.passUnretained(content).toOpaque())
        }
        guard let bits else { return nil }
        return UnsafeMutableRawPointer(bitPattern: bits)
    }

    vt.marshal_async = { _, fn, data in
        // dispatch_async_f: C fn ptr onto the main queue. NEVER dispatch_sync.
        guard let fn else { return }
        let bits = Int(bitPattern: data)
        DispatchQueue.main.async { fn(UnsafeMutableRawPointer(bitPattern: bits)) }
    }

    vt.show_overlay = { _, message in
        let messageStr = cstr(message)
        MainActor.assumeIsolated {
            ndShowOverlay(messageStr)
        }
    }

    // node_visible / node_bounds / snapshot / semantic_action (Task 5): the
    // automation half of the vtable, implemented in Automation.swift —
    // AppKit peers of src/gtk/backend.zig's vtNodeVisible/vtNodeBounds/
    // vtSnapshot/vtSemanticAction.
    vt.node_visible = { _, w in
        let widgetBits = Int(bitPattern: w)
        return MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return false }
            return ndNodeVisible(viewFrom(widgetPtr))
        }
    }

    vt.node_bounds = { _, w, out in
        let widgetBits = Int(bitPattern: w)
        let outBits = Int(bitPattern: out)
        return MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits),
                  let outPtr = UnsafeMutableRawPointer(bitPattern: outBits) else { return false }
            var rect = nd_rect()
            guard ndNodeBounds(viewFrom(widgetPtr), &rect) else { return false }
            outPtr.assumingMemoryBound(to: nd_rect.self).pointee = rect
            return true
        }
    }

    vt.snapshot = { _, pngPath in
        let pathStr = cstr(pngPath)
        return MainActor.assumeIsolated {
            ndSnapshot(pathStr)
        }
    }

    vt.semantic_action = { _, w, nodeID, action, argJson, resultOut, errOut in
        let widgetBits = Int(bitPattern: w)
        let actionStr = cstr(action)
        let argStr = cstr(argJson)
        let resultOutBits = Int(bitPattern: resultOut)
        let errOutBits = Int(bitPattern: errOut)
        return MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return -32601 }
            let resultOutPtr = UnsafeMutableRawPointer(bitPattern: resultOutBits)?
                .assumingMemoryBound(to: UnsafeMutablePointer<CChar>?.self)
            let errOutPtr = UnsafeMutableRawPointer(bitPattern: errOutBits)?
                .assumingMemoryBound(to: UnsafeMutablePointer<CChar>?.self)
            return ndSemanticAction(viewFrom(widgetPtr), nodeID, actionStr, argStr, resultOutPtr, errOutPtr)
        }
    }

    return vt
}

/// testIDs (Task 5): mirrors the tracked `testID` prop onto AppKit's own
/// accessibility identifier for real-user/VoiceOver parity. This is NOT the
/// getTree `testID` source — that flows from core-owned `Tree.meta`
/// (src/tree.zig) independent of AppKit entirely — so a missing/absent
/// testID here is a no-op, never an error. Called from both `create` (after
/// the initial build) and `apply_props` (testID can arrive/change on any
/// update, same as every other prop).
func ndApplyTestID(_ view: NSView, _ propsJson: String) {
    guard let testID = propStr(parseProps(propsJson), "testID") else { return }
    view.setAccessibilityIdentifier(testID)
}

/// cssClasses (Task 6): decodes `props.cssClasses` — Task 2's validated
/// Adwaita-class allowlist, riding in the ordinary props JSON rather than a
/// dedicated vtable field (the C-ABI vtable is frozen at 18 fields) — and
/// applies AppKit's mapped subset via `ndApplyCssClasses`. Called from both
/// `create` and `apply_props`, mirroring `ndApplyTestID`.
func ndApplyCssClassesIfPresent(_ view: NSView, _ propsJson: String) {
    guard let classes = propArray(parseProps(propsJson), "cssClasses") else { return }
    ndApplyCssClasses(view, classes)
}

/// `ndApplyCssClasses` (Task 6 — a real semantic mapping, not a no-op): maps
/// the Adwaita/GTK classes AppKit has a natural equivalent for onto control
/// properties. Every color used is a dynamic system color
/// (`.controlAccentColor`, `.secondaryLabelColor`, ...) rather than a
/// hardcoded hex value, so dark mode keeps working automatically.
///
/// TextArea/ScrollView widgets are `NSScrollView` wrappers around an
/// `NSTextView` document view — font/color classes below target that inner
/// text view, not the scroll view itself (mirrors `ndApplyStyle`'s
/// `applyTextColor`/`applyFont` helpers).
///
/// Structural classes (`navigation-sidebar`, `card`, `view`, `toolbar`,
/// `boxed-list`, `osd`, ...) are silently ignored — those roles come from
/// the SplitView/HeaderBar widgets themselves on the Mac (later M11 tasks),
/// not from class strings.
func ndApplyCssClasses(_ view: NSView, _ classes: [String]) {
    let textTarget: NSView = {
        if let scrollView = view as? NSScrollView, let textView = scrollView.documentView as? NSTextView {
            return textView
        }
        return view
    }()

    // Set-replace, not additive (mirrors GTK's applyCssClasses, which removes
    // every allowlist class not in `value`): reset the properties the switch
    // below can touch to their baseline FIRST, so a class dropped from the
    // list actually clears its effect (e.g. a former "suggested-action" no
    // longer leaves keyEquivalent="\r"/an accent bezel, a former "flat" gets
    // its border back). Baselines match a freshly-created control from
    // NDGen/Widgets.swift's create arms (.rounded bezel, no key equivalent).
    if let btn = view as? NSButton {
        btn.bezelColor = nil
        btn.keyEquivalent = ""
        btn.hasDestructiveAction = false
        btn.isBordered = true
        btn.showsBorderOnlyWhileMouseInside = false
    }
    if let field = textTarget as? NSTextField {
        field.font = .systemFont(ofSize: NSFont.systemFontSize)
        field.textColor = .labelColor
    } else if let textView = textTarget as? NSTextView {
        textView.font = .systemFont(ofSize: NSFont.systemFontSize)
        textView.textColor = .labelColor
    }

    for cls in classes {
        switch cls {
        case "suggested-action":
            guard let btn = view as? NSButton else { continue }
            btn.bezelColor = .controlAccentColor
            btn.keyEquivalent = "\r"
        case "destructive-action":
            guard let btn = view as? NSButton else { continue }
            btn.bezelColor = .systemRed
            btn.hasDestructiveAction = true
        case "pill":
            // Modern AppKit buttons are already rounded — no layer hacks.
            break
        case "flat":
            guard let btn = view as? NSButton else { continue }
            btn.isBordered = false
            btn.showsBorderOnlyWhileMouseInside = true
        case "title-1":
            applyCssFont(textTarget, .largeTitle)
        case "title-2":
            applyCssFont(textTarget, .title1)
        case "title-3":
            applyCssFont(textTarget, .title2)
        case "title-4":
            applyCssFont(textTarget, .title3)
        case "heading":
            applyCssFont(textTarget, .headline)
        case "caption":
            applyCssFont(textTarget, .caption1)
        case "caption-heading":
            applyCssFont(textTarget, .caption2)
        case "body":
            applyCssFont(textTarget, .body)
        case "dimmed":
            applyCssTextColor(textTarget, .secondaryLabelColor)
        case "monospace":
            applyCssFontValue(textTarget, .monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular))
        case "numeric":
            applyCssFontValue(textTarget, .monospacedDigitSystemFont(ofSize: NSFont.systemFontSize, weight: .regular))
        default:
            // navigation-sidebar, card, view, toolbar, boxed-list, osd, ...:
            // structural roles owned by the SplitView/HeaderBar widgets on
            // the Mac — silently ignored here.
            break
        }
    }
}

private func applyCssFont(_ view: NSView, _ style: NSFont.TextStyle) {
    applyCssFontValue(view, .preferredFont(forTextStyle: style, options: [:]))
}

private func applyCssFontValue(_ view: NSView, _ font: NSFont) {
    if let field = view as? NSTextField {
        field.font = font
    } else if let textView = view as? NSTextView {
        textView.font = font
    }
}

private func applyCssTextColor(_ view: NSView, _ color: NSColor) {
    if let field = view as? NSTextField {
        field.textColor = color
    } else if let textView = view as? NSTextView {
        textView.textColor = color
    }
}

/// `ndApplyStyle` (peer of GTK's `style.applyStyle`; AppKit styling is
/// `NSColor`/`NSFont`/frame insets, not CSS — M6a-D5's reasoning kept
/// `style.zig` GTK-only). Decodes the `style` JSON object per M6b-D3's
/// style-key set (background/color/font/padding/margin/border). Best-effort
/// choices, documented per key:
///  - `background` -> `layer.backgroundColor` (forces `wantsLayer = true`).
///  - `color` -> the text color of the nearest text-bearing control
///    (NSTextField/NSButton/NSTextView-in-scrollview).
///  - `font` -> `NSFont` on the same text-bearing controls (family + size).
///  - `border` -> `layer.borderWidth`/`borderColor`/`cornerRadius`.
///  - `padding`/`margin` -> AppKit has no first-class equivalent to GTK's
///    CSS padding or widget margins; both are approximated by insetting the
///    view's frame within its parent by the given amount post-layout. This
///    is a v1 best-effort (documented per M6b-D3's "best-effort" callout) —
///    NSStackView already provides real spacing via the Box `spacing` prop,
///    so `padding`/`margin` mainly matter for non-stack leaf widgets.
func ndApplyStyle(_ view: NSView, _ nodeID: UInt32, _ styleJson: String) {
    let style = parseProps(styleJson)

    if let bg = style["background"] as? String, let color = nsColor(fromHexOrName: bg) {
        view.wantsLayer = true
        view.layer?.backgroundColor = color.cgColor
    }
    if let colorStr = style["color"] as? String, let color = nsColor(fromHexOrName: colorStr) {
        applyTextColor(view, color)
    }
    if let fontObj = style["font"] as? [String: Any] {
        applyFont(view, fontObj)
    }
    if let borderObj = style["border"] as? [String: Any] {
        view.wantsLayer = true
        if let width = (borderObj["width"] as? NSNumber)?.doubleValue {
            view.layer?.borderWidth = CGFloat(width)
        }
        if let colorStr = borderObj["color"] as? String, let color = nsColor(fromHexOrName: colorStr) {
            view.layer?.borderColor = color.cgColor
        }
        if let radius = (borderObj["radius"] as? NSNumber)?.doubleValue {
            view.layer?.cornerRadius = CGFloat(radius)
        }
    }
    if let paddingObj = style["padding"] as? [String: Any] {
        applyInset(view, paddingObj)
    }
    if let marginObj = style["margin"] as? [String: Any] {
        applyInset(view, marginObj)
    }
}

private func applyTextColor(_ view: NSView, _ color: NSColor) {
    if let field = view as? NSTextField {
        field.textColor = color
    } else if let button = view as? NSButton {
        let attributed = NSMutableAttributedString(string: button.title)
        attributed.addAttribute(.foregroundColor, value: color, range: NSRange(location: 0, length: attributed.length))
        button.attributedTitle = attributed
    } else if let scrollView = view as? NSScrollView, let textView = scrollView.documentView as? NSTextView {
        textView.textColor = color
    }
}

private func applyFont(_ view: NSView, _ fontObj: [String: Any]) {
    let size = (fontObj["size"] as? NSNumber)?.doubleValue ?? NSFont.systemFontSize
    let family = fontObj["family"] as? String
    let font = family.flatMap { NSFont(name: $0, size: CGFloat(size)) } ?? NSFont.systemFont(ofSize: CGFloat(size))
    if let field = view as? NSTextField {
        field.font = font
    } else if let button = view as? NSButton {
        button.font = font
    } else if let scrollView = view as? NSScrollView, let textView = scrollView.documentView as? NSTextView {
        textView.font = font
    }
}

/// Best-effort padding/margin: insets the view's frame within its current
/// bounds allocation by the given per-edge amounts. Applied once at style-
/// apply time (not re-derived on every layout pass), matching v1's scope.
private func applyInset(_ view: NSView, _ insetObj: [String: Any]) {
    let top = (insetObj["top"] as? NSNumber)?.doubleValue ?? 0
    let left = (insetObj["left"] as? NSNumber)?.doubleValue ?? 0
    let bottom = (insetObj["bottom"] as? NSNumber)?.doubleValue ?? 0
    let right = (insetObj["right"] as? NSNumber)?.doubleValue ?? 0
    guard top != 0 || left != 0 || bottom != 0 || right != 0 else { return }
    var frame = view.frame
    frame.origin.x += CGFloat(left)
    frame.origin.y += CGFloat(top)
    frame.size.width -= CGFloat(left + right)
    frame.size.height -= CGFloat(top + bottom)
    view.frame = frame
}

/// Parses `#RRGGBB`/`#RRGGBBAA` hex strings (the schema's style color
/// shape); unrecognized values are ignored (defensive — the React renderer
/// already validates style keys, per M6b-D6's `ndApplyStyle` doc comment).
private func nsColor(fromHexOrName hex: String) -> NSColor? {
    var s = hex
    if s.hasPrefix("#") { s.removeFirst() }
    guard s.count == 6 || s.count == 8 else { return nil }
    guard let value = UInt32(s, radix: 16) else { return nil }
    let hasAlpha = s.count == 8
    let r, g, b, a: CGFloat
    if hasAlpha {
        r = CGFloat((value >> 24) & 0xFF) / 255
        g = CGFloat((value >> 16) & 0xFF) / 255
        b = CGFloat((value >> 8) & 0xFF) / 255
        a = CGFloat(value & 0xFF) / 255
    } else {
        r = CGFloat((value >> 16) & 0xFF) / 255
        g = CGFloat((value >> 8) & 0xFF) / 255
        b = CGFloat(value & 0xFF) / 255
        a = 1
    }
    return NSColor(srgbRed: r, green: g, blue: b, alpha: a)
}

// MARK: - Grid placement helpers (referenced by the generated ndAppendChild/
// ndInsertBefore/ndRemoveChild Grid arms)

/// Sparse position -> cell tracking so `ndGridRemove` can find and clear a
/// child's cell without NSGridView exposing a child->cell lookup itself.
nonisolated(unsafe) private var gridCells: [ObjectIdentifier: [ObjectIdentifier: (row: Int, col: Int)]] = [:]

func ndGridPlace(_ grid: NSGridView, _ child: NSView, row: Int, column: Int, rowSpan: Int, columnSpan: Int) {
    while grid.numberOfRows <= row + max(rowSpan, 1) - 1 { grid.addRow(with: []) }
    while grid.numberOfColumns <= column + max(columnSpan, 1) - 1 { grid.addColumn(with: []) }
    grid.cell(atColumnIndex: column, rowIndex: row).contentView = child
    if rowSpan > 1 || columnSpan > 1 {
        grid.mergeCells(inHorizontalRange: NSRange(location: column, length: max(columnSpan, 1)),
                         verticalRange: NSRange(location: row, length: max(rowSpan, 1)))
    }
    gridCells[ObjectIdentifier(grid), default: [:]][ObjectIdentifier(child)] = (row, column)
}

func ndGridRemove(_ grid: NSGridView, _ child: NSView) {
    guard let cell = gridCells[ObjectIdentifier(grid)]?[ObjectIdentifier(child)] else {
        child.removeFromSuperview()
        return
    }
    grid.cell(atColumnIndex: cell.col, rowIndex: cell.row).contentView = nil
    gridCells[ObjectIdentifier(grid)]?[ObjectIdentifier(child)] = nil
}
