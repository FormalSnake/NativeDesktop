import AppKit
import UniformTypeIdentifiers

/// Window dialog commands: showAlert (NSAlert sheet), openFile/saveFile
/// (NSOpenPanel/NSSavePanel sheets), showAbout (the standard about panel).
/// Results fire the Window node's alertResult/openFileResult/saveFileResult
/// events, the same widgetCommand -> same-node result-event pattern as
/// executeJavaScript -> javaScriptResult, mirroring src/gtk/dialogs.zig.
///
/// Multi-window correctness: every sheet resolves the owning NSWindow from
/// the Window node's OWN handle via ndWindow(for:) (SplitController.swift),
/// never a global, so two windows can each run their own dialog.

nonisolated(unsafe) private var ndWindowNodeIDs: [ObjectIdentifier: UInt32] = [:]

/// Generated ndConnectEvents Window arm: stash the node id on the handle so
/// `ndWindowCommand` (which gets no node id of its own) can address results.
func ndWindowDialogsConnect(_ view: NSView, nodeID: UInt32) {
    ndWindowNodeIDs[ObjectIdentifier(view)] = nodeID
}

/// Generated ndWidgetCommand Window arm.
func ndWindowCommand(_ view: NSView, _ command: String, _ argJson: String) {
    let arg = parseProps(argJson)
    switch command {
    case "showAlert": ndCmdShowAlert(view, arg)
    case "openFile": ndCmdOpenFile(view, arg)
    case "saveFile": ndCmdSaveFile(view, arg)
    case "showAbout": ndCmdShowAbout(arg)
    default:
        FileHandle.standardError.write("ND_WARN unknown Window command \(command)\n".data(using: .utf8)!)
    }
}

private func ndWindowNodeID(_ view: NSView) -> UInt32? {
    ndWindowNodeIDs[ObjectIdentifier(view)]
}

/// release_node purge seam (Backend.swift's `ndPurgeNodeRegistries`).
func ndWindowDialogsPurge(_ view: NSView) {
    ndWindowNodeIDs[ObjectIdentifier(view)] = nil
}

/// data-payload emit through JSONSerialization (paths and labels are
/// arbitrary strings; hand-rolled escaping isn't worth the risk here).
private func ndEmitData(_ nodeID: UInt32, _ name: String, _ payload: [String: Any]) {
    guard JSONSerialization.isValidJSONObject(["data": payload]),
          let data = try? JSONSerialization.data(withJSONObject: ["data": payload]),
          let json = String(data: data, encoding: .utf8) else { return }
    ndEmitEvent(nodeID, name, json)
}

// MARK: - showAlert
// arg: { title, body?, buttons: [{ id, label, style?: "default"|"suggested"|
// "destructive" }], defaultId?, closeId? } -> alertResult { buttonId }.
// HIG: verb-titled buttons; Enter fires the suggested/default response only
// (never defaulted onto a destructive one — the app opts in via defaultId);
// Esc fires the close response; destructive buttons carry
// hasDestructiveAction. NSAlert lays buttons out per platform convention
// (first added = rightmost/primary).

private func ndCmdShowAlert(_ view: NSView, _ arg: [String: Any]) {
    guard let nodeID = ndWindowNodeID(view) else { return }
    let alert = NSAlert()
    alert.messageText = propStr(arg, "title") ?? ""
    if let body = propStr(arg, "body") { alert.informativeText = body }

    var ids: [String] = []
    var suggestedIdx: Int? = nil
    let buttons = (arg["buttons"] as? [Any])?.compactMap { $0 as? [String: Any] } ?? []
    for (i, spec) in buttons.enumerated() {
        let id = spec["id"] as? String ?? "button\(i)"
        let label = spec["label"] as? String ?? id
        let button = alert.addButton(withTitle: label)
        // Clear NSAlert's automatic Return on the first button — the default
        // is an explicit opt-in below (suggested style or defaultId), GTK parity.
        button.keyEquivalent = ""
        ids.append(id)
        if let style = spec["style"] as? String {
            if style == "destructive" {
                button.hasDestructiveAction = true
            } else if style == "suggested", suggestedIdx == nil {
                suggestedIdx = i
            }
        }
    }
    var defaultIdx = suggestedIdx
    if let defaultId = propStr(arg, "defaultId"), let idx = ids.firstIndex(of: defaultId) {
        defaultIdx = idx
    }
    if let idx = defaultIdx, idx < alert.buttons.count {
        alert.buttons[idx].keyEquivalent = "\r"
    }
    let closeId = propStr(arg, "closeId")
    if let closeId, let idx = ids.firstIndex(of: closeId), idx < alert.buttons.count {
        alert.buttons[idx].keyEquivalent = "\u{1b}"
    }

    let finish: (NSApplication.ModalResponse) -> Void = { response in
        let idx = response.rawValue - NSApplication.ModalResponse.alertFirstButtonReturn.rawValue
        let buttonId = (idx >= 0 && idx < ids.count) ? ids[idx] : (closeId ?? ids.first ?? "")
        ndEmitData(nodeID, "alertResult", ["buttonId": buttonId])
    }
    if let window = ndWindow(for: view) {
        alert.beginSheetModal(for: window, completionHandler: finish)
    } else {
        finish(alert.runModal())
    }
}

// MARK: - openFile / saveFile
// openFile arg: { multiple?, directories?, filters?: [{ name, extensions[] }] }
//   -> openFileResult { canceled, paths: string[] }
// saveFile arg: { suggestedName?, defaultDir?, filters? }
//   -> saveFileResult { canceled, path: string|null }

private func ndApplyFilters(_ panel: NSSavePanel, _ arg: [String: Any]) {
    guard let filters = (arg["filters"] as? [Any])?.compactMap({ $0 as? [String: Any] }), !filters.isEmpty else { return }
    var types: [UTType] = []
    for filter in filters {
        for ext in (filter["extensions"] as? [Any])?.compactMap({ $0 as? String }) ?? [] {
            if let t = UTType(filenameExtension: ext) { types.append(t) }
        }
    }
    if !types.isEmpty { panel.allowedContentTypes = types }
}

private func ndCmdOpenFile(_ view: NSView, _ arg: [String: Any]) {
    guard let nodeID = ndWindowNodeID(view) else { return }
    let panel = NSOpenPanel()
    let directories = propBool(arg, "directories") ?? false
    panel.canChooseFiles = !directories
    panel.canChooseDirectories = directories
    panel.allowsMultipleSelection = propBool(arg, "multiple") ?? false
    ndApplyFilters(panel, arg)

    let finish: (NSApplication.ModalResponse) -> Void = { response in
        let canceled = response != .OK
        let paths = canceled ? [] : panel.urls.map(\.path)
        ndEmitData(nodeID, "openFileResult", ["canceled": canceled, "paths": paths])
    }
    if let window = ndWindow(for: view) {
        panel.beginSheetModal(for: window, completionHandler: finish)
    } else {
        panel.begin(completionHandler: finish)
    }
}

private func ndCmdSaveFile(_ view: NSView, _ arg: [String: Any]) {
    guard let nodeID = ndWindowNodeID(view) else { return }
    let panel = NSSavePanel()
    if let name = propStr(arg, "suggestedName") { panel.nameFieldStringValue = name }
    if let dir = propStr(arg, "defaultDir") { panel.directoryURL = URL(fileURLWithPath: dir) }
    ndApplyFilters(panel, arg)

    let finish: (NSApplication.ModalResponse) -> Void = { response in
        let canceled = response != .OK
        let path: Any = (!canceled ? panel.url?.path : nil) ?? NSNull()
        ndEmitData(nodeID, "saveFileResult", ["canceled": canceled, "path": path])
    }
    if let window = ndWindow(for: view) {
        panel.beginSheetModal(for: window, completionHandler: finish)
    } else {
        panel.begin(completionHandler: finish)
    }
}

// MARK: - showAbout
// arg: { appName, version?, developer?, website? } — fire-and-forget (the
// standard about panel is app-modal and ignores the issuing window; routed
// through the Window node purely for one dispatch path, same as GTK).

private func ndCmdShowAbout(_ arg: [String: Any]) {
    var options: [NSApplication.AboutPanelOptionKey: Any] = [:]
    if let name = propStr(arg, "appName") { options[.applicationName] = name }
    if let version = propStr(arg, "version") { options[.applicationVersion] = version }
    var creditLines: [String] = []
    if let developer = propStr(arg, "developer") { creditLines.append(developer) }
    if let website = propStr(arg, "website") { creditLines.append(website) }
    if !creditLines.isEmpty {
        let credits = NSMutableAttributedString(
            string: creditLines.joined(separator: "\n"),
            attributes: [.font: NSFont.systemFont(ofSize: NSFont.smallSystemFontSize),
                         .foregroundColor: NSColor.labelColor])
        if let website = propStr(arg, "website"), let url = URL(string: website),
           let range = credits.string.range(of: website) {
            credits.addAttribute(.link, value: url, range: NSRange(range, in: credits.string))
        }
        options[.credits] = credits
    }
    NSApp.orderFrontStandardAboutPanel(options: options)
}
