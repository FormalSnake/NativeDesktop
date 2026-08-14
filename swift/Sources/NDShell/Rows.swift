import AppKit

// AppKit peers of libadwaita's boxed-list rows (`<row>` / `<switchrow>`).
// Each is an NSStackView so NDSettingsGroupView's SwiftUI grouped Form hosts
// it as a native row (normalizeNativeRow already special-cases NSStackView),
// and so a bare row outside a <settingsgroup> still lays out sanely.

/// `<row>`: leading icon + title/subtitle text column in the leading gravity,
/// suffix children in the trailing gravity, prefix children ahead of the text.
class NDRowView: NSStackView {
    let titleLabel = NSTextField(labelWithString: "")
    let subtitleLabel = NSTextField(labelWithString: "")
    private let textColumn = NSStackView()
    private var iconView: NSImageView?
    var ndNodeID: UInt32 = 0
    var ndActivatable = false

    init() {
        super.init(frame: .zero)
        orientation = .horizontal
        alignment = .centerY
        spacing = 10
        edgeInsets = .init()

        titleLabel.font = .systemFont(ofSize: NSFont.systemFontSize)
        titleLabel.lineBreakMode = .byTruncatingTail
        subtitleLabel.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        subtitleLabel.textColor = .secondaryLabelColor
        subtitleLabel.lineBreakMode = .byTruncatingTail
        subtitleLabel.isHidden = true

        textColumn.orientation = .vertical
        textColumn.alignment = .leading
        textColumn.spacing = 2
        textColumn.addArrangedSubview(titleLabel)
        textColumn.addArrangedSubview(subtitleLabel)
        addView(textColumn, in: .leading)
        setContentHuggingPriority(.defaultLow, for: .horizontal)
    }

    @available(*, unavailable)
    required init(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func applyTitle(_ title: String?) {
        if let t = title { titleLabel.stringValue = t }
    }

    func applySubtitle(_ subtitle: String?) {
        if let s = subtitle {
            subtitleLabel.stringValue = s
            subtitleLabel.isHidden = s.isEmpty
        }
    }

    /// The row's leading icon, created on first use and retargeted after
    /// that so an `iconData` update never stacks a second prefix icon.
    /// Raw image bytes carry their own colour, so only a symbol takes the
    /// secondary-label tint.
    func applyIcon(_ image: NSImage, tinted: Bool) {
        let iv = iconView ?? {
            let created = NSImageView()
            iconView = created
            packPrefix(created)
            return created
        }()
        iv.image = image
        iv.contentTintColor = tinted ? .secondaryLabelColor : nil
    }

    func packPrefix(_ child: NSView) {
        // Prefix views sit ahead of the text column within the leading gravity.
        insertView(child, at: 0, in: .leading)
    }

    func packSuffix(_ child: NSView) {
        addView(child, in: .trailing)
        // Track controls (NSSlider and friends) report no intrinsic width and
        // would collapse in the trailing gravity — give them a System
        // Settings-sized track.
        if child.intrinsicContentSize.width == NSView.noIntrinsicMetric {
            child.widthAnchor.constraint(equalToConstant: 160).isActive = true
        }
    }

    override func mouseUp(with event: NSEvent) {
        super.mouseUp(with: event)
        guard ndActivatable, ndNodeID != 0 else { return }
        let point = convert(event.locationInWindow, from: nil)
        guard bounds.contains(point) else { return }
        ndEmitEvent(ndNodeID, "activated", "{}")
    }
}

/// `<switchrow>`: an NDRowView whose fixed trailing control is an NSSwitch.
final class NDSwitchRowView: NDRowView {
    let toggle = NSSwitch()

    override init() {
        super.init()
        toggle.target = self
        toggle.action = #selector(toggled(_:))
        addView(toggle, in: .trailing)
    }

    @objc private func toggled(_ sender: NSSwitch) {
        guard ndNodeID != 0, !ndIsEchoSuppressed(self) else { return }
        ndEmitEvent(ndNodeID, "toggled", "{\"checked\":\(sender.state == .on)}")
    }
}

func makeRow(_ props: [String: Any]) -> NSView {
    let row = NDRowView()
    row.applyTitle(propStr(props, "title") ?? "")
    row.applySubtitle(propStr(props, "subtitle"))
    if let icon = propStr(props, "iconName"), let image = ndResolveSymbolImage(icon) {
        row.applyIcon(image, tinted: true)
    }
    // Image bytes beat a symbol name: a favicon has no SF Symbol, and this is
    // the only way a browser sidebar row can show one.
    ndRowApplyIconData(row, props)
    row.ndActivatable = propBool(props, "activatable") ?? false
    return row
}

/// Row.iconData, shared by the create and update arms. 16pt matches the
/// symbol the `iconName` path renders (and SourceTrees' own icon column).
private func ndRowApplyIconData(_ row: NDRowView, _ props: [String: Any]) {
    guard let data = propStr(props, "iconData"),
          let image = ndIconImageFromData(data, side: 16, what: "Row")
    else { return }
    row.applyIcon(image, tinted: false)
}

func makeSwitchRow(_ props: [String: Any]) -> NSView {
    let row = NDSwitchRowView()
    row.applyTitle(propStr(props, "title") ?? "")
    row.applySubtitle(propStr(props, "subtitle"))
    row.toggle.state = (propBool(props, "checked") ?? false) ? .on : .off
    return row
}

/// Generated Row applyProps arm: title/subtitle/iconData merged.
func ndRowApply(_ view: NSView, _ props: [String: Any]) {
    guard let row = view as? NDRowView else { return }
    row.applyTitle(propStr(props, "title"))
    row.applySubtitle(propStr(props, "subtitle"))
    ndRowApplyIconData(row, props)
}

/// Generated SwitchRow applyProps arm: title/subtitle/checked merged; the
/// checked write replays a state the app already owns, so it never re-emits.
func ndSwitchRowApply(_ view: NSView, _ props: [String: Any]) {
    guard let row = view as? NDSwitchRowView else { return }
    row.applyTitle(propStr(props, "title"))
    row.applySubtitle(propStr(props, "subtitle"))
    if let checked = propBool(props, "checked") {
        let want: NSControl.StateValue = checked ? .on : .off
        if row.toggle.state != want {
            withEchoSuppressed(row) { row.toggle.state = want }
        }
    }
}

/// Generated Row structural arms (slot-addressed; `before` plays no part).
func ndRowPack(_ row: NDRowView, _ child: NSView, slot: String) {
    if slot == "prefix" {
        row.packPrefix(child)
    } else {
        row.packSuffix(child)
    }
}

func ndRowUnpack(_ row: NDRowView, _ child: NSView) {
    row.removeView(child)
}

/// Generated ndConnectEvents arms (webview idiom: record the id; the view
/// emits directly).
func ndRowConnect(_ view: NSView, nodeID: UInt32) {
    (view as? NDRowView)?.ndNodeID = nodeID
}

func ndSwitchRowConnect(_ view: NSView, nodeID: UInt32) {
    (view as? NDSwitchRowView)?.ndNodeID = nodeID
}
