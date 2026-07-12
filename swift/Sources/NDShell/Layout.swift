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
/// non-zero padding switches to it — a real AppKit bezel style change, not a
/// layer hack.
final class NDButton: NSButton {
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
/// represent. "start" already matches the stack's own `.leading`/`.centerY`
/// alignment, so no constraint is needed. "center"/"end" have no v1 mapping.
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

    let align = (vertical ? flags.halign : flags.valign) ?? "fill"
    switch align {
    case "fill":
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
        break // stack alignment (.leading/.centerY) already covers this.
    default:
        FileHandle.standardError.write(
            "ND_WARN halign/valign \"\(align)\" is unimplemented on AppKit v1 (only fill/start supported)\n".data(using: .utf8)!)
    }
}

/// Undoes `ndBoxChildAttached`'s cross-axis constraint and, if no remaining
/// arranged subview still expands along the stack's main axis, restores the
/// `.gravityAreas` distribution the stack was created with.
func ndBoxChildDetached(_ stack: NSStackView, _ child: NSView) {
    if let existing = ndCrossAxisConstraints[ObjectIdentifier(child)] {
        existing.isActive = false
        ndCrossAxisConstraints[ObjectIdentifier(child)] = nil
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
