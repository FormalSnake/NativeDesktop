import AppKit

/// freedesktop icon name -> SF Symbol lookup for Button's `iconName` prop
/// (M12 Feature 1). Only the names notes/main.tsx and friends are known to
/// use are mapped; anything else passes through verbatim at the call site
/// (`ndApplyButtonIcon`'s `?? iconName` fallback), which covers direct SF
/// Symbol names.
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
    // M13: menu-bar / notes-rework icon names (document-new/edit-delete above;
    // pin variants here so the foreseen <menuitem iconName> set resolves).
    "view-pin": "pin",
    "pin": "pin",
    "starred": "star.fill",
    "starred-symbolic": "star.fill", // M11 SourceList Wave 3: gallery's SourceList "Starred" row icon
    "non-starred": "star",
    "edit-copy": "doc.on.doc",
    "edit-cut": "scissors",
    "edit-paste": "doc.on.clipboard",
]

func ndSFSymbol(forFreedesktop name: String) -> String? {
    ndSFSymbolMap[name]
}

/// `ndCreate`'s Button arm (generated) calls this when `iconName` is set.
/// Resolves the freedesktop name to an SF Symbol (falling back to the name
/// itself, which covers direct SF Symbol names), sets the button's image,
/// and picks an image position from whether a label is also present. Falls
/// back to title-only (via `ND_WARN`) if the symbol can't be resolved.
func ndApplyButtonIcon(_ b: NSButton, iconName: String, label: String) {
    let symbol = ndSFSymbol(forFreedesktop: iconName) ?? iconName
    if let img = NSImage(systemSymbolName: symbol, accessibilityDescription: label.isEmpty ? nil : label) {
        b.image = img
        b.imagePosition = label.isEmpty ? .imageOnly : .imageLeading
    } else {
        FileHandle.standardError.write("ND_WARN unknown iconName \(iconName)\n".data(using: .utf8)!)
    }
}
