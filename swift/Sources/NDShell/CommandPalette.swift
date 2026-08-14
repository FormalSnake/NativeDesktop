import AppKit

/// CommandPalette: a centered, modal Cmd-K overlay (peer of
/// src/gtk/commandpalette.zig's AdwDialog). The tracked handle is a host-only
/// NSView (the Popover idiom) that lives in the tree only so `self.window`
/// resolves the window to paint over; `open` toggles a transparent full-window
/// backdrop plus a centered Liquid Glass card (a borderless search field over
/// an NSTableView of results) whose height follows its row count.
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
private let paletteRowID = NSUserInterfaceItemIdentifier("nd-command-palette-row")

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

/// Full-window hit target behind the card; a click on the bare backdrop
/// cancels. Transparent: macOS does not dim behind an overlay panel, so this
/// view exists for the outside-click contract, not as a scrim. Owns the
/// Cmd/Ctrl+Return "submit as-is" key equivalent (a modifier-Return never
/// reaches the field editor's doCommandBySelector).
private final class NDPaletteBackdrop: NSView {
    weak var handle: NDCommandPaletteHandleView?
    override var isFlipped: Bool { true }
    override func mouseDown(with event: NSEvent) { handle?.userCancel() }
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

/// Shadow host for the glass card. NSGlassEffectView clips to its own corner
/// radius, so the drop shadow has to live one level out; the path is rebuilt
/// every layout pass because the card's height follows its row count. Also
/// stops a click on the card's own chrome from reaching the backdrop's cancel.
private final class NDPaletteCard: NSView {
    override func layout() {
        super.layout()
        layer?.shadowPath = CGPath(
            roundedRect: bounds,
            cornerWidth: NDRadius.palette,
            cornerHeight: NDRadius.palette,
            transform: nil)
    }
    override func mouseDown(with event: NSEvent) {}
}

/// Keyboard focus legitimately stays in the search field, which leaves the
/// table unemphasized and its selection grey. The highlighted row is the
/// palette's primary affordance, so force the emphasized rendering rather than
/// hand-painting a fill: AppKit keeps its own accent colour, inset and
/// curvature for the table's style.
private final class NDPaletteRowView: NSTableRowView {
    override var isEmphasized: Bool {
        get { true }
        set {}
    }
}

final class NDCommandPaletteHandleView: NSView, NSTextFieldDelegate, NSTableViewDataSource, NSTableViewDelegate {
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

    private var backdrop: NDPaletteBackdrop?
    private var searchField: NSTextField?
    private var tableView: NSTableView?
    private var separator: NSBox?
    private var listHeight: NSLayoutConstraint?

    override init(frame: NSRect) {
        super.init(frame: frame)
        isHidden = true // host-only handle: never takes layout space
    }
    required init?(coder: NSCoder) {
        super.init(coder: coder)
        isHidden = true
    }
    override var intrinsicContentSize: NSSize { .zero }

    // The backdrop and card are subviews of the window's contentView, not of
    // this handle, so an unmount-while-open would strand them: tear down when
    // the handle leaves its window.
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
        updateListHeight()
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
            // Not mounted/realized yet (create-time open): retry on a short
            // tick until an anchor window exists. A single same-turn retry
            // lost the race whenever the handle took more than one main-queue
            // turn to land in a window, leaving the panel unpresented for
            // good; the spin variant also saturated the main queue while
            // unanchored. dismiss() clears pendingOpen, which stops the
            // chain; a deallocated handle stops it via the weak self.
            pendingOpen = true
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { [weak self] in
                guard let self, self.pendingOpen else { return }
                self.pendingOpen = false
                self.presentPalette()
            }
            return
        }
        pendingOpen = false
        // A background-spawned host is inactive: no key window, and the field
        // never receives focus. Automation runs opt into activation so the
        // palette anchors and types like it would for a user; real apps open
        // palettes from user input, when the app is already active.
        if !NSApp.isActive, ProcessInfo.processInfo.environment["NATIVE_AUTOMATION"] == "1" {
            NSApp.activate(ignoringOtherApps: true)
        }
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
        backdrop?.removeFromSuperview()
        backdrop = nil
        searchField = nil
        tableView = nil
        separator = nil
        listHeight = nil
        if !programmatic { ndEmitEvent(nodeID, "cancel", "{}") }
    }

    private func buildUI(in content: NSView) {
        let backdrop = NDPaletteBackdrop(frame: content.bounds)
        backdrop.handle = self
        backdrop.autoresizingMask = [.width, .height]
        backdrop.setAccessibilityIdentifier("nd-command-palette-backdrop")

        let card = NDPaletteCard()
        card.translatesAutoresizingMaskIntoConstraints = false
        card.wantsLayer = true
        card.layer?.shadowColor = NSColor.black.cgColor
        card.layer?.shadowOpacity = 0.32
        card.layer?.shadowRadius = 28
        // Undirected: a layer's y axis follows its view's flippedness, so an
        // offset shadow would fall the wrong way in one of the two geometries.
        card.layer?.shadowOffset = .zero
        backdrop.addSubview(card)

        let glass = NSGlassEffectView()
        glass.translatesAutoresizingMaskIntoConstraints = false
        glass.cornerRadius = NDRadius.palette
        card.addSubview(glass)

        // The chrome hangs off `body` but is constrained against `card`, whose
        // height therefore falls out of this content. `body` is pinned rather
        // than left to NSGlassEffectView's own contentView layout so the glass
        // has a content rect the moment the card is measured.
        let body = NSView()
        body.translatesAutoresizingMaskIntoConstraints = false
        glass.contentView = body

        let icon = NSImageView()
        icon.translatesAutoresizingMaskIntoConstraints = false
        icon.image = NSImage(systemSymbolName: "magnifyingglass", accessibilityDescription: nil)
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 18, weight: .regular)
        icon.contentTintColor = .secondaryLabelColor
        body.addSubview(icon)

        let field = NSTextField()
        field.translatesAutoresizingMaskIntoConstraints = false
        field.isBezeled = false
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = .systemFont(ofSize: 20)
        field.placeholderString = placeholder
        field.stringValue = pendingQuery
        field.delegate = self
        body.addSubview(field)

        let separator = NSBox()
        separator.translatesAutoresizingMaskIntoConstraints = false
        separator.boxType = .separator
        body.addSubview(separator)

        let table = NSTableView()
        let column = NSTableColumn(identifier: paletteColumnID)
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)
        table.headerView = nil
        table.style = .inset
        table.rowHeight = NDPaletteMetrics.rowHeight
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
        body.addSubview(scroll)

        let cardWidth = card.widthAnchor.constraint(equalToConstant: NDPaletteMetrics.width)
        cardWidth.priority = .defaultHigh
        // Content-driven list height, broken by the required cap below it once
        // there are more rows than the card may show.
        let listHeight = scroll.heightAnchor.constraint(equalToConstant: 0)
        listHeight.priority = .defaultHigh
        NSLayoutConstraint.activate([
            card.centerXAnchor.constraint(equalTo: backdrop.centerXAnchor),
            card.centerYAnchor.constraint(equalTo: backdrop.centerYAnchor),
            cardWidth,
            card.widthAnchor.constraint(lessThanOrEqualTo: backdrop.widthAnchor, constant: -40),
            card.heightAnchor.constraint(lessThanOrEqualTo: backdrop.heightAnchor, constant: -80),

            glass.leadingAnchor.constraint(equalTo: card.leadingAnchor),
            glass.trailingAnchor.constraint(equalTo: card.trailingAnchor),
            glass.topAnchor.constraint(equalTo: card.topAnchor),
            glass.bottomAnchor.constraint(equalTo: card.bottomAnchor),

            body.leadingAnchor.constraint(equalTo: card.leadingAnchor),
            body.trailingAnchor.constraint(equalTo: card.trailingAnchor),
            body.topAnchor.constraint(equalTo: card.topAnchor),
            body.bottomAnchor.constraint(equalTo: card.bottomAnchor),

            icon.leadingAnchor.constraint(equalTo: card.leadingAnchor, constant: 18),
            icon.centerYAnchor.constraint(equalTo: field.centerYAnchor),
            icon.widthAnchor.constraint(equalToConstant: 20),
            icon.heightAnchor.constraint(equalToConstant: 20),

            field.topAnchor.constraint(equalTo: card.topAnchor, constant: 16),
            field.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 10),
            field.trailingAnchor.constraint(equalTo: card.trailingAnchor, constant: -18),

            separator.topAnchor.constraint(equalTo: field.bottomAnchor, constant: 14),
            separator.leadingAnchor.constraint(equalTo: card.leadingAnchor),
            separator.trailingAnchor.constraint(equalTo: card.trailingAnchor),
            separator.heightAnchor.constraint(equalToConstant: 1),

            scroll.topAnchor.constraint(equalTo: separator.bottomAnchor),
            scroll.leadingAnchor.constraint(equalTo: card.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: card.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: card.bottomAnchor, constant: -6),
            scroll.heightAnchor.constraint(lessThanOrEqualToConstant: NDPaletteMetrics.maxListHeight),
            listHeight,
        ])

        content.addSubview(backdrop)
        table.reloadData()
        self.backdrop = backdrop
        self.searchField = field
        self.tableView = table
        self.separator = separator
        self.listHeight = listHeight
        updateListHeight()
    }

    /// Height of the rendered rows, read back from the table so the row
    /// metrics the style applies (intercell spacing, inset margins) are the
    /// ones the card is sized against. An empty result set collapses the list
    /// and its separator, leaving the search field alone on the card.
    private func updateListHeight() {
        separator?.isHidden = rows.isEmpty
        guard let listHeight else { return }
        guard let table = tableView, !rows.isEmpty else {
            listHeight.constant = 0
            return
        }
        listHeight.constant = table.rect(ofRow: rows.count - 1).maxY
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
    // presented backdrop. Automation routes setValue/type/click here (Automation
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

    // ---- NSTextFieldDelegate ----

    func controlTextDidChange(_ obj: Notification) {
        guard !suppressQueryEmit, let field = obj.object as? NSTextField else { return }
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

    func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        let rowView = tableView.makeView(withIdentifier: paletteRowID, owner: self) as? NDPaletteRowView ?? NDPaletteRowView()
        rowView.identifier = paletteRowID
        rowView.backgroundColor = .clear // the card is glass: no opaque row plate over it
        return rowView
    }

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

    // NSTableCellView recolours `textField` over an emphasized selection fill
    // but knows nothing about the subtitle or the symbol, which would keep
    // their unselected colours against the accent.
    override var backgroundStyle: NSView.BackgroundStyle {
        didSet {
            let onFill = backgroundStyle == .emphasized
            subtitleField.textColor = onFill
                ? NSColor.alternateSelectedControlTextColor.withAlphaComponent(0.8)
                : .secondaryLabelColor
            iconView.contentTintColor = onFill ? .alternateSelectedControlTextColor : nil
        }
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
