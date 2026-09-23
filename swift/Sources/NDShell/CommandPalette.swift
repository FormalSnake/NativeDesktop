import AppKit

/// CommandPalette: a floating command bar over the window (peer of
/// src/gtk/commandpalette.zig's AdwDialog). The tracked handle is a host-only
/// NSView (the Popover idiom) that lives in the tree only so `self.window`
/// resolves the window to paint over; `open` toggles a dimmed full-window
/// backdrop plus a Liquid Glass card (a borderless search field over an
/// NSTableView of one-line results) whose top edge is fixed and whose height
/// follows its row count.
///
/// CONTROLLED: the app owns `query` and `items`; the widget never filters or
/// reorders. Every keystroke fires queryChanged with the text the user typed;
/// the app feeds back the next result set. A present always starts from the
/// last `query` the app set, never from what was typed into an earlier
/// present. Highlight is internal (Up/Down/Home/End clamp within the current
/// rows). onActivate carries the highlighted/clicked row's id; onSubmit the
/// typed text (plain Return with no highlight, or Cmd/Ctrl Return regardless)
/// so a directory picker can accept a typed path that matches no listed row.
/// onCancel fires on Esc / click-outside; a React-driven close (open=false)
/// is flagged so it does not echo.
///
/// Inline autocompletion: the first row's `completion`, when it extends what
/// was typed and the last edit inserted text, is shown selected after the
/// caret. It never reaches queryChanged until Tab or Right accepts it.
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
    var iconData: String?
    var hint: String?
    var completion: String?
}

private func paletteRow(from obj: [String: Any]) -> PaletteRow {
    PaletteRow(
        id: obj["id"] as? String ?? "",
        title: obj["title"] as? String ?? "",
        subtitle: obj["subtitle"] as? String,
        iconName: obj["iconName"] as? String,
        iconData: obj["iconData"] as? String,
        hint: obj["hint"] as? String,
        completion: obj["completion"] as? String
    )
}

/// Full-window scrim behind the card; a click on it cancels. Owns the
/// Cmd/Ctrl+Return "submit as typed" key equivalent (a modifier-Return never
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
    /// The app's last `query`. Every present starts from it.
    private var controlledQuery = ""
    /// What the user typed, without any completion shown after it.
    private var typed = ""
    /// The completion currently drawn selected after `typed`.
    private var suffix = ""
    /// Set by an edit that inserted text, cleared by one that removed text:
    /// Backspace over a completion must not bring it straight back.
    private var mayComplete = false
    // Content fingerprint of the rendered rows. A controlled app hands back a
    // fresh `items` array on every render; without this the table would reload
    // and reset the highlight on each one, so reload only when rows change.
    private var rowsSig = ""

    private var pendingOpen = false
    private var presented = false
    private var suppressQueryEmit = false
    private weak var returnFocus: NSResponder?

    private var backdrop: NDPaletteBackdrop?
    private var card: NSView?
    private var searchField: NSTextField?
    private var tableView: NSTableView?
    private var scrollView: NSScrollView?
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
        searchField?.setAccessibilityLabel(ph)
    }

    func setQuery(_ q: String) {
        controlledQuery = q
        guard presented, let field = searchField else { return }
        typed = q
        suffix = ""
        mayComplete = false
        if field.stringValue != q {
            suppressQueryEmit = true
            field.stringValue = q
            suppressQueryEmit = false
        }
        // A reseed while open reads like a fresh open: the whole text selected.
        field.currentEditor()?.selectAll(nil)
    }

    func setRows(_ newRows: [PaletteRow]) {
        let sig = Self.rowsSignature(newRows)
        if sig != rowsSig {
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
        if presented { applyCompletion() }
    }

    private static func rowsSignature(_ rows: [PaletteRow]) -> String {
        var s = ""
        for r in rows {
            for part in [r.id, r.title, r.subtitle ?? "", r.iconName ?? "", r.iconData ?? "", r.hint ?? "", r.completion ?? ""] {
                s += part
                s += "\u{1f}"
            }
            s += "\u{1e}"
        }
        return s
    }

    // Keep the search field first responder across a reload, but never steal
    // the field editor mid-edit (that would reselect the text); only re-grab
    // when nothing is editing it.
    private func reassertFieldFocus() {
        guard let field = searchField, let window = field.window, field.currentEditor() == nil else { return }
        window.makeFirstResponder(field)
    }

    // ---- inline completion ----

    /// Draws the first row's completion after the typed text, or takes a stale
    /// one away. Runs after every edit (against the rows already shown, so the
    /// completion keeps up with fast typing) and again when new rows land.
    private func applyCompletion() {
        guard let field = searchField, let editor = field.currentEditor() as? NSTextView else { return }
        var next = ""
        if mayComplete, let full = rows.first?.completion {
            let typed16 = (typed as NSString).length
            let full16 = (full as NSString).length
            if typed16 > 0, full16 > typed16,
               (full as NSString).substring(to: typed16).caseInsensitiveCompare(typed) == .orderedSame {
                next = (full as NSString).substring(from: typed16)
            }
        }
        if next == suffix && field.stringValue == typed + suffix { return }
        suffix = next
        suppressQueryEmit = true
        field.stringValue = typed + suffix
        suppressQueryEmit = false
        let start = (typed as NSString).length
        editor.selectedRange = NSRange(location: start, length: (suffix as NSString).length)
    }

    /// Tab or Right over a completion: it becomes typed text.
    private func acceptCompletion() -> Bool {
        guard !suffix.isEmpty, let field = searchField else { return false }
        typed += suffix
        suffix = ""
        mayComplete = false
        field.currentEditor()?.selectedRange = NSRange(location: (typed as NSString).length, length: 0)
        ndEmitEvent(nodeID, "queryChanged", "{\"text\":\(ndJsonString(typed))}")
        return true
    }

    /// One path for every change of the typed text, from a keystroke or from
    /// automation, so both see the same completion rules.
    private func userEdited(_ text: String) {
        mayComplete = (text as NSString).length > (typed as NSString).length
        typed = text
        suffix = ""
        ndEmitEvent(nodeID, "queryChanged", "{\"text\":\(ndJsonString(text))}")
        applyCompletion()
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
        returnFocus = window.firstResponder
        typed = controlledQuery
        suffix = ""
        mayComplete = false
        buildUI(in: content)
        presented = true
        highlight(rows.isEmpty ? -1 : 0)
        // Becoming first responder selects the field's whole text, which is
        // what a seeded open (the current address) wants.
        if let field = searchField { window.makeFirstResponder(field) }
    }

    func userCancel() { dismiss(programmatic: false) }

    private func dismiss(programmatic: Bool) {
        pendingOpen = false
        guard presented else { return }
        presented = false
        let window = backdrop?.window
        backdrop?.removeFromSuperview()
        backdrop = nil
        card = nil
        searchField = nil
        tableView = nil
        scrollView = nil
        separator = nil
        listHeight = nil
        // Focus goes back where it was (the page, usually); a responder that
        // left the window meanwhile leaves it with the window.
        if let window, let back = returnFocus, (back as? NSView)?.window === window || back === window {
            window.makeFirstResponder(back)
        }
        returnFocus = nil
        if !programmatic { ndEmitEvent(nodeID, "cancel", "{}") }
    }

    private func buildUI(in content: NSView) {
        let backdrop = NDPaletteBackdrop(frame: content.bounds)
        backdrop.handle = self
        backdrop.autoresizingMask = [.width, .height]
        backdrop.wantsLayer = true
        backdrop.layer?.backgroundColor = NSColor.black.withAlphaComponent(NDPaletteMetrics.dimAlpha).cgColor
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
        card.setAccessibilityElement(true)
        card.setAccessibilityRole(.group)
        card.setAccessibilityLabel(placeholder ?? "Command bar")
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
        field.cell?.isScrollable = true
        field.cell?.wraps = false
        field.cell?.usesSingleLineMode = true
        field.lineBreakMode = .byClipping
        field.placeholderString = placeholder
        field.setAccessibilityLabel(placeholder)
        field.stringValue = controlledQuery
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
        table.intercellSpacing = NSSize(width: 0, height: 0)
        table.backgroundColor = .clear
        table.dataSource = self
        table.delegate = self
        table.target = self
        table.action = #selector(rowClicked(_:))
        table.setAccessibilityLabel("Results")

        let scroll = NSScrollView()
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = false
        scroll.documentView = table
        body.addSubview(scroll)

        let m = NDPaletteMetrics.margin
        let cardWidth = card.widthAnchor.constraint(equalToConstant: NDPaletteMetrics.width)
        cardWidth.priority = .defaultHigh
        // The top edge rides a fixed fraction of the window height, so the field
        // does not move while results change. The backdrop is flipped, which
        // makes its `.bottom` its height.
        let top = NSLayoutConstraint(item: card, attribute: .top, relatedBy: .equal,
                                     toItem: backdrop, attribute: .bottom,
                                     multiplier: NDPaletteMetrics.topFraction, constant: 0)
        top.priority = .defaultHigh
        // Content-driven list height, broken by the required cap below it once
        // there are more rows than the card may show.
        let listHeight = scroll.heightAnchor.constraint(equalToConstant: 0)
        listHeight.priority = .defaultHigh
        NSLayoutConstraint.activate([
            card.centerXAnchor.constraint(equalTo: backdrop.centerXAnchor),
            top,
            card.topAnchor.constraint(greaterThanOrEqualTo: backdrop.topAnchor, constant: m),
            cardWidth,
            card.widthAnchor.constraint(lessThanOrEqualTo: backdrop.widthAnchor, constant: -2 * m),
            card.bottomAnchor.constraint(lessThanOrEqualTo: backdrop.bottomAnchor, constant: -m),

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

            scroll.topAnchor.constraint(equalTo: separator.bottomAnchor, constant: 6),
            scroll.leadingAnchor.constraint(equalTo: card.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: card.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: card.bottomAnchor, constant: -6),
            scroll.heightAnchor.constraint(lessThanOrEqualToConstant: NDPaletteMetrics.maxListHeight),
            listHeight,
        ])

        content.addSubview(backdrop)
        table.reloadData()
        self.backdrop = backdrop
        self.card = card
        self.searchField = field
        self.tableView = table
        self.scrollView = scroll
        self.separator = separator
        self.listHeight = listHeight
        updateListHeight()
    }

    /// Height of the rendered rows, read back from the table so the row
    /// metrics the style applies (inset margins) are the ones the card is
    /// sized against. An empty result set collapses the list and its
    /// separator, leaving the search field alone on the card.
    private func updateListHeight() {
        separator?.isHidden = rows.isEmpty
        scrollView?.isHidden = rows.isEmpty
        guard let listHeight else { return }
        guard let table = tableView, !rows.isEmpty else {
            listHeight.constant = -6 // cancels the list's top gap: the field alone, evenly padded
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

    /// The typed text, never an unaccepted completion.
    func emitSubmit() {
        ndEmitEvent(nodeID, "submit", "{\"text\":\(ndJsonString(presented ? typed : controlledQuery))}")
    }

    // ---- automation ----
    // The tracked node is the host handle; the real field/table live in the
    // presented backdrop. Automation routes setValue/type/click here (Automation
    // .swift) so a headless test drives the same paths a user would.

    var automationPresented: Bool { presented }
    var automationRowCount: Int { rows.count }

    func automationSetQuery(_ text: String) {
        searchField?.stringValue = text
        userEdited(text)
    }

    func automationAppendQuery(_ text: String) -> String {
        let full = typed + text
        searchField?.stringValue = full
        userEdited(full)
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

    /// `paletteLayout`: the drawn geometry, relative to the backdrop (the
    /// window's content area), as the JSON the RPC answers.
    func automationLayout() -> String {
        var out: [String: Any] = [
            "ref": Int(nodeID), "presented": presented, "rows": [[String: Any]](),
            "fieldText": "", "selectionStart": 0, "selectionLength": 0, "dimmed": false,
        ]
        guard presented, let backdrop, let card, let field = searchField, let table = tableView else {
            return ndJSONObjectString(out)
        }
        backdrop.layoutSubtreeIfNeeded()
        // Alignment rects: what Auto Layout placed, which for a symbol image
        // excludes the padding its glyph carries outside the layout box.
        func rect(_ v: NSView) -> [String: Int] {
            let local = v.superview.map { _ in v.alignmentRect(forFrame: v.frame) } ?? v.frame
            let r = (v.superview ?? v).convert(local, to: backdrop)
            return ["x": Int(r.origin.x.rounded()), "y": Int(r.origin.y.rounded()),
                    "w": Int(r.width.rounded()), "h": Int(r.height.rounded())]
        }
        out["window"] = rect(backdrop)
        out["panel"] = rect(card)
        out["field"] = rect(field)
        out["fieldText"] = field.stringValue
        let sel = field.currentEditor()?.selectedRange ?? NSRange(location: 0, length: 0)
        out["selectionStart"] = sel.location
        out["selectionLength"] = sel.length
        out["dimmed"] = (backdrop.layer?.backgroundColor?.alpha ?? 0) > 0
        var drawn: [[String: Any]] = []
        // Rows the list shows whole; one half scrolled out is clipped by it.
        let visibleRect = table.visibleRect
        let visible = table.rows(in: visibleRect)
        for i in visible.location..<(visible.location + visible.length) {
            guard visibleRect.contains(table.rect(ofRow: i)) else { continue }
            guard let rowView = table.rowView(atRow: i, makeIfNecessary: false),
                  let cell = table.view(atColumn: 0, row: i, makeIfNecessary: false) as? NDPaletteCell else { continue }
            var entry: [String: Any] = ["row": rect(rowView), "highlighted": table.selectedRow == i,
                                        "truncated": cell.isTruncated]
            entry["icon"] = cell.iconShown ? rect(cell.iconSlot) : NSNull()
            entry["title"] = rect(cell.titleSlot)
            entry["subtitle"] = cell.subtitleShown ? rect(cell.subtitleSlot) : NSNull()
            entry["hint"] = cell.hintShown ? rect(cell.hintSlot) : NSNull()
            drawn.append(entry)
        }
        out["rows"] = drawn
        return ndJSONObjectString(out)
    }

    /// A first row carrying a completion stands for the completed text. Once
    /// Backspace has taken the completion away, Enter means what was typed.
    private func commitReturn() {
        let sel = tableView?.selectedRow ?? -1
        if sel == 0, suffix.isEmpty, let full = rows.first?.completion, !full.isEmpty,
           full.caseInsensitiveCompare(typed) != .orderedSame {
            emitSubmit()
            return
        }
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
        userEdited(field.stringValue)
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
        // Tab never leaves the field: it accepts a completion or does nothing.
        case #selector(NSResponder.insertTab(_:)), #selector(NSResponder.insertBacktab(_:)):
            _ = acceptCompletion()
            return true
        case #selector(NSResponder.moveRight(_:)), #selector(NSResponder.moveToEndOfLine(_:)):
            return acceptCompletion()
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
        cell.configure(with: row < rows.count ? rows[row] : PaletteRow(id: "", title: ""))
        return cell
    }
}

/// Result cell, one line: a 16 pt icon, the title, the subtitle in secondary
/// text after it, and the hint pinned to the trailing edge. The subtitle gives
/// way first, then the title; the hint never truncates.
final class NDPaletteCell: NSTableCellView {
    private let iconView = NSImageView()
    private let titleField = NSTextField(labelWithString: "")
    private let subtitleField = NSTextField(labelWithString: "")
    private let hintField = NSTextField(labelWithString: "")
    private var subtitleGap: NSLayoutConstraint!
    private var hintGap: NSLayoutConstraint!

    var iconSlot: NSView { iconView }
    var titleSlot: NSView { titleField }
    var subtitleSlot: NSView { subtitleField }
    var hintSlot: NSView { hintField }
    var iconShown: Bool { !iconView.isHidden }
    var subtitleShown: Bool { !subtitleField.isHidden }
    var hintShown: Bool { !hintField.isHidden }
    var isTruncated: Bool {
        layoutSubtreeIfNeeded()
        func cut(_ f: NSTextField) -> Bool {
            !f.isHidden && f.attributedStringValue.size().width > f.frame.width + 0.5
        }
        return cut(titleField) || cut(subtitleField)
    }

    // NSTableCellView recolours `textField` over an emphasized selection fill
    // but knows nothing about the subtitle, the hint or the symbol, which would
    // keep their unselected colours against the accent.
    override var backgroundStyle: NSView.BackgroundStyle {
        didSet {
            let onFill = backgroundStyle == .emphasized
            let secondary = onFill
                ? NSColor.alternateSelectedControlTextColor.withAlphaComponent(0.8)
                : NSColor.secondaryLabelColor
            subtitleField.textColor = secondary
            hintField.textColor = secondary
            iconView.contentTintColor = onFill ? .alternateSelectedControlTextColor : .secondaryLabelColor
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
        let font = NSFont.systemFont(ofSize: NSFont.systemFontSize)
        iconView.translatesAutoresizingMaskIntoConstraints = false
        iconView.imageScaling = .scaleProportionallyUpOrDown
        iconView.contentTintColor = .secondaryLabelColor
        for f in [titleField, subtitleField, hintField] {
            f.translatesAutoresizingMaskIntoConstraints = false
            f.font = font
            f.lineBreakMode = .byTruncatingTail
            f.maximumNumberOfLines = 1
            f.cell?.usesSingleLineMode = true
        }
        subtitleField.textColor = .secondaryLabelColor
        hintField.textColor = .secondaryLabelColor
        hintField.alignment = .right

        titleField.setContentCompressionResistancePriority(.init(740), for: .horizontal)
        titleField.setContentHuggingPriority(.init(760), for: .horizontal)
        subtitleField.setContentCompressionResistancePriority(.init(730), for: .horizontal)
        subtitleField.setContentHuggingPriority(.init(250), for: .horizontal)
        hintField.setContentCompressionResistancePriority(.init(760), for: .horizontal)
        hintField.setContentHuggingPriority(.init(760), for: .horizontal)

        addSubview(iconView)
        addSubview(titleField)
        addSubview(subtitleField)
        addSubview(hintField)
        textField = titleField

        let side = NDPaletteMetrics.iconSide
        subtitleGap = subtitleField.leadingAnchor.constraint(equalTo: titleField.trailingAnchor, constant: 8)
        hintGap = hintField.leadingAnchor.constraint(greaterThanOrEqualTo: subtitleField.trailingAnchor, constant: 16)
        // The title may not take the whole row: a long page title still leaves
        // room to see which site it is on.
        let titleCap = titleField.widthAnchor.constraint(lessThanOrEqualTo: widthAnchor, multiplier: 0.62)
        NSLayoutConstraint.activate([
            iconView.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            iconView.centerYAnchor.constraint(equalTo: centerYAnchor),
            iconView.widthAnchor.constraint(equalToConstant: side),
            iconView.heightAnchor.constraint(equalToConstant: side),

            titleField.leadingAnchor.constraint(equalTo: iconView.trailingAnchor, constant: 10),
            titleField.centerYAnchor.constraint(equalTo: centerYAnchor),
            titleCap,

            subtitleGap,
            subtitleField.firstBaselineAnchor.constraint(equalTo: titleField.firstBaselineAnchor),

            hintGap,
            hintField.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            hintField.firstBaselineAnchor.constraint(equalTo: titleField.firstBaselineAnchor),
        ])
    }

    func configure(with row: PaletteRow) {
        titleField.stringValue = row.title
        let sub = row.subtitle ?? ""
        subtitleField.stringValue = sub
        subtitleField.isHidden = sub.isEmpty
        subtitleGap.constant = sub.isEmpty ? 0 : 8
        let hint = row.hint ?? ""
        hintField.stringValue = hint
        hintField.isHidden = hint.isEmpty

        var image: NSImage?
        if let data = row.iconData, !data.isEmpty {
            image = ndIconImageFromData(data, side: NDPaletteMetrics.iconSide, what: "CommandPalette")
        }
        if image == nil, let iconName = row.iconName, !iconName.isEmpty {
            let symbol = ndSFSymbol(forFreedesktop: iconName) ?? iconName // NDShell/Icons.swift
            image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)?
                .withSymbolConfiguration(.init(pointSize: 13, weight: .regular))
        }
        iconView.image = image
        // An empty slot still holds the column, so titles line up whether or
        // not a row has an icon.
        iconView.isHidden = image == nil
        iconView.imageScaling = (row.iconData?.isEmpty == false && image != nil)
            ? .scaleProportionallyUpOrDown : .scaleNone

        let parts = [row.title, sub, hint].filter { !$0.isEmpty }
        setAccessibilityLabel(parts.joined(separator: ", "))
    }
}

private func ndJSONObjectString(_ obj: [String: Any]) -> String {
    guard let data = try? JSONSerialization.data(withJSONObject: obj),
          let s = String(data: data, encoding: .utf8) else { return "{}" }
    return s
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
