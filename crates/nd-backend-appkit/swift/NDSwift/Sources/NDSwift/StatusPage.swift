import AppKit
import SwiftUI

/// Where the page's label and description sit inside `ContentUnavailableView`'s own frame.
@MainActor final class NDStatusPageInk {
    var top: CGFloat?
    var bottom: CGFloat?
}

struct NDStatusPageBody: View {
    var title: String
    var symbol: String?
    var detail: String?
    var compact: Bool
    /// The view's own margins and the room it keeps for actions, trimmed off so the page
    /// measures as its icon, title and description: the status page lays its actions out under
    /// it and centres the whole group, which with the margins kept sat off centre.
    var trim = EdgeInsets()
    var ink: NDStatusPageInk?

    private static let space = "nd.status-page"

    var body: some View {
        ContentUnavailableView {
            label.onGeometryChange(for: CGRect.self) { $0.frame(in: .named(Self.space)) } action: { [ink, detail] in
                ink?.top = $0.minY
                if detail == nil { ink?.bottom = $0.maxY }
            }
        } description: {
            if let detail {
                Text(detail).onGeometryChange(for: CGFloat.self) { $0.frame(in: .named(Self.space)).maxY } action: {
                    [ink] in ink?.bottom = $0
                }
            }
        }
        .controlSize(compact ? .small : .regular)
        .coordinateSpace(.named(Self.space))
        .padding(.top, -trim.top)
        .padding(.bottom, -trim.bottom)
    }

    @ViewBuilder private var label: some View {
        if let symbol {
            Label(title, systemImage: symbol)
        } else {
            Text(title)
        }
    }

    /// The same page with its margins measured and trimmed: laid out once at its natural size
    /// in a probe, since the margins are the OS's and change with it.
    @MainActor func trimmed() -> Self {
        let ink = NDStatusPageInk()
        var probe = self
        probe.trim = EdgeInsets()
        probe.ink = ink
        let view = NSHostingView(rootView: probe)
        let size = view.fittingSize
        view.frame = CGRect(origin: .zero, size: size)
        view.layoutSubtreeIfNeeded()
        var page = self
        page.ink = nil
        if let top = ink.top, let bottom = ink.bottom {
            page.trim = EdgeInsets(top: max(top, 0), leading: 0, bottom: max(size.height - bottom, 0), trailing: 0)
        }
        return page
    }
}

@MainActor
private func statusPage(_ view: UnsafeMutableRawPointer) -> NSHostingView<NDStatusPageBody> {
    Unmanaged<NSHostingView<NDStatusPageBody>>.fromOpaque(view).takeUnretainedValue()
}

private func string(_ pointer: UnsafePointer<CChar>?) -> String? {
    pointer.map { String(cString: $0) }
}

/// Returns a +1 retained `NSHostingView`; the caller releases it. `symbol` and `detail` may be null.
@MainActor @_cdecl("nd_swift_status_page_create")
public func ndSwiftStatusPageCreate(
    _ title: UnsafePointer<CChar>, _ symbol: UnsafePointer<CChar>?, _ detail: UnsafePointer<CChar>?,
    _ compact: Bool
) -> UnsafeMutableRawPointer {
    let body = NDStatusPageBody(
        title: String(cString: title), symbol: string(symbol), detail: string(detail), compact: compact)
    return Unmanaged.passRetained(NSHostingView(rootView: body.trimmed())).toOpaque()
}

@MainActor @_cdecl("nd_swift_status_page_update")
public func ndSwiftStatusPageUpdate(
    _ view: UnsafeMutableRawPointer, _ title: UnsafePointer<CChar>, _ symbol: UnsafePointer<CChar>?,
    _ detail: UnsafePointer<CChar>?, _ compact: Bool
) {
    statusPage(view).rootView = NDStatusPageBody(
        title: String(cString: title), symbol: string(symbol), detail: string(detail), compact: compact
    ).trimmed()
}

/// The size the page takes at `width` (no width when negative): the description wraps to it.
@MainActor @_cdecl("nd_swift_status_page_measure")
public func ndSwiftStatusPageMeasure(
    _ view: UnsafeMutableRawPointer, _ width: Double, _ outWidth: UnsafeMutablePointer<Double>,
    _ outHeight: UnsafeMutablePointer<Double>
) {
    let hosting = statusPage(view)
    var size = hosting.fittingSize
    if width >= 0 {
        let fitted = NSHostingController(rootView: hosting.rootView)
            .sizeThatFits(in: CGSize(width: width, height: .infinity))
        if fitted.width.isFinite && fitted.height.isFinite {
            size = CGSize(width: min(fitted.width, width), height: fitted.height)
        }
    }
    outWidth.pointee = size.width
    outHeight.pointee = size.height
}
