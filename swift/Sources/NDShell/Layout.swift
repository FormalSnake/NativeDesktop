import AppKit
import Foundation
import ObjectiveC

// AppKit peer of GTK's box-layout style keys (src/gtk/style.zig's hexpand/
// vexpand/halign/valign + padding/margin + size requests). The flags are
// recorded at style-apply time (`ndApplyStyle` in Backend.swift) and read by
// `NDBoxView.layout()`. src/tree.zig applies style BEFORE the child is
// appended on create, so there is nothing to place against yet when they
// arrive.
struct NDLayoutFlags {
    var hexpand = false
    var vexpand = false
    var halign: String? = nil
    var valign: String? = nil
    var margin = NSEdgeInsets()
    var minWidth: CGFloat = 0
    var minHeight: CGFloat = 0
}

/// Per-child flags, keyed off the child view's identity — same idiom as
/// Backend.swift's `gridCells`. NOT private: read/written from both this
/// file (box layout) and Backend.swift (`ndApplyStyle`).
nonisolated(unsafe) var ndLayoutFlags: [ObjectIdentifier: NDLayoutFlags] = [:]

/// The widget kind (`schema/widgets.json` name) each tracked view was created
/// for, recorded by the generated `ndCreate` wrapper. The cross-axis default
/// is resolved from the KIND, never from the view's live intrinsic size: a
/// SwiftUI-hosted leaf reports a different intrinsic size before and after its
/// first measurement, and reading it per style apply flipped children between
/// natural and fill mid-session.
nonisolated(unsafe) var ndWidgetKinds: [ObjectIdentifier: String] = [:]

func ndRecordWidgetKind(_ view: NSView, _ kind: String) {
    ndWidgetKinds[ObjectIdentifier(view)] = kind
}

/// Separators inside a `boxed-list` card, which the box draws as an inset
/// hairline row divider rather than a full-bleed system line
/// (`ndStyleBoxedListDivider`).
nonisolated(unsafe) var ndBoxedListDividers: Set<ObjectIdentifier> = []

/// The four safe-area pins the generated Window append arm gives a plain root
/// child, in top/leading/trailing/bottom order. Kept so `ndWindowRootInset`
/// can be re-derived when the root's shape changes instead of being frozen at
/// attach (a root box that gains its scrolling child after mount).
nonisolated(unsafe) var ndWindowRootPins: [ObjectIdentifier: [NSLayoutConstraint]] = [:]

func ndRegisterWindowRootPins(_ child: NSView, _ pins: [NSLayoutConstraint]) {
    ndWindowRootPins[ObjectIdentifier(child)] = pins
}

/// Recomputes a window root child's content margin against its CURRENT shape.
func ndRefreshWindowRootInset(_ child: NSView) {
    guard let pins = ndWindowRootPins[ObjectIdentifier(child)], pins.count == 4 else { return }
    let inset = ndWindowRootInset(child)
    guard pins[0].constant != inset else { return }
    pins[0].constant = inset
    pins[1].constant = inset
    pins[2].constant = -inset
    pins[3].constant = -inset
}

/// release_node purge seam (Backend.swift's `ndPurgeNodeRegistries`).
func ndLayoutPurge(_ view: NSView) {
    let id = ObjectIdentifier(view)
    ndLayoutFlags[id] = nil
    ndWidgetKinds[id] = nil
    ndBoxedListDividers.remove(id)
    ndWindowRootPins[id] = nil
}


/// `NSButton` subclass carrying GTK-style `padding` — AppKit buttons have no
/// content-inset API, so padding is folded into `intrinsicContentSize`
/// instead (mirrors NDTextField below). `.rounded` bezels draw a fixed-height
/// pill centered in a taller frame (padding would just add dead space above/
/// below); `.flexiblePush` actually stretches to fill the frame, so a
/// non-zero padding switches to it (a real AppKit bezel style change).
final class NDButton: NSButton {
    /// A button promoted into a native source-list row remains the React/tree
    /// model and semantic-action target, but must not compete with the visible
    /// NSTableView for physical hit testing. Its padded stack-layout frame does
    /// not match the table's native row geometry, which otherwise makes clicks
    /// resolve to a neighboring invisible button.
    var ndIsSidebarRowModel = false

    /// Marked by the generated ToggleButton create arm: a toggle keeps its
    /// custom view when placed in a toolbar — promotion to a system-drawn
    /// momentary item (HeaderBar.swift's promotedItem) would hide its
    /// on/off state.
    var ndIsToggle = false

    /// A promoted toolbar item snapshots tooltip (and the label derived from
    /// it for icon-only buttons) at build time (HeaderBar.swift's
    /// promotedItem) — a later tooltip update must rebuild that item or the
    /// toolbar keeps showing and speaking the stale text for the session.
    override var toolTip: String? {
        didSet {
            guard toolTip != oldValue else { return }
            ndToolbarOwner(of: self)?.reseedItem(for: self)
        }
    }

    /// A sidebar row's visible text lives on the table cell, not on this
    /// button: the button is an alpha-0 model object and the cell's textField
    /// is only written inside tableView(_:viewFor:row:), which runs on reload.
    /// Without this a label-only update (Button.label is createAndUpdate) sets
    /// the title and the sidebar keeps showing the stale text for the session.
    override var title: String {
        didSet {
            guard title != oldValue, ndSidebarRowButtons.contains(ObjectIdentifier(self)) else { return }
            ndEnclosingSidebarTable(self)?.reload()
        }
    }

    override func hitTest(_ point: NSPoint) -> NSView? {
        ndIsSidebarRowModel ? nil : super.hitTest(point)
    }

    var ndPadding = NSEdgeInsets(top: 0, left: 0, bottom: 0, right: 0) {
        didSet {
            invalidateIntrinsicContentSize()
            let nonzero = ndPadding.top != 0 || ndPadding.left != 0 || ndPadding.bottom != 0 || ndPadding.right != 0
            if nonzero && bezelStyle == .rounded {
                if #available(macOS 14.0, *) {
                    bezelStyle = .flexiblePush
                }
            }
        }
    }

    override var intrinsicContentSize: NSSize {
        let s = super.intrinsicContentSize
        return NSSize(width: s.width + ndPadding.left + ndPadding.right, height: s.height + ndPadding.top + ndPadding.bottom)
    }
}

/// `NSTextField` subclass carrying GTK-style `padding` (same
/// intrinsicContentSize-inflation trick as NDButton; text fields have no
/// bezel-shape wrinkle so there's no equivalent of NDButton's bezelStyle switch).
final class NDTextField: NSTextField {
    var ndPadding = NSEdgeInsets(top: 0, left: 0, bottom: 0, right: 0) {
        didSet { invalidateIntrinsicContentSize() }
    }

    override var intrinsicContentSize: NSSize {
        let s = super.intrinsicContentSize
        return NSSize(width: s.width + ndPadding.left + ndPadding.right, height: s.height + ndPadding.top + ndPadding.bottom)
    }
}

/// Container for a SplitView content pane's real child, pinned to the
/// safe-area top so the pane's content clears the unified toolbar while the
/// pane itself still spans the full slot (see the SplitView content append
/// arm in tools/codegen.ts).
final class NDPaneHostView: NSView {
    override nonisolated var isFlipped: Bool { true }
}


// ============================================================================
// Box layout
// ============================================================================

/// Widget kinds whose native AppKit form is self-sized on both axes. GTK box
/// children fill the perpendicular axis unless told otherwise, and
/// transplanting that rule verbatim is what turned every AppKit control into a
/// full-window bar: a push button, popup, switch or date picker stretched to
/// the window width is never native, whatever the app tree says. AppKit's
/// answer is the opposite one, so these keep their own size and everything
/// else keeps GTK's fill.
///
/// Deliberately absent, because filling IS their native form: TextInput,
/// SearchInput and ComboBox (an empty field's natural width is a few points),
/// Image (one photo would dictate the window width), LevelIndicator and Chart
/// (a capacity bar or plot spanning its container), Breadcrumb, and every
/// container or scroll shape.
///
/// An app that genuinely wants a stretched control still says so with an
/// explicit `halign`/`valign` of "fill" or with the cross-axis expand flag, so
/// the GTK tree keeps working unchanged.
private let ndSelfSizedKinds: Set<String> = [
    "Label", "Button", "ToggleButton", "Radio", "Checkbox", "Select", "Switch",
    "SegmentedControl", "DatePicker", "ColorPicker", "MenuButton", "SplitButton",
    "FontPicker", "ShareButton", "LinkButton", "Spinner", "ProgressCircle",
    "NumberInput", "Avatar", "Badge", "Tag", "Kbd",
]

/// Whether `view`'s own natural size along `axis` is also its CORRECT size
/// there. Answered from the create-time widget kind, never from a live
/// intrinsic-size read.
func ndSelfSizedOnAxis(_ view: NSView, _ axis: NSLayoutConstraint.Orientation) -> Bool {
    let kind = ndWidgetKinds[ObjectIdentifier(view)] ?? ""
    // Length axis vs thickness axis: a slider and a determinate progress bar
    // have a natural thickness and no natural length, and answering per axis
    // is what stops a horizontal one from being stretched to a row's full
    // height without also pinning its length.
    if kind == "Slider" {
        let vertical = (view as? NDSliderView)?.vertical ?? false
        return axis == (vertical ? .horizontal : .vertical)
    }
    if kind == "ProgressBar" { return axis == .vertical }
    return ndSelfSizedKinds.contains(kind)
}

/// The AppKit peer of a GTK theme's natural width for a control whose length
/// carries no content of its own: an empty entry, a slider track, a capacity
/// bar. AppKit reports nothing for those, so along a horizontal box's main
/// axis they measured 0 and painted nothing.
private let ndNaturalLengthFloors: [String: CGFloat] = [
    "TextInput": 150,
    "SearchInput": 150,
    "ComboBox": 150,
    "Slider": 150,
    "ProgressBar": 150,
    "LevelIndicator": 100,
]

/// A label's text width is its natural size, never its minimum: taking it as a
/// minimum is what let one long caption on a hidden tab page hold the whole
/// window open at 1157pt.
private let ndLabelMinimumWidth: CGFloat = 60

/// GTK's `propagate-natural-height` cap for a scroll-shaped data widget.
private let ndScrollNaturalRowCap = 10

/// Floor for a scroll shape whose document has no row model (TextArea,
/// CodeEditor, a plain ScrollView with unconstrained content).
private let ndScrollNaturalMinHeight: CGFloat = 60

/// Leading inset of a `boxed-list` separator, drawn under the row text rather
/// than full-bleed (`ndStyleBoxedListDivider`).
private let ndBoxedListDividerInset: CGFloat = 12

/// Measurement calls `fittingSize`, which runs an Auto Layout or SwiftUI pass
/// that can invalidate an intrinsic size from inside our own. Propagating that
/// back up the chain would schedule a fresh layout on every pass forever, so
/// invalidation is suppressed while a box lays out: the size the pass reads
/// is already the new one.
nonisolated(unsafe) private var ndBoxLayoutDepth = 0

/// Marks every enclosing box's measurement stale. Called for anything that can
/// change what a box has to place: child attach/detach, a `visible` toggle, a
/// text/title update, a style apply, a padding or spacing change, and a hosted
/// SwiftUI leaf resizing itself.
func ndInvalidateBoxChain(from view: NSView) {
    guard ndBoxLayoutDepth == 0 else { return }
    var child: NSView = view
    var cursor: NSView? = view
    while let current = cursor {
        if let box = current as? NDBoxView {
            // Only the one child whose size can have changed, never the whole
            // cache: a commit of N appends must cost N flag sets and ONE
            // measurement pass, not N sweeps of N entries.
            if current !== child { box.ndInvalidateChildMeasure(child) }
            box.ndMarkLayoutDirty()
        }
        ndScheduleShapeRefresh(current)
        child = current
        cursor = current.superview
    }
}

/// Window roots whose content margin depends on a shape that is still
/// settling, and whether a coalescing pass is already queued. Deriving it
/// walks the root's children, so it runs once per runloop turn rather than
/// once per attach.
nonisolated(unsafe) private var ndPendingShapeRefresh: [ObjectIdentifier: NSView] = [:]
nonisolated(unsafe) private var ndShapeRefreshScheduled = false

private func ndScheduleShapeRefresh(_ view: NSView) {
    let key = ObjectIdentifier(view)
    guard ndWindowRootPins[key] != nil else { return }
    ndPendingShapeRefresh[key] = view
    guard !ndShapeRefreshScheduled else { return }
    ndShapeRefreshScheduled = true
    DispatchQueue.main.async {
        ndShapeRefreshScheduled = false
        let pending = ndPendingShapeRefresh
        ndPendingShapeRefresh.removeAll()
        for view in pending.values { ndRefreshWindowRootInset(view) }
    }
}

/// The natural (unconstrained) size of a box child, measured once per dirty
/// cycle by the box that owns it.
func ndNaturalChildSize(_ view: NSView) -> NSSize {
    if let box = view as? NDBoxView { return box.ndNaturalSize() }
    // The tracked TabView handle is the CONTROLLER's view, which for the
    // segmented-control-on-top style is a plain NSView wrapping the NSTabView
    // plus its strip, not the tab view itself.
    if let tabs = ndTabViewController(for: view)?.tabView { return ndTabViewNaturalSize(tabs, host: view) }
    // Intrinsic first, fitting only where there is no intrinsic answer: a
    // leaf's intrinsic size IS its natural size, while an NSHostingView's
    // fitting size is the size its SwiftUI body would ACCEPT, which for a
    // slider or a checkbox is three times the control it draws.
    var size = NSSize.zero
    let intrinsic = view.intrinsicContentSize
    if intrinsic.width != NSView.noIntrinsicMetric { size.width = intrinsic.width }
    if intrinsic.height != NSView.noIntrinsicMetric { size.height = intrinsic.height }
    if size.width <= 0 || size.height <= 0 {
        let fitting = view.fittingSize
        if size.width <= 0 { size.width = fitting.width }
        if size.height <= 0 { size.height = fitting.height }
    }
    // NSTextField's intrinsic width runs a point or two short of the width its
    // cell actually draws into, and a label held at the shorter one loses its
    // last glyph. `fittingSize` goes through the cell and is the number
    // NSStackView used before this.
    if view is NSTextField {
        let fitting = view.fittingSize
        size.width = max(size.width, fitting.width)
        size.height = max(size.height, fitting.height)
    }
    if let scroll = view as? NSScrollView {
        let content = ndScrollNaturalSize(scroll)
        size.width = max(size.width, content.width)
        size.height = max(size.height, content.height)
    }
    if let floor = ndNaturalLengthFloors[ndWidgetKinds[ObjectIdentifier(view)] ?? ""] {
        if (view as? NDSliderView)?.vertical == true {
            size.height = max(size.height, floor)
        } else {
            size.width = max(size.width, floor)
        }
    }
    return NSSize(width: max(0, size.width), height: max(0, size.height))
}

/// The size below which a child must not be squeezed. Separate from the
/// natural size on purpose (the GTK model): a wrapping label's minimum is a
/// floor, its natural is the width of its text, and only the floor may reach a
/// window or split-pane minimum.
///
/// The per-axis answer comes from whether AppKit reports an intrinsic size on
/// that axis at all: a control that describes its own size must not be
/// squashed below it, while a container or a scroll shape has no content size
/// to defend and is the right thing to take space back from.
func ndMinimumChildSize(_ view: NSView) -> NSSize {
    if let box = view as? NDBoxView { return box.ndMinimumSize() }
    let natural = ndNaturalChildSize(view)
    let intrinsic = view.intrinsicContentSize
    var floor = NSSize.zero
    if intrinsic.width != NSView.noIntrinsicMetric { floor.width = natural.width }
    if intrinsic.height != NSView.noIntrinsicMetric { floor.height = natural.height }
    let kind = ndWidgetKinds[ObjectIdentifier(view)] ?? ""
    if kind == "Label" {
        floor.width = min(floor.width, ndLabelMinimumWidth)
    } else if ndNaturalLengthFloors[kind] != nil {
        // The length these carry is a preference, not a requirement: an entry
        // or a track is as long as it is given.
        if (view as? NDSliderView)?.vertical == true { floor.height = 0 } else { floor.width = 0 }
    }
    return floor
}

/// An `NSTabView`'s content height is whatever the SELECTED page needs, so
/// measuring it live moved every sibling below the tab strip on each switch
/// (68 nodes by up to 456pt in the gallery). GTK stacks are homogeneous by
/// default; measure the same way, as the maximum over all pages.
private func ndTabViewNaturalSize(_ tabs: NSTabView, host: NSView) -> NSSize {
    var content = NSSize.zero
    for item in tabs.tabViewItems {
        guard let page = item.viewController?.view else { continue }
        let size = ndNaturalChildSize(page)
        content.width = max(content.width, size.width)
        content.height = max(content.height, size.height)
    }
    // Chrome is the tab strip plus the content border, read off live geometry
    // when there is any so a control-size or style change tracks. Before the
    // first pass there is none, and the fallback is what keeps a tab view from
    // measuring as its strip alone.
    var chrome = NSSize(width: 0, height: 40)
    if host.bounds.width > 0, host.bounds.height > 0 {
        let rect = tabs.contentRect
        chrome = NSSize(width: max(0, host.bounds.width - rect.width),
                        height: max(0, host.bounds.height - rect.height))
    }
    let floor = tabs.minimumSize
    return NSSize(width: max(content.width + chrome.width, floor.width),
                  height: max(content.height + chrome.height, floor.height))
}

/// GTK's `propagate-natural-height` for the scroll-shaped data widgets. An
/// NSScrollView's own fitting height is its scroller metrics and no content at
/// all, which measured a populated `<table>` as 0pt tall and let its header
/// paint over the next sibling.
private func ndScrollNaturalSize(_ scroll: NSScrollView) -> NSSize {
    guard let doc = scroll.documentView else { return .zero }
    if let table = doc as? NSTableView {
        let rows = max(1, min(table.numberOfRows, ndScrollNaturalRowCap))
        var height = CGFloat(rows) * (table.rowHeight + table.intercellSpacing.height)
        if table.headerView != nil { height += max(table.headerView?.frame.height ?? 0, table.rowHeight) }
        return NSSize(width: 0, height: height + scroll.contentInsets.top + scroll.contentInsets.bottom)
    }
    return NSSize(width: 0, height: max(doc.fittingSize.height, ndScrollNaturalMinHeight))
}

/// The view behind every `<box>` (nested boxes included): a deterministic
/// manual-layout container rather than an `NSStackView`.
///
/// An NSStackView sizes its children through content-hugging and compression
/// -resistance priority fights, and those priorities had to be rewritten on
/// every style apply. Two consequences the constraint model could not be
/// talked out of: the leftover space along the main axis went to ONE view
/// (`.fill` hands it to the lowest-hugging arranged subview) where GTK splits
/// it evenly among expanding children, and the cross-axis default was
/// re-decided from a LIVE `intrinsicContentSize` read, so a colour change
/// could flip a child between natural and fill depending on whether a SwiftUI
/// leaf had measured yet.
///
/// Here the box owns the arithmetic: children are frame-placed from a cached
/// bottom-up measurement, expanding children split leftover space equally, and
/// the cross-axis default is resolved from the create-time widget kind
/// (`ndSelfSizedOnAxis`). A child that must stay constraint-driven internally
/// (a split view, a scroll view, a hosted SwiftUI leaf) still gets its frame
/// set from out here; only what is INSIDE it is Auto Layout's business.
final class NDBoxView: NSView {
    // nonisolated: every coordinate conversion reads this, and a MainActor
    // -isolated getter puts an executor check on each one.
    override nonisolated var isFlipped: Bool { true }

    var ndOrientation: NSUserInterfaceLayoutOrientation = .vertical {
        didSet { if ndOrientation != oldValue { ndInvalidateBoxChain(from: self) } }
    }

    var ndSpacing: CGFloat = ndStandardSpacing {
        didSet { if ndSpacing != oldValue { ndInvalidateBoxChain(from: self) } }
    }

    var ndPadding = NSEdgeInsets() {
        didSet { if !ndEdgeInsetsEqual(ndPadding, oldValue) { ndInvalidateBoxChain(from: self) } }
    }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        // Below NSWindow's own windowSizeStayPut (500): a box reports what its
        // content wants, and a window that cannot grant it shrinks the content
        // rather than resizing itself. At the AppKit default (750) a tall page
        // inside a homogeneous tab view stretched the window to 1426pt tall.
        setContentCompressionResistancePriority(NSLayoutConstraint.Priority(400), for: .horizontal)
        setContentCompressionResistancePriority(NSLayoutConstraint.Priority(400), for: .vertical)
        // GTK boxes clip. Without this a page that does not fit its slot draws
        // over whatever is next to it instead of being cut off at the edge.
        clipsToBounds = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("NDBoxView is not NSCoding-decodable") }

    /// The React child list in document order. Decoration subviews (a card
    /// backing, the source-list table, the hover overlay) are ordinary
    /// constraint-pinned subviews and are deliberately not in here.
    private(set) var ndChildren: [NSView] = []

    /// Membership index beside `ndChildren`. React never re-appends a child
    /// that is already here, so the identity scan that would answer this is
    /// pure O(n) overhead on the one path a 10k-node mount runs 10k times.
    private var childKeys: Set<ObjectIdentifier> = []

    private var naturalCache: NSSize? = nil
    private var minimumCache: NSSize? = nil
    private var lastLaidOutSize = NSSize(width: -1, height: -1)
    private var childNaturals: [ObjectIdentifier: NSSize] = [:]
    private var childMinimums: [ObjectIdentifier: NSSize] = [:]

    // MARK: children

    func ndAppend(_ child: NSView) {
        let key = ObjectIdentifier(child)
        if childKeys.contains(key) { detachFromList(child) }
        ndChildren.append(child)
        childKeys.insert(key)
        ndAdopt(child)
    }

    func ndInsert(_ child: NSView, before sibling: NSView) {
        let key = ObjectIdentifier(child)
        if childKeys.contains(key) { detachFromList(child) }
        let index = ndChildren.firstIndex { $0 === sibling } ?? ndChildren.count
        ndChildren.insert(child, at: index)
        childKeys.insert(key)
        ndAdopt(child)
    }

    func ndRemove(_ child: NSView) {
        detachFromList(child)
        if child.superview === self { child.removeFromSuperview() }
        ndInvalidateChildMeasure(child)
        ndBoxChildDetached(self, child)
        ndInvalidateBoxChain(from: self)
    }

    private func detachFromList(_ child: NSView) {
        guard childKeys.remove(ObjectIdentifier(child)) != nil else { return }
        if let index = ndChildren.firstIndex(where: { $0 === child }) { ndChildren.remove(at: index) }
    }

    private func ndAdopt(_ child: NSView) {
        child.translatesAutoresizingMaskIntoConstraints = true
        if child.superview !== self { addSubview(child) }
        ndInvalidateBoxChain(from: self)
        ndBoxChildAttached(self, child)
    }

    // MARK: measurement

    /// Drops only the totals; the per-child measurements stay, since a
    /// sibling's size did not change. Recomputed lazily in the next layout or
    /// measurement, so a batch of appends costs one pass, not one per append.
    func ndMarkLayoutDirty() {
        naturalCache = nil
        minimumCache = nil
        invalidateIntrinsicContentSize()
        needsLayout = true
    }

    func ndInvalidateChildMeasure(_ child: NSView) {
        let key = ObjectIdentifier(child)
        childNaturals[key] = nil
        childMinimums[key] = nil
    }

    private func naturalSize(of child: NSView) -> NSSize {
        let key = ObjectIdentifier(child)
        if let cached = childNaturals[key] { return cached }
        var size = ndNaturalChildSize(child)
        let flags = ndLayoutFlags[key] ?? NDLayoutFlags()
        size.width = max(size.width, flags.minWidth)
        size.height = max(size.height, flags.minHeight)
        childNaturals[key] = size
        return size
    }

    private func minimumSize(of child: NSView) -> NSSize {
        let key = ObjectIdentifier(child)
        if let cached = childMinimums[key] { return cached }
        var size = ndMinimumChildSize(child)
        let flags = ndLayoutFlags[key] ?? NDLayoutFlags()
        size.width = max(size.width, flags.minWidth)
        size.height = max(size.height, flags.minHeight)
        childMinimums[key] = size
        return size
    }

    /// The box's own natural size: children at their natural sizes, plus
    /// spacing, margins and padding.
    func ndNaturalSize() -> NSSize {
        if let cached = naturalCache { return cached }
        let size = aggregate { naturalSize(of: $0) }
        naturalCache = size
        return size
    }

    /// The box's own minimum size, aggregated from its children's minimums.
    /// Read by the split-pane slots so a pane cannot be squeezed past what its
    /// content needs, and used as the shrink floor below.
    func ndMinimumSize() -> NSSize {
        if let cached = minimumCache { return cached }
        let size = aggregate { minimumSize(of: $0) }
        minimumCache = size
        return size
    }

    private func aggregate(_ sizeOf: (NSView) -> NSSize) -> NSSize {
        let horizontal = ndOrientation == .horizontal
        var main: CGFloat = 0
        var cross: CGFloat = 0
        var count = 0
        for child in ndChildren where !child.isHidden {
            let flags = ndLayoutFlags[ObjectIdentifier(child)] ?? NDLayoutFlags()
            let size = sizeOf(child)
            let outerWidth = size.width + flags.margin.left + flags.margin.right
            let outerHeight = size.height + flags.margin.top + flags.margin.bottom
            main += horizontal ? outerWidth : outerHeight
            cross = max(cross, horizontal ? outerHeight : outerWidth)
            count += 1
        }
        if count > 1 { main += ndSpacing * CGFloat(count - 1) }
        let own = ndLayoutFlags[ObjectIdentifier(self)] ?? NDLayoutFlags()
        let width = (horizontal ? main : cross) + ndPadding.left + ndPadding.right
        let height = (horizontal ? cross : main) + ndPadding.top + ndPadding.bottom
        return NSSize(width: max(width, own.minWidth), height: max(height, own.minHeight))
    }

    override var intrinsicContentSize: NSSize {
        guard ndChildren.contains(where: { !$0.isHidden }) else {
            return NSSize(width: NSView.noIntrinsicMetric, height: NSView.noIntrinsicMetric)
        }
        return ndNaturalSize()
    }

    // MARK: layout

    override func layout() {
        super.layout()
        ndPlaceChildren()
    }

    private func ndPlaceChildren() {
        let visible = ndChildren.filter { !$0.isHidden }
        guard !visible.isEmpty else { return }
        // A box with no size of its own yet (created, not installed) has
        // nothing meaningful to place, and handing children a zero frame is
        // not free: a hosted SwiftUI body collapses and does not come back
        // when the real size arrives.
        guard bounds.width > 0, bounds.height > 0 else { return }
        ndBoxLayoutDepth += 1
        defer { ndBoxLayoutDepth -= 1 }

        // A child that reflows is measured against the cross size it is being
        // given, so every cached measurement is only valid for the bounds it
        // was taken at. Nothing in the tree changes on a resize, so no
        // invalidation reaches here otherwise, and a box first laid out at
        // zero width kept a zero measurement.
        if !NSEqualSizes(bounds.size, lastLaidOutSize) {
            lastLaidOutSize = bounds.size
            childNaturals.removeAll(keepingCapacity: true)
            childMinimums.removeAll(keepingCapacity: true)
            naturalCache = nil
            minimumCache = nil
        }

        let horizontal = ndOrientation == .horizontal
        let crossAxis: NSLayoutConstraint.Orientation = horizontal ? .vertical : .horizontal
        let rightToLeft = userInterfaceLayoutDirection == .rightToLeft
        let innerWidth = max(0, bounds.width - ndPadding.left - ndPadding.right)
        let innerHeight = max(0, bounds.height - ndPadding.top - ndPadding.bottom)
        let crossAvailable = horizontal ? innerHeight : innerWidth
        let mainAvailable = horizontal ? innerWidth : innerHeight

        var crossSizes: [CGFloat] = []
        var crossOffsets: [CGFloat] = []
        var mainSizes: [CGFloat] = []
        var mainFloors: [CGFloat] = []
        var mainLeads: [CGFloat] = []
        var mainTrails: [CGFloat] = []
        var expanding: [Bool] = []
        var unmeasurable: [Bool] = []

        for child in visible {
            let key = ObjectIdentifier(child)
            let flags = ndLayoutFlags[key] ?? NDLayoutFlags()
            let divider = ndBoxedListDividers.contains(key)
            let crossLead = horizontal ? flags.margin.top : flags.margin.left
            let crossTrail = horizontal ? flags.margin.bottom : flags.margin.right
            let slot = max(0, crossAvailable - crossLead - crossTrail)

            // GTK's default is "fill" for every child; here it is resolved per
            // child, because a stretched AppKit control is never native. GTK's
            // expand implies fill on the same axis, so a child that expands
            // across the box still fills it.
            let crossExpands = horizontal ? flags.vexpand : flags.hexpand
            var align = (horizontal ? flags.valign : flags.halign)
                ?? ((crossExpands || !ndSelfSizedOnAxis(child, crossAxis)) ? "fill" : "natural")
            if !["fill", "natural", "start", "center", "end"].contains(align) {
                FileHandle.standardError.write(
                    "ND_WARN halign/valign \"\(align)\" is not a recognized alignment value (expected fill/start/center/end)\n".data(using: .utf8)!)
                align = "fill"
            }

            var natural = naturalSize(of: child)
            var crossSize = align == "fill" ? slot : min(horizontal ? natural.height : natural.width, slot)
            var crossOffset: CGFloat
            switch align {
            case "fill", "start":
                crossOffset = crossLead
            case "center":
                crossOffset = crossLead + (slot - crossSize) / 2
            case "end":
                crossOffset = crossLead + (slot - crossSize)
            default:
                // The resolved default for a control whose own cross size is
                // the right one: a vertical box places it at the leading edge
                // (GTK "start"), a horizontal one centers it across the row,
                // which is how AppKit places a control in a row.
                crossOffset = horizontal ? crossLead + (slot - crossSize) / 2 : crossLead
            }
            if divider {
                crossSize = max(0, slot - ndBoxedListDividerInset)
                crossOffset = crossLead + ndBoxedListDividerInset
            }
            // A vertical box's cross axis is the writing axis, so start/end
            // and the natural default follow the layout direction.
            if !horizontal && rightToLeft && align != "fill" {
                crossOffset = max(0, crossAvailable - crossSize - crossOffset)
            }

            // Main axis follows cross axis for anything that reflows: a
            // wrapping label, and a SwiftUI body, whose unconstrained ideal
            // size is not the size it settles at once it has a width (a hosted
            // slider measures 48pt tall unasked and 16pt once placed). Hand
            // the child the cross size it is about to get, then re-ask.
            if crossSize > 0, ndReflows(child, given: crossSize, horizontal: horizontal, natural: natural) {
                let current = horizontal ? child.frame.height : child.frame.width
                if abs(current - crossSize) > 0.5 {
                    if !horizontal, let field = child as? NSTextField {
                        field.preferredMaxLayoutWidth = crossSize
                        field.invalidateIntrinsicContentSize()
                    }
                    child.setFrameSize(horizontal
                        ? NSSize(width: child.frame.width, height: crossSize)
                        : NSSize(width: crossSize, height: child.frame.height))
                    childNaturals[key] = nil
                    natural = naturalSize(of: child)
                }
            }

            let mainNatural = divider ? 1 : (horizontal ? natural.width : natural.height)
            let minimum = minimumSize(of: child)
            crossSizes.append(crossSize)
            crossOffsets.append(crossOffset)
            mainSizes.append(mainNatural)
            mainFloors.append(divider ? 1 : (horizontal ? minimum.width : minimum.height))
            mainLeads.append(horizontal ? flags.margin.left : flags.margin.top)
            mainTrails.append(horizontal ? flags.margin.right : flags.margin.bottom)
            expanding.append(horizontal ? flags.hexpand : flags.vexpand)
            // A container reports `noIntrinsicMetric`: a split view, a scroll
            // view or a tab view has no content size of its own and filling is
            // its native form. GTK never has this case (every widget there has
            // a natural size), so there is no expand flag in the tree to carry
            // it, and a box whose only child is one of these would otherwise
            // pack it at the synthesized natural size and leave the rest blank.
            let hostIntrinsic = child.intrinsicContentSize
            let intrinsicMain = horizontal ? hostIntrinsic.width : hostIntrinsic.height
            // A tab view reports the size of its tallest page, which is a
            // measurement, not a request: the pages are meant to run to the
            // slot's edge the way they do on every other platform.
            unmeasurable.append(intrinsicMain == NSView.noIntrinsicMetric || ndTabViewController(for: child) != nil)
        }

        ndDistributeMainAxis(&mainSizes, floors: mainFloors, leads: mainLeads, trails: mainTrails,
                             expanding: expanding, unmeasurable: unmeasurable, available: mainAvailable)

        var cursor = horizontal ? ndPadding.left : ndPadding.top
        let order = (horizontal && rightToLeft) ? Array(visible.indices).reversed().map { $0 } : Array(visible.indices)
        for i in order {
            cursor += mainLeads[i]
            let rect: NSRect
            if horizontal {
                rect = NSRect(x: cursor, y: ndPadding.top + crossOffsets[i],
                              width: mainSizes[i], height: crossSizes[i])
            } else {
                rect = NSRect(x: ndPadding.left + crossOffsets[i], y: cursor,
                              width: crossSizes[i], height: mainSizes[i])
            }
            // Backing-aligned, or a run of equal rows lands on alternating
            // 30/31pt pitches and the text in them renders soft. Sizes round
            // OUTWARD, not to nearest: a label rounded down by a third of a
            // point loses its last glyph.
            let placed = backingAlignedRect(rect, options: [.alignMinXNearest, .alignMinYNearest,
                                                            .alignWidthOutward, .alignHeightOutward])
            // Re-asserted every pass, not just at attach: a view controller's
            // view turns it back off when the controller loads (NSTabView
            // then sized itself from its tallest page instead of the slot).
            if !visible[i].translatesAutoresizingMaskIntoConstraints {
                visible[i].translatesAutoresizingMaskIntoConstraints = true
            }
            if !NSEqualRects(visible[i].frame, placed) { visible[i].frame = placed }
            cursor += mainSizes[i] + mainTrails[i] + ndSpacing
        }
    }

    /// GTK hands leftover main-axis space to the expanding children in equal
    /// shares (NSStackView's `.fill` gave it all to one). An overflow is taken
    /// back from the expanding children first, then from everyone down to
    /// their own minimum, so nothing is placed on top of a sibling.
    private func ndDistributeMainAxis(_ sizes: inout [CGFloat], floors: [CGFloat],
                                      leads: [CGFloat], trails: [CGFloat],
                                      expanding: [Bool], unmeasurable: [Bool],
                                      available: CGFloat) {
        guard !sizes.isEmpty else { return }
        var used = ndSpacing * CGFloat(sizes.count - 1)
        for i in sizes.indices { used += sizes[i] + leads[i] + trails[i] }
        let slack = available - used
        if slack > 0 {
            // Children that asked for the space first; a child with no size of
            // its own only gets it when nothing else claimed it, so an
            // unflagged scroll view next to a real vexpand child stays put.
            var takers = sizes.indices.filter { expanding[$0] }
            if takers.isEmpty { takers = sizes.indices.filter { unmeasurable[$0] } }
            guard !takers.isEmpty else { return }
            let share = slack / CGFloat(takers.count)
            for i in takers { sizes[i] += share }
            return
        }
        var deficit = -slack
        for pass in 0..<2 {
            guard deficit > 0.5 else { break }
            let victims = sizes.indices.filter { pass == 0 ? expanding[$0] : true }
                .filter { sizes[$0] > floors[$0] }
            guard !victims.isEmpty else { continue }
            let headroom = victims.reduce(CGFloat(0)) { $0 + sizes[$1] - floors[$1] }
            guard headroom > 0 else { continue }
            let take = min(deficit, headroom)
            for i in victims {
                sizes[i] -= (sizes[i] - floors[i]) / headroom * take
            }
            deficit -= take
        }
    }
}

/// Whether measuring `child` again against the cross size it is about to get
/// can produce a different main-axis size. Asked per child on every pass, so
/// the cheap "no" comes first: re-measuring a label that already fits, or a
/// control AppKit sizes on its own, costs a text-layout pass and answers with
/// the number already in hand.
private func ndReflows(_ child: NSView, given crossSize: CGFloat, horizontal: Bool, natural: NSSize) -> Bool {
    if let field = child as? NSTextField {
        // Only a wrapping label reflows, and only when the text does not fit.
        // `ellipsize` sets truncation instead, and such a label must be
        // truncated by the box rather than grow it.
        guard !field.isEditable, !horizontal else { return false }
        switch field.lineBreakMode {
        case .byWordWrapping, .byCharWrapping: return natural.width > crossSize + 0.5
        default: return false
        }
    }
    if child is NDBoxView || child is NSStackView { return true }
    if ndTabViewController(for: child) != nil { return true }
    return ndIsSwiftUIHosted(type(of: child))
}

/// Whether a class is an `NSHostingView` of some body. SwiftUI reports an
/// unconstrained ideal size until it has a width, so a hosted leaf has to be
/// measured again once the box has given it one (a hosted slider answers 48pt
/// tall unasked and 16pt placed). NSHostingView is generic, so there is no
/// single type to cast to; the walk is done once per class.
nonisolated(unsafe) private var ndSwiftUIHostedClasses: [ObjectIdentifier: Bool] = [:]

private func ndIsSwiftUIHosted(_ cls: AnyClass) -> Bool {
    let key = ObjectIdentifier(cls)
    if let known = ndSwiftUIHostedClasses[key] { return known }
    var answer = false
    var cursor: AnyClass? = cls
    while let current = cursor {
        if NSStringFromClass(current).contains("NSHostingView") { answer = true; break }
        cursor = class_getSuperclass(current)
    }
    ndSwiftUIHostedClasses[key] = answer
    return answer
}

/// Turns a `boxed-list` separator into a faint, leading-inset hairline row
/// divider (vs a standalone separator's full-bleed system line): a 1pt
/// `.separatorColor` fill, inset under the row text on the leading edge and
/// flush to the trailing card edge. Uses a `.custom` NSBox so the line is a
/// true hairline (`NSBox.fillColor` takes the dynamic `.separatorColor`
/// directly, so it tracks dark mode without a CALayer color). The geometry is
/// the box's (`ndBoxedListDividers`), so it survives a padding change with no
/// constraint to replay. Idempotent.
func ndStyleBoxedListDivider(_ sep: NSBox, in box: NDBoxView) {
    sep.boxType = .custom
    sep.borderWidth = 0
    sep.borderColor = .clear
    sep.fillColor = .separatorColor
    sep.titlePosition = .noTitle
    sep.contentViewMargins = .zero
    ndBoxedListDividers.insert(ObjectIdentifier(sep))
    ndInvalidateBoxChain(from: box)
}

/// Restores a separator to a full-bleed system line when its box drops
/// `boxed-list` (set-replace).
func ndUnstyleBoxedListDivider(_ sep: NSBox, in box: NDBoxView) {
    ndBoxedListDividers.remove(ObjectIdentifier(sep))
    sep.boxType = .separator
    sep.fillColor = .clear
    ndInvalidateBoxChain(from: box)
}

/// Structural bookkeeping for a child that just joined `box`.
///
/// A subtree attaching somewhere inside a sidebar box changes that sidebar's
/// shape, and its shape is what decides whether the source-list table takes
/// the box over at all (SidebarTable.swift's `ndReconcileSidebarTable`). The
/// box is still empty when its cssClasses land, so this attach hook is where
/// that decision actually gets made. `box` need not be the classed box itself
/// (a real app nests row buttons inside structural wrapper boxes, say a host
/// or project section), so this walks UP to find the enclosing sidebar.
func ndBoxChildAttached(_ box: NDBoxView, _ child: NSView) {
    if let sidebar = ndEnclosingSidebar(box) {
        ndReconcileSidebarTable(sidebar)
    }
    if let sep = child as? NSBox, ndBoxedLists.contains(ObjectIdentifier(box)) {
        ndStyleBoxedListDivider(sep, in: box)
    }
    // A source list only learns whether it is standing in for a sidebar pane
    // once it has a parent (SidebarTable.swift's surface rule).
    if let scroll = child as? NSScrollView, ndIsSourceListKind(child) {
        ndRefreshSourceListMaterial(scroll)
    }
}

/// The widget kinds whose native form is a source-list surface.
func ndIsSourceListKind(_ view: NSView) -> Bool {
    let kind = ndWidgetKinds[ObjectIdentifier(view)] ?? ""
    return kind == "SourceList" || kind == "SourceTree"
}

/// Mirror of `ndBoxChildAttached`. AppKit has already taken `child` out of the
/// box's child list by now, so the reconcile sees the post-removal shape.
func ndBoxChildDetached(_ box: NDBoxView, _ child: NSView) {
    ndBoxedListDividers.remove(ObjectIdentifier(child))
    if let sidebar = ndEnclosingSidebar(box) {
        ndSidebarRowButtons.remove(ObjectIdentifier(child))
        ndSidebarFallbackRows.remove(ObjectIdentifier(child))
        ndReconcileSidebarTable(sidebar)
    }
}
