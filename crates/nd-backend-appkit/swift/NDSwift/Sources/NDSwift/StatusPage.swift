import AppKit
import SwiftUI

struct NDStatusPageBody: View {
    var title: String
    var symbol: String?
    var detail: String?
    var compact: Bool

    var body: some View {
        ContentUnavailableView {
            if let symbol {
                Label(title, systemImage: symbol)
            } else {
                Text(title)
            }
        } description: {
            if let detail {
                Text(detail)
            }
        }
        .controlSize(compact ? .small : .regular)
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
    return Unmanaged.passRetained(NSHostingView(rootView: body)).toOpaque()
}

@MainActor @_cdecl("nd_swift_status_page_update")
public func ndSwiftStatusPageUpdate(
    _ view: UnsafeMutableRawPointer, _ title: UnsafePointer<CChar>, _ symbol: UnsafePointer<CChar>?,
    _ detail: UnsafePointer<CChar>?, _ compact: Bool
) {
    statusPage(view).rootView = NDStatusPageBody(
        title: String(cString: title), symbol: string(symbol), detail: string(detail), compact: compact)
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
