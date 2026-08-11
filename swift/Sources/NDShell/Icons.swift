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
    "list-remove": "minus",
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
    "go-up": "chevron.up",
    "go-down": "chevron.down",
    "go-home": "house",
    "window-close": "xmark",
    "document-save": "square.and.arrow.down",
    "view-refresh": "arrow.clockwise",
    "open-menu": "ellipsis.circle",
    "view-more": "ellipsis",
    // Menu-bar icon names (document-new/edit-delete above; pin variants here
    // so the <menuitem iconName> set resolves).
    "view-pin": "pin",
    "pin": "pin",
    "starred": "star.fill", // also covers the gallery's SourceList "Starred" row (-symbolic stripped)
    "non-starred": "star",
    "edit-copy": "doc.on.doc",
    "edit-cut": "scissors",
    "edit-paste": "doc.on.clipboard",
    "edit-undo": "arrow.uturn.backward",
    "edit-redo": "arrow.uturn.forward",
    "edit-clear": "xmark.circle.fill",
    // CanaryOrchestrator sidebar host glyphs + run-row status icons.
    "computer": "laptopcomputer",
    "network-server": "server.rack",
    "network-offline": "wifi.slash",
    "mail-unread": "envelope.badge",
    "content-loading": "arrow.triangle.2.circlepath",
    "emblem-ok": "checkmark.circle",
    "emblem-important": "exclamationmark.circle",
    "dialog-question": "questionmark.circle",
    "dialog-error": "exclamationmark.triangle",
    "dialog-warning": "exclamationmark.triangle",
    "dialog-information": "info.circle",
    "help-about": "info.circle",
    "process-stop": "xmark.octagon",
    "media-playback-stop": "stop.circle",
    "media-playback-start": "play.fill",
    "media-playback-pause": "pause.fill",
    "audio-volume-high": "speaker.wave.3",
    "audio-volume-muted": "speaker.slash",
    "web-browser": "globe",
    "folder": "folder",
    "folder-new": "folder.badge.plus",
    "user-home": "house",
    "utilities-terminal": "terminal",
    "view-dual": "rectangle.split.2x1",
    "view-grid": "square.grid.2x2",
    "pan-up": "chevron.up",
    "pan-down": "chevron.down",
    "pan-start": "chevron.backward",
    "pan-end": "chevron.forward",
    "zoom-in": "plus.magnifyingglass",
    "zoom-out": "minus.magnifyingglass",
    "zoom-original": "1.magnifyingglass",
    "sidebar-show": "sidebar.leading",
    "object-select": "checkmark",
    "document-edit": "pencil",
    "document-print": "printer",
    "preferences-system": "gearshape",
    "emblem-system": "gearshape",
    "applications-system": "gearshape",
    "system-users": "person.2",
    "avatar-default": "person.crop.circle",
    "bookmark-new": "bookmark",
    "appointment-new": "calendar.badge.plus",
    "alarm": "alarm",
    "changes-prevent": "lock",
    "changes-allow": "lock.open",
    "camera-photo": "camera",
    "image-x-generic": "photo",
    "text-x-generic": "doc.text",
    "drive-harddisk": "internaldrive",
    "media-eject": "eject",
    "input-keyboard": "keyboard",
]

func ndSFSymbol(forFreedesktop name: String) -> String? {
    let bare = name.hasSuffix("-symbolic") ? String(name.dropLast("-symbolic".count)) : name
    return ndSFSymbolMap[bare]
}

/// Symbol configuration for the generated Image arms, built from the
/// schema's `symbolScale`/`symbolWeight`/`symbolRenderingMode` props. An
/// axis left absent keeps the system default; all three absent returns nil
/// so an unconfigured `<image iconName>` stays byte-identical to before.
func ndSymbolConfiguration(scale: String?, weight: String?, renderingMode: String?) -> NSImage.SymbolConfiguration? {
    guard scale != nil || weight != nil || renderingMode != nil else { return nil }
    var config = NSImage.SymbolConfiguration()
    if scale != nil || weight != nil {
        let s: NSImage.SymbolScale
        switch scale ?? "medium" {
        case "small": s = .small
        case "large": s = .large
        default: s = .medium
        }
        let w: NSFont.Weight
        switch weight ?? "regular" {
        case "medium": w = .medium
        case "semibold": w = .semibold
        case "bold": w = .bold
        default: w = .regular
        }
        config = NSImage.SymbolConfiguration(pointSize: NSFont.systemFontSize, weight: w, scale: s)
    }
    switch renderingMode ?? "" {
    case "hierarchical": config = config.applying(.preferringHierarchical())
    case "multicolor": config = config.applying(.preferringMulticolor())
    case "monochrome": config = config.applying(.preferringMonochrome())
    default: break
    }
    return config
}

/// Symbol configuration captured at create for each `<image>` (the symbol
/// props are create-only), so an `iconName` update re-resolves with the same
/// treatment. Same accepted leak profile as Backend.swift's `ndLayoutFlags`
/// (bounded by live widget count).
nonisolated(unsafe) var ndImageSymbolConfigs: [ObjectIdentifier: NSImage.SymbolConfiguration] = [:]

/// Shared symbol resolver for both the `<button iconName>` and `<image
/// iconName>` widget arms: resolves the freedesktop/GTK name to an SF Symbol
/// (falling back to the name itself, which covers direct SF Symbol names)
/// and tries `NSImage(systemSymbolName:)`, applying `config` when given
/// (HIG: symbol weight tracks adjacent text, icon-only controls use the
/// large scale — callers derive the right configuration). Only for names
/// that don't resolve as a symbol does it fall back to a real asset lookup —
/// bundle asset catalog (`NSImage(named:)`) then filesystem path
/// (`contentsOfFile:`) — which covers `<image path>`-style values arriving
/// here as `iconName`. Degrades to `nil` (via `ND_WARN`) when nothing
/// resolves, never crashes.
func ndResolveSymbolImage(_ name: String, config: NSImage.SymbolConfiguration? = nil) -> NSImage? {
    let symbol = ndSFSymbol(forFreedesktop: name) ?? name
    if let img = NSImage(systemSymbolName: symbol, accessibilityDescription: nil) {
        if let config, let configured = img.withSymbolConfiguration(config) { return configured }
        return img
    }
    if let img = NSImage(named: name) ?? NSImage(contentsOfFile: name) {
        return img
    }
    FileHandle.standardError.write("ND_WARN unknown iconName \(name)\n".data(using: .utf8)!)
    return nil
}

/// `ndCreate`'s Button arm (generated) calls this when `iconName` is set.
/// Resolves via `ndResolveSymbolImage` with a configuration derived from
/// the button itself — point size tracks the button's font, weight stays
/// regular, scale is `.large` for icon-only buttons and `.medium` next to a
/// label, hierarchical rendering preferred (HIG symbol guidance) — then
/// sets the image with a label-derived accessibility description and an
/// image position matching label presence. Falls back to title-only if the
/// symbol can't be resolved (the resolver already emits `ND_WARN`).
func ndApplyButtonIcon(_ b: NSButton, iconName: String, label: String) {
    let config = NSImage.SymbolConfiguration(
        pointSize: b.font?.pointSize ?? NSFont.systemFontSize,
        weight: .regular,
        scale: label.isEmpty ? .large : .medium
    ).applying(.preferringHierarchical())
    guard let img = ndResolveSymbolImage(iconName, config: config) else { return }
    if label.isEmpty {
        // Icon-only buttons still need a spoken/overflow name (VoiceOver, the
        // toolbar overflow menu once promoted): humanize the freedesktop name.
        let bare = iconName.hasSuffix("-symbolic") ? String(iconName.dropLast("-symbolic".count)) : iconName
        img.accessibilityDescription = bare.replacingOccurrences(of: "-", with: " ")
    } else {
        img.accessibilityDescription = label
    }
    b.image = img
    b.imagePosition = label.isEmpty ? .imageOnly : .imageLeading
}
