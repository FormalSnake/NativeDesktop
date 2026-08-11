import AppKit

// The single home for interface metric constants that were previously
// scattered as literals across Backend/CommandPalette/Toasts (HIG: nested
// corners stay concentric with their container; spacing and control density
// track the platform standard, not per-app numbers).

/// Platform-standard sibling spacing, resolved by the generated Box create/
/// apply arms when `spacing` is the -1 "platform default" sentinel (the GTK
/// peer resolves the same sentinel to 6).
let ndStandardSpacing: CGFloat = 8

/// Baseline corner radii for ND's own chrome surfaces — the values the
/// concentric derivation falls back to when no rounded container exists.
enum NDRadius {
    /// boxed-list grouped card (Backend.swift's ndApplyBoxedListCard).
    static let card: CGFloat = 10
    /// command palette card (CommandPalette.swift).
    static let palette: CGFloat = 12
    /// floating overlays: toasts, HUD panels (Toasts.swift).
    static let overlay: CGFloat = 14
}

/// Radius for a shape nested `inset` points inside a container whose corner
/// radius is `containerRadius`, kept concentric (inner = outer - inset,
/// floored at 0). macOS 27 ships first-class corner-concentricity APIs; the
/// macOS 26 SDK this package pins (Package.swift platforms) does not declare
/// them, so the arithmetic form is the implementation until the SDK moves —
/// every call site already routes through here, so adopting the system API
/// later is a change in this one file.
func ndConcentricRadius(containerRadius: CGFloat, inset: CGFloat) -> CGFloat {
    max(0, containerRadius - inset)
}

/// Concentric radius for `view` nested inside its nearest rounded ancestor:
/// reads that ancestor's effective layer corner radius and derives the inner
/// radius from the actual geometric inset between the two. Falls back to
/// `fallback` when no rounded ancestor exists (a top-level surface such as a
/// pane-hosted card keeps its own baseline curve).
func ndConcentricRadius(in view: NSView, fallback: CGFloat) -> CGFloat {
    var ancestor = view.superview
    while let container = ancestor {
        if let radius = container.layer?.cornerRadius, radius > 0 {
            container.layoutSubtreeIfNeeded()
            let f = view.convert(view.bounds, to: container)
            let inset = max(0, min(f.minX, f.minY,
                                   container.bounds.width - f.maxX,
                                   container.bounds.height - f.maxY))
            return ndConcentricRadius(containerRadius: radius, inset: inset)
        }
        ancestor = container.superview
    }
    return fallback
}

/// Window.density -> Tahoe's compact control-size metrics on the window's
/// LIVE content root (generated Window create arm). `standard` is the
/// baseline; anything else unknown degrades to it loudly.
func ndApplyDensity(_ win: NSWindow, _ density: String) {
    let compact: Bool
    switch density {
    case "compact": compact = true
    case "standard": compact = false
    default:
        FileHandle.standardError.write("ND_WARN unknown Window.density \(density)\n".data(using: .utf8)!)
        compact = false
    }
    win.contentView?.prefersCompactControlSizeMetrics = compact
}
