import AppKit
import CNd
import Dispatch
import Foundation

// Must outlive main(): the core stores &gVTable and calls through it for the
// process's whole life (mirrors src/gtk/main.zig's module-level `the_vtable`).
//
// `nonisolated(unsafe)`: the vtable fields are `@convention(c)` function
// pointers, which carry no actor isolation, so Swift 6 strict concurrency
// treats every call site as `nonisolated`. The ABI contract (include/nd.h)
// guarantees these only ever fire on the embedder's UI thread (the core
// marshals via `marshal_async`), so touching AppKit types here is safe in
// practice even though the compiler can't see that guarantee.
nonisolated(unsafe) var gVTable = nd_backend()
nonisolated(unsafe) var gCtx: OpaquePointer? = nil
// Set by the generated Window create arm (NDGen/Widgets.swift's `ndCreate`)
// so get_window/show_overlay/etc. can reach the live NSWindow. Its observer
// records every window into `ndContentToWindow` (below), so a window can still
// be resolved from its node handle after the single `gWindow` moves on to a
// later window — the multi-window seam that needs no edit to the generated arm.
nonisolated(unsafe) var gWindow: NSWindow? = nil {
    didSet {
        guard let win = gWindow, let content = win.contentView else { return }
        ndContentToWindow[ObjectIdentifier(content)] = win
        ndInstallFileDrop(on: win)
    }
}
// Every app window keyed by its create-time content view's identity (the handle
// src/tree.zig binds each `<window>` node to). Lets `ndWindow(for:)` resolve a
// window from its node handle even after a SplitView orphans that view, which is
// what makes N `<window>` roots each present an independent OS window without a
// single-window global standing in the way. Populated by `gWindow`'s observer.
nonisolated(unsafe) var ndContentToWindow: [ObjectIdentifier: NSWindow] = [:]
// The window's single unified NSToolbar manager, set by the
// generated Window create arm. The pane <headerbar>s register their items
// into it; it owns the tracking separator aligned to the split's divider.
nonisolated(unsafe) var ndWindowToolbarManager: NDToolbarManager? = nil
// The live content view the NEXT `snapshot` renders (multi-window). The
// `snapshot` ABI op carries no window handle, so automation.zig's
// selectSnapshotWindow resolves the target window through `resolve_window`
// first, whose closure records the resolved content view here; `ndSnapshot`
// consumes it one-shot (falling back to the global window otherwise). Weak so a
// closed window's content view can't be pinned alive by a stale target.
nonisolated(unsafe) weak var ndSnapshotTargetContent: NSView? = nil

let app = NSApplication.shared
app.setActivationPolicy(.regular)

guard let ctx = nd_init() else {
    FileHandle.standardError.write("ND_RUNTIME_ERROR nd_init failed\n".data(using: .utf8)!)
    exit(1)
}
gCtx = ctx
gVTable = buildVTable()
nd_register_backend(ctx, &gVTable)
nd_set_backend_name(ctx, "appkit")

// Packaged-app launch env (nd-app.json), before the plugin block reads
// ND_PLUGINS and before nd_start_runtime snapshots the environment.
NDBundleBootstrap.apply()

// Opt-in capability ACL + native plugins. An absent env var keeps the safe
// default: core UI ops granted, everything else unchanged.
if let grants = ProcessInfo.processInfo.environment["ND_ACL_GRANTS"] {
    grants.withCString { nd_set_acl(ctx, $0) }
}
if ProcessInfo.processInfo.environment["ND_PLUGINS"] == "1" {
    nd_load_plugins_from_env(ctx)
}

if nd_start_runtime(ctx) != 0 {
    FileHandle.standardError.write("ND_RUNTIME_ERROR nd_start_runtime failed\n".data(using: .utf8)!)
    exit(1)
}
if ProcessInfo.processInfo.environment["NATIVE_AUTOMATION"] == "1" {
    if nd_start_automation(ctx) != 0 {
        FileHandle.standardError.write("ND_AUTOMATION_ERROR nd_start_automation failed\n".data(using: .utf8)!)
    }
}
// Every quit path goes through NSApplication.terminate(_:) (MenuBar's Quit
// item / Cmd-Q), which calls exit() inside run() — code after run() never
// executes. applicationWillTerminate is the one seam where the window/view
// hierarchy is still alive, so plugin deinit() and native-view destroy()
// callbacks receive live NSViews.
final class NDAppDelegate: NSObject, NSApplicationDelegate {
    func applicationWillTerminate(_ notification: Notification) {
        if let ctx = gCtx { nd_shutdown(ctx) }
    }

    // App-level activation stream (system-capability seam).
    func applicationDidBecomeActive(_ notification: Notification) {
        NDSystem.emitEvent(channel: "app.activate", dataJson: "{}")
    }

    func applicationDidResignActive(_ notification: Notification) {
        NDSystem.emitEvent(channel: "app.deactivate", dataJson: "{}")
    }

    // OS launch delivery: file:// URLs collapse into one `app.openFile`
    // {"paths":[...]}; every other scheme emits its own `app.openUrl`.
    func application(_ application: NSApplication, open urls: [URL]) {
        var filePaths: [String] = []
        for url in urls {
            if url.isFileURL {
                filePaths.append(url.path)
            } else {
                NDSystem.emitEvent(
                    channel: "app.openUrl",
                    dataJson: "{\"url\":\(NDSystem.jsonFragment(url.absoluteString))}")
            }
        }
        if !filePaths.isEmpty {
            NDSystem.emitEvent(
                channel: "app.openFile",
                dataJson: "{\"paths\":\(NDSystem.jsonFragment(filePaths))}")
        }
    }
}
// NSApplication.delegate does not retain; this top-level `let` keeps it alive.
let appDelegate = NDAppDelegate()
app.delegate = appDelegate
app.run()
