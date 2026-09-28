import AppKit

// Keys typed straight after an app menu item runs. The item's action reaches
// the app as an event and the app answers on a later turn, so a field it opens
// (a command bar on cmd+T) takes the keyboard only after the keys typed right
// behind the chord were dispatched: they went to the page, or to the window
// with nothing focused, and were lost. For a short while after a menu item
// fires, plain keys that would not land in a text field are held, and handed
// over in order once one takes the keyboard. If none does, they go where they
// were going.
@MainActor enum NDTypeAhead {
    /// How long after the item fires a field has to take the keyboard.
    private static let window: TimeInterval = 0.5
    private static var deadline: TimeInterval = 0
    private static var held: [NSEvent] = []
    private static var replaying = false
    private static var monitor: Any?
    private static var polling = false

    /// A menu item of the app's own just ran.
    static func arm() {
        install()
        deadline = ProcessInfo.processInfo.systemUptime + window
        poll()
    }

    private static func install() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .keyUp]) { event in
            nonisolated(unsafe) let held = event
            let pass = MainActor.assumeIsolated { NDTypeAhead.intercept(held) }
            return pass ? event : nil
        }
    }

    /// Whether `event` goes on now; one that does not is held.
    private static func intercept(_ event: NSEvent) -> Bool {
        if replaying { return true }
        let armed = ProcessInfo.processInfo.systemUptime < deadline
        guard armed || !held.isEmpty else { return true }
        // A chord is a command of its own, not typing.
        let chord = event.modifierFlags.intersection([.command, .control, .option])
        if !chord.isEmpty { return true }
        if textFieldHasKeyboard() {
            if held.isEmpty { return true }
            held.append(event)
            flush()
            return false
        }
        guard armed else { return true }
        held.append(event)
        return false
    }

    private static func textFieldHasKeyboard() -> Bool {
        guard let responder = NSApp.keyWindow?.firstResponder as? NSTextView else { return false }
        return responder.isFieldEditor
    }

    /// Checked on a short tick while armed: the field can take the keyboard
    /// with no key arriving to notice it.
    private static func poll() {
        guard !polling else { return }
        polling = true
        func tick() {
            if !held.isEmpty, textFieldHasKeyboard() { flush() }
            if ProcessInfo.processInfo.systemUptime >= deadline {
                // No field came: the keys go where they were going.
                if !held.isEmpty { flush() }
                polling = false
                return
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.016) { MainActor.assumeIsolated { tick() } }
        }
        tick()
    }

    /// Posted at the head of the queue, last first, so they run before
    /// anything that arrived behind them and in the order they were typed.
    private static func flush() {
        let events = held
        held.removeAll()
        replaying = true
        for event in events.reversed() { NSApp.postEvent(event, atStart: true) }
        // The monitor sees them again when they are dispatched, on later
        // turns; it lets them through until the queue has run them.
        DispatchQueue.main.async { MainActor.assumeIsolated { NDTypeAhead.replaying = false } }
    }
}
