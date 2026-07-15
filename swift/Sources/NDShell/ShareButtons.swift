import AppKit

/// ShareButton is macOS-exclusive (ND_PLATFORM_NOOP on GTK, which has no
/// share sheet): an NSButton with the standard `square.and.arrow.up` symbol
/// that shows NSSharingServicePicker anchored on itself (macOS 13+ shows the
/// preview popover automatically). `items` heuristics: http(s) strings share
/// as URLs, absolute/tilde paths as file URLs, anything else as plain text.
// The delegate conformance is declared @MainActor (isolated conformance):
// NSSharingServicePickerDelegate itself isn't main-actor-annotated, but the
// picker only calls back on the UI thread.
final class NDShareButton: NSButton, @MainActor NSSharingServicePickerDelegate {
    var shareItems: [String] = []
    // Keep the picker alive for the duration of its popover (show() does not
    // retain it past the call).
    private var activePicker: NSSharingServicePicker?

    @objc func sharePressed(_ sender: Any?) {
        let items: [Any] = shareItems.map { s -> Any in
            if s.hasPrefix("http://") || s.hasPrefix("https://"), let u = URL(string: s) { return u }
            if s.hasPrefix("/") || s.hasPrefix("~") {
                return URL(fileURLWithPath: (s as NSString).expandingTildeInPath)
            }
            return s
        }
        guard !items.isEmpty else {
            FileHandle.standardError.write("ND_WARN ShareButton clicked with empty items\n".data(using: .utf8)!)
            return
        }
        let picker = NSSharingServicePicker(items: items)
        picker.delegate = self
        activePicker = picker
        picker.show(relativeTo: bounds, of: self, preferredEdge: .minY)
    }

    func sharingServicePicker(_ sharingServicePicker: NSSharingServicePicker, didChoose service: NSSharingService?) {
        activePicker = nil
    }
}

/// `ndCreate`'s ShareButton arm (generated) calls this.
func makeShareButton(_ props: [String: Any]) -> NSView {
    let b = NDShareButton(title: propStr(props, "label") ?? "", target: nil, action: nil)
    b.setButtonType(.momentaryPushIn)
    b.bezelStyle = .rounded
    let iconName = propStr(props, "iconName")
    let symbol = iconName.map { ndSFSymbol(forFreedesktop: $0) ?? $0 } ?? "square.and.arrow.up"
    if let img = NSImage(systemSymbolName: symbol, accessibilityDescription: b.title.isEmpty ? "Share" : b.title) {
        b.image = img
        b.imagePosition = b.title.isEmpty ? .imageOnly : .imageLeading
    }
    b.shareItems = propArray(props, "items") ?? []
    b.target = b
    b.action = #selector(NDShareButton.sharePressed(_:))
    return b
}

/// Generated ndApplyProps ShareButton arm — merged apply.
func ndShareButtonApply(_ view: NSView, _ props: [String: Any]) {
    guard let b = view as? NDShareButton else { return }
    if let l = propStr(props, "label") {
        b.title = l
        b.imagePosition = l.isEmpty ? .imageOnly : .imageLeading
    }
    if let items = propArray(props, "items") { b.shareItems = items }
}
