import AppKit

/// CommandPalette: a centered, dimmed, modal Cmd-K overlay (peer of
/// src/gtk/commandpalette.zig's AdwDialog). The tracked handle is a host-only
/// NSView (the Popover idiom) that lives in the tree only so `self.window`
/// resolves the window to paint over; `open` toggles a full-window dimmed
/// scrim plus a centered card (NSSearchField over an NSTableView of results).
///
/// CONTROLLED: the app owns `query` and `items`; the widget never filters or
/// reorders. Every keystroke fires queryChanged; the app feeds back the next
/// result set. Highlight is internal (Up/Down/Home/End clamp within the
/// current rows). onActivate carries the highlighted/clicked row's id;
/// onSubmit the raw query text (plain Return with no highlight, or Cmd/Ctrl
/// Return regardless) so a directory picker can accept a typed path that
/// matches no listed row. onCancel fires on Esc / click-outside; a
/// React-driven close (open=false) is flagged so it does not echo.
private let paletteColumnID = NSUserInterfaceItemIdentifier("nd-command-palette-column")
private let paletteCellID = NSUserInterfaceItemIdentifier("nd-command-palette-cell")

/// One result row (peer of the GTK backend's CommandPaletteItem decode).
/// Sendable so the nonisolated dispatch bridges can decode the objectList and
/// hand the finished rows to the MainActor handle (the untyped [[String: Any]]
/// itself is not Sendable, so it never crosses the actor boundary).
struct PaletteRow: Sendable {
    var id: String
    var title: String
    var subtitle: String?
    var iconName: String?
}

private func paletteRow(from obj: [String: Any]) -> PaletteRow {
    PaletteRow(
        id: obj["id"] as? String ?? "",
        title: obj["title"] as? String ?? "",
        subtitle: obj["subtitle"] as? String,
        iconName: obj["iconName"] as? String
    )
}

/// Full-window dim behind the card; a click on the bare scrim cancels.
private final class NDPaletteScrim: NSView {
    weak var handle: NDCommandPaletteHandleView?
    override var isFlipped: Bool { true }
    override func mouseDown(with event: NSEvent) { handle?.userCancel() }
}

/// The floating card. Owns the Cmd/Ctrl+Return "submit as-is" key equivalent
/// (a modifier-Return never reaches the field editor's doCommandBySelector).
private final class NDPaletteCard: NSVisualEffectView {
    weak var handle: NDCommandPaletteHandleView?
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        if event.type == .keyDown,
           event.modifierFlags.contains(.command) || event.modifierFlags.contains(.control),
           event.keyCode == 36 || event.keyCode == 76 { // Return / keypad Enter
            handle?.emitSubmit()
            return true
        }
        return super.performKeyEquivalent(with: event)
    }
}

final class NDCommandPaletteHandleView: NSView, NSSearchFieldDelegate, NSTableViewDataSource, NSTableViewDelegate {
    var nodeID: UInt32 = 0
    fileprivate var placeholder: String?
    fileprivate var rows: [PaletteRow] = []
    private var pendingQuery = ""
    // Content fingerprint of the rendered rows. A controlled app hands back a
    // fresh `items` array on every render; without this the table would reload
    // and reset the highlight on each one, so reload only when rows change.
    private var rowsSig = ""

    private var pendingOpen = false
    private var presented = false
    private var programmaticClose = false
    private var suppressQueryEmit = false

    private var scrim: NDPaletteScrim?
    private var searchField: NSSearchField?
    private var tableView: NSTableView?

    override init(frame: NSRect) {
        super.init(frame: frame)
        isHidden = true // host-only handle: never takes layout space
    }
    required init?(coder: NSCoder) {
        super.init(coder: coder)
        isHidden = true
    }
    override var intrinsicContentSize: NSSize { .zero }

    // The scrim/card are subviews of the window's contentView, not of this
    // handle, so an unmount-while-open would strand them — tear down when the
    // handle leaves its window.
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil && presented { dismiss(programmatic: true) }
    }

    // ---- controlled props ----

    func setPlaceholder(_ ph: String) {
        placeholder = ph
        searchField?.placeholderString = ph
    }

    func setQuery(_ q: String) {
        pendingQuery = q
        guard let field = searchField, field.stringValue != q else { return }
        suppressQueryEmit = true
        field.stringValue = q
        suppressQueryEmit = false
    }

    func setRows(_ newRows: [PaletteRow]) {
        let sig = Self.rowsSignature(newRows)
        if sig == rowsSig { return } // rows render identically: keep table, selection, focus
        rowsSig = sig
        rows = newRows
        tableView?.reloadData()
        // Fresh results: the top row is the highlighted default (Return drills in).
        if presented {
            highlight(rows.isEmpty ? -1 : 0)
            reassertFieldFocus()
        }
    }

    private static func rowsSignature(_ rows: [PaletteRow]) -> String {
        var s = ""
        for r in rows { s += "\(r.id)\u{1f}\(r.title)\u{1f}\(r.subtitle ?? "")\u{1f}\(r.iconName ?? "")\u{1e}" }
        return s
    }

    // Keep the search field first responder across a reload, but never steal
    // the field editor mid-edit (that would reselect the text) — only re-grab
    // when nothing is editing it.
    private func reassertFieldFocus() {
        guard let field = searchField, let window = field.window, field.currentEditor() == nil else { return }
        window.makeFirstResponder(field)
    }

    // ---- present / dismiss ----

    func applyOpen(_ open: Bool) {
        if open { presentPalette() } else { dismiss(programmatic: true) }
    }

    private func presentPalette() {
        if presented { return }
        // Present over the application's active window (the visible window/tab),
        // not merely the handle's own window, so the overlay covers whatever the
        // user is looking at regardless of where the handle sits in the tree.
        guard let window = NSApp.keyWindow ?? NSApp.mainWindow ?? self.window, let content = window.contentView else {
            // Not mounted/realized yet (create-time open): retry next turn.
            pendingOpen = true
            DispatchQueue.main.async { [weak self] in
                guard let self, self.pendingOpen else { return }
                self.pendingOpen = false
                self.presentPalette()
            }
            return
        }
        pendingOpen = false
        buildUI(in: content)
        presented = true
        highlight(rows.isEmpty ? -1 : 0)
        if let field = searchField { window.makeFirstResponder(field) }
    }

    func userCancel() { dismiss(programmatic: false) }

    private func dismiss(programmatic: Bool) {
        pendingOpen = false
        guard presented else { return }
        presented = false
        scrim?.removeFromSuperview()
        scrim = nil
        searchField = nil
        tableView = nil
        if !programmatic { ndEmitEvent(nodeID, "cancel", "{}") }
    }

    private func buildUI(in content: NSView) {
        let scrim = NDPaletteScrim(frame: content.bounds)
        scrim.handle = self
        scrim.autoresizingMask = [.width, .height]
        scrim.wantsLayer = true
        scrim.layer?.backgroundColor = NSColor.black.withAlphaComponent(0.28).cgColor
        scrim.setAccessibilityIdentifier("nd-command-palette-scrim")

        let card = NDPaletteCard()
        card.handle = self
        card.translatesAutoresizingMaskIntoConstraints = false
        card.material = .popover
        card.blendingMode = .withinWindow
        card.state = .active
        card.wantsLayer = true
        card.layer?.cornerRadius = ndConcentricRadius(in: card, fallback: NDRadius.palette)
        card.layer?.masksToBounds = true
        scrim.addSubview(card)

        let field = NSSearchField()
        field.translatesAutoresizingMaskIntoConstraints = false
        field.placeholderString = placeholder
        field.stringValue = pendingQuery
        field.sendsWholeSearchString = false
        field.delegate = self
        card.addSubview(field)

        let table = NSTableView()
        let column = NSTableColumn(identifier: paletteColumnID)
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)
        table.headerView = nil
        table.style = .inset
        table.rowHeight = 40
        table.backgroundColor = .clear
        table.dataSource = self
        table.delegate = self
        table.target = self
        table.action = #selector(rowClicked(_:))

        let scroll = NSScrollView()
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        scroll.documentView = table
        card.addSubview(scroll)

        let cardWidth = card.widthAnchor.constraint(equalToConstant: 600)
        cardWidth.priority = .defaultHigh
        NSLayoutConstraint.activate([
            card.centerXAnchor.constraint(equalTo: scrim.centerXAnchor),
            card.centerYAnchor.constraint(equalTo: scrim.centerYAnchor),
            cardWidth,
            card.widthAnchor.constraint(lessThanOrEqualTo: scrim.widthAnchor, constant: -40),
            card.heightAnchor.constraint(equalToConstant: 420),

            field.topAnchor.constraint(equalTo: card.topAnchor, constant: 12),
            field.leadingAnchor.constraint(equalTo: card.leadingAnchor, constant: 12),
            field.trailingAnchor.constraint(equalTo: card.trailingAnchor, constant: -12),

            scroll.topAnchor.constraint(equalTo: field.bottomAnchor, constant: 8),
            scroll.leadingAnchor.constraint(equalTo: card.leadingAnchor, constant: 8),
            scroll.trailingAnchor.constraint(equalTo: card.trailingAnchor, constant: -8),
            scroll.bottomAnchor.constraint(equalTo: card.bottomAnchor, constant: -8),
        ])

        content.addSubview(scrim)
        table.reloadData()
        self.scrim = scrim
        self.searchField = field
        self.tableView = table
    }

    // ---- highlight ----

    private func highlight(_ idx: Int) {
        guard let table = tableView else { return }
        if idx >= 0 && idx < rows.count {
            table.selectRowIndexes(IndexSet(integer: idx), byExtendingSelection: false)
            table.scrollRowToVisible(idx)
        } else {
            table.deselectAll(nil)
        }
    }

    private func moveHighlight(_ delta: Int) {
        guard !rows.isEmpty else { return }
        let cur = tableView?.selectedRow ?? -1
        let next = cur < 0 ? 0 : max(0, min(rows.count - 1, cur + delta))
        highlight(next)
    }

    // ---- emit ----

    private func emitActivate(_ idx: Int) {
        guard idx >= 0 && idx < rows.count else { return }
        ndEmitEvent(nodeID, "activate", "{\"text\":\(ndJsonString(rows[idx].id))}")
    }

    func emitSubmit() {
        ndEmitEvent(nodeID, "submit", "{\"text\":\(ndJsonString(searchField?.stringValue ?? pendingQuery))}")
    }

    // ---- automation ----
    // The tracked node is the host handle; the real field/table live in the
    // presented scrim. Automation routes setValue/type/click here (Automation
    // .swift) so a headless test drives the same paths a user would.

    var automationPresented: Bool { presented }
    var automationRowCount: Int { rows.count }

    func automationSetQuery(_ text: String) {
        searchField?.stringValue = text
        pendingQuery = text
        ndEmitEvent(nodeID, "queryChanged", "{\"text\":\(ndJsonString(text))}")
    }

    func automationAppendQuery(_ text: String) -> String {
        let full = (searchField?.stringValue ?? pendingQuery) + text
        searchField?.stringValue = full
        pendingQuery = full
        ndEmitEvent(nodeID, "queryChanged", "{\"text\":\(ndJsonString(full))}")
        return full
    }

    func automationActivateRow(_ idx: Int) -> Bool {
        guard idx >= 0 && idx < rows.count else { return false }
        highlight(idx)
        emitActivate(idx)
        return true
    }

    func automationClickHighlight() {
        let sel = tableView?.selectedRow ?? -1
        let idx = (sel >= 0 && sel < rows.count) ? sel : (rows.isEmpty ? -1 : 0)
        if idx >= 0 {
            highlight(idx)
            emitActivate(idx)
        }
    }

    func automationSubmit() { emitSubmit() }

    private func commitReturn() {
        let sel = tableView?.selectedRow ?? -1
        if sel >= 0 && sel < rows.count { emitActivate(sel) } else { emitSubmit() }
    }

    @objc func rowClicked(_ sender: NSTableView) {
        let r = sender.clickedRow
        guard r >= 0 && r < rows.count else { return }
        highlight(r)
        emitActivate(r)
    }

    // ---- NSSearchFieldDelegate ----

    func controlTextDidChange(_ obj: Notification) {
        guard !suppressQueryEmit, let field = obj.object as? NSSearchField else { return }
        pendingQuery = field.stringValue
        ndEmitEvent(nodeID, "queryChanged", "{\"text\":\(ndJsonString(field.stringValue))}")
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        switch selector {
        case #selector(NSResponder.moveUp(_:)): moveHighlight(-1); return true
        case #selector(NSResponder.moveDown(_:)): moveHighlight(1); return true
        case #selector(NSResponder.moveToBeginningOfDocument(_:)):
            if !rows.isEmpty { highlight(0) }
            return true
        case #selector(NSResponder.moveToEndOfDocument(_:)):
            if !rows.isEmpty { highlight(rows.count - 1) }
            return true
        case #selector(NSResponder.insertNewline(_:)): commitReturn(); return true
        case #selector(NSResponder.cancelOperation(_:)): userCancel(); return true
        default: return false
        }
    }

    // ---- NSTableViewDataSource / Delegate ----

    func numberOfRows(in tableView: NSTableView) -> Int { rows.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let cell = tableView.makeView(withIdentifier: paletteCellID, owner: self) as? NDPaletteCell ?? NDPaletteCell()
        cell.identifier = paletteCellID
        cell.configure(with: row < rows.count ? rows[row] : PaletteRow(id: "", title: "", subtitle: nil, iconName: nil))
        return cell
    }
}

/// Result cell: optional leading SF Symbol, a title, and an optional subtitle
/// (peer of the GTK backend's AdwActionRow title/subtitle/prefix-icon layout).
final class NDPaletteCell: NSTableCellView {
    private let iconView = NSImageView()
    private let titleField = NSTextField(labelWithString: "")
    private let subtitleField = NSTextField(labelWithString: "")
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
        subtitleField.translatesAutoresizingMaskIntoConstraints = false
        subtitleField.lineBreakMode = .byTruncatingTail
        subtitleField.textColor = .secondaryLabelColor
        subtitleField.font = .systemFont(ofSize: NSFont.smallSystemFontSize)

        addSubview(iconView)
        addSubview(titleField)
        addSubview(subtitleField)
        textField = titleField

        iconWidthConstraint = iconView.widthAnchor.constraint(equalToConstant: 18)
        NSLayoutConstraint.activate([
            iconView.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            iconView.centerYAnchor.constraint(equalTo: centerYAnchor),
            iconWidthConstraint,
            iconView.heightAnchor.constraint(equalToConstant: 18),

            titleField.leadingAnchor.constraint(equalTo: iconView.trailingAnchor, constant: 8),
            titleField.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            titleField.topAnchor.constraint(equalTo: topAnchor, constant: 5),

            subtitleField.leadingAnchor.constraint(equalTo: titleField.leadingAnchor),
            subtitleField.trailingAnchor.constraint(equalTo: titleField.trailingAnchor),
            subtitleField.topAnchor.constraint(equalTo: titleField.bottomAnchor, constant: 1),
        ])
    }

    func configure(with row: PaletteRow) {
        titleField.stringValue = row.title
        let sub = row.subtitle ?? ""
        subtitleField.stringValue = sub
        subtitleField.isHidden = sub.isEmpty
        if let iconName = row.iconName {
            let symbol = ndSFSymbol(forFreedesktop: iconName) ?? iconName // NDShell/Icons.swift
            iconView.image = NSImage(systemSymbolName: symbol, accessibilityDescription: row.title)
        } else {
            iconView.image = nil
        }
        iconView.isHidden = iconView.image == nil
        iconWidthConstraint.constant = iconView.isHidden ? 0 : 18
    }
}

// ---- generated-dispatch bridges (NDGen/Widgets.swift arms call these) -------

func makeCommandPalette(_ props: [String: Any]) -> NSView {
    let handle = NDCommandPaletteHandleView()
    if let ph = propStr(props, "placeholder") { handle.setPlaceholder(ph) }
    if let q = propStr(props, "query") { handle.setQuery(q) }
    if let raw = propObjArray(props, "items") { handle.setRows(raw.map(paletteRow(from:))) }
    if propBool(props, "open") ?? false { handle.applyOpen(true) } // no window yet: pending
    return handle
}

func ndCommandPaletteApplyPlaceholder(_ view: NSView, _ placeholder: String) {
    (view as? NDCommandPaletteHandleView)?.setPlaceholder(placeholder)
}

func ndCommandPaletteApplyQuery(_ view: NSView, _ query: String) {
    (view as? NDCommandPaletteHandleView)?.setQuery(query)
}

func ndCommandPaletteApplyItems(_ view: NSView, _ raw: [[String: Any]]) {
    (view as? NDCommandPaletteHandleView)?.setRows(raw.map(paletteRow(from:)))
}

func ndCommandPaletteApplyOpen(_ view: NSView, _ open: Bool) {
    (view as? NDCommandPaletteHandleView)?.applyOpen(open)
}

func ndCommandPaletteConnect(_ view: NSView, nodeID: UInt32) {
    (view as? NDCommandPaletteHandleView)?.nodeID = nodeID
}
