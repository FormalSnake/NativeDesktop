import AppKit
import CNd

/// Payload shape for an emitted event's JSON body (peer of `protocol.zig`'s
/// `EventPayload` / `tools/codegen.ts`'s `PayloadKind`). `ndConnectEvents`
/// (generated) picks the kind per widget.event; `EventDispatcher` builds the
/// actual JSON from the live control value at fire time.
enum PayloadKind {
    case none, text, checked, value, index
}

/// One view's wiring: the node it belongs to, the NDP event name to emit,
/// and how to build the payload. Keyed by event name so a single view (e.g.
/// TextInput) can carry two independent wirings (`changed` + `activate`).
private struct Wiring {
    let nodeID: UInt32
    let name: String
    let payload: PayloadKind
}

/// Target/action dispatcher for every AppKit control (peer of GTK's
/// `emitEventAdapter` -> `nd_emit_event`). AppKit has one target/action pair
/// per NSControl, so controls with a single event (Button, Checkbox, Radio,
/// Select, Slider, the ListView table) wire target/action directly here.
/// TextInput/TextArea need two independent triggers (continuous `changed`
/// via delegate, Enter-only `activate` via action), so `wire` installs a
/// delegate for those kinds instead of overwriting `.action`.
final class EventDispatcher: NSObject {
    // ABI contract: every call into this dispatcher arrives on the
    // embedder's UI thread (control target/action, delegate callbacks, and
    // `ndConnectEvents` itself all fire main-thread-only), so the shared
    // singleton is safe in practice even though `EventDispatcher` isn't
    // `Sendable` — same `nonisolated(unsafe)` reasoning as main.swift's
    // `gVTable`/`gCtx`.
    nonisolated(unsafe) static let shared = EventDispatcher()

    // (view identity, event name) -> wiring. A view can hold >1 entry
    // (TextInput: "changed" + "activate").
    private var wiring: [ObjectIdentifier: [String: Wiring]] = [:]
    // Views currently being mutated by a React-driven ndApplyProps update —
    // suppress the echo event that mutation would otherwise fire (peer of
    // GTK's blockEcho/unblockEcho).
    var suppressed: Set<ObjectIdentifier> = []

    /// Called by the generated `ndConnectEvents`. `action` is the selector
    /// the emitter picked (`fireNone`/`fireText`/`fireChecked`/`fireValue`/
    /// `fireIndex`) — used for plain NSControl target/action wiring; the
    /// TextInput/TextArea delegate paths below re-derive the right fire
    /// method themselves from `payload`/`name` since a delegate callback
    /// carries no selector of its own.
    func wire(_ view: NSView, nodeID: UInt32, name: String, payload: PayloadKind, action: Selector) {
        let key = ObjectIdentifier(view)
        wiring[key, default: [:]][name] = Wiring(nodeID: nodeID, name: name, payload: payload)

        if let field = view as? NSTextField {
            // `changed` must fire continuously (every keystroke); target/action
            // on NSTextField only fires on Enter (that's `activate`). Install a
            // delegate for continuous change notifications and keep target/
            // action for `activate`.
            field.delegate = TextFieldEventBridge.shared
            if name == "activate" {
                field.target = self
                field.action = #selector(fireText(_:))
            }
            return
        }
        if let scrollView = view as? NSScrollView, let textView = scrollView.documentView as? NSTextView {
            // TextArea: tracked handle is the NSScrollView, but the delegate
            // lives on the NSTextView inside it. `changed` is the only event.
            textView.delegate = TextViewEventBridge.shared
            return
        }
        if view is NSScrollView {
            // ListView: tracked handle is the NSScrollView wrapping the
            // NSTableView (M6b-D2). Row selection is reported through
            // `ListViewDataSource.tableViewSelectionDidChange` ->
            // `wiringFireIndex`, not target/action (NSScrollView isn't an
            // NSControl) — the wiring entry recorded above is all that's
            // needed; nothing further to install here.
            return
        }
        if let control = view as? NSControl {
            control.target = self
            control.action = action
            return
        }
    }

    /// Called by `ListViewDataSource.tableViewSelectionDidChange` (row
    /// selection has no NSControl target/action path since the tracked
    /// handle is the NSScrollView, not the NSTableView).
    func wiringFireIndex(_ scrollView: NSScrollView, index: Int) {
        guard let name = soleEventName(scrollView) else { return }
        emit(scrollView, name: name, json: jsonObject(["index": .int(index)]))
    }

    private func wiring(for view: NSView, name: String) -> Wiring? {
        wiring[ObjectIdentifier(view)]?[name]
    }

    private func emit(_ view: NSView, name: String, json: String) {
        let key = ObjectIdentifier(view)
        guard !suppressed.contains(key), let w = wiring(for: view, name: name) else { return }
        json.withCString { cJson in
            name.withCString { cName in
                nd_emit_event(gCtx, w.nodeID, cName, cJson)
            }
        }
    }

    // MARK: - NSControl target/action fire methods (one per PayloadKind)

    @objc func fireNone(_ sender: NSControl) {
        guard let name = soleEventName(sender) else { return }
        emit(sender, name: name, json: "{}")
    }

    @objc func fireText(_ sender: NSControl) {
        // TextInput's `activate` fires here (target/action, Enter key).
        guard let name = soleEventName(sender, preferring: "activate") else { return }
        let text = (sender as? NSTextField)?.stringValue ?? ""
        emit(sender, name: name, json: jsonObject(["text": .string(text)]))
    }

    @objc func fireChecked(_ sender: NSButton) {
        guard let name = soleEventName(sender) else { return }
        emit(sender, name: name, json: jsonObject(["checked": .bool(sender.state == .on)]))
    }

    @objc func fireValue(_ sender: NSSlider) {
        guard let name = soleEventName(sender) else { return }
        emit(sender, name: name, json: jsonObject(["value": .double(sender.doubleValue)]))
    }

    @objc func fireIndex(_ sender: NSControl) {
        guard let name = soleEventName(sender) else { return }
        let idx: Int
        if let pop = sender as? NSPopUpButton {
            idx = pop.indexOfSelectedItem
        } else {
            idx = 0
        }
        emit(sender, name: name, json: jsonObject(["index": .int(idx)]))
    }

    /// Most wired views have exactly one event name; `preferring` picks
    /// between TextInput's two (`changed`/`activate`) when both are present.
    private func soleEventName(_ view: NSView, preferring: String? = nil) -> String? {
        guard let byName = wiring[ObjectIdentifier(view)] else { return nil }
        if let preferring, byName[preferring] != nil { return preferring }
        return byName.keys.first
    }

    /// Fired by `TextFieldEventBridge`/`TextViewEventBridge` for continuous
    /// edits (`changed`), which have no NSControl target/action equivalent.
    /// Also called directly by `Automation.swift`'s `semanticSetValue`/
    /// `semanticType`: AppKit does not fire change notifications for
    /// programmatic `stringValue`/`string` writes (unlike GTK's
    /// `Editable.setText`/`TextBuffer.setText`, which fire "changed" as a
    /// side effect), so semantic input must replay this emit path itself.
    func fireChanged(_ view: NSView, text: String) {
        emit(view, name: "changed", json: jsonObject(["text": .string(text)]))
    }
}

/// `NSTextFieldDelegate` bridge: `controlTextDidChange` fires on every
/// keystroke, unlike target/action (Enter-only). Shared across all wired
/// TextInputs — the notification carries the originating field.
private final class TextFieldEventBridge: NSObject, NSTextFieldDelegate {
    nonisolated(unsafe) static let shared = TextFieldEventBridge()
    func controlTextDidChange(_ obj: Notification) {
        guard let field = obj.object as? NSTextField else { return }
        EventDispatcher.shared.fireChanged(field, text: field.stringValue)
    }
}

/// `NSTextViewDelegate` bridge for TextArea's `changed` event. The wired
/// view (per `EventDispatcher.wiring`) is the outer `NSScrollView`, not the
/// `NSTextView` itself — `textDidChange` hands us the text view, so we walk
/// up to its enclosing scroll view to find the wiring key.
private final class TextViewEventBridge: NSObject, NSTextViewDelegate {
    nonisolated(unsafe) static let shared = TextViewEventBridge()
    func textDidChange(_ notification: Notification) {
        guard let textView = notification.object as? NSTextView,
              let scrollView = textView.enclosingScrollView else { return }
        EventDispatcher.shared.fireChanged(scrollView, text: textView.string)
    }
}

// MARK: - minimal JSON value builder (payloads are small, fixed-shape objects)

private enum JSONValue {
    case string(String)
    case bool(Bool)
    case double(Double)
    case int(Int)
}

private func jsonObject(_ fields: [String: JSONValue]) -> String {
    var parts: [String] = []
    for (k, v) in fields {
        switch v {
        case .string(let s):
            let escaped = s
                .replacingOccurrences(of: "\\", with: "\\\\")
                .replacingOccurrences(of: "\"", with: "\\\"")
                .replacingOccurrences(of: "\n", with: "\\n")
            parts.append("\"\(k)\":\"\(escaped)\"")
        case .bool(let b):
            parts.append("\"\(k)\":\(b)")
        case .double(let d):
            parts.append("\"\(k)\":\(d)")
        case .int(let i):
            parts.append("\"\(k)\":\(i)")
        }
    }
    return "{" + parts.joined(separator: ",") + "}"
}

/// Called by the generated `ndApplyProps` update guards: mutate a control's
/// value without echoing a change event back to React (the AppKit peer of
/// GTK's `blockEcho`/`unblockEcho`). Re-entrancy-safe for nested calls to
/// the same view (rare, but cheap to support with a plain insert/remove
/// pair since AppKit mutations here are synchronous, not reentrant across
/// suppress scopes in practice).
func withEchoSuppressed(_ view: NSView, _ body: () -> Void) {
    let key = ObjectIdentifier(view)
    let wasSuppressed = EventDispatcher.shared.suppressed.contains(key)
    EventDispatcher.shared.suppressed.insert(key)
    body()
    if !wasSuppressed { EventDispatcher.shared.suppressed.remove(key) }
}

/// Radio group membership (`ObjectIdentifier(button) -> group name`),
/// populated by the generated `ndCreate`'s Radio arm. AppKit auto-excludes
/// radio buttons that share the same target + superview; since every wired
/// Radio shares `EventDispatcher.shared` as its target, buttons within the
/// same superview already mutually exclude regardless of `group` — this map
/// exists so a future cross-container grouping need (radios in different
/// superviews sharing one logical group) has a place to hang additional
/// logic without a schema/ABI change. Read but not required by v1's
/// single-superview groups.
nonisolated(unsafe) var radioGroupIdentifier: [ObjectIdentifier: String] = [:]
