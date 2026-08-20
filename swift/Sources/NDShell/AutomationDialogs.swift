import Foundation

/// `ND_AUTOMATION_DIALOG_SCRIPT`: scripted answers for native dialogs so an
/// automated run never blocks on UI nobody is there to click. Peer of
/// `src/automation_dialogs.zig`, which serves the GTK host and the core's own
/// systemRequest/widgetCommand interceptors; those never consult
/// `webview.scriptDialog`, so the two readers cannot take from the same queue.
///
/// The env var carries inline JSON, or `@/path/to.json`, mapping a method name
/// to a FIFO of answers. An exhausted queue is a LOUD failure that still
/// answers: a page parked on `alert()` never runs another line of JavaScript,
/// which takes every later content script and automation eval down with it.
enum NDAutomationDialogs {
    enum Next {
        case unscripted
        case exhausted
        case response([String: Any])
    }

    nonisolated(unsafe) private static var queues: [String: [[String: Any]]] = [:]
    nonisolated(unsafe) private static var loaded = false

    static func take(_ method: String) -> Next {
        load()
        guard var queue = queues[method] else { return .unscripted }
        guard !queue.isEmpty else { return .exhausted }
        let head = queue.removeFirst()
        queues[method] = queue
        return .response(head)
    }

    private static func load() {
        guard !loaded else { return }
        loaded = true
        let environment = ProcessInfo.processInfo.environment
        guard environment["NATIVE_AUTOMATION"] == "1",
              let script = environment["ND_AUTOMATION_DIALOG_SCRIPT"], !script.isEmpty else { return }
        var data: Data?
        if script.hasPrefix("@") {
            data = FileManager.default.contents(atPath: String(script.dropFirst()))
        } else {
            data = script.data(using: .utf8)
        }
        guard let data, let parsed = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            FileHandle.standardError.write("ND_WARN ND_AUTOMATION_DIALOG_SCRIPT is not a JSON object\n".data(using: .utf8)!)
            return
        }
        for (method, entries) in parsed {
            queues[method] = (entries as? [Any] ?? []).compactMap { $0 as? [String: Any] }
        }
    }
}

/// One scripted answer to a page dialog, in the shape both webview backends
/// use: `accepted` plus the `text` a prompt was typed with.
struct NDScriptedDialogAnswer {
    let accepted: Bool
    let text: String

    /// Nil when the run scripted nothing for page dialogs, which is what keeps
    /// a real user's session showing real sheets.
    static func next() -> NDScriptedDialogAnswer? {
        switch NDAutomationDialogs.take("webview.scriptDialog") {
        case .unscripted:
            return nil
        case .exhausted:
            FileHandle.standardError.write(
                "ND_WARN WebView scriptDialog: the automation dialog script ran out of answers; dismissing\n"
                    .data(using: .utf8)!)
            return NDScriptedDialogAnswer(accepted: false, text: "")
        case .response(let fields):
            return NDScriptedDialogAnswer(
                accepted: (fields["accepted"] as? NSNumber)?.boolValue ?? true,
                text: fields["text"] as? String ?? ""
            )
        }
    }
}
