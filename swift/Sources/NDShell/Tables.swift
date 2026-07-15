import AppKit

/// Table: a view-based NSTableView in an NSScrollView (the tracked
/// handle is the scroll view, the SourceList convention). Columns are
/// rebuilt whenever the `columns` prop lands; each carries a
/// `sortDescriptorPrototype` so header clicks produce a native sort
/// indicator, BUT the data source NEVER reorders itself: header clicks emit
/// `sortChanged { columnId, direction }` and JS owns row order (only the app
/// knows if a column is numeric/date/lexical; cross-platform contract set by
/// the GTK backend's no-op-sorter mechanism). Rows arrive as the schema's
/// TableRow objectList (`{ id?, cells: [...] }`, positional cells indexed
/// by the columns array order); GTK's 0x1F cell join is its internal storage
/// trick, not the wire format.
private let ndTableCellID = NSUserInterfaceItemIdentifier("nd-table-cell")

struct NDTableColumnSpec {
    let id: String
    let title: String
    let width: Int?
}

final class NDTableDataSource: NSObject, NSTableViewDataSource, NSTableViewDelegate {
    var nodeID: UInt32 = 0
    var columns: [NDTableColumnSpec] = []
    var rows: [[String]] = []
    weak var tableView: NSTableView?
    weak var scrollView: NSScrollView?

    func numberOfRows(in tableView: NSTableView) -> Int { rows.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard let column = tableColumn,
              let colIdx = columns.firstIndex(where: { $0.id == column.identifier.rawValue }) else { return nil }
        let cell: NSTableCellView
        if let recycled = tableView.makeView(withIdentifier: ndTableCellID, owner: self) as? NSTableCellView {
            cell = recycled
        } else {
            cell = NSTableCellView()
            cell.identifier = ndTableCellID
            let field = NSTextField(labelWithString: "")
            field.lineBreakMode = .byTruncatingTail
            field.translatesAutoresizingMaskIntoConstraints = false
            cell.addSubview(field)
            cell.textField = field
            NSLayoutConstraint.activate([
                field.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 2),
                field.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -2),
                field.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
            ])
        }
        let cells = row < rows.count ? rows[row] : []
        cell.textField?.stringValue = colIdx < cells.count ? cells[colIdx] : ""
        return cell
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        guard let tableView = notification.object as? NSTableView,
              let scrollView = tableView.enclosingScrollView,
              !ndIsEchoSuppressed(scrollView) else { return }
        ndEmitEvent(nodeID, "selectionChanged", "{\"index\":\(tableView.selectedRow)}")
    }

    @objc func rowDoubleClicked(_ sender: NSTableView) {
        guard sender.clickedRow >= 0 else { return }
        ndEmitEvent(nodeID, "rowActivated", "{\"index\":\(sender.clickedRow)}")
    }

    /// Header-click sorting: notify JS, never reorder natively.
    func tableView(_ tableView: NSTableView, sortDescriptorsDidChange oldDescriptors: [NSSortDescriptor]) {
        guard let descriptor = tableView.sortDescriptors.first, let key = descriptor.key else { return }
        let direction = descriptor.ascending ? "ascending" : "descending"
        ndEmitEvent(nodeID, "sortChanged", "{\"data\":{\"columnId\":\(ndJsonString(key)),\"direction\":\"\(direction)\"}}")
    }
}

nonisolated(unsafe) private var tableDataSources: [ObjectIdentifier: NDTableDataSource] = [:]

private func tableDataSource(for view: NSView) -> NDTableDataSource? {
    tableDataSources[ObjectIdentifier(view)]
}

/// `ndCreate`'s Table arm (generated) calls this.
func makeTable(_ props: [String: Any]) -> NSView {
    let tableView = NSTableView()
    tableView.usesAlternatingRowBackgroundColors = false
    tableView.allowsMultipleSelection = false
    tableView.allowsColumnReordering = false

    let source = NDTableDataSource()
    source.tableView = tableView
    tableView.dataSource = source
    tableView.delegate = source
    tableView.target = source
    tableView.doubleAction = #selector(NDTableDataSource.rowDoubleClicked(_:))

    let scrollView = NSScrollView()
    scrollView.hasVerticalScroller = true
    scrollView.documentView = tableView
    scrollView.drawsBackground = false
    source.scrollView = scrollView
    tableDataSources[ObjectIdentifier(scrollView)] = source

    ndTableSetColumns(scrollView, propObjArray(props, "columns") ?? [])
    ndTableSetRows(scrollView, propObjArray(props, "rows") ?? [])
    ndTableSetShowRowSeparators(scrollView, propBool(props, "showRowSeparators") ?? true)
    let selIdx = propInt(props, "selectedIndex") ?? -1
    if selIdx >= 0 { ndTableSetSelectedIndex(scrollView, selIdx) }
    return scrollView
}

/// Generated ndApplyProps Table.columns arm: full rebuild — remove every
/// NSTableColumn, re-add from the spec. Runs only when the columns prop is
/// in the diff, so user-resized widths survive unrelated updates.
func ndTableSetColumns(_ view: NSView, _ raw: [[String: Any]]) {
    guard let source = tableDataSource(for: view), let tableView = source.tableView else { return }
    source.columns = raw.map {
        NDTableColumnSpec(
            id: $0["id"] as? String ?? "",
            title: $0["title"] as? String ?? "",
            width: ($0["width"] as? NSNumber)?.intValue
        )
    }
    for column in tableView.tableColumns { tableView.removeTableColumn(column) }
    for spec in source.columns {
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier(spec.id))
        column.title = spec.title
        if let width = spec.width {
            column.width = CGFloat(width)
        } else {
            column.resizingMask = [.autoresizingMask, .userResizingMask]
        }
        column.sortDescriptorPrototype = NSSortDescriptor(key: spec.id, ascending: true)
        tableView.addTableColumn(column)
    }
    tableView.reloadData()
    tableView.sizeLastColumnToFit()
}

/// Generated ndApplyProps Table.rows arm: selection preserved across the
/// rebuild inside the echo guard (SourceList's ndSourceListSetItems pattern).
func ndTableSetRows(_ view: NSView, _ raw: [[String: Any]]) {
    guard let source = tableDataSource(for: view), let tableView = source.tableView else { return }
    let prevSelected = tableView.selectedRow
    withEchoSuppressed(view) {
        source.rows = raw.map { obj in
            (obj["cells"] as? [Any])?.compactMap { $0 as? String } ?? []
        }
        tableView.reloadData()
        if prevSelected >= 0 && prevSelected < source.rows.count {
            tableView.selectRowIndexes(IndexSet(integer: prevSelected), byExtendingSelection: false)
        }
    }
}

/// Generated ndApplyProps Table.selectedIndex arm.
func ndTableSetSelectedIndex(_ view: NSView, _ index: Int) {
    guard let source = tableDataSource(for: view), let tableView = source.tableView else { return }
    guard tableView.selectedRow != index else { return }
    withEchoSuppressed(view) {
        if index >= 0 && index < source.rows.count {
            tableView.selectRowIndexes(IndexSet(integer: index), byExtendingSelection: false)
        } else {
            tableView.deselectAll(nil)
        }
    }
}

/// Generated ndApplyProps Table.showRowSeparators arm (horizontal grid lines
/// are AppKit's row-separator mechanism for view-based tables).
func ndTableSetShowRowSeparators(_ view: NSView, _ show: Bool) {
    guard let source = tableDataSource(for: view), let tableView = source.tableView else { return }
    tableView.gridStyleMask = show ? .solidHorizontalGridLineMask : []
}

/// Generated ndConnectEvents Table arm (one call wires all three events).
func ndTableConnect(_ view: NSView, nodeID: UInt32) {
    tableDataSource(for: view)?.nodeID = nodeID
}
