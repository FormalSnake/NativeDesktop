import AppKit

/// CommandPalette: a floating command bar over the window (peer of
/// src/gtk/commandpalette.zig's AdwDialog). The tracked handle is a host-only
/// NSView (the Popover idiom) that lives in the tree only so `self.window`
/// resolves the window to paint over; `open` toggles a scrim over the whole
/// window and two quiet cards on it: the field, and below it the one-line
/// results (an NSTableView), whose card follows its row count. The field's top
/// edge is fixed.
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

/// The scrim: the window's ground colour over everything, toolbar and sidebar
/// included, so the bar is the only thing asking to be read. A click on it
/// cancels. Owns the Cmd/Ctrl+Return "submit as typed" key equivalent (a
/// modifier-Return never reaches the field editor's doCommandBySelector).
private final class NDPaletteBackdrop: NSView {
    weak var handle: NDCommandPaletteHandleView?
    override var isFlipped: Bool { true }
    override func mouseDown(with event: NSEvent) { handle?.userCancel() }
    // A backdrop fading out has been let go of; clicks go through it.
    override func hitTest(_ point: NSPoint) -> NSView? { handle == nil ? nil : super.hitTest(point) }
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        if event.type == .keyDown,
           event.modifierFlags.contains(.command) || event.modifierFlags.contains(.control),
           event.keyCode == 36 || event.keyCode == 76 { // Return / keypad Enter
            handle?.emitSubmit(typedOnly: true)
            return true
        }
        return super.performKeyEquivalent(with: event)
    }
}

/// One of the bar's two cards: ground fill, hairline edge, a soft shadow.
/// Colours are resolved in `updateLayer`, which AppKit calls again when the
/// window's appearance changes. Also stops a click on the card from reaching
/// the scrim's cancel.
private final class NDPaletteSurface: NSView {
    private let shadowAlpha: Float
    private let shadowBlur: CGFloat
    private let shadowDrop: CGFloat

    init(shadowAlpha: Float, shadowBlur: CGFloat, shadowDrop: CGFloat) {
        self.shadowAlpha = shadowAlpha
        self.shadowBlur = shadowBlur
        self.shadowDrop = shadowDrop
        super.init(frame: .zero)
        wantsLayer = true
        translatesAutoresizingMaskIntoConstraints = false
    }
    required init?(coder: NSCoder) { fatalError("not used") }

    override var wantsUpdateLayer: Bool { true }
    override func updateLayer() {
        guard let layer else { return }
        effectiveAppearance.performAsCurrentDrawingAppearance {
            layer.backgroundColor = NDPaletteColors.ground.cgColor
            layer.borderColor = NDPaletteColors.hairline.cgColor
        }
        layer.borderWidth = 1
        layer.cornerRadius = NDPaletteMetrics.cardRadius
        layer.cornerCurve = .continuous
        layer.shadowColor = NSColor.black.cgColor
        layer.shadowOpacity = shadowAlpha
        layer.shadowRadius = shadowBlur / 2
        // An unflipped layer's y points up, so a drop below the card is negative.
        layer.shadowOffset = CGSize(width: 0, height: -shadowDrop)
    }
    override func layout() {
        super.layout()
        layer?.shadowPath = CGPath(roundedRect: bounds, cornerWidth: NDPaletteMetrics.cardRadius,
                                   cornerHeight: NDPaletteMetrics.cardRadius, transform: nil)
    }
    override func mouseDown(with event: NSEvent) {}
}

/// The highlighted row is a wash of grey with rounded corners, never the
/// accent: keyboard focus stays in the field, and the row only marks where
/// the arrow keys have walked to.
private final class NDPaletteRowView: NSTableRowView {
    override var isEmphasized: Bool {
        get { false }
        set {}
    }
    override func drawSelection(in dirtyRect: NSRect) {
        guard isSelected else { return }
        effectiveAppearance.performAsCurrentDrawingAppearance {
            NDPaletteColors.wash.setFill()
            NSBezierPath(roundedRect: bounds, xRadius: NDPaletteMetrics.rowRadius,
                         yRadius: NDPaletteMetrics.rowRadius).fill()
        }
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

    /// Fresh rows highlight the first one (a picker), or nothing (an address
    /// bar, where Enter means the field).
    private var highlightFirst = true

    private var backdrop: NDPaletteBackdrop?
    /// Both cards together; the field's card is on top, the list's below.
    private var card: NSView?
    private var listCard: NSView?
    private var searchField: NSTextField?
    private var tableView: NSTableView?
    private var scrollView: NSScrollView?
    private var listHeight: NSLayoutConstraint?
    private var listBottom: NSLayoutConstraint?
    private var fieldBottom: NSLayoutConstraint?

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
        searchField?.placeholderAttributedString = placeholderText()
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
                highlight(defaultHighlight)
                reassertFieldFocus()
            }
        }
        if presented { applyCompletion() }
    }

    func setHighlightFirst(_ on: Bool) {
        highlightFirst = on
    }

    private var defaultHighlight: Int { highlightFirst && !rows.isEmpty ? 0 : -1 }

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
        quietSelection()
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
        // The window's frame view, not its content view: the toolbar and a
        // sidebar draw above the content view, and Arc dims all of them.
        buildUI(in: content.superview ?? content)
        presented = true
        animateIn()
        highlight(defaultHighlight)
        // Becoming first responder selects the field's whole text, which is
        // what a seeded open (the current address) wants.
        if let field = searchField { window.makeFirstResponder(field) }
        quietSelection()
    }

    func userCancel() { dismiss(programmatic: false) }

    private func dismiss(programmatic: Bool) {
        pendingOpen = false
        guard presented else { return }
        presented = false
        let window = backdrop?.window
        if let backdrop { animateOut(backdrop) }
        backdrop = nil
        card = nil
        searchField = nil
        tableView = nil
        scrollView = nil
        listHeight = nil
        listBottom = nil
        fieldBottom = nil
        listCard = nil
        // Focus goes back where it was (the page, usually); a responder that
        // left the window meanwhile leaves it with the window.
        if let window, let back = returnFocus, (back as? NSView)?.window === window || back === window {
            window.makeFirstResponder(back)
        }
        returnFocus = nil
        if !programmatic { ndEmitEvent(nodeID, "cancel", "{}") }
    }

    // ---- motion ----
    // The scrim and the field fade in; the list settles in from a touch
    // smaller, anchored at its top, on a short spring. Leaving is a quicker
    // fade. Reduce Motion keeps the fades and drops the scale.

    private static let closeDuration: CFTimeInterval = 0.12
    private static let listScale: CGFloat = 0.98
    private static let easeOut = CAMediaTimingFunction(controlPoints: 0.23, 1, 0.32, 1)

    /// Response 0.30 s, damping 0.86, as stiffness and damping for a unit mass.
    private static func settle(_ keyPath: String) -> CASpringAnimation {
        let a = CASpringAnimation(keyPath: keyPath)
        let response = 0.30
        let fraction = 0.86
        a.mass = 1
        a.stiffness = pow(2 * .pi / response, 2)
        a.damping = 4 * .pi * fraction / response
        a.duration = a.settlingDuration
        return a
    }

    private func animateIn() {
        guard let backdrop, let dim = backdrop.layer else { return }
        backdrop.layoutSubtreeIfNeeded()
        let fade = Self.settle("opacity")
        fade.fromValue = 0
        fade.toValue = 1
        dim.add(fade, forKey: "nd-palette-in")
        if let list = listCard { animateListIn(list) }
    }

    private func animateListIn(_ list: NSView) {
        guard !list.isHidden, let layer = list.layer else { return }
        let fade = Self.settle("opacity")
        fade.fromValue = 0
        fade.toValue = 1
        layer.add(fade, forKey: "nd-palette-list-fade")
        guard !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion else { return }
        // Layer-backed views keep their anchor at the corner, so the scale is
        // built around the card's top edge by hand. The card is unflipped:
        // its top is the layer's maxY.
        let b = list.bounds
        var t = CATransform3DMakeTranslation(b.midX, b.maxY, 0)
        t = CATransform3DScale(t, Self.listScale, Self.listScale, 1)
        t = CATransform3DTranslate(t, -b.midX, -b.maxY, 0)
        let scale = Self.settle("transform")
        scale.fromValue = NSValue(caTransform3D: t)
        scale.toValue = NSValue(caTransform3D: CATransform3DIdentity)
        layer.add(scale, forKey: "nd-palette-list-scale")
    }

    /// The old scrim fades out on its own; the handle has already let go of
    /// it, so a reopen during the fade builds a fresh one beside it.
    private func animateOut(_ old: NSView) {
        guard let layer = old.layer else { old.removeFromSuperview(); return }
        (old as? NDPaletteBackdrop)?.handle = nil
        CATransaction.begin()
        CATransaction.setCompletionBlock { old.removeFromSuperview() }
        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = 1
        fade.toValue = 0
        fade.duration = Self.closeDuration
        fade.timingFunction = Self.easeOut
        layer.add(fade, forKey: "nd-palette-out")
        layer.opacity = 0
        CATransaction.commit()
    }

    private func buildUI(in content: NSView) {
        let backdrop = NDPaletteBackdrop(frame: content.bounds)
        backdrop.handle = self
        backdrop.autoresizingMask = [.width, .height]
        backdrop.wantsLayer = true
        content.effectiveAppearance.performAsCurrentDrawingAppearance {
            backdrop.layer?.backgroundColor = NDPaletteColors.ground
                .withAlphaComponent(NDPaletteMetrics.scrimAlpha).cgColor
        }
        backdrop.setAccessibilityIdentifier("nd-command-palette-backdrop")

        // A plain holder for the two cards, so the pair is placed as one.
        let card = NSView()
        card.translatesAutoresizingMaskIntoConstraints = false
        card.setAccessibilityElement(true)
        card.setAccessibilityRole(.group)
        card.setAccessibilityLabel(placeholder ?? "Command bar")
        backdrop.addSubview(card)

        let fieldCard = NDPaletteSurface(shadowAlpha: 0.06, shadowBlur: 24, shadowDrop: 8)
        card.addSubview(fieldCard)
        let listCard = NDPaletteSurface(shadowAlpha: 0.07, shadowBlur: 20, shadowDrop: 6)
        card.addSubview(listCard)

        let field = NSTextField()
        field.translatesAutoresizingMaskIntoConstraints = false
        field.isBezeled = false
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        let font = NSFont.systemFont(ofSize: NDPaletteMetrics.fieldFontSize)
        field.font = font
        field.textColor = NDPaletteColors.ink
        field.cell?.isScrollable = true
        field.cell?.wraps = false
        field.cell?.usesSingleLineMode = true
        field.lineBreakMode = .byClipping
        field.placeholderAttributedString = placeholderText()
        field.setAccessibilityLabel(placeholder)
        field.stringValue = controlledQuery
        field.delegate = self
        fieldCard.addSubview(field)

        let table = NSTableView()
        let column = NSTableColumn(identifier: paletteColumnID)
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)
        table.headerView = nil
        table.style = .plain
        table.rowHeight = NDPaletteMetrics.rowHeight
        table.intercellSpacing = NSSize(width: 0, height: 0)
        table.backgroundColor = .clear
        table.gridStyleMask = []
        table.selectionHighlightStyle = .regular
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
        listCard.addSubview(scroll)

        let m = NDPaletteMetrics.margin
        let pad = NDPaletteMetrics.listPadding
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
        // The line box of the field's font, so the card's vertical inset is
        // measured from the text rather than from a guessed control height.
        let lineHeight = ceil(font.ascender - font.descender + font.leading)
        NSLayoutConstraint.activate([
            card.centerXAnchor.constraint(equalTo: backdrop.centerXAnchor),
            top,
            card.topAnchor.constraint(greaterThanOrEqualTo: backdrop.topAnchor, constant: m),
            cardWidth,
            card.widthAnchor.constraint(lessThanOrEqualTo: backdrop.widthAnchor, constant: -2 * m),
            card.bottomAnchor.constraint(lessThanOrEqualTo: backdrop.bottomAnchor, constant: -m),

            fieldCard.topAnchor.constraint(equalTo: card.topAnchor),
            fieldCard.leadingAnchor.constraint(equalTo: card.leadingAnchor),
            fieldCard.trailingAnchor.constraint(equalTo: card.trailingAnchor),

            field.leadingAnchor.constraint(equalTo: fieldCard.leadingAnchor, constant: NDPaletteMetrics.fieldInsetX),
            field.trailingAnchor.constraint(equalTo: fieldCard.trailingAnchor, constant: -NDPaletteMetrics.fieldInsetX),
            field.topAnchor.constraint(equalTo: fieldCard.topAnchor, constant: NDPaletteMetrics.fieldInsetY),
            field.bottomAnchor.constraint(equalTo: fieldCard.bottomAnchor, constant: -NDPaletteMetrics.fieldInsetY),
            field.heightAnchor.constraint(equalToConstant: lineHeight),

            listCard.topAnchor.constraint(equalTo: fieldCard.bottomAnchor, constant: NDPaletteMetrics.gap),
            listCard.leadingAnchor.constraint(equalTo: card.leadingAnchor),
            listCard.trailingAnchor.constraint(equalTo: card.trailingAnchor),

            scroll.topAnchor.constraint(equalTo: listCard.topAnchor, constant: pad),
            scroll.leadingAnchor.constraint(equalTo: listCard.leadingAnchor, constant: pad),
            scroll.trailingAnchor.constraint(equalTo: listCard.trailingAnchor, constant: -pad),
            scroll.bottomAnchor.constraint(equalTo: listCard.bottomAnchor, constant: -pad),
            scroll.heightAnchor.constraint(lessThanOrEqualToConstant: NDPaletteMetrics.maxListHeight),
            listHeight,
        ])

        // The holder ends at the list card, or at the field when there is no list.
        listBottom = listCard.bottomAnchor.constraint(equalTo: card.bottomAnchor)
        fieldBottom = fieldCard.bottomAnchor.constraint(equalTo: card.bottomAnchor)

        content.addSubview(backdrop)
        table.reloadData()
        self.backdrop = backdrop
        self.card = card
        self.listCard = listCard
        self.searchField = field
        self.tableView = table
        self.scrollView = scroll
        self.listHeight = listHeight
        updateListHeight()
    }

    private func placeholderText() -> NSAttributedString {
        NSAttributedString(string: placeholder ?? "", attributes: [
            .font: NSFont.systemFont(ofSize: NDPaletteMetrics.fieldFontSize),
            .foregroundColor: NDPaletteColors.ink.withAlphaComponent(0.3),
        ])
    }

    /// Selected text is a tenth of the ink rather than a block of accent,
    /// which over a pale field would be the loudest thing in the window.
    private func quietSelection() {
        guard let editor = searchField?.currentEditor() as? NSTextView else { return }
        editor.selectedTextAttributes = [
            .backgroundColor: NDPaletteColors.ink.withAlphaComponent(0.12),
            .foregroundColor: NDPaletteColors.ink,
        ]
    }

    /// Height of the rendered rows, read back from the table. An empty result
    /// set has no list card at all: the field stands alone, and the pair's
    /// bottom is the field's.
    private func updateListHeight() {
        guard let listHeight else { return }
        let wasHidden = listCard?.isHidden ?? true
        listCard?.isHidden = rows.isEmpty
        listBottom?.isActive = !rows.isEmpty
        fieldBottom?.isActive = rows.isEmpty
        guard let table = tableView, !rows.isEmpty else {
            listHeight.constant = 0
            return
        }
        listHeight.constant = table.rect(ofRow: rows.count - 1).maxY
        if wasHidden, presented, let list = listCard {
            list.superview?.layoutSubtreeIfNeeded()
            animateListIn(list)
        }
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
        // An address bar walks back up out of the list into the field.
        if !highlightFirst, delta < 0, cur <= 0 {
            highlight(-1)
            return
        }
        let next = cur < 0 ? 0 : max(0, min(rows.count - 1, cur + delta))
        highlight(next)
    }

    // ---- emit ----

    private func emitActivate(_ idx: Int) {
        guard idx >= 0 && idx < rows.count else { return }
        ndEmitEvent(nodeID, "activate", "{\"text\":\(ndJsonString(rows[idx].id))}")
    }

    /// The field as shown: Return accepts a completion. `typedOnly` (Cmd or
    /// Ctrl+Return) leaves an unaccepted completion out.
    func emitSubmit(typedOnly: Bool = false) {
        let text = presented ? (typedOnly ? typed : typed + suffix) : controlledQuery
        ndEmitEvent(nodeID, "submit", "{\"text\":\(ndJsonString(text))}")
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

    // The highlight is a grey wash, so the texts keep their colours on it;
    // this stops NSTableCellView recolouring the title for a selection.
    override var backgroundStyle: NSView.BackgroundStyle {
        get { .normal }
        set {}
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
        iconView.imageScaling = .scaleProportionallyUpOrDown
        iconView.contentTintColor = NDPaletteColors.muted
        for f in [titleField, subtitleField, hintField] {
            f.translatesAutoresizingMaskIntoConstraints = false
            f.font = .systemFont(ofSize: 12)
            f.textColor = NDPaletteColors.muted
            f.lineBreakMode = .byTruncatingTail
            f.maximumNumberOfLines = 1
            f.cell?.usesSingleLineMode = true
        }
        titleField.font = .systemFont(ofSize: 13)
        titleField.textColor = NDPaletteColors.ink
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
            iconView.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            iconView.centerYAnchor.constraint(equalTo: centerYAnchor),
            iconView.widthAnchor.constraint(equalToConstant: side),
            iconView.heightAnchor.constraint(equalToConstant: side),

            titleField.leadingAnchor.constraint(equalTo: iconView.trailingAnchor, constant: 10),
            titleField.centerYAnchor.constraint(equalTo: centerYAnchor),
            titleCap,

            subtitleGap,
            subtitleField.firstBaselineAnchor.constraint(equalTo: titleField.firstBaselineAnchor),

            hintGap,
            hintField.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
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
    if let h = propBool(props, "highlightFirst") { handle.setHighlightFirst(h) }
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

func ndCommandPaletteApplyHighlightFirst(_ view: NSView, _ on: Bool) {
    (view as? NDCommandPaletteHandleView)?.setHighlightFirst(on)
}

func ndCommandPaletteApplyOpen(_ view: NSView, _ open: Bool) {
    (view as? NDCommandPaletteHandleView)?.applyOpen(open)
}

func ndCommandPaletteConnect(_ view: NSView, nodeID: UInt32) {
    (view as? NDCommandPaletteHandleView)?.nodeID = nodeID
}
