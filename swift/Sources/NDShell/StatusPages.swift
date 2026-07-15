import AppKit
import SwiftUI

/// StatusPage (M15): SwiftUI `ContentUnavailableView` — the exact native
/// empty-state primitive (macOS 14+) — hosted in an NSHostingView, with the
/// React children (action buttons) mounted into a plain horizontal
/// NSStackView BELOW it rather than inside the SwiftUI `actions` builder:
/// the children are live NSViews the reconciler owns, and re-wrapping them in
/// NSViewRepresentable would add a sizing bridge for zero visual gain. The
/// centered column mirrors the GTK backend's AdwStatusPage + inner action box.
struct NDStatusPageBody: View {
    var iconSymbol: String?
    var title: String
    var descriptionText: String?

    var body: some View {
        ContentUnavailableView {
            if let iconSymbol {
                Label(title, systemImage: iconSymbol)
            } else {
                Text(title)
            }
        } description: {
            if let descriptionText, !descriptionText.isEmpty {
                Text(descriptionText)
            }
        }
    }
}

final class NDStatusPageView: NSView {
    private var iconName: String?
    private var title = ""
    private var desc: String?
    private var hosting: NSHostingView<NDStatusPageBody>!
    private let column = NSStackView()
    let actions = NSStackView()

    override var isFlipped: Bool { true }

    init() {
        super.init(frame: .zero)
        hosting = NSHostingView(rootView: NDStatusPageBody(iconSymbol: nil, title: "", descriptionText: nil))
        hosting.sizingOptions = [.intrinsicContentSize]

        actions.orientation = .horizontal
        actions.spacing = 8

        column.orientation = .vertical
        column.alignment = .centerX
        column.spacing = 12
        column.translatesAutoresizingMaskIntoConstraints = false
        column.addArrangedSubview(hosting)
        column.addArrangedSubview(actions)
        addSubview(column)
        // The top/bottom >= pins give the page a real minimum height (its
        // centered column would otherwise paint outside a zero-height frame
        // when stacked without vexpand).
        NSLayoutConstraint.activate([
            column.centerXAnchor.constraint(equalTo: centerXAnchor),
            column.centerYAnchor.constraint(equalTo: centerYAnchor),
            column.leadingAnchor.constraint(greaterThanOrEqualTo: leadingAnchor, constant: 16),
            column.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -16),
            column.topAnchor.constraint(greaterThanOrEqualTo: topAnchor, constant: 24),
            column.bottomAnchor.constraint(lessThanOrEqualTo: bottomAnchor, constant: -24),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("NDStatusPageView is not NSCoding-decodable") }

    func apply(iconName: String?, title: String?, description: String?) {
        if let iconName { self.iconName = iconName }
        if let title { self.title = title }
        if let description { self.desc = description }
        let symbol = self.iconName.map { ndSFSymbol(forFreedesktop: $0) ?? $0 }
        hosting.rootView = NDStatusPageBody(iconSymbol: symbol, title: self.title, descriptionText: self.desc)
    }

    func pack(_ child: NSView, before: NSView?) {
        if actions.arrangedSubviews.contains(child) { actions.removeArrangedSubview(child) }
        if let before, let idx = actions.arrangedSubviews.firstIndex(of: before) {
            actions.insertArrangedSubview(child, at: idx)
        } else {
            actions.addArrangedSubview(child)
        }
    }

    func unpack(_ child: NSView) {
        actions.removeArrangedSubview(child)
        child.removeFromSuperview()
    }
}

/// `ndCreate`'s StatusPage arm (generated) calls this.
func makeStatusPage(_ props: [String: Any]) -> NSView {
    let page = NDStatusPageView()
    ndStatusPageApply(page, props)
    return page
}

/// Generated ndApplyProps StatusPage arm — merged apply.
func ndStatusPageApply(_ view: NSView, _ props: [String: Any]) {
    (view as? NDStatusPageView)?.apply(
        iconName: propStr(props, "iconName"),
        title: propStr(props, "title"),
        description: propStr(props, "description")
    )
}

/// Generated structural StatusPage arms.
func ndStatusPagePack(_ parent: NSView, _ child: NSView, before: NSView?) {
    (parent as? NDStatusPageView)?.pack(child, before: before)
}

func ndStatusPageUnpack(_ parent: NSView, _ child: NSView) {
    (parent as? NDStatusPageView)?.unpack(child)
}
