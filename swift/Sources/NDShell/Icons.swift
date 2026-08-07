import AppKit

/// freedesktop/GTK symbolic icon name -> SF Symbol lookup, keyed on the BARE
/// name — `ndSFSymbol` strips a trailing "-symbolic" before lookup, since
/// GTK accepts both forms but AppKit only knows SF Symbol names. Covers the
/// names notes/main.tsx and friends use, plus CanaryOrchestrator's sidebar/
/// run-status vocabulary; anything else passes through verbatim at the call
/// site (`ndResolveSymbolImage`/`ndApplyButtonIcon`'s `?? name` fallback),
/// which covers direct SF Symbol names.
private let ndSFSymbolMap: [String: String] = [
    "list-add": "plus",
    "document-new": "square.and.pencil",
    "edit-delete": "trash",
    "user-trash": "trash",
    "edit-find": "magnifyingglass",
    "system-search": "magnifyingglass",
    "view-list": "list.bullet",
    "checklist": "checklist",
    "mail-send": "paperplane",
    "document-open": "folder",
    "emblem-shared": "person.crop.circle",
    "go-previous": "chevron.backward",
    "go-next": "chevron.forward",
    "window-close": "xmark",
    "document-save": "square.and.arrow.down",
    "view-refresh": "arrow.clockwise",
    "open-menu": "ellipsis.circle",
    // Menu-bar icon names (document-new/edit-delete above; pin variants here
    // so the <menuitem iconName> set resolves).
    "view-pin": "pin",
    "pin": "pin",
    "starred": "star.fill", // also covers the gallery's SourceList "Starred" row (-symbolic stripped)
    "non-starred": "star",
    "edit-copy": "doc.on.doc",
    "edit-cut": "scissors",
    "edit-paste": "doc.on.clipboard",
    // CanaryOrchestrator sidebar host glyphs + run-row status icons.
    "computer": "laptopcomputer",
    "network-server": "server.rack",
    "mail-unread": "envelope.badge",
    "content-loading": "arrow.triangle.2.circlepath",
    "emblem-ok": "checkmark.circle",
    "dialog-question": "questionmark.circle",
    "dialog-error": "exclamationmark.triangle",
    "media-playback-stop": "stop.circle",
    "web-browser": "globe",
    "folder": "folder",
    "utilities-terminal": "terminal",
    "view-dual": "rectangle.split.2x1",
    "view-grid": "square.grid.2x2",
]

func ndSFSymbol(forFreedesktop name: String) -> String? {
    let bare = name.hasSuffix("-symbolic") ? String(name.dropLast("-symbolic".count)) : name
    return ndSFSymbolMap[bare]
}

/// Shared symbol resolver for both the `<button iconName>` and `<image
/// iconName>` widget arms: resolves the freedesktop/GTK name to an SF Symbol
/// (falling back to the name itself, which covers direct SF Symbol names)
/// and tries `NSImage(systemSymbolName:)`. Only for names that don't resolve
/// as a symbol does it fall back to a real asset lookup — bundle asset
/// catalog (`NSImage(named:)`) then filesystem path (`contentsOfFile:`) —
/// which covers `<image path>`-style values arriving here as `iconName`.
/// Degrades to `nil` (via `ND_WARN`) when nothing resolves, never crashes.
func ndResolveSymbolImage(_ name: String) -> NSImage? {
    let symbol = ndSFSymbol(forFreedesktop: name) ?? name
    if let img = NSImage(systemSymbolName: symbol, accessibilityDescription: nil) {
        return img
    }
    if let img = NSImage(named: name) ?? NSImage(contentsOfFile: name) {
        return img
    }
    FileHandle.standardError.write("ND_WARN unknown iconName \(name)\n".data(using: .utf8)!)
    return nil
}

/// `ndCreate`'s Button arm (generated) calls this when `iconName` is set.
/// Resolves via `ndResolveSymbolImage`, sets the button's image with a
/// label-derived accessibility description, and picks an image position
/// from whether a label is also present. Falls back to title-only if the
/// symbol can't be resolved (the resolver already emits `ND_WARN`).
func ndApplyButtonIcon(_ b: NSButton, iconName: String, label: String) {
    guard let img = ndResolveSymbolImage(iconName) else { return }
    if !label.isEmpty { img.accessibilityDescription = label }
    b.image = img
    b.imagePosition = label.isEmpty ? .imageOnly : .imageLeading
}
