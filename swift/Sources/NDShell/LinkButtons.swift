import AppKit
import SwiftUI

/// LinkButton (M15): SwiftUI `Button` + `.buttonStyle(.link)` hosted in an
/// NSHostingView — AppKit has no link primitive (the `.inline` bezel is the
/// badge style and deprecated; design brief 2026-07). Cross-platform policy
/// (GTK-agent binding): `activate` ALWAYS fires (payload = uri); the native
/// URL open only runs when `openExternal` is true. `visited` is app-owned
/// state approximated with system purple (no native visited-link color).
struct NDLinkButtonBody: View {
    var label: String
    var visited: Bool
    var onClick: () -> Void

    var body: some View {
        Button(action: onClick) {
            Text(label).underline()
        }
        .buttonStyle(.link)
        .foregroundStyle(visited ? Color(nsColor: .systemPurple) : Color(nsColor: .linkColor))
        .onHover { inside in
            if inside { NSCursor.pointingHand.push() } else { NSCursor.pop() }
        }
    }
}

final class NDLinkButtonView: NSHostingView<NDLinkButtonBody> {
    var nodeID: UInt32 = 0
    var label = ""
    var uri = ""
    var visited = false
    var openExternal = false

    required init(rootView: NDLinkButtonBody) {
        super.init(rootView: rootView)
        // Leaf widget inside an AppKit Auto Layout tree: behave like an
        // intrinsic-sized NSControl (design brief NSHostingView practice).
        sizingOptions = [.intrinsicContentSize]
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("NDLinkButtonView is not NSCoding-decodable") }

    func refresh() {
        rootView = NDLinkButtonBody(
            label: label.isEmpty ? uri : label,
            visited: visited,
            onClick: { [weak self] in self?.clicked() }
        )
    }

    private func clicked() {
        ndEmitEvent(nodeID, "activate", "{\"text\":\(ndJsonString(uri))}")
        if openExternal, let real = URL(string: uri) {
            NSWorkspace.shared.open(real)
        }
    }
}

/// `ndCreate`'s LinkButton arm (generated) calls this.
func makeLinkButton(_ props: [String: Any]) -> NSView {
    let v = NDLinkButtonView(rootView: NDLinkButtonBody(label: "", visited: false, onClick: {}))
    ndLinkButtonApply(v, props)
    return v
}

/// Generated ndApplyProps LinkButton arm — one merged apply for all four
/// createAndUpdate props (diffed updates carry only changed keys; absent
/// keys keep prior state).
func ndLinkButtonApply(_ view: NSView, _ props: [String: Any]) {
    guard let v = view as? NDLinkButtonView else { return }
    if let l = propStr(props, "label") { v.label = l }
    if let u = propStr(props, "uri") { v.uri = u }
    if let vis = propBool(props, "visited") { v.visited = vis }
    if let oe = propBool(props, "openExternal") { v.openExternal = oe }
    v.refresh()
}

/// Generated ndConnectEvents LinkButton arm.
func ndLinkButtonConnect(_ view: NSView, nodeID: UInt32) {
    (view as? NDLinkButtonView)?.nodeID = nodeID
}
