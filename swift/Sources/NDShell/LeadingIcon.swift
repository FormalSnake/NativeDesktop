import AppKit

/// The leading icon inside a text or search field: the padlock browsers put at
/// the start of the address bar (`leadingIconName` / `leadingIconTooltip` /
/// `leadingIconLabel`, event `leadingIconClicked`). GTK spells this as
/// GtkEntry's primary icon; AppKit has no such slot on NSTextField and only a
/// non-interactive glyph on NSSearchField, so both get the same overlay button
/// and the field's text is inset past it.
///
/// On a search field the built-in search-button cell keeps drawing (its image
/// swapped for the app's symbol), because that cell is what NSSearchFieldCell
/// measures its text rect against; the overlay only takes the click. A plain
/// text field has nothing to measure against, so NDTextFieldCell carries the
/// inset instead.
final class NDLeadingIconButton: NSButton {
    var nodeID: UInt32 = 0

    override init(frame: NSRect) {
        super.init(frame: frame)
        isBordered = false
        bezelStyle = .shadowlessSquare
        imagePosition = .imageOnly
        setButtonType(.momentaryChange)
        focusRingType = .default
        target = self
        action = #selector(fire)
        // Keyboard activation: the button is a real key view, so Tab reaches
        // it and Space fires it, which an entry icon cannot do on GTK.
        setContentHuggingPriority(.required, for: .horizontal)
    }

    required init?(coder: NSCoder) { fatalError("not supported") }

    override var acceptsFirstResponder: Bool { isEnabled }

    @objc private func fire() {
        guard nodeID != 0 else { return }
        ndEmitEvent(nodeID, "leadingIconClicked", "{}")
    }
}

/// `NSTextFieldCell` that leaves room at the leading edge for the icon button.
final class NDTextFieldCell: NSTextFieldCell {
    var ndLeadingInset: CGFloat = 0

    private func inset(_ rect: NSRect) -> NSRect {
        guard ndLeadingInset > 0 else { return rect }
        var r = rect
        r.origin.x += ndLeadingInset
        r.size.width = max(0, r.size.width - ndLeadingInset)
        return r
    }

    override func drawingRect(forBounds rect: NSRect) -> NSRect {
        inset(super.drawingRect(forBounds: rect))
    }

    override func edit(withFrame rect: NSRect, in controlView: NSView, editor: NSText, delegate: Any?, event: NSEvent?) {
        super.edit(withFrame: inset(rect), in: controlView, editor: editor, delegate: delegate, event: event)
    }

    override func select(withFrame rect: NSRect, in controlView: NSView, editor: NSText, delegate: Any?, start: Int, length: Int) {
        super.select(withFrame: inset(rect), in: controlView, editor: editor, delegate: delegate, start: start, length: length)
    }
}

private let ndLeadingIconWidth: CGFloat = 22

/// The overlay button belonging to a field, if it has one.
func ndLeadingIconButton(of view: NSView) -> NDLeadingIconButton? {
    view.subviews.compactMap { $0 as? NDLeadingIconButton }.first
}

/// The icon's rectangle in the field's own coordinates, for a popover that
/// asked to point at the icon rather than at the whole field
/// (`Popover.anchorSlot = "leadingIcon"`).
func ndLeadingIconRect(of view: NSView) -> NSRect? {
    guard let button = ndLeadingIconButton(of: view), !button.isHidden else { return nil }
    return button.frame
}

/// Merged create + applyProps arm for the three leading-icon props. Absent
/// keys keep what the field already has; an empty name removes the icon, which
/// is what a dropped prop reaches the host as.
func ndApplyLeadingIcon(_ view: NSView, _ props: [String: Any]) {
    guard let field = view as? NSTextField else { return }
    let name = propStr(props, "leadingIconName")
    if let name, name.isEmpty {
        ndRemoveLeadingIcon(field)
        return
    }
    let button: NDLeadingIconButton
    if let existing = ndLeadingIconButton(of: field) {
        button = existing
    } else {
        guard name != nil else { return } // a tooltip with no icon has nothing to sit on
        button = NDLeadingIconButton(frame: .zero)
        field.addSubview(button)
        ndLayoutLeadingIcon(field)
        ndLeadingIconAdoptNodeID(field)
    }
    if let name {
        let image = ndResolveSymbolImage(ndSFSymbol(forFreedesktop: name) ?? name)
        button.image = image
        // A search field measures its text rect from the search-button cell,
        // so the symbol goes there too and the overlay stays transparent.
        if let cell = field.cell as? NSSearchFieldCell {
            cell.searchButtonCell?.image = image
            button.image = nil
        } else if let cell = field.cell as? NDTextFieldCell {
            cell.ndLeadingInset = ndLeadingIconWidth
        }
    }
    if let tip = propStr(props, "leadingIconTooltip") {
        button.toolTip = tip.isEmpty ? nil : tip
    }
    if let label = propStr(props, "leadingIconLabel") {
        button.setAccessibilityLabel(label.isEmpty ? nil : label)
    } else if button.accessibilityLabel() == nil, let tip = button.toolTip {
        button.setAccessibilityLabel(tip)
    }
    field.needsDisplay = true
}

private func ndRemoveLeadingIcon(_ field: NSTextField) {
    ndLeadingIconButton(of: field)?.removeFromSuperview()
    (field.cell as? NSSearchFieldCell)?.searchButtonCell?.image =
        NSImage(systemSymbolName: "magnifyingglass", accessibilityDescription: nil)
    (field.cell as? NDTextFieldCell)?.ndLeadingInset = 0
    field.needsDisplay = true
}

/// Keeps the overlay over the glyph. Called on create and from the field's own
/// `layout()`, since the search-button rect moves with the field's width.
func ndLayoutLeadingIcon(_ field: NSTextField) {
    guard let button = ndLeadingIconButton(of: field) else { return }
    let bounds = field.bounds
    let rect: NSRect
    if let cell = field.cell as? NSSearchFieldCell {
        rect = cell.searchButtonRect(forBounds: bounds)
    } else {
        let h = min(bounds.height, ndLeadingIconWidth)
        rect = NSRect(x: bounds.minX + 3, y: bounds.midY - h / 2, width: ndLeadingIconWidth - 3, height: h)
    }
    button.frame = rect.isEmpty ? NSRect(x: bounds.minX, y: bounds.minY, width: ndLeadingIconWidth, height: bounds.height) : rect
}

/// Generated ndConnectEvents arm for SearchInput/TextInput's
/// `leadingIconClicked`: the overlay may not exist yet when events are wired
/// (props and events land in the same commit, in that order), so the id is
/// recorded on the field and adopted by whichever button appears.
private nonisolated(unsafe) var ndLeadingIconNodeIDs: [ObjectIdentifier: UInt32] = [:]

func ndLeadingIconConnect(_ view: NSView, nodeID: UInt32) {
    ndLeadingIconNodeIDs[ObjectIdentifier(view)] = nodeID
    ndLeadingIconButton(of: view)?.nodeID = nodeID
}

/// `activateLeadingIcon`: fires the icon exactly as a click on it would, so an
/// app can open the same panel from a keyboard shortcut and a gate can reach
/// the icon without a pointer.
func ndEntryCommand(_ view: NSView, _ command: String) {
    guard command == "activateLeadingIcon" else { return }
    ndLeadingIconButton(of: view)?.performClick(nil)
}

func ndLeadingIconAdoptNodeID(_ field: NSTextField) {
    guard let id = ndLeadingIconNodeIDs[ObjectIdentifier(field)] else { return }
    ndLeadingIconButton(of: field)?.nodeID = id
}
