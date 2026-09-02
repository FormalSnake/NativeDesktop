import AppKit
import CNd

// Must outlive main(): the core stores &gVTable and calls through it for the
// process's whole life (mirrors src/gtk/main.zig's module-level
// `the_vtable`). `gWindow`/`gCtx` are declared in main.swift.

/// Rebuilds an `NSView` from a raw vtable handle. Handles ride the ABI as
/// retained `Unmanaged<NSView>` pointers (`create` calls `passRetained`;
/// every other op here calls `takeUnretainedValue` — the core owns the
/// retain for the node's lifetime and drops it through `release_node` when
/// the id leaves the tree, same contract as `src/gtk/backend.zig`'s
/// ref_sink'd widget handles).
@inline(__always) private func viewFrom(_ p: UnsafeMutableRawPointer?) -> NSView {
    Unmanaged<NSView>.fromOpaque(p!).takeUnretainedValue()
}
@inline(__always) private func cstr(_ p: UnsafePointer<CChar>?) -> String {
    p.map { String(cString: $0) } ?? ""
}

/// The real AppKit backend. Every
/// `@convention(c)` closure decodes its raw-pointer/C-string args and calls
/// into the generated `NDGen.Widgets` dispatcher — never a narrower
/// concrete type at this layer (the generated dispatcher owns per-kind
/// casts, mirroring `src/gtk/backend.zig`'s `vt*` wrappers).
///
/// Swift 6 strict concurrency: raw pointers cross into `MainActor
/// .assumeIsolated` closures as `Int` bit patterns (capturing
/// `UnsafeMutableRawPointer` directly trips the sending-risk check even
/// though the ABI guarantees every call arrives on the UI thread already).
/// C strings are decoded to Swift `String` *before* entering the isolated
/// closure.
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
            let view = viewFrom(widgetPtr)
            guard let f = view as? NSTextField, f.stringValue != textStr else { return }
            f.stringValue = textStr
            ndInvalidateBoxChain(from: view)
        }
    }

    vt.set_visible = { _, w, visible in
        let widgetBits = Int(bitPattern: w)
        MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return }
            let view = viewFrom(widgetPtr)
            guard view.isHidden == visible else { return }
            view.isHidden = !visible
            ndInvalidateBoxChain(from: view)
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
            let view = viewFrom(widgetPtr)
            // A `<toolbarview>` pane is a LOGICAL holder: its own view never
            // enters the hierarchy (HeaderBar.swift), so `superview` is nil
            // from create to teardown, and the core gates its remove op on
            // this answer (src/tree.zig). Reporting nil meant a sidebar pane
            // could be unmounted from the tree while its NSSplitViewItem
            // stayed, holding the gutter. The pane's attachment is what it
            // reports instead, and the generated SplitView/Window remove arms
            // that take one are idempotent.
            if let pane = view as? NDToolbarPaneView {
                return pane.paneController != nil || pane.manager != nil
            }
            return view.superview != nil
        }
    }

    vt.unparent = { _, w in
        let widgetBits = Int(bitPattern: w)
        MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return }
            let view = viewFrom(widgetPtr)
            // dev-mode hot-reload/Restart sweep (Tree.gcOldGenerations et al.)
            // tears down doomed widgets via this path instead of remove_child
            // — a torn-down `<paned>` needs the same controller cleanup here.
            ndPanedTeardown(view)
            view.removeFromSuperview()
        }
    }

    vt.get_window = { _ in
        let bits: Int? = MainActor.assumeIsolated {
            // src/tree.zig's post-crash respawn path
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

    // node_visible / node_bounds / snapshot / semantic_action: the
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

    // Drops the core's create-time +1 (see viewFrom's contract) when the
    // core forgets a node id (tree remove / generation GC / clearAppNodes).
    // The view stays alive while AppKit's hierarchy still references it.
    vt.release_node = { _, w in
        let widgetBits = Int(bitPattern: w)
        MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return }
            let view = Unmanaged<NSView>.fromOpaque(widgetPtr).takeUnretainedValue()
            ndPurgeNodeRegistries(view)
            Unmanaged<NSView>.fromOpaque(widgetPtr).release()
        }
    }

    return vt
}

/// Purges every ObjectIdentifier-keyed side table entry for a view the core
/// is about to release. ObjectIdentifier is a raw address: once the view
/// deallocates, the allocator can hand the same block to the next same-size
/// widget, which would silently inherit the dead node's state (a stale
/// toolbar click adapter, another widget's badge or empty-state overlay, a
/// wrong automation kind). One call site, invoked from `vt.release_node`
/// right before the ownership release, so new registries have exactly one
/// place to hook into. Registries private to other files purge through
/// their own `nd*Purge` helpers below; registries keyed by NSWindow
/// (ndWindowToolbarStyles, WindowTabs.swift's tables) are exempt — windows
/// never ride `release_node`.
@MainActor func ndPurgeNodeRegistries(_ view: NSView) {
    let id = ObjectIdentifier(view)
    ndNodeTypography[id] = nil
    ndDisabledViews.remove(id)
    ndNavigationSidebars.remove(id)
    ndBoxedLists.remove(id)
    ndBoxedListBackings[id] = nil
    ndToolbarStrips.remove(id)
    ndToolbarBackings[id] = nil
    ndPillBadged.remove(id)
    ndActivatableState[id] = nil
    gridCells[id] = nil
    for grid in gridCells.keys { gridCells[grid]?[id] = nil }
    // HeaderBar.swift's toolbar state (a promoted button's prominence/badge/
    // item/click adapter — the adapter's weak backref is what made a
    // recycled button render normal but dead to clicks).
    ndToolbarProminent.remove(id)
    ndToolbarBadges[id] = nil
    ndToolbarPromotedItems[id] = nil
    ndToolbarItemTargets[id] = nil
    ndImageSymbolConfigs[id] = nil
    ndSidebarTables[id] = nil
    ndSidebarRowButtons.remove(id)
    ndForcedSidebars.remove(id)
    ndSidebarFallbackRows.remove(id)
    ndSplitControllers[id] = nil
    ndContentToWindow[id] = nil
    radioGroupIdentifier[id] = nil
    EventDispatcher.shared.purge(view)
    ndMenuManager?.purgeOwner(view)
    ndLayoutPurge(view)
    ndHoverPurge(view)
    ndEmptyStatePurge(view)
    ndAutomationPurge(view)
    ndTablePurge(view)
    ndTreeViewPurge(view)
    ndSourceTreePurge(view)
    ndSettingsGroupPurge(view)
    ndWindowDialogsPurge(view)
    ndListViewPurge(view)
    ndSourceListPurge(view)
    ndSourceListSurfacePurge(view)
    ndDragDropPurge(view)
    ndCodeEditorPurge(view)
    ndPaneInstallPurge(view)
    ndPanedTeardown(view)
}

/// testIDs: mirrors the tracked `testID` prop onto AppKit's own
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

/// cssClasses: decodes `props.cssClasses` (the validated Adwaita-class
/// allowlist, riding in the ordinary props JSON rather than a dedicated
/// vtable field so the C-ABI vtable stays minimal) and
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

/// Box views (`NDBoxView`) carrying a sidebar structural class, either
/// `navigation-sidebar` (the portable contract) or `nd-native-sidebar` (the
/// unconditional opt-in). On the Mac each is backed by a real source-list
/// `NSTableView` (SidebarTable.swift), whose row model is the box's own child
/// buttons, giving native accent/focus selection, row metrics, and font for
/// free instead of a stack of generic push buttons. Set-replace like
/// `ndNodeTypography`: inserted when a sidebar class is present, removed when
/// both drop. NOT private, since `ndBoxChildAttached` (Layout.swift) reads it
/// at child-attach time. Same accepted leak profile as `ndLayoutFlags`
/// (bounded by live widget count).
nonisolated(unsafe) var ndNavigationSidebars: Set<ObjectIdentifier> = []

/// Box views (`NDBoxView`) carrying the `boxed-list` structural class. They
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

/// The same, for boxes carrying the `card` structural class. `card` is
/// libadwaita's surface class and therefore the portable contract for "this
/// group of children is one raised panel" — the unchanged app tree that gets a
/// rounded surface on GTK has to get one here too. It read as a no-op on the
/// Mac before, which is why a kanban column and its cards rendered as bare
/// text on a flat pane. Distinct from `boxed-list`, which additionally turns
/// its `<separator>` children into inset row dividers. See `ndApplySurfaceCard`.
nonisolated(unsafe) private var ndSurfaceCardBackings: [ObjectIdentifier: NSBox] = [:]

/// Box views (`NDBoxView`) carrying the `toolbar` structural class. They
/// render as a native header strip on the Mac: an `NSVisualEffectView`
/// `.headerView` backing plus a 1 pt bottom hairline — see
/// `ndApplyToolbarStrip`. Set-replace like `ndBoxedLists`.
nonisolated(unsafe) private var ndToolbarStrips: Set<ObjectIdentifier> = []

/// The backing view behind each `toolbar` strip (the hairline NSBox lives
/// inside it, so removing the backing removes both). Reused on re-apply,
/// removed when the class drops. See `ndApplyToolbarStrip`.
nonisolated(unsafe) private var ndToolbarBackings: [ObjectIdentifier: NDToolbarStripBacking] = [:]

/// A `toolbar` strip's own content inset. The backing spans the pane edge to
/// edge; the controls inside it do not, or the first icon sits against the
/// window frame. An app-declared `padding` overrides it (`applyPadding`).
/// Belongs in Metrics.swift once that file settles.
private let ndToolbarStripInsets = NSEdgeInsets(top: 4, left: 12, bottom: 4, right: 12)

func ndEdgeInsetsEqual(_ a: NSEdgeInsets, _ b: NSEdgeInsets) -> Bool {
    a.top == b.top && a.left == b.left && a.bottom == b.bottom && a.right == b.right
}

/// The behavior of the split item whose pane contains `view`, or nil when it
/// is not inside one. Resolves the item the way the generated SplitView remove
/// arm does: walk up to the subview the split view itself hosts, then match
/// that against the controller's items.
func ndEnclosingSplitItemBehavior(_ view: NSView) -> NSSplitViewItem.Behavior? {
    var v: NSView? = view
    while let cur = v {
        if let split = cur.superview as? NSSplitView,
           let controller = ndSplitViewController(for: split),
           let item = controller.splitViewItems.first(where: { $0.viewController.view === cur }) {
            return item.behavior
        }
        v = cur.superview
    }
    return nil
}

/// The backing view behind a `toolbar` strip: a plain `NSView` that hosts the
/// hairline, and hosts the `.headerView` material only when the strip is NOT
/// inside a sidebar pane. macOS 26 draws the sidebar on glass, and a visual
/// effect view inside one prevents that glass from showing through (WWDC25
/// 310, "Build an AppKit app with the new design": "you should remove these
/// visual effect views"). SplitController.swift makes the pane host itself a
/// plain NSView for the same reason; this is that fix one level in.
///
/// The decision can't be made when the class lands: the strip's box is
/// normally still unparented then (src/tree.zig applies props before append),
/// so there is no split item to ask. It is made, and re-made, whenever the
/// backing reaches a window.
final class NDToolbarStripBacking: NSView {
    private var material: NSVisualEffectView?

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        syncMaterial()
    }

    private func syncMaterial() {
        let wanted = window != nil && ndEnclosingSplitItemBehavior(self) != .sidebar
        if wanted, material == nil {
            let effect = NSVisualEffectView()
            // `.headerView` is the interior header material; `.titlebar` is
            // the window's own and over-blurs an interior pane strip.
            effect.material = .headerView
            effect.blendingMode = .withinWindow
            effect.state = .followsWindowActiveState
            effect.translatesAutoresizingMaskIntoConstraints = false
            addSubview(effect, positioned: .below, relativeTo: nil)
            NSLayoutConstraint.activate([
                effect.leadingAnchor.constraint(equalTo: leadingAnchor),
                effect.trailingAnchor.constraint(equalTo: trailingAnchor),
                effect.topAnchor.constraint(equalTo: topAnchor),
                effect.bottomAnchor.constraint(equalTo: bottomAnchor),
            ])
            material = effect
        } else if !wanted, let effect = material {
            effect.removeFromSuperview()
            material = nil
        }
    }
}

/// `ndApplyCssClasses` is a real semantic mapping: it maps
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
/// Most structural classes (`card`, `view`, `osd`, ...) are
/// silently ignored — those roles come from the SplitView/HeaderBar widgets
/// themselves on the Mac, not from class strings. `nd-native-sidebar`,
/// `boxed-list` and `toolbar` are the exceptions: recorded into their
/// registries and turned into a real source-list `NSTableView`
/// (SidebarTable.swift), a native grouped `NSBox` card, and a `.headerView`
/// material strip respectively — so the sidebar, boxed-list and toolbar
/// forms the app declares render natively on both backends without any
/// per-platform app code.
func ndApplyCssClasses(_ view: NSView, _ classes: [String]) {
    // Set-replace, not additive (mirrors GTK's applyCssClasses, which removes
    // every allowlist class not in `value`): reset the button properties the
    // switch below can touch to their baseline FIRST, so a class dropped from
    // the list actually clears its effect.
    if let btn = view as? NSButton {
        // A standing `prominent` prop (ndButtonApplyProminent) owns the
        // accent bezel — the class reset must not clear it on an unrelated
        // cssClasses update.
        btn.bezelColor = ndToolbarProminent.contains(ObjectIdentifier(btn)) ? .controlAccentColor : nil
        btn.keyEquivalent = ""
        btn.hasDestructiveAction = false
        btn.isBordered = true
        btn.showsBorderOnlyWhileMouseInside = false
        btn.borderShape = .automatic
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
            // A real capsule, not "rounded enough" (the reset above restores
            // .automatic when the class drops). NSTextField pills are the
            // set-replace block after this loop.
            if let btn = view as? NSButton { btn.borderShape = .capsule }
        case "flat":
            guard let btn = view as? NSButton else { continue }
            // A sidebar row is a hidden table-row provider — `flat` is a no-op.
            if ndSidebarRowButtons.contains(ObjectIdentifier(btn)) { break }
            btn.isBordered = false
            btn.showsBorderOnlyWhileMouseInside = true
        default:
            // Typography classes are handled by ndRecomputeTypography below,
            // not this switch. `nd-native-sidebar`/`boxed-list`/`toolbar`/
            // `card` are structural and handled in the NSStackView blocks
            // after this loop. The rest (view, osd, ...) are roles owned by
            // the SplitView/HeaderBar widgets on the Mac — ignored here.
            break
        }
    }

    // Record the sidebar structural class (set-replace) and back the box with
    // a source-list NSTableView (SidebarTable.swift).
    //
    // `navigation-sidebar` is libadwaita's own class and therefore the
    // portable contract: the unchanged app tree that gets a native sidebar on
    // GTK gets one here. It is gated on the box's rows actually being
    // row-shaped, because the table covers the whole box and the composite
    // libadwaita row (button + caption + badge) would lose everything that is
    // not the button. `nd-native-sidebar` stays for exactly those boxes as an
    // unconditional takeover that skips the gate.
    //
    // The gate is evaluated on child attach, not here: the box is normally
    // still empty when its classes land (src/tree.zig applies props before
    // append), so `ndBoxChildAttached` re-runs the reconcile as rows arrive.
    if let stack = view as? NDBoxView {
        let id = ObjectIdentifier(stack)
        let forced = classes.contains("nd-native-sidebar")
        if forced || classes.contains("navigation-sidebar") {
            ndNavigationSidebars.insert(id)
            if forced { ndForcedSidebars.insert(id) } else { ndForcedSidebars.remove(id) }
            ndReconcileSidebarTable(stack)
        } else if ndNavigationSidebars.remove(id) != nil {
            ndForcedSidebars.remove(id)
            ndRemoveSidebarTable(stack)
            ndClearSidebarRowFallback(stack)
        }
    }

    // A sidebar row's `suggested-action` changed (navigation): re-align the
    // table's native selection with it. The row button may sit several
    // structural wrapper boxes below the classed box itself (a host/project
    // section), so walk up to the enclosing table rather than assuming the
    // button's immediate superview is it.
    if let btn = view as? NDButton, ndSidebarRowButtons.contains(ObjectIdentifier(btn)),
       let table = ndEnclosingSidebarTable(btn) {
        table.syncSelection()
    }

    // Same event in a sidebar the takeover declined: re-assert the source-list
    // row rendering that the set-replace baseline at the top of this function
    // just reset. Selection comes from `classes`, the incoming set, because
    // the recorded one is only written at the end of this function.
    if let btn = view as? NDButton, ndSidebarFallbackRows.contains(ObjectIdentifier(btn)) {
        ndApplySidebarRowStyle(btn, selected: classes.contains("suggested-action"))
    }

    // Record `boxed-list` (set-replace) and apply the grouped-card treatment,
    // retro-styling any `<separator>` children that already attached before
    // the class landed (either create order — see Layout.swift's header).
    if let box = view as? NDBoxView {
        let id = ObjectIdentifier(box)
        if classes.contains("boxed-list") {
            ndBoxedLists.insert(id)
            ndApplyBoxedListCard(box, enabled: true)
            for sub in box.ndChildren {
                if let sep = sub as? NSBox { ndStyleBoxedListDivider(sep, in: box) }
            }
        } else if ndBoxedLists.remove(id) != nil {
            ndApplyBoxedListCard(box, enabled: false)
            for sub in box.ndChildren {
                if let sep = sub as? NSBox { ndUnstyleBoxedListDivider(sep, in: box) }
            }
        }
    }

    // Record `toolbar` (set-replace) and apply the native header-strip
    // treatment (mirror of the boxed-list block above).
    if let box = view as? NDBoxView {
        let id = ObjectIdentifier(box)
        if classes.contains("toolbar") {
            ndToolbarStrips.insert(id)
            ndApplyToolbarStrip(box, enabled: true)
        } else if ndToolbarStrips.remove(id) != nil {
            ndApplyToolbarStrip(box, enabled: false)
        }
    }

    // A strip button's own class update runs the set-replace baseline and then
    // `flat`, both of which undo the strip metric; re-assert it here rather
    // than leaving the strip to drift on the first cssClasses update.
    if let btn = view as? NSButton, let box = btn.superview as? NDBoxView, ndIsToolbarStrip(box) {
        ndNormalizeToolbarStripButton(btn)
    }

    // `card`: the raised surface (same block shape as `boxed-list`). A box
    // carrying BOTH gets the grouped-list treatment only — that one already
    // draws a surface, and two stacked backings would double the fill.
    if let box = view as? NDBoxView {
        let id = ObjectIdentifier(box)
        let want = classes.contains("card") && !classes.contains("boxed-list")
        if want {
            ndApplySurfaceCard(box, enabled: true)
        } else if ndSurfaceCardBackings[id] != nil {
            ndApplySurfaceCard(box, enabled: false)
        }
    }

    // `pill` on a text label: the capsule count badge apps hand-roll on GTK.
    // Set-replace, but gated on a recorded prior state — an unconditional
    // disable would clobber a bezeled TextInput's own drawsBackground.
    if let field = view as? NSTextField {
        let had = ndPillBadged.contains(ObjectIdentifier(field))
        let want = classes.contains("pill")
        if want != had {
            ndApplyPillBadge(field, enabled: want)
            if want { ndPillBadged.insert(ObjectIdentifier(field)) } else { ndPillBadged.remove(ObjectIdentifier(field)) }
        }
    }

    // `activatable` on a box row: native hover feedback (set-replace; the
    // teardown lives HERE, the reset path, so a dropped class removes the
    // live tracking area rather than leaking it).
    if let box = view as? NDBoxView {
        ndApplyActivatable(box, enabled: classes.contains("activatable"))
    }

    var typography = ndNodeTypography[ObjectIdentifier(view)] ?? NDTypography()
    typography.classes = classes
    ndNodeTypography[ObjectIdentifier(view)] = typography
    ndRecomputeTypography(view)
    // Typography classes resize the text (`title-1`, `caption`, `numeric`).
    ndInvalidateBoxChain(from: view)
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
    guard let stack = box as? NDBoxView else { return }
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
        backing.cornerRadius = ndConcentricRadius(in: stack, fallback: NDRadius.card)
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

/// Raised-surface treatment for a `card` box: the same `NSBox` backing idiom
/// as `ndApplyBoxedListCard`, filled with `.controlBackgroundColor` and hair-
/// lined with `.separatorColor` so the panel reads against the window
/// background in both appearances the way a native grouped panel does. Both
/// colours are dynamic semantic `NSColor`s, so light/dark tracks without a
/// `cgColor` to re-resolve on an appearance change.
func ndApplySurfaceCard(_ box: NSView, enabled: Bool) {
    guard let stack = box as? NDBoxView else { return }
    let id = ObjectIdentifier(stack)
    guard enabled else {
        if let backing = ndSurfaceCardBackings[id] {
            backing.removeFromSuperview()
            ndSurfaceCardBackings[id] = nil
        }
        return
    }
    if ndSurfaceCardBackings[id] != nil { return }
    let backing = NSBox()
    backing.boxType = .custom
    backing.borderWidth = 1
    backing.borderColor = .separatorColor
    backing.fillColor = .controlBackgroundColor
    backing.cornerRadius = ndConcentricRadius(in: stack, fallback: NDRadius.card)
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
    ndSurfaceCardBackings[id] = backing
}

/// Native header-strip treatment for a `toolbar` box: an `NDToolbarStripBacking`
/// drawn behind the stack's controls (which supplies the header material where
/// it is allowed to, see that class), a 1 pt `.separatorColor` bottom hairline,
/// and the strip's own content inset. The hairline `NSBox` lives inside the
/// backing, so `enabled: false` removes both in one `removeFromSuperview`
/// (set-replace when the class drops). Modeled line-for-line on
/// `ndApplyBoxedListCard` above.
func ndApplyToolbarStrip(_ box: NSView, enabled: Bool) {
    guard let stack = box as? NDBoxView else { return }
    let id = ObjectIdentifier(stack)
    guard enabled else {
        if let backing = ndToolbarBackings[id] {
            backing.removeFromSuperview()
            ndToolbarBackings[id] = nil
        }
        if ndEdgeInsetsEqual(stack.ndPadding, ndToolbarStripInsets) {
            stack.ndPadding = NSEdgeInsets()
        }
        // Hand the buttons back to the shape the create arm gave them, then to
        // whatever their own classes say (mirror of `ndClearSidebarRowFallback`).
        for child in stack.ndChildren {
            guard let btn = child as? NSButton else { continue }
            btn.bezelStyle = .push
            ndApplyCssClasses(btn, ndCssClasses(of: btn))
        }
        return
    }
    // The strip inset, unless the app declared a `padding` of its own.
    // `applyPadding` owns that case, and re-asserts this default whenever the
    // padding is absent (every styled node sends an all-zero baseline).
    if ndEdgeInsetsEqual(stack.ndPadding, NSEdgeInsets()) {
        stack.ndPadding = ndToolbarStripInsets
    }
    if ndToolbarBackings[id] != nil { return }
    let backing = NDToolbarStripBacking()
    backing.translatesAutoresizingMaskIntoConstraints = false
    stack.addSubview(backing, positioned: .below, relativeTo: nil)

    let hairline = NSBox()
    hairline.boxType = .custom
    hairline.borderWidth = 0
    hairline.borderColor = .clear
    hairline.titlePosition = .noTitle
    hairline.contentViewMargins = .zero
    hairline.fillColor = .separatorColor
    hairline.translatesAutoresizingMaskIntoConstraints = false
    backing.addSubview(hairline)

    NSLayoutConstraint.activate([
        backing.leadingAnchor.constraint(equalTo: stack.leadingAnchor),
        backing.trailingAnchor.constraint(equalTo: stack.trailingAnchor),
        backing.topAnchor.constraint(equalTo: stack.topAnchor),
        backing.bottomAnchor.constraint(equalTo: stack.bottomAnchor),
        hairline.leadingAnchor.constraint(equalTo: backing.leadingAnchor),
        hairline.trailingAnchor.constraint(equalTo: backing.trailingAnchor),
        hairline.bottomAnchor.constraint(equalTo: backing.bottomAnchor),
        hairline.heightAnchor.constraint(equalToConstant: 1),
    ])
    ndToolbarBackings[id] = backing
    ndNormalizeToolbarStripButtons(stack)
}

/// Whether `view` is a box carrying the `toolbar` structural class.
func ndIsToolbarStrip(_ view: NSView) -> Bool {
    ndToolbarStrips.contains(ObjectIdentifier(view))
}

/// One control size and one baseline for every button in a `toolbar` strip.
///
/// A borderless icon-only NSButton takes its intrinsic size from the glyph,
/// and a glyph's box is per-symbol: measured at the system font size,
/// `arrow.clockwise` is 18x21, `plus` 18x17, a labelled button 45x16, and all
/// three report a 9pt intrinsic height with different alignment-rect insets.
/// That is four heights and three baselines in one strip. `.toolbar` is the
/// AppKit bezel for exactly this control, and it reports 20pt high with a 14pt
/// baseline whatever it holds; `showsBorderOnlyWhileMouseInside` keeps the
/// flat-until-hover affordance a strip wants, so `flat` still means what it
/// says. The glyphs are re-rendered at one point size and scale so an
/// icon-only button and a labelled one draw the same size symbol.
func ndNormalizeToolbarStripButtons(_ box: NDBoxView) {
    for child in box.ndChildren {
        guard let btn = child as? NSButton else { continue }
        ndNormalizeToolbarStripButton(btn)
    }
}

func ndNormalizeToolbarStripButton(_ btn: NSButton) {
    btn.controlSize = .regular
    btn.bezelStyle = .toolbar
    btn.isBordered = true
    // A row button promoted to the accent bezel keeps a standing border; every
    // other strip button reveals its bezel on hover.
    btn.showsBorderOnlyWhileMouseInside = btn.bezelColor == nil
    guard let image = btn.image else { return }
    if image.isTemplate {
        if let resized = image.withSymbolConfiguration(ndToolbarStripSymbolConfig) { btn.image = resized }
    } else if image.size.height != ndToolbarStripGlyphSide {
        // Raw image bytes take no symbol configuration (withSymbolConfiguration
        // hands a bitmap straight back unchanged), so they are squared off at
        // the box a symbol would have drawn in.
        image.size = NSSize(width: ndToolbarStripGlyphSide, height: ndToolbarStripGlyphSide)
        btn.invalidateIntrinsicContentSize()
    }
}

nonisolated(unsafe) private let ndToolbarStripSymbolConfig = NSImage.SymbolConfiguration(
    pointSize: NSFont.systemFontSize, weight: .regular, scale: .medium
).applying(.preferringHierarchical())

nonisolated(unsafe) private let ndToolbarStripGlyphSide: CGFloat =
    NSImage(systemSymbolName: "square", accessibilityDescription: nil)?
        .withSymbolConfiguration(ndToolbarStripSymbolConfig)?.size.height ?? NSFont.systemFontSize

/// Text fields currently carrying the `pill` capsule treatment (set-replace
/// bookkeeping — see the gated block in `ndApplyCssClasses`).
nonisolated(unsafe) private var ndPillBadged: Set<ObjectIdentifier> = []

/// Capsule badge treatment for a `pill`-classed text label. NOT the design
/// doc's NSBox-behind-the-field: an NSTextField draws its own text FIRST and
/// subviews after, so a backing subview would paint over the glyphs. The
/// field's own `drawsBackground` path resolves its dynamic `backgroundColor`
/// at draw time (appearance changes redraw correctly); only the SHAPE rides
/// the layer, which is appearance-independent. Typography (`numeric`/
/// `caption` classes) stays with the cascade — this is fill + shape + inset
/// only.
func ndApplyPillBadge(_ field: NSTextField, enabled: Bool) {
    if enabled {
        field.wantsLayer = true
        field.layer?.cornerRadius = 9
        field.layer?.masksToBounds = true
        field.drawsBackground = true
        field.backgroundColor = .quaternarySystemFill
        if let nd = field as? NDTextField, nd.ndPadding.left == 0, nd.ndPadding.right == 0 {
            nd.ndPadding = NSEdgeInsets(top: 1, left: 7, bottom: 1, right: 7)
        }
    } else {
        field.drawsBackground = false
        field.layer?.cornerRadius = 0
        if let nd = field as? NDTextField, nd.ndPadding.top == 1, nd.ndPadding.left == 7 {
            nd.ndPadding = NSEdgeInsets()
        }
    }
}

/// `activatable` hover feedback: the tracked box gets an NSTrackingArea
/// (same `.inVisibleRect` idiom as Hover.swift) whose owner toggles a
/// quaternary-fill NSBox behind the row's children at the concentric radius.
/// NSBox.fillColor is a dynamic color that redraws on appearance change —
/// never a CALayer cgColor. Teardown on class drop flips `enabled` off (the
/// reset path in `ndApplyCssClasses`): the installed tracking area goes
/// inert rather than being removed — storing/iterating NSTrackingArea
/// around the nonisolated registry trips the region-isolation checker, and
/// the tracker-lives-forever profile matches `ndHoverTrackers`.
final class NDActivatableHighlighter: NSObject {
    weak var overlay: NSBox?
    var enabled = true
    init(overlay: NSBox) { self.overlay = overlay }
    @objc func mouseEntered(with event: NSEvent) { if enabled { overlay?.isHidden = false } }
    @objc func mouseExited(with event: NSEvent) { overlay?.isHidden = true }
}

nonisolated(unsafe) private var ndActivatableState: [ObjectIdentifier: (highlighter: NDActivatableHighlighter, overlay: NSBox)] = [:]

func ndApplyActivatable(_ stack: NDBoxView, enabled: Bool) {
    let id = ObjectIdentifier(stack)
    if let state = ndActivatableState[id] {
        state.highlighter.enabled = enabled
        state.overlay.isHidden = true
        return
    }
    guard enabled else { return }
    let overlay = NSBox()
    overlay.boxType = .custom
    overlay.borderWidth = 0
    overlay.borderColor = .clear
    overlay.titlePosition = .noTitle
    overlay.contentViewMargins = .zero
    overlay.fillColor = .quaternarySystemFill
    overlay.cornerRadius = ndConcentricRadius(in: stack, fallback: 6)
    overlay.isHidden = true
    overlay.translatesAutoresizingMaskIntoConstraints = false
    stack.addSubview(overlay, positioned: .below, relativeTo: nil)
    NSLayoutConstraint.activate([
        overlay.leadingAnchor.constraint(equalTo: stack.leadingAnchor),
        overlay.trailingAnchor.constraint(equalTo: stack.trailingAnchor),
        overlay.topAnchor.constraint(equalTo: stack.topAnchor),
        overlay.bottomAnchor.constraint(equalTo: stack.bottomAnchor),
    ])
    let highlighter = NDActivatableHighlighter(overlay: overlay)
    ndActivatableState[id] = (highlighter, overlay)
    let area = NSTrackingArea(
        rect: .zero,
        options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
        owner: highlighter,
        userInfo: nil
    )
    stack.addTrackingArea(area)
}

/// The view a node's font/colour cascade actually writes to. TextArea and
/// ScrollView widgets are `NSScrollView` wrappers around an `NSTextView`
/// document view, and it is that inner view the cascade means; everything
/// else is its own text target.
///
/// A `<codeeditor>` is deliberately NOT resolved through. Its NSTextView owns
/// a monospaced font and per-run syntax colours, and `NSTextView.textColor`
/// writes across the WHOLE storage, so the cascade's own baseline
/// (`.systemFont` + `.labelColor`, re-asserted on every style or class
/// update) repainted every highlighted token in the body colour and swapped
/// the monospaced font for the system one.
private func ndTypographyTextTarget(_ view: NSView) -> NSView {
    if let scrollView = view as? NSScrollView,
       let textView = scrollView.documentView as? NSTextView,
       !(textView is NDCodeTextView) {
        return textView
    }
    return view
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
/// The baseline and class steps below write to `ndTypographyTextTarget`; the
/// style steps call `applyFont`/`applyTextColor`, which resolve through the
/// same helper (and also handle NSButton's attributedTitle color path).
func ndRecomputeTypography(_ view: NSView) {
    let textTarget = ndTypographyTextTarget(view)
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
        case "error":
            applyCssTextColor(textTarget, .systemRed)
        case "warning":
            applyCssTextColor(textTarget, .systemOrange)
        case "success":
            applyCssTextColor(textTarget, .systemGreen)
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
/// `NSColor`/`NSFont`/Auto Layout, not CSS, which is why `style.zig` stays
/// GTK-only). Decodes the `style` JSON object's
/// key set (background/color/font/padding/margin/border/hexpand/
/// vexpand/halign/valign). Best-effort choices, documented per key:
///  - `background` -> `layer.backgroundColor` (forces `wantsLayer = true`).
///  - `color` -> the text color of the nearest text-bearing control
///    (NSTextField/NSButton/NSTextView-in-scrollview).
///  - `font` -> `NSFont` on the same text-bearing controls (fontSize/
///    fontFamily/fontWeight).
///  - `border` -> `layer.borderWidth`/`borderColor`/`cornerRadius`.
///  - `padding` -> dispatched by view type; see `applyPadding`.
///  - `margin` -> recorded into `ndLayoutFlags` as the per-child gap the
///    PARENT box leaves around this widget, which is what GTK widget margins
///    are. Outside a box there is nothing to leave the gap in, so it is
///    dropped there.
///  - `hexpand`/`vexpand`/`halign`/`valign`/`minWidth`/`minHeight` -> recorded
///    into `ndLayoutFlags` (Layout.swift) and read by the enclosing
///    `NDBoxView` on its next layout.
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
    // nd-native-sidebar rows are hidden table-row providers, so neither needs a
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

    // Set-replace, like background/border/padding above: an absent key resets
    // to the default rather than leaving the last value standing. Expand and
    // align were additive before, so a child that expanded once expanded for
    // the rest of the session.
    var flags = ndLayoutFlags[ObjectIdentifier(view)] ?? NDLayoutFlags()
    flags.hexpand = (style["hexpand"] as? NSNumber)?.boolValue ?? false
    flags.vexpand = (style["vexpand"] as? NSNumber)?.boolValue ?? false
    flags.halign = style["halign"] as? String
    flags.valign = style["valign"] as? String
    flags.margin = style["margin"].flatMap(parseEdgeInsets) ?? NSEdgeInsets()
    flags.minWidth = CGFloat((style["minWidth"] as? NSNumber)?.doubleValue ?? 0)
    flags.minHeight = CGFloat((style["minHeight"] as? NSNumber)?.doubleValue ?? 0)
    ndLayoutFlags[ObjectIdentifier(view)] = flags

    ndApplyMinSize(view, style)
    // Unconditional: `font` and `padding` change what the widget measures, not
    // just where it sits, and a box caches its children's measurements until
    // something says otherwise. A bold label placed at the size it had in the
    // system font loses its last glyph.
    ndInvalidateBoxChain(from: view)
}

/// One >= constraint per axis per view, the AppKit peer of GTK's
/// gtk_widget_set_size_request, for the views a constraint still reaches: a
/// pane root, a scroll document, a clamp child. A box child is frame-placed,
/// so its minimum is honoured by the box's own measurement instead
/// (`NDLayoutFlags.minWidth`/`minHeight`, written alongside this in
/// `ndApplyStyle`). Set-replace like the rest of the style surface: a key
/// dropping out of the style object deactivates its constraint. Priority 999,
/// not required, so a frame-based parent's translated autoresizing constraints
/// win a conflict instead of throwing an unsatisfiable-constraint exception.
nonisolated(unsafe) private var ndMinSizeConstraints: [ObjectIdentifier: (width: NSLayoutConstraint?, height: NSLayoutConstraint?)] = [:]

private func ndApplyMinSize(_ view: NSView, _ style: [String: Any]) {
    let minW = (style["minWidth"] as? NSNumber)?.doubleValue
    let minH = (style["minHeight"] as? NSNumber)?.doubleValue
    let key = ObjectIdentifier(view)
    if minW == nil && minH == nil && ndMinSizeConstraints[key] == nil { return }
    var pair = ndMinSizeConstraints[key] ?? (nil, nil)

    if let w = minW {
        if let c = pair.width {
            c.constant = CGFloat(w)
        } else {
            let c = view.widthAnchor.constraint(greaterThanOrEqualToConstant: CGFloat(w))
            c.priority = NSLayoutConstraint.Priority(999)
            c.isActive = true
            pair.width = c
        }
    } else if let c = pair.width {
        c.isActive = false
        pair.width = nil
    }

    if let h = minH {
        if let c = pair.height {
            c.constant = CGFloat(h)
        } else {
            let c = view.heightAnchor.constraint(greaterThanOrEqualToConstant: CGFloat(h))
            c.priority = NSLayoutConstraint.Priority(999)
            c.isActive = true
            pair.height = c
        }
    } else if let c = pair.height {
        c.isActive = false
        pair.height = nil
    }

    if pair.width == nil && pair.height == nil {
        ndMinSizeConstraints.removeValue(forKey: key)
    } else {
        ndMinSizeConstraints[key] = pair
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
///  - NDBoxView -> the box's own `ndPadding`.
///  - NDButton/NDTextField -> `ndPadding` (Layout.swift; inflates
///    intrinsicContentSize instead of touching the frame directly).
///  - NSScrollView wrapping an NSTextView (TextArea) -> `textContainerInset`.
///  - any other NSScrollView (ScrollView) -> `contentInsets`. This is also
///    what Table/TreeView/SourceTree/SourceList/ListView land on: all five
///    return the scroll view, not the table inside it.
///  - anything else -> dropped, with one `ND_WARN` per widget shape.
///
/// The remainder (Image, Separator, Slider, ProgressBar, Spinner, DatePicker,
/// ColorPicker, LevelIndicator, Grid, TabView, Clamp, Banner, StatusPage, the
/// plain-NSButton controls Checkbox/Radio/Select/Switch/SegmentedControl, and
/// SearchInput) has no native inset API, and the obvious fix (wrapping the
/// view in a padding container) is not available here: the tracked handle IS
/// this view, and every generated attach arm requires it to be the direct
/// child of its parent (`stack.removeArrangedSubview(child)`, the
/// `arrangedSubviews` index lookup in the insert arm, `cell.contentView =
/// child` in ndGridPlace, `has_parent`). A wrapper spliced in here would leave
/// the container behind on removal. Style also arrives BEFORE the child is
/// appended (src/tree.zig), so at this point there is usually no parent to
/// splice into at all. Giving these shapes real padding means either an
/// NDButton-style intrinsicContentSize subclass per shape in the create arms,
/// or per-arranged-subview spacing in `ndBoxChildAttached`; both are codegen
/// changes, not changes to this dispatch.
private func applyPadding(_ view: NSView, _ insets: NSEdgeInsets) {
    if ndUsesNativeSettingsInsets(view) {
        if let box = view as? NDBoxView { box.ndPadding = .init() }
    } else if let box = view as? NDBoxView {
        // A `toolbar` strip carries its own content inset (`ndApplyToolbarStrip`).
        // An app-declared `padding` still wins; the all-zero baseline every
        // styled node sends does not, or the strip would lose its inset on the
        // first style apply after the class landed.
        let declared = !ndEdgeInsetsEqual(insets, NSEdgeInsets())
        box.ndPadding = (declared || !ndToolbarStrips.contains(ObjectIdentifier(box)))
            ? insets
            : ndToolbarStripInsets
    } else if let stack = view as? NSStackView {
        // NDNumberInputView and NDSettingsGroupView are the only remaining
        // stacks the core tracks; neither reconciles box children.
        stack.edgeInsets = insets
    } else if let button = view as? NDButton {
        button.ndPadding = insets
    } else if let field = view as? NDTextField {
        field.ndPadding = insets
    } else if let scrollView = view as? NSScrollView, let textView = scrollView.documentView as? NSTextView {
        textView.textContainerInset = NSSize(width: (insets.left + insets.right) / 2, height: (insets.top + insets.bottom) / 2)
    } else if let scrollView = view as? NSScrollView {
        // A style with no padding must not seize the insets: every styled
        // widget lands here with all-zero insets (the set-replace baseline),
        // and disabling automaticallyAdjustsContentInsets then strips the
        // safe-area content insets a pane-root scroll view needs to keep its
        // rows out of the titlebar region. Only real padding takes over.
        let zero = insets.top == 0 && insets.left == 0 && insets.bottom == 0 && insets.right == 0
        if zero && scrollView.automaticallyAdjustsContentInsets { return }
        scrollView.automaticallyAdjustsContentInsets = false
        scrollView.contentInsets = insets
    } else if !ndEdgeInsetsEqual(insets, NSEdgeInsets()) {
        // Dropped, but not silently: GTK emits `padding` as CSS for any
        // widget, so the same app tree keeps its spacing there and loses it
        // here. Reported per shape rather than per node, so a widget updating
        // its style every frame produces one line, not a stream.
        let shape = String(describing: type(of: view))
        if ndPaddingUnsupported.insert(shape).inserted {
            FileHandle.standardError.write(
                "ND_WARN style.padding has no AppKit mapping for \(shape); use the parent box's padding instead\n".data(using: .utf8)!)
        }
    }
}

/// Widget shapes already reported by `applyPadding`'s final arm. Keyed by
/// class name, not by view, so the warning is one line per shape per process
/// and needs no `ndPurgeNodeRegistries` entry.
nonisolated(unsafe) private var ndPaddingUnsupported: Set<String> = []

private func applyTextColor(_ view: NSView, _ color: NSColor) {
    if let field = view as? NSTextField {
        field.textColor = color
    } else if let button = view as? NSButton {
        let attributed = NSMutableAttributedString(string: button.title)
        attributed.addAttribute(.foregroundColor, value: color, range: NSRange(location: 0, length: attributed.length))
        button.attributedTitle = attributed
    } else if let textView = ndTypographyTextTarget(view) as? NSTextView {
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
    } else if let textView = ndTypographyTextTarget(view) as? NSTextView {
        textView.font = font
    }
}

/// Parses `#RRGGBB`/`#RRGGBBAA` hex strings (the schema's style color
/// shape); unrecognized values are ignored (defensive — the React renderer
/// already validates style keys).
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
