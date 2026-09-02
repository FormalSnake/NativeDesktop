import AppKit
import SwiftUI

/// React rows currently hosted by a native grouped Form. AppKit style updates
/// consult this registry so shared GTK-oriented padding cannot be reapplied
/// after attachment and double SwiftUI's native row insets.
nonisolated(unsafe) private var ndSettingsGroupRows: Set<ObjectIdentifier> = []

func ndUsesNativeSettingsInsets(_ view: NSView) -> Bool {
    view is NDSettingsGroupView || ndSettingsGroupRows.contains(ObjectIdentifier(view))
}

/// release_node purge seam (Backend.swift's `ndPurgeNodeRegistries`).
func ndSettingsGroupPurge(_ view: NSView) {
    ndSettingsGroupRows.remove(ObjectIdentifier(view))
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
    let title: String
    let subtitle: String

    var body: some View {
        Form {
            Section {
                ForEach(Array(rows.enumerated()), id: \.element) { _, row in
                    NDSettingsGroupRow(view: row)
                }
            } header: {
                if !title.isEmpty { Text(title) }
            } footer: {
                if !subtitle.isEmpty { Text(subtitle) }
            }
        }
        .formStyle(.grouped)
        // The shared content column already owns its pane inset. Grouped Form
        // still keeps a native 20 pt section gutter on every side even with
        // zero scroll-content margins, so collapse it symmetrically: the card's
        // visible bounds then equal this host view's frame, and the surrounding
        // tree's padding/spacing is the ONLY thing that positions it — no
        // phantom strip above the first card, no extra gap to a sibling like
        // Reset, and no lopsided negative inset that would clip a card's top.
        .contentMargins(.all, 0, for: .scrollContent)
        .padding(-20)
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
    /// AdwPreferencesGroup parity: `title` renders as the Section header,
    /// `description` as its footer text (set at create + applyProps).
    var ndTitle: String = "" {
        didSet { if ndTitle != oldValue { refresh() } }
    }
    var ndDescription: String = "" {
        didSet { if ndDescription != oldValue { refresh() } }
    }
    private lazy var host = NSHostingView(rootView: NDSettingsGroupSurface(rows: rows, title: ndTitle, subtitle: ndDescription))

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

    /// NDBoxView places this view by frame, and an NSStackView reports no
    /// intrinsic size of its own, so what the box measures is this view's
    /// `fittingSize`: the hosted Form's ideal height for the width the SwiftUI
    /// body last laid out at. The host's own width constraint does not catch
    /// up until the next constraint pass, so without pushing the width through
    /// here a measurement taken in between answers with the Form wrapped to a
    /// narrower column, thousands of points tall.
    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        guard newSize.width > 0, host.frame.width != newSize.width else { return }
        host.setFrameSize(NSSize(width: newSize.width, height: host.frame.height))
        host.layoutSubtreeIfNeeded()
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
        // NDRowView/NDSwitchRowView own their internal layout.
        if view is NDRowView { return }
        if let box = view as? NDBoxView { box.ndPadding = .init() }
    }

    private func refresh() {
        host.rootView = NDSettingsGroupSurface(rows: rows, title: ndTitle, subtitle: ndDescription)
        invalidateIntrinsicContentSize()
    }
}

/// Generated SettingsGroup applyProps arm: title/description merged.
func ndSettingsGroupApply(_ view: NSView, _ props: [String: Any]) {
    guard let group = view as? NDSettingsGroupView else { return }
    if let t = propStr(props, "title") { group.ndTitle = t }
    if let d = propStr(props, "description") { group.ndDescription = d }
}
