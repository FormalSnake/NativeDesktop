import AppKit
import SwiftUI

/// DatePicker: SwiftUI `DatePicker`, pinned to a UTC Gregorian calendar so
/// the ISO `YYYY-MM-DD` wire value round-trips with no timezone day-drift
/// (the one date-anchor rule, enforced identically on both backends — Date
/// is an instant; an unpinned local calendar shifts the day near midnight).
/// `in:` supplies the min/maxDate clamp: SwiftUI's DatePicker refuses values
/// outside the range the same way NSDatePicker did, so the clamped date is
/// what JS hears, matching the contract the GTK backend implements by hand
/// in cbCalendarDaySelected.

private let ndUTC = TimeZone(identifier: "UTC")!

private let ndISOFormatter: DateFormatter = {
    let f = DateFormatter()
    f.dateFormat = "yyyy-MM-dd"
    f.timeZone = ndUTC
    f.locale = Locale(identifier: "en_US_POSIX")
    var cal = Calendar(identifier: .gregorian)
    cal.timeZone = ndUTC
    f.calendar = cal
    return f
}()

func ndDateFromISO(_ s: String) -> Date? {
    s.isEmpty ? nil : ndISOFormatter.date(from: s)
}

func ndISOFromDate(_ d: Date) -> String {
    ndISOFormatter.string(from: d)
}

private let ndUTCGregorian: Calendar = {
    var cal = Calendar(identifier: .gregorian)
    cal.timeZone = ndUTC
    return cal
}()

final class NDDatePickerView: NDHostedLeaf {
    private var date = Date()
    private var minDate: Date?
    private var maxDate: Date?
    private var graphical = true

    func applyCreate(value: String, displayStyle: String, minDate: String?, maxDate: String?) {
        graphical = displayStyle != "field"
        if let d = ndDateFromISO(value) {
            date = d
        } else {
            // Default: today, re-anchored to UTC midnight so the first
            // emitted value round-trips exactly.
            date = ndDateFromISO(ndISOFromDate(Date())) ?? Date()
        }
        applyLimits(minDate, maxDate)
        refreshLeaf()
    }

    /// min/maxDate merge (diffed updates carry only the changed key; the
    /// empty string clears a bound — GTK ndCalendarSetLimits parity).
    func applyLimits(_ minISO: String?, _ maxISO: String?) {
        if let s = minISO { minDate = s.isEmpty ? nil : ndDateFromISO(s) }
        if let s = maxISO { maxDate = s.isEmpty ? nil : ndDateFromISO(s) }
        // A new bound can exclude the current value — clamp it the same way
        // NSDatePicker did natively.
        if let minDate, date < minDate { date = minDate }
        if let maxDate, date > maxDate { date = maxDate }
        refreshLeaf()
    }

    /// Generated ndApplyProps DatePicker.value arm: compare at DAY
    /// granularity (ISO string equality after the UTC pin) inside the echo
    /// guard.
    func setValueFromProps(_ iso: String) {
        guard let d = ndDateFromISO(iso), ndISOFromDate(date) != iso else { return }
        withEchoSuppressed(self) { setDate(d, emit: false) }
    }

    private func setDate(_ d: Date, emit: Bool) {
        var clamped = d
        if let minDate, clamped < minDate { clamped = minDate }
        if let maxDate, clamped > maxDate { clamped = maxDate }
        date = clamped
        refreshLeaf()
        if emit, !ndIsEchoSuppressed(self) {
            ndEmitEvent(ndNodeID, "dateChanged", "{\"text\":\(ndJsonString(ndISOFromDate(clamped)))}")
        }
    }

    override func leafContent() -> AnyView {
        let binding = Binding<Date>(
            get: { [weak self] in self?.date ?? Date() },
            set: { [weak self] in self?.setDate($0, emit: true) })
        let picker: DatePicker<Text>
        if let minDate, let maxDate {
            picker = DatePicker("", selection: binding, in: minDate...maxDate, displayedComponents: .date)
        } else if let minDate {
            picker = DatePicker("", selection: binding, in: minDate..., displayedComponents: .date)
        } else if let maxDate {
            picker = DatePicker("", selection: binding, in: ...maxDate, displayedComponents: .date)
        } else {
            picker = DatePicker("", selection: binding, displayedComponents: .date)
        }
        // .stepperField (not .field) to match the old .textFieldAndStepper
        // AppKit style exactly. The two styles resolve to different concrete
        // types, so the branch has to wrap the whole styled view, not just
        // pick between two style values.
        let styled: AnyView = graphical
            ? AnyView(picker.datePickerStyle(.graphical))
            : AnyView(picker.datePickerStyle(.stepperField))
        return AnyView(
            styled
                .environment(\.calendar, ndUTCGregorian)
                .environment(\.timeZone, ndUTC)
                .labelsHidden())
    }

    override var ndA11yValueJSON: String { ndJsonString(ndISOFromDate(date)) }
}

/// `ndCreate`'s DatePicker arm (generated) calls this.
func makeDatePicker(_ props: [String: Any]) -> NSView {
    let picker = NDDatePickerView()
    // displayStyle "field" = HIG Textual (limited space), "calendar" = Graphical.
    picker.applyCreate(
        value: propStr(props, "value") ?? "",
        displayStyle: propStr(props, "displayStyle") ?? "calendar",
        minDate: propStr(props, "minDate"),
        maxDate: propStr(props, "maxDate"))
    return picker
}

/// Generated ndApplyProps DatePicker.value arm.
func ndDatePickerSetValue(_ view: NSView, _ iso: String) {
    (view as? NDDatePickerView)?.setValueFromProps(iso)
}

/// Generated ndApplyProps DatePicker.minDate/maxDate arm.
func ndDatePickerSetLimits(_ view: NSView, _ minISO: String?, _ maxISO: String?) {
    (view as? NDDatePickerView)?.applyLimits(minISO, maxISO)
}

/// Generated ndConnectEvents DatePicker arm.
func ndDatePickerConnect(_ view: NSView, nodeID: UInt32) {
    (view as? NDDatePickerView)?.ndNodeID = nodeID
}
