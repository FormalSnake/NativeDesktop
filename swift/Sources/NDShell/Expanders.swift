import AppKit
import SwiftUI

/// Expander: SwiftUI `DisclosureGroup`, the system disclosure control (its
/// whole label row is a click target natively, so no extra gesture
/// recognizer is needed for GtkExpander's "click the label too" parity).
/// `expanded` is controlled: user toggles emit `toggled` {checked} and
/// re-render; React-driven updates ride withEchoSuppressed and re-render too
/// — this class has no persistent SwiftUI state of its own, so every change
/// goes through `refreshLeaf()` the same way Row/Chart do.
final class NDExpanderView: NDHostedLeaf {
    private(set) var expanded = false
    private var label = ""
    private var childView: NSView?

    func applyCreate(label: String, expanded: Bool) {
        self.label = label
        self.expanded = expanded
        refreshLeaf()
    }

    func setLabel(_ label: String) {
        self.label = label
        refreshLeaf()
    }

    /// React-driven `expanded` write, echo-suppressed like every other
    /// controlled prop so the app's own round-trip never re-emits `toggled`.
    func setExpandedFromProps(_ e: Bool) {
        guard e != expanded else { return }
        withEchoSuppressed(self) { setExpanded(e, emit: false) }
    }

    private func setExpanded(_ e: Bool, emit: Bool) {
        expanded = e
        refreshLeaf()
        if emit, !ndIsEchoSuppressed(self) {
            ndEmitEvent(ndNodeID, "toggled", "{\"checked\":\(e)}")
        }
    }

    /// Single-child slot (generated structural Expander arms).
    func setContentChild(_ child: NSView) {
        childView = child
        refreshLeaf()
    }

    func clearContentChild(_ child: NSView) {
        if childView === child { childView = nil }
        refreshLeaf()
    }

    override func leafContent() -> AnyView {
        AnyView(
            DisclosureGroup(isExpanded: Binding(
                get: { [weak self] in self?.expanded ?? false },
                set: { [weak self] in self?.setExpanded($0, emit: true) }
            )) {
                if let childView = self.childView { NDNativeChild(view: childView) }
            } label: {
                Text(label)
            })
    }
}

/// `ndCreate`'s Expander arm (generated) calls this.
func makeExpander(_ props: [String: Any]) -> NSView {
    let expander = NDExpanderView()
    expander.applyCreate(label: propStr(props, "label") ?? "", expanded: propBool(props, "expanded") ?? false)
    return expander
}

/// Generated ndApplyProps Expander arm — merged apply.
func ndExpanderApply(_ view: NSView, _ props: [String: Any]) {
    guard let ex = view as? NDExpanderView else { return }
    if let l = propStr(props, "label") { ex.setLabel(l) }
    if let e = propBool(props, "expanded") { ex.setExpandedFromProps(e) }
}

/// Generated ndConnectEvents Expander arm.
func ndExpanderConnect(_ view: NSView, nodeID: UInt32) {
    (view as? NDExpanderView)?.ndNodeID = nodeID
}
