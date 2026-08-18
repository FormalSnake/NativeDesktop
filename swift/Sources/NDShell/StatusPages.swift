import AppKit
import SwiftUI

/// StatusPage: SwiftUI `ContentUnavailableView`, the native empty-state
/// primitive (macOS 14+), hosted through NDHostedLeaf. Action buttons are
/// React-owned NSViews wrapped in `NDNativeChild` and placed straight in
/// `ContentUnavailableView`'s own `actions` slot rather than a separate
/// NSStackView bolted on below — the same centered column the GTK backend's
/// AdwStatusPage draws, but the actions row is now real SwiftUI layout
/// instead of hand-pinned constraints.
struct NDStatusPageBody: View {
    var iconSymbol: String?
    var title: String
    var descriptionText: String?
    var actionViews: [NSView]

    var body: some View {
        ContentUnavailableView(
            label: {
                if let iconSymbol {
                    Label(title, systemImage: iconSymbol)
                } else {
                    Text(title)
                }
            },
            description: {
                if let descriptionText, !descriptionText.isEmpty {
                    Text(descriptionText)
                }
            },
            actions: {
                HStack(spacing: 8) {
                    ForEach(Array(actionViews.enumerated()), id: \.element) { _, view in
                        NDNativeChild(view: view)
                    }
                }
            })
    }
}

final class NDStatusPageView: NDHostedLeaf {
    private var iconName: String?
    private var title = ""
    private var desc: String?
    private var actionViews: [NSView] = []

    func apply(iconName: String?, title: String?, description: String?) {
        if let iconName { self.iconName = iconName }
        if let title { self.title = title }
        if let description { self.desc = description }
        refreshLeaf()
    }

    override func leafContent() -> AnyView {
        let symbol = iconName.map { ndSFSymbol(forFreedesktop: $0) ?? $0 }
        return AnyView(NDStatusPageBody(iconSymbol: symbol, title: title, descriptionText: desc, actionViews: actionViews))
    }

    func pack(_ child: NSView, before: NSView?) {
        actionViews.removeAll { $0 === child }
        if let before, let idx = actionViews.firstIndex(of: before) {
            actionViews.insert(child, at: idx)
        } else {
            actionViews.append(child)
        }
        refreshLeaf()
    }

    func unpack(_ child: NSView) {
        actionViews.removeAll { $0 === child }
        child.removeFromSuperview()
        refreshLeaf()
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
