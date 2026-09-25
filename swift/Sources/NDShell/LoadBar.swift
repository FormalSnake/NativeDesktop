import SwiftUI

// `<progressbar cssClasses={["osd"]}>`: the thin load bar GNOME Web draws with
// Adwaita's `progressbar.osd`, which on macOS was a full system progress bar.
// The fill slides to each new value, and once the value reaches 1 the bar
// fades out rather than vanishing, so a finished load reads as finished.

/// How long the fill takes to reach a new value, and the fade that follows a
/// finished load (short: a load bar is seen on every navigation).
private let ndLoadBarSlide = 0.25
private let ndLoadBarFade = 0.3
private let ndLoadBarFadeDelay = 0.2
let ndLoadBarThickness: CGFloat = 2

struct NDLoadBarView: View {
    let fraction: Double
    let restart: Bool
    var quiet = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var clamped: Double { min(max(fraction, 0), 1) }

    var body: some View {
        GeometryReader { geometry in
            Rectangle()
                .fill(quiet ? Color(nsColor: .tertiaryLabelColor) : Color.accentColor)
                .frame(width: geometry.size.width * CGFloat(clamped), height: ndLoadBarThickness)
                .animation(restart || reduceMotion ? nil : .easeOut(duration: ndLoadBarSlide), value: clamped)
                .opacity(clamped >= 1 ? 0 : 1)
                .animation(restart ? nil : .easeOut(duration: ndLoadBarFade).delay(ndLoadBarFadeDelay), value: clamped >= 1)
        }
        .frame(height: ndLoadBarThickness)
        // Under a full-size-content title bar the hosting view reports the
        // title bar's height as a top safe area, which would draw the bar that
        // far down the page instead of on its edge.
        .ignoresSafeArea()
        .accessibilityHidden(true)
    }
}
