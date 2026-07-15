import AppKit

/// Expander (M15): NSButton `.pushDisclosure` (chevron — panel expansion per
/// the HIG disclosure-controls page) + a collapsible single-child section,
/// animated via the standard NSStackView hidden-arranged-subview pattern.
/// `expanded` is controlled: user toggles flip natively AND emit `toggled`
/// {checked}; React-driven updates ride withEchoSuppressed.
final class NDExpanderView: NSStackView {
    var nodeID: UInt32 = 0
    let disclosure = NSButton()
    let labelField = NSTextField(labelWithString: "")
    let content = FlippedView()
    private(set) var expanded = false

    init(label: String, expanded: Bool) {
        super.init(frame: .zero)
        orientation = .vertical
        alignment = .leading
        spacing = 6

        disclosure.bezelStyle = .pushDisclosure
        disclosure.setButtonType(.pushOnPushOff)
        disclosure.title = ""
        disclosure.target = self
        disclosure.action = #selector(togglePressed(_:))
        labelField.stringValue = label

        let header = NSStackView()
        header.orientation = .horizontal
        header.spacing = 6
        header.alignment = .centerY
        header.addArrangedSubview(disclosure)
        header.addArrangedSubview(labelField)
        addArrangedSubview(header)

        addArrangedSubview(content)
        content.widthAnchor.constraint(equalTo: widthAnchor).isActive = true

        // The label is a click target too (GtkExpander parity).
        let click = NSClickGestureRecognizer(target: self, action: #selector(labelClicked(_:)))
        labelField.addGestureRecognizer(click)

        self.expanded = expanded
        disclosure.state = expanded ? .on : .off
        content.isHidden = !expanded
    }

    required init?(coder: NSCoder) { fatalError("NDExpanderView is not NSCoding-decodable") }

    @objc private func togglePressed(_ sender: NSButton) {
        setExpanded(sender.state == .on, animated: true, emit: true)
    }

    @objc private func labelClicked(_ sender: Any?) {
        setExpanded(!expanded, animated: true, emit: true)
    }

    func setExpanded(_ e: Bool, animated: Bool, emit: Bool) {
        guard e != expanded || content.isHidden == e else {
            // Same logical state but keep the button glyph honest.
            disclosure.state = e ? .on : .off
            return
        }
        expanded = e
        disclosure.state = e ? .on : .off
        if animated {
            NSAnimationContext.runAnimationGroup { ctx in
                ctx.duration = 0.2
                ctx.allowsImplicitAnimation = true
                content.isHidden = !e
                self.window?.layoutIfNeeded()
            }
        } else {
            content.isHidden = !e
        }
        if emit, !ndIsEchoSuppressed(self) {
            ndEmitEvent(nodeID, "toggled", "{\"checked\":\(e)}")
        }
    }

    /// Single-child slot (generated structural Expander arms).
    func setContentChild(_ child: NSView) {
        content.subviews.forEach { $0.removeFromSuperview() }
        child.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(child)
        NSLayoutConstraint.activate([
            child.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            child.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            child.topAnchor.constraint(equalTo: content.topAnchor),
            child.bottomAnchor.constraint(equalTo: content.bottomAnchor),
        ])
    }

    func clearContentChild(_ child: NSView) {
        if child.superview === content { child.removeFromSuperview() }
    }
}

/// `ndCreate`'s Expander arm (generated) calls this.
func makeExpander(_ props: [String: Any]) -> NSView {
    NDExpanderView(
        label: propStr(props, "label") ?? "",
        expanded: propBool(props, "expanded") ?? false
    )
}

/// Generated ndApplyProps Expander arm — merged apply.
func ndExpanderApply(_ view: NSView, _ props: [String: Any]) {
    guard let ex = view as? NDExpanderView else { return }
    if let l = propStr(props, "label") { ex.labelField.stringValue = l }
    if let e = propBool(props, "expanded"), e != ex.expanded {
        withEchoSuppressed(view) { ex.setExpanded(e, animated: ex.window != nil, emit: true) }
    }
}

/// Generated ndConnectEvents Expander arm.
func ndExpanderConnect(_ view: NSView, nodeID: UInt32) {
    (view as? NDExpanderView)?.nodeID = nodeID
}
