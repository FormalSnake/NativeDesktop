import AppKit
import WebKit

/// The `<webview>` context-menu model: the item tree an app hands to
/// `setContextMenuItems`, and the hit-test matching that decides which of those
/// items a given right-click earns. Peer of `src/gtk/context_menu.zig`: the
/// two files answer the same questions the same way, so an app's menu reads the
/// same on both backends.
struct NDContextMask: OptionSet {
    let rawValue: Int
    static let page = NDContextMask(rawValue: 1 << 0)
    static let link = NDContextMask(rawValue: 1 << 1)
    static let image = NDContextMask(rawValue: 1 << 2)
    static let selection = NDContextMask(rawValue: 1 << 3)
    static let editable = NDContextMask(rawValue: 1 << 4)
    static let all: NDContextMask = [.page, .link, .image, .selection, .editable]

    static func named(_ name: String) -> NDContextMask {
        switch name {
        case "page": return .page
        case "link": return .link
        case "image": return .image
        case "selection": return .selection
        case "editable": return .editable
        case "all": return .all
        default: return []
        }
    }
}

/// What the hit test found under the pointer.
struct NDContextMenuHit {
    var link = ""
    var image = ""
    var selection = ""
    var editable = false
    /// When this hit was observed. WKWebView reports no hit test of its own, so
    /// the page-side agent's report is only usable while it is fresh.
    var at = Date.distantPast

    var contexts: NDContextMask {
        var bits: NDContextMask = []
        if !link.isEmpty { bits.insert(.link) }
        if !image.isEmpty { bits.insert(.image) }
        if !selection.isEmpty { bits.insert(.selection) }
        if editable { bits.insert(.editable) }
        // Chrome's rule, and the one users read off a browser: "page" means the
        // click landed on nothing more specific.
        return bits.isEmpty ? .page : bits
    }
}

struct NDContextMenuItem {
    enum Kind: String {
        case normal, checkbox, radio, separator
    }

    var id = ""
    var label = ""
    var kind = Kind.normal
    var checked = false
    var enabled = true
    var contexts = NDContextMask.page
    /// `*`-wildcard globs tested against the hit's link or image URL. Empty
    /// means "any target".
    var targetUrlGlobs: [String] = []
    var children: [NDContextMenuItem] = []

    /// Parses the `setContextMenuItems` argument (`{ items: [...] }` or a bare
    /// array). Malformed entries are skipped rather than failing the whole
    /// command: one bad item must not cost an app its menu.
    static func parse(_ arg: Any?) -> [NDContextMenuItem] {
        let array: [Any]
        if let list = arg as? [Any] {
            array = list
        } else if let obj = arg as? [String: Any], let list = obj["items"] as? [Any] {
            array = list
        } else {
            return []
        }
        return array.compactMap { parseOne($0 as? [String: Any]) }
    }

    private static func parseOne(_ obj: [String: Any]?) -> NDContextMenuItem? {
        guard let obj else { return nil }
        let kind = Kind(rawValue: obj["type"] as? String ?? "normal") ?? .normal
        if kind == .separator {
            return NDContextMenuItem(kind: .separator, contexts: .all)
        }
        guard let id = obj["id"] as? String, !id.isEmpty,
              let label = obj["label"] as? String, !label.isEmpty
        else { return nil }

        var item = NDContextMenuItem(
            id: id,
            label: label,
            kind: kind,
            checked: (obj["checked"] as? NSNumber)?.boolValue ?? false,
            enabled: (obj["enabled"] as? NSNumber)?.boolValue ?? true
        )
        if let names = obj["contexts"] as? [String] {
            var bits: NDContextMask = []
            for name in names { bits.formUnion(NDContextMask.named(name)) }
            if !bits.isEmpty { item.contexts = bits }
        }
        item.targetUrlGlobs = obj["targetUrlGlobs"] as? [String] ?? []
        if let children = obj["children"] as? [Any] {
            item.children = children.compactMap { parseOne($0 as? [String: Any]) }
        }
        return item
    }

    /// Does this item belong in the menu this hit earned?
    func matches(_ hit: NDContextMenuHit) -> Bool {
        if kind == .separator { return true }
        let bits = hit.contexts.intersection(contexts)
        if bits.isEmpty { return false }
        if targetUrlGlobs.isEmpty { return true }
        // Which URL a target glob is tested against depends on which context
        // matched: a link item tests the href, an image item the source.
        if bits.contains(.link), NDContextMenuItem.anyGlobMatches(targetUrlGlobs, hit.link) { return true }
        if bits.contains(.image), NDContextMenuItem.anyGlobMatches(targetUrlGlobs, hit.image) { return true }
        return false
    }

    /// True when this item, or anything under it, would be shown. A submenu
    /// whose every child was filtered out is not a submenu, it is a dead label.
    func survives(_ hit: NDContextMenuHit) -> Bool {
        guard matches(hit) else { return false }
        if children.isEmpty { return true }
        return children.contains { $0.kind != .separator && $0.survives(hit) }
    }

    static func anyGlobMatches(_ globs: [String], _ url: String) -> Bool {
        if url.isEmpty { return false }
        return globs.contains { globMatches($0, url) }
    }

    /// `*` matches any run of characters (including none); everything else is
    /// literal. Deliberately NOT Chrome's match-pattern grammar; see
    /// docs/webview.md.
    static func globMatches(_ pattern: String, _ text: String) -> Bool {
        let p = Array(pattern)
        let t = Array(text)
        var pi = 0
        var ti = 0
        var star: Int?
        var starT = 0
        while ti < t.count {
            if pi < p.count, p[pi] == t[ti] {
                pi += 1
                ti += 1
            } else if pi < p.count, p[pi] == "*" {
                star = pi
                starT = ti
                pi += 1
            } else if let s = star {
                pi = s + 1
                starT += 1
                ti = starT
            } else {
                return false
            }
        }
        while pi < p.count, p[pi] == "*" { pi += 1 }
        return pi == p.count
    }
}

/// Carries one item's identity and the click it belongs to from the NSMenuItem
/// to the event. AppKit menu items need an object target, so this is it.
final class NDContextMenuAction: NSObject, NSMenuItemValidation {
    let id: String
    let kind: NDContextMenuItem.Kind
    let checked: Bool
    let enabled: Bool
    weak var view: NDWebView?

    init(id: String, kind: NDContextMenuItem.Kind, checked: Bool, enabled: Bool, view: NDWebView) {
        self.id = id
        self.kind = kind
        self.checked = checked
        self.enabled = enabled
        self.view = view
    }

    @objc func run(_ sender: NSMenuItem) {
        view?.ndContextMenuItemActivated(id: id, kind: kind, checked: checked)
    }

    /// A contextual menu auto-enables its items by asking the target, so a
    /// disabled item has to say so here, because `isEnabled` alone is
    /// overwritten at display time.
    func validateMenuItem(_ item: NSMenuItem) -> Bool {
        enabled
    }
}

/// WebKit's own menu items carry identifiers, which is the one context signal
/// the UI process has without asking the page. Used as a backstop when the
/// page-side hit report has not landed (or is stale): the strings are WebKit's
/// published item identifiers, compared as text so no private symbol is linked.
enum NDStockMenuContexts {
    static func contexts(of menu: NSMenu) -> NDContextMask {
        var bits: NDContextMask = []
        for item in menu.items {
            switch item.identifier?.rawValue ?? "" {
            case "WKMenuItemIdentifierOpenLink", "WKMenuItemIdentifierOpenLinkInNewWindow",
                 "WKMenuItemIdentifierCopyLink", "WKMenuItemIdentifierDownloadLinkedFile":
                bits.insert(.link)
            case "WKMenuItemIdentifierCopyImage", "WKMenuItemIdentifierDownloadImage",
                 "WKMenuItemIdentifierOpenImageInNewWindow", "WKMenuItemIdentifierRevealImage":
                bits.insert(.image)
            case "WKMenuItemIdentifierCopy", "WKMenuItemIdentifierLookUp",
                 "WKMenuItemIdentifierSearchWeb", "WKMenuItemIdentifierTranslate":
                bits.insert(.selection)
            case "WKMenuItemIdentifierPaste":
                bits.insert(.editable)
            default:
                break
            }
        }
        return bits
    }
}
