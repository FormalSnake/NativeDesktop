import AppKit

/// TrayItem is macOS-exclusive (ND_PLATFORM_NOOP on GTK): an
/// NSStatusBar.system status item behind a host-only NSView handle (the
/// NDMenuNodeView idiom). HIG menu bar rules apply: a template SF Symbol
/// image (black-and-clear, system-tinted for light/dark/tint), and a menu
/// rather than a popover. The Menu/MenuItem children build the item's menu
/// via the generalized NDMenuManager owner registry. Lifecycle rides the
/// node's structural ops: the cross-cutting detach guard removes the status
/// item when the node unmounts (a tray item never enters any view hierarchy).
final class NDTrayItemView: NSView {
    let statusItem: NSStatusItem

    init(iconName: String?, tooltip: String?) {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        super.init(frame: .zero)
        apply(iconName: iconName, tooltip: tooltip)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("NDTrayItemView is not NSCoding-decodable") }

    func apply(iconName: String?, tooltip: String?) {
        if let iconName, let button = statusItem.button {
            let symbol = ndSFSymbol(forFreedesktop: iconName) ?? iconName
            if let img = NSImage(systemSymbolName: symbol, accessibilityDescription: tooltip) {
                img.isTemplate = true
                button.image = img
            } else {
                FileHandle.standardError.write("ND_WARN unknown TrayItem iconName \(iconName)\n".data(using: .utf8)!)
            }
        }
        if let tooltip { statusItem.button?.toolTip = tooltip }
    }

    func teardown() {
        NSStatusBar.system.removeStatusItem(statusItem)
    }
}

/// `ndCreate`'s TrayItem arm (generated) calls this.
func makeTrayItem(_ props: [String: Any]) -> NSView {
    NDTrayItemView(iconName: propStr(props, "iconName"), tooltip: propStr(props, "tooltip"))
}

/// Generated ndApplyProps TrayItem arm — merged apply.
func ndTrayItemApply(_ view: NSView, _ props: [String: Any]) {
    (view as? NDTrayItemView)?.apply(
        iconName: propStr(props, "iconName"),
        tooltip: propStr(props, "tooltip")
    )
}

// MARK: - cross-cutting structural guards (peers of the Popover guards):
// a TrayItem child is app chrome wherever it's declared — it must never be
// box-packed into window content, and unmounting it removes the status item.

func ndTrayItemStructuralAttach(_ child: NSView) -> Bool {
    child is NDTrayItemView
}

func ndTrayItemStructuralDetach(_ child: NSView) -> Bool {
    guard let tray = child as? NDTrayItemView else { return false }
    tray.teardown()
    return true
}
