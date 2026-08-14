import AppKit

// A sidebar box (`<box cssClasses={["navigation-sidebar"]}>` of flat
// `<button>` rows) renders on the Mac as a REAL source-list NSTableView
// (`.sourceList` style) — the native primitive that gives accent-when-key /
// neutral-gray-when-unfocused selection, source-list row metrics/font, and row
// insets for free, instead of the hand-rolled layer pill this replaces.
//
// `navigation-sidebar` is libadwaita's own class, so it is the portable
// contract: the app tree that gets a native sidebar on GTK gets one here too,
// with no second `nd-`prefixed name to opt into. It is gated on the box's
// children actually being row-shaped (`ndIsSourceListShaped`), because the
// table covers the whole box and a composite row (button + caption + badge)
// would lose everything that is not the button. `nd-native-sidebar` stays for
// exactly those boxes as an unconditional takeover that skips the gate.
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

/// Sidebar boxes whose takeover is unconditional, i.e. the ones carrying
/// `nd-native-sidebar`. That class predates routing `navigation-sidebar` here
/// and stays a gate bypass, because the boxes using it (examples/
/// ndwidgets-probe) mix section labels in with the row buttons, a shape
/// `ndIsSourceListShaped` declines. Set-replace, purged with the node.
nonisolated(unsafe) var ndForcedSidebars: Set<ObjectIdentifier> = []

/// Row buttons in a sidebar whose takeover was declined: they draw themselves
/// (`ndApplySidebarRowStyle`) instead of backing a table row. Read from
/// `ndApplyCssClasses`, which has to re-assert that rendering after its
/// set-replace baseline resets the bezel.
nonisolated(unsafe) var ndSidebarFallbackRows: Set<ObjectIdentifier> = []

private let ndSidebarCellID = NSUserInterfaceItemIdentifier("nd-sidebar-cell")
private let ndSidebarColumnID = NSUserInterfaceItemIdentifier("nd-sidebar-col")

/// Walks up from `view` to the nearest ancestor (inclusive) carrying a sidebar
/// class. Row buttons in a real app's sidebar routinely sit several structural
/// wrapper boxes below the classed box itself (a host/project section around
/// each run row), so attach/detach hooks (`ndBoxChildAttached`/
/// `ndBoxChildDetached`, Layout.swift) can't assume `view` IS the tracked box.
func ndEnclosingSidebar(_ view: NSView) -> NSStackView? {
    var v: NSView? = view
    while let cur = v {
        if let stack = cur as? NSStackView, ndNavigationSidebars.contains(ObjectIdentifier(stack)) {
            return stack
        }
        v = cur.superview
    }
    return nil
}

/// The source-list table backing `view`'s enclosing sidebar, if that sidebar
/// got the takeover at all. A sidebar the gate declined has no table, and its
/// rows must not fall through to an outer sidebar's.
func ndEnclosingSidebarTable(_ view: NSView) -> NDSidebarTable? {
    ndEnclosingSidebar(view).flatMap { ndSidebarTables[ObjectIdentifier($0)] }
}

final class NDSidebarTable: NSObject, NSTableViewDataSource, NSTableViewDelegate {
    weak var box: NSStackView?
    let scrollView: NSScrollView
    let tableView: NSTableView
    /// Guards native row activation while selection is being synchronized from
    /// the app's `suggested-action` state.
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
        // Route user activation through NSTableView's action and `clickedRow`,
        // which is the row AppKit actually hit-tested for the mouse event.
        // `tableViewSelectionDidChange`/`selectedRow` is not an activation API:
        // it can describe an earlier selection during delegate callbacks and it
        // does not fire at all when the already-selected row is clicked again.
        tableView.target = self
        tableView.action = #selector(rowClicked(_:))
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = false
        scrollView.documentView = tableView
        tableView.sizeLastColumnToFit()
    }

    /// The row model: every NDButton in the box's subtree, document order, at
    /// ANY depth — a real app's sidebar commonly nests row buttons inside
    /// structural wrapper boxes (a host/project section around each run row),
    /// not as direct children of the nd-native-sidebar box itself. Recursion
    /// stops at a nested `nd-native-sidebar` box, which owns its own rows.
    var rowButtons: [NDButton] {
        box.map(NDSidebarTable.collectRowButtons) ?? []
    }

    static func collectRowButtons(_ view: NSView) -> [NDButton] {
        guard let stack = view as? NSStackView else { return [] }
        var out: [NDButton] = []
        for child in stack.arrangedSubviews {
            if let btn = child as? NDButton {
                out.append(btn)
            } else if !ndNavigationSidebars.contains(ObjectIdentifier(child)) {
                out.append(contentsOf: collectRowButtons(child))
            }
        }
        return out
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
    /// detach/reorder — anywhere in the subtree, not just directly under
    /// `box`). Marks any row button newly reached by the recursive walk
    /// (idempotent — a wrapper box, e.g. a host/project section, can attach
    /// its own already-populated button descendants in one shot).
    func reload() {
        for btn in rowButtons { ndMarkSidebarRowButton(btn) }
        // The table may be installed before React appends the backing buttons.
        // Every later addArrangedSubview becomes a newer sibling and can sit
        // above the table despite the install-time `.above` positioning. Move
        // the table back to the front whenever the row model changes.
        box?.addSubview(scrollView, positioned: .above, relativeTo: nil)
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
        // Selection is visual state only. User activation is handled by
        // `rowClicked`, whose `clickedRow` comes from the native mouse hit-test.
        // Keeping this delegate method intentionally side-effect-free also
        // prevents programmatic selection sync from echoing an app click.
    }

    @objc private func rowClicked(_ sender: NSTableView) {
        guard !updatingSelection else { return }
        let row = sender.clickedRow
        let buttons = rowButtons
        guard row >= 0, row < buttons.count else { return }
        let btn = buttons[row]
        // Route the native row click to the backing button's wired `clicked`
        // action so the unchanged app tree receives its normal onClick event.
        if let action = btn.action, let target = btn.target {
            NSApp.sendAction(action, to: target, from: btn)
        }
    }
}

/// Installs the source-list table for a sidebar box (idempotent): marks/hides
/// the existing button children as rows and pins the table's scroll view to
/// fill the box. Reached through `ndReconcileSidebarTable`, never directly,
/// since that is where the row-shape gate lives.
func ndInstallSidebarTable(_ box: NSStackView) {
    let id = ObjectIdentifier(box)
    if ndSidebarTables[id] != nil { return }
    let table = NDSidebarTable(box: box)
    ndSidebarTables[id] = table
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

/// Reverses `ndInstallSidebarTable` when a box drops its sidebar class, or
/// when its rows stop being row-shaped (set-replace): removes the table and
/// un-hides the button children.
func ndRemoveSidebarTable(_ box: NSStackView) {
    let id = ObjectIdentifier(box)
    guard let table = ndSidebarTables[id] else { return }
    table.scrollView.removeFromSuperview()
    ndSidebarTables[id] = nil
    for btn in NDSidebarTable.collectRowButtons(box) {
        ndSidebarRowButtons.remove(ObjectIdentifier(btn))
        btn.ndIsSidebarRowModel = false
        btn.setAccessibilityHidden(false)
        btn.alphaValue = 1
    }
}

/// Marks `btn` as a source-list row model. The table draws and physically
/// handles the row; the backing button remains mounted only so semantic
/// automation can target its testID and dispatch its existing onClick action.
/// It is explicitly excluded from AppKit hit testing and accessibility because
/// its padded NSStackView frame differs from the table's native row geometry;
/// alpha alone does not make an NSView non-interactive.
func ndMarkSidebarRowButton(_ btn: NDButton) {
    let id = ObjectIdentifier(btn)
    guard !ndSidebarRowButtons.contains(id) else { return }
    ndSidebarRowButtons.insert(id)
    btn.ndIsSidebarRowModel = true
    btn.setAccessibilityHidden(true)
    btn.alphaValue = 0
    btn.keyEquivalent = ""
    btn.bezelColor = nil
}

/// Whether a sidebar box's children are actually ROW-SHAPED: every arranged
/// descendant is either a structural wrapper box or a titled button, and there
/// is at least one button. The table is pinned over the whole box, so anything
/// in there that is not a button (a caption, a badge, an icon, a separator:
/// the composite libadwaita row) would be silently covered by it. Those boxes
/// keep the per-row fallback rendering instead of losing content to the
/// takeover.
func ndIsSourceListShaped(_ box: NSStackView) -> Bool {
    var buttons = 0
    func walk(_ stack: NSStackView) -> Bool {
        for child in stack.arrangedSubviews {
            if let btn = child as? NDButton {
                guard !btn.title.isEmpty else { return false }
                buttons += 1
            } else if let nested = child as? NSStackView {
                // A nested sidebar owns its own rows (same stop as
                // `collectRowButtons`), so it does not vote on this one.
                if ndNavigationSidebars.contains(ObjectIdentifier(nested)) { continue }
                guard walk(nested) else { return false }
            } else {
                return false
            }
        }
        return true
    }
    return walk(box) && buttons > 0
}

/// Decides whether a sidebar-classed box is backed by the source-list table
/// and keeps that decision current. Called from `ndApplyCssClasses` when the
/// class lands and from `ndBoxChildAttached`/`ndBoxChildDetached` for every
/// later shape change, so a box that only becomes row-shaped once React has
/// appended its rows still gets the takeover, and one that stops being
/// row-shaped gives it back.
func ndReconcileSidebarTable(_ box: NSStackView) {
    // An empty box is "not populated yet", not "not row-shaped": the class
    // lands before React appends anything (src/tree.zig applies props before
    // append), and replacing every row at once momentarily empties the box.
    // Neither should install a table or tear a live one down.
    if box.arrangedSubviews.isEmpty { return }
    let id = ObjectIdentifier(box)
    if ndForcedSidebars.contains(id) || ndIsSourceListShaped(box) {
        ndClearSidebarRowFallback(box)
        if let table = ndSidebarTables[id] {
            table.reload()
        } else {
            ndInstallSidebarTable(box)
        }
    } else {
        ndRemoveSidebarTable(box)
        for btn in NDSidebarTable.collectRowButtons(box) {
            ndSidebarFallbackRows.insert(ObjectIdentifier(btn))
            ndApplySidebarRowStyle(btn, selected: ndCssClasses(of: btn).contains("suggested-action"))
        }
    }
}

/// Source-list row rendering for a sidebar the takeover declined. The rows
/// stay ordinary buttons, but must stop reading as push buttons in a control
/// strip:
///  - no bezel on ANY row, selected or not. Selection used to come from
///    `suggested-action`'s bordered accent bezel, and a bordered bezel
///    reserves horizontal content insets a borderless one does not, so the
///    selected row's title started ~15pt right of its neighbours': picking a
///    row visibly shifted its own text.
///  - selection is the accent tint instead, which is what a row with no table
///    under it can honestly draw.
///  - no keyEquivalent. A sidebar row is not the window's Return-key default
///    button, which is what `suggested-action` otherwise makes it.
func ndApplySidebarRowStyle(_ btn: NDButton, selected: Bool) {
    btn.isBordered = false
    btn.showsBorderOnlyWhileMouseInside = false
    btn.bezelColor = nil
    btn.keyEquivalent = ""
    btn.contentTintColor = selected ? .controlAccentColor : .labelColor
}

/// Reverses `ndApplySidebarRowStyle` when the takeover arrives after all, or
/// when the sidebar class drops: each row goes back to whatever its own
/// cssClasses say, replayed from the set `ndApplyCssClasses` recorded.
func ndClearSidebarRowFallback(_ box: NSStackView) {
    for btn in NDSidebarTable.collectRowButtons(box) {
        guard ndSidebarFallbackRows.remove(ObjectIdentifier(btn)) != nil else { continue }
        btn.contentTintColor = nil
        ndApplyCssClasses(btn, ndCssClasses(of: btn))
    }
}
