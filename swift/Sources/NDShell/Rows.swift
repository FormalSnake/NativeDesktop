import AppKit
import SwiftUI

// AppKit peers of libadwaita's boxed-list rows (`<row>` / `<switchrow>`),
// hosted as SwiftUI `LabeledContent` (SwiftUILeaves.swift).
//
// Why not an NSStackView any more: a hand-built row has to decide for itself
// where the label column ends and the control column begins, and it decided
// differently per row — a suffix that could stretch filled the card, one that
// could not sat hard against the text, and a leading icon pushed the title
// out of line with its neighbours. LabeledContent is the API that owns that
// split on macOS, so the rows line up with each other and with System
// Settings without the toolkit measuring anything.
//
// The app's child control still arrives from the React tree as an NSView and
// stays one: it is placed through NDNativeChild, never rebuilt in SwiftUI.

/// `<row>`: leading icon + title/subtitle text column as the label, suffix
/// children as the content, prefix children ahead of the text.
class NDRowView: NDHostedLeaf {
    var ndActivatable = false

    private var title = ""
    private var subtitle = ""
    private var icon: NSImage?
    // A symbol (iconName) takes the row's secondary-label tint; raw image
    // bytes (iconData) carry their own colour and render as given.
    private var iconTinted = true
    private var prefixViews: [NSView] = []
    private var suffixViews: [NSView] = []

    func applyTitle(_ title: String?) {
        guard let title, title != self.title else { return }
        self.title = title
        refreshLeaf()
    }

    func applySubtitle(_ subtitle: String?) {
        guard let subtitle, subtitle != self.subtitle else { return }
        self.subtitle = subtitle
        refreshLeaf()
    }

    func applyIcon(_ image: NSImage?, tinted: Bool = true) {
        icon = image
        iconTinted = tinted
        refreshLeaf()
    }

    func packPrefix(_ child: NSView) {
        // Prefix children sit ahead of the icon and the text column, which is
        // where the AppKit stack put them.
        prefixViews.insert(child, at: 0)
        refreshLeaf()
    }

    func packSuffix(_ child: NSView) {
        suffixViews.append(child)
        refreshLeaf()
    }

    func unpack(_ child: NSView) {
        prefixViews.removeAll { $0 === child }
        suffixViews.removeAll { $0 === child }
        child.removeFromSuperview()
        refreshLeaf()
    }

    override func leafContent() -> AnyView {
        AnyView(
            LabeledContent {
                HStack(spacing: 8) { children(suffixViews) }
            } label: {
                HStack(spacing: 8) {
                    children(prefixViews)
                    if let icon {
                        if iconTinted {
                            Image(nsImage: icon).foregroundStyle(.secondary)
                        } else {
                            Image(nsImage: icon)
                        }
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        Text(title)
                        if !subtitle.isEmpty {
                            Text(subtitle).font(.subheadline).foregroundStyle(.secondary)
                        }
                    }
                }
            })
    }

    @ViewBuilder private func children(_ views: [NSView]) -> some View {
        ForEach(Array(views.enumerated()), id: \.element) { _, view in
            NDNativeChild(view: view, minWidth: ndChildNeedsWidth(view) ? 160 : 0)
                .frame(minWidth: ndChildNeedsWidth(view) ? 160 : nil)
        }
    }

    /// The row affordance. SwiftUI hit-testing hands clicks that miss a real
    /// control back to the hosting view, so this still only fires on the row
    /// itself and never on the control it holds.
    override func mouseUp(with event: NSEvent) {
        super.mouseUp(with: event)
        guard ndActivatable, ndNodeID != 0 else { return }
        let point = convert(event.locationInWindow, from: nil)
        guard bounds.contains(point) else { return }
        ndEmitEvent(ndNodeID, "activated", "{}")
    }
}

/// `<switchrow>`: an NDRowView whose fixed content is an NSSwitch. The switch
/// stays a real AppKit control rather than a SwiftUI `Toggle` so the a11y
/// probe, `semanticClick` and `semanticSetValue` keep reading and driving the
/// same object they always did.
final class NDSwitchRowView: NDRowView {
    let toggle = NSSwitch()

    required init(rootView: AnyView) {
        super.init(rootView: rootView)
        toggle.target = self
        toggle.action = #selector(toggled(_:))
        packSuffix(toggle)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("NDSwitchRowView is not NSCoding-decodable") }

    @objc private func toggled(_ sender: NSSwitch) {
        guard ndNodeID != 0, !ndIsEchoSuppressed(self) else { return }
        ndEmitEvent(ndNodeID, "toggled", "{\"checked\":\(sender.state == .on)}")
    }
}

func makeRow(_ props: [String: Any]) -> NSView {
    let row = NDRowView()
    row.applyTitle(propStr(props, "title") ?? "")
    row.applySubtitle(propStr(props, "subtitle"))
    if let icon = propStr(props, "iconName") {
        row.applyIcon(ndResolveSymbolImage(icon))
    }
    // Image bytes beat a symbol name: a favicon has no SF Symbol, and this is
    // the only way a browser sidebar row can show one.
    ndRowApplyIconData(row, props)
    row.ndActivatable = propBool(props, "activatable") ?? false
    row.refreshLeaf()
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
    row.refreshLeaf()
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
    row.unpack(child)
}

/// Generated ndConnectEvents arms (webview idiom: record the id; the view
/// emits directly).
func ndRowConnect(_ view: NSView, nodeID: UInt32) {
    (view as? NDRowView)?.ndNodeID = nodeID
}

func ndSwitchRowConnect(_ view: NSView, nodeID: UInt32) {
    (view as? NDSwitchRowView)?.ndNodeID = nodeID
}
