import AppKit

// Task 8 owner directive: HeaderBar on Mac is a REAL unified NSToolbar on
// the NSWindow, not a styled stand-in bar view — the AppKit peer of the
// GTK side's `gtk_window_set_titlebar` takeover (NDGen/Widgets.swift's
// generated Window append/remove arms).

/// Host-rendered widget handle for a mounted `<headerbar>`. It never joins
/// the window's content-view subview hierarchy — `ndInstallHeaderBar` hands
/// the window's real `NSToolbar` over to it instead, so this class exists
/// only to hold state (title, slot children, the owning toolbar manager)
/// until/while it's installed.
final class NDHeaderBarView: NSView {
    var ndTitle: String = ""
    var startViews: [NSView] = []
    var endViews: [NSView] = []
    weak var attachedWindow: NSWindow?
    var toolbarManager: NDToolbarManager?
}

// Monotonic source for both toolbar identifiers ("nd-toolbar-<n>") and
// per-child item identifiers ("nd-hb-<n>") — single global counter, module
// scope, same `nonisolated(unsafe)` idiom as Backend.swift's `gridCells`
// (this process's UI code all runs on one thread in practice).
nonisolated(unsafe) private var ndHeaderBarNextID = 0
private func ndHeaderBarFreshID() -> Int {
    ndHeaderBarNextID += 1
    return ndHeaderBarNextID
}

/// Owns the live `NSToolbar` for one installed HeaderBar. Item identifiers
/// are minted per child view so the delegate callbacks can map an
/// identifier back to the `NSView` that becomes the item's `.view`.
final class NDToolbarManager: NSObject, NSToolbarDelegate {
    private weak var bar: NDHeaderBarView?
    private var idsByView: [ObjectIdentifier: NSToolbarItem.Identifier] = [:]
    private(set) var toolbar: NSToolbar

    init(bar: NDHeaderBarView) {
        self.bar = bar
        self.toolbar = NSToolbar(identifier: "nd-toolbar-\(ndHeaderBarFreshID())")
        super.init()
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = false
    }

    private func identifier(for view: NSView) -> NSToolbarItem.Identifier {
        let key = ObjectIdentifier(view)
        if let existing = idsByView[key] { return existing }
        let id = NSToolbarItem.Identifier(rawValue: "nd-hb-\(ndHeaderBarFreshID())")
        idsByView[key] = id
        return id
    }

    /// Rebuilds the toolbar from scratch (new `NSToolbar` instance, fresh
    /// item identifiers) rather than mutating a live toolbar's item list in
    /// place — simpler and reliable for a full pack/unpack refresh, and
    /// cheap since a HeaderBar carries only a handful of items.
    func rebuild() {
        guard let bar = bar, let win = bar.attachedWindow else { return }
        idsByView.removeAll()
        toolbar = NSToolbar(identifier: "nd-toolbar-\(ndHeaderBarFreshID())")
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = false
        win.toolbar = toolbar
    }

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        guard let bar = bar else { return [] }
        let start = bar.startViews.map { identifier(for: $0) }
        let end = bar.endViews.map { identifier(for: $0) }
        return start + [.flexibleSpace] + end
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        toolbarDefaultItemIdentifiers(toolbar)
    }

    func toolbar(_ toolbar: NSToolbar, itemForItemIdentifier itemIdentifier: NSToolbarItem.Identifier,
                 willBeInsertedIntoToolbar flag: Bool) -> NSToolbarItem? {
        guard let bar = bar,
              let view = (bar.startViews + bar.endViews).first(where: { idsByView[ObjectIdentifier($0)] == itemIdentifier })
        else { return nil }
        let item = NSToolbarItem(itemIdentifier: itemIdentifier)
        item.view = view
        return item
    }
}

/// Appends `child` to HeaderBar's start/end slot arrays (generated
/// `ndAppendChild`/`ndInsertBefore` HeaderBar arm) and, if already
/// installed on a window, rebuilds the live toolbar to reflect it.
func ndHeaderBarPack(_ bar: NDHeaderBarView, _ child: NSView, slot: String) {
    if slot == "end" {
        bar.endViews.append(child)
    } else {
        bar.startViews.append(child)
    }
    bar.toolbarManager?.rebuild()
}

/// Removes `child` from whichever slot array holds it (generated
/// `ndRemoveChild` HeaderBar arm) and rebuilds the toolbar if installed.
func ndHeaderBarUnpack(_ bar: NDHeaderBarView, _ child: NSView) {
    bar.startViews.removeAll { $0 === child }
    bar.endViews.removeAll { $0 === child }
    bar.toolbarManager?.rebuild()
}

/// Hands the process's window's real `NSToolbar` over to `bar` (generated
/// `ndAppendChild` Window arm, HeaderBar branch). `bar` itself never joins
/// the content-view hierarchy. Window close/minimize/zoom buttons are
/// untouched by any of this — only `toolbar`/`toolbarStyle` are set.
func ndInstallHeaderBar(_ bar: NDHeaderBarView) {
    guard let win = gWindow else { return }
    bar.attachedWindow = win
    let manager = NDToolbarManager(bar: bar)
    bar.toolbarManager = manager
    win.toolbar = manager.toolbar
    win.toolbarStyle = .unified
    if !bar.ndTitle.isEmpty { win.title = bar.ndTitle }
}

/// Reverses `ndInstallHeaderBar` (generated `ndRemoveChild` Window arm).
func ndRemoveHeaderBar(_ bar: NDHeaderBarView) {
    if let win = bar.attachedWindow, win.toolbar === bar.toolbarManager?.toolbar {
        win.toolbar = nil
    }
    bar.toolbarManager = nil
    bar.attachedWindow = nil
}
