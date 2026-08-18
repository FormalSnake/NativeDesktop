import AppKit
import Charts
import SwiftUI

/// Chart: SwiftUI **Swift Charts** in an NSHostingView. Swift Charts IS the
/// macOS charting framework, so the widget hands it the data and lets it own
/// axes, ticks, legend placement and the light/dark + accent response; the
/// GTK backend draws the same six types with Cairo because GNOME has no
/// equivalent library (src/gtk/chart.zig).
///
/// Data model: the app owns `series` — the widget never sorts, filters or
/// rewrites points (Table's rows contract). Every prop change rebuilds the
/// SwiftUI body from the parsed model, which is the whole update path.
///
/// Colour: a series' explicit `color` wins; otherwise slot 0 is the live
/// system accent and the rest walk the system palette, so a one-series chart
/// is accent-coloured. Candlesticks ignore that and use the up/down
/// convention (system green / system red) unless the series names a colour.

struct NDChartPoint: Identifiable {
    let id: Int
    let x: Double
    let y: Double
    let label: String?
    let open: Double
    let high: Double
    let low: Double
    let close: Double
    let hasOHLC: Bool
}

struct NDChartSeries: Identifiable {
    let id: Int
    let seriesId: String
    let label: String
    /// Legend key: `label` made unique, since two series sharing a label
    /// would collapse into one entry of the foreground-style scale.
    let key: String
    let color: Color?
    let points: [NDChartPoint]
}

struct NDChartSlice: Identifiable {
    let id: Int
    let name: String
    let value: Double
}

/// Slot 0 is the accent; the rest are system colours, which already carry
/// their own light/dark and increased-contrast variants.
private let ndChartPalette: [Color] = [
    Color(nsColor: .systemOrange),
    Color(nsColor: .systemGreen),
    Color(nsColor: .systemPurple),
    Color(nsColor: .systemYellow),
    Color(nsColor: .systemRed),
    Color(nsColor: .systemBrown),
]

func ndChartPaletteColor(_ index: Int) -> Color {
    if index == 0 { return Color(nsColor: .controlAccentColor) }
    return ndChartPalette[(index - 1) % ndChartPalette.count]
}

struct NDChartBody: View {
    var kind: String
    var series: [NDChartSeries]
    var xLabel: String
    var yLabel: String
    var showLegend: Bool
    var showGrid: Bool
    var animated: Bool
    /// Bumped on every apply so `.animation(_:value:)` has something to key
    /// the transition off.
    var revision: Int
    var onSelect: (Int, Int) -> Void

    var body: some View {
        Group {
            if kind == "pie" {
                pieChart
            } else {
                cartesianChart
            }
        }
        .chartLegend(showLegend ? .visible : .hidden)
        .animation(motion, value: revision)
        .padding(4)
    }

    /// The OS switch outranks the prop, the rule <skeleton> set (GTK peer:
    /// gtk-enable-animations).
    private var motion: Animation? {
        guard animated, !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion else { return nil }
        return .easeOut(duration: 0.35)
    }

    // MARK: - pie

    private var slices: [NDChartSlice] {
        guard let first = series.first else { return [] }
        return first.points.enumerated().map { idx, p in
            NDChartSlice(id: idx, name: p.label ?? "\(idx + 1)", value: abs(p.y))
        }
    }

    /// A pie reads ONE series: a second angular scale in the same plot is not
    /// a chart, it is two charts (SectorMark takes a single angle scale).
    private var pieChart: some View {
        Chart(slices) { slice in
            SectorMark(angle: .value("Value", slice.value), angularInset: 1.5)
                .cornerRadius(3)
                .foregroundStyle(by: .value("Slice", slice.name))
        }
        .chartForegroundStyleScale(domain: slices.map(\.name), mapping: sliceColor)
    }

    private func sliceColor(_ name: String) -> Color {
        ndChartPaletteColor(slices.firstIndex { $0.name == name } ?? 0)
    }

    // MARK: - cartesian

    private var cartesianChart: some View {
        Chart {
            ForEach(series) { s in
                ForEach(s.points) { p in
                    mark(p, s)
                }
            }
        }
        .chartForegroundStyleScale(domain: series.map(\.key), mapping: seriesColor)
        .chartXAxis {
            AxisMarks {
                if showGrid { AxisGridLine() }
                AxisTick()
                AxisValueLabel()
            }
        }
        .chartYAxis {
            AxisMarks {
                if showGrid { AxisGridLine() }
                AxisTick()
                AxisValueLabel()
            }
        }
        .chartXAxisLabel { if !xLabel.isEmpty { Text(xLabel) } }
        .chartYAxisLabel { if !yLabel.isEmpty { Text(yLabel) } }
        .chartOverlay { proxy in
            GeometryReader { geo in
                Rectangle()
                    .fill(.clear)
                    .contentShape(Rectangle())
                    // DragGesture with no minimum distance is the tap
                    // recognizer that also reports where the tap landed.
                    .gesture(DragGesture(minimumDistance: 0).onEnded { drag in
                        select(at: drag.location, proxy: proxy, geo: geo)
                    })
            }
        }
    }

    private func seriesColor(_ key: String) -> Color {
        guard let s = series.first(where: { $0.key == key }) else { return ndChartPaletteColor(0) }
        return s.color ?? ndChartPaletteColor(s.id)
    }

    private func mark(_ p: NDChartPoint, _ s: NDChartSeries) -> AnyChartContent {
        let x = PlottableValue.value(xLabel.isEmpty ? "X" : xLabel, p.x)
        let y = PlottableValue.value(yLabel.isEmpty ? "Y" : yLabel, p.y)
        let by = PlottableValue.value("Series", s.key)
        switch kind {
        case "area":
            return AnyChartContent(
                AreaMark(x: x, y: y)
                    .foregroundStyle(by: by)
                    .interpolationMethod(.catmullRom))
        case "bar":
            return AnyChartContent(
                BarMark(x: x, y: y)
                    .foregroundStyle(by: by)
                    .cornerRadius(3))
        case "scatter":
            return AnyChartContent(
                PointMark(x: x, y: y)
                    .foregroundStyle(by: by))
        case "candlestick":
            return AnyChartContent(candle(p, s))
        default:
            return AnyChartContent(
                LineMark(x: x, y: y)
                    .foregroundStyle(by: by)
                    .interpolationMethod(.catmullRom))
        }
    }

    /// One candle = a RuleMark wick spanning low…high plus a RectangleMark
    /// body spanning open…close, the composition Swift Charts documents for
    /// financial plots (it ships no candlestick mark).
    private func candle(_ p: NDChartPoint, _ s: NDChartSeries) -> some ChartContent {
        let tint = s.color ?? Color(nsColor: p.close >= p.open ? .systemGreen : .systemRed)
        let x = PlottableValue.value("X", p.x)
        return ChartContentBuilder.buildBlock(
            RuleMark(
                x: x,
                yStart: .value("Low", p.low),
                yEnd: .value("High", p.high)
            ).foregroundStyle(tint),
            RectangleMark(
                x: x,
                yStart: .value("Open", p.open),
                yEnd: .value("Close", p.close),
                width: .ratio(0.5)
            ).foregroundStyle(tint)
        )
    }

    // MARK: - selection

    private func select(at location: CGPoint, proxy: ChartProxy, geo: GeometryProxy) {
        guard let anchor = proxy.plotFrame else { return }
        let plot = geo[anchor]
        let local = CGPoint(x: location.x - plot.minX, y: location.y - plot.minY)

        var best: (series: Int, point: Int)?
        var bestDistance = 24.0 * 24.0
        for s in series {
            for p in s.points {
                let value = p.hasOHLC ? p.close : p.y
                guard let px = proxy.position(forX: p.x), let py = proxy.position(forY: value) else { continue }
                let dx = px - local.x
                let dy = py - local.y
                let d = dx * dx + dy * dy
                if d <= bestDistance {
                    bestDistance = d
                    best = (s.id, p.id)
                }
            }
        }
        if let best { onSelect(best.series, best.point) }
    }
}

final class NDChartView: NSHostingView<NDChartBody> {
    var nodeID: UInt32 = 0
    private var kind = "line"
    private var series: [NDChartSeries] = []
    private var xLabel = ""
    private var yLabel = ""
    private var showLegend = true
    private var showGrid = true
    private var animated = true
    private var revision = 0

    required init(rootView: NDChartBody) {
        super.init(rootView: rootView)
        // A chart fills the space it is given; leaving the standard sizing
        // options on would pin it to SwiftUI's ideal size instead.
        sizingOptions = []
        // The height floor is the schema's Chart.minContentHeight, applied in
        // makeChart; only the width one is fixed here.
        widthAnchor.constraint(greaterThanOrEqualToConstant: 160).isActive = true
        // The accent colour can change while the app runs; the palette reads
        // it at build time, so rebuild when it does (GTK peer: the
        // AdwStyleManager "notify" watch).
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(systemColorsChanged),
            name: NSColor.systemColorsDidChangeNotification,
            object: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("NDChartView is not NSCoding-decodable") }

    deinit { NotificationCenter.default.removeObserver(self) }

    @objc private func systemColorsChanged() { refresh(bumpRevision: false) }

    /// Takes parsed values, not the props dictionary: `[String: Any]` is not
    /// Sendable, and this class inherits NSHostingView's main-actor isolation.
    func apply(
        kind: String?, series: [NDChartSeries]?, xLabel: String?, yLabel: String?,
        showLegend: Bool?, showGrid: Bool?, animated: Bool?
    ) {
        if let kind { self.kind = kind }
        if let series { self.series = series }
        if let xLabel { self.xLabel = xLabel }
        if let yLabel { self.yLabel = yLabel }
        if let showLegend { self.showLegend = showLegend }
        if let showGrid { self.showGrid = showGrid }
        if let animated { self.animated = animated }
        refresh(bumpRevision: true)
    }

    private func refresh(bumpRevision: Bool) {
        if bumpRevision { revision &+= 1 }
        rootView = NDChartBody(
            kind: kind,
            series: series,
            xLabel: xLabel,
            yLabel: yLabel,
            showLegend: showLegend,
            showGrid: showGrid,
            animated: animated,
            revision: revision,
            onSelect: { [weak self] seriesIndex, pointIndex in
                self?.emitSelection(seriesIndex, pointIndex)
            })
    }

    private func emitSelection(_ seriesIndex: Int, _ pointIndex: Int) {
        guard seriesIndex < series.count else { return }
        let s = series[seriesIndex]
        guard pointIndex < s.points.count else { return }
        let p = s.points[pointIndex]
        ndEmitEvent(nodeID, "pointSelected",
                    "{\"data\":{\"seriesId\":\(ndJsonString(s.seriesId)),\"seriesIndex\":\(seriesIndex),\"index\":\(pointIndex),\"x\":\(p.x),\"y\":\(p.y)}}")
    }
}

/// Duplicate labels get trailing spaces rather than a numeric suffix: the
/// legend still reads right, and the scale domain stays unique.
func ndChartParseSeries(_ raw: [[String: Any]]) -> [NDChartSeries] {
    var usedKeys: Set<String> = []
    return raw.enumerated().map { index, obj in
        let label = propStr(obj, "label") ?? ""
        var key = label.isEmpty ? "Series \(index + 1)" : label
        while usedKeys.contains(key) { key += " " }
        usedKeys.insert(key)
        var color: Color?
        if let spec = propStr(obj, "color"), let nsColor = ndColorFromHex(spec) {
            color = Color(nsColor: nsColor)
        }
        let points = (obj["points"] as? [Any])?.compactMap { $0 as? [String: Any] } ?? []
        return NDChartSeries(
            id: index,
            seriesId: propStr(obj, "id") ?? "",
            label: label,
            key: key,
            color: color,
            points: points.enumerated().map(ndChartParsePoint))
    }
}

/// `open` + `close` together are what makes a point a candle; `high`/`low`
/// fall back to the body extremes so a partial OHLC still draws.
private func ndChartParsePoint(_ index: Int, _ obj: [String: Any]) -> NDChartPoint {
    let open = propDouble(obj, "open")
    let close = propDouble(obj, "close")
    let o = open ?? 0
    let c = close ?? 0
    return NDChartPoint(
        id: index,
        x: propDouble(obj, "x") ?? Double(index),
        y: propDouble(obj, "y") ?? 0,
        label: propStr(obj, "label"),
        open: o,
        high: propDouble(obj, "high") ?? max(o, c),
        low: propDouble(obj, "low") ?? min(o, c),
        close: c,
        hasOHLC: open != nil && close != nil)
}

/// `ndCreate`'s Chart arm (generated) calls this.
func makeChart(_ props: [String: Any], minContentHeight: Int) -> NSView {
    let view = NDChartView(rootView: NDChartBody(
        kind: "line", series: [], xLabel: "", yLabel: "",
        showLegend: true, showGrid: true, animated: true, revision: 0,
        onSelect: { _, _ in }))
    // Same floor mechanism as TextArea/ScrollView: the chart fills whatever it
    // is given (sizingOptions is empty), so inside a scroller it would measure
    // zero without a real constraint.
    if minContentHeight > 0 {
        view.frame.size.height = CGFloat(minContentHeight)
        let floor_ = view.heightAnchor.constraint(greaterThanOrEqualToConstant: CGFloat(minContentHeight))
        floor_.priority = NSLayoutConstraint.Priority(999)
        floor_.isActive = true
    }
    ndChartApply(view, props)
    return view
}

/// Generated ndApplyProps Chart arm — one merged apply, since the SwiftUI
/// body is rebuilt whole from the model on any key.
func ndChartApply(_ view: NSView, _ props: [String: Any]) {
    (view as? NDChartView)?.apply(
        kind: propStr(props, "type"),
        series: propObjArray(props, "series").map(ndChartParseSeries),
        xLabel: propStr(props, "xLabel"),
        yLabel: propStr(props, "yLabel"),
        showLegend: propBool(props, "showLegend"),
        showGrid: propBool(props, "showGrid"),
        animated: propBool(props, "animated"))
}

/// Generated ndConnectEvents Chart arm.
func ndChartConnect(_ view: NSView, nodeID: UInt32) {
    (view as? NDChartView)?.nodeID = nodeID
}
