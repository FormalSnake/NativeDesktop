import AppKit
import SwiftUI

/// Mirrors the Rust `GaugeProps`; props cross as JSON.
struct GaugeProps: Codable, Equatable {
    var value: Double
    var min: Double
    var max: Double
    var label: String
}

/// SwiftUI's Gauge has no AppKit counterpart, so it is hosted. Apple restyles it with the OS.
// docs:begin swift-widget
struct GaugeBody: View {
    let props: GaugeProps

    var body: some View {
        Gauge(value: props.value, in: props.min...props.max) {
            Text(props.label)
        } currentValueLabel: {
            Text(props.value, format: .number.precision(.fractionLength(0)))
        }
        .gaugeStyle(.accessoryCircularCapacity)
    }
}

private func decode(_ json: UnsafePointer<CChar>) -> GaugeProps {
    let data = Data(String(cString: json).utf8)
    return (try? JSONDecoder().decode(GaugeProps.self, from: data))
        ?? GaugeProps(value: 0, min: 0, max: 100, label: "")
}

@MainActor
private func hosting(_ view: UnsafeMutableRawPointer) -> NSHostingView<GaugeBody> {
    Unmanaged<NSHostingView<GaugeBody>>.fromOpaque(view).takeUnretainedValue()
}

/// Returns a +1 retained `NSHostingView`; the caller releases it.
@MainActor @_cdecl("nd_widget_gauge_create")
public func placesGaugeCreate(_ props: UnsafePointer<CChar>) -> UnsafeMutableRawPointer {
    Unmanaged.passRetained(NSHostingView(rootView: GaugeBody(props: decode(props)))).toOpaque()
}

@MainActor @_cdecl("nd_widget_gauge_update")
public func placesGaugeUpdate(_ view: UnsafeMutableRawPointer, _ props: UnsafePointer<CChar>) {
    hosting(view).rootView = GaugeBody(props: decode(props))
}

@MainActor @_cdecl("nd_widget_gauge_measure")
public func placesGaugeMeasure(
    _ view: UnsafeMutableRawPointer, _ width: UnsafeMutablePointer<Double>,
    _ height: UnsafeMutablePointer<Double>
) {
    let size = hosting(view).fittingSize
    width.pointee = size.width
    height.pointee = size.height
}
// docs:end swift-widget
