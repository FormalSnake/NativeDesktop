import AppKit
import SwiftUI

struct NDGaugeBody: View {
    var value: Double
    var lower: Double
    var upper: Double
    var label: String

    var body: some View {
        Gauge(value: value, in: lower...upper) {
            Text(label)
        } currentValueLabel: {
            Text(value, format: .number.precision(.fractionLength(0)))
        }
        .gaugeStyle(.accessoryCircularCapacity)
        .padding()
    }
}

@MainActor
private func gauge(_ view: UnsafeMutableRawPointer) -> NSHostingView<NDGaugeBody> {
    Unmanaged<NSHostingView<NDGaugeBody>>.fromOpaque(view).takeUnretainedValue()
}

/// Returns a +1 retained `NSHostingView`; the caller releases it.
@MainActor @_cdecl("nd_swift_gauge_create")
public func ndSwiftGaugeCreate(
    _ value: Double, _ lower: Double, _ upper: Double, _ label: UnsafePointer<CChar>
) -> UnsafeMutableRawPointer {
    let body = NDGaugeBody(value: value, lower: lower, upper: upper, label: String(cString: label))
    return Unmanaged.passRetained(NSHostingView(rootView: body)).toOpaque()
}

@MainActor @_cdecl("nd_swift_gauge_update")
public func ndSwiftGaugeUpdate(
    _ view: UnsafeMutableRawPointer, _ value: Double, _ lower: Double, _ upper: Double,
    _ label: UnsafePointer<CChar>
) {
    gauge(view).rootView = NDGaugeBody(value: value, lower: lower, upper: upper, label: String(cString: label))
}

@MainActor @_cdecl("nd_swift_gauge_measure")
public func ndSwiftGaugeMeasure(
    _ view: UnsafeMutableRawPointer, _ width: UnsafeMutablePointer<Double>,
    _ height: UnsafeMutablePointer<Double>
) {
    let size = gauge(view).fittingSize
    width.pointee = size.width
    height.pointee = size.height
}
