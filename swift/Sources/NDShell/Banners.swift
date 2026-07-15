import AppKit
import SwiftUI

/// Banner: SwiftUI chrome in an NSHostingView. It's a content-layer strip,
/// so system colors only and NO Liquid Glass (HIG: glass belongs to the
/// control/navigation layer). `revealed` animates a clipping height
/// constraint via NSAnimationContext (slide/clip, not fade) to match
/// AdwBanner's built-in reveal animation.
struct NDBannerBody: View {
    var title: String
    var buttonLabel: String?
    var onButton: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Text(title)
                .fontWeight(.medium)
                .lineLimit(1)
            Spacer(minLength: 0)
            if let buttonLabel, !buttonLabel.isEmpty {
                Button(buttonLabel, action: onButton)
                    .buttonStyle(.bordered)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 7)
        .background(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(Color(nsColor: .controlAccentColor).opacity(0.16))
        )
    }
}

final class NDBannerView: NSView {
    var nodeID: UInt32 = 0
    private var title = ""
    private var buttonLabel: String?
    private(set) var revealed = false
    private var hosting: NSHostingView<NDBannerBody>!
    private var heightConstraint: NSLayoutConstraint!

    override var isFlipped: Bool { true }

    init() {
        super.init(frame: .zero)
        clipsToBounds = true
        hosting = NSHostingView(rootView: NDBannerBody(title: "", buttonLabel: nil, onButton: {}))
        hosting.translatesAutoresizingMaskIntoConstraints = false
        addSubview(hosting)
        heightConstraint = heightAnchor.constraint(equalToConstant: 0)
        // No bottom pin: collapsing clips the fixed-height chrome instead of
        // squashing its layout mid-animation.
        NSLayoutConstraint.activate([
            hosting.leadingAnchor.constraint(equalTo: leadingAnchor),
            hosting.trailingAnchor.constraint(equalTo: trailingAnchor),
            hosting.topAnchor.constraint(equalTo: topAnchor),
            heightConstraint,
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("NDBannerView is not NSCoding-decodable") }

    func apply(title: String?, buttonLabel: String?, revealed: Bool?) {
        if let title { self.title = title }
        if let buttonLabel { self.buttonLabel = buttonLabel.isEmpty ? nil : buttonLabel }
        hosting.rootView = NDBannerBody(
            title: self.title,
            buttonLabel: self.buttonLabel,
            onButton: { [weak self] in
                guard let self else { return }
                ndEmitEvent(self.nodeID, "buttonClicked", "{}")
            }
        )
        if let revealed, revealed != self.revealed {
            setRevealed(revealed, animated: window != nil)
        } else if self.revealed {
            // Content changed while revealed: track the new fitting height.
            heightConstraint.constant = hosting.fittingSize.height
        }
    }

    private func setRevealed(_ r: Bool, animated: Bool) {
        revealed = r
        let target = r ? hosting.fittingSize.height : 0
        guard animated else {
            heightConstraint.constant = target
            return
        }
        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = 0.22
            ctx.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            ctx.allowsImplicitAnimation = true
            heightConstraint.animator().constant = target
            self.window?.layoutIfNeeded()
        }
    }
}

/// `ndCreate`'s Banner arm (generated) calls this.
func makeBanner(_ props: [String: Any]) -> NSView {
    let banner = NDBannerView()
    ndBannerApply(banner, props)
    return banner
}

/// Generated ndApplyProps Banner arm — merged apply (absent keys keep state).
func ndBannerApply(_ view: NSView, _ props: [String: Any]) {
    (view as? NDBannerView)?.apply(
        title: propStr(props, "title"),
        buttonLabel: propStr(props, "buttonLabel"),
        revealed: propBool(props, "revealed")
    )
}

/// Generated ndConnectEvents Banner arm.
func ndBannerConnect(_ view: NSView, nodeID: UInt32) {
    (view as? NDBannerView)?.nodeID = nodeID
}
