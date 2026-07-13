import AppKit

// A `navigation-sidebar` box (`<box cssClasses={["navigation-sidebar"]}>` of
// flat `<button>` rows) renders on the Mac as a REAL source-list NSTableView
// (`.sourceList` style) — the native primitive that gives accent-when-key /
// neutral-gray-when-unfocused selection, source-list row metrics/font, and row
// insets for free, instead of the hand-rolled layer pill this replaces.
//
// The box's `<button>` children stay in the tree (the reconciler keeps
// appending/removing/reordering them) but become the table's ROW MODEL: each
// arranged NDButton is hidden and its `title` + `onClick` back a table row.
// Selection is driven off which button carries `suggested-action` (the app's
// selection signal); a user row click is routed back to that button's wired
// `clicked` action, so the SAME app tree drives navigation with no per-row
// styling. The tracked handle stays the box (NSStackView); the table's
// NSScrollView is a background subview pinned to it.

/// Per-box table controller, keyed by the box's identity. NOT private — read
/// from `ndBoxChildAttached`/`ndBoxChildDetached` (Layout.swift) and
/// `ndApplyCssClasses` (Backend.swift).
nonisolated(unsafe) var ndSidebarTables: [ObjectIdentifier: NDSidebarTable] = [:]

/// Buttons currently acting as source-list rows (hidden data providers). Lets
/// `ndApplyCssClasses` skip the generic push-button bezel/keyEquivalent for
/// them — the table owns selection.
nonisolated(unsafe) var ndSidebarRowButtons: Set<ObjectIdentifier> = []

private let ndSidebarCellID = NSUserInterfaceItemIdentifier("nd-sidebar-cell")
private let ndSidebarColumnID = NSUserInterfaceItemIdentifier("nd-sidebar-col")

final class NDSidebarTable: NSObject, NSTableViewDataSource, NSTableViewDelegate {
    weak var box: NSStackView?
    let scrollView: NSScrollView
    let tableView: NSTableView
    /// Guards `tableViewSelectionDidChange` so a programmatic `selectRowIndexes`
    /// (selection following the app's `suggested-action`) doesn't fire the
    /// row's `onClick` and loop.
    private var updatingSelection = false

    init(box: NSStackView) {
        self.scrollView = NSScrollView()
        self.tableView = NSTableView()
        self.box = box
        super.init()
        let column = NSTableColumn(identifier: ndSidebarColumnID)
        column.resizingMask = .autoresizingMask
        tableView.addTableColumn(column)
        tableView.headerView = nil
        tableView.style = .sourceList
        tableView.selectionHighlightStyle = .sourceList
        tableView.backgroundColor = .clear      // vibrancy sidebar shows through
        tableView.autoresizingMask = [.width]
        tableView.dataSource = self
        tableView.delegate = self
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = false
        scrollView.documentView = tableView
        tableView.sizeLastColumnToFit()
    }

    /// The row model: the box's arranged NDButton children, in order (hidden
    /// ones included — `arrangedSubviews` keeps them).
    var rowButtons: [NDButton] {
        box?.arrangedSubviews.compactMap { $0 as? NDButton } ?? []
    }

    func numberOfRows(in tableView: NSTableView) -> Int { rowButtons.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let cell = (tableView.makeView(withIdentifier: ndSidebarCellID, owner: self) as? NSTableCellView) ?? {
            let c = NSTableCellView()
            c.identifier = ndSidebarCellID
            let tf = NSTextField(labelWithString: "")
            tf.translatesAutoresizingMaskIntoConstraints = false
            tf.lineBreakMode = .byTruncatingTail
            c.addSubview(tf)
            c.textField = tf     // lets the source-list cell own the selected-row text color
            NSLayoutConstraint.activate([
                tf.leadingAnchor.constraint(equalTo: c.leadingAnchor, constant: 4),
                tf.trailingAnchor.constraint(equalTo: c.trailingAnchor, constant: -4),
                tf.centerYAnchor.constraint(equalTo: c.centerYAnchor),
            ])
            return c
        }()
        let buttons = rowButtons
        cell.textField?.stringValue = row < buttons.count ? buttons[row].title : ""
        return cell
    }

    /// Rebuilds the rows and re-syncs the selection (rows changed: attach/
    /// detach/reorder).
    func reload() {
        tableView.reloadData()
        syncSelection()
    }

    /// The selected row = the button carrying `suggested-action`.
    private func selectedIndex() -> Int {
        for (i, btn) in rowButtons.enumerated() where ndCssClasses(of: btn).contains("suggested-action") { return i }
        return -1
    }

    /// Aligns the table's selection with the app's `suggested-action` row,
    /// programmatically (guarded so it doesn't echo back as a row click).
    func syncSelection() {
        let want = selectedIndex()
        guard tableView.selectedRow != want else { return }
        updatingSelection = true
        if want >= 0 {
            tableView.selectRowIndexes(IndexSet(integer: want), byExtendingSelection: false)
        } else {
            tableView.deselectAll(nil)
        }
        updatingSelection = false
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        guard !updatingSelection else { return }   // programmatic sync, not a user click
        let sel = tableView.selectedRow
        let buttons = rowButtons
        guard sel >= 0, sel < buttons.count else { return }
        let btn = buttons[sel]
        // Route the user's selection to the backing button's wired `clicked`
        // action — the exact same path a real click on the button would take,
        // so the app's onClick (category switch) runs unchanged.
        if let action = btn.action, let target = btn.target {
            NSApp.sendAction(action, to: target, from: btn)
        }
    }
}

/// Installs the source-list table for a `navigation-sidebar` box (idempotent):
/// marks/hides the existing button children as rows and pins the table's scroll
/// view to fill the box.
func ndInstallSidebarTable(_ box: NSStackView) {
    let id = ObjectIdentifier(box)
    if ndSidebarTables[id] != nil { return }
    let table = NDSidebarTable(box: box)
    ndSidebarTables[id] = table
    for btn in box.arrangedSubviews.compactMap({ $0 as? NDButton }) { ndMarkSidebarRowButton(btn) }
    let sv = table.scrollView
    sv.translatesAutoresizingMaskIntoConstraints = false
    box.addSubview(sv, positioned: .above, relativeTo: nil)
    NSLayoutConstraint.activate([
        sv.leadingAnchor.constraint(equalTo: box.leadingAnchor),
        sv.trailingAnchor.constraint(equalTo: box.trailingAnchor),
        sv.topAnchor.constraint(equalTo: box.topAnchor),
        sv.bottomAnchor.constraint(equalTo: box.bottomAnchor),
    ])
    table.reload()
}

/// Reverses `ndInstallSidebarTable` when a box drops `navigation-sidebar`
/// (set-replace): removes the table and un-hides the button children.
func ndRemoveSidebarTable(_ box: NSStackView) {
    let id = ObjectIdentifier(box)
    guard let table = ndSidebarTables[id] else { return }
    table.scrollView.removeFromSuperview()
    ndSidebarTables[id] = nil
    for btn in box.arrangedSubviews.compactMap({ $0 as? NDButton }) {
        ndSidebarRowButtons.remove(ObjectIdentifier(btn))
        btn.alphaValue = 1
    }
}

/// Marks `btn` a source-list row: made invisible (the table draws the row) and
/// stripped of any generic `suggested-action` default-button idiom it may have
/// picked up before it was known to be a row (create applies its class before
/// attach). Uses `alphaValue = 0` rather than `isHidden` so the button stays
/// ACTIONABLE — the automation/accessibility harness (`checkActionable`:
/// `!isHidden && window != nil`) can still drive the sidebar by testID, and a
/// semantic click fires the row's `onClick` exactly as a table-row click does.
/// The button sits behind the source-list table (added `.above`), so real
/// clicks land on the table, not it.
func ndMarkSidebarRowButton(_ btn: NDButton) {
    let id = ObjectIdentifier(btn)
    guard !ndSidebarRowButtons.contains(id) else { return }
    ndSidebarRowButtons.insert(id)
    btn.alphaValue = 0
    btn.keyEquivalent = ""
    btn.bezelColor = nil
}
