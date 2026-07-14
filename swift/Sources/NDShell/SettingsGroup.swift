import AppKit
import SwiftUI

/// React rows currently hosted by a native grouped Form. AppKit style updates
/// consult this registry so shared GTK-oriented padding cannot be reapplied
/// after attachment and double SwiftUI's native row insets.
nonisolated(unsafe) private var ndSettingsGroupRows: Set<ObjectIdentifier> = []

func ndUsesNativeSettingsInsets(_ view: NSView) -> Bool {
    view is NDSettingsGroupView || ndSettingsGroupRows.contains(ObjectIdentifier(view))
}

/// Bridges an existing React-owned AppKit view into a native SwiftUI form row.
/// SwiftUI controls only placement: identity, props, and event handlers remain
/// attached to the original `NSView` instance.
private struct NDSettingsGroupRow: NSViewRepresentable {
    let view: NSView

    func makeNSView(context: Context) -> NSView {
        view
    }

    func updateNSView(_ nsView: NSView, context: Context) {}
}

private struct NDSettingsGroupSurface: View {
    let rows: [NSView]

    var body: some View {
        Form {
            Section {
                ForEach(Array(rows.enumerated()), id: \.element) { _, row in
                    NDSettingsGroupRow(view: row)
                }
            }
        }
        .formStyle(.grouped)
        .scrollDisabled(true)
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// Semantic settings container for the AppKit backend.
///
/// Unlike a painted card approximation, this uses SwiftUI's native grouped
/// `Form`/`Section` style—the same platform API intended for settings UI. React
/// continues to own every child `NSView`; the hosting view only arranges those
/// existing handles as native form rows.
final class NDSettingsGroupView: NSStackView {
    private var rows: [NSView] = []
    private lazy var host = NSHostingView(rootView: NDSettingsGroupSurface(rows: rows))

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        orientation = .vertical
        alignment = .leading
        distribution = .gravityAreas
        spacing = 0

        host.translatesAutoresizingMaskIntoConstraints = false
        host.sizingOptions = [.intrinsicContentSize]
        super.addArrangedSubview(host)
        host.widthAnchor.constraint(equalTo: widthAnchor).isActive = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func appendReactView(_ view: NSView) {
        guard !isStructuralSeparator(view) else { return }
        normalizeNativeRow(view)
        rows.append(view)
        refresh()
    }

    func insertReactView(_ view: NSView, before sibling: NSView) {
        guard !isStructuralSeparator(view) else { return }
        normalizeNativeRow(view)
        rows.removeAll { $0 === view }
        let index = rows.firstIndex { $0 === sibling } ?? rows.endIndex
        rows.insert(view, at: index)
        refresh()
    }

    func removeReactView(_ view: NSView) {
        guard !isStructuralSeparator(view) else { return }
        rows.removeAll { $0 === view }
        ndSettingsGroupRows.remove(ObjectIdentifier(view))
        view.removeFromSuperview()
        refresh()
    }

    private func isStructuralSeparator(_ view: NSView) -> Bool {
        guard let box = view as? NSBox else { return false }
        return box.boxType == .separator
    }

    /// A grouped Form owns its row insets. Shared trees may carry padding for
    /// GTK's boxed-list implementation; retaining it here doubles the native
    /// vertical inset and makes every macOS row taller than System Settings.
    private func normalizeNativeRow(_ view: NSView) {
        ndSettingsGroupRows.insert(ObjectIdentifier(view))
        guard let stack = view as? NSStackView else { return }
        stack.edgeInsets = .init()
        ndBoxReconcileChildren(stack)
    }

    private func refresh() {
        host.rootView = NDSettingsGroupSurface(rows: rows)
        invalidateIntrinsicContentSize()
    }
}
