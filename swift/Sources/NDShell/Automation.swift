import AppKit
import CNd
import Foundation

// The AppKit peer of src/gtk/backend.zig's vtNodeVisible/vtNodeBounds/
// vtSnapshot/vtSemanticAction. node_visible/node_bounds/snapshot/
// semantic_action fill the automation half of the vtable; the socket/
// framing/waitFor/SLO machinery is core-owned and starts answering
// the moment these are wired into buildVTable().

// MARK: - automation window policy

/// NATIVE_AUTOMATION hosts opt their windows out of tiling window managers.
/// Drive scripts assert defaultWidth/Height geometry, and a tiler (OmniWM on
/// the dev box) retiles any standard resizable window ~200ms after present —
/// measured: 1100x700 -> a 1008x1267 half-screen tile with no user input —
/// turning every geometry gate into a race. The AXDialog subrole is the
/// conventional "leave me floating" signal tilers honor. User-facing runs
/// (no NATIVE_AUTOMATION) keep the standard subrole: being managed by the
/// user's window manager is correct platform behavior, not a defect.
func ndAutomationApplyWindowPolicy(_ win: NSWindow) {
    guard ProcessInfo.processInfo.environment["NATIVE_AUTOMATION"] == "1" else { return }
    win.setAccessibilitySubrole(.dialog)
}

// MARK: - node_visible / node_bounds

/// `visible` folds "mapped" into the same contract GTK's vtNodeVisible
/// uses: hidden OR not in a window == not visible.
@MainActor func ndNodeVisible(_ view: NSView) -> Bool {
    // Menu nodes are host-only NDMenuNodeViews (never in a window); treat
    // them as actionable so a MenuItem ref survives checkActionable and reaches
    // semanticClick.
    if ndIsMenuNode(view) { return true }
    // The palette's tracked node is an always-hidden host handle; its real
    // field/table live in the presented scrim. Report actionable exactly while
    // presented so automation can drive it (and only then).
    if let palette = view as? NDCommandPaletteHandleView { return palette.automationPresented }
    // A Window node's handle is orphaned (.window == nil) once a SplitView
    // takes over as contentViewController — resolve through the create-time
    // registry so a chrome window's root doesn't report invisible.
    if let win = ndContentToWindow[ObjectIdentifier(view)] { return win.isVisible }
    // A ToastOverlay wrapping the window's SplitView is a logical holder
    // that never enters the hierarchy (Toasts.swift setChild) — report its
    // host window's visibility, same contract as the Window-root branch.
    if let overlay = view as? NDToastOverlayView, overlay.ndEmbeddedSplit != nil,
       let win = overlay.ndHostWindow {
        return win.isVisible
    }
    // A header button promoted into the window toolbar (HeaderBar.swift's
    // system-drawn items) leaves the view hierarchy but stays the tracked
    // model — visible while the current rebuild promoted it.
    if let btn = view as? NDButton, ndToolbarPromotedItems[ObjectIdentifier(btn)] != nil {
        return ndWindowToolbarManager?.ownerWindow()?.isVisible ?? false
    }
    return !view.isHidden && view.window != nil
}

/// Frame (in the live content view's flipped space) of the internal control
/// AppKit draws for a promoted toolbar button — located in the window's
/// theme frame by TARGET identity (each promoted button has a unique
/// NDToolbarItemTarget adapter, and the system copies the item's
/// target/action onto its internal NSToolbarButton; measured on macOS 26).
/// nil while the item sits in the overflow menu.
@MainActor private func ndPromotedControlRect(for button: NDButton) -> NSRect? {
    guard let adapter = ndToolbarItemTargets[ObjectIdentifier(button)],
          let win = ndWindowToolbarManager?.ownerWindow(),
          let theme = win.contentView?.superview,
          let content = ndLiveContentView(ofWindow: win),
          let control = ndFindControl(in: theme, target: adapter) else { return nil }
    return control.convert(control.bounds, to: content)
}

@MainActor private func ndFindControl(in root: NSView, target: AnyObject) -> NSControl? {
    for sub in root.subviews {
        if let control = sub as? NSControl, control.target === target { return control }
        if let found = ndFindControl(in: sub, target: target) { return found }
    }
    return nil
}

/// `bounds` = the view's frame converted into the window's content-view
/// space, in `nd_rect` (top-left y-down — the content view is a
/// `FlippedView`, so this conversion lands directly in getTree's
/// `logical-window-topleft` contract, no extra flip needed).
///
/// NSStackView-arranged children size themselves via Auto Layout,
/// not manual `setFrame` — their `.frame` is only valid after a layout pass
/// runs. `layoutSubtreeIfNeeded()` on the content view forces that pass
/// before we read anything, otherwise stack-arranged widgets (the counter's
/// label/button) report all-zero geometry.
@MainActor func ndNodeBounds(_ view: NSView, _ out: inout nd_rect) -> Bool {
    // Menu nodes have no geometry; report a nominal non-degenerate rect so
    // checkActionable (w>0 ∧ h>0) admits a MenuItem ref for semanticClick.
    if ndIsMenuNode(view) {
        out = nd_rect(x: 0, y: 0, w: 1, h: 1)
        return true
    }
    // Palette host handle: a zero-size tracked node whose real surface is the
    // presented scrim. Report a nominal non-degenerate rect so checkActionable
    // admits it for the routed setValue/type/click actions.
    if view is NDCommandPaletteHandleView {
        out = nd_rect(x: 0, y: 0, w: 1, h: 1)
        return true
    }
    // A Window node's handle is its create-time content FlippedView; once a
    // SplitView takes over as contentViewController that view is orphaned
    // (.window == nil) and convert() below has no common window to route
    // through, returning garbage. Window handles are identified by the
    // create-time registry and report their window's LIVE content bounds.
    if let win = ndContentToWindow[ObjectIdentifier(view)] {
        guard let live = ndLiveContentView(ofWindow: win) else { return false }
        live.layoutSubtreeIfNeeded()
        out = nd_rect(x: 0, y: 0, w: Int32(live.bounds.width), h: Int32(live.bounds.height))
        return true
    }
    // Detached split-wrapping ToastOverlay (see ndNodeVisible): its handle has
    // no geometry of its own — report its host window's live content bounds.
    if let overlay = view as? NDToastOverlayView, overlay.ndEmbeddedSplit != nil,
       let win = overlay.ndHostWindow {
        guard let live = ndLiveContentView(ofWindow: win) else { return false }
        live.layoutSubtreeIfNeeded()
        out = nd_rect(x: 0, y: 0, w: Int32(live.bounds.width), h: Int32(live.bounds.height))
        return true
    }
    // Promoted toolbar button (see ndNodeVisible): geometry lives in the
    // toolbar's internal control, not the detached tracked view. An item in
    // overflow reports a nominal non-degenerate rect so checkActionable
    // still admits it for semanticClick (menu-node precedent above).
    if let btn = view as? NDButton, ndToolbarPromotedItems[ObjectIdentifier(btn)] != nil {
        if let r = ndPromotedControlRect(for: btn) {
            out = nd_rect(x: Int32(r.origin.x), y: Int32(r.origin.y), w: Int32(r.width), h: Int32(r.height))
        } else {
            out = nd_rect(x: 0, y: 0, w: 1, h: 1)
        }
        return true
    }
    // Resolve the LIVE, flipped content — see
    // SplitController.swift's ndLiveContentView for the full rationale.
    // Multi-window: convert into the widget's OWN window's content view, not the
    // single global one — a widget in window B must report bounds in window B's
    // space. A not-yet-shown widget (window == nil) falls back to the global.
    guard let content = ndLiveContentView(ofWindow: view.window) ?? ndLiveContentView() else { return false }
    content.layoutSubtreeIfNeeded()
    let r = view.convert(view.bounds, to: content)
    out = nd_rect(x: Int32(r.origin.x), y: Int32(r.origin.y), w: Int32(r.width), h: Int32(r.height))
    return true
}

// MARK: - snapshot: the fidelity ladder

/// `non-blank ≡ the PNG bitmap has >1 distinct pixel colour` (the exact
/// acceptance gate). Un-premultiplies RGB by alpha before
/// comparing, and skips near-transparent pixels entirely. Two failure modes
/// were measured directly against the real counter app and both would slip
/// past a naive "any byte differs" check:
///  - `NSBitmapImageRep`'s buffer is PREMULTIPLIED alpha: an antialiased
///    edge pixel of the same true white background paints as e.g. RGB
///    `(151,151,151)` at alpha 151 (151 = 255 * 151/255) versus RGB
///    `(249,249,249)` at alpha 249 elsewhere (same true colour, different
///    stored bytes), so comparing raw stored RGB treats window-shape
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

/// `bitmapImageRepForCachingDisplay(in:)` returns a PER-VIEW CACHED rep that
/// AppKit reuses across calls, and `cacheDisplay(in:to:)` does not clear it:
/// anything the current draw pass leaves unpainted keeps the PREVIOUS
/// capture's pixels. The macOS 26 glass sidebar paints nothing in an
/// offscreen pass (see ndCompositeSidebarPanes), so its region is exactly
/// where stale text ghosts. Zero the buffer before every cacheDisplay so
/// unpainted stays transparent.
@MainActor func ndZeroRep(_ rep: NSBitmapImageRep) {
    guard let data = rep.bitmapData else { return }
    memset(data, 0, rep.bytesPerRow * rep.pixelsHigh)
}

/// The contentView draws onto a TRANSPARENT bitmap in every capture rung —
/// the opaque window background is painted by the window server, not by the
/// contentView's own draw pass. Under dark mode the controls therefore
/// render as white-on-transparent, which flattens to white-on-white in any
/// PNG viewer AND collapses to a single un-premultiplied color under the
/// blank check. (An earlier probe's "blank cacheDisplay" result was exactly
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
@MainActor func flattenOntoWindowBackground(_ rep: NSBitmapImageRep, _ window: NSWindow?) -> NSBitmapImageRep {
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
    // Resolve the (multi-window: TARGET) window's dynamic background color in
    // its own appearance (dark mode -> dark gray), falling back to the generic
    // window color. Forced to alpha 1: under Liquid Glass the window's
    // backgroundColor is NOT opaque, and a translucent base lets stale pixels
    // show through the flatten.
    var bgCG: CGColor = NSColor.windowBackgroundColor.cgColor
    if let win = window ?? gWindow {
        win.effectiveAppearance.performAsCurrentDrawingAppearance {
            bgCG = win.backgroundColor.withAlphaComponent(1).cgColor
        }
    }
    let full = CGRect(x: 0, y: 0, width: width, height: height)
    ctx.setFillColor(bgCG)
    // .copy ERASES the destination instead of blending over it — the fill
    // must replace whatever the context held, then the capture composites
    // over that opaque base normally.
    ctx.setBlendMode(.copy)
    ctx.fill(full)
    ctx.setBlendMode(.normal)
    ctx.draw(cg, in: full)
    guard let outCG = ctx.makeImage() else { return rep }
    return NSBitmapImageRep(cgImage: outCG)
}

/// The macOS 26 sidebar glass container renders as an opaque plate in every
/// offscreen capture rung (its blur + content compose via the window server,
/// not the view's own draw pass), so a base capture of the split shows the
/// sidebar as a blank white column — while the sidebar pane's OWN view
/// (`item.viewController.view`, our NDPaneHostView) captures its widgets
/// perfectly when rendered outside that glass ancestor context (both
/// measured live against examples/notes). Compose the two: draw each
/// non-collapsed sidebar-behavior item's pane view over the base capture at
/// its frame, on a window-background backdrop (the pane's dark-mode pixels
/// are white-on-transparent, same as flattenOntoWindowBackground's input).
/// Pure-CG compositing — NSGraphicsContext(bitmapImageRep:) silently draws
/// nothing in this unbundled process (see flattenOntoWindowBackground).
@MainActor func ndCompositeSidebarPanes(_ rep: NSBitmapImageRep, _ content: NSView, _ window: NSWindow?) -> NSBitmapImageRep {
    guard let split = content as? NSSplitView, let controller = ndSplitViewController(for: split) else { return rep }
    let panes = controller.splitViewItems.filter { $0.behavior == .sidebar && !$0.isCollapsed }
    guard !panes.isEmpty, let baseCG = rep.cgImage else { return rep }
    let width = rep.pixelsWide
    let height = rep.pixelsHigh
    guard content.bounds.width > 0, content.bounds.height > 0,
          let ctx = CGContext(
              data: nil,
              width: width,
              height: height,
              bitsPerComponent: 8,
              bytesPerRow: 0,
              space: CGColorSpaceCreateDeviceRGB(),
              bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
          ) else { return rep }
    // Alpha 1 for the same reason as flattenOntoWindowBackground: a
    // translucent Liquid Glass background fill cannot erase the base's stale
    // sidebar pixels.
    var bgCG: CGColor = NSColor.windowBackgroundColor.cgColor
    if let win = window ?? gWindow {
        win.effectiveAppearance.performAsCurrentDrawingAppearance {
            bgCG = win.backgroundColor.withAlphaComponent(1).cgColor
        }
    }
    ctx.draw(baseCG, in: CGRect(x: 0, y: 0, width: width, height: height))
    let sx = CGFloat(width) / content.bounds.width
    let sy = CGFloat(height) / content.bounds.height
    for item in panes {
        let pane = item.viewController.view
        guard pane.bounds.width > 0, pane.bounds.height > 0, pane.window === content.window,
              let paneRep = pane.bitmapImageRepForCachingDisplay(in: pane.bounds) else { continue }
        ndZeroRep(paneRep) // AppKit reuses this rep per view; see ndZeroRep
        pane.cacheDisplay(in: pane.bounds, to: paneRep)
        guard let paneCG = paneRep.cgImage else { continue }
        // `content` is flipped (y-down): CG rect y measures from the bottom.
        let r = pane.convert(pane.bounds, to: content)
        let rect = CGRect(
            x: r.origin.x * sx,
            y: (content.bounds.height - r.maxY) * sy,
            width: r.width * sx,
            height: r.height * sy
        )
        ctx.setFillColor(bgCG)
        // .copy ERASES the base's stale sidebar region (default sourceOver
        // would blend the translucent fill over it, leaving ghosts); back to
        // .normal so the pane pixels composite over the opaque backdrop.
        ctx.setBlendMode(.copy)
        ctx.fill(rect)
        ctx.setBlendMode(.normal)
        ctx.draw(paneCG, in: rect)
    }
    guard let outCG = ctx.makeImage() else { return rep }
    return NSBitmapImageRep(cgImage: outCG)
}

/// Writes `rep` as a PNG to `path` iff it's non-blank. Returns whether it
/// wrote (i.e. whether this rung succeeded). The rep is flattened onto the
/// window background first — see `flattenOntoWindowBackground`.
@MainActor func writeIfNonBlank(_ rawRep: NSBitmapImageRep, _ path: String, _ window: NSWindow?) -> Bool {
    let rep = flattenOntoWindowBackground(rawRep, window)
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
/// bool, with no room in the C signature for a rung field, so the drive log
/// is the record of which path rendered).
///
/// The controls render correctly into EVERY rung — but as dark-mode white-on-
/// TRANSPARENT pixels, because the opaque window background belongs to the
/// window server, not the contentView's draw pass. Both the human eye (on a
/// white viewer background) and the un-premultiplying blank check read that
/// as "blank". Fixed by flattening every captured rep onto the window's
/// effective background color before the check + write
/// (`flattenOntoWindowBackground`). Rung 1 (`cacheDisplay`) wins outright.
@MainActor func ndSnapshot(_ pngPath: String) -> Bool {
    // Resolve the LIVE content — see
    // SplitController.swift's ndLiveContentView for the full rationale.
    // Multi-window: the target window selected by the preceding `resolve_window`
    // (automation.zig's selectSnapshotWindow) wins; consumed one-shot so a later
    // stray snapshot falls back to the global window rather than a stale target.
    let content = ndSnapshotTargetContent ?? ndLiveContentView()
    ndSnapshotTargetContent = nil
    guard let content else { return false }
    let targetWindow = content.window
    let bounds = content.bounds
    content.layoutSubtreeIfNeeded() // real Auto Layout pass before any capture rung

    // Rung 0 (opt-in): ScreenCaptureKit captures the real composited window
    // — glass included — so it is correct rather than approximated, and the
    // sidebar compositor is skipped entirely. Behind an env flag because it
    // prompts for screen-recording TCC on an ungranted machine (the ladder
    // below keeps the stock-runner no-prompt contract by default). A denied
    // grant falls through to the ladder.
    if ProcessInfo.processInfo.environment["ND_AUTOMATION_CAPTURE"] == "screencapturekit",
       let win = targetWindow, ndSnapshotViaSCK(win, pngPath) {
        FileHandle.standardError.write("ND_SNAPSHOT_RUNG rung=0\n".data(using: .utf8)!)
        return true
    }

    // Rung 1: force display, then cacheDisplay. The sidebar composite
    // (ndCompositeSidebarPanes) redraws glass-hosted sidebar panes that
    // capture as an opaque white plate in the base pass.
    content.displayIfNeeded()
    if let rep = content.bitmapImageRepForCachingDisplay(in: bounds) {
        ndZeroRep(rep) // AppKit reuses this rep per view; see ndZeroRep
        content.cacheDisplay(in: bounds, to: rep)
        let composed = ndCompositeSidebarPanes(rep, content, targetWindow)
        if writeIfNonBlank(composed, pngPath, targetWindow) {
            FileHandle.standardError.write("ND_SNAPSHOT_RUNG rung=1\n".data(using: .utf8)!)
            return true
        }
    }
    // Rung 2: layer-back the tree, render the CALayer into a CGContext bitmap.
    setWantsLayerRecursive(content)
    if let rep = renderLayerBitmap(content, bounds), writeIfNonBlank(rep, pngPath, targetWindow) {
        FileHandle.standardError.write("ND_SNAPSHOT_RUNG rung=2\n".data(using: .utf8)!)
        return true
    }
    // Rung 3: per-view lockFocus composite.
    if let rep = compositeLockFocus(content, bounds), writeIfNonBlank(rep, pngPath, targetWindow) {
        FileHandle.standardError.write("ND_SNAPSHOT_RUNG rung=3\n".data(using: .utf8)!)
        return true
    }
    // Rung 4: PDF -> raster.
    if let rep = pdfRasterize(content, bounds), writeIfNonBlank(rep, pngPath, targetWindow) {
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

/// Checkbox/Radio/Button all construct a plain `NSButton`, and ListView/
/// SourceList both construct a plain `NSScrollView`+`NSTableView` — AppKit
/// gives no reliable way to read either distinction back structurally
/// afterwards (`buttonType` is setter-only via `setButtonType(_:)`, and a
/// `.sourceList`-style NSTableView is still an NSTableView). Recorded once
/// at create time instead of reflected after the fact, keyed by view
/// identity (peer of `Events.swift`'s `radioGroupIdentifier` side-table,
/// same reasoning). Only needed for kinds that collide on a shared concrete
/// class — every other kind (TextInput, TextArea, Slider, Select,
/// ScrollView) is already unambiguous from its concrete AppKit class.
nonisolated(unsafe) private var buttonKindOverride: [ObjectIdentifier: String] = [:]

/// Called by `Backend.swift`'s `create` vtable closure right after
/// `ndCreate` returns, with the exact schema `kind` string the ABI call
/// carried — the ground truth `widgetKind` below would otherwise have to
/// guess at via reflection.
@MainActor func ndRecordButtonKind(_ view: NSView, _ kind: String) {
    guard kind == "Checkbox" || kind == "Radio" || kind == "Switch" || kind == "Button" || kind == "SourceList" || kind == "SourceTree" else { return }
    buttonKindOverride[ObjectIdentifier(view)] = kind
}

/// Widget-kind lookup for the semantic-action dispatch (peer of GTK's
/// `widgetKind`): the vtable call carries only the widget handle + node_id,
/// not the tracked `widget_type` string (that lives in core-owned
/// `Tree.meta`), so most kinds are read back structurally from the concrete
/// AppKit class every `ndCreate` arm constructs; Checkbox/Radio/Button
/// (all plain `NSButton`) and SourceList (an NSTableView-backed
/// NSScrollView, same concrete shape ListView constructs)
/// consult `buttonKindOverride` instead.
@MainActor private func widgetKind(_ view: NSView) -> String {
    if view is NSTextView { return "TextArea" }
    if let scroll = view as? NSScrollView {
        if scroll.documentView is NSTextView { return "TextArea" }
        if scroll.documentView is NSTableView {
            return buttonKindOverride[ObjectIdentifier(scroll)] ?? "ListView"
        }
        return "ScrollView"
    }
    // NSSwitch is an NSControl rather than an NSButton despite sharing the
    // button state API, so classify it explicitly before the generic button.
    if view is NSSwitch { return "Switch" }
    // SwitchRow subclasses Row (NDRowView) — check the subclass first.
    if view is NDSwitchRowView { return "SwitchRow" }
    if view is NDRowView { return "Row" }
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
    // A menu node dispatches its item (custom onSelect fires "selected";
    // a disabled item is a no-op, so onSelect does not fire — state unchanged).
    if ndIsMenuNode(view) {
        _ = ndMenuSemanticClick(view, nodeID)
        setResultRaw(resultOut, "{\"ref\":\(nodeID),\"dispatched\":true}")
        return 0
    }
    // SourceList: "click" activates the currently-selected row
    // (there's no separate NSControl to `performClick` — the tracked handle
    // is the NSScrollView). No-op (still reports dispatched) if nothing is
    // selected, unlike GTK's fallback-to-row-0 (a deliberate deviation; no
    // test currently exercises the no-selection case).
    if widgetKind(view) == "SourceList", let scroll = view as? NSScrollView,
       let tableView = scroll.documentView as? NSTableView, tableView.selectedRow >= 0 {
        EventDispatcher.shared.fireIndexNamed(scroll, name: "rowActivated", index: tableView.selectedRow)
        setResultRaw(resultOut, "{\"ref\":\(nodeID),\"dispatched\":true}")
        return 0
    }
    // SourceTree: "click" activates the selected row (rowActivated {nodeId});
    // no-op (still reports dispatched) when nothing is selected, same
    // deviation from GTK's fallback-to-first-row as SourceList above.
    if widgetKind(view) == "SourceTree" {
        _ = ndSourceTreeSemanticActivate(view)
        setResultRaw(resultOut, "{\"ref\":\(nodeID),\"dispatched\":true}")
        return 0
    }
    // SwitchRow click toggles like a user tap (row activation = its switch);
    // Row click emits `activated` (the boxed-list row affordance).
    if let row = view as? NDSwitchRowView {
        row.toggle.state = row.toggle.state == .on ? .off : .on
        ndEmitEvent(row.ndNodeID, "toggled", "{\"checked\":\(row.toggle.state == .on)}")
        setResultRaw(resultOut, "{\"ref\":\(nodeID),\"dispatched\":true}")
        return 0
    }
    if let row = view as? NDRowView {
        ndEmitEvent(row.ndNodeID, "activated", "{}")
        setResultRaw(resultOut, "{\"ref\":\(nodeID),\"dispatched\":true}")
        return 0
    }
    // Promoted toolbar button: the tracked view is detached (performClick
    // would try to highlight in a nil window) — fire its wired action with
    // the button as sender, exactly what the toolbar item's adapter does.
    if let btn = view as? NDButton, btn.window == nil,
       ndToolbarPromotedItems[ObjectIdentifier(btn)] != nil {
        if let action = btn.action { _ = NSApp.sendAction(action, to: btn.target, from: btn) }
        setResultRaw(resultOut, "{\"ref\":\(nodeID),\"dispatched\":true}")
        return 0
    }
    (view as? NSControl)?.performClick(nil)
    setResultRaw(resultOut, "{\"ref\":\(nodeID),\"dispatched\":true}")
    return 0
}

private func invalidValue(_ errOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?, _ nodeID: UInt32) -> Int32 {
    setErrRaw(errOut, nodeID)
    return -32602
}

/// Palette automation: string setValue -> query (queryChanged), integer
/// setValue -> activate the row at that index, bool setValue -> submit, type ->
/// append into the query, click -> activate the current highlight. Peer of
/// GTK's `automationAction`.
@MainActor private func semanticPalette(_ palette: NDCommandPaletteHandleView, _ nodeID: UInt32, _ action: String,
                                        _ args: [String: Any],
                                        _ resultOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?,
                                        _ errOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) -> Int32 {
    guard palette.automationPresented else { setErrRaw(errOut, nodeID); return -32001 } // not actionable while closed
    switch action {
    case "setValue":
        guard let value = args["value"] else { return invalidValue(errOut, nodeID) }
        if let s = value as? String {
            palette.automationSetQuery(s)
        } else if let num = value as? NSNumber {
            // JSONSerialization encodes booleans as CFBoolean and integers as
            // CFNumber; a bare `as? Bool` also matches integers, so split on the
            // CoreFoundation type instead.
            if CFGetTypeID(num) == CFBooleanGetTypeID() {
                guard num.boolValue else { return invalidValue(errOut, nodeID) }
                palette.automationSubmit()
            } else {
                guard palette.automationActivateRow(num.intValue) else { return invalidValue(errOut, nodeID) }
            }
        } else {
            return invalidValue(errOut, nodeID)
        }
        setResultRaw(resultOut, "{\"ref\":\(nodeID),\"applied\":true}")
        return 0
    case "type":
        guard let text = args["text"] as? String else { return invalidValue(errOut, nodeID) }
        let full = palette.automationAppendQuery(text)
        setResultRaw(resultOut, "{\"ref\":\(nodeID),\"text\":\"\(escapeJSONString(full))\"}")
        return 0
    case "click":
        palette.automationClickHighlight()
        setResultRaw(resultOut, "{\"ref\":\(nodeID),\"dispatched\":true}")
        return 0
    default:
        setErrRaw(errOut, nodeID)
        return -32601
    }
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
        EventDispatcher.shared.fireChanged(scroll, text: text) // wired key is the NSScrollView
    case "Checkbox", "Radio":
        guard let boolValue = value as? Bool, let button = view as? NSButton else { return invalidValue(errOut, nodeID) }
        button.state = boolValue ? .on : .off
        EventDispatcher.shared.fireChecked(button)
    case "Switch":
        guard let boolValue = value as? Bool, let toggle = view as? NSSwitch else { return invalidValue(errOut, nodeID) }
        toggle.state = boolValue ? .on : .off
        EventDispatcher.shared.fireChecked(toggle)
    case "SwitchRow":
        guard let boolValue = value as? Bool, let row = view as? NDSwitchRowView else { return invalidValue(errOut, nodeID) }
        row.toggle.state = boolValue ? .on : .off
        // The switch's own action doesn't fire on programmatic writes —
        // replay the row's emit exactly like a user toggle.
        ndEmitEvent(row.ndNodeID, "toggled", "{\"checked\":\(boolValue)}")
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
    case "SourceList":
        guard let idx = (value as? NSNumber)?.intValue, let scroll = view as? NSScrollView,
              let tableView = scroll.documentView as? NSTableView,
              idx >= 0, idx < tableView.numberOfRows else { return invalidValue(errOut, nodeID) }
        // NOT followed by an explicit EventDispatcher fire, unlike every arm
        // above: `selectRowIndexes` posts `tableViewSelectionDidChange`
        // itself (SourceList.swift's `SourceListDataSource`), which fires
        // "selectionChanged" via `fireIndexNamed` — the one control here
        // where AppKit's own notification already replays what a live user
        // selection would produce, so re-firing here would double the event.
        tableView.selectRowIndexes(IndexSet(integer: idx), byExtendingSelection: false)
    case "SourceTree":
        // Id-addressed: the value is a node ID string ("" deselects).
        // selectRowIndexes posts the selection notification itself, same
        // no-explicit-fire contract as the SourceList arm above.
        guard let id = value as? String, ndSourceTreeSemanticSelect(view, id) else { return invalidValue(errOut, nodeID) }
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

/// `a11y` — the live per-node accessibility probe behind getTree's
/// enabled/focused/value fields. Value reads mirror
/// `semanticSetValue`'s kind dispatch so both sides of a round-trip agree
/// on what a widget's value is. Menu nodes answer -32601 (defaults).
@MainActor private func semanticA11y(_ view: NSView, _ nodeID: UInt32,
                           _ resultOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?,
                           _ errOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) -> Int32 {
    if ndIsMenuNode(view) {
        setErrRaw(errOut, nodeID)
        return -32601
    }
    var enabled = true
    if let control = view as? NSControl { enabled = control.isEnabled }
    let win = view.window ?? ndWindow(for: view)
    var focused = false
    if let fr = win?.firstResponder {
        if let frv = fr as? NSView { focused = (frv === view) || frv.isDescendant(of: view) }
        // An editing NSTextField's first responder is the shared field
        // editor (hosted by the window, not the field) — currentEditor is
        // the reliable focus signal for text fields.
        if !focused, let field = view as? NSTextField, field.currentEditor() != nil { focused = true }
    }

    var valueJson = "null"
    switch widgetKind(view) {
    case "TextInput", "SearchInput":
        if let field = view as? NSTextField { valueJson = "\"\(escapeJSONString(field.stringValue))\"" }
    case "TextArea":
        if let scroll = view as? NSScrollView, let tv = scroll.documentView as? NSTextView {
            valueJson = "\"\(escapeJSONString(tv.string))\""
        }
    case "Checkbox", "Radio":
        if let button = view as? NSButton { valueJson = button.state == .on ? "true" : "false" }
    case "Switch":
        if let toggle = view as? NSSwitch { valueJson = toggle.state == .on ? "true" : "false" }
    case "SwitchRow":
        if let row = view as? NDSwitchRowView { valueJson = row.toggle.state == .on ? "true" : "false" }
    case "Slider":
        if let slider = view as? NSSlider { valueJson = "\(slider.doubleValue)" }
    case "Select":
        if let pop = view as? NSPopUpButton { valueJson = "\(pop.indexOfSelectedItem)" }
    case "SourceList", "Table", "TreeView":
        if let scroll = view as? NSScrollView, let tableView = scroll.documentView as? NSTableView,
           tableView.selectedRow >= 0 {
            valueJson = "\(tableView.selectedRow)"
        }
    case "SourceTree":
        // Id-addressed widget: the value is the selected node's ID, not a row index.
        if let id = ndSourceTreeSelectedId(view) { valueJson = "\"\(escapeJSONString(id))\"" }
    default:
        break
    }
    setResultRaw(resultOut, "{\"enabled\":\(enabled),\"focused\":\(focused),\"value\":\(valueJson)}")
    return 0
}

private func numArg(_ args: [String: Any]?, _ key: String) -> Double? {
    return (args?[key] as? NSNumber)?.doubleValue
}

/// `semantic_action` — dispatch on `action`, peer of GTK's
/// `vtSemanticAction`. Returns 0 ok / -32601 unknown action / -32602 invalid
/// value. The input-synthesis arms (pointer/drag/keys) arrive with a WINDOW
/// node's handle and post real NSEvents (Input.swift); doubleClick/
/// rightClick/hover target a widget ref's center.
@MainActor func ndSemanticAction(_ view: NSView, _ nodeID: UInt32, _ action: String, _ argJson: String,
                       _ resultOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?,
                       _ errOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) -> Int32 {
    let args = parseProps(argJson)
    // Palette: route setValue/type/click to the real field/table; a11y and the
    // rest fall through to the generic host-view handling below.
    if let palette = view as? NDCommandPaletteHandleView,
       action == "setValue" || action == "type" || action == "click" {
        return semanticPalette(palette, nodeID, action, args, resultOut, errOut)
    }
    switch action {
    case "window.close":
        // Window-root unmount (tree.zig remove arm): close the native
        // window/tab — no-op if the user already closed it.
        ndWindowTabsClose(view)
        return 0
    case "click":
        return semanticClick(view, nodeID, resultOut)
    case "setValue":
        return semanticSetValue(view, nodeID, args, resultOut, errOut)
    case "type":
        return semanticType(view, nodeID, args, resultOut, errOut)
    case "scroll":
        return semanticScroll(view, nodeID, args, resultOut)
    case "a11y":
        return semanticA11y(view, nodeID, resultOut, errOut)
    case "windowState":
        // Frontmost-window probe behind the automation windows/resolve
        // ranking. Load-bearing on AppKit: a background tab window's views
        // are hidden but its WINDOW is not key — the key bit is what
        // separates it from the front tab.
        guard let win = view.window ?? ndWindow(for: view) else {
            setResultRaw(resultOut, "{\"key\":false,\"main\":false,\"visible\":false,\"title\":null}")
            return 0
        }
        let title = escapeJSONString(win.title)
        setResultRaw(resultOut, "{\"key\":\(win.isKeyWindow),\"main\":\(win.isMainWindow),\"visible\":\(win.isVisible),\"title\":\"\(title)\"}")
        return 0
    case "pointer":
        guard let phase = args["phase"] as? String, let x = numArg(args, "x"), let y = numArg(args, "y"),
              ndPostPointerPhase(view, phase: phase, x: x, y: y,
                                 button: (args["button"] as? String) ?? "left",
                                 clickCount: (args["clickCount"] as? NSNumber)?.intValue ?? 1)
        else { return invalidValue(errOut, nodeID) }
        setResultRaw(resultOut, "{\"dispatched\":true}")
        return 0
    case "drag":
        guard let fromX = numArg(args, "fromX"), let fromY = numArg(args, "fromY"),
              let toX = numArg(args, "toX"), let toY = numArg(args, "toY") else {
            return invalidValue(errOut, nodeID)
        }
        let steps = (args["steps"] as? NSNumber)?.intValue ?? 12
        guard ndPostDrag(view, fromX: fromX, fromY: fromY, toX: toX, toY: toY,
                         steps: steps,
                         durationMs: (args["durationMs"] as? NSNumber)?.intValue ?? 160,
                         button: (args["button"] as? String) ?? "left")
        else { return invalidValue(errOut, nodeID) }
        setResultRaw(resultOut, "{\"dispatched\":true,\"fromX\":\(fromX),\"fromY\":\(fromY),\"toX\":\(toX),\"toY\":\(toY),\"steps\":\(steps)}")
        return 0
    case "keys":
        guard let spec = args["keys"] as? String, ndPostKeys(view, spec: spec) else {
            return invalidValue(errOut, nodeID)
        }
        setResultRaw(resultOut, "{\"dispatched\":true}")
        return 0
    case "doubleClick":
        guard ndPostClicksAtCenter(view, button: "left", clicks: 2, dismissAfter: false) else {
            return invalidValue(errOut, nodeID)
        }
        setResultRaw(resultOut, "{\"ref\":\(nodeID),\"dispatched\":true}")
        return 0
    case "rightClick":
        guard ndPostClicksAtCenter(view, button: "right", clicks: 1, dismissAfter: true) else {
            return invalidValue(errOut, nodeID)
        }
        setResultRaw(resultOut, "{\"ref\":\(nodeID),\"dispatched\":true}")
        return 0
    case "hover":
        guard ndPostHoverAtCenter(view) else { return invalidValue(errOut, nodeID) }
        setResultRaw(resultOut, "{\"ref\":\(nodeID),\"dispatched\":true}")
        return 0
    default:
        setErrRaw(errOut, nodeID)
        return -32601
    }
}
