import AppKit

/// Hand-written ListView host code (M6b-D2): a single-column `NSTableView`
/// inside an `NSScrollView`, view-based row recycling over a flat
/// `[String]`. The tracked handle the core sees is the `NSScrollView`
/// (matches GTK's `ScrolledWindow` wrapping `GtkListView`); `getTree`'s
/// `itemCount` is derived core-side from `items.len` (src/tree.zig:32) —
/// this file never reports it directly, and recycled row views are never
/// added to the automation tree (only the tracked NSScrollView is).
private let listColumnID = NSUserInterfaceItemIdentifier("nd-listview-column")
private let listCellID = NSUserInterfaceItemIdentifier("nd-listview-cell")

final class ListViewDataSource: NSObject, NSTableViewDataSource, NSTableViewDelegate {
    var items: [String] = []
    var selectedIndex: Int = -1
    weak var tableView: NSTableView?

    func numberOfRows(in tableView: NSTableView) -> Int { items.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let cell = tableView.makeView(withIdentifier: listCellID, owner: self) as? NSTableCellField
            ?? NSTableCellField()
        cell.identifier = listCellID
        cell.stringValue = row < items.count ? items[row] : ""
        return cell
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        guard let tableView = notification.object as? NSTableView else { return }
        selectedIndex = tableView.selectedRow
        guard let scrollView = tableView.enclosingScrollView else { return }
        EventDispatcher.shared.wiringFireIndex(scrollView, index: selectedIndex)
    }
}

/// A plain, non-editable text cell (view-based row recycling reuses
/// instances via `makeView(withIdentifier:)`, so this must be cheap to
/// reconfigure — just `stringValue`, no per-row layout work).
private final class NSTableCellField: NSTableCellView {
    let label = NSTextField(labelWithString: "")

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        commonInit()
    }
    required init?(coder: NSCoder) {
        super.init(coder: coder)
        commonInit()
    }
    private func commonInit() {
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(label)
        textField = label
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 4),
            label.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -4),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }
    var stringValue: String {
        get { label.stringValue }
        set { label.stringValue = newValue }
    }
}

// Associates each tracked NSScrollView handle with its data source (the
// NSTableView's dataSource/delegate are weak-ish in practice via AppKit's
// own retain, but we own the source object's lifetime here explicitly since
// nothing else references it).
nonisolated(unsafe) private var listDataSources: [ObjectIdentifier: ListViewDataSource] = [:]

/// `ndCreate`'s ListView arm (generated) calls this. Builds the
/// NSScrollView + single-column NSTableView; the returned NSScrollView is
/// the tracked `nd_widget` handle.
func makeListView(_ props: [String: Any]) -> NSView {
    let tableView = NSTableView()
    let column = NSTableColumn(identifier: listColumnID)
    column.title = ""
    column.resizingMask = .autoresizingMask
    tableView.addTableColumn(column)
    tableView.headerView = nil
    tableView.rowSizeStyle = .default
    tableView.usesAlternatingRowBackgroundColors = true

    let source = ListViewDataSource()
    source.tableView = tableView
    if let items = propArray(props, "items") { source.items = items }
    if let idx = propInt(props, "selectedIndex") { source.selectedIndex = idx }
    tableView.dataSource = source
    tableView.delegate = source

    let scrollView = NSScrollView()
    scrollView.hasVerticalScroller = true
    scrollView.documentView = tableView
    // Table views need their column to track the scroll view's width.
    tableView.autoresizingMask = [.width]
    tableView.sizeLastColumnToFit()

    listDataSources[ObjectIdentifier(scrollView)] = source
    if source.selectedIndex >= 0 && source.selectedIndex < source.items.count {
        tableView.selectRowIndexes(IndexSet(integer: source.selectedIndex), byExtendingSelection: false)
    }
    tableView.reloadData()
    ndEmptyStateApply(scrollView, props)
    ndEmptyStateUpdate(scrollView, isEmpty: source.items.isEmpty)
    return scrollView
}

private func dataSource(for view: NSView) -> ListViewDataSource? {
    listDataSources[ObjectIdentifier(view)]
}

/// `ndApplyProps`'s ListView.items arm (generated) calls this with the
/// tracked NSScrollView handle.
func ndListViewSetItems(_ view: NSView, _ items: [String]) {
    guard let source = dataSource(for: view) else { return }
    source.items = items
    source.tableView?.reloadData()
    ndEmptyStateUpdate(view, isEmpty: items.isEmpty)
}

/// `ndApplyProps`'s ListView.selectedIndex arm (generated) calls this.
func ndListViewSetSelectedIndex(_ view: NSView, _ index: Int) {
    guard let source = dataSource(for: view), let tableView = source.tableView else { return }
    guard source.selectedIndex != index else { return }
    withEchoSuppressed(view) {
        source.selectedIndex = index
        if index >= 0 && index < source.items.count {
            tableView.selectRowIndexes(IndexSet(integer: index), byExtendingSelection: false)
        } else {
            tableView.deselectAll(nil)
        }
    }
}
