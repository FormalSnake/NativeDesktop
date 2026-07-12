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
/// `NSColor`/`NSFont`/Auto Layout, not CSS — M6a-D5's reasoning kept
/// `style.zig` GTK-only). Decodes the `style` JSON object per M6b-D3's
/// style-key set (background/color/font/padding/margin/border/hexpand/
/// vexpand/halign/valign). Best-effort choices, documented per key:
///  - `background` -> `layer.backgroundColor` (forces `wantsLayer = true`).
///  - `color` -> the text color of the nearest text-bearing control
///    (NSTextField/NSButton/NSTextView-in-scrollview).
///  - `font` -> `NSFont` on the same text-bearing controls (fontSize/
///    fontFamily/fontWeight).
///  - `border` -> `layer.borderWidth`/`borderColor`/`cornerRadius`.
///  - `padding` -> dispatched by view type; see `applyPadding`.
///  - `margin` -> silently ignored on AppKit v1. GTK widget margins are a
///    per-child gap the PARENT leaves around this widget; NSStackView has no
///    per-arranged-subview margin equivalent (only the stack's own
///    `edgeInsets`, which is `padding`'s job). A prior version approximated
///    this by mutating the view's frame post-layout, which fought Auto
///    Layout on every subsequent pass — removed rather than kept as a lie.
///  - `hexpand`/`vexpand`/`halign`/`valign` -> recorded into `ndLayoutFlags`
///    (Layout.swift) and reconciled against the parent NSStackView, if any,
///    via `ndBoxChildAttached`.
func ndApplyStyle(_ view: NSView, _ nodeID: UInt32, _ styleJson: String) {
    let style = parseProps(styleJson)

    // `background`/`border`/`padding` are the CSS-target keys GTK rebuilds
    // wholesale per apply (src/gtk/style.zig compileCss regenerates the
    // node's whole scoped CSS block from the CURRENT style object, so a
    // dropped key reverts to baseline there). Mirror that: write each one's
    // FULL effective state every apply — absent sub-keys fall back to
    // baseline — instead of only writing the sub-keys present in the JSON.
    // Additive-only writes leave residue (e.g. an amber pin border that
    // never clears because unpinning's style object simply omits
    // borderColor/borderWidth).
    //
    // `color`/`font` are also CSS-target on GTK but are deliberately NOT
    // reset here: on AppKit, ndApplyCssClasses (title-*/caption/dimmed/
    // monospace) writes the same NSFont/textColor properties, and a
    // prop-diff update that changes only `style` does not re-apply
    // cssClasses — resetting font/color to baseline here would clobber
    // standing cssClasses typography with nothing to restore it. The
    // correct fix is a per-node style cascade recompute (baseline ->
    // cssClasses -> style); tracked as a follow-up. Left monotonic for now.
    if let bg = style["background"] as? String, let color = nsColor(fromHexOrName: bg) {
        view.wantsLayer = true
        view.layer?.backgroundColor = color.cgColor
    } else if let layer = view.layer {
        layer.backgroundColor = nil
    }
    if let colorStr = style["color"] as? String, let color = nsColor(fromHexOrName: colorStr) {
        applyTextColor(view, color)
    }
    if let fontObj = style["font"] as? [String: Any] {
        applyFont(view, fontObj)
    }
    let borderObj = style["border"] as? [String: Any]
    let borderWidth = (borderObj?["borderWidth"] as? NSNumber)?.doubleValue ?? 0
    let borderColor = (borderObj?["borderColor"] as? String).flatMap { nsColor(fromHexOrName: $0) }
    let borderRadius = (borderObj?["borderRadius"] as? NSNumber)?.doubleValue ?? 0
    if borderWidth != 0 || borderColor != nil || borderRadius != 0 {
        view.wantsLayer = true
        view.layer?.borderWidth = CGFloat(borderWidth)
        view.layer?.borderColor = borderColor?.cgColor
        view.layer?.cornerRadius = CGFloat(borderRadius)
    } else if let layer = view.layer {
        layer.borderWidth = 0
        layer.borderColor = nil
        layer.cornerRadius = 0
    }
    // NSEdgeInsets() (all-zero) is the baseline when `padding` drops out of
    // the style object, so this always runs. (Aside: NDButton's ndPadding
    // didSet only switches its bezel TO `.flexiblePush`, never back — a
    // pre-existing one-way behavior, left as-is here.)
    let insets = style["padding"].flatMap(parseEdgeInsets) ?? NSEdgeInsets()
    applyPadding(view, insets)
    // `margin`: silently ignored on AppKit v1 (see doc comment above).

    if style["hexpand"] != nil || style["vexpand"] != nil || style["halign"] != nil || style["valign"] != nil {
        var flags = ndLayoutFlags[ObjectIdentifier(view)] ?? NDLayoutFlags()
        if let h = (style["hexpand"] as? NSNumber)?.boolValue { flags.hexpand = h }
        if let v = (style["vexpand"] as? NSNumber)?.boolValue { flags.vexpand = v }
        if let ha = style["halign"] as? String { flags.halign = ha }
        if let va = style["valign"] as? String { flags.valign = va }
        ndLayoutFlags[ObjectIdentifier(view)] = flags

        view.setContentHuggingPriority(flags.hexpand ? NSLayoutConstraint.Priority(1) : NSLayoutConstraint.Priority(250), for: .horizontal)
        view.setContentHuggingPriority(flags.vexpand ? NSLayoutConstraint.Priority(1) : NSLayoutConstraint.Priority(250), for: .vertical)

        if let stack = view.superview as? NSStackView {
            ndBoxChildAttached(stack, view)
        }
    }
}

/// Scalar (all four sides) or per-side object ({top,left,bottom,right},
/// missing = 0) — mirrors GTK style.zig's `applyMarginSpacing`/
/// `emitSpacingCss` scalar-or-object handling for the same `padding`/`margin`
/// JSON shape.
private func parseEdgeInsets(_ value: Any) -> NSEdgeInsets? {
    if let scalar = (value as? NSNumber)?.doubleValue {
        return NSEdgeInsets(top: CGFloat(scalar), left: CGFloat(scalar), bottom: CGFloat(scalar), right: CGFloat(scalar))
    }
    if let obj = value as? [String: Any] {
        let top = (obj["top"] as? NSNumber)?.doubleValue ?? 0
        let left = (obj["left"] as? NSNumber)?.doubleValue ?? 0
        let bottom = (obj["bottom"] as? NSNumber)?.doubleValue ?? 0
        let right = (obj["right"] as? NSNumber)?.doubleValue ?? 0
        return NSEdgeInsets(top: CGFloat(top), left: CGFloat(left), bottom: CGFloat(bottom), right: CGFloat(right))
    }
    return nil
}

/// Dispatches `padding` by view type — AppKit has no single "content inset"
/// API, so each widget shape gets its own real mapping instead of the old
/// one-shot frame mutation:
///  - NSStackView -> the stack's own `edgeInsets`, then reconcile children
///    (their cross-axis "fill" constraint bakes the insets into its constant).
///  - NDButton/NDTextField -> `ndPadding` (Layout.swift; inflates
///    intrinsicContentSize instead of touching the frame directly).
///  - NSScrollView wrapping an NSTextView (TextArea) -> `textContainerInset`.
///  - any other NSScrollView (ScrollView) -> `contentInsets`.
///  - anything else -> silently ignored (no AppKit equivalent for this
///    widget shape in v1).
private func applyPadding(_ view: NSView, _ insets: NSEdgeInsets) {
    if let stack = view as? NSStackView {
        stack.edgeInsets = insets
        ndBoxReconcileChildren(stack)
    } else if let button = view as? NDButton {
        button.ndPadding = insets
    } else if let field = view as? NDTextField {
        field.ndPadding = insets
    } else if let scrollView = view as? NSScrollView, let textView = scrollView.documentView as? NSTextView {
        textView.textContainerInset = NSSize(width: (insets.left + insets.right) / 2, height: (insets.top + insets.bottom) / 2)
    } else if let scrollView = view as? NSScrollView {
        scrollView.automaticallyAdjustsContentInsets = false
        scrollView.contentInsets = insets
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
    let size = (fontObj["fontSize"] as? NSNumber)?.doubleValue ?? NSFont.systemFontSize
    let family = fontObj["fontFamily"] as? String
    var font = family.flatMap { NSFont(name: $0, size: CGFloat(size)) } ?? NSFont.systemFont(ofSize: CGFloat(size))
    if (fontObj["fontWeight"] as? String) == "bold" {
        if family != nil {
            font = NSFontManager.shared.convert(font, toHaveTrait: .boldFontMask)
        } else {
            font = NSFont.boldSystemFont(ofSize: CGFloat(size))
        }
    }
    if let field = view as? NSTextField {
        field.font = font
    } else if let button = view as? NSButton {
        button.font = font
    } else if let scrollView = view as? NSScrollView, let textView = scrollView.documentView as? NSTextView {
        textView.font = font
    }
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
