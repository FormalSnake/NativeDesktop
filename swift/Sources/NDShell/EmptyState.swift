import AppKit

// Shared empty-state chrome for the data-view widgets (SourceList / ListView /
// Table / TreeView / SourceTree) — the AppKit peer of src/gtk/emptystate.zig.
// When the item array is empty AND the app set any of emptyIconName /
// emptyTitle / emptyDescription, a centered icon+title+description overlay
// shows over the scroll view. Opt-in per widget: no props, no overlay.

private final class NDEmptyStateEntry {
    var icon: String = ""
    var title: String = ""
    var desc: String = ""
    var overlay: NSStackView?
    var isEmpty = false
    var configured: Bool { !icon.isEmpty || !title.isEmpty || !desc.isEmpty }
}

nonisolated(unsafe) private var ndEmptyStates: [ObjectIdentifier: NDEmptyStateEntry] = [:]

private func entry(for view: NSView) -> NDEmptyStateEntry {
    let key = ObjectIdentifier(view)
    if let existing = ndEmptyStates[key] { return existing }
    let fresh = NDEmptyStateEntry()
    ndEmptyStates[key] = fresh
    return fresh
}

/// Generated applyProps arm (merged) AND create-time seed: records the three
/// props (a diffed update carries only changed keys; "" clears one) and
/// refreshes the overlay in place.
func ndEmptyStateApply(_ view: NSView, _ props: [String: Any]) {
    let e = entry(for: view)
    if let icon = propStr(props, "emptyIconName") { e.icon = icon }
    if let title = propStr(props, "emptyTitle") { e.title = title }
    if let desc = propStr(props, "emptyDescription") { e.desc = desc }
    refresh(view, e)
}

/// Called by each data-view module wherever its item array lands.
func ndEmptyStateUpdate(_ view: NSView, isEmpty: Bool) {
    let e = entry(for: view)
    e.isEmpty = isEmpty
    refresh(view, e)
}

private func refresh(_ view: NSView, _ e: NDEmptyStateEntry) {
    guard e.isEmpty, e.configured else {
        e.overlay?.removeFromSuperview()
        e.overlay = nil
        return
    }
    let stack = e.overlay ?? {
        let s = NSStackView()
        s.orientation = .vertical
        s.alignment = .centerX
        s.spacing = 6
        s.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(s)
        NSLayoutConstraint.activate([
            s.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            s.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            s.leadingAnchor.constraint(greaterThanOrEqualTo: view.leadingAnchor, constant: 20),
        ])
        e.overlay = s
        return s
    }()
    stack.arrangedSubviews.forEach { $0.removeFromSuperview() }
    if !e.icon.isEmpty, let image = ndResolveSymbolImage(e.icon) {
        let iv = NSImageView(image: image)
        iv.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 34, weight: .regular)
        iv.contentTintColor = .tertiaryLabelColor
        stack.addArrangedSubview(iv)
    }
    if !e.title.isEmpty {
        let title = NSTextField(labelWithString: e.title)
        title.font = .systemFont(ofSize: NSFont.systemFontSize + 2, weight: .semibold)
        title.textColor = .secondaryLabelColor
        title.alignment = .center
        stack.addArrangedSubview(title)
    }
    if !e.desc.isEmpty {
        let desc = NSTextField(wrappingLabelWithString: e.desc)
        desc.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        desc.textColor = .tertiaryLabelColor
        desc.alignment = .center
        stack.addArrangedSubview(desc)
    }
}
