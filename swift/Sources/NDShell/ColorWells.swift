import AppKit

/// ColorPicker: NSColorWell with the modern `.minimal` well style
/// (macOS 13+ convention; the swatch opens the picker popover). The wire
/// value is `#rrggbb` (or `#rrggbbaa` when not fully opaque), matching
/// src/gtk/style.zig's color convention and the GTK backend's ndRgbaToHex.
final class NDColorWell: NSColorWell {
    /// The shared NSColorPanel is process-global; alpha support is per-widget
    /// in the schema, so each well re-asserts its own setting on activation.
    var ndSupportsAlpha = false

    override func activate(_ exclusive: Bool) {
        NSColorPanel.shared.showsAlpha = ndSupportsAlpha
        super.activate(exclusive)
    }
}

/// `ndCreate`'s ColorPicker arm (generated) calls this.
func makeColorPicker(_ props: [String: Any]) -> NSView {
    let well = NDColorWell()
    well.colorWellStyle = .minimal
    well.ndSupportsAlpha = propBool(props, "supportsAlpha") ?? false
    if let v = propStr(props, "value"), let color = ndColorFromHex(v) {
        well.color = color
    }
    return well
}

/// Generated ndApplyProps ColorPicker.value arm: compare-then-set inside the
/// echo guard (the well's action re-fires on programmatic sets in some
/// panel-attached states; the guard makes it definitively silent).
func ndColorPickerSetValue(_ view: NSView, _ hex: String) {
    guard let well = view as? NSColorWell, let color = ndColorFromHex(hex) else { return }
    guard !ndColorsClose(well.color, color) else { return }
    withEchoSuppressed(view) { well.color = color }
}

/// #rrggbb / #rrggbbaa -> NSColor (sRGB, the same space ndHexFromColor reads
/// back, so the round-trip is stable).
func ndColorFromHex(_ hex: String) -> NSColor? {
    var s = hex
    if s.hasPrefix("#") { s.removeFirst() }
    guard s.count == 6 || s.count == 8, let value = UInt32(s, radix: 16) else { return nil }
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

/// NSColor -> wire hex. The alpha byte is included only when not fully
/// opaque (GTK ndRgbaToHex parity).
func ndHexFromColor(_ color: NSColor) -> String {
    let c = color.usingColorSpace(.sRGB) ?? color
    let r = Int((c.redComponent * 255).rounded())
    let g = Int((c.greenComponent * 255).rounded())
    let b = Int((c.blueComponent * 255).rounded())
    let a = Int((c.alphaComponent * 255).rounded())
    if a == 255 { return String(format: "#%02x%02x%02x", r, g, b) }
    return String(format: "#%02x%02x%02x%02x", r, g, b, a)
}

/// Component-wise closeness within 1/255 (GTK ndRgbaClose parity) — hex has
/// 8-bit resolution, so float noise below that must not count as a change.
func ndColorsClose(_ a: NSColor, _ b: NSColor) -> Bool {
    guard let x = a.usingColorSpace(.sRGB), let y = b.usingColorSpace(.sRGB) else { return false }
    let eps: CGFloat = 1.0 / 255.0
    return abs(x.redComponent - y.redComponent) < eps
        && abs(x.greenComponent - y.greenComponent) < eps
        && abs(x.blueComponent - y.blueComponent) < eps
        && abs(x.alphaComponent - y.alphaComponent) < eps
}
