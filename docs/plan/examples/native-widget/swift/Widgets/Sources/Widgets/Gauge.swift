import AppKit
import NDKit
import SwiftUI

struct GaugeProps: Codable, Equatable {
    var value: Double
    var min: Double
    var max: Double
    var label: String
}

/// SwiftUI's Gauge has no AppKit counterpart, so it is hosted. Apple restyles it with the OS.
@MainActor
enum GaugeWidget: NDSwiftWidget {
    static let name = "Gauge"

    static func make(props: GaugeProps, emit: NDEmitter<Never>) -> NSView {
        NSHostingView(rootView: GaugeBody(props: props))
    }

    static func update(_ view: NSView, props: GaugeProps) {
        (view as? NSHostingView<GaugeBody>)?.rootView = GaugeBody(props: props)
    }

    static func value(_ view: NSView) -> NDSemanticValue? {
        guard let props = (view as? NSHostingView<GaugeBody>)?.rootView.props else { return nil }
        return .range(value: props.value, min: props.min, max: props.max)
    }
}

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

#ndExport(GaugeWidget.self)
