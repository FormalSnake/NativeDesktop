import AppKit
import SwiftUI

// Shared base for AppKit leaf widgets whose body is SwiftUI hosted in an
// NSHostingView (the Charts.swift seam, generalized). The point is that Apple
// owns the chrome: a `LabeledContent` row picks up System Settings metrics,
// accent response and light/dark for free, and keeps picking them up as the
// design language moves, which hand-built AppKit stacks do not.
//
// The hazard this file exists to close: the universal props are implemented
// generically against NSView/NSControl (`enabled` -> NSControl.isEnabled,
// `tooltip` -> view.toolTip, `focus` -> makeFirstResponder), and the
// automation a11y probe reads enabled/focused/value back off the same
// concrete AppKit classes. An NSHostingView is neither an NSControl nor the
// control the user sees, so a naive migration would leave `enabled` painting
// nothing, `focus` landing on a container, and the probe reading a shape the
// drives do not expect. NDHostedLeaf threads all three into SwiftUI state
// instead, and the generic arms in tools/codegen.ts route through it.
//
// Drag and drop needs no seam here: DragDrop.swift attaches a pan recognizer
// and a transparent NDDropZone subview to any NSView, and a hosting view is
// one.

/// Universal-prop state a hosted leaf's SwiftUI body observes. One instance
/// per leaf, owned by the NSView the core tracks.
@MainActor final class NDLeafState: ObservableObject {
    @Published var enabled = true
    @Published var tooltip = ""
    /// Bumped by `focusLeaf()`. The body keys its `@FocusState` write off the
    /// change, which is the only way to move SwiftUI focus from AppKit.
    @Published var focusToken = 0
    /// Written back by the body. Deliberately NOT @Published: it is read by
    /// the a11y probe, and republishing it from inside a body update would
    /// re-enter the same update.
    var swiftUIFocused = false
}

/// The universal props, applied as SwiftUI modifiers around a leaf's content.
struct NDLeafChrome<Content: View>: View {
    @ObservedObject var state: NDLeafState
    @FocusState private var focused: Bool
    let content: Content

    var body: some View {
        let base = content
            .disabled(!state.enabled)
            .focused($focused)
            .onChange(of: state.focusToken) { _, _ in focused = true }
            .onChange(of: focused) { _, now in state.swiftUIFocused = now }
        // `.help("")` still opens an empty tooltip box, so the modifier is
        // applied only when there is a tip to show.
        Group {
            if state.tooltip.isEmpty { base } else { base.help(state.tooltip) }
        }
    }
}

/// One React-owned `NSView` placed inside a SwiftUI body. Identity, props and
/// event wiring stay on the original instance — SwiftUI only positions it.
/// This is the ONLY thing crossing into SwiftUI from the React tree: a leaf
/// hosts its own chrome and the app's single child control, never an
/// arbitrary subtree.
struct NDNativeChild: NSViewRepresentable {
    let view: NSView
    /// Track controls (NSSlider and friends) report no size of their own and
    /// would collapse to nothing; 0 means "size yourself".
    var minWidth: CGFloat = 0

    func makeNSView(context: Context) -> NSView { view }
    func updateNSView(_ nsView: NSView, context: Context) {}
}

/// Whether a child can size itself horizontally. Same predicate the AppKit
/// row used before this migration, kept so a slider in a Row suffix still
/// gets a System Settings-sized track.
@MainActor func ndChildNeedsWidth(_ child: NSView) -> Bool {
    child.intrinsicContentSize.width == NSView.noIntrinsicMetric && child.fittingSize.width <= 0
}

/// A second way to reach the universal-prop seam besides subclassing
/// NDHostedLeaf: a widget that needs its OWN NSView wrapper for something
/// NSHostingView can't do itself (Banner's reveal-height clip animation)
/// still owns an `NDLeafState` and wraps its content in `NDLeafChrome`
/// directly, then conforms here so `ndHostedLeafSetEnabled` and friends
/// find it the same way they find a real NDHostedLeaf.
@MainActor protocol NDLeafChromeHosting: AnyObject {
    var leafState: NDLeafState { get }
}

/// Base class for a widget whose body is SwiftUI. Subclasses override
/// `leafContent()` and call `refreshLeaf()` after any prop change; everything
/// universal is handled here.
class NDHostedLeaf: NSHostingView<AnyView>, NDLeafChromeHosting {
    let leafState = NDLeafState()
    var ndNodeID: UInt32 = 0

    required init(rootView: AnyView) {
        super.init(rootView: rootView)
        // Height comes from the SwiftUI body, width from whatever lays the
        // leaf out (a Form row, a box). Low hugging is what lets the second
        // half happen: with the default the ideal width would win and a row
        // would refuse to fill its card. Compression resistance stays at the
        // AppKit default, not low: a low one let a Form row's siblings squash
        // the control down to nothing.
        sizingOptions = [.intrinsicContentSize]
        setContentHuggingPriority(.defaultLow, for: .horizontal)
        setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("NDHostedLeaf is not NSCoding-decodable") }

    convenience init() { self.init(rootView: AnyView(EmptyView())) }

    /// Subclass hook: the leaf's content, rebuilt whole on every prop change
    /// (Charts' update path — SwiftUI diffs, so there is nothing to reconcile
    /// by hand).
    func leafContent() -> AnyView { AnyView(EmptyView()) }

    final func refreshLeaf() {
        rootView = AnyView(NDLeafChrome(state: leafState, content: leafContent()))
        invalidateIntrinsicContentSize()
        // The enclosing boxes measure this leaf once per dirty cycle, so a
        // body that resized itself has to say so. Hooked here rather than in
        // invalidateIntrinsicContentSize: NSHostingView calls that from inside
        // its own layout, and dirtying the chain from there schedules a fresh
        // pass on every pass.
        ndInvalidateBoxChain(from: self)
    }

    /// `a11y` value for this leaf, as a JSON fragment. Widgets backed by a
    /// real AppKit control inside the body leave this alone — the probe reads
    /// the control, so both halves of a setValue round-trip keep agreeing.
    var ndA11yValueJSON: String { "null" }
}

// ============================================================================
// Generated-dispatcher seam. The generic `enabled` / `tooltip` / `focus` arms
// in tools/codegen.ts call these before falling through to their NSView
// handling.
// ============================================================================

/// The `enabled` prop for a hosted leaf: `.disabled()` is what dims a SwiftUI
/// control AND stops it taking events. `ndDisabledViews` is still the
/// bookkeeping the a11y probe reads, so the caller records it either way.
@MainActor func ndHostedLeafSetEnabled(_ view: NSView, _ on: Bool) -> Bool {
    guard let host = view as? NDLeafChromeHosting else { return false }
    host.leafState.enabled = on
    return true
}

/// The `tooltip` prop for a hosted leaf: `.help()` rather than `view.toolTip`,
/// because the hosting view's tracking area does not cover controls SwiftUI
/// draws inside it.
@MainActor func ndHostedLeafSetTooltip(_ view: NSView, _ tip: String) -> Bool {
    guard let host = view as? NDLeafChromeHosting else { return false }
    host.leafState.tooltip = tip
    return true
}

/// The `focus` command for a hosted leaf. The view becomes first responder so
/// the window's focus is inside this leaf, then the token bump moves
/// SwiftUI's own `@FocusState` onto the content.
@MainActor func ndHostedLeafFocus(_ view: NSView) -> Bool {
    guard let host = view as? NDLeafChromeHosting else { return false }
    if let window = view.window ?? ndWindow(for: view) { window.makeFirstResponder(view) }
    host.leafState.focusToken &+= 1
    return true
}

/// Whether SwiftUI reports focus inside this leaf. The probe's first-responder
/// walk already catches controls SwiftUI backs with a real NSView; this covers
/// the ones it does not.
@MainActor func ndHostedLeafFocused(_ view: NSView) -> Bool {
    (view as? NDLeafChromeHosting)?.leafState.swiftUIFocused ?? false
}
