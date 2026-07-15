import AppKit

/// DatePicker (M15): NSDatePicker restricted to `.yearMonthDay` and pinned to
/// a UTC Gregorian calendar, so the ISO `YYYY-MM-DD` wire value round-trips
/// with no timezone day-drift (the one date-anchor rule, enforced identically
/// on both backends — Date is an instant; an unpinned local calendar shifts
/// the day near midnight). min/maxDate clamp NATIVELY here: NSDatePicker
/// refuses/clamps out-of-range values before its action fires, so the clamped
/// date is what JS hears — the same contract the GTK backend implements by
/// hand in cbCalendarDaySelected.

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

/// Read by EventDispatcher.fireDateText (Events.swift).
func ndDatePickerISO(_ picker: NSDatePicker) -> String {
    ndISOFromDate(picker.dateValue)
}

/// `ndCreate`'s DatePicker arm (generated) calls this.
func makeDatePicker(_ props: [String: Any]) -> NSView {
    let picker = NSDatePicker()
    picker.datePickerMode = .single
    picker.datePickerElements = [.yearMonthDay]
    // displayStyle "field" = HIG Textual (limited space), "calendar" = Graphical.
    picker.datePickerStyle = (propStr(props, "displayStyle") ?? "calendar") == "field"
        ? .textFieldAndStepper : .clockAndCalendar
    var cal = Calendar(identifier: .gregorian)
    cal.timeZone = ndUTC
    picker.calendar = cal
    picker.timeZone = ndUTC
    if let v = propStr(props, "value"), let d = ndDateFromISO(v) {
        picker.dateValue = d
    } else {
        // Default: today, re-anchored to UTC midnight so the first emitted
        // value round-trips exactly.
        picker.dateValue = ndDateFromISO(ndISOFromDate(Date())) ?? Date()
    }
    ndDatePickerSetLimits(picker, propStr(props, "minDate"), propStr(props, "maxDate"))
    return picker
}

/// Generated ndApplyProps DatePicker.value arm: compare at DAY granularity
/// (ISO string equality after the UTC pin) inside the echo guard.
func ndDatePickerSetValue(_ view: NSView, _ iso: String) {
    guard let picker = view as? NSDatePicker, let d = ndDateFromISO(iso) else { return }
    guard ndDatePickerISO(picker) != iso else { return }
    withEchoSuppressed(view) { picker.dateValue = d }
}

/// min/maxDate merge (diffed updates carry only the changed key; the empty
/// string clears a bound — GTK ndCalendarSetLimits parity). NSDatePicker
/// clamps its own dateValue when a new bound excludes it.
func ndDatePickerSetLimits(_ view: NSView, _ minISO: String?, _ maxISO: String?) {
    guard let picker = view as? NSDatePicker else { return }
    if let s = minISO { picker.minDate = s.isEmpty ? nil : ndDateFromISO(s) }
    if let s = maxISO { picker.maxDate = s.isEmpty ? nil : ndDateFromISO(s) }
}
