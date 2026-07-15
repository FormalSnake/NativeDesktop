import AppKit

/// TreeView: a single-column NSOutlineView in an NSScrollView (tracked
/// handle = scroll view, SourceList convention). The schema's flat TreeNode
/// list (id/parentId) is grouped into a parentId index and rebuilt on every
/// `nodes` update, REUSING item instances by id — NSOutlineView tracks
/// expansion by item identity, so stable instances keep open branches open
/// across React updates. Events carry `{ nodeId }` because flattened visible
/// indexes are unstable across expand/collapse; `selectedIndex`
/// still addresses the flattened visible row list, matching the schema.
/// Expansion state is driven from each node's `expanded` flag,
/// echo-suppressed; user expands/collapses emit nodeExpanded/nodeCollapsed.
/// Row content reuses NDSourceCell (title/badge/iconName, the SourceList
/// row shape).
private let ndTreeCellID = NSUserInterfaceItemIdentifier("nd-tree-cell")

/// `@unchecked Sendable`: items cross into MainActor AppKit calls
/// (expandItem/row(forItem:), both taking Any?) from the nominally
/// nonisolated apply funcs — the ABI contract guarantees every touch happens
/// on the UI thread, which is what the region checker can't see (the same
/// reasoning as the module's nonisolated(unsafe) globals).
final class NDTreeItem: NSObject, @unchecked Sendable {
    var nodeId: String = ""
    var title: String = ""
    var badge: String?
    var iconName: String?
    var hasChildren = false
    var expandedFlag = false
    var children: [NDTreeItem] = []
}

final class NDTreeDataSource: NSObject, NSOutlineViewDataSource, NSOutlineViewDelegate {
    var nodeID: UInt32 = 0
    var roots: [NDTreeItem] = []
    var itemsByID: [String: NDTreeItem] = [:]
    weak var outlineView: NSOutlineView?
    weak var scrollView: NSScrollView?
    /// Blocks nodeExpanded/nodeCollapsed while programmatic expansion replays
    /// the `expanded` flags (peer of withEchoSuppressed, but scoped to the
    /// outline's own notifications).
    var suppressExpandEvents = false

    func outlineView(_ outlineView: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int {
        guard let item = item as? NDTreeItem else { return roots.count }
        return item.children.count
    }

    func outlineView(_ outlineView: NSOutlineView, child index: Int, ofItem item: Any?) -> Any {
        guard let item = item as? NDTreeItem else { return roots[index] }
        return item.children[index]
    }

    func outlineView(_ outlineView: NSOutlineView, isItemExpandable item: Any) -> Bool {
        guard let item = item as? NDTreeItem else { return false }
        // hasChildren with no loaded children still shows an expander — the
        // app appends children on nodeExpanded (lazy-load contract).
        return item.hasChildren || !item.children.isEmpty
    }

    func outlineView(_ outlineView: NSOutlineView, viewFor tableColumn: NSTableColumn?, item: Any) -> NSView? {
        guard let item = item as? NDTreeItem else { return nil }
        let cell = outlineView.makeView(withIdentifier: ndTreeCellID, owner: self) as? NDSourceCell ?? NDSourceCell()
        cell.identifier = ndTreeCellID
        cell.configure(with: SourceRow(title: item.title, badge: item.badge, iconName: item.iconName))
        return cell
    }

    func outlineViewSelectionDidChange(_ notification: Notification) {
        guard let outlineView = notification.object as? NSOutlineView,
              let scrollView = outlineView.enclosingScrollView,
              !ndIsEchoSuppressed(scrollView) else { return }
        let selected = outlineView.item(atRow: outlineView.selectedRow) as? NDTreeItem
        emitNode("selectionChanged", selected?.nodeId)
    }

    @objc func rowDoubleClicked(_ sender: NSOutlineView) {
        guard sender.clickedRow >= 0 else { return }
        // item(atRow:) returns a non-Sendable Any? — resolve to the Sendable
        // node id inside the isolation the ABI already guarantees.
        let nodeId: String? = MainActor.assumeIsolated {
            (sender.item(atRow: sender.clickedRow) as? NDTreeItem)?.nodeId
        }
        guard let nodeId else { return }
        emitNode("rowActivated", nodeId)
    }

    func outlineViewItemDidExpand(_ notification: Notification) {
        guard let item = notification.userInfo?["NSObject"] as? NDTreeItem else { return }
        item.expandedFlag = true
        guard !suppressExpandEvents else { return }
        emitNode("nodeExpanded", item.nodeId)
    }

    func outlineViewItemDidCollapse(_ notification: Notification) {
        guard let item = notification.userInfo?["NSObject"] as? NDTreeItem else { return }
        item.expandedFlag = false
        guard !suppressExpandEvents else { return }
        emitNode("nodeCollapsed", item.nodeId)
    }

    /// `{ data: { nodeId } }`; nodeId is JSON null on deselection (GTK parity).
    private func emitNode(_ name: String, _ nodeId: String?) {
        let value = nodeId.map(ndJsonString) ?? "null"
        ndEmitEvent(nodeID, name, "{\"data\":{\"nodeId\":\(value)}}")
    }
}

nonisolated(unsafe) private var treeDataSources: [ObjectIdentifier: NDTreeDataSource] = [:]

private func treeDataSource(for view: NSView) -> NDTreeDataSource? {
    treeDataSources[ObjectIdentifier(view)]
}

/// `ndCreate`'s TreeView arm (generated) calls this.
func makeTreeView(_ props: [String: Any]) -> NSView {
    let outlineView = NSOutlineView()
    let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("nd-tree-column"))
    column.resizingMask = .autoresizingMask
    outlineView.addTableColumn(column)
    outlineView.outlineTableColumn = column
    outlineView.headerView = nil
    outlineView.autoresizesOutlineColumn = false
    outlineView.indentationPerLevel = CGFloat(propInt(props, "indentationPerLevel") ?? 16)
    outlineView.autoresizingMask = [.width]

    let source = NDTreeDataSource()
    source.outlineView = outlineView
    outlineView.dataSource = source
    outlineView.delegate = source
    outlineView.target = source
    outlineView.doubleAction = #selector(NDTreeDataSource.rowDoubleClicked(_:))

    let scrollView = NSScrollView()
    scrollView.hasVerticalScroller = true
    scrollView.documentView = outlineView
    scrollView.drawsBackground = false
    outlineView.sizeLastColumnToFit()
    source.scrollView = scrollView
    treeDataSources[ObjectIdentifier(scrollView)] = source

    ndTreeViewSetNodes(scrollView, propObjArray(props, "nodes") ?? [])
    let selIdx = propInt(props, "selectedIndex") ?? -1
    if selIdx >= 0 { ndTreeViewSetSelectedIndex(scrollView, selIdx) }
    return scrollView
}

/// Generated ndApplyProps TreeView.nodes arm: rebuild the grouped index from
/// the flat id/parentId list, reusing items by id (expansion identity), then
/// replay each node's `expanded` flag with events suppressed. Selection is
/// preserved by node id across the rebuild.
func ndTreeViewSetNodes(_ view: NSView, _ raw: [[String: Any]]) {
    guard let source = treeDataSource(for: view), let outlineView = source.outlineView else { return }
    let prevSelectedID: String? = MainActor.assumeIsolated {
        (outlineView.item(atRow: outlineView.selectedRow) as? NDTreeItem)?.nodeId
    }

    var newByID: [String: NDTreeItem] = [:]
    var ordered: [(item: NDTreeItem, parentId: String?)] = []
    for obj in raw {
        guard let id = obj["id"] as? String else { continue }
        let item = source.itemsByID[id] ?? NDTreeItem()
        item.nodeId = id
        item.title = obj["title"] as? String ?? ""
        item.badge = obj["badge"] as? String
        item.iconName = obj["iconName"] as? String
        item.hasChildren = (obj["hasChildren"] as? NSNumber)?.boolValue ?? false
        item.expandedFlag = (obj["expanded"] as? NSNumber)?.boolValue ?? false
        item.children = []
        newByID[id] = item
        ordered.append((item, obj["parentId"] as? String))
    }
    var roots: [NDTreeItem] = []
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

/// Generated ndApplyProps TreeView.selectedIndex arm — the index addresses
/// the flattened VISIBLE row list (schema contract).
func ndTreeViewSetSelectedIndex(_ view: NSView, _ index: Int) {
    guard let source = treeDataSource(for: view), let outlineView = source.outlineView else { return }
    guard outlineView.selectedRow != index else { return }
    withEchoSuppressed(view) {
        if index >= 0 && index < outlineView.numberOfRows {
            outlineView.selectRowIndexes(IndexSet(integer: index), byExtendingSelection: false)
        } else {
            outlineView.deselectAll(nil)
        }
    }
}

/// Generated ndConnectEvents TreeView arm (one call wires all four events).
func ndTreeViewConnect(_ view: NSView, nodeID: UInt32) {
    treeDataSource(for: view)?.nodeID = nodeID
}
