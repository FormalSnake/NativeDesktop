import AppKit

/// AppKit peer of AdwClamp (`<clamp>`): the single child fills the available
/// width up to `maximumSize`, then stays centered. The width cap is a hard
/// constraint; the fill is a high-but-breakable equality so narrow windows
/// keep edge-to-edge content exactly like AdwClamp below its threshold.
/// (`tighteningThreshold`'s gradual easing has no constraint-system
/// equivalent; the cap+center behavior is the part apps depend on.)
final class NDClampView: NSView {
    override nonisolated var isFlipped: Bool { true }
    var maximumSize: CGFloat = 600
    private var child: NSView?

    func setClampChild(_ view: NSView) {
        child?.removeFromSuperview()
        child = view
        addSubview(view)
        view.translatesAutoresizingMaskIntoConstraints = false
        let fill = view.widthAnchor.constraint(equalTo: widthAnchor)
        // Below windowSizeStayPut (500): at 750 the solver preferred shrinking
        // the WINDOW to the required maximumSize cap over leaving this equality
        // unsatisfied (measured: defaultWidth 720 launched at the cap and
        // resizes snapped back). Above content hugging (250) so the child
        // still stretches edge-to-edge in narrow windows.
        fill.priority = NSLayoutConstraint.Priority(490)
        NSLayoutConstraint.activate([
            view.topAnchor.constraint(equalTo: topAnchor),
            view.bottomAnchor.constraint(equalTo: bottomAnchor),
            view.centerXAnchor.constraint(equalTo: centerXAnchor),
            view.widthAnchor.constraint(lessThanOrEqualToConstant: maximumSize),
            view.leadingAnchor.constraint(greaterThanOrEqualTo: leadingAnchor),
            fill,
        ])
    }

    func clearClampChild(_ view: NSView) {
        guard child === view else { return }
        view.removeFromSuperview()
        child = nil
    }
}

func makeClamp(_ props: [String: Any]) -> NSView {
    let clamp = NDClampView()
    clamp.maximumSize = CGFloat(propInt(props, "maximumSize") ?? 600)
    // tighteningThreshold is accepted but has no AppKit mapping (see class doc).
    return clamp
}
