import AppKit

/// Native system tabs for the <window> widget (AppKit side).
///
/// The app model is tabs-as-windows and AppKit's window tabbing IS that
/// model: every `<window tabGroup="x">` React root is a real NSWindow joined
/// into the group's native tab bar (tabbingIdentifier "nd.x"), so Safari-
/// style drag-out/drag-in/reorder, the tab overview (Show All Tabs), and the
/// Window-menu merge items all come from the OS. The React tree never
/// changes when the user drags a tab between windows — the NSWindow (and its
/// content, e.g. a live <webview>) moves intact.
///
/// Close protocol mirrors the GTK side's deferred close: a user close on a
/// tab-group window emits the node's `closed` event and returns false from
/// windowShouldClose; the app unmounts the <window>, and the remove op's
/// "window.close" semantic action performs the real close(). Plain windows
/// close natively and emit `closed` as information. Already-closed windows
/// make the semantic close a no-op, so remove stays idempotent.

private let ndTabIdentifierPrefix = "nd."

// Keyed by NSWindow identity. The delegate array retains the per-window
// delegate objects (NSWindow.delegate is weak).
nonisolated(unsafe) private var ndTabWindowNodeIDs: [ObjectIdentifier: UInt32] = [:]
nonisolated(unsafe) private var ndTabWindowDelegates: [ObjectIdentifier: NDWindowTabDelegate] = [:]
nonisolated(unsafe) private var ndClosedWindows: Set<ObjectIdentifier> = []

private func ndTabEmit(_ nodeID: UInt32, _ name: String, _ payload: [String: Any]) {
    guard let data = try? JSONSerialization.data(withJSONObject: ["data": payload]),
          let json = String(data: data, encoding: .utf8) else { return }
    ndEmitEvent(nodeID, name, json)
}

private func ndIsTabWindow(_ win: NSWindow) -> Bool {
    win.tabbingIdentifier.hasPrefix(ndTabIdentifierPrefix)
}

/// Generated create arm (Widgets.swift): present the new window. A tabGroup
/// member joins its group's most recent window — key window first, so a new
/// tab lands where the user is working (Chrome behavior) — appended at the
/// trailing edge of that window's tab group, Safari-style.
func ndWindowTabsPresent(_ win: NSWindow, tabGroup: String?) {
    guard let group = tabGroup else {
        win.center()
        win.makeKeyAndOrderFront(nil)
        return
    }
    let identifier = ndTabIdentifierPrefix + group
    win.tabbingMode = .preferred
    win.tabbingIdentifier = identifier

    let candidates = NSApp.windows.filter { $0 !== win && $0.tabbingIdentifier == identifier && $0.isVisible }
    let parent = candidates.first { $0.isKeyWindow } ?? candidates.last
    if let parent {
        // AppKit has been observed throwing NSException from addTabbedWindow
        // while the tab overview is open (the flow Ghostty wraps in an ObjC
        // catcher) — close the overview before joining instead.
        if parent.tabGroup?.isOverviewVisible == true {
            parent.tabGroup?.isOverviewVisible = false
        }
        (parent.tabGroup?.windows.last ?? parent).addTabbedWindow(win, ordered: .above)
    } else {
        win.center()
    }
    win.makeKeyAndOrderFront(nil)
}

/// Generated ndConnectEvents Window arm: record the node id and install the
/// tab-lifecycle delegate on the owning window.
func ndWindowTabsConnect(_ view: NSView, nodeID: UInt32) {
    guard let win = ndWindow(for: view) else { return }
    let key = ObjectIdentifier(win)
    ndTabWindowNodeIDs[key] = nodeID
    if ndTabWindowDelegates[key] == nil {
        let delegate = NDWindowTabDelegate()
        ndTabWindowDelegates[key] = delegate
        win.delegate = delegate
    }
}

/// Generated ndWidgetCommand Window arm (tab commands only — dialogs stay in
/// WindowDialogs.swift).
func ndWindowTabsCommand(_ view: NSView, _ command: String, _ argJson: String) {
    guard let win = ndWindow(for: view) else { return }
    switch command {
    case "showTabOverview":
        win.toggleTabOverview(nil)
    case "present":
        // C5: raise/focus an open window/tab. Each `<window tabGroup>` member
        // is its own real NSWindow (this file's header comment) — presenting
        // one means switching the OS tab bar to it, then ordering front.
        win.tabGroup?.selectedWindow = win
        win.makeKeyAndOrderFront(nil)
    default:
        break
    }
}

/// The "window.close" semantic action (tree.zig remove arm): the React root
/// unmounted, so the OS window goes too. close() bypasses windowShouldClose
/// by design — JS already made the decision.
func ndWindowTabsClose(_ view: NSView) {
    guard let win = ndWindow(for: view) else { return }
    let key = ObjectIdentifier(win)
    if ndClosedWindows.contains(key) { return }
    win.close()
}

final class NDWindowTabDelegate: NSObject, NSWindowDelegate {
    /// Tab-group windows defer to JS (`closed` event -> unmount -> semantic
    /// close); plain windows close natively with `closed` as information.
    /// Closing a whole tabbed window fires this once per member tab — each
    /// tab node reports itself, which is exactly the per-<window> contract.
    func windowShouldClose(_ sender: NSWindow) -> Bool {
        guard let nodeID = ndTabWindowNodeIDs[ObjectIdentifier(sender)] else { return true }
        ndTabEmit(nodeID, "closed", [:])
        return !ndIsTabWindow(sender)
    }

    func windowWillClose(_ notification: Notification) {
        guard let win = notification.object as? NSWindow else { return }
        ndClosedWindows.insert(ObjectIdentifier(win))
        ndTabWindowDelegates[ObjectIdentifier(win)] = nil
    }

    /// Existence of this method in the responder chain is what makes AppKit
    /// show the "+" button on the tab bar; the click becomes the node's
    /// newTabRequested so the app renders another <window tabGroup>.
    @objc func newWindowForTab(_ sender: Any?) {
        guard let win = NSApp.keyWindow ?? NSApp.mainWindow,
              let nodeID = ndTabWindowNodeIDs[ObjectIdentifier(win)] else { return }
        var payload: [String: Any] = [:]
        if ndIsTabWindow(win) {
            payload["tabGroup"] = String(win.tabbingIdentifier.dropFirst(ndTabIdentifierPrefix.count))
        }
        ndTabEmit(nodeID, "newTabRequested", payload)
    }

    /// `focused` fires per-window, including tab-group members: AppKit
    /// gives each `<window tabGroup>` its own real NSWindow (this file's
    /// header comment), so switching native tabs resigns the outgoing
    /// window's key status and makes the incoming one key — no extra
    /// tab-selection wiring needed beyond these two delegate methods.
    func windowDidBecomeKey(_ notification: Notification) {
        guard let win = notification.object as? NSWindow,
              let nodeID = ndTabWindowNodeIDs[ObjectIdentifier(win)] else { return }
        ndEmitEvent(nodeID, "focused", "{\"checked\":true}")
    }

    func windowDidResignKey(_ notification: Notification) {
        guard let win = notification.object as? NSWindow,
              let nodeID = ndTabWindowNodeIDs[ObjectIdentifier(win)] else { return }
        ndEmitEvent(nodeID, "focused", "{\"checked\":false}")
    }
}
