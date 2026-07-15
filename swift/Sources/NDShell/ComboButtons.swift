import AppKit

/// MenuButton + SplitButton: one NSComboButton (macOS 13+) class covers
/// both — `.unified` for the menu-only MenuButton and `.split` for
/// SplitButton (separate arrow segment; the main segment's target/action is
/// wired to `clicked` by the ordinary EventDispatcher path, since
/// NSComboButton is an NSControl). Menus are built from Menu/MenuItem
/// children through the generalized NDMenuManager owner registry
/// (MenuBar.swift), so MenuItem's `selected` routes identically from the
/// menubar, both button kinds, and TrayItem.

/// `.unified` performs its action on a plain click (menu on click-and-hold);
/// a MenuButton has no action of its own, so its action POPS the menu —
/// click-to-open, matching GtkMenuButton.
final class NDComboMenuProxy: NSObject {
    nonisolated(unsafe) static let shared = NDComboMenuProxy()

    @objc func popMenu(_ sender: NSControl) {
        guard let combo = sender as? NSComboButton else { return }
        let origin = NSPoint(x: combo.bounds.minX, y: combo.isFlipped ? combo.bounds.maxY + 4 : combo.bounds.minY - 4)
        combo.menu.popUp(positioning: nil, at: origin, in: combo)
    }
}

/// `ndCreate`'s MenuButton arm (generated) calls this.
func makeMenuButton(_ props: [String: Any]) -> NSView {
    let combo = NSComboButton(title: propStr(props, "label") ?? "", menu: NSMenu(), target: NDComboMenuProxy.shared, action: #selector(NDComboMenuProxy.popMenu(_:)))
    combo.style = .unified
    if let icon = propStr(props, "iconName") { ndApplyComboIcon(combo, icon) }
    ndSeedComboAlignment(combo)
    return combo
}

/// `ndCreate`'s SplitButton arm (generated) calls this. The default action
/// (-> `clicked`) is wired later by ndConnectEvents' NSControl path.
func makeSplitButton(_ props: [String: Any]) -> NSView {
    let combo = NSComboButton(title: propStr(props, "label") ?? "", menu: NSMenu(), target: nil, action: nil)
    combo.style = .split
    if let icon = propStr(props, "iconName") { ndApplyComboIcon(combo, icon) }
    ndSeedComboAlignment(combo)
    return combo
}

/// Content-sized like Button/ToggleButton, not stretched to fill a parent
/// vertical Box — the GTK arm hardcodes setHalign(.start)/hexpand(0) at
/// create for exactly this reason. Seeding `ndLayoutFlags` (instead of
/// constraining here) keeps ndApplyStyle's merge semantics: an explicit
/// app-level halign still wins, absence keeps "start".
private func ndSeedComboAlignment(_ combo: NSComboButton) {
    var flags = ndLayoutFlags[ObjectIdentifier(combo)] ?? NDLayoutFlags()
    flags.halign = "start"
    ndLayoutFlags[ObjectIdentifier(combo)] = flags
}

func ndApplyComboIcon(_ combo: NSComboButton, _ iconName: String) {
    let symbol = ndSFSymbol(forFreedesktop: iconName) ?? iconName
    if let img = NSImage(systemSymbolName: symbol, accessibilityDescription: combo.title.isEmpty ? nil : combo.title) {
        combo.image = img
    } else {
        FileHandle.standardError.write("ND_WARN unknown iconName \(iconName)\n".data(using: .utf8)!)
    }
}

/// Generated structural MenuButton/SplitButton/TrayItem arms: children are
/// Menu/MenuItem host handles (NDMenuNodeView) — route them into the owner
/// registry, which rebuilds this owner's NSMenu on every structural change.
func ndMenuOwnerAppend(_ owner: NSView, _ child: NSView) {
    guard let node = ndMenuNode(child) else { return }
    ndEnsureMenuManager()
    ndMenuManager?.ownerAppend(owner, node)
}

func ndMenuOwnerRemove(_ owner: NSView, _ child: NSView) {
    guard let node = ndMenuNode(child) else { return }
    ndMenuManager?.ownerRemove(owner, node)
}

/// NDMenuManager.rebuild hands each owner its freshly built menu here.
func ndMenuAssign(_ menu: NSMenu, to view: NSView) {
    if let combo = view as? NSComboButton {
        combo.menu = menu
    } else if let tray = view as? NDTrayItemView {
        tray.statusItem.menu = menu
    }
}
