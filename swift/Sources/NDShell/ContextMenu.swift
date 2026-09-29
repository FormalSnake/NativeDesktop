import AppKit

// The universal `contextMenu` prop and its `contextMenuSelected` event for the
// AppKit backend. Driven from one arm above the generated kind dispatch, like
// the drag/drop trio (tools/codegen.ts, UNIVERSAL_PROP_OWNERS): the menu goes
// on `NSView.menu`, which AppKit already shows for a right click and a
// control-click on any view and offers to VoiceOver as the view's Show Menu
// action, so no view class of the core's needs a `menu(for:)` override.

/// Target and delegate for one node's menu. Holds the node id the connect arm
/// hands over, which the props arm does not have at create time.
private final class NDContextMenuTarget: NSObject, NSMenuDelegate {
    var nodeID: UInt32 = 0

    @objc func pick(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? String else { return }
        ndEmitEvent(nodeID, "contextMenuSelected", "{\"text\":\"\(ndJsonEscape(id))\"}")
    }

    /// The open menu, one line per item, for a drive: the menu's tracking
    /// loop owns the main thread, so the automation socket cannot answer while
    /// it is up.
    func menuWillOpen(_ menu: NSMenu) {
        guard ndContextMenuTrace else { return }
        var lines = ""
        for (index, item) in menu.items.enumerated() {
            let kind = item.isSeparatorItem ? "separator" : "item"
            lines += "ND_CONTEXT_MENU node=\(nodeID) index=\(index) kind=\(kind) enabled=\(item.isEnabled ? 1 : 0) label=\(item.title)\n"
        }
        FileHandle.standardError.write(Data(lines.utf8))
    }
}

private let ndContextMenuTrace = ProcessInfo.processInfo.environment["NATIVE_AUTOMATION"] == "1"

nonisolated(unsafe) private var ndContextMenuTargets: [ObjectIdentifier: NDContextMenuTarget] = [:]

/// The entries as AppKit gets them: a separator only between two commands, so
/// an app that builds its list from optional groups never shows a leading,
/// trailing or doubled one.
private func ndContextMenuEntries(_ raw: [[String: Any]]) -> [[String: Any]] {
    var out: [[String: Any]] = []
    for entry in raw {
        if entry["separator"] as? Bool == true {
            if let last = out.last, last["separator"] as? Bool != true { out.append(entry) }
        } else if entry["id"] is String, entry["label"] is String {
            out.append(entry)
        }
    }
    if out.last?["separator"] as? Bool == true { out.removeLast() }
    return out
}

/// Universal props arm, called from both `ndCreate` and `ndApplyProps`. An
/// empty list takes the menu away.
func ndContextMenuApply(_ view: NSView, _ props: [String: Any]) {
    guard let raw = props["contextMenu"] as? [Any] else { return }
    let entries = ndContextMenuEntries(raw.compactMap { $0 as? [String: Any] })
    let key = ObjectIdentifier(view)
    guard !entries.isEmpty else {
        view.menu = nil
        return
    }
    let target = ndContextMenuTargets[key] ?? NDContextMenuTarget()
    ndContextMenuTargets[key] = target
    let menu = NSMenu()
    // An item's `enabled` is the app's to say; AppKit's validation would
    // otherwise enable every item that has a target.
    menu.autoenablesItems = false
    menu.delegate = target
    for entry in entries {
        if entry["separator"] as? Bool == true {
            menu.addItem(.separator())
            continue
        }
        let item = NSMenuItem(title: entry["label"] as? String ?? "", action: #selector(NDContextMenuTarget.pick(_:)), keyEquivalent: "")
        item.target = target
        item.representedObject = entry["id"] as? String
        item.isEnabled = entry["enabled"] as? Bool ?? true
        // Shown for the menu bar's binding; a context menu is not in the
        // key-equivalent chain, so this binds nothing a second time.
        if let spec = entry["accelerator"] as? String, let (key, mods) = ndParseAccelerator(spec) {
            item.keyEquivalent = key
            item.keyEquivalentModifierMask = mods
        }
        menu.addItem(item)
    }
    view.menu = menu
}

/// Universal connect arm: where the menu's target learns its node id.
func ndContextMenuConnect(_ view: NSView, nodeID: UInt32) {
    ndContextMenuTargets[ObjectIdentifier(view)]?.nodeID = nodeID
}

/// release_node purge seam (Backend.swift's `ndPurgeNodeRegistries`).
func ndContextMenuPurge(_ view: NSView) {
    ndContextMenuTargets[ObjectIdentifier(view)] = nil
    view.menu = nil
}
