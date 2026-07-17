import AppKit
import Foundation

// AppKit peer of GTK's box-layout style keys (src/gtk/style.zig's hexpand/
// vexpand/halign/valign + padding). NSStackView has no first-class expand/
// align widget properties like GtkWidget does, so the flags below are
// recorded at style-apply time (`ndApplyStyle` in Backend.swift) and
// consulted at attach time (`ndBoxChildAttached`, called from the generated
// Box append/insertBefore arms) — src/tree.zig applies style BEFORE the
// child is appended on create, so there is no stack to reconcile against
// yet; the recorded flags are read back once the child actually attaches.
struct NDLayoutFlags {
    var hexpand = false
    var vexpand = false
    var halign: String? = nil
    var valign: String? = nil
}

/// Per-child flags, keyed off the child view's identity — same idiom as
/// Backend.swift's `gridCells`. NOT private: read/written from both this
/// file (attach/detach reconciliation) and Backend.swift (`ndApplyStyle`).
nonisolated(unsafe) var ndLayoutFlags: [ObjectIdentifier: NDLayoutFlags] = [:]

/// The cross-axis "fill" constraint installed per child (see
/// `ndBoxChildAttached`), tracked so it can be deactivated/replaced instead
/// of accumulating a duplicate every time style is re-applied.
nonisolated(unsafe) private var ndCrossAxisConstraints: [ObjectIdentifier: NSLayoutConstraint] = [:]

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
    override var isFlipped: Bool { true }
}

/// Reconciles one arranged subview's expand/align style against its parent
/// stack — called when the child attaches (Box append/insertBefore arms) and
/// again whenever its style changes while already attached (`ndApplyStyle`).
///
/// Main axis: GTK's expand means "this child absorbs leftover space along the
/// box's orientation". NSStackView has no per-child expand flag, only
/// content-hugging priority plus a `.fill` distribution that hands leftover
/// space to the lowest-hugging arranged subview — so an expanding child gets
/// a low hugging priority and flips the stack to `.fill` (stacks are created
/// with `.gravityAreas`, GTK's non-expand default).
///
/// Cross axis: GTK's box children fill the perpendicular axis unless told
/// otherwise (`halign`/`valign`, default "fill"). NSStackView alignment
/// doesn't stretch arranged subviews, so "fill" is approximated with an
/// explicit cross-axis anchor constraint against the stack, inset by the
/// stack's own edgeInsets so it doesn't fight the padding those insets
/// represent.
///
/// "start"/"center"/"end" all need a per-child leading/top/centerX/centerY/
/// trailing/bottom pin against the stack — but a bare pin alone doesn't
/// work: NSStackView installs its OWN low-priority (~260) alignment
/// constraint on every arranged subview PLUS an even-weaker (~250) "fill"
/// wish pulling the far edge toward the stack's own far edge. A
/// single-anchor pin at priority 999 doesn't outright beat or lose to that
/// pair — the solver satisfies all three simultaneously by stretching the
/// child to span the full cross axis (which trivially satisfies a
/// centerX/trailing pin too, since a fully stretched view's center and
/// trailing edge already coincide with the stack's), so the child never
/// actually MOVES, it balloons instead (confirmed empirically: a probe pin
/// alone always produced a full-width/height stretch, regardless of
/// priority 999 or 1000). Fixing the child's cross-axis size first
/// (raising contentHuggingPriority AND contentCompressionResistancePriority
/// to `.required` for that axis, so its intrinsic size becomes
/// non-negotiable) removes that degree of freedom; with size pinned, the
/// 999 pin is the only thing left that can move the child, and it cleanly
/// wins over the stack's own ~260 pin (confirmed empirically: same setup
/// with hugging fixed first reliably produced the intended
/// start/centered/trailing-aligned frame, stable across repeated layout
/// passes and coexisting correctly with fill siblings in the same stack).
///
/// "start" used to be a no-op (relying on the stack's own create-time
/// alignment, Widgets.swift ~46-51: `.leading` for vertical stacks,
/// `.centerY` for horizontal) — that coincidentally matched GTK's "start"
/// for vertical boxes (`.leading` IS "start"), but was silently wrong for
/// horizontal ones: GTK's valign="start" always means top, never centered.
/// Now all three non-fill cases share one recipe, and "fill" resets
/// hugging/compression back to the AppKit defaults (250/750) on every call
/// so a child that previously held "start"/"center"/"end" doesn't leave a
/// stale required-priority behind when its align later changes — this
/// function must be idempotent regardless of entry path (direct attach from
/// the generated Box arms, restyle via `ndApplyStyle`, or
/// `ndBoxReconcileChildren`'s padding-change replay).
func ndBoxChildAttached(_ stack: NSStackView, _ child: NSView) {
    let flags = ndLayoutFlags[ObjectIdentifier(child)] ?? NDLayoutFlags()
    let vertical = stack.orientation == .vertical

    let expands = vertical ? flags.vexpand : flags.hexpand
    child.setContentHuggingPriority(expands ? NSLayoutConstraint.Priority(1) : NSLayoutConstraint.Priority(250),
                                     for: vertical ? .vertical : .horizontal)
    if expands {
        stack.distribution = .fill
    }

    if let existing = ndCrossAxisConstraints[ObjectIdentifier(child)] {
        existing.isActive = false
        ndCrossAxisConstraints[ObjectIdentifier(child)] = nil
    }

    let crossAxis: NSLayoutConstraint.Orientation = vertical ? .horizontal : .vertical
    let align = (vertical ? flags.halign : flags.valign) ?? "fill"
    switch align {
    case "fill":
        child.setContentHuggingPriority(NSLayoutConstraint.Priority(250), for: crossAxis)
        child.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(750), for: crossAxis)
        let constraint: NSLayoutConstraint
        if vertical {
            constraint = child.widthAnchor.constraint(equalTo: stack.widthAnchor,
                                                        constant: -(stack.edgeInsets.left + stack.edgeInsets.right))
        } else {
            constraint = child.heightAnchor.constraint(equalTo: stack.heightAnchor,
                                                         constant: -(stack.edgeInsets.top + stack.edgeInsets.bottom))
        }
        constraint.priority = NSLayoutConstraint.Priority(999)
        constraint.isActive = true
        ndCrossAxisConstraints[ObjectIdentifier(child)] = constraint
    case "start":
        // An explicit pin, not "no constraint": that was only ever correct
        // for vertical stacks, whose OWN create-time alignment happens to be
        // `.leading` (Widgets.swift ~46-51) — the same GTK "start" position.
        // Horizontal stacks are created `.centerY`, so a horizontal box's
        // "start" silently rendered as vertically CENTERED, not top-aligned
        // (GTK's valign="start" always means top, independent of the
        // .leading/.trailing text-direction axis that halign="start" rides
        // on). Pinning explicitly (same hugging-fix recipe as center/end)
        // fixes that mismatch and costs nothing for vertical boxes, whose
        // `.leading` stack default already puts the pin's target exactly
        // where the old free-ride landed.
        child.setContentHuggingPriority(NSLayoutConstraint.Priority(1000), for: crossAxis)
        child.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(1000), for: crossAxis)
        let constraint: NSLayoutConstraint
        if vertical {
            constraint = child.leadingAnchor.constraint(equalTo: stack.leadingAnchor,
                                                          constant: stack.edgeInsets.left)
        } else {
            constraint = child.topAnchor.constraint(equalTo: stack.topAnchor,
                                                      constant: stack.edgeInsets.top)
        }
        constraint.priority = NSLayoutConstraint.Priority(999)
        constraint.isActive = true
        ndCrossAxisConstraints[ObjectIdentifier(child)] = constraint
    case "center":
        child.setContentHuggingPriority(NSLayoutConstraint.Priority(1000), for: crossAxis)
        child.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(1000), for: crossAxis)
        let constraint: NSLayoutConstraint
        if vertical {
            constraint = child.centerXAnchor.constraint(equalTo: stack.centerXAnchor,
                                                          constant: (stack.edgeInsets.left - stack.edgeInsets.right) / 2)
        } else {
            constraint = child.centerYAnchor.constraint(equalTo: stack.centerYAnchor,
                                                          constant: (stack.edgeInsets.top - stack.edgeInsets.bottom) / 2)
        }
        constraint.priority = NSLayoutConstraint.Priority(999)
        constraint.isActive = true
        ndCrossAxisConstraints[ObjectIdentifier(child)] = constraint
    case "end":
        child.setContentHuggingPriority(NSLayoutConstraint.Priority(1000), for: crossAxis)
        child.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(1000), for: crossAxis)
        let constraint: NSLayoutConstraint
        if vertical {
            constraint = child.trailingAnchor.constraint(equalTo: stack.trailingAnchor,
                                                           constant: -stack.edgeInsets.right)
        } else {
            constraint = child.bottomAnchor.constraint(equalTo: stack.bottomAnchor,
                                                         constant: -stack.edgeInsets.bottom)
        }
        constraint.priority = NSLayoutConstraint.Priority(999)
        constraint.isActive = true
        ndCrossAxisConstraints[ObjectIdentifier(child)] = constraint
    default:
        FileHandle.standardError.write(
            "ND_WARN halign/valign \"\(align)\" is not a recognized alignment value (expected fill/start/center/end)\n".data(using: .utf8)!)
    }

    // A subtree attaching somewhere inside a `nd-native-sidebar` box becomes
    // (part of) its source-list table's row model (SidebarTable.swift):
    // reload marks/hides any newly-reached button and refreshes the table.
    // `stack` need not be the classed box itself — a real app nests row
    // buttons inside structural wrapper boxes (a host/project section), so
    // this walks UP to find the enclosing sidebar, not just an exact match.
    if let table = ndEnclosingSidebarTable(stack) {
        table.reload()
    }

    // A separator inside a `boxed-list` card renders as an inset hairline row
    // divider (vs a standalone separator's full-bleed line). Same both-orders
    // rationale as the sidebar promote above.
    if let sep = child as? NSBox, ndBoxedLists.contains(ObjectIdentifier(stack)) {
        ndStyleBoxedListDivider(sep, in: stack)
    }
}

/// The leading/trailing inset pins installed on a `boxed-list` separator (see
/// `ndStyleBoxedListDivider`), tracked so a reconcile (padding change) or a
/// class drop can replace/deactivate them rather than stacking duplicates.
nonisolated(unsafe) private var ndBoxedListDividerPins: [ObjectIdentifier: [NSLayoutConstraint]] = [:]

/// Turns a `boxed-list` separator into a faint, leading-inset hairline row
/// divider (vs a standalone separator's full-bleed system line): a 1pt
/// `.separatorColor` fill, inset ~12pt on the leading edge (under the row
/// text) and flush to the trailing card edge. Uses a `.custom` NSBox so the
/// line is a true hairline — `NSBox.fillColor` takes the dynamic
/// `.separatorColor` directly, so it tracks dark mode without a CALayer color.
/// Replaces the default cross-axis fill constraint with fully-pinned
/// leading/trailing + a 1pt height (both edges pinned fully determine the
/// width, so no ballooning). Idempotent: re-pins on reconcile.
func ndStyleBoxedListDivider(_ sep: NSBox, in stack: NSStackView) {
    let id = ObjectIdentifier(sep)
    sep.boxType = .custom
    sep.borderWidth = 0
    sep.borderColor = .clear
    sep.fillColor = .separatorColor
    sep.titlePosition = .noTitle
    sep.contentViewMargins = .zero

    if let fill = ndCrossAxisConstraints[id] {
        fill.isActive = false
        ndCrossAxisConstraints[id] = nil
    }
    for pin in ndBoxedListDividerPins[id] ?? [] { pin.isActive = false }
    let lead = sep.leadingAnchor.constraint(equalTo: stack.leadingAnchor, constant: stack.edgeInsets.left + 12)
    let trail = sep.trailingAnchor.constraint(equalTo: stack.trailingAnchor, constant: -stack.edgeInsets.right)
    let height = sep.heightAnchor.constraint(equalToConstant: 1)
    for c in [lead, trail, height] { c.priority = NSLayoutConstraint.Priority(999) }
    NSLayoutConstraint.activate([lead, trail, height])
    ndBoxedListDividerPins[id] = [lead, trail, height]
}

/// Restores a separator to a full-bleed system line when its box drops
/// `boxed-list` (set-replace) — reverts the box type and lets
/// `ndBoxChildAttached` reinstall the default fill constraint.
func ndUnstyleBoxedListDivider(_ sep: NSBox, in stack: NSStackView) {
    let id = ObjectIdentifier(sep)
    for pin in ndBoxedListDividerPins[id] ?? [] { pin.isActive = false }
    ndBoxedListDividerPins[id] = nil
    sep.boxType = .separator
    sep.fillColor = .clear
    ndBoxChildAttached(stack, sep)
}

/// Undoes `ndBoxChildAttached`'s cross-axis constraint and, if no remaining
/// arranged subview still expands along the stack's main axis, restores the
/// `.gravityAreas` distribution the stack was created with.
func ndBoxChildDetached(_ stack: NSStackView, _ child: NSView) {
    if let existing = ndCrossAxisConstraints[ObjectIdentifier(child)] {
        existing.isActive = false
        ndCrossAxisConstraints[ObjectIdentifier(child)] = nil
    }

    // A row (or a wrapper box carrying rows) removed from somewhere inside a
    // `nd-native-sidebar` box: drop the direct child from the row set and
    // reload the source-list table (SidebarTable.swift) — same enclosing-
    // table walk as the attach arm above, since `stack` need not be the
    // classed box itself.
    if let table = ndEnclosingSidebarTable(stack) {
        ndSidebarRowButtons.remove(ObjectIdentifier(child))
        table.reload()
    }

    let vertical = stack.orientation == .vertical
    let stillExpanding = stack.arrangedSubviews.contains { view in
        let flags = ndLayoutFlags[ObjectIdentifier(view)] ?? NDLayoutFlags()
        return vertical ? flags.vexpand : flags.hexpand
    }
    if !stillExpanding {
        stack.distribution = .gravityAreas
    }
}

/// Re-runs `ndBoxChildAttached` for every arranged subview — used when the
/// stack's own `edgeInsets` change (padding re-applied), since the "fill"
/// cross-axis constraint's constant bakes those insets in.
func ndBoxReconcileChildren(_ stack: NSStackView) {
    for child in stack.arrangedSubviews {
        ndBoxChildAttached(stack, child)
    }
}
