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
///
/// Geometry: columns always carry `.userResizingMask`, so a divider drag is
/// AppKit's own; the resulting widths ride `columnsResized` (debounced, since
/// the notification tracks the pointer through the drag). Header reordering
/// is `allowsColumnReordering` and reports through `columnsReordered`.
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
    var selectionMode = "single"
    weak var tableView: NSTableView?
    weak var scrollView: NSScrollView?
    /// Silences the geometry events while our own code rebuilds the columns:
    /// a data-driven rebuild is not a user gesture.
    var suppressColumnEvents = false
    private var resizeDebounce: DispatchWorkItem?

    func numberOfRows(in tableView: NSTableView) -> Int { rows.count }

    /// `selectionMode: "none"`. NSTableView has no flag for it, so the
    /// delegate refuses every row (GTK peer: GtkNoSelection).
    func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool {
        selectionMode != "none"
    }

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

    /// The payload carries the whole selection AND its first row in `index`,
    /// so a single-select app reading `e.index` is untouched by the multiple
    /// mode existing.
    func tableViewSelectionDidChange(_ notification: Notification) {
        guard let tableView = notification.object as? NSTableView,
              let scrollView = tableView.enclosingScrollView,
              !ndIsEchoSuppressed(scrollView) else { return }
        let indexes = tableView.selectedRowIndexes.sorted()
        let list = indexes.map(String.init).joined(separator: ",")
        let first = indexes.first ?? -1
        ndEmitEvent(nodeID, "selectionChanged", "{\"index\":\(first),\"data\":{\"indexes\":[\(list)]}}")
    }

    /// A live divider drag posts this per pixel, so the report waits for it
    /// to settle (PanedController's debounce, one work item per table).
    @objc func columnDidResize(_ note: Notification) {
        guard !suppressColumnEvents else { return }
        resizeDebounce?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.emitColumnWidths() }
        resizeDebounce = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.12, execute: work)
    }

    private func emitColumnWidths() {
        guard let tableView else { return }
        let body = tableView.tableColumns
            .map { "{\"id\":\(ndJsonString($0.identifier.rawValue)),\"width\":\(Int($0.width.rounded()))}" }
            .joined(separator: ",")
        ndEmitEvent(nodeID, "columnsResized", "{\"data\":{\"columns\":[\(body)]}}")
    }

    @objc func columnDidMove(_ note: Notification) {
        guard !suppressColumnEvents, let tableView else { return }
        let ids = tableView.tableColumns.map { ndJsonString($0.identifier.rawValue) }.joined(separator: ",")
        ndEmitEvent(nodeID, "columnsReordered", "{\"data\":{\"columnIds\":[\(ids)]}}")
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

/// release_node purge seam (Backend.swift's `ndPurgeNodeRegistries`). The
/// column-geometry observers are unowned references, so they go with it.
func ndTablePurge(_ view: NSView) {
    if let source = tableDataSource(for: view) {
        NotificationCenter.default.removeObserver(source)
    }
    tableDataSources[ObjectIdentifier(view)] = nil
}

/// `ndCreate`'s Table arm (generated) calls this.
func makeTable(_ props: [String: Any]) -> NSView {
    let tableView = NSTableView()
    tableView.usesAlternatingRowBackgroundColors = false
    let mode = propStr(props, "selectionMode") ?? "single"
    tableView.allowsMultipleSelection = mode == "multiple"
    tableView.allowsEmptySelection = true
    tableView.allowsColumnResizing = true
    tableView.allowsColumnReordering = propBool(props, "columnsReorderable") ?? false

    let source = NDTableDataSource()
    source.selectionMode = mode
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
    NotificationCenter.default.addObserver(
        source, selector: #selector(NDTableDataSource.columnDidResize(_:)),
        name: NSTableView.columnDidResizeNotification, object: tableView)
    NotificationCenter.default.addObserver(
        source, selector: #selector(NDTableDataSource.columnDidMove(_:)),
        name: NSTableView.columnDidMoveNotification, object: tableView)

    ndEmptyStateApply(scrollView, props)
    ndTableSetColumns(scrollView, propObjArray(props, "columns") ?? [])
    ndTableSetRows(scrollView, propObjArray(props, "rows") ?? [])
    ndTableSetShowRowSeparators(scrollView, propBool(props, "showRowSeparators") ?? true)
    let selIdx = propInt(props, "selectedIndex") ?? -1
    if selIdx >= 0 { ndTableSetSelectedIndex(scrollView, selIdx) }
    if let rows = propIntArray(props, "selectedIndexes") { ndTableSetSelectedIndexes(scrollView, rows) }
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
    source.suppressColumnEvents = true
    for column in tableView.tableColumns { tableView.removeTableColumn(column) }
    for spec in source.columns {
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier(spec.id))
        column.title = spec.title
        // A declared width is the starting width, never a lock: the divider
        // stays draggable either way, which is what columnsResized reports.
        column.resizingMask = spec.width == nil ? [.autoresizingMask, .userResizingMask] : [.userResizingMask]
        if let width = spec.width { column.width = CGFloat(width) }
        column.sortDescriptorPrototype = NSSortDescriptor(key: spec.id, ascending: true)
        tableView.addTableColumn(column)
    }
    tableView.reloadData()
    tableView.sizeLastColumnToFit()
    source.suppressColumnEvents = false
}

/// Generated ndApplyProps Table.selectedIndexes arm (the multiple-selection
/// peer of selectedIndex; the app owns both).
func ndTableSetSelectedIndexes(_ view: NSView, _ indexes: [Int]) {
    guard let source = tableDataSource(for: view), let tableView = source.tableView else { return }
    var set = IndexSet()
    for index in indexes where index >= 0 && index < source.rows.count { set.insert(index) }
    guard tableView.selectedRowIndexes != set else { return }
    withEchoSuppressed(view) {
        tableView.selectRowIndexes(set, byExtendingSelection: false)
    }
}

/// Generated ndApplyProps Table.columnsReorderable arm.
func ndTableSetColumnsReorderable(_ view: NSView, _ on: Bool) {
    tableDataSource(for: view)?.tableView?.allowsColumnReordering = on
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
    ndEmptyStateUpdate(view, isEmpty: raw.isEmpty)
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
