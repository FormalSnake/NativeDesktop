import AppKit

/// Hand-written SourceList host code (M11 SourceList Wave 2): an
/// `NSTableView` in `.sourceList` style inside an `NSScrollView`, view-based
/// row recycling over `[SourceRow]` (sibling of ListView.swift's flat
/// `[String]` recycling, same overall shape). The tracked handle the core
/// sees is the `NSScrollView` (matches ListView's contract); `getTree`'s
/// `rows`/`itemCount` are derived core-side from `items` (src/tree.zig) —
/// this file never reports them directly, and recycled cell views are never
/// added to the automation tree (only the tracked NSScrollView is).
///
/// Unlike ListView (one index-payload event, `rowActivated`), SourceList
/// carries TWO on the same tracked view — `selectionChanged` (row click) and
/// `rowActivated` (double-click, or a semantic click on the current
/// selection) — so firing goes through `EventDispatcher.fireIndexNamed`
/// (explicit event name) instead of `wiringFireIndex` (which picks the
/// view's sole wired event name; ambiguous here).
private let sourceListColumnID = NSUserInterfaceItemIdentifier("nd-sourcelist-column")
private let sourceListCellID = NSUserInterfaceItemIdentifier("nd-sourcelist-cell")

/// One SourceList row (peer of the GTK backend's `ndBuildSourceRows` object
/// shape: `title`/`badge`/`iconName` read off the `items` objectList prop).
struct SourceRow {
    var title: String
    var badge: String?
    var iconName: String?
}

final class SourceListDataSource: NSObject, NSTableViewDataSource, NSTableViewDelegate {
    var rows: [SourceRow] = []
    weak var scrollView: NSScrollView?
    weak var tableView: NSTableView?

    func numberOfRows(in tableView: NSTableView) -> Int { rows.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let cell = tableView.makeView(withIdentifier: sourceListCellID, owner: self) as? NDSourceCell
            ?? NDSourceCell()
        cell.identifier = sourceListCellID
        cell.configure(with: row < rows.count ? rows[row] : SourceRow(title: "", badge: nil, iconName: nil))
        return cell
    }

    /// Fires on every selection change, user-driven OR programmatic
    /// (`selectRowIndexes` posts this notification unconditionally — the
    /// asymmetry `Automation.swift`'s `semanticSetValue` SourceList arm
    /// relies on instead of re-firing manually). `ndSourceListSetItems`/
    /// `ndSourceListSetSelectedIndex` suppress the echo via
    /// `withEchoSuppressed` for the React-driven (non-semantic) path.
    func tableViewSelectionDidChange(_ notification: Notification) {
        guard let tableView = notification.object as? NSTableView,
              let scrollView = tableView.enclosingScrollView else { return }
        EventDispatcher.shared.fireIndexNamed(scrollView, name: "selectionChanged", index: tableView.selectedRow)
    }

    /// `doubleAction` target (wired in `makeSourceList`). `clickedRow` is -1
    /// for a double-click outside any row (e.g. below the last row) — no
    /// event in that case, mirroring GTK's row-or-nothing dispatch.
    @objc func rowDoubleClicked(_ sender: NSTableView) {
        guard let scrollView = sender.enclosingScrollView, sender.clickedRow >= 0 else { return }
        EventDispatcher.shared.fireIndexNamed(scrollView, name: "rowActivated", index: sender.clickedRow)
    }
}

/// Sidebar row cell: optional leading SF Symbol, a title, and an optional
/// trailing badge (peer of the GTK backend's AdwActionRow prefix-icon/
/// suffix-badge layout). View-based row recycling reuses instances via
/// `makeView(withIdentifier:)` (same idiom as ListView.swift's
/// `NSTableCellField`), so `configure` must be cheap to re-run per row.
final class NDSourceCell: NSTableCellView {
    private let iconView = NSImageView()
    private let titleField = NSTextField(labelWithString: "")
    private let badgeField = NSTextField(labelWithString: "")
    private var iconWidthConstraint: NSLayoutConstraint!

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
        badgeField.translatesAutoresizingMaskIntoConstraints = false
        badgeField.textColor = .secondaryLabelColor
        badgeField.font = .monospacedDigitSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular)
        badgeField.alignment = .right

        addSubview(iconView)
        addSubview(titleField)
        addSubview(badgeField)
        textField = titleField

        iconWidthConstraint = iconView.widthAnchor.constraint(equalToConstant: 16)
        NSLayoutConstraint.activate([
            iconView.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 4),
            iconView.centerYAnchor.constraint(equalTo: centerYAnchor),
            iconWidthConstraint,
            iconView.heightAnchor.constraint(equalToConstant: 16),

            titleField.leadingAnchor.constraint(equalTo: iconView.trailingAnchor, constant: 6),
            titleField.centerYAnchor.constraint(equalTo: centerYAnchor),

            badgeField.leadingAnchor.constraint(greaterThanOrEqualTo: titleField.trailingAnchor, constant: 6),
            badgeField.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -4),
            badgeField.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    func configure(with row: SourceRow) {
        titleField.stringValue = row.title
        if let iconName = row.iconName {
            let symbol = ndSFSymbol(forFreedesktop: iconName) ?? iconName  // NDShell/Icons.swift
            iconView.image = NSImage(systemSymbolName: symbol, accessibilityDescription: row.title)
        } else {
            iconView.image = nil
        }
        iconView.isHidden = iconView.image == nil
        iconWidthConstraint.constant = iconView.isHidden ? 0 : 16
        badgeField.stringValue = row.badge ?? ""
        badgeField.isHidden = row.badge == nil
    }
}

// Associates each tracked NSScrollView handle with its data source (same
// idiom as ListView.swift's `listDataSources`).
nonisolated(unsafe) private var sourceListDataSources: [ObjectIdentifier: SourceListDataSource] = [:]

/// release_node purge seam (Backend.swift's `ndPurgeNodeRegistries`).
func ndSourceListPurge(_ view: NSView) {
    sourceListDataSources[ObjectIdentifier(view)] = nil
}

private func sourceRow(from obj: [String: Any]) -> SourceRow {
    SourceRow(
        title: obj["title"] as? String ?? "",
        badge: obj["badge"] as? String,
        iconName: obj["iconName"] as? String
    )
}

/// `ndCreate`'s SourceList arm (generated) calls this. Builds the
/// NSScrollView + single-column `.sourceList`-style NSTableView; the
/// returned NSScrollView is the tracked `nd_widget` handle.
func makeSourceList(_ props: [String: Any]) -> NSView {
    let tableView = NSTableView()
    let column = NSTableColumn(identifier: sourceListColumnID)
    column.title = ""
    column.resizingMask = .autoresizingMask
    tableView.addTableColumn(column)
    tableView.headerView = nil
    tableView.style = .sourceList
    tableView.selectionHighlightStyle = .sourceList
    tableView.backgroundColor = .clear // glass sidebar shows through, paired with scrollView.drawsBackground below
    tableView.autoresizingMask = [.width]

    let source = SourceListDataSource()
    source.tableView = tableView
    if let raw = propObjArray(props, "items") { source.rows = raw.map(sourceRow(from:)) }
    tableView.dataSource = source
    tableView.delegate = source
    tableView.target = source
    tableView.doubleAction = #selector(SourceListDataSource.rowDoubleClicked(_:))

    let scrollView = NSScrollView()
    scrollView.hasVerticalScroller = true
    scrollView.documentView = tableView
    scrollView.drawsBackground = false // glass sidebar shows through
    tableView.sizeLastColumnToFit()
    source.scrollView = scrollView

    sourceListDataSources[ObjectIdentifier(scrollView)] = source
    let selIdx = propInt(props, "selectedIndex") ?? -1
    if selIdx >= 0 && selIdx < source.rows.count {
        tableView.selectRowIndexes(IndexSet(integer: selIdx), byExtendingSelection: false)
    }
    tableView.reloadData()
    ndEmptyStateApply(scrollView, props)
    ndEmptyStateUpdate(scrollView, isEmpty: source.rows.isEmpty)
    return scrollView
}

private func sourceListDataSource(for view: NSView) -> SourceListDataSource? {
    sourceListDataSources[ObjectIdentifier(view)]
}

/// `ndApplyProps`'s SourceList.items arm (generated) calls this with the
/// tracked NSScrollView handle. Preserves the current selection across the
/// rebuild (peer of the GTK backend's `blockEcho`-wrapped removeAll+rebuild+
/// reselect for the same reason: a naive reload could otherwise leak a
/// transient deselect echo before the row lands back in the same place).
func ndSourceListSetItems(_ view: NSView, _ raw: [[String: Any]]) {
    guard let source = sourceListDataSource(for: view), let tableView = source.tableView else { return }
    let prevSelected = tableView.selectedRow
    withEchoSuppressed(view) {
        source.rows = raw.map(sourceRow(from:))
        tableView.reloadData()
        if prevSelected >= 0 && prevSelected < source.rows.count {
            tableView.selectRowIndexes(IndexSet(integer: prevSelected), byExtendingSelection: false)
        }
    }
    ndEmptyStateUpdate(view, isEmpty: raw.isEmpty)
}

/// `ndApplyProps`'s SourceList.selectedIndex arm (generated) calls this.
/// Exact peer of `ndListViewSetSelectedIndex` (ListView.swift).
func ndSourceListSetSelectedIndex(_ view: NSView, _ index: Int) {
    guard let source = sourceListDataSource(for: view), let tableView = source.tableView else { return }
    guard tableView.selectedRow != index else { return }
    withEchoSuppressed(view) {
        if index >= 0 && index < source.rows.count {
            tableView.selectRowIndexes(IndexSet(integer: index), byExtendingSelection: false)
        } else {
            tableView.deselectAll(nil)
        }
    }
}
