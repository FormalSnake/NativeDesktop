import AppKit

/// SourceTree: a `.sourceList`-style NSOutlineView in an NSScrollView
/// (tracked handle = scroll view, SourceList convention) — the hierarchical
/// sidebar with per-row trailing actions. The schema's flat SourceTreeNode
/// list (id/parentId) is grouped into a parent index and rebuilt on every
/// `nodes` update, REUSING item instances by id — NSOutlineView tracks
/// expansion by item identity, so stable instances keep open branches open
/// across React updates (same contract as TreeViews.swift). Events carry
/// `{ nodeId }` (actionClicked adds `actionId`); `selectedId` is the
/// controlled selection prop, "" meaning no selection. `section` nodes are
/// native group rows (isGroupItem, unselectable, no outline cell).
private let ndSourceTreeCellID = NSUserInterfaceItemIdentifier("nd-sourcetree-cell")
/// Section rows recycle in their own pool: they are unselectable and carry
/// their own smaller semibold type, so keeping them out of the selectable
/// rows' pool means a recycled cell never has to unwind a section's styling
/// onto a row that can be selected.
private let ndSourceTreeSectionCellID = NSUserInterfaceItemIdentifier("nd-sourcetree-section-cell")

/// `@unchecked Sendable` for the same reason as NDTreeItem: items cross into
/// MainActor AppKit calls from the nominally nonisolated apply funcs, and the
/// ABI contract guarantees every touch happens on the UI thread.
final class NDSourceTreeItem: NSObject, @unchecked Sendable {
    var nodeId: String = ""
    var title: String = ""
    var caption: String?
    var iconName: String?
    var iconData: String?
    var captionIconName: String?
    var badge: String?
    var section = false
    var hasChildren = false
    var expandedFlag = false
    var selectable = true
    var actionIds: [String] = []
    var testID: String?
    var children: [NDSourceTreeItem] = []
}

struct NDSourceTreeAction {
    var id: String
    var iconName: String
    var label: String?
    var tooltip: String?
    var destructive: Bool
}

/// Trailing-action button: carries its node/action ids so the shared click
/// handler needs no per-button closure state.
final class NDSourceTreeActionButton: NSButton {
    var nodeId = ""
    var actionId = ""
}

/// Row view with hover tracking: `actionVisibility: "hover"` shows the
/// trailing action buttons only while the pointer is over the row.
final class NDSourceTreeRowView: NSTableRowView {
    var hoverActions = false

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        for area in trackingAreas where area.owner === self { removeTrackingArea(area) }
        guard hoverActions else { return }
        addTrackingArea(NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeInKeyWindow], owner: self))
    }

    private func setActionsVisible(_ visible: Bool) {
        for sub in subviews {
            (sub as? NDSourceTreeCell)?.setActionsVisible(visible)
        }
    }

    override func mouseEntered(with event: NSEvent) {
        super.mouseEntered(with: event)
        setActionsVisible(true)
    }

    override func mouseExited(with event: NSEvent) {
        super.mouseExited(with: event)
        setActionsVisible(false)
    }
}

final class NDSourceTreeDataSource: NSObject, NSOutlineViewDataSource, NSOutlineViewDelegate {
    var nodeID: UInt32 = 0
    var roots: [NDSourceTreeItem] = []
    var itemsByID: [String: NDSourceTreeItem] = [:]
    var actions: [String: NDSourceTreeAction] = [:]
    var hoverActions = true
    weak var outlineView: NSOutlineView?
    weak var scrollView: NSScrollView?
    /// Peer of NDTreeDataSource.suppressExpandEvents: blocks nodeExpanded/
    /// nodeCollapsed while programmatic expansion replays the `expanded` flags.
    var suppressExpandEvents = false

    func outlineView(_ outlineView: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int {
        guard let item = item as? NDSourceTreeItem else { return roots.count }
        return item.children.count
    }

    func outlineView(_ outlineView: NSOutlineView, child index: Int, ofItem item: Any?) -> Any {
        guard let item = item as? NDSourceTreeItem else { return roots[index] }
        return item.children[index]
    }

    func outlineView(_ outlineView: NSOutlineView, isItemExpandable item: Any) -> Bool {
        guard let item = item as? NDSourceTreeItem else { return false }
        return item.hasChildren || !item.children.isEmpty
    }

    func outlineView(_ outlineView: NSOutlineView, isGroupItem item: Any) -> Bool {
        (item as? NDSourceTreeItem)?.section ?? false
    }

    func outlineView(_ outlineView: NSOutlineView, shouldShowOutlineCellForItem item: Any) -> Bool {
        // Sections expand/collapse from app state only (no disclosure
        // triangle) — the native macOS source-list group look.
        !((item as? NDSourceTreeItem)?.section ?? false)
    }

    func outlineView(_ outlineView: NSOutlineView, shouldSelectItem item: Any) -> Bool {
        guard let item = item as? NDSourceTreeItem else { return false }
        return !item.section && item.selectable
    }

    func outlineView(_ outlineView: NSOutlineView, heightOfRowByItem item: Any) -> CGFloat {
        // Two-line rows (title + caption) need the taller fixed height; the
        // outline view's own rowHeight covers every single-line row. Fixed
        // heights, not usesAutomaticRowHeights: the cell centers its title
        // with a toggled constraint, which automatic sizing cannot resolve.
        guard let item = item as? NDSourceTreeItem, item.caption != nil else { return outlineView.rowHeight }
        return 36
    }

    func outlineView(_ outlineView: NSOutlineView, rowViewForItem item: Any) -> NSTableRowView? {
        let rowView = NDSourceTreeRowView()
        rowView.hoverActions = hoverActions
        return rowView
    }

    func outlineView(_ outlineView: NSOutlineView, viewFor tableColumn: NSTableColumn?, item: Any) -> NSView? {
        guard let item = item as? NDSourceTreeItem else { return nil }
        let cellID = item.section ? ndSourceTreeSectionCellID : ndSourceTreeCellID
        let cell = outlineView.makeView(withIdentifier: cellID, owner: self) as? NDSourceTreeCell ?? NDSourceTreeCell()
        cell.identifier = cellID
        let rowActions = item.actionIds.compactMap { actions[$0] }
        cell.configure(with: item, actions: rowActions, target: self, hidden: hoverActions)
        return cell
    }

    func outlineViewSelectionDidChange(_ notification: Notification) {
        guard let outlineView = notification.object as? NSOutlineView,
              let scrollView = outlineView.enclosingScrollView,
              !ndIsEchoSuppressed(scrollView) else { return }
        // A source list only draws its emphasized selection (accent capsule,
        // white title) while it is first responder. A click already makes it
        // one; a selection driven through the automation seam does not, and
        // left the row in the greyed inactive state. Echo-suppressed
        // selections (the `selectedId` prop, the post-reload restore) never
        // reach here, so this can't steal focus from a search field the user
        // is typing in.
        if let window = outlineView.window, window.firstResponder !== outlineView {
            window.makeFirstResponder(outlineView)
        }
        let selected = outlineView.item(atRow: outlineView.selectedRow) as? NDSourceTreeItem
        emitNode("selectionChanged", selected?.nodeId)
    }

    @objc func rowDoubleClicked(_ sender: NSOutlineView) {
        guard sender.clickedRow >= 0 else { return }
        let nodeId: String? = MainActor.assumeIsolated {
            (sender.item(atRow: sender.clickedRow) as? NDSourceTreeItem)?.nodeId
        }
        guard let nodeId else { return }
        emitNode("rowActivated", nodeId)
    }

    @objc func actionClicked(_ sender: NDSourceTreeActionButton) {
        emitAction(sender.nodeId, sender.actionId)
    }

    func emitAction(_ nodeId: String, _ actionId: String) {
        ndEmitEvent(nodeID, "actionClicked",
                    "{\"data\":{\"nodeId\":\(ndJsonString(nodeId)),\"actionId\":\(ndJsonString(actionId))}}")
    }

    func outlineViewItemDidExpand(_ notification: Notification) {
        guard let item = notification.userInfo?["NSObject"] as? NDSourceTreeItem else { return }
        item.expandedFlag = true
        guard !suppressExpandEvents else { return }
        emitNode("nodeExpanded", item.nodeId)
    }

    func outlineViewItemDidCollapse(_ notification: Notification) {
        guard let item = notification.userInfo?["NSObject"] as? NDSourceTreeItem else { return }
        item.expandedFlag = false
        guard !suppressExpandEvents else { return }
        emitNode("nodeCollapsed", item.nodeId)
    }

    /// `{ data: { nodeId } }`; nodeId is JSON null on deselection (GTK parity).
    func emitNode(_ name: String, _ nodeId: String?) {
        let value = nodeId.map(ndJsonString) ?? "null"
        ndEmitEvent(nodeID, name, "{\"data\":{\"nodeId\":\(value)}}")
    }
}

/// Sidebar row cell: leading SF Symbol, a title with an optional caption
/// line under it (caption may carry its own small leading symbol), a
/// trailing numeric badge, and trailing action buttons. Modeled on
/// NDSourceCell (NDGen/SourceList.swift); view-based recycling reuses
/// instances via `makeView(withIdentifier:)`, so `configure` re-runs per row.
final class NDSourceTreeCell: NSTableCellView {
    private let iconView = NSImageView()
    private let titleField = NSTextField(labelWithString: "")
    private let captionIconView = NSImageView()
    private let captionField = NSTextField(labelWithString: "")
    private let badgeField = NSTextField(labelWithString: "")
    private let actionsStack = NSStackView()
    private var iconWidthConstraint: NSLayoutConstraint!
    private var captionHeightConstraint: NSLayoutConstraint!
    private var titleTopConstraint: NSLayoutConstraint!
    private var titleCenterConstraint: NSLayoutConstraint!
    private var isSection = false
    private var tintedIcon = false

    // HIG *Focus and selection* documents exactly two source-list states:
    // focused rows get white text on the accent fill, unfocused rows "the
    // standard text color and a gray background highlight". Bind the title to
    // `NSTableCellView.textField` and macOS 26 draws a THIRD state nothing
    // documents — the accent colour on the neutral fill (measured rgb(51,111,
    // 223) on 26.5.1, where Finder's own sidebar draws that row in plain
    // white). AppKit only recolours the bound field, and knows nothing about
    // the caption, badge or symbol either, so leave `textField` unset and
    // drive every element from `backgroundStyle` here. `.textColor` (not
    // `.labelColor`) is the unfocused title: it is the one AppKit itself
    // used, and it renders opaque in the sidebar's vibrant appearance where
    // labelColor's 85% alpha washes out to ~70% grey.
    override var backgroundStyle: NSView.BackgroundStyle {
        didSet { applySelectionColors() }
    }

    private func applySelectionColors() {
        let onFill = backgroundStyle == .emphasized
        if isSection {
            titleField.textColor = .secondaryLabelColor
        } else {
            titleField.textColor = onFill ? .alternateSelectedControlTextColor : .textColor
        }
        let secondary: NSColor = onFill
            ? NSColor.alternateSelectedControlTextColor.withAlphaComponent(0.8)
            : .secondaryLabelColor
        captionField.textColor = secondary
        badgeField.textColor = secondary
        captionIconView.contentTintColor = secondary
        iconView.contentTintColor = (onFill && !tintedIcon) ? .alternateSelectedControlTextColor : nil
    }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        commonInit()
    }
    required init?(coder: NSCoder) {
        super.init(coder: coder)
        commonInit()
    }

    private func commonInit() {
        iconView.translatesAutoresizingMaskIntoConstraints = false
        iconView.imageScaling = .scaleProportionallyDown
        titleField.translatesAutoresizingMaskIntoConstraints = false
        titleField.lineBreakMode = .byTruncatingTail
        captionIconView.translatesAutoresizingMaskIntoConstraints = false
        captionIconView.imageScaling = .scaleProportionallyDown
        captionField.translatesAutoresizingMaskIntoConstraints = false
        captionField.lineBreakMode = .byTruncatingTail
        captionField.textColor = .secondaryLabelColor
        captionField.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        badgeField.translatesAutoresizingMaskIntoConstraints = false
        badgeField.textColor = .secondaryLabelColor
        badgeField.font = .monospacedDigitSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular)
        badgeField.alignment = .right
        actionsStack.translatesAutoresizingMaskIntoConstraints = false
        actionsStack.orientation = .horizontal
        actionsStack.spacing = 2

        addSubview(iconView)
        addSubview(titleField)
        addSubview(captionIconView)
        addSubview(captionField)
        addSubview(badgeField)
        addSubview(actionsStack)

        iconWidthConstraint = iconView.widthAnchor.constraint(equalToConstant: 16)
        captionHeightConstraint = captionField.heightAnchor.constraint(equalToConstant: 0)
        captionHeightConstraint.isActive = false
        // configure() activates exactly one of the pair: captionless rows
        // center the title, caption rows pin it to the top of the taller row
        // (heightOfRowByItem) so the caption line fits underneath.
        titleTopConstraint = titleField.topAnchor.constraint(equalTo: topAnchor, constant: 2)
        titleCenterConstraint = titleField.centerYAnchor.constraint(equalTo: centerYAnchor)
        titleCenterConstraint.isActive = true
        NSLayoutConstraint.activate([
            iconView.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 4),
            iconView.centerYAnchor.constraint(equalTo: centerYAnchor),
            iconWidthConstraint,
            iconView.heightAnchor.constraint(equalToConstant: 16),

            titleField.leadingAnchor.constraint(equalTo: iconView.trailingAnchor, constant: 6),

            captionIconView.leadingAnchor.constraint(equalTo: iconView.trailingAnchor, constant: 6),
            captionIconView.centerYAnchor.constraint(equalTo: captionField.centerYAnchor),
            captionIconView.widthAnchor.constraint(equalToConstant: 11),
            captionIconView.heightAnchor.constraint(equalToConstant: 11),

            captionField.leadingAnchor.constraint(equalTo: captionIconView.trailingAnchor, constant: 3),
            captionField.topAnchor.constraint(equalTo: titleField.bottomAnchor),
            captionField.bottomAnchor.constraint(lessThanOrEqualTo: bottomAnchor, constant: -2),

            badgeField.leadingAnchor.constraint(greaterThanOrEqualTo: titleField.trailingAnchor, constant: 6),
            badgeField.trailingAnchor.constraint(equalTo: actionsStack.leadingAnchor, constant: -4),
            badgeField.centerYAnchor.constraint(equalTo: centerYAnchor),

            actionsStack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -4),
            actionsStack.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    func configure(with item: NDSourceTreeItem, actions: [NDSourceTreeAction], target: NDSourceTreeDataSource, hidden: Bool) {
        titleField.stringValue = item.title
        isSection = item.section
        titleField.font = item.section
            ? .systemFont(ofSize: NSFont.smallSystemFontSize, weight: .semibold)
            : .systemFont(ofSize: NSFont.systemFontSize)
        // Image bytes beat a theme name: a favicon has no freedesktop name (or
        // SF Symbol), and this is the only way a browser sidebar can show one.
        // Raw bytes carry their own colour, so the selection tint must not
        // touch them (`tintedIcon`).
        tintedIcon = false
        if let data = item.iconData, let image = ndImageFromDataURL(data) {
            image.size = NSSize(width: 16, height: 16)
            iconView.image = image
            tintedIcon = true
        } else if let iconName = item.iconName {
            let symbol = ndSFSymbol(forFreedesktop: iconName) ?? iconName  // NDShell/Icons.swift
            iconView.image = NSImage(systemSymbolName: symbol, accessibilityDescription: item.title)
        } else {
            iconView.image = nil
        }
        iconView.isHidden = iconView.image == nil
        iconWidthConstraint.constant = iconView.isHidden ? 0 : 16

        captionField.stringValue = item.caption ?? ""
        captionField.isHidden = item.caption == nil
        // Hidden views still constrain: collapse the caption line and switch
        // the title back to centered, or a captionless row over-constrains
        // (title.bottom + intrinsic caption height overflows the row).
        captionHeightConstraint.isActive = item.caption == nil
        titleTopConstraint.isActive = item.caption != nil
        titleCenterConstraint.isActive = item.caption == nil
        if let capIcon = item.captionIconName, item.caption != nil {
            let symbol = ndSFSymbol(forFreedesktop: capIcon) ?? capIcon
            captionIconView.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
        } else {
            captionIconView.image = nil
        }
        captionIconView.isHidden = captionIconView.image == nil

        badgeField.stringValue = item.badge ?? ""
        badgeField.isHidden = item.badge == nil
        // Recycled cells keep whatever colours their previous row had, and
        // `backgroundStyle` does not re-fire on reuse.
        applySelectionColors()

        for sub in actionsStack.arrangedSubviews {
            actionsStack.removeArrangedSubview(sub)
            sub.removeFromSuperview()
        }
        for action in actions {
            let btn = NDSourceTreeActionButton()
            btn.nodeId = item.nodeId
            btn.actionId = action.id
            if let label = action.label {
                btn.title = label
                btn.bezelStyle = .inline
                btn.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
            } else {
                let symbol = ndSFSymbol(forFreedesktop: action.iconName) ?? action.iconName
                btn.image = NSImage(systemSymbolName: symbol, accessibilityDescription: action.tooltip ?? action.id)
                btn.title = ""
                btn.isBordered = false
            }
            if action.destructive { btn.contentTintColor = .systemRed }
            if let tooltip = action.tooltip { btn.toolTip = tooltip }
            btn.target = target
            btn.action = #selector(NDSourceTreeDataSource.actionClicked(_:))
            actionsStack.addArrangedSubview(btn)
        }
        setActionsVisible(!hidden)
    }

    /// Alpha, never `isHidden`: a hidden arranged subview leaves the stack, so
    /// the badge and the title take its width the moment the pointer leaves —
    /// the row's content shifts under the cursor. `isEnabled` is what keeps a
    /// transparent button out of the click path; alpha alone does not make an
    /// NSView non-interactive.
    func setActionsVisible(_ visible: Bool) {
        for sub in actionsStack.arrangedSubviews {
            sub.alphaValue = visible ? 1 : 0
            (sub as? NSControl)?.isEnabled = visible
        }
    }
}

nonisolated(unsafe) private var sourceTreeDataSources: [ObjectIdentifier: NDSourceTreeDataSource] = [:]

private func sourceTreeDataSource(for view: NSView) -> NDSourceTreeDataSource? {
    sourceTreeDataSources[ObjectIdentifier(view)]
}

/// release_node purge seam (Backend.swift's `ndPurgeNodeRegistries`).
func ndSourceTreePurge(_ view: NSView) {
    sourceTreeDataSources[ObjectIdentifier(view)] = nil
}

private func sourceTreeAction(from obj: [String: Any]) -> NDSourceTreeAction? {
    guard let id = obj["id"] as? String else { return nil }
    return NDSourceTreeAction(
        id: id,
        iconName: obj["iconName"] as? String ?? "",
        label: obj["label"] as? String,
        tooltip: obj["tooltip"] as? String,
        destructive: (obj["destructive"] as? NSNumber)?.boolValue ?? false
    )
}

/// `ndCreate`'s SourceTree arm (generated) calls this.
func makeSourceTree(_ props: [String: Any]) -> NSView {
    let outlineView = NSOutlineView()
    let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("nd-sourcetree-column"))
    column.resizingMask = .autoresizingMask
    outlineView.addTableColumn(column)
    outlineView.outlineTableColumn = column
    outlineView.headerView = nil
    outlineView.autoresizesOutlineColumn = false
    // `style = .sourceList` alone: `selectionHighlightStyle = .sourceList` is
    // deprecated in favour of exactly this property.
    outlineView.style = .sourceList
    outlineView.floatsGroupRows = false
    outlineView.indentationPerLevel = CGFloat(propInt(props, "indentationPerLevel") ?? 14)
    outlineView.autoresizingMask = [.width]

    let source = NDSourceTreeDataSource()
    source.outlineView = outlineView
    if let vis = propStr(props, "actionVisibility") { source.hoverActions = vis != "always" }
    if let raw = propObjArray(props, "actions") {
        for obj in raw {
            if let action = sourceTreeAction(from: obj) { source.actions[action.id] = action }
        }
    }
    outlineView.dataSource = source
    outlineView.delegate = source
    outlineView.target = source
    outlineView.doubleAction = #selector(NDSourceTreeDataSource.rowDoubleClicked(_:))

    let scrollView = NSScrollView()
    scrollView.hasVerticalScroller = true
    scrollView.documentView = outlineView
    scrollView.drawsBackground = false // glass sidebar shows through
    outlineView.sizeLastColumnToFit()
    source.scrollView = scrollView
    sourceTreeDataSources[ObjectIdentifier(scrollView)] = source

    ndEmptyStateApply(scrollView, props)
    ndSourceTreeSetNodes(scrollView, propObjArray(props, "nodes") ?? [])
    if let sel = propStr(props, "selectedId"), !sel.isEmpty { ndSourceTreeSetSelectedId(scrollView, sel) }
    return scrollView
}

/// Generated ndApplyProps SourceTree.nodes arm: rebuild the grouped index
/// from the flat id/parentId list, reusing items by id (expansion identity),
/// then replay each node's `expanded` flag with events suppressed. Selection
/// is preserved by node id across the rebuild.
func ndSourceTreeSetNodes(_ view: NSView, _ raw: [[String: Any]]) {
    guard let source = sourceTreeDataSource(for: view), let outlineView = source.outlineView else { return }
    ndEmptyStateUpdate(view, isEmpty: raw.isEmpty)
    let prevSelectedID: String? = MainActor.assumeIsolated {
        (outlineView.item(atRow: outlineView.selectedRow) as? NDSourceTreeItem)?.nodeId
    }

    var newByID: [String: NDSourceTreeItem] = [:]
    var ordered: [(item: NDSourceTreeItem, parentId: String?)] = []
    for obj in raw {
        guard let id = obj["id"] as? String else { continue }
        let item = source.itemsByID[id] ?? NDSourceTreeItem()
        item.nodeId = id
        item.title = obj["title"] as? String ?? ""
        item.caption = obj["caption"] as? String
        item.iconName = obj["iconName"] as? String
        item.iconData = obj["iconData"] as? String
        item.captionIconName = obj["captionIconName"] as? String
        item.badge = obj["badge"] as? String
        item.section = (obj["section"] as? NSNumber)?.boolValue ?? false
        item.hasChildren = (obj["hasChildren"] as? NSNumber)?.boolValue ?? false
        item.expandedFlag = (obj["expanded"] as? NSNumber)?.boolValue ?? false
        item.selectable = (obj["selectable"] as? NSNumber)?.boolValue ?? !item.section
        item.actionIds = (obj["actionIds"] as? [Any])?.compactMap { $0 as? String } ?? []
        item.testID = obj["testID"] as? String
        item.children = []
        newByID[id] = item
        ordered.append((item, obj["parentId"] as? String))
    }
    var roots: [NDSourceTreeItem] = []
    for (item, parentId) in ordered {
        if let parentId, let parent = newByID[parentId] {
            parent.children.append(item)
        } else {
            roots.append(item)
        }
    }
    source.itemsByID = newByID
    source.roots = roots

    withEchoSuppressed(view) {
        source.suppressExpandEvents = true
        outlineView.reloadData()
        // Parent-first (schema declaration order): a child can only expand
        // once its ancestors are visible rows.
        for (item, _) in ordered {
            if item.expandedFlag {
                outlineView.expandItem(item)
            } else {
                outlineView.collapseItem(item)
            }
        }
        source.suppressExpandEvents = false
        if let prevSelectedID, let item = newByID[prevSelectedID] {
            let row = outlineView.row(forItem: item)
            if row >= 0 { outlineView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false) }
        }
    }
}

/// Generated ndApplyProps SourceTree.actions arm — the action catalog rows
/// reference by id. Reload rebuilds the visible cells' trailing buttons.
func ndSourceTreeSetActions(_ view: NSView, _ raw: [[String: Any]]) {
    guard let source = sourceTreeDataSource(for: view), let outlineView = source.outlineView else { return }
    source.actions = [:]
    for obj in raw {
        if let action = sourceTreeAction(from: obj) { source.actions[action.id] = action }
    }
    withEchoSuppressed(view) {
        source.suppressExpandEvents = true
        outlineView.reloadData()
        // reloadData collapses everything — replay the standing flags.
        for (_, item) in source.itemsByID where item.expandedFlag {
            outlineView.expandItem(item)
        }
        source.suppressExpandEvents = false
    }
}

/// Generated ndApplyProps SourceTree.selectedId arm — "" clears selection.
func ndSourceTreeSetSelectedId(_ view: NSView, _ id: String) {
    guard let source = sourceTreeDataSource(for: view), let outlineView = source.outlineView else { return }
    let current: String? = MainActor.assumeIsolated {
        (outlineView.item(atRow: outlineView.selectedRow) as? NDSourceTreeItem)?.nodeId
    }
    if id.isEmpty {
        guard current != nil else { return }
        withEchoSuppressed(view) { outlineView.deselectAll(nil) }
        return
    }
    guard current != id, let item = source.itemsByID[id] else { return }
    withEchoSuppressed(view) {
        let row = outlineView.row(forItem: item)
        if row >= 0 { outlineView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false) }
    }
}

/// Generated ndConnectEvents SourceTree arm (one call wires all five events).
func ndSourceTreeConnect(_ view: NSView, nodeID: UInt32) {
    sourceTreeDataSource(for: view)?.nodeID = nodeID
}

// ---- automation (Automation.swift semantic arms) ---------------------------

/// a11y value probe: the selected node's id, or nil.
func ndSourceTreeSelectedId(_ view: NSView) -> String? {
    guard let source = sourceTreeDataSource(for: view), let outlineView = source.outlineView else { return nil }
    return MainActor.assumeIsolated {
        (outlineView.item(atRow: outlineView.selectedRow) as? NDSourceTreeItem)?.nodeId
    }
}

/// setValue {value: "<nodeId>"|""}: selects by node id, emitting exactly one
/// selectionChanged {nodeId} for the landed selection.
///
/// A selection that MOVES stays unsuppressed: `selectRowIndexes` posts
/// `outlineViewSelectionDidChange` itself, and that path also makes the
/// outline view first responder so the row draws its emphasized state. A
/// selection that does NOT move posts nothing, so the event is emitted here
/// instead. That state is reachable whenever a controlled `selectedId` frame
/// computed before the last selection lands after it: the prop arm re-applies
/// it echo-suppressed, leaving the view holding a row the app does not, and a
/// setValue for that same row used to emit zero events (GTK peer:
/// src/gtk/sourcetree.zig's semanticSelect).
@MainActor func ndSourceTreeSemanticSelect(_ view: NSView, _ id: String) -> Bool {
    guard let source = sourceTreeDataSource(for: view), let outlineView = source.outlineView else { return false }
    let current = (outlineView.item(atRow: outlineView.selectedRow) as? NDSourceTreeItem)?.nodeId
    if id.isEmpty {
        if current == nil {
            source.emitNode("selectionChanged", nil)
        } else {
            outlineView.deselectAll(nil)
        }
        return true
    }
    guard let item = source.itemsByID[id], !item.section, item.selectable else { return false }
    let row = outlineView.row(forItem: item)
    guard row >= 0 else { return false }
    if current == id {
        // The notification path's first-responder step has to happen here too,
        // or a row selected through the prop arm keeps drawing unemphasized
        // after an automation select of that same row.
        if let window = outlineView.window, window.firstResponder !== outlineView {
            window.makeFirstResponder(outlineView)
        }
        source.emitNode("selectionChanged", id)
    } else {
        outlineView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
    }
    return true
}

/// rowAction {actionId, testId?}: dispatches a row's trailing action as if
/// its button were clicked (actionClicked {nodeId, actionId}). testId picks
/// the node by its per-node testID; absent, the selected row is the target.
/// The row must be realized (visible under the current expansion, like a
/// user-reachable button) and the action declared on that node.
@MainActor func ndSourceTreeSemanticRowAction(_ view: NSView, actionId: String, testId: String?) -> Bool {
    guard let source = sourceTreeDataSource(for: view), let outlineView = source.outlineView else { return false }
    let item: NDSourceTreeItem?
    if let testId {
        item = source.itemsByID.values.first { $0.testID == testId }
    } else {
        item = outlineView.item(atRow: outlineView.selectedRow) as? NDSourceTreeItem
    }
    guard let item, outlineView.row(forItem: item) >= 0,
          item.actionIds.contains(actionId), source.actions[actionId] != nil else { return false }
    source.emitAction(item.nodeId, actionId)
    return true
}

/// click — activates the currently-selected row, emitting rowActivated
/// {nodeId}. False when nothing is selected.
@MainActor func ndSourceTreeSemanticActivate(_ view: NSView) -> Bool {
    guard let source = sourceTreeDataSource(for: view), let outlineView = source.outlineView,
          let item = outlineView.item(atRow: outlineView.selectedRow) as? NDSourceTreeItem else { return false }
    source.emitNode("rowActivated", item.nodeId)
    return true
}

/// A row icon from raw image bytes — a `data:<mime>;base64,<payload>` URL or a
/// bare base64 payload. Peer of src/gtk/sourcetree.zig's imageFromData; a
/// payload AppKit cannot decode answers nil rather than failing the row.
func ndImageFromDataURL(_ value: String) -> NSImage? {
    let payload: Substring
    if value.hasPrefix("data:"), let comma = value.firstIndex(of: ",") {
        payload = value[value.index(after: comma)...]
    } else {
        payload = value[...]
    }
    guard let data = Data(base64Encoded: String(payload), options: .ignoreUnknownCharacters) else { return nil }
    return NSImage(data: data)
}
