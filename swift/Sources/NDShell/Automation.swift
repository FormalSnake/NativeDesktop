import AppKit
import CNd
import Foundation

// The AppKit peer of src/gtk/backend.zig's vtNodeVisible/vtNodeBounds/
// vtSnapshot/vtSemanticAction (Task 5). node_visible/node_bounds/snapshot/
// semantic_action fill the automation half of the vtable; the socket/
// framing/waitFor/SLO machinery is core-owned (M6a-D3) and starts answering
// the moment these are wired into buildVTable().

// MARK: - node_visible / node_bounds

/// `visible` folds "mapped" into the same contract GTK's vtNodeVisible uses
/// (M6a Task 4 v1 decision): hidden OR not in a window == not visible.
@MainActor func ndNodeVisible(_ view: NSView) -> Bool {
    !view.isHidden && view.window != nil
}

/// `bounds` = the view's frame converted into the window's content-view
/// space, in `nd_rect` (top-left y-down — the content view is a
/// `FlippedView`, so this conversion lands directly in getTree's
/// `logical-window-topleft` contract, no extra flip needed).
///
/// NSStackView-arranged children (M6b-D3) size themselves via Auto Layout,
/// not manual `setFrame` — their `.frame` is only valid after a layout pass
/// runs. `layoutSubtreeIfNeeded()` on the content view forces that pass
/// before we read anything, otherwise stack-arranged widgets (the counter's
/// label/button) report all-zero geometry.
@MainActor func ndNodeBounds(_ view: NSView, _ out: inout nd_rect) -> Bool {
    // M11 Phase C (Risk 1 + Risk 2): resolve the LIVE, flipped content — see
    // SplitController.swift's ndLiveContentView for the full rationale.
    guard let content = ndLiveContentView() else { return false }
    content.layoutSubtreeIfNeeded()
    let r = view.convert(view.bounds, to: content)
    out = nd_rect(x: Int32(r.origin.x), y: Int32(r.origin.y), w: Int32(r.width), h: Int32(r.height))
    return true
}

// MARK: - snapshot: the fidelity ladder (M6b-D4)

/// `non-blank ≡ the PNG bitmap has >1 distinct pixel colour` (the exact
/// acceptance gate, M6b-D4). Un-premultiplies RGB by alpha before
/// comparing, and skips near-transparent pixels entirely. Two failure modes
/// were measured directly against the real counter app and both would slip
/// past a naive "any byte differs" check:
///  - `NSBitmapImageRep`'s buffer is PREMULTIPLIED alpha: an antialiased
///    edge pixel of the same true white background paints as e.g. RGB
///    `(151,151,151)` at alpha 151 (151 = 255 * 151/255) versus RGB
///    `(249,249,249)` at alpha 249 elsewhere — same true colour, different
///    stored bytes — so comparing raw stored RGB treats window-shape
///    antialiasing as "content" and false-passes a blank rung. Dividing
///    each component by `alpha/255` recovers the true (unpremultiplied)
///    colour, which is flat white across every edge pixel, correctly
///    blank.
///  - Fully-transparent (alpha≈0) pixels can carry incidental/undefined RGB
///    that isn't visible content at all, and dividing by a near-zero alpha
///    would explode quantization noise — skipped outright via
///    `alphaThreshold`.
/// Samples every Nth pixel and early-exits on the first unpremultiplied-RGB
/// difference — cheap and robust for both truly-blank and richly-rendered
/// frames.
@MainActor func hasMoreThanOneColor(_ rep: NSBitmapImageRep) -> Bool {
    guard let data = rep.bitmapData else { return false }
    let bpp = rep.bitsPerPixel / 8
    let rgbBytes = min(bpp, 3) // compare R,G,B only — never alpha
    guard rgbBytes > 0 else { return false }
    let pixelCount = rep.pixelsWide * rep.pixelsHigh
    guard pixelCount > 1 else { return false }
    let bytesPerRow = rep.bytesPerRow
    let stride = max(1, pixelCount / 20_000) // sample ~20k pixels max on huge bitmaps
    let hasAlpha = bpp > 3
    let alphaThreshold: UInt8 = 16 // skip near-transparent pixels: not visible, and unsafe to un-premultiply

    func unpremultiplied(_ offset: Int) -> [UInt8]? {
        guard hasAlpha else { return (0..<rgbBytes).map { (data + offset + $0).pointee } }
        let alpha = (data + offset + 3).pointee
        guard alpha >= alphaThreshold else { return nil }
        let scale = 255.0 / Double(alpha)
        return (0..<rgbBytes).map { UInt8((Double((data + offset + $0).pointee) * scale).rounded().clamped(to: 0...255)) }
    }

    var first: [UInt8]? = nil
    var i = 0
    while i < pixelCount {
        let x = i % rep.pixelsWide
        let y = i / rep.pixelsWide
        let offset = y * bytesPerRow + x * bpp
        guard let px = unpremultiplied(offset) else {
            i += stride
            continue
        }
        if let first {
            if px != first { return true }
        } else {
            first = px
        }
        i += stride
    }
    return false
}

private extension Double {
    func clamped(to range: ClosedRange<Double>) -> Double {
        Swift.min(Swift.max(self, range.lowerBound), range.upperBound)
    }
}

/// The contentView draws onto a TRANSPARENT bitmap in every capture rung —
/// the opaque window background is painted by the window server, not by the
/// contentView's own draw pass. Under dark mode the controls therefore
/// render as white-on-transparent, which flattens to white-on-white in any
/// PNG viewer AND collapses to a single un-premultiplied color under the
/// blank check. (The M6a probe's "blank cacheDisplay" result was exactly
/// this illusion: the alpha channel of those PNGs contained a pixel-perfect
/// render the whole time.) Composite every captured rep over the window's
/// effective background color before both the blank check and the write.
/// Implemented with a pure-CoreGraphics CGBitmapContext (CPU memory
/// drawing): drawing through `NSGraphicsContext(bitmapImageRep:)` silently
/// produces nothing in this SSH-launched unbundled process (verified — even
/// a direct `cgContext.fill` lands no pixels), which is also why the
/// layer-render and lockFocus rungs came back truly empty. `cacheDisplay`
/// fills its rep internally, so rung 1 + this CG flatten is the working
/// combination.
@MainActor func flattenOntoWindowBackground(_ rep: NSBitmapImageRep) -> NSBitmapImageRep {
    let width = rep.pixelsWide
    let height = rep.pixelsHigh
    guard let cg = rep.cgImage,
          let ctx = CGContext(
              data: nil,
              width: width,
              height: height,
              bitsPerComponent: 8,
              bytesPerRow: 0,
              space: CGColorSpaceCreateDeviceRGB(),
              bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
          ) else { return rep }
    // Resolve the window's dynamic background color in its own appearance
    // (dark mode -> dark gray), falling back to the generic window color.
    var bgCG: CGColor = NSColor.windowBackgroundColor.cgColor
    if let win = gWindow {
        win.effectiveAppearance.performAsCurrentDrawingAppearance {
            bgCG = win.backgroundColor.cgColor
        }
    }
    let full = CGRect(x: 0, y: 0, width: width, height: height)
    ctx.setFillColor(bgCG)
    ctx.fill(full)
    ctx.draw(cg, in: full)
    guard let outCG = ctx.makeImage() else { return rep }
    return NSBitmapImageRep(cgImage: outCG)
}

/// Writes `rep` as a PNG to `path` iff it's non-blank. Returns whether it
/// wrote (i.e. whether this rung succeeded). The rep is flattened onto the
/// window background first — see `flattenOntoWindowBackground`.
@MainActor func writeIfNonBlank(_ rawRep: NSBitmapImageRep, _ path: String) -> Bool {
    let rep = flattenOntoWindowBackground(rawRep)
    guard hasMoreThanOneColor(rep),
          let png = rep.representation(using: .png, properties: [:]) else { return false }
    do {
        try png.write(to: URL(fileURLWithPath: path))
        return true
    } catch {
        return false
    }
}

/// Rung 2: recursively force every view in the tree to be layer-backed, then
/// render the content view's backing CALayer into a CGContext bitmap.
@MainActor func setWantsLayerRecursive(_ view: NSView) {
    view.wantsLayer = true
    for sub in view.subviews { setWantsLayerRecursive(sub) }
}

@MainActor func renderLayerBitmap(_ content: NSView, _ bounds: NSRect) -> NSBitmapImageRep? {
    content.displayIfNeeded()
    guard let layer = content.layer else { return nil }
    let width = max(1, Int(bounds.width))
    let height = max(1, Int(bounds.height))
    guard let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil,
        pixelsWide: width,
        pixelsHigh: height,
        bitsPerSample: 8,
        samplesPerPixel: 4,
        hasAlpha: true,
        isPlanar: false,
        colorSpaceName: .deviceRGB,
        bytesPerRow: 0,
        bitsPerPixel: 0
    ) else { return nil }
    guard let ctx = NSGraphicsContext(bitmapImageRep: rep) else { return nil }
    let cgContext = ctx.cgContext
    cgContext.clear(CGRect(x: 0, y: 0, width: width, height: height))
    layer.render(in: cgContext)
    return rep
}

/// Rung 3: per-view `lockFocus()` composite, drawing each mapped subview
/// into one target rep at its window-space (== content-view-space, content
/// is flipped) frame — a manual fallback for when the layer tree (rung 2)
/// isn't populated either.
@MainActor func compositeLockFocus(_ content: NSView, _ bounds: NSRect) -> NSBitmapImageRep? {
    guard let rep = content.bitmapImageRepForCachingDisplay(in: bounds) else { return nil }
    rep.size = bounds.size

    func draw(_ view: NSView) {
        guard !view.isHidden else { return }
        let frameInContent = view.convert(view.bounds, to: content)
        if view.responds(to: #selector(NSView.lockFocus)), view.bounds.width > 0, view.bounds.height > 0 {
            view.lockFocus()
            if let viewRep = NSBitmapImageRep(focusedViewRect: view.bounds) {
                NSGraphicsContext.saveGraphicsState()
                NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
                viewRep.draw(in: frameInContent)
                NSGraphicsContext.restoreGraphicsState()
            }
            view.unlockFocus()
        }
        for sub in view.subviews { draw(sub) }
    }
    NSGraphicsContext.saveGraphicsState()
    draw(content)
    NSGraphicsContext.restoreGraphicsState()
    return rep
}

/// Rung 4: PDF -> raster. `dataWithPDF(inside:)` walks AppKit's print/PDF
/// drawing path (independent of the on-screen display path the earlier
/// rungs rely on), so it can succeed even when live compositing is blank.
@MainActor func pdfRasterize(_ content: NSView, _ bounds: NSRect) -> NSBitmapImageRep? {
    let pdfData = content.dataWithPDF(inside: bounds)
    guard let image = NSImage(data: pdfData) else { return nil }
    guard let tiff = image.tiffRepresentation, let rep = NSBitmapImageRep(data: tiff) else { return nil }
    return rep
}

/// `snapshot` — TCC-free in-process capture only. `CGWindowListCreateImage`
/// / ScreenCaptureKit / anything that can trigger a screen-recording TCC
/// prompt is FORBIDDEN (the entire point of the ladder is avoiding that
/// prompt on a stock, ungranted macOS runner). Tries each rung in order,
/// stops at the first non-blank PNG; logs which rung won to stderr as
/// `ND_SNAPSHOT_RUNG rung=N` (the frozen ABI's `snapshot` returns only a
/// bool — no room in the C signature for a rung field — so the drive log is
/// the record of which path rendered, per the plan's fallback instruction).
///
/// RESOLVED RISK (was "labels/buttons render blank"): the controls rendered
/// correctly into EVERY rung all along — but as dark-mode white-on-
/// TRANSPARENT pixels, because the opaque window background belongs to the
/// window server, not the contentView's draw pass. Both the human eye (on a
/// white viewer background) and the un-premultiplying blank check read that
/// as "blank". Fixed by flattening every captured rep onto the window's
/// effective background color before the check + write
/// (`flattenOntoWindowBackground`). Rung 1 (`cacheDisplay`) wins outright.
@MainActor func ndSnapshot(_ pngPath: String) -> Bool {
    // M11 Phase C (Risk 1): resolve the LIVE content — see
    // SplitController.swift's ndLiveContentView for the full rationale.
    guard let content = ndLiveContentView() else { return false }
    let bounds = content.bounds
    content.layoutSubtreeIfNeeded() // real Auto Layout pass before any capture rung

    // Rung 1: force display, then cacheDisplay.
    content.displayIfNeeded()
    if let rep = content.bitmapImageRepForCachingDisplay(in: bounds) {
        content.cacheDisplay(in: bounds, to: rep)
        if writeIfNonBlank(rep, pngPath) {
            FileHandle.standardError.write("ND_SNAPSHOT_RUNG rung=1\n".data(using: .utf8)!)
            return true
        }
    }
    // Rung 2: layer-back the tree, render the CALayer into a CGContext bitmap.
    setWantsLayerRecursive(content)
    if let rep = renderLayerBitmap(content, bounds), writeIfNonBlank(rep, pngPath) {
        FileHandle.standardError.write("ND_SNAPSHOT_RUNG rung=2\n".data(using: .utf8)!)
        return true
    }
    // Rung 3: per-view lockFocus composite.
    if let rep = compositeLockFocus(content, bounds), writeIfNonBlank(rep, pngPath) {
        FileHandle.standardError.write("ND_SNAPSHOT_RUNG rung=3\n".data(using: .utf8)!)
        return true
    }
    // Rung 4: PDF -> raster.
    if let rep = pdfRasterize(content, bounds), writeIfNonBlank(rep, pngPath) {
        FileHandle.standardError.write("ND_SNAPSHOT_RUNG rung=4\n".data(using: .utf8)!)
        return true
    }
    FileHandle.standardError.write("ND_SNAPSHOT_RUNG rung=none\n".data(using: .utf8)!)
    return false // core answers the RPC with -32603 (empty snapshot), same as GTK
}

// MARK: - semantic_action

/// Mallocs a NUL-terminated copy of `json` (freed by the core via libc
/// `free`/`nd_free`, uniform across languages — peer of GTK's `mallocZ`).
private func mallocZ(_ json: String) -> UnsafeMutablePointer<CChar>? {
    let utf8 = Array(json.utf8)
    guard let buf = malloc(utf8.count + 1) else { return nil }
    let bytes = buf.assumingMemoryBound(to: UInt8.self)
    for (i, b) in utf8.enumerated() { bytes[i] = b }
    bytes[utf8.count] = 0
    return buf.assumingMemoryBound(to: CChar.self)
}

private func setResultRaw(_ out: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?, _ json: String) {
    out?.pointee = mallocZ(json)
}

private func setErrRaw(_ out: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?, _ nodeID: UInt32) {
    out?.pointee = mallocZ("{\"ref\":\(nodeID)}")
}

/// Checkbox/Radio/Button all construct a plain `NSButton` — AppKit gives no
/// reliable way to read the button "type" back afterwards (`buttonType` is
/// setter-only via `setButtonType(_:)`; the cell doesn't forward a getter
/// either). Recorded once at create time instead of reflected after the
/// fact, keyed by view identity (peer of `Events.swift`'s
/// `radioGroupIdentifier` side-table, same reasoning). Only needed for the
/// three kinds that collide on `NSButton` — every other kind (TextInput,
/// TextArea, Slider, Select, ListView, ScrollView) is already unambiguous
/// from its concrete AppKit class.
nonisolated(unsafe) private var buttonKindOverride: [ObjectIdentifier: String] = [:]

/// Called by `Backend.swift`'s `create` vtable closure right after
/// `ndCreate` returns, with the exact schema `kind` string the ABI call
/// carried — the ground truth `widgetKind` below would otherwise have to
/// guess at via reflection.
@MainActor func ndRecordButtonKind(_ view: NSView, _ kind: String) {
    guard kind == "Checkbox" || kind == "Radio" || kind == "Button" else { return }
    buttonKindOverride[ObjectIdentifier(view)] = kind
}

/// Widget-kind lookup for the semantic-action dispatch (peer of GTK's
/// `widgetKind`): the vtable call carries only the widget handle + node_id,
/// not the tracked `widget_type` string (that lives in core-owned
/// `Tree.meta`), so most kinds are read back structurally from the concrete
/// AppKit class every `ndCreate` arm constructs; Checkbox/Radio/Button
/// (all plain `NSButton`) consult `buttonKindOverride` instead.
@MainActor private func widgetKind(_ view: NSView) -> String {
    if view is NSTextView { return "TextArea" }
    if let scroll = view as? NSScrollView {
        if scroll.documentView is NSTextView { return "TextArea" }
        if scroll.documentView is NSTableView { return "ListView" }
        return "ScrollView"
    }
    // NSPopUpButton is an NSButton subclass — must be checked before the
    // generic NSButton arm below, or every Select misclassifies as Button.
    if view is NSPopUpButton { return "Select" }
    if view is NSButton {
        return buttonKindOverride[ObjectIdentifier(view)] ?? "Button"
    }
    if view is NSTextField { return "TextInput" }
    if view is NSSlider { return "Slider" }
    return ""
}

private func escapeJSONString(_ s: String) -> String {
    var out = ""
    for scalar in s.unicodeScalars {
        switch scalar {
        case "\"": out += "\\\""
        case "\\": out += "\\\\"
        case "\n": out += "\\n"
        case "\r": out += "\\r"
        case "\t": out += "\\t"
        default:
            if scalar.value < 0x20 {
                out += String(format: "\\u%04x", scalar.value)
            } else {
                out.unicodeScalars.append(scalar)
            }
        }
    }
    return out
}

@MainActor private func semanticClick(_ view: NSView, _ nodeID: UInt32,
                            _ resultOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) -> Int32 {
    (view as? NSControl)?.performClick(nil)
    setResultRaw(resultOut, "{\"ref\":\(nodeID),\"dispatched\":true}")
    return 0
}

private func invalidValue(_ errOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?, _ nodeID: UInt32) -> Int32 {
    setErrRaw(errOut, nodeID)
    return -32602
}

/// `setValue` — per-control, mirroring GTK's `semanticSetValue`. Setting a
/// control's value programmatically does NOT fire its change notification in
/// AppKit (unlike GTK's `Editable.setText`/`Range.setValue`, which fire
/// their signals as a side effect), so each arm sets the value directly and
/// then explicitly replays the exact `EventDispatcher` fire method a live
/// user edit would trigger, so React sees exactly one `changed`/`toggled`/
/// `valueChanged`/`selectionChanged` event.
@MainActor private func semanticSetValue(_ view: NSView, _ nodeID: UInt32, _ args: [String: Any]?,
                               _ resultOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?,
                               _ errOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) -> Int32 {
    guard let args, let value = args["value"] else { return invalidValue(errOut, nodeID) }
    let kind = widgetKind(view)

    switch kind {
    case "TextInput":
        guard let text = value as? String, let field = view as? NSTextField else { return invalidValue(errOut, nodeID) }
        field.stringValue = text
        EventDispatcher.shared.fireChanged(field, text: text) // mirrors GTK's setText firing "changed"
    case "TextArea":
        guard let text = value as? String, let scroll = view as? NSScrollView,
              let textView = scroll.documentView as? NSTextView else { return invalidValue(errOut, nodeID) }
        textView.string = text
        EventDispatcher.shared.fireChanged(scroll, text: text) // wired key is the NSScrollView (M6b-D2)
    case "Checkbox", "Radio":
        guard let boolValue = value as? Bool, let btn = view as? NSButton else { return invalidValue(errOut, nodeID) }
        btn.state = boolValue ? .on : .off
        EventDispatcher.shared.fireChecked(btn)
    case "Slider":
        let num: Double?
        if let n = value as? NSNumber { num = n.doubleValue } else { num = nil }
        guard let doubleValue = num, let slider = view as? NSSlider else { return invalidValue(errOut, nodeID) }
        slider.doubleValue = doubleValue
        EventDispatcher.shared.fireValue(slider)
    case "Select":
        guard let idx = (value as? NSNumber)?.intValue, let pop = view as? NSPopUpButton,
              idx >= 0, idx < pop.numberOfItems else { return invalidValue(errOut, nodeID) }
        pop.selectItem(at: idx)
        EventDispatcher.shared.fireIndex(pop)
    default:
        setErrRaw(errOut, nodeID)
        return -32602
    }
    setResultRaw(resultOut, "{\"ref\":\(nodeID),\"applied\":true}")
    return 0
}

/// `type` — append text to `NSTextField.stringValue` then fire `changed`,
/// mirroring GTK's `Editable.insertText(-1)` append semantics. Result JSON
/// carries the full text after insertion (`{"ref":N,"text":"..."}`), same
/// shape as GTK's `semanticType`.
@MainActor private func semanticType(_ view: NSView, _ nodeID: UInt32, _ args: [String: Any]?,
                           _ resultOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?,
                           _ errOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) -> Int32 {
    guard widgetKind(view) == "TextInput", let field = view as? NSTextField else {
        return invalidValue(errOut, nodeID)
    }
    guard let text = args?["text"] as? String else { return invalidValue(errOut, nodeID) }
    field.stringValue += text
    let full = field.stringValue
    EventDispatcher.shared.fireChanged(field, text: full)
    setResultRaw(resultOut, "{\"ref\":\(nodeID),\"text\":\"\(escapeJSONString(full))\"}")
    return 0
}

/// `scroll` — `NSScrollView.contentView` scroll by `dx`/`dy`, mirroring
/// GTK's adjustment-delta semantics. Result carries the new scroll offsets,
/// same shape as GTK's `semanticScroll` (`{"ref":N,"x":...,"y":...}`).
@MainActor private func semanticScroll(_ view: NSView, _ nodeID: UInt32, _ args: [String: Any]?,
                             _ resultOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) -> Int32 {
    guard let scrollView = view as? NSScrollView else {
        setResultRaw(resultOut, "{\"ref\":\(nodeID),\"x\":0,\"y\":0}")
        return 0
    }
    let clipView = scrollView.contentView
    var origin = clipView.bounds.origin
    if let dx = (args?["dx"] as? NSNumber)?.doubleValue { origin.x += CGFloat(dx) }
    if let dy = (args?["dy"] as? NSNumber)?.doubleValue { origin.y += CGFloat(dy) }
    clipView.scroll(to: origin)
    scrollView.reflectScrolledClipView(clipView)
    let newOrigin = clipView.bounds.origin
    setResultRaw(resultOut, "{\"ref\":\(nodeID),\"x\":\(newOrigin.x),\"y\":\(newOrigin.y)}")
    return 0
}

/// `semantic_action` — dispatch on `action`, peer of GTK's `vtSemanticAction`
/// (M6b-D5). Returns 0 ok / -32601 unknown action / -32602 invalid value.
@MainActor func ndSemanticAction(_ view: NSView, _ nodeID: UInt32, _ action: String, _ argJson: String,
                       _ resultOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?,
                       _ errOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) -> Int32 {
    let args = parseProps(argJson)
    switch action {
    case "click":
        return semanticClick(view, nodeID, resultOut)
    case "setValue":
        return semanticSetValue(view, nodeID, args, resultOut, errOut)
    case "type":
        return semanticType(view, nodeID, args, resultOut, errOut)
    case "scroll":
        return semanticScroll(view, nodeID, args, resultOut)
    default:
        setErrRaw(errOut, nodeID)
        return -32601
    }
}
