import AppKit

/// The trailing icon inside a text or search field: the page state browsers
/// show at the end of the address bar, such as the zoom magnifier
/// (`trailingIconName` / `trailingIconTooltip` / `trailingIconLabel`, event
/// `trailingIconClicked`). GTK spells this as GtkEntry's secondary icon. AppKit
/// has no such slot, so the field gets an overlay button at its trailing edge.
///
/// A search field keeps its cancel-button cell, because that cell is what
/// NSSearchFieldCell measures its text rect against, but the cell draws nothing
/// while the icon is set: the overlay draws the symbol and takes the click, so
/// the field is never cleared by a press meant for the icon.
final class NDTrailingIconButton: NSButton {
    var nodeID: UInt32 = 0

    override init(frame: NSRect) {
        super.init(frame: frame)
        isBordered = false
        imagePosition = .imageOnly
        imageScaling = .scaleProportionallyDown
        setButtonType(.momentaryChange)
        contentTintColor = .secondaryLabelColor
        target = self
        action = #selector(fire)
        setContentHuggingPriority(.required, for: .horizontal)
    }

    required init?(coder: NSCoder) { fatalError("not supported") }

    override var acceptsFirstResponder: Bool { isEnabled }

    @objc private func fire() {
        guard nodeID != 0 else { return }
        ndEmitEvent(nodeID, "trailingIconClicked", "{}")
    }
}

private let ndTrailingIconWidth: CGFloat = 22

func ndTrailingIconButton(of view: NSView) -> NDTrailingIconButton? {
    view.subviews.compactMap { $0 as? NDTrailingIconButton }.first
}

/// The icon's rectangle in the field's own coordinates, for
/// `Popover.anchorSlot = "trailingIcon"`.
func ndTrailingIconRect(of view: NSView) -> NSRect? {
    guard let button = ndTrailingIconButton(of: view), !button.isHidden else { return nil }
    return button.frame
}

/// Merged create + applyProps arm for the three trailing-icon props. Absent
/// keys keep what the field already has; an empty name removes the icon.
func ndApplyTrailingIcon(_ view: NSView, _ props: [String: Any]) {
    guard let field = view as? NSTextField else { return }
    let name = propStr(props, "trailingIconName")
    if let name, name.isEmpty {
        ndRemoveTrailingIcon(field)
        return
    }
    let button: NDTrailingIconButton
    if let existing = ndTrailingIconButton(of: field) {
        button = existing
    } else {
        guard name != nil else { return }
        button = NDTrailingIconButton(frame: .zero)
        field.addSubview(button)
        if let cell = field.cell as? NSSearchFieldCell {
            let blank = NSImage(size: NSSize(width: 1, height: 1))
            cell.cancelButtonCell?.image = blank
            cell.cancelButtonCell?.alternateImage = blank
            // macOS 26 draws its own clear glyph whatever the image is.
            cell.cancelButtonCell?.isTransparent = true
        } else if let cell = field.cell as? NDTextFieldCell {
            cell.ndTrailingInset = ndTrailingIconWidth
        }
        ndLayoutTrailingIcon(field)
        ndTrailingIconAdoptNodeID(field)
    }
    if let name {
        button.image = ndResolveSymbolImage(ndSFSymbol(forFreedesktop: name) ?? name)
    }
    if let tip = propStr(props, "trailingIconTooltip") {
        button.toolTip = tip.isEmpty ? nil : tip
    }
    if let label = propStr(props, "trailingIconLabel"), !label.isEmpty {
        button.setAccessibilityLabel(label)
    } else if let tip = button.toolTip {
        button.setAccessibilityLabel(tip)
    }
    field.needsDisplay = true
}

private func ndRemoveTrailingIcon(_ field: NSTextField) {
    guard let button = ndTrailingIconButton(of: field) else { return }
    button.removeFromSuperview()
    (field.cell as? NSSearchFieldCell)?.resetCancelButtonCell()
    (field.cell as? NDTextFieldCell)?.ndTrailingInset = 0
    field.needsDisplay = true
}

/// Keeps the overlay on the trailing edge. Called on create and from the
/// field's own `layout()`, since the cancel-button rect moves with the width.
func ndLayoutTrailingIcon(_ field: NSTextField) {
    guard let button = ndTrailingIconButton(of: field) else { return }
    let bounds = field.bounds
    var rect = NSRect.zero
    if let cell = field.cell as? NSSearchFieldCell {
        rect = cell.cancelButtonRect(forBounds: bounds)
    }
    if rect.isEmpty {
        let h = min(bounds.height, ndTrailingIconWidth)
        rect = NSRect(x: bounds.maxX - ndTrailingIconWidth, y: bounds.midY - h / 2, width: ndTrailingIconWidth - 3, height: h)
    }
    // Only on a real change: this runs from the field's own layout().
    if button.frame != rect { button.frame = rect }
}

private nonisolated(unsafe) var ndTrailingIconNodeIDs: [ObjectIdentifier: UInt32] = [:]

/// Generated ndConnectEvents arm for `trailingIconClicked`. The overlay may not
/// exist yet, so the id is recorded on the field and adopted later.
func ndTrailingIconConnect(_ view: NSView, nodeID: UInt32) {
    ndTrailingIconNodeIDs[ObjectIdentifier(view)] = nodeID
    ndTrailingIconButton(of: view)?.nodeID = nodeID
}

func ndTrailingIconAdoptNodeID(_ field: NSTextField) {
    guard let id = ndTrailingIconNodeIDs[ObjectIdentifier(field)] else { return }
    ndTrailingIconButton(of: field)?.nodeID = id
}
