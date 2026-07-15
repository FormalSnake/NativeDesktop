import AppKit
import CNd
import Foundation

// M13 menu bar. macOS 26 renders a <menubar> as the real NSApp.mainMenu: the
// standard default menus (App/File/Edit/View/Window/Help, responder-chain
// selectors) that every app gets, extended by the declared <menu>/<menuitem>
// tree. Menu nodes ride the ordinary create/append vtable ops, so each is a
// host-only NSView subclass (NDMenuNodeView — like HeaderBar.swift's
// NDToolbarPaneView) that NEVER enters the view hierarchy: that keeps the
// ABI's blind Unmanaged<NSView> cast in Backend.swift valid for a menu handle,
// while ndIsMenuNode lets the automation guards (node_visible/node_bounds/
// semanticClick, Automation.swift) treat them as chrome. NDMenuManager owns the
// live NSMenu and rebuilds it on any change (the NDToolbarManager pattern).
//
// All functions here are nonisolated (plain), matching the generated
// nonisolated ndCreate/ndConnectEvents/ndAppendChild arms that call them and
// the NDToolbarManager/EventDispatcher precedent — the ABI contract guarantees
// they only ever fire on the UI thread.

enum NDMenuKind { case menubar, menu, item }

enum NDMenuRole: String {
    case none, separator, about, settings, quit, undo, redo, cut, copy, paste
    case delete, selectAll, close, minimize, zoom, fullscreen
}

/// The retained model for one menu node. `children` is the ordered child list
/// the NDMenuManager walks to (re)assemble the NSMenu.
final class NDMenuNode {
    let kind: NDMenuKind
    var label: String = ""
    var iconName: String?
    var accelerator: String?
    var role: NDMenuRole = .none
    var enabled: Bool = true
    var defaults: Bool = true
    var nodeID: UInt32 = 0
    var children: [NDMenuNode] = []
    init(_ kind: NDMenuKind) { self.kind = kind }
}

/// Host-only handle. Never added to a window; exists purely so the ABI handle
/// round-trip (Unmanaged<NSView>) stays valid for a menu node.
final class NDMenuNodeView: NSView {
    let node: NDMenuNode
    init(_ node: NDMenuNode) {
        self.node = node
        super.init(frame: .zero)
    }
    required init?(coder: NSCoder) { fatalError("NDMenuNodeView is host-only") }
}

func ndIsMenuNode(_ view: NSView) -> Bool { view is NDMenuNodeView }
func ndMenuNode(_ view: NSView) -> NDMenuNode? { (view as? NDMenuNodeView)?.node }

// The sole menu manager + the nodeID->node table for semanticClick, reached
// through module-scope globals (same nonisolated(unsafe) idiom as
// ndWindowToolbarManager) so scheduleRebuild's dispatch closure captures
// nothing non-Sendable.
nonisolated(unsafe) var ndMenuManager: NDMenuManager? = nil
nonisolated(unsafe) var ndMenuNodesByID: [UInt32: NDMenuNode] = [:]

// MARK: - generated-arm entry points

func ndEnsureMenuManager() {
    if ndMenuManager == nil {
        let m = NDMenuManager()
        ndMenuManager = m
        m.rebuild()
        FileHandle.standardError.write("ND_MENUBAR_DEFAULTS_INSTALLED\n".data(using: .utf8)!)
    }
}

func ndMenubarCreate(_ defaults: Bool) -> NSView {
    let node = NDMenuNode(.menubar)
    node.defaults = defaults
    return NDMenuNodeView(node)
}

func ndMenuCreate(_ label: String) -> NSView {
    let node = NDMenuNode(.menu)
    node.label = label
    return NDMenuNodeView(node)
}

func ndMenuItemCreate(_ props: [String: Any]) -> NSView {
    let node = NDMenuNode(.item)
    node.label = propStr(props, "label") ?? ""
    node.iconName = propStr(props, "iconName")
    node.accelerator = propStr(props, "accelerator")
    node.role = NDMenuRole(rawValue: propStr(props, "role") ?? "none") ?? .none
    node.enabled = propBool(props, "enabled") ?? true
    return NDMenuNodeView(node)
}

func ndMenuItemSetEnabled(_ view: NSView, _ enabled: Bool) {
    guard let node = ndMenuNode(view) else { return }
    node.enabled = enabled
    ndMenuManager?.scheduleRebuild()
}

func ndMenuItemConnect(_ view: NSView, nodeID: UInt32) {
    guard let node = ndMenuNode(view) else { return }
    node.nodeID = nodeID
    ndMenuNodesByID[nodeID] = node
}

func ndMenuAppendChild(_ parent: NSView, _ child: NSView) {
    guard let p = ndMenuNode(parent), let c = ndMenuNode(child) else { return }
    p.children.append(c)
    ndMenuManager?.scheduleRebuild()
}

func ndMenuRemoveChild(_ parent: NSView, _ child: NSView) {
    guard let p = ndMenuNode(parent), let c = ndMenuNode(child) else { return }
    p.children.removeAll { $0 === c }
    ndMenuManager?.scheduleRebuild()
}

/// Window append arm: a <menubar> child is app chrome, not window content.
/// Returns true if `child` was a menubar (and got installed).
func ndMenuAttachToWindow(_ child: NSView) -> Bool {
    guard let node = ndMenuNode(child), node.kind == .menubar else { return false }
    ndMenuManager?.setDeclaredMenubar(node)
    return true
}

/// backend semanticClick routes a menu node's click here (Automation.swift).
func ndMenuSemanticClick(_ view: NSView, _ nodeID: UInt32) -> Bool {
    guard let node = ndMenuNode(view) else { return false }
    ndMenuManager?.fire(node)
    return true
}

// MARK: - NDMenuManager

/// The single app menu manager: builds NSApp.mainMenu (defaults merged with the
/// declared <menubar>) and coalesces rebuilds onto the next main-queue turn.
final class NDMenuManager: NSObject, NSMenuItemValidation {
    private var declaredMenubar: NDMenuNode?
    private var rebuildScheduled = false
    // NSMenuItem -> node, for custom items only (validation + fire).
    private var itemNodes: [ObjectIdentifier: NDMenuNode] = [:]
    // M15 generalization (the GTK menu-owner-registry mirror): any view that
    // hosts a menu built from Menu/MenuItem children — MenuButton/SplitButton
    // (NSComboButton.menu) and TrayItem (NSStatusItem.menu). Owners are held
    // weakly so a dropped node can't be pinned alive by this registry; dead
    // entries are pruned on rebuild. MenuItem `selected` keeps working for
    // every owner because rebuild() re-registers ALL owners' items into the
    // one `itemNodes` table it just cleared.
    private struct NDMenuOwner {
        weak var view: NSView?
        var children: [NDMenuNode] = []
    }
    private var owners: [ObjectIdentifier: NDMenuOwner] = [:]

    func setDeclaredMenubar(_ node: NDMenuNode) {
        declaredMenubar = node
        scheduleRebuild()
    }

    func ownerAppend(_ view: NSView, _ node: NDMenuNode) {
        var entry = owners[ObjectIdentifier(view)] ?? NDMenuOwner(view: view)
        entry.children.append(node)
        owners[ObjectIdentifier(view)] = entry
        scheduleRebuild()
    }

    func ownerRemove(_ view: NSView, _ node: NDMenuNode) {
        guard var entry = owners[ObjectIdentifier(view)] else { return }
        entry.children.removeAll { $0 === node }
        owners[ObjectIdentifier(view)] = entry
        scheduleRebuild()
    }

    /// Coalesces bursts of menu structural ops within one commit into a single
    /// rebuild on the next main-queue turn (the NDToolbarManager idiom: reach
    /// the sole manager via the global, so nothing non-Sendable is captured).
    func scheduleRebuild() {
        if rebuildScheduled { return }
        rebuildScheduled = true
        DispatchQueue.main.async {
            guard let m = ndMenuManager else { return }
            m.rebuildScheduled = false
            m.rebuild()
        }
    }

    func rebuild() {
        itemNodes.removeAll()
        let useDefaults = declaredMenubar?.defaults ?? true
        let mainMenu = NSMenu()

        // The first top-level item is always the App menu (title ignored at
        // runtime — AppKit shows the process name).
        let appMenu = NSMenu(title: "App")
        addAppDefaults(appMenu)
        addSubmenu(appMenu, to: mainMenu)

        var windowMenu: NSMenu?
        var helpMenu: NSMenu?
        var defaultTitles: [String: NSMenu] = [:]
        if useDefaults {
            let fileMenu = NSMenu(title: "File"); addFileDefaults(fileMenu)
            let editMenu = NSMenu(title: "Edit"); addEditDefaults(editMenu)
            let viewMenu = NSMenu(title: "View"); addViewDefaults(viewMenu)
            let winMenu = NSMenu(title: "Window"); addWindowDefaults(winMenu)
            let hlpMenu = NSMenu(title: "Help")
            for m in [fileMenu, editMenu, viewMenu, winMenu, hlpMenu] { addSubmenu(m, to: mainMenu) }
            defaultTitles = ["File": fileMenu, "Edit": editMenu, "View": viewMenu, "Window": winMenu, "Help": hlpMenu]
            windowMenu = winMenu
            helpMenu = hlpMenu
        }

        // Merge declared <menu>s: a label matching a default title appends to
        // that menu after a separator; any other label is a new top-level menu
        // inserted after View (before Window), in declaration order.
        if let menubar = declaredMenubar {
            for menuNode in menubar.children where menuNode.kind == .menu {
                if useDefaults, let target = defaultTitles[menuNode.label] {
                    target.addItem(.separator())
                    for itemNode in menuNode.children { addMenuItem(itemNode, to: target) }
                } else {
                    let submenu = NSMenu(title: menuNode.label)
                    for itemNode in menuNode.children { addMenuItem(itemNode, to: submenu) }
                    let holder = NSMenuItem()
                    holder.submenu = submenu
                    if useDefaults, let win = windowMenu,
                       let idx = mainMenu.items.firstIndex(where: { $0.submenu === win }) {
                        mainMenu.insertItem(holder, at: idx)
                    } else {
                        mainMenu.addItem(holder)
                    }
                }
            }
        }

        NSApp.mainMenu = mainMenu
        if useDefaults {
            NSApp.windowsMenu = windowMenu
            NSApp.helpMenu = helpMenu
        }

        // M15: rebuild every registered menu owner from the same declared
        // nodes in this one pass — itemNodes was cleared above, so the
        // item->node routing for menubar AND owners is always re-registered
        // together (a Menu list mutation after append refreshes all hosts).
        var dead: [ObjectIdentifier] = []
        for (key, entry) in owners {
            guard let view = entry.view else {
                dead.append(key)
                continue
            }
            let menu = NSMenu()
            for node in entry.children { addOwnerChild(node, to: menu) }
            ndMenuAssign(menu, to: view)
        }
        for key in dead { owners[key] = nil }
    }

    /// Owner menus commonly mix items and nested <menu> submenus (unlike the
    /// menubar, whose top level is menu-only).
    private func addOwnerChild(_ node: NDMenuNode, to menu: NSMenu) {
        switch node.kind {
        case .item:
            addMenuItem(node, to: menu)
        case .menu:
            let submenu = NSMenu(title: node.label)
            for child in node.children { addOwnerChild(child, to: submenu) }
            let holder = NSMenuItem()
            holder.title = node.label
            holder.submenu = submenu
            menu.addItem(holder)
        case .menubar:
            break
        }
    }

    private func addSubmenu(_ menu: NSMenu, to main: NSMenu) {
        let holder = NSMenuItem()
        holder.submenu = menu
        main.addItem(holder)
    }

    // MARK: default menus (responder-chain selectors, target nil except NSApp)

    private func item(_ title: String, _ sel: Selector, _ keyEq: String = "",
                      _ mods: NSEvent.ModifierFlags = .command, target: AnyObject? = nil) -> NSMenuItem {
        let it = NSMenuItem(title: title, action: sel, keyEquivalent: keyEq)
        if !keyEq.isEmpty { it.keyEquivalentModifierMask = mods }
        it.target = target
        return it
    }

    private func addAppDefaults(_ m: NSMenu) {
        m.addItem(item("About", #selector(NSApplication.orderFrontStandardAboutPanel(_:)), "", [], target: NSApp))
        m.addItem(.separator())
        m.addItem(item("Hide", #selector(NSApplication.hide(_:)), "h", .command, target: NSApp))
        m.addItem(item("Hide Others", #selector(NSApplication.hideOtherApplications(_:)), "h", [.command, .option], target: NSApp))
        m.addItem(item("Show All", #selector(NSApplication.unhideAllApplications(_:)), "", [], target: NSApp))
        m.addItem(.separator())
        m.addItem(item("Quit", #selector(NSApplication.terminate(_:)), "q", .command, target: NSApp))
    }

    private func addFileDefaults(_ m: NSMenu) {
        m.addItem(item("Close", #selector(NSWindow.performClose(_:)), "w"))
    }

    private func addEditDefaults(_ m: NSMenu) {
        m.addItem(item("Undo", Selector(("undo:")), "z"))
        m.addItem(item("Redo", Selector(("redo:")), "z", [.command, .shift]))
        m.addItem(.separator())
        m.addItem(item("Cut", #selector(NSText.cut(_:)), "x"))
        m.addItem(item("Copy", #selector(NSText.copy(_:)), "c"))
        m.addItem(item("Paste", #selector(NSText.paste(_:)), "v"))
        m.addItem(item("Delete", #selector(NSText.delete(_:)), "", []))
        m.addItem(item("Select All", #selector(NSText.selectAll(_:)), "a"))
    }

    private func addViewDefaults(_ m: NSMenu) {
        m.addItem(item("Enter Full Screen", #selector(NSWindow.toggleFullScreen(_:)), "f", [.command, .control]))
    }

    private func addWindowDefaults(_ m: NSMenu) {
        m.addItem(item("Minimize", #selector(NSWindow.performMiniaturize(_:)), "m"))
        m.addItem(item("Zoom", #selector(NSWindow.performZoom(_:)), "", []))
        m.addItem(.separator())
        m.addItem(item("Bring All to Front", #selector(NSApplication.arrangeInFront(_:)), "", [], target: NSApp))
    }

    // MARK: declared items

    private func addMenuItem(_ node: NDMenuNode, to menu: NSMenu) {
        if node.role == .separator {
            menu.addItem(.separator())
            return
        }
        var title = node.label
        if title.isEmpty {
            if node.role == .settings { title = "Preferences…" }
            else if node.role == .about { title = "About" }
        }
        let it = NSMenuItem()
        it.title = title
        if let accel = node.accelerator, let (keyEq, mods) = ndParseAccelerator(accel) {
            it.keyEquivalent = keyEq
            it.keyEquivalentModifierMask = mods
        }
        if let iconName = node.iconName {
            let symbol = ndSFSymbol(forFreedesktop: iconName) ?? iconName
            if let img = NSImage(systemSymbolName: symbol, accessibilityDescription: nil) {
                it.image = img
                // NSMenuItem.preferredImageVisibility (the macOS 27 "show symbol
                // images" opt-in) is absent from the current SDK; images render
                // by default here, so setting .image suffices.
            }
        }
        switch node.role {
        case .none, .settings:
            // Custom item (onSelect wins over any role): fires "selected".
            it.target = self
            it.action = #selector(NDMenuManager.ndMenuFire(_:))
            itemNodes[ObjectIdentifier(it)] = node
        case .about:
            it.target = NSApp
            it.action = #selector(NSApplication.orderFrontStandardAboutPanel(_:))
        default:
            // A native role: responder-chain selector, target nil.
            if let sel = ndRoleSelector(node.role) { it.action = sel }
        }
        it.isEnabled = node.enabled
        menu.addItem(it)
    }

    // MARK: dispatch

    @objc func ndMenuFire(_ sender: NSMenuItem) {
        guard let node = itemNodes[ObjectIdentifier(sender)], node.enabled else { return }
        ndMenuEmitSelected(node.nodeID)
    }

    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        if let node = itemNodes[ObjectIdentifier(menuItem)] { return node.enabled }
        return true
    }

    /// Fires a node as if clicked (used by automation semanticClick).
    func fire(_ node: NDMenuNode) {
        switch node.role {
        case .none, .settings:
            if node.enabled { ndMenuEmitSelected(node.nodeID) }
        case .separator:
            break
        case .about:
            NSApp.sendAction(#selector(NSApplication.orderFrontStandardAboutPanel(_:)), to: NSApp, from: nil)
        default:
            if let sel = ndRoleSelector(node.role) { NSApp.sendAction(sel, to: nil, from: nil) }
        }
    }
}

private func ndMenuEmitSelected(_ nodeID: UInt32) {
    "selected".withCString { cName in
        "{}".withCString { cJson in
            nd_emit_event(gCtx, nodeID, cName, cJson)
        }
    }
}

private func ndRoleSelector(_ role: NDMenuRole) -> Selector? {
    switch role {
    case .quit: return #selector(NSApplication.terminate(_:))
    case .undo: return Selector(("undo:"))
    case .redo: return Selector(("redo:"))
    case .cut: return #selector(NSText.cut(_:))
    case .copy: return #selector(NSText.copy(_:))
    case .paste: return #selector(NSText.paste(_:))
    case .delete: return #selector(NSText.delete(_:))
    case .selectAll: return #selector(NSText.selectAll(_:))
    case .close: return #selector(NSWindow.performClose(_:))
    case .minimize: return #selector(NSWindow.performMiniaturize(_:))
    case .zoom: return #selector(NSWindow.performZoom(_:))
    case .fullscreen: return #selector(NSWindow.toggleFullScreen(_:))
    default: return nil
    }
}

/// "primary+shift+n" -> ("n", [.command, .shift]). `primary` = ⌘ on macOS.
func ndParseAccelerator(_ spec: String) -> (String, NSEvent.ModifierFlags)? {
    var mods: NSEvent.ModifierFlags = []
    var key: String?
    for part in spec.split(separator: "+") {
        switch part {
        case "primary": mods.insert(.command)
        case "shift": mods.insert(.shift)
        case "alt": mods.insert(.option)
        case "ctrl": mods.insert(.control)
        default: key = String(part)
        }
    }
    guard let k = key else { return nil }
    return (ndKeyEquivalent(k), mods)
}

private func ndKeyEquivalent(_ k: String) -> String {
    switch k {
    case "enter": return "\r"
    case "escape": return "\u{1b}"
    case "backspace": return "\u{8}"
    case "delete": return "\u{7f}"
    case "space": return " "
    case "tab": return "\t"
    case "up": return String(UnicodeScalar(0xF700)!)
    case "down": return String(UnicodeScalar(0xF701)!)
    case "left": return String(UnicodeScalar(0xF702)!)
    case "right": return String(UnicodeScalar(0xF703)!)
    case "comma": return ","
    case "period": return "."
    default:
        if k.count >= 2, k.first == "f", let n = Int(k.dropFirst()), n >= 1, n <= 12 {
            return String(UnicodeScalar(0xF704 + (n - 1))!)  // NSF1..NSF12FunctionKey
        }
        return k  // single printable char
    }
}
