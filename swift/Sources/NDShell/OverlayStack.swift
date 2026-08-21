import AppKit

/// Z-stack container backing `<overlay>`: the first attached child is the
/// content, pinned to every edge so it sizes the overlay exactly as it would
/// size a plain parent. Every later child floats above it, positioned from
/// the halign/valign flags ndApplyStyle records per child (Layout.swift), the
/// same keys that position an overlay child on GTK. Constraints are built at
/// attach; the child's props (style included) land before the structural op,
/// so the flags are already recorded by then.
final class NDOverlayView: NSView {
    private(set) var contentChild: NSView?
    private var floatingConstraints: [ObjectIdentifier: [NSLayoutConstraint]] = [:]

    func addOverlayChild(_ child: NSView) {
        if child.superview == self { return } // no layer reorder, mirror the GTK arm
        child.translatesAutoresizingMaskIntoConstraints = false
        addSubview(child)
        if contentChild == nil {
            contentChild = child
            let constraints = [
                child.leadingAnchor.constraint(equalTo: leadingAnchor),
                child.trailingAnchor.constraint(equalTo: trailingAnchor),
                child.topAnchor.constraint(equalTo: topAnchor),
                child.bottomAnchor.constraint(equalTo: bottomAnchor),
            ]
            NSLayoutConstraint.activate(constraints)
            floatingConstraints[ObjectIdentifier(child)] = constraints
        } else {
            constrainFloating(child)
        }
    }

    func removeOverlayChild(_ child: NSView) {
        if child == contentChild { contentChild = nil }
        if let constraints = floatingConstraints.removeValue(forKey: ObjectIdentifier(child)) {
            NSLayoutConstraint.deactivate(constraints)
        }
        child.removeFromSuperview()
    }

    /// fill (the default) pins both edges of an axis; start/end pick one edge;
    /// center centers. A floating bar with valign start and default halign
    /// therefore spans the full width at the top, GTK-overlay semantics
    /// without any positioning prop.
    private func constrainFloating(_ child: NSView) {
        let flags = ndLayoutFlags[ObjectIdentifier(child)] ?? NDLayoutFlags()
        var constraints: [NSLayoutConstraint] = []
        switch flags.halign ?? "fill" {
        case "start": constraints.append(child.leadingAnchor.constraint(equalTo: leadingAnchor))
        case "end": constraints.append(child.trailingAnchor.constraint(equalTo: trailingAnchor))
        case "center": constraints.append(child.centerXAnchor.constraint(equalTo: centerXAnchor))
        default:
            constraints.append(child.leadingAnchor.constraint(equalTo: leadingAnchor))
            constraints.append(child.trailingAnchor.constraint(equalTo: trailingAnchor))
        }
        switch flags.valign ?? "fill" {
        case "start": constraints.append(child.topAnchor.constraint(equalTo: topAnchor))
        case "end": constraints.append(child.bottomAnchor.constraint(equalTo: bottomAnchor))
        case "center": constraints.append(child.centerYAnchor.constraint(equalTo: centerYAnchor))
        default:
            constraints.append(child.topAnchor.constraint(equalTo: topAnchor))
            constraints.append(child.bottomAnchor.constraint(equalTo: bottomAnchor))
        }
        NSLayoutConstraint.activate(constraints)
        floatingConstraints[ObjectIdentifier(child)] = constraints
    }
}
