import AppKit

// AppKit host-side of the <codeeditor> widget: an NSTextView on TextKit 2,
// syntax attributes applied to its NSTextStorage. GTK peer: src/gtk/codeeditor.zig.
//
// TextKit 2 (`NSTextView(usingTextLayoutManager: true)`) is what the line-number
// ruler reads: the layout fragments it enumerates are the laid-out lines, so the
// gutter tracks scrolling and font size without a second layout pass of its own.
//
// Highlighting is a token pass over the whole storage, not an incremental
// parser. That is the right trade for the sizes an editor widget in a desktop
// app actually holds, and it keeps the one thing that matters honest: the
// attributes are real NSAttributedString runs, so selection, copy, find and
// accessibility all see the same text the user does.
//
// `theme` is "light" | "dark" and forces the view's NSAppearance; every token
// colour is a semantic system colour, so it follows that (or the app's own
// appearance, when theme is unset) instead of hardcoding a palette macOS would
// then contradict.
//
// LSP is out of scope BY DESIGN. The widget renders the `diagnostics` array it
// is handed and never analyses the text — a language server belongs in
// app-side TS, the same division <table> draws between its rows and whatever
// produced them.

// ============================================================================
// State
// ============================================================================

struct NDCodeDiagnostic {
    let line: Int
    let column: Int
    let severity: String
    let message: String
}

final class NDCodeEditorEntry {
    var nodeID: UInt32 = 0
    weak var textView: NDCodeTextView?
    weak var ruler: NDLineNumberRuler?
    var coordinator: NDCodeEditorCoordinator?
    var language = ""
    var tabWidth = 4
    var diagnostics: [NDCodeDiagnostic] = []
    /// Set while an apply writes `text`, so the delegate cannot echo a
    /// React-driven write back as a user edit.
    var suppress = false
}

nonisolated(unsafe) private var ndCodeEditors: [ObjectIdentifier: NDCodeEditorEntry] = [:]

/// release_node purge seam (Backend.swift's `ndPurgeNodeRegistries`).
func ndCodeEditorPurge(_ view: NSView) {
    ndCodeEditors[ObjectIdentifier(view)] = nil
}

private func entry(for view: NSView) -> NDCodeEditorEntry {
    let key = ObjectIdentifier(view)
    if let existing = ndCodeEditors[key] { return existing }
    let fresh = NDCodeEditorEntry()
    ndCodeEditors[key] = fresh
    return fresh
}

// ============================================================================
// Tokenizer
// ============================================================================

/// Keyword sets per language. The app's `language` string is lowercased and
/// looked up here; an unknown language still gets comments, strings, numbers
/// and types, which is most of what makes code readable.
private let ndCodeKeywords: [String: Set<String>] = [
    "swift": ["actor", "as", "associatedtype", "async", "await", "break", "case", "catch", "class", "continue", "default", "defer", "deinit", "do", "else", "enum", "extension", "fallthrough", "false", "final", "for", "func", "guard", "if", "import", "in", "init", "inout", "internal", "is", "let", "nil", "open", "operator", "private", "protocol", "public", "repeat", "return", "self", "static", "struct", "subscript", "super", "switch", "throw", "throws", "true", "try", "typealias", "var", "where", "while"],
    "typescript": ["any", "as", "async", "await", "break", "case", "catch", "class", "const", "continue", "declare", "default", "delete", "do", "else", "enum", "export", "extends", "false", "finally", "for", "from", "function", "if", "implements", "import", "in", "instanceof", "interface", "let", "new", "null", "of", "private", "protected", "public", "readonly", "return", "static", "super", "switch", "this", "throw", "true", "try", "type", "typeof", "undefined", "var", "void", "while", "yield"],
    "javascript": ["async", "await", "break", "case", "catch", "class", "const", "continue", "default", "delete", "do", "else", "export", "extends", "false", "finally", "for", "from", "function", "if", "import", "in", "instanceof", "let", "new", "null", "of", "return", "static", "super", "switch", "this", "throw", "true", "try", "typeof", "undefined", "var", "void", "while", "yield"],
    "rust": ["as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while"],
    "zig": ["align", "and", "anytype", "asm", "break", "catch", "comptime", "const", "continue", "defer", "else", "enum", "errdefer", "error", "export", "extern", "fn", "for", "if", "inline", "or", "orelse", "packed", "pub", "return", "struct", "switch", "test", "try", "union", "unreachable", "usingnamespace", "var", "volatile", "while"],
    "python": ["and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "try", "while", "with", "yield"],
    "go": ["break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for", "func", "go", "goto", "if", "import", "interface", "map", "package", "range", "return", "select", "struct", "switch", "type", "var"],
    "c": ["auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else", "enum", "extern", "float", "for", "goto", "if", "int", "long", "register", "return", "short", "signed", "sizeof", "static", "struct", "switch", "typedef", "union", "unsigned", "void", "volatile", "while"],
    "java": ["abstract", "boolean", "break", "byte", "case", "catch", "char", "class", "const", "continue", "default", "do", "double", "else", "enum", "extends", "final", "finally", "float", "for", "if", "implements", "import", "instanceof", "int", "interface", "long", "new", "package", "private", "protected", "public", "return", "short", "static", "super", "switch", "this", "throw", "throws", "try", "void", "while"],
    "ruby": ["begin", "break", "case", "class", "def", "do", "else", "elsif", "end", "ensure", "false", "for", "if", "in", "module", "nil", "return", "self", "super", "then", "true", "unless", "until", "when", "while", "yield"],
    "shell": ["case", "do", "done", "elif", "else", "esac", "export", "fi", "for", "function", "if", "in", "local", "return", "then", "until", "while"],
]

/// Spellings that differ from the key above. Same intent as the GTK side's
/// alias table: the app writes the name people use, not the id a table uses.
private let ndCodeLanguageAliases: [String: String] = [
    "ts": "typescript", "tsx": "typescript", "js": "javascript", "jsx": "javascript",
    "mjs": "javascript", "cpp": "c", "c++": "c", "objective-c": "c", "cs": "java",
    "py": "python", "rs": "rust", "sh": "shell", "bash": "shell", "zsh": "shell",
]

private func ndCodeKeywordSet(_ language: String) -> Set<String> {
    let key = language.lowercased()
    if let direct = ndCodeKeywords[key] { return direct }
    if let alias = ndCodeLanguageAliases[key], let set = ndCodeKeywords[alias] { return set }
    return []
}

/// Compiled once; NSRegularExpression is expensive to build and these never
/// change.
private let ndCommentPattern = try! NSRegularExpression(
    pattern: "//[^\\n]*|#[^\\n]*|/\\*(?s:.)*?\\*/", options: [])
private let ndStringPattern = try! NSRegularExpression(
    pattern: "\"(?:\\\\.|[^\"\\\\\\n])*\"|'(?:\\\\.|[^'\\\\\\n])*'|`(?:\\\\.|[^`\\\\])*`", options: [])
private let ndNumberPattern = try! NSRegularExpression(
    pattern: "\\b(?:0[xXbBoO][0-9a-fA-F_]+|\\d[\\d_]*(?:\\.\\d[\\d_]*)?(?:[eE][-+]?\\d+)?)\\b", options: [])
private let ndWordPattern = try! NSRegularExpression(pattern: "\\b[A-Za-z_][A-Za-z0-9_]*\\b", options: [])

/// Re-attributes the whole storage. Comments and strings run LAST so they win
/// over a keyword or number that happens to sit inside one.
private func ndCodeHighlight(_ textView: NDCodeTextView, language: String) {
    guard let storage = textView.textStorage else { return }
    let text = storage.string
    let full = NSRange(location: 0, length: (text as NSString).length)
    guard full.length > 0 else { return }

    storage.beginEditing()
    storage.setAttributes([
        .font: textView.font ?? NSFont.monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular),
        .foregroundColor: NSColor.textColor,
    ], range: full)

    let keywords = ndCodeKeywordSet(language)
    ndWordPattern.enumerateMatches(in: text, options: [], range: full) { match, _, _ in
        guard let range = match?.range else { return }
        let word = (text as NSString).substring(with: range)
        if keywords.contains(word) {
            storage.addAttribute(.foregroundColor, value: NSColor.systemPink, range: range)
        } else if let first = word.first, first.isUppercase {
            // A capitalized identifier is a type in every language listed above
            // and harmless everywhere else.
            storage.addAttribute(.foregroundColor, value: NSColor.systemTeal, range: range)
        }
    }
    ndNumberPattern.enumerateMatches(in: text, options: [], range: full) { match, _, _ in
        guard let range = match?.range else { return }
        storage.addAttribute(.foregroundColor, value: NSColor.systemPurple, range: range)
    }
    ndStringPattern.enumerateMatches(in: text, options: [], range: full) { match, _, _ in
        guard let range = match?.range else { return }
        storage.addAttribute(.foregroundColor, value: NSColor.systemRed, range: range)
    }
    ndCommentPattern.enumerateMatches(in: text, options: [], range: full) { match, _, _ in
        guard let range = match?.range else { return }
        storage.addAttribute(.foregroundColor, value: NSColor.systemGreen, range: range)
    }
    storage.endEditing()
}

/// Squiggles, applied after the token pass so a diagnostic is never painted
/// over. The message rides as the range's tooltip, which is where AppKit
/// already shows per-run help.
private func ndCodeApplyDiagnostics(_ textView: NDCodeTextView, _ diagnostics: [NDCodeDiagnostic]) {
    guard let storage = textView.textStorage else { return }
    let ns = storage.string as NSString
    let full = NSRange(location: 0, length: ns.length)
    storage.beginEditing()
    storage.removeAttribute(.underlineStyle, range: full)
    storage.removeAttribute(.underlineColor, range: full)
    storage.removeAttribute(.toolTip, range: full)
    for diagnostic in diagnostics {
        guard let range = ndCodeRange(ns, line: diagnostic.line, column: diagnostic.column) else { continue }
        let color: NSColor = diagnostic.severity == "warning" ? .systemOrange
            : (diagnostic.severity == "error" ? .systemRed : .systemBlue)
        storage.addAttribute(.underlineStyle, value: NSUnderlineStyle.thick.rawValue | NSUnderlineStyle.patternDot.rawValue, range: range)
        storage.addAttribute(.underlineColor, value: color, range: range)
        if !diagnostic.message.isEmpty {
            storage.addAttribute(.toolTip, value: diagnostic.message, range: range)
        }
    }
    storage.endEditing()
}

/// 1-based line/column (what compilers and language servers report) to the
/// character range from that column to the end of the line.
private func ndCodeRange(_ text: NSString, line: Int, column: Int) -> NSRange? {
    guard line >= 1, text.length > 0 else { return nil }
    var index = 0
    var current = 1
    while current < line {
        let lineRange = text.lineRange(for: NSRange(location: index, length: 0))
        let next = NSMaxRange(lineRange)
        if next <= index || next >= text.length { return nil }
        index = next
        current += 1
    }
    let lineRange = text.lineRange(for: NSRange(location: index, length: 0))
    // lineRange includes the newline; the squiggle stops before it.
    var end = NSMaxRange(lineRange)
    while end > lineRange.location, text.character(at: end - 1) == 10 || text.character(at: end - 1) == 13 { end -= 1 }
    let start = min(lineRange.location + max(0, column - 1), end)
    guard end > start else { return nil }
    return NSRange(location: start, length: end - start)
}

/// 1-based line/column of a character index, for cursorMoved and clicks.
private func ndCodePosition(_ text: NSString, at index: Int) -> (line: Int, column: Int) {
    let clamped = max(0, min(index, text.length))
    var line = 1
    var lineStart = 0
    var scan = 0
    while scan < clamped {
        if text.character(at: scan) == 10 {
            line += 1
            lineStart = scan + 1
        }
        scan += 1
    }
    return (line, clamped - lineStart + 1)
}

// ============================================================================
// Views
// ============================================================================

final class NDCodeTextView: NSTextView {
    weak var owner: NSScrollView?

    private var entry: NDCodeEditorEntry? {
        guard let owner else { return nil }
        return ndCodeEditors[ObjectIdentifier(owner)]
    }

    /// diagnosticClicked reports the diagnostic on the clicked LINE, not the
    /// exact glyph: asking the user to hit one character of a squiggle is not
    /// a target anyone can reach.
    override func mouseDown(with event: NSEvent) {
        super.mouseDown(with: event)
        guard let e = entry, e.nodeID != 0, !e.diagnostics.isEmpty else { return }
        let point = convert(event.locationInWindow, from: nil)
        let index = characterIndexForInsertion(at: point)
        let position = ndCodePosition(string as NSString, at: index)
        guard let hit = e.diagnostics.first(where: { $0.line == position.line }) else { return }
        let json = "{\"data\":{\"line\":\(hit.line),\"column\":\(hit.column),"
            + "\"severity\":\"\(ndJsonEscape(hit.severity))\",\"message\":\"\(ndJsonEscape(hit.message))\"}}"
        ndEmitEvent(e.nodeID, "diagnosticClicked", json)
    }
}

/// The gutter. TextKit 2 hands back one layout fragment per laid-out line, so
/// the numbers come from walking those rather than from a second measurement
/// of the text.
final class NDLineNumberRuler: NSRulerView {
    /// Do NOT override `draw(_:)` to paint a gutter background. `NSRulerView`
    /// drives its own drawing through this method, and a `draw` override that
    /// calls it directly instead of going through `super` takes the enclosing
    /// scroll view's text rendering down with it — the editor comes up with
    /// line numbers and no text at all.
    override func drawHashMarksAndLabels(in rect: NSRect) {
        guard let textView = clientView as? NSTextView,
              let layoutManager = textView.textLayoutManager else { return }

        let font = NSFont.monospacedDigitSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular)
        let attributes: [NSAttributedString.Key: Any] = [
            .font: font,
            .foregroundColor: NSColor.secondaryLabelColor,
        ]
        let inset = textView.textContainerInset.height
        let origin = convert(NSPoint.zero, from: textView)
        let visible = scrollView?.contentView.bounds ?? .zero

        var lineNumber = 1
        layoutManager.enumerateTextLayoutFragments(from: nil, options: [.ensuresLayout]) { fragment in
            let frame = fragment.layoutFragmentFrame
            if frame.maxY >= visible.minY, frame.minY <= visible.maxY {
                let label = "\(lineNumber)" as NSString
                let size = label.size(withAttributes: attributes)
                let y = origin.y + frame.minY + inset
                label.draw(at: NSPoint(x: ruleThickness - size.width - 6, y: y), withAttributes: attributes)
            }
            lineNumber += 1
            return frame.minY <= visible.maxY
        }
    }
}

/// NSTextViewDelegate for one editor. `@unchecked Sendable` for the same
/// reason NDDragSourceProxy is: every path in and out is an AppKit callback on
/// the main thread, and the generated dispatcher that builds it is nonisolated.
final class NDCodeEditorCoordinator: NSObject, NSTextViewDelegate, @unchecked Sendable {
    weak var owner: NSScrollView?

    private var entry: NDCodeEditorEntry? {
        guard let owner else { return nil }
        return ndCodeEditors[ObjectIdentifier(owner)]
    }

    func textDidChange(_ notification: Notification) {
        guard let e = entry, let textView = e.textView, !e.suppress else { return }
        ndCodeHighlight(textView, language: e.language)
        ndCodeApplyDiagnostics(textView, e.diagnostics)
        e.ruler?.needsDisplay = true
        guard e.nodeID != 0 else { return }
        ndEmitEvent(e.nodeID, "changed", "{\"text\":\"\(ndJsonEscape(textView.string))\"}")
    }

    func textViewDidChangeSelection(_ notification: Notification) {
        guard let e = entry, let textView = e.textView, e.nodeID != 0, !e.suppress else { return }
        let position = ndCodePosition(textView.string as NSString, at: textView.selectedRange().location)
        ndEmitEvent(e.nodeID, "cursorMoved", "{\"data\":{\"line\":\(position.line),\"column\":\(position.column)}}")
    }
}

// ============================================================================
// Generated-dispatcher seam
// ============================================================================

func ndCodeEditorCreate(_ props: [String: Any]) -> NSView {
    let scroll = NSScrollView()
    scroll.hasVerticalScroller = true
    scroll.hasHorizontalScroller = true
    scroll.autohidesScrollers = true
    // No bezel, unlike `<textarea>`: a code editor is a document surface, not a
    // form field, and neither Xcode nor any other Mac editor draws a field
    // border around one. At the editor's width the bezel's top edge also read
    // as a rule crossing the pane rather than as the edge of a control.
    scroll.borderType = .noBorder
    scroll.drawsBackground = true

    let textView = NDCodeTextView(usingTextLayoutManager: true)
    textView.owner = scroll
    textView.isRichText = false
    // Every macOS text-input convenience is wrong in a code editor: it is not
    // prose, so the only squiggle under `nam` should be the DIAGNOSTIC's, and
    // an autocorrected identifier or a smart quote silently changes what
    // compiles. AppKit defaults several of these from the user's own system
    // settings, so each has to be turned off by name — continuous spell
    // checking especially, which is on for most people.
    textView.isAutomaticQuoteSubstitutionEnabled = false
    textView.isAutomaticDashSubstitutionEnabled = false
    textView.isAutomaticSpellingCorrectionEnabled = false
    textView.isAutomaticTextReplacementEnabled = false
    textView.isAutomaticTextCompletionEnabled = false
    textView.isAutomaticLinkDetectionEnabled = false
    textView.isAutomaticDataDetectionEnabled = false
    textView.isContinuousSpellCheckingEnabled = false
    textView.isGrammarCheckingEnabled = false
    textView.smartInsertDeleteEnabled = false
    textView.allowsUndo = true
    textView.drawsBackground = true
    textView.backgroundColor = .textBackgroundColor
    textView.textContainerInset = NSSize(width: 4, height: 6)
    textView.font = NSFont.monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)
    // Code does not reflow: a wrapped line breaks the column arithmetic every
    // diagnostic and every cursor report is expressed in.
    textView.isHorizontallyResizable = true
    textView.textContainer?.widthTracksTextView = false
    textView.textContainer?.size = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
    scroll.documentView = textView

    let ruler = NDLineNumberRuler(scrollView: scroll, orientation: .verticalRuler)
    ruler.clientView = textView
    ruler.ruleThickness = 40
    scroll.verticalRulerView = ruler
    scroll.hasVerticalRuler = true

    // An NSScrollView reports no intrinsic size, so an editor in a
    // content-sized column laid out at zero height and rendered nothing.
    // GTK's peer propagates the document's natural height; the AppKit idiom
    // is TextArea's floor, at the same priority.
    scroll.frame.size.height = 120
    let floor_ = scroll.heightAnchor.constraint(greaterThanOrEqualToConstant: 120)
    floor_.priority = NSLayoutConstraint.Priority(999)
    floor_.isActive = true

    let e = entry(for: scroll)
    e.textView = textView
    e.ruler = ruler
    let coordinator = NDCodeEditorCoordinator()
    coordinator.owner = scroll
    textView.delegate = coordinator
    e.coordinator = coordinator

    ndCodeEditorApply(scroll, props)
    return scroll
}

func ndCodeEditorApply(_ view: NSView, _ props: [String: Any]) {
    guard let scroll = view as? NSScrollView,
          let e = ndCodeEditors[ObjectIdentifier(scroll)],
          let textView = e.textView else { return }

    if let theme = propStr(props, "theme") {
        // "light"/"dark" force the view's appearance; anything else (including
        // "") hands the decision back to the app's own appearance, which is
        // what every token colour below is keyed to.
        switch theme.lowercased() {
        case "light": scroll.appearance = NSAppearance(named: .aqua)
        case "dark": scroll.appearance = NSAppearance(named: .darkAqua)
        default: scroll.appearance = nil
        }
    }
    if let language = propStr(props, "language") { e.language = language }
    if let readOnly = propBool(props, "readOnly") {
        textView.isEditable = !readOnly
        textView.isSelectable = true
    }
    if let width = propInt(props, "tabWidth") {
        e.tabWidth = max(1, width)
        let advance = ("0" as NSString).size(withAttributes: [.font: textView.font as Any]).width
        let style = NSMutableParagraphStyle()
        style.tabStops = []
        style.defaultTabInterval = advance * CGFloat(e.tabWidth)
        textView.defaultParagraphStyle = style
        textView.textStorage?.addAttribute(
            .paragraphStyle, value: style,
            range: NSRange(location: 0, length: (textView.string as NSString).length))
    }
    if let show = propBool(props, "showLineNumbers") {
        scroll.rulersVisible = show
    }
    if let diagnostics = props["diagnostics"] as? [[String: Any]] {
        e.diagnostics = diagnostics.map {
            NDCodeDiagnostic(
                line: propInt($0, "line") ?? 1,
                column: propInt($0, "column") ?? 1,
                severity: propStr($0, "severity") ?? "error",
                message: propStr($0, "message") ?? "")
        }
    }
    if let text = propStr(props, "text"), text != textView.string {
        e.suppress = true
        textView.string = text
        e.suppress = false
    }
    ndCodeHighlight(textView, language: e.language)
    ndCodeApplyDiagnostics(textView, e.diagnostics)
    e.ruler?.needsDisplay = true
}

func ndCodeEditorConnect(_ view: NSView, nodeID: UInt32) {
    guard let e = ndCodeEditors[ObjectIdentifier(view)] else { return }
    e.nodeID = nodeID
}
