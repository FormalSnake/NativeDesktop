#if canImport(CCef)
import AppKit
import CCef
import Foundation

/// Chromium's context-menu model, drawn as a real NSMenu.
///
/// Chrome style needs this: the browser's widget believes it lives in the Views
/// window the anchor still stands for, and Chromium's own Views menu is raised
/// against that widget's event stream. A right-click the window server routes to
/// the host's window never reaches it, so the menu is never presented at all.
/// Taking `run_context_menu` over puts the menu in the host's own window, where
/// the click actually landed.
///
/// `cef_menu_model_t` may not be referenced once `run_context_menu` returns, so
/// the model is copied into the owned tree below while the callback is still on
/// the stack, and the NSMenu is built from the copy. Peer of `src/cef/gtkmenu.zig`.

struct NDCefMenuEntry {
    enum Kind { case command, check, radio, separator, submenu }

    var label = ""
    var commandID: Int32 = -1
    var kind = Kind.command
    var enabled = true
    var checked = false
    var keyEquivalent = ""
    var modifiers: NSEvent.ModifierFlags = []
    var children: [NDCefMenuEntry] = []
}

/// One menu in flight, from `run_context_menu` to the pick or the dismissal.
/// The `cef_run_context_menu_callback_t` reference `run_context_menu` was handed
/// is this object's, and `answer` is the only place it is given back.
@MainActor final class NDCefContextMenuRun: NSObject {
    private var callback: UnsafeMutablePointer<cef_run_context_menu_callback_t>?
    private let menu: NSMenu
    private weak var view: NDCefWebView?
    private var tracking = false

    init?(entries: [NDCefMenuEntry], callback: UnsafeMutablePointer<cef_run_context_menu_callback_t>, view: NDCefWebView) {
        guard !entries.isEmpty else { return nil }
        self.callback = callback
        self.view = view
        menu = NSMenu()
        super.init()
        // A contextual menu asks its target to validate every item at display
        // time, which would overwrite what the model said about each one.
        menu.autoenablesItems = false
        build(entries, into: menu)
        guard !menu.items.isEmpty else { return nil }
    }

    private func build(_ entries: [NDCefMenuEntry], into menu: NSMenu) {
        for entry in entries {
            if entry.kind == .separator {
                menu.addItem(.separator())
                continue
            }
            let item = NSMenuItem(title: entry.label, action: nil, keyEquivalent: entry.keyEquivalent)
            item.keyEquivalentModifierMask = entry.modifiers
            item.isEnabled = entry.enabled
            if entry.kind == .submenu {
                let submenu = NSMenu(title: entry.label)
                submenu.autoenablesItems = false
                build(entry.children, into: submenu)
                item.submenu = submenu
            } else {
                item.target = self
                item.action = #selector(pick(_:))
                item.tag = Int(entry.commandID)
                // AppKit draws no radio mark of its own, so a selected radio
                // reads as a checked item, which is what Chrome's own mac menu
                // does with the same model.
                item.state = entry.checked ? .on : .off
            }
            menu.addItem(item)
        }
    }

    /// One trace line per item the user is actually looking at. AppKit publishes
    /// no accessibility element for a contextual menu, so the host's own report
    /// is the only handle a drive has on what was drawn. Peer of the Linux
    /// engine's `menuShown`.
    func trace(_ entries: [NDCefMenuEntry], depth: Int = 0) {
        for (index, entry) in entries.enumerated() {
            view?.ndTrace(
                "chrome menuShown depth=\(depth) index=\(index) id=\(entry.commandID) kind=\(entry.kind)"
                    + " enabled=\(entry.enabled ? 1 : 0) checked=\(entry.checked ? 1 : 0) label=\(entry.label)")
            if !entry.children.isEmpty { trace(entry.children, depth: depth + 1) }
        }
    }

    /// `point` is in the host view's own coordinates.
    func present(at point: NSPoint) {
        guard let view else {
            answer(command: 0)
            return
        }
        // Next run-loop turn on purpose: `popUp` runs a nested tracking loop,
        // and entering one from inside the CEF callback that produced the model
        // unwinds through a stack frame CEF still owns.
        DispatchQueue.main.async { [weak self] in
            guard let self, self.callback != nil else { return }
            self.tracking = true
            self.menu.popUp(positioning: nil, at: point, in: view)
            self.tracking = false
            // A pick has already answered by the time tracking ends; anything
            // else is a dismissal.
            self.answer(command: 0)
        }
    }

    /// The view went away, the page navigated, or the host is quitting.
    func cancel() {
        if tracking { menu.cancelTracking() }
        answer(command: 0)
    }

    @objc private func pick(_ sender: NSMenuItem) {
        answer(command: Int32(sender.tag))
    }

    /// Exactly once, whichever path gets here first.
    private func answer(command: Int32) {
        guard let callback else { return }
        self.callback = nil
        if command == 0 {
            callback.pointee.cancel?(callback)
        } else {
            callback.pointee.cont?(callback, command, EVENTFLAG_NONE)
        }
        nd_cef_ref_release(callback)
        view?.ndTrace("chrome menu answered command=\(command)")
        view?.forgetContextMenu(self)
    }
}

// MARK: - Copying the model

/// Runs on the CEF UI thread, where |model| is only valid for the length of the
/// callback that handed it over.
@MainActor func ndCefCopyMenuModel(
    _ model: UnsafeMutablePointer<cef_menu_model_t>, depth: Int = 0
) -> [NDCefMenuEntry] {
    guard depth <= 6, let count = model.pointee.get_count?(model) else { return [] }
    var out: [NDCefMenuEntry] = []
    for index in 0..<count {
        if model.pointee.is_visible_at?(model, index) == 0 { continue }
        let type = model.pointee.get_type_at?(model, index) ?? MENUITEMTYPE_COMMAND
        if type == MENUITEMTYPE_SEPARATOR {
            // Dropping items leaves rules with nothing on one side of them, so
            // one is only kept when something drawable precedes it.
            if out.isEmpty || out[out.count - 1].kind == .separator { continue }
            out.append(NDCefMenuEntry(kind: .separator))
            continue
        }
        let commandID = model.pointee.get_command_id_at?(model, index) ?? -1
        if ndCefMenuItemDenied(commandID) { continue }

        var children: [NDCefMenuEntry] = []
        if type == MENUITEMTYPE_SUBMENU {
            guard let submenu = model.pointee.get_sub_menu_at?(model, index) else { continue }
            children = ndCefCopyMenuModel(submenu, depth: depth + 1)
            nd_cef_ref_release(submenu)
            // A submenu whose every child was dropped is a dead label.
            if !children.contains(where: { $0.kind != .separator }) { continue }
        }

        var entry = NDCefMenuEntry(
            label: ndCefMenuLabel(model, index),
            commandID: commandID,
            kind: {
                switch type {
                case MENUITEMTYPE_SUBMENU: return .submenu
                case MENUITEMTYPE_CHECK: return .check
                case MENUITEMTYPE_RADIO: return .radio
                default: return .command
                }
            }(),
            enabled: model.pointee.is_enabled_at?(model, index) != 0,
            checked: model.pointee.is_checked_at?(model, index) != 0,
            children: children
        )
        let accelerator = ndCefMenuAccelerator(model, index)
        entry.keyEquivalent = accelerator.key
        entry.modifiers = accelerator.modifiers
        out.append(entry)
    }
    while let last = out.last, last.kind == .separator { out.removeLast() }
    return out
}

/// Chromium's labels carry Windows-style `&` mnemonics, which an NSMenu draws
/// literally. `&&` is the escaped ampersand.
private func ndCefMenuLabel(_ model: UnsafeMutablePointer<cef_menu_model_t>, _ index: Int) -> String {
    guard let raw = model.pointee.get_label_at?(model, index) else { return "" }
    defer { nd_cef_string_free(raw) }
    let text = ndCefString(raw)
    var out = ""
    var iterator = text.makeIterator()
    var pending: Character?
    while let character = pending ?? iterator.next() {
        pending = nil
        guard character == "&" else {
            out.append(character)
            continue
        }
        guard let next = iterator.next() else { break }
        if next == "&" { out.append("&") } else { pending = next }
    }
    return out
}

/// The accelerator the model reports, as an NSMenuItem key equivalent. Key codes
/// are Windows virtual keys; anything outside the set a context menu uses is
/// reported as no accelerator rather than guessed at.
private func ndCefMenuAccelerator(
    _ model: UnsafeMutablePointer<cef_menu_model_t>, _ index: Int
) -> (key: String, modifiers: NSEvent.ModifierFlags) {
    guard model.pointee.has_accelerator_at?(model, index) != 0 else { return ("", []) }
    var keyCode: Int32 = 0
    var shift: Int32 = 0
    var control: Int32 = 0
    var alt: Int32 = 0
    guard model.pointee.get_accelerator_at?(model, index, &keyCode, &shift, &control, &alt) != 0 else {
        return ("", [])
    }
    let key: String
    switch keyCode {
    case 0x08: key = "\u{8}"
    case 0x0D: key = "\r"
    case 0x1B: key = "\u{1b}"
    case 0x20: key = " "
    case 0x25: key = String(UnicodeScalar(0xF702)!)
    case 0x26: key = String(UnicodeScalar(0xF700)!)
    case 0x27: key = String(UnicodeScalar(0xF703)!)
    case 0x28: key = String(UnicodeScalar(0xF701)!)
    case 0x2E: key = "\u{7f}"
    case 0x30...0x39, 0x41...0x5A:
        key = String(UnicodeScalar(UInt8(keyCode))).lowercased()
    case 0x70...0x7B: key = String(UnicodeScalar(0xF704 + UInt32(keyCode - 0x70))!)
    default: return ("", [])
    }
    var modifiers: NSEvent.ModifierFlags = []
    // CefMenuModelImpl reports a ui::Accelerator's shift/ctrl/alt and drops
    // EF_COMMAND_DOWN, so a chord Chromium spells with control is the one this
    // platform spells with command.
    if control != 0 { modifiers.insert(.command) }
    if alt != 0 { modifiers.insert(.option) }
    if shift != 0 { modifiers.insert(.shift) }
    return (key, modifiers)
}

// MARK: - The commands a drawn item may carry

/// An item whose command this engine refuses is dropped rather than drawn: an
/// entry that does nothing when picked is worse than no entry. The open-link
/// commands stay, because `on_context_menu_command` reroutes them to the app's
/// `newWindow` before the deny list is consulted.
func ndCefMenuItemDenied(_ commandID: Int32) -> Bool {
    guard commandID >= 0 else { return false }
    if ndCefOpenLinkCommands.contains(commandID) { return false }
    return ndCefBlockedChromeCommands.contains(commandID)
}

/// The subset of the deny list that Chrome's own context menu offers for a link,
/// and that the app gets as `newWindow` with the link's URL instead.
let ndCefOpenLinkCommands: Set<Int32> = ndCefCommandIDs([
    "IDC_CONTENT_CONTEXT_OPENLINKNEWTAB", "IDC_CONTENT_CONTEXT_OPENLINKNEWWINDOW",
    "IDC_CONTENT_CONTEXT_OPENLINKOFFTHERECORD", "IDC_CONTENT_CONTEXT_OPENLINKINPROFILE",
    "IDC_CONTENT_CONTEXT_OPENLINKBOOKMARKAPP",
])

/// `cef_id_for_command_id_name` answers -1 for a name this build does not know,
/// which is a name that cannot be triggered either.
func ndCefCommandIDs(_ names: [String]) -> Set<Int32> {
    var ids: Set<Int32> = []
    for name in names {
        let id = name.withCString { nd_cef_command_id($0) }
        if id >= 0 { ids.insert(id) }
    }
    return ids
}
#endif
