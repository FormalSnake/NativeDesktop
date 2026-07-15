import AppKit

/// FontPicker: a bordered NSButton showing the current font, opening
/// the ONE shared NSFontPanel (Apple convention: one panel per app). The
/// selection arrives through NSFontManager -> `changeFont(_:)`; the
/// FontPickerCoordinator singleton is the font manager's target and tracks
/// which node opened the panel, so only one FontPicker receives live
/// updates at a time.
///
/// Wire value is canonical Pango font description syntax
/// ("Family [Bold] [Italic] size") so GTK needs zero conversion; the NSFont
/// mapping happens entirely here. Generic Pango alias fallback (documented):
/// Sans/Sans-Serif/System -> .systemFont; Monospace/Mono ->
/// .monospacedSystemFont; Serif -> the system serif design. Unknown families
/// fall back to the system font.
final class FontPickerCoordinator: NSObject, NSFontChanging {
    nonisolated(unsafe) static let shared = FontPickerCoordinator()

    private(set) weak var activeButton: NDFontPickerButton?

    func activate(_ button: NDFontPickerButton) {
        activeButton = button
        let fm = NSFontManager.shared
        fm.target = self
        fm.setSelectedFont(button.currentFont, isMultiple: false)
        fm.orderFrontFontPanel(nil)
    }

    func changeFont(_ sender: NSFontManager?) {
        guard let fm = sender, let button = activeButton else { return }
        let newFont = fm.convert(button.currentFont)
        button.setCurrentFont(newFont, emit: true)
    }

    func validModesForFontPanel(_ fontPanel: NSFontPanel) -> NSFontPanel.ModeMask {
        [.collection, .face, .size]
    }
}

final class NDFontPickerButton: NSButton {
    var nodeID: UInt32 = 0
    private(set) var currentFont: NSFont = .systemFont(ofSize: NSFont.systemFontSize)

    @objc func openPanel(_ sender: Any?) {
        FontPickerCoordinator.shared.activate(self)
    }

    func setCurrentFont(_ font: NSFont, emit: Bool) {
        currentFont = font
        let pango = ndPangoString(from: font)
        title = pango
        if emit, !ndIsEchoSuppressed(self) {
            ndEmitEvent(nodeID, "fontChanged", "{\"text\":\(ndJsonString(pango))}")
        }
    }
}

/// `ndCreate`'s FontPicker arm (generated) calls this.
func makeFontPicker(_ props: [String: Any]) -> NSView {
    let b = NDFontPickerButton(title: "", target: nil, action: nil)
    b.setButtonType(.momentaryPushIn)
    b.bezelStyle = .rounded
    b.target = b
    b.action = #selector(NDFontPickerButton.openPanel(_:))
    b.setCurrentFont(ndFontFromPango(propStr(props, "value") ?? "Sans 12"), emit: false)
    return b
}

/// Generated ndApplyProps FontPicker.value arm: compare canonical strings
/// inside the echo guard (setCurrentFont's emit path re-checks it anyway).
func ndFontPickerSetValue(_ view: NSView, _ pango: String) {
    guard let b = view as? NDFontPickerButton else { return }
    guard ndPangoString(from: b.currentFont) != pango else { return }
    withEchoSuppressed(view) { b.setCurrentFont(ndFontFromPango(pango), emit: false) }
}

/// Generated ndConnectEvents FontPicker arm.
func ndFontPickerConnect(_ view: NSView, nodeID: UInt32) {
    (view as? NDFontPickerButton)?.nodeID = nodeID
}

// MARK: - Pango <-> NSFont conversion

func ndPangoString(from font: NSFont) -> String {
    let traits = NSFontManager.shared.traits(of: font)
    var family = font.familyName ?? font.fontName
    if family.hasPrefix(".") {
        // Private system families (.AppleSystemUIFont, .SFNS-*) don't
        // round-trip through Pango/GTK — emit the generic alias they map
        // back from in ndFontFromPango.
        family = font.isFixedPitch ? "Monospace" : "Sans"
    }
    var parts = [family]
    if traits.contains(.boldFontMask) { parts.append("Bold") }
    if traits.contains(.italicFontMask) { parts.append("Italic") }
    let size = font.pointSize
    parts.append(size == size.rounded() ? String(Int(size)) : String(format: "%.1f", size))
    return parts.joined(separator: " ")
}

/// Style words Pango may append after the family (only the ones NSFont can
/// honestly express are mapped; the rest are consumed so they don't pollute
/// the family name).
private let ndPangoStyleWords: Set<String> = [
    "bold", "italic", "oblique", "regular", "normal", "light", "ultralight",
    "thin", "medium", "semibold", "semi-bold", "heavy", "black", "book",
]

func ndFontFromPango(_ value: String) -> NSFont {
    var tokens = value.split(separator: " ").map(String.init)
    var size: CGFloat = 12
    if let last = tokens.last, let s = Double(last) {
        size = CGFloat(s)
        tokens.removeLast()
    }
    var bold = false
    var italic = false
    while let last = tokens.last, ndPangoStyleWords.contains(last.lowercased()) {
        switch last.lowercased() {
        case "bold", "semibold", "semi-bold", "heavy", "black": bold = true
        case "italic", "oblique": italic = true
        default: break
        }
        tokens.removeLast()
    }
    let family = tokens.joined(separator: " ")

    var font: NSFont
    switch family.lowercased() {
    case "", "sans", "sans-serif", "system", "system-ui":
        font = .systemFont(ofSize: size)
    case "monospace", "mono":
        font = .monospacedSystemFont(ofSize: size, weight: .regular)
    case "serif":
        let desc = NSFont.systemFont(ofSize: size).fontDescriptor.withDesign(.serif)
        font = desc.flatMap { NSFont(descriptor: $0, size: size) } ?? .systemFont(ofSize: size)
    default:
        font = NSFontManager.shared.font(withFamily: family, traits: [], weight: 5, size: size)
            ?? NSFont(name: family, size: size)
            ?? .systemFont(ofSize: size)
    }
    if bold { font = NSFontManager.shared.convert(font, toHaveTrait: .boldFontMask) }
    if italic { font = NSFontManager.shared.convert(font, toHaveTrait: .italicFontMask) }
    return font
}
