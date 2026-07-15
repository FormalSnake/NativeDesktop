import AppKit

/// NumberInput: the HIG composite of "a stepper sits next to a field
/// that displays its current value". One tracked NSStackView handle holding
/// an NSTextField (NumberFormatter enforces digits + range) and an NSStepper
/// (min/max/increment/wraps), kept in two-way sync. Both inputs funnel into
/// ONE `valueChanged`; the field emits on Enter and focus-out (GtkSpinButton
/// parity — typed text commits on activate, not per keystroke). Programmatic
/// `value` writes ride the standard withEchoSuppressed guard, probed at emit
/// time via ndIsEchoSuppressed.
final class NDNumberInputView: NSStackView, NSTextFieldDelegate {
    var nodeID: UInt32 = 0
    let field = NSTextField()
    let stepper = NSStepper()
    private var lastEmitted: Double

    init(value: Double, min: Double, max: Double, step: Double, digits: Int, wraps: Bool) {
        lastEmitted = value
        super.init(frame: .zero)
        orientation = .horizontal
        spacing = 3
        alignment = .centerY

        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        formatter.minimumFractionDigits = digits
        formatter.maximumFractionDigits = digits
        formatter.minimum = NSNumber(value: min)
        formatter.maximum = NSNumber(value: max)
        field.formatter = formatter
        field.alignment = .right
        field.doubleValue = value
        field.delegate = self
        field.target = self
        field.action = #selector(fieldActivated(_:))
        field.widthAnchor.constraint(greaterThanOrEqualToConstant: 64).isActive = true

        stepper.minValue = min
        stepper.maxValue = max
        stepper.increment = step
        stepper.valueWraps = wraps
        stepper.autorepeat = true
        stepper.doubleValue = value
        stepper.target = self
        stepper.action = #selector(stepperChanged(_:))

        addArrangedSubview(field)
        addArrangedSubview(stepper)
    }

    required init?(coder: NSCoder) { fatalError("NDNumberInputView is not NSCoding-decodable") }

    @objc private func stepperChanged(_ sender: NSStepper) {
        field.doubleValue = sender.doubleValue
        emitIfChanged(sender.doubleValue)
    }

    @objc private func fieldActivated(_ sender: NSTextField) { syncFromField() }

    func controlTextDidEndEditing(_ obj: Notification) { syncFromField() }

    private func syncFromField() {
        // The formatter already rejects out-of-range commits; clamp defensively
        // so the stepper can never drift outside its own range.
        let clamped = Swift.min(stepper.maxValue, Swift.max(stepper.minValue, field.doubleValue))
        if abs(field.doubleValue - clamped) > 1e-9 { field.doubleValue = clamped }
        stepper.doubleValue = clamped
        emitIfChanged(clamped)
    }

    private func emitIfChanged(_ v: Double) {
        guard abs(v - lastEmitted) > 1e-9 else { return }
        lastEmitted = v
        guard !ndIsEchoSuppressed(self) else { return }
        ndEmitEvent(nodeID, "valueChanged", "{\"value\":\(v)}")
    }

    /// React-driven `value` update: both subviews in one write, no emit
    /// (programmatic writes never fire target/action; lastEmitted moves so a
    /// later user edit back to the old value still emits).
    func setValueProgrammatically(_ v: Double) {
        guard abs(stepper.doubleValue - v) > 1e-9 || abs(field.doubleValue - v) > 1e-9 else { return }
        field.doubleValue = v
        stepper.doubleValue = v
        lastEmitted = v
    }
}

/// `ndCreate`'s NumberInput arm (generated) calls this.
func makeNumberInput(_ props: [String: Any]) -> NSView {
    NDNumberInputView(
        value: propDouble(props, "value") ?? 0,
        min: propDouble(props, "min") ?? 0,
        max: propDouble(props, "max") ?? 100,
        step: propDouble(props, "step") ?? 1,
        digits: propInt(props, "digits") ?? 0,
        wraps: propBool(props, "wraps") ?? false
    )
}

/// Generated ndApplyProps NumberInput.value arm.
func ndNumberInputSetValue(_ view: NSView, _ v: Double) {
    (view as? NDNumberInputView)?.setValueProgrammatically(v)
}

/// Generated ndConnectEvents NumberInput arm.
func ndNumberInputConnect(_ view: NSView, nodeID: UInt32) {
    (view as? NDNumberInputView)?.nodeID = nodeID
}
