import AppKit
import CNd

/// Host-rendered crash overlay (peer of `src/gtk/overlay.zig`, wired
/// through `vtable.show_overlay`). Empty `message` clears; non-empty
/// shows a semi-opaque panel over the window's `contentView` with the crash
/// text and, in dev mode only, a Restart button whose action emits the
/// reserved `nd_emit_event(gCtx, 0, "restart", "{}")` sentinel that
/// `abi.zig` routes to `Runtime.restart` (src/gtk/backend.zig:246's
/// `onRestartIdle` contract, mirrored here).
private final class OverlayPanel: NSView {
    override var isFlipped: Bool { true }
}

// `ndShowOverlay` only ever runs on the embedder's UI thread (the core
// marshals via vtable.marshal_async before calling show_overlay), so this
// module-scope state is safe in practice — same `nonisolated(unsafe)`
// reasoning as main.swift's `gVTable`/`gCtx`/`gWindow`.
//
// One panel per window (multi-window): a JS crash is the single Bun process
// dying, so every window loses its live UI at once — the overlay paints on ALL
// open windows, and clearing removes every panel.
nonisolated(unsafe) private var overlayPanels: [OverlayPanel] = []

nonisolated(unsafe) private let isDevMode: Bool = {
    ProcessInfo.processInfo.environment["ND_DEV"] == "1"
}()

/// Builds a crash panel sized to `content` and adds it as `content`'s subview.
/// Nonisolated like `ndShowOverlay` itself (the core calls show_overlay inside
/// `MainActor.assumeIsolated`, so it is on the UI thread at runtime).
private func addOverlayPanel(to content: NSView, message: String) {
    let panel = OverlayPanel(frame: content.bounds)
    panel.autoresizingMask = [.width, .height]
    panel.wantsLayer = true
    panel.layer?.backgroundColor = NSColor.black.withAlphaComponent(0.75).cgColor
    panel.setAccessibilityIdentifier("nd-overlay-panel")
    registerOverlayNode(panel, "Box", "nd-overlay-panel", nil)

    let title = NSTextField(labelWithString: "Runtime crashed")
    title.font = NSFont.boldSystemFont(ofSize: 16)
    title.textColor = .white
    title.alignment = .center
    title.frame = NSRect(x: 20, y: 40, width: panel.bounds.width - 40, height: 24)
    title.autoresizingMask = [.width]
    title.setAccessibilityIdentifier("nd-overlay-title")
    panel.addSubview(title)
    registerOverlayNode(title, "Label", "nd-overlay-title", "Runtime crashed")

    let errorLabel = NSTextField(wrappingLabelWithString: message)
    errorLabel.textColor = .white
    errorLabel.alignment = .center
    errorLabel.frame = NSRect(x: 20, y: 80, width: panel.bounds.width - 40, height: panel.bounds.height - 140)
    errorLabel.autoresizingMask = [.width, .height]
    errorLabel.setAccessibilityIdentifier("nd-overlay-error")
    panel.addSubview(errorLabel)
    registerOverlayNode(errorLabel, "Label", "nd-overlay-error", message)

    if isDevMode {
        let restart = NSButton(title: "Restart", target: RestartTrampoline.shared, action: #selector(RestartTrampoline.fire))
        restart.bezelStyle = .rounded
        restart.frame = NSRect(x: panel.bounds.width / 2 - 40, y: panel.bounds.height - 44, width: 80, height: 24)
        restart.autoresizingMask = [.minXMargin, .maxXMargin]
        restart.setAccessibilityIdentifier("nd-overlay-restart")
        panel.addSubview(restart)
        registerOverlayNode(restart, "Button", "nd-overlay-restart", "Restart")
    }

    content.addSubview(panel)
    overlayPanels.append(panel)
}

/// Puts one overlay widget into the core's retained tree under the reserved
/// overlay generation, the AppKit peer of src/gtk/overlay.zig's
/// `registerOverlayNode`. An accessibility identifier alone is invisible to
/// `getTree`, which reads the core's tree and not the AppKit a11y hierarchy,
/// so without this an agent goes blind exactly when the app has crashed.
/// The core stores the pointer without retaining it; `overlayPanels` keeps the
/// panel (and with it its subviews) alive until `nd_overlay_clear_nodes` has
/// dropped the ids.
private func registerOverlayNode(_ view: NSView, _ widgetType: String, _ testID: String, _ text: String?) {
    guard let ctx = gCtx else { return }
    let handle = Unmanaged.passUnretained(view).toOpaque()
    widgetType.withCString { type in
        testID.withCString { id in
            if let text {
                text.withCString { value in
                    nd_overlay_register_node(ctx, handle, type, id, value)
                }
            } else {
                nd_overlay_register_node(ctx, handle, type, id, nil)
            }
        }
    }
}

func ndShowOverlay(_ message: String) {
    // Replace any existing panels rather than stacking (empty message = clear).
    // The tree ids go first: the core holds unretained pointers to these views,
    // so they must stop being reachable before the panels are released.
    if let ctx = gCtx { nd_overlay_clear_nodes(ctx) }
    for panel in overlayPanels { panel.removeFromSuperview() }
    overlayPanels.removeAll()
    if message.isEmpty { return }

    // Paint on every app window's LIVE content (see ndLiveContentView for the
    // resolution rationale). `ndContentToWindow`'s values are the app's
    // <window> nodes; dedup by identity and skip any that have gone away.
    var seen = Set<ObjectIdentifier>()
    for win in ndContentToWindow.values {
        guard seen.insert(ObjectIdentifier(win)).inserted else { continue }
        guard let content = ndLiveContentView(ofWindow: win) else { continue }
        addOverlayPanel(to: content, message: message)
    }
    // Fallback for the pre-registry / single-window path: if nothing resolved
    // through the registry, paint on the current global content.
    if overlayPanels.isEmpty, let content = ndLiveContentView() {
        addOverlayPanel(to: content, message: message)
    }
    // Same marker + format as the GTK embedder (src/gtk/overlay.zig) — the
    // kill9-equivalent Mac leg greps for it.
    FileHandle.standardError.write("ND_OVERLAY_SHOWN dev=\(isDevMode)\n".data(using: .utf8)!)
}

/// Restart's action fires from inside the button's own click-event dispatch
/// (the same call stack GTK's `onRestartClicked` defers off of via
/// `glib.idleAdd`); `DispatchQueue.main.async` is the AppKit peer of that
/// deferral, avoiding re-entrancy into whatever teardown/rebuild the restart
/// triggers.
private final class RestartTrampoline: NSObject {
    nonisolated(unsafe) static let shared = RestartTrampoline()
    @objc func fire() {
        DispatchQueue.main.async {
            "{}".withCString { payload in
                "restart".withCString { name in
                    nd_emit_event(gCtx, 0, name, payload)
                }
            }
        }
    }
}
