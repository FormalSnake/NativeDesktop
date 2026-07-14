import AppKit
import CNd

// Must outlive main(): the core stores &gVTable and calls through it for the
// process's whole life (mirrors src/gtk/main.zig's module-level `the_vtable`
// and this file's T1 predecessor, buildStubVTable). `gWindow`/`gCtx` are
// declared in main.swift.

/// Rebuilds an `NSView` from a raw vtable handle. Handles ride the ABI as
/// retained `Unmanaged<NSView>` pointers (`create` calls `passRetained`;
/// every other op here calls `takeUnretainedValue` — the core owns the
/// retain for the node's lifetime, balanced by `unparent`/GC, same
/// contract as `src/gtk/backend.zig`'s widget handles).
@inline(__always) private func viewFrom(_ p: UnsafeMutableRawPointer?) -> NSView {
    Unmanaged<NSView>.fromOpaque(p!).takeUnretainedValue()
}
@inline(__always) private func cstr(_ p: UnsafePointer<CChar>?) -> String {
    p.map { String(cString: $0) } ?? ""
}

/// The real AppKit backend (T3), replacing T1's `buildStubVTable`. Every
/// `@convention(c)` closure decodes its raw-pointer/C-string args and calls
/// into the generated `NDGen.Widgets` dispatcher — never a narrower
/// concrete type at this layer (the generated dispatcher owns per-kind
/// casts, mirroring `src/gtk/backend.zig`'s `vt*` wrappers).
///
/// Swift 6 strict concurrency: raw pointers cross into `MainActor
/// .assumeIsolated` closures as `Int` bit patterns (capturing
/// `UnsafeMutableRawPointer` directly trips the sending-risk check even
/// though the ABI guarantees every call arrives on the UI thread already —
/// same pattern main.swift's T1 stub established). C strings are decoded to
/// Swift `String` *before* entering the isolated closure.
func buildVTable() -> nd_backend {
    var vt = nd_backend()

    vt.create = { _, kind, propsJson in
        let kindStr = cstr(kind)
        let propsStr = cstr(propsJson)
        let bits: Int? = MainActor.assumeIsolated {
            guard let v = ndCreate(kindStr, propsStr) else { return nil }
            ndApplyTestID(v, propsStr)
            ndApplyCssClassesIfPresent(v, propsStr)
            ndRecordButtonKind(v, kindStr)
            return Int(bitPattern: Unmanaged.passRetained(v).toOpaque())
        }
        guard let bits else { return nil }
        return UnsafeMutableRawPointer(bitPattern: bits)
    }

    vt.apply_props = { _, w, kind, propsJson in
        let widgetBits = Int(bitPattern: w)
        let kindStr = cstr(kind)
        let propsStr = cstr(propsJson)
        MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return }
            let view = viewFrom(widgetPtr)
            ndApplyProps(view, kindStr, propsStr)
            ndApplyTestID(view, propsStr)
            ndApplyCssClassesIfPresent(view, propsStr)
        }
    }

    vt.append_child = { _, parent, pkind, child, attachedJson in
        let parentBits = Int(bitPattern: parent)
        let childBits = Int(bitPattern: child)
        let pkindStr = cstr(pkind)
        let attachedStr = cstr(attachedJson)
        MainActor.assumeIsolated {
            guard let parentPtr = UnsafeMutableRawPointer(bitPattern: parentBits),
                  let childPtr = UnsafeMutableRawPointer(bitPattern: childBits) else { return }
            ndAppendChild(viewFrom(parentPtr), pkindStr, viewFrom(childPtr), attachedStr)
        }
    }

    vt.insert_before = { _, parent, pkind, child, before, attachedJson in
        let parentBits = Int(bitPattern: parent)
        let childBits = Int(bitPattern: child)
        let beforeBits = Int(bitPattern: before)
        let pkindStr = cstr(pkind)
        let attachedStr = cstr(attachedJson)
        MainActor.assumeIsolated {
            guard let parentPtr = UnsafeMutableRawPointer(bitPattern: parentBits),
                  let childPtr = UnsafeMutableRawPointer(bitPattern: childBits) else { return }
            let beforeView = UnsafeMutableRawPointer(bitPattern: beforeBits).map {
                Unmanaged<NSView>.fromOpaque($0).takeUnretainedValue()
            }
            ndInsertBefore(viewFrom(parentPtr), pkindStr, viewFrom(childPtr), beforeView, attachedStr)
        }
    }

    vt.remove_child = { _, parent, pkind, child in
        let parentBits = Int(bitPattern: parent)
        let childBits = Int(bitPattern: child)
        let pkindStr = cstr(pkind)
        MainActor.assumeIsolated {
            guard let parentPtr = UnsafeMutableRawPointer(bitPattern: parentBits),
                  let childPtr = UnsafeMutableRawPointer(bitPattern: childBits) else { return }
            ndRemoveChild(viewFrom(parentPtr), pkindStr, viewFrom(childPtr))
        }
    }

    vt.set_text = { _, w, text in
        let widgetBits = Int(bitPattern: w)
        let textStr = cstr(text)
        MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return }
            if let f = viewFrom(widgetPtr) as? NSTextField { f.stringValue = textStr }
        }
    }

    vt.set_visible = { _, w, visible in
        let widgetBits = Int(bitPattern: w)
        MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return }
            viewFrom(widgetPtr).isHidden = !visible
        }
    }

    vt.apply_style = { _, w, nodeID, styleJson in
        let widgetBits = Int(bitPattern: w)
        let styleStr = cstr(styleJson)
        MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return }
            ndApplyStyle(viewFrom(widgetPtr), nodeID, styleStr)
        }
    }

    vt.connect_events = { _, w, kind, nodeID in
        let widgetBits = Int(bitPattern: w)
        let kindStr = cstr(kind)
        MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return }
            ndConnectEvents(viewFrom(widgetPtr), kindStr, nodeID)
        }
    }

    vt.has_parent = { _, w in
        let widgetBits = Int(bitPattern: w)
        return MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return false }
            return viewFrom(widgetPtr).superview != nil
        }
    }

    vt.unparent = { _, w in
        let widgetBits = Int(bitPattern: w)
        MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return }
            viewFrom(widgetPtr).removeFromSuperview()
        }
    }

    vt.get_window = { _ in
        let bits: Int? = MainActor.assumeIsolated {
            // M11 Phase C (Risk 1): src/tree.zig's post-crash respawn path
            // binds a fresh reconciler root's "Window" node to whatever this
            // returns (backend.getWindow()), so it must resolve to the
            // CURRENT live content (SplitController.swift's ndLiveContentView),
            // not the Window create arm's FlippedView, which is orphaned
            // once a SplitView becomes contentViewController.
            guard let content = ndLiveContentView() else { return nil }
            return Int(bitPattern: Unmanaged.passUnretained(content).toOpaque())
        }
        guard let bits else { return nil }
        return UnsafeMutableRawPointer(bitPattern: bits)
    }

    vt.marshal_async = { _, fn, data in
        // dispatch_async_f: C fn ptr onto the main queue. NEVER dispatch_sync.
        guard let fn else { return }
        let bits = Int(bitPattern: data)
        DispatchQueue.main.async { fn(UnsafeMutableRawPointer(bitPattern: bits)) }
    }

    vt.show_overlay = { _, message in
        let messageStr = cstr(message)
        MainActor.assumeIsolated {
            ndShowOverlay(messageStr)
        }
    }

    // node_visible / node_bounds / snapshot / semantic_action (Task 5): the
    // automation half of the vtable, implemented in Automation.swift —
    // AppKit peers of src/gtk/backend.zig's vtNodeVisible/vtNodeBounds/
    // vtSnapshot/vtSemanticAction.
    vt.node_visible = { _, w in
        let widgetBits = Int(bitPattern: w)
        return MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return false }
            return ndNodeVisible(viewFrom(widgetPtr))
        }
    }

    vt.node_bounds = { _, w, out in
        let widgetBits = Int(bitPattern: w)
        let outBits = Int(bitPattern: out)
        return MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits),
                  let outPtr = UnsafeMutableRawPointer(bitPattern: outBits) else { return false }
            var rect = nd_rect()
            guard ndNodeBounds(viewFrom(widgetPtr), &rect) else { return false }
            outPtr.assumingMemoryBound(to: nd_rect.self).pointee = rect
            return true
        }
    }

    vt.snapshot = { _, pngPath in
        let pathStr = cstr(pngPath)
        return MainActor.assumeIsolated {
            ndSnapshot(pathStr)
        }
    }

    vt.semantic_action = { _, w, nodeID, action, argJson, resultOut, errOut in
        let widgetBits = Int(bitPattern: w)
        let actionStr = cstr(action)
        let argStr = cstr(argJson)
        let resultOutBits = Int(bitPattern: resultOut)
        let errOutBits = Int(bitPattern: errOut)
        return MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return -32601 }
            let resultOutPtr = UnsafeMutableRawPointer(bitPattern: resultOutBits)?
                .assumingMemoryBound(to: UnsafeMutablePointer<CChar>?.self)
            let errOutPtr = UnsafeMutableRawPointer(bitPattern: errOutBits)?
                .assumingMemoryBound(to: UnsafeMutablePointer<CChar>?.self)
            return ndSemanticAction(viewFrom(widgetPtr), nodeID, actionStr, argStr, resultOutPtr, errOutPtr)
        }
    }

    vt.widget_command = { _, w, kind, command, argJson in
        let widgetBits = Int(bitPattern: w)
        let kindStr = cstr(kind)
        let commandStr = cstr(command)
        let argStr = cstr(argJson)
        MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return }
            ndWidgetCommand(viewFrom(widgetPtr), kindStr, commandStr, argStr)
        }
    }

    // Multi-window reconstruction: resolve a Window node's handle to its
    // window's CURRENT live content view (the create arm's FlippedView is
    // orphaned once a SplitView becomes contentViewController — see
    // `ndLiveContentView(for:)`). Mirrors get_window's passUnretained handling:
    // the returned view is retained by the live hierarchy.
    vt.resolve_window = { _, handle in
        let handleBits = Int(bitPattern: handle)
        let bits: Int? = MainActor.assumeIsolated {
            guard let handlePtr = UnsafeMutableRawPointer(bitPattern: handleBits) else { return nil }
            guard let live = ndLiveContentView(for: viewFrom(handlePtr)) else { return nil }
            // Doubles as the snapshot-target selector (multi-window): a
            // screenshot's selectSnapshotWindow resolves its target window here
            // right before calling snapshot, so record the resolved content view
            // for ndSnapshot to render. Reconstruction also lands here, harmless
            // — the value is refreshed before every screenshot and consumed once.
            ndSnapshotTargetContent = live
            return Int(bitPattern: Unmanaged.passUnretained(live).toOpaque())
        }
        guard let bits else { return nil }
        return UnsafeMutableRawPointer(bitPattern: bits)
    }

    // Widget-preserving cross-window move (drag a tab between windows): relocate
    // a live NSView from `oldParent` to `newParent` without recreating it, so a
    // WKWebView keeps its loaded page / scroll / JS state. The core holds the
    // create-time `passRetained` +1 on every node (see `viewFrom`), so
    // `removeFromSuperview` inside `ndRemoveChild` never deallocates the view —
    // no explicit AppKit retain is needed across the move (unlike GTK). Reuses
    // the ordinary per-kind remove + insert dispatchers so the target attaches
    // correctly whatever the parent kind. `oldParent`/`before` are nullable.
    vt.reparent_child = { _, child, oldParent, oldKind, newParent, newKind, before, attachedJson in
        let childBits = Int(bitPattern: child)
        let oldParentBits = Int(bitPattern: oldParent)
        let newParentBits = Int(bitPattern: newParent)
        let beforeBits = Int(bitPattern: before)
        let oldKindStr = cstr(oldKind)
        let newKindStr = cstr(newKind)
        let attachedStr = cstr(attachedJson)
        MainActor.assumeIsolated {
            guard let childPtr = UnsafeMutableRawPointer(bitPattern: childBits),
                  let newParentPtr = UnsafeMutableRawPointer(bitPattern: newParentBits) else { return }
            let childView = viewFrom(childPtr)
            if let oldParentPtr = UnsafeMutableRawPointer(bitPattern: oldParentBits) {
                ndRemoveChild(viewFrom(oldParentPtr), oldKindStr, childView)
            } else {
                childView.removeFromSuperview()
            }
            let beforeView = UnsafeMutableRawPointer(bitPattern: beforeBits).map {
                Unmanaged<NSView>.fromOpaque($0).takeUnretainedValue()
            }
            ndInsertBefore(viewFrom(newParentPtr), newKindStr, childView, beforeView, attachedStr)
        }
    }

    // System-capability seam (vtable #22): dialogs/clipboard/notifications/
    // recent/credentials. Fire-and-forget — `NDSystem.handleRequest` answers
    // each request with exactly one `nd_system_response`. C strings decoded
    // before entering MainActor isolation, mirroring every closure above.
    vt.system_request = { _, id, method, params in
        let methodStr = cstr(method)
        let paramsStr = cstr(params)
        MainActor.assumeIsolated {
            NDSystem.handleRequest(id: id, method: methodStr, paramsJson: paramsStr)
        }
    }

    return vt
}

/// testIDs (Task 5): mirrors the tracked `testID` prop onto AppKit's own
/// accessibility identifier for real-user/VoiceOver parity. This is NOT the
/// getTree `testID` source — that flows from core-owned `Tree.meta`
/// (src/tree.zig) independent of AppKit entirely — so a missing/absent
/// testID here is a no-op, never an error. Called from both `create` (after
/// the initial build) and `apply_props` (testID can arrive/change on any
/// update, same as every other prop).
func ndApplyTestID(_ view: NSView, _ propsJson: String) {
    guard let testID = propStr(parseProps(propsJson), "testID") else { return }
    view.setAccessibilityIdentifier(testID)
}

/// cssClasses (Task 6): decodes `props.cssClasses` — Task 2's validated
/// Adwaita-class allowlist, riding in the ordinary props JSON rather than a
/// dedicated vtable field (the C-ABI vtable stays minimal) — and
/// applies AppKit's mapped subset via `ndApplyCssClasses`. Called from both
/// `create` and `apply_props`, mirroring `ndApplyTestID`.
func ndApplyCssClassesIfPresent(_ view: NSView, _ propsJson: String) {
    guard let classes = propArray(parseProps(propsJson), "cssClasses") else { return }
    ndApplyCssClasses(view, classes)
}

/// Per-node typography cascade state — the font/color a node's text target
/// should end up with once classes AND style are both accounted for. Keyed
/// by the OUTER widget view's identity (the vtable handle), not the
/// resolved text target, using the same `nonisolated(unsafe)` map idiom as
/// `gridCells`/`ndLayoutFlags`. `nil` means "absent from the last props/style
/// JSON seen", not "unset" — that's what makes dropping a key a real
/// set-replace instead of a no-op. Entries are never pruned, same accepted
/// leak profile as `ndLayoutFlags` (bounded by live widget count).
private struct NDTypography {
    var classes: [String] = []
    var fontObj: [String: Any]? = nil
    var colorStr: String? = nil
}
nonisolated(unsafe) private var ndNodeTypography: [ObjectIdentifier: NDTypography] = [:]

/// Box views (`NSStackView`) carrying the `navigation-sidebar` structural
/// class. On the Mac each is backed by a real source-list `NSTableView`
/// (SidebarTable.swift) — its child buttons become the table's row model —
/// giving native accent/focus selection, row metrics, and font for free
/// instead of a stack of generic push buttons. Set-replace like
/// `ndNodeTypography`: inserted when the class is present, removed when it
/// drops. NOT private — read from `ndBoxChildAttached` (Layout.swift) at
/// child-attach time. Same accepted leak profile as `ndLayoutFlags` (bounded
/// by live widget count).
nonisolated(unsafe) var ndNavigationSidebars: Set<ObjectIdentifier> = []

/// Box views (`NSStackView`) carrying the `boxed-list` structural class. They
/// render as iOS-style grouped inset cards on the Mac (a rounded, elevated
/// rounded-rect over the pane) and their `<separator>` children become inset
/// hairline row dividers — see `ndApplyBoxedListCard` / `ndStyleBoxedListDivider`.
/// Set-replace and NOT private for the same reasons as `ndNavigationSidebars`
/// (read from `ndBoxChildAttached` in Layout.swift).
nonisolated(unsafe) var ndBoxedLists: Set<ObjectIdentifier> = []

/// The native background `NSBox` that draws each `boxed-list` box's grouped
/// card (keyed by the box's identity), tracked so it's reused on re-apply and
/// removed when the class drops. See `ndApplyBoxedListCard`.
nonisolated(unsafe) private var ndBoxedListBackings: [ObjectIdentifier: NSBox] = [:]

/// `ndApplyCssClasses` (Task 6 — a real semantic mapping, not a no-op): maps
/// the Adwaita/GTK classes AppKit has a natural equivalent for onto control
/// properties. Every color used is a dynamic system color
/// (`.controlAccentColor`, `.secondaryLabelColor`, ...) rather than a
/// hardcoded hex value, so dark mode keeps working automatically.
///
/// Button-bezel classes (`suggested-action`/`destructive-action`/`pill`/
/// `flat`) are applied directly below, set-replace against a baseline reset
/// (a class dropped from the list must clear its effect — e.g. a former
/// "suggested-action" no longer leaves keyEquivalent="\r"/an accent bezel, a
/// former "flat" gets its border back; baselines match a freshly-created
/// control from NDGen/Widgets.swift's create arms).
///
/// Typography classes (`title-*`/`heading`/`caption*`/`body`/`dimmed`/
/// `monospace`/`numeric`) are NOT applied here — they're recorded into the
/// per-node `ndNodeTypography` registry and replayed by
/// `ndRecomputeTypography`, which also layers in any standing `style` font/
/// color (see that function's doc comment for the full cascade order).
///
/// Most structural classes (`card`, `view`, `toolbar`, `osd`, ...) are
/// silently ignored — those roles come from the SplitView/HeaderBar widgets
/// themselves on the Mac, not from class strings. `navigation-sidebar` and
/// `boxed-list` are the exceptions: recorded into their registries and turned
/// into a real source-list `NSTableView` (SidebarTable.swift) and a native
/// grouped `NSBox` card respectively — so the box-of-flat-buttons sidebar and
/// boxed-list forms the app declares render natively on both backends without
/// any per-platform app code.
func ndApplyCssClasses(_ view: NSView, _ classes: [String]) {
    // Set-replace, not additive (mirrors GTK's applyCssClasses, which removes
    // every allowlist class not in `value`): reset the button properties the
    // switch below can touch to their baseline FIRST, so a class dropped from
    // the list actually clears its effect.
    if let btn = view as? NSButton {
        btn.bezelColor = nil
        btn.keyEquivalent = ""
        btn.hasDestructiveAction = false
        btn.isBordered = true
        btn.showsBorderOnlyWhileMouseInside = false
    }

    for cls in classes {
        switch cls {
        case "suggested-action":
            guard let btn = view as? NSButton else { continue }
            // A sidebar row's selection is the source-list table's, not a
            // default-button bezel/keyEquivalent (a hidden row must not become
            // the window's Enter-key default button).
            if ndSidebarRowButtons.contains(ObjectIdentifier(btn)) { break }
            btn.bezelColor = .controlAccentColor
            btn.keyEquivalent = "\r"
        case "destructive-action":
            guard let btn = view as? NSButton else { continue }
            btn.bezelColor = .systemRed
            btn.hasDestructiveAction = true
        case "pill":
            // Modern AppKit buttons are already rounded — no layer hacks.
            break
        case "flat":
            guard let btn = view as? NSButton else { continue }
            // A sidebar row is a hidden table-row provider — `flat` is a no-op.
            if ndSidebarRowButtons.contains(ObjectIdentifier(btn)) { break }
            btn.isBordered = false
            btn.showsBorderOnlyWhileMouseInside = true
        default:
            // Typography classes are handled by ndRecomputeTypography below,
            // not this switch. `navigation-sidebar`/`boxed-list` are structural
            // and handled in the NSStackView blocks after this loop. The rest
            // (card, view, toolbar, osd, ...) are roles owned by the
            // SplitView/HeaderBar widgets on the Mac — silently ignored here.
            break
        }
    }

    // Record the `navigation-sidebar` structural class (set-replace) and back
    // the box with a source-list NSTableView (SidebarTable.swift). Install
    // captures whatever button children already attached before the class
    // landed (either create order — see Layout.swift's header comment).
    if let stack = view as? NSStackView {
        let id = ObjectIdentifier(stack)
        if classes.contains("navigation-sidebar") {
            ndNavigationSidebars.insert(id)
            ndInstallSidebarTable(stack)
        } else if ndNavigationSidebars.remove(id) != nil {
            ndRemoveSidebarTable(stack)
        }
    }

    // A sidebar row's `suggested-action` changed (navigation): re-align the
    // table's native selection with it. The button's box is its superview.
    if let btn = view as? NDButton, ndSidebarRowButtons.contains(ObjectIdentifier(btn)),
       let stack = btn.superview as? NSStackView, let table = ndSidebarTables[ObjectIdentifier(stack)] {
        table.syncSelection()
    }

    // Record `boxed-list` (set-replace) and apply the grouped-card treatment,
    // retro-styling any `<separator>` children that already attached before
    // the class landed (either create order — see Layout.swift's header).
    if let stack = view as? NSStackView {
        let id = ObjectIdentifier(stack)
        if classes.contains("boxed-list") {
            ndBoxedLists.insert(id)
            ndApplyBoxedListCard(stack, enabled: true)
            for sub in stack.arrangedSubviews {
                if let sep = sub as? NSBox { ndStyleBoxedListDivider(sep, in: stack) }
            }
        } else if ndBoxedLists.remove(id) != nil {
            ndApplyBoxedListCard(stack, enabled: false)
            for sub in stack.arrangedSubviews {
                if let sep = sub as? NSBox { ndUnstyleBoxedListDivider(sep, in: stack) }
            }
        }
    }

    var typography = ndNodeTypography[ObjectIdentifier(view)] ?? NDTypography()
    typography.classes = classes
    ndNodeTypography[ObjectIdentifier(view)] = typography
    ndRecomputeTypography(view)
}

/// The standing `cssClasses` recorded for a node's text target (read by
/// `NDSidebarTable` to find the `suggested-action` selected row). Kept here so
/// `ndNodeTypography` stays private to this file.
func ndCssClasses(of view: NSView) -> [String] {
    ndNodeTypography[ObjectIdentifier(view)]?.classes ?? []
}

/// Grouped-card treatment for a `boxed-list` box: a native `NSBox` drawn
/// behind the stack's rows, `boxType = .custom`, `cornerRadius ≈ 10`, no
/// border, with a dynamic semantic `fillColor` (`underPageBackgroundColor` —
/// the one of the panel semantics that reads distinct from this split's pane in
/// both light and dark). `NSBox.fillColor` is a real dynamic color that redraws
/// on an appearance change, so light/dark track for free with no CALayer
/// cgColor to hand-resolve. `enabled` false removes the backing NSBox
/// (set-replace when the class drops).
func ndApplyBoxedListCard(_ box: NSView, enabled: Bool) {
    guard let stack = box as? NSStackView else { return }
    let id = ObjectIdentifier(stack)
    guard enabled else {
        if let backing = ndBoxedListBackings[id] {
            backing.removeFromSuperview()
            ndBoxedListBackings[id] = nil
        }
        return
    }
    // A native NSBox drawn behind the stack's rows IS the card — its
    // `fillColor` takes a dynamic semantic NSColor and redraws itself on an
    // appearance change, so light/dark track for free with no CALayer cgColor
    // to hand-resolve. Added as a non-arranged background subview pinned to the
    // stack's bounds (the rows are inset inside it by their own padding).
    let backing: NSBox
    if let existing = ndBoxedListBackings[id] {
        backing = existing
    } else {
        backing = NSBox()
        backing.boxType = .custom
        backing.borderWidth = 0
        backing.borderColor = .clear
        backing.cornerRadius = 10
        backing.titlePosition = .noTitle
        backing.contentViewMargins = .zero
        backing.translatesAutoresizingMaskIntoConstraints = false
        stack.addSubview(backing, positioned: .below, relativeTo: nil)
        NSLayoutConstraint.activate([
            backing.leadingAnchor.constraint(equalTo: stack.leadingAnchor),
            backing.trailingAnchor.constraint(equalTo: stack.trailingAnchor),
            backing.topAnchor.constraint(equalTo: stack.topAnchor),
            backing.bottomAnchor.constraint(equalTo: stack.bottomAnchor),
        ])
        ndBoxedListBackings[id] = backing
    }
    backing.fillColor = .underPageBackgroundColor
}

/// Recomputes the full per-node typography cascade for `view`'s text target
/// from its stored `ndNodeTypography` state: baseline -> class typography (in
/// stored order) -> style font -> style color. Style beats classes (GTK
/// parity: a node's scoped `.nd-<id>` CSS block is APPLICATION priority,
/// which beats theme classes there, so `style` wins over `cssClasses` here
/// too). Called after every write to the registry, from both
/// `ndApplyCssClasses` and `ndApplyStyle`, so whichever one runs last always
/// replays the COMPLETE standing state instead of leaving the other's
/// contribution stale — this is what makes a cssClasses-only update stop
/// wiping a standing style font, and a style-only update stop leaving stale
/// class typography behind.
///
/// TextArea/ScrollView widgets are `NSScrollView` wrappers around an
/// `NSTextView` document view — the baseline and class steps below target
/// that inner text view, not the scroll view itself; the style steps call
/// `applyFont`/`applyTextColor`, which resolve the same way internally (and
/// also handle NSButton's attributedTitle color path).
func ndRecomputeTypography(_ view: NSView) {
    let textTarget: NSView = {
        if let scrollView = view as? NSScrollView, let textView = scrollView.documentView as? NSTextView {
            return textView
        }
        return view
    }()
    let typography = ndNodeTypography[ObjectIdentifier(view)] ?? NDTypography()

    applyCssFontValue(textTarget, .systemFont(ofSize: NSFont.systemFontSize))
    applyCssTextColor(textTarget, .labelColor)

    for cls in typography.classes {
        switch cls {
        case "title-1":
            applyCssFont(textTarget, .largeTitle)
        case "title-2":
            applyCssFont(textTarget, .title1)
        case "title-3":
            applyCssFont(textTarget, .title2)
        case "title-4":
            applyCssFont(textTarget, .title3)
        case "heading":
            applyCssFont(textTarget, .headline)
        case "caption":
            applyCssFont(textTarget, .caption1)
        case "caption-heading":
            applyCssFont(textTarget, .caption2)
        case "body":
            applyCssFont(textTarget, .body)
        case "dimmed":
            applyCssTextColor(textTarget, .secondaryLabelColor)
        case "monospace":
            applyCssFontValue(textTarget, .monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular))
        case "numeric":
            applyCssFontValue(textTarget, .monospacedDigitSystemFont(ofSize: NSFont.systemFontSize, weight: .regular))
        default:
            break
        }
    }

    if let fontObj = typography.fontObj {
        applyFont(view, fontObj)
    }
    if let colorStr = typography.colorStr, let color = nsColor(fromHexOrName: colorStr) {
        applyTextColor(view, color)
    }
}

private func applyCssFont(_ view: NSView, _ style: NSFont.TextStyle) {
    applyCssFontValue(view, .preferredFont(forTextStyle: style, options: [:]))
}

private func applyCssFontValue(_ view: NSView, _ font: NSFont) {
    if let field = view as? NSTextField {
        field.font = font
    } else if let textView = view as? NSTextView {
        textView.font = font
    }
}

private func applyCssTextColor(_ view: NSView, _ color: NSColor) {
    if let field = view as? NSTextField {
        field.textColor = color
    } else if let textView = view as? NSTextView {
        textView.textColor = color
    }
}

/// `ndApplyStyle` (peer of GTK's `style.applyStyle`; AppKit styling is
/// `NSColor`/`NSFont`/Auto Layout, not CSS — M6a-D5's reasoning kept
/// `style.zig` GTK-only). Decodes the `style` JSON object per M6b-D3's
/// style-key set (background/color/font/padding/margin/border/hexpand/
/// vexpand/halign/valign). Best-effort choices, documented per key:
///  - `background` -> `layer.backgroundColor` (forces `wantsLayer = true`).
///  - `color` -> the text color of the nearest text-bearing control
///    (NSTextField/NSButton/NSTextView-in-scrollview).
///  - `font` -> `NSFont` on the same text-bearing controls (fontSize/
///    fontFamily/fontWeight).
///  - `border` -> `layer.borderWidth`/`borderColor`/`cornerRadius`.
///  - `padding` -> dispatched by view type; see `applyPadding`.
///  - `margin` -> silently ignored on AppKit v1. GTK widget margins are a
///    per-child gap the PARENT leaves around this widget; NSStackView has no
///    per-arranged-subview margin equivalent (only the stack's own
///    `edgeInsets`, which is `padding`'s job). A prior version approximated
///    this by mutating the view's frame post-layout, which fought Auto
///    Layout on every subsequent pass — removed rather than kept as a lie.
///  - `hexpand`/`vexpand`/`halign`/`valign` -> recorded into `ndLayoutFlags`
///    (Layout.swift) and reconciled against the parent NSStackView, if any,
///    via `ndBoxChildAttached`.
func ndApplyStyle(_ view: NSView, _ nodeID: UInt32, _ styleJson: String) {
    let style = parseProps(styleJson)

    // `background`/`border`/`padding` are the CSS-target keys GTK rebuilds
    // wholesale per apply (src/gtk/style.zig compileCss regenerates the
    // node's whole scoped CSS block from the CURRENT style object, so a
    // dropped key reverts to baseline there). Mirror that: write each one's
    // FULL effective state every apply — absent sub-keys fall back to
    // baseline — instead of only writing the sub-keys present in the JSON.
    // Additive-only writes leave residue (e.g. an amber pin border that
    // never clears because unpinning's style object simply omits
    // borderColor/borderWidth).
    //
    // `color`/`font` are also CSS-target on GTK, and feed the same per-node
    // typography cascade `ndApplyCssClasses` writes into (`ndNodeTypography`,
    // keyed off this view's identity): store the raw style value below —
    // `nil` when the key is absent, which IS the set-replace signal, not a
    // no-op — then let `ndRecomputeTypography` replay baseline -> classes ->
    // style in full. That makes this function and `ndApplyCssClasses`
    // order-independent: whichever one runs last recomputes from the
    // complete standing state, so a style-only update no longer leaves stale
    // class typography behind, and a cssClasses-only update no longer wipes
    // a standing style font (see `ndRecomputeTypography`'s doc comment for
    // the full cascade order).
    // (The `boxed-list` card fill lives on its own background NSBox subview and
    // navigation-sidebar rows are hidden table-row providers, so neither needs a
    // guard here — the `background`/`border` blocks run unconditionally.)
    if let bg = style["background"] as? String, let color = nsColor(fromHexOrName: bg) {
        view.wantsLayer = true
        view.layer?.backgroundColor = color.cgColor
    } else if let layer = view.layer {
        layer.backgroundColor = nil
    }
    var typography = ndNodeTypography[ObjectIdentifier(view)] ?? NDTypography()
    typography.fontObj = style["font"] as? [String: Any]
    typography.colorStr = style["color"] as? String
    ndNodeTypography[ObjectIdentifier(view)] = typography
    ndRecomputeTypography(view)
    let borderObj = style["border"] as? [String: Any]
    let borderWidth = (borderObj?["borderWidth"] as? NSNumber)?.doubleValue ?? 0
    let borderColor = (borderObj?["borderColor"] as? String).flatMap { nsColor(fromHexOrName: $0) }
    let borderRadius = (borderObj?["borderRadius"] as? NSNumber)?.doubleValue ?? 0
    if borderWidth != 0 || borderColor != nil || borderRadius != 0 {
        view.wantsLayer = true
        view.layer?.borderWidth = CGFloat(borderWidth)
        view.layer?.borderColor = borderColor?.cgColor
        view.layer?.cornerRadius = CGFloat(borderRadius)
    } else if let layer = view.layer {
        layer.borderWidth = 0
        layer.borderColor = nil
        layer.cornerRadius = 0
    }
    // NSEdgeInsets() (all-zero) is the baseline when `padding` drops out of
    // the style object, so this always runs. (Aside: NDButton's ndPadding
    // didSet only switches its bezel TO `.flexiblePush`, never back — a
    // pre-existing one-way behavior, left as-is here.)
    let insets = style["padding"].flatMap(parseEdgeInsets) ?? NSEdgeInsets()
    applyPadding(view, insets)
    // `margin`: silently ignored on AppKit v1 (see doc comment above).

    if style["hexpand"] != nil || style["vexpand"] != nil || style["halign"] != nil || style["valign"] != nil {
        var flags = ndLayoutFlags[ObjectIdentifier(view)] ?? NDLayoutFlags()
        if let h = (style["hexpand"] as? NSNumber)?.boolValue { flags.hexpand = h }
        if let v = (style["vexpand"] as? NSNumber)?.boolValue { flags.vexpand = v }
        if let ha = style["halign"] as? String { flags.halign = ha }
        if let va = style["valign"] as? String { flags.valign = va }
        ndLayoutFlags[ObjectIdentifier(view)] = flags

        view.setContentHuggingPriority(flags.hexpand ? NSLayoutConstraint.Priority(1) : NSLayoutConstraint.Priority(250), for: .horizontal)
        view.setContentHuggingPriority(flags.vexpand ? NSLayoutConstraint.Priority(1) : NSLayoutConstraint.Priority(250), for: .vertical)

        if let stack = view.superview as? NSStackView {
            ndBoxChildAttached(stack, view)
        }
    }
}

/// Scalar (all four sides) or per-side object ({top,left,bottom,right},
/// missing = 0) — mirrors GTK style.zig's `applyMarginSpacing`/
/// `emitSpacingCss` scalar-or-object handling for the same `padding`/`margin`
/// JSON shape.
private func parseEdgeInsets(_ value: Any) -> NSEdgeInsets? {
    if let scalar = (value as? NSNumber)?.doubleValue {
        return NSEdgeInsets(top: CGFloat(scalar), left: CGFloat(scalar), bottom: CGFloat(scalar), right: CGFloat(scalar))
    }
    if let obj = value as? [String: Any] {
        let top = (obj["top"] as? NSNumber)?.doubleValue ?? 0
        let left = (obj["left"] as? NSNumber)?.doubleValue ?? 0
        let bottom = (obj["bottom"] as? NSNumber)?.doubleValue ?? 0
        let right = (obj["right"] as? NSNumber)?.doubleValue ?? 0
        return NSEdgeInsets(top: CGFloat(top), left: CGFloat(left), bottom: CGFloat(bottom), right: CGFloat(right))
    }
    return nil
}

/// Dispatches `padding` by view type — AppKit has no single "content inset"
/// API, so each widget shape gets its own real mapping instead of the old
/// one-shot frame mutation:
///  - NSStackView -> the stack's own `edgeInsets`, then reconcile children
///    (their cross-axis "fill" constraint bakes the insets into its constant).
///  - NDButton/NDTextField -> `ndPadding` (Layout.swift; inflates
///    intrinsicContentSize instead of touching the frame directly).
///  - NSScrollView wrapping an NSTextView (TextArea) -> `textContainerInset`.
///  - any other NSScrollView (ScrollView) -> `contentInsets`.
///  - anything else -> silently ignored (no AppKit equivalent for this
///    widget shape in v1).
private func applyPadding(_ view: NSView, _ insets: NSEdgeInsets) {
    if ndUsesNativeSettingsInsets(view) {
        if let stack = view as? NSStackView {
            stack.edgeInsets = .init()
            ndBoxReconcileChildren(stack)
        }
    } else if let stack = view as? NSStackView {
        stack.edgeInsets = insets
        ndBoxReconcileChildren(stack)
    } else if let button = view as? NDButton {
        button.ndPadding = insets
    } else if let field = view as? NDTextField {
        field.ndPadding = insets
    } else if let scrollView = view as? NSScrollView, let textView = scrollView.documentView as? NSTextView {
        textView.textContainerInset = NSSize(width: (insets.left + insets.right) / 2, height: (insets.top + insets.bottom) / 2)
    } else if let scrollView = view as? NSScrollView {
        scrollView.automaticallyAdjustsContentInsets = false
        scrollView.contentInsets = insets
    }
}

private func applyTextColor(_ view: NSView, _ color: NSColor) {
    if let field = view as? NSTextField {
        field.textColor = color
    } else if let button = view as? NSButton {
        let attributed = NSMutableAttributedString(string: button.title)
        attributed.addAttribute(.foregroundColor, value: color, range: NSRange(location: 0, length: attributed.length))
        button.attributedTitle = attributed
    } else if let scrollView = view as? NSScrollView, let textView = scrollView.documentView as? NSTextView {
        textView.textColor = color
    }
}

private func applyFont(_ view: NSView, _ fontObj: [String: Any]) {
    let size = (fontObj["fontSize"] as? NSNumber)?.doubleValue ?? NSFont.systemFontSize
    let family = fontObj["fontFamily"] as? String
    var font = family.flatMap { NSFont(name: $0, size: CGFloat(size)) } ?? NSFont.systemFont(ofSize: CGFloat(size))
    if (fontObj["fontWeight"] as? String) == "bold" {
        if family != nil {
            font = NSFontManager.shared.convert(font, toHaveTrait: .boldFontMask)
        } else {
            font = NSFont.boldSystemFont(ofSize: CGFloat(size))
        }
    }
    if let field = view as? NSTextField {
        field.font = font
    } else if let button = view as? NSButton {
        button.font = font
    } else if let scrollView = view as? NSScrollView, let textView = scrollView.documentView as? NSTextView {
        textView.font = font
    }
}

/// Parses `#RRGGBB`/`#RRGGBBAA` hex strings (the schema's style color
/// shape); unrecognized values are ignored (defensive — the React renderer
/// already validates style keys, per M6b-D6's `ndApplyStyle` doc comment).
private func nsColor(fromHexOrName hex: String) -> NSColor? {
    var s = hex
    if s.hasPrefix("#") { s.removeFirst() }
    guard s.count == 6 || s.count == 8 else { return nil }
    guard let value = UInt32(s, radix: 16) else { return nil }
    let hasAlpha = s.count == 8
    let r, g, b, a: CGFloat
    if hasAlpha {
        r = CGFloat((value >> 24) & 0xFF) / 255
        g = CGFloat((value >> 16) & 0xFF) / 255
        b = CGFloat((value >> 8) & 0xFF) / 255
        a = CGFloat(value & 0xFF) / 255
    } else {
        r = CGFloat((value >> 16) & 0xFF) / 255
        g = CGFloat((value >> 8) & 0xFF) / 255
        b = CGFloat(value & 0xFF) / 255
        a = 1
    }
    return NSColor(srgbRed: r, green: g, blue: b, alpha: a)
}

// MARK: - Grid placement helpers (referenced by the generated ndAppendChild/
// ndInsertBefore/ndRemoveChild Grid arms)

/// Sparse position -> cell tracking so `ndGridRemove` can find and clear a
/// child's cell without NSGridView exposing a child->cell lookup itself.
nonisolated(unsafe) private var gridCells: [ObjectIdentifier: [ObjectIdentifier: (row: Int, col: Int)]] = [:]

func ndGridPlace(_ grid: NSGridView, _ child: NSView, row: Int, column: Int, rowSpan: Int, columnSpan: Int) {
    while grid.numberOfRows <= row + max(rowSpan, 1) - 1 { grid.addRow(with: []) }
    while grid.numberOfColumns <= column + max(columnSpan, 1) - 1 { grid.addColumn(with: []) }
    grid.cell(atColumnIndex: column, rowIndex: row).contentView = child
    if rowSpan > 1 || columnSpan > 1 {
        grid.mergeCells(inHorizontalRange: NSRange(location: column, length: max(columnSpan, 1)),
                         verticalRange: NSRange(location: row, length: max(rowSpan, 1)))
    }
    gridCells[ObjectIdentifier(grid), default: [:]][ObjectIdentifier(child)] = (row, column)
}

func ndGridRemove(_ grid: NSGridView, _ child: NSView) {
    guard let cell = gridCells[ObjectIdentifier(grid)]?[ObjectIdentifier(child)] else {
        child.removeFromSuperview()
        return
    }
    grid.cell(atColumnIndex: cell.col, rowIndex: cell.row).contentView = nil
    gridCells[ObjectIdentifier(grid)]?[ObjectIdentifier(child)] = nil
}
