import AppKit
import SwiftUI

/// ColorPicker: SwiftUI `ColorPicker`, itself backed by an NSColorWell with
/// the modern `.minimal` well style (macOS 13+ convention; the swatch opens
/// the picker popover) — the same control as before, now routed through
/// NDHostedLeaf. The wire value is `#rrggbb` (or `#rrggbbaa` when not fully
/// opaque), matching src/gtk/style.zig's color convention and the GTK
/// backend's ndRgbaToHex.
final class NDColorPickerView: NDHostedLeaf {
    private var color: Color = .black
    private var nsColor: NSColor = .black
    private var supportsAlpha = false

    func applyCreate(value: String, supportsAlpha: Bool) {
        self.supportsAlpha = supportsAlpha
        if let c = ndColorFromHex(value) {
            nsColor = c
            color = Color(nsColor: c)
        }
        refreshLeaf()
    }

    /// React-driven `value` write, echo-suppressed like every other
    /// controlled prop.
    func setValueFromProps(_ hex: String) {
        guard let c = ndColorFromHex(hex), !ndColorsClose(c, nsColor) else { return }
        withEchoSuppressed(self) { setColor(c, emit: false) }
    }

    private func setColor(_ c: NSColor, emit: Bool) {
        nsColor = c
        color = Color(nsColor: c)
        refreshLeaf()
        if emit, !ndIsEchoSuppressed(self) {
            ndEmitEvent(ndNodeID, "colorChanged", "{\"text\":\(ndJsonString(ndHexFromColor(c)))}")
        }
    }

    override func leafContent() -> AnyView {
        AnyView(
            ColorPicker(
                "",
                selection: Binding(
                    get: { [weak self] in self?.color ?? .black },
                    set: { [weak self] newColor in self?.setColor(NSColor(newColor), emit: true) }
                ),
                supportsOpacity: supportsAlpha
            )
            .labelsHidden())
    }

    override var ndA11yValueJSON: String { ndJsonString(ndHexFromColor(nsColor)) }
}

/// `ndCreate`'s ColorPicker arm (generated) calls this.
func makeColorPicker(_ props: [String: Any]) -> NSView {
    let well = NDColorPickerView()
    well.applyCreate(value: propStr(props, "value") ?? "#000000", supportsAlpha: propBool(props, "supportsAlpha") ?? false)
    return well
}

/// Generated ndApplyProps ColorPicker.value arm.
func ndColorPickerSetValue(_ view: NSView, _ hex: String) {
    (view as? NDColorPickerView)?.setValueFromProps(hex)
}

/// Generated ndConnectEvents ColorPicker arm.
func ndColorPickerConnect(_ view: NSView, nodeID: UInt32) {
    (view as? NDColorPickerView)?.ndNodeID = nodeID
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
