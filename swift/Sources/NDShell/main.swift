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
// so get_window/show_overlay/etc. can reach the live NSWindow.
nonisolated(unsafe) var gWindow: NSWindow? = nil
// The window's single unified NSToolbar manager (M11 Phase B), set by the
// generated Window create arm. The pane <headerbar>s register their items
// into it; it owns the tracking separator aligned to the split's divider.
nonisolated(unsafe) var ndWindowToolbarManager: NDToolbarManager? = nil

let app = NSApplication.shared
app.setActivationPolicy(.regular)

guard let ctx = nd_init() else {
    FileHandle.standardError.write("ND_RUNTIME_ERROR nd_init failed\n".data(using: .utf8)!)
    exit(1)
}
gCtx = ctx
gVTable = buildVTable()
nd_register_backend(ctx, &gVTable)

// M10: opt-in capability ACL + native plugin. Absent env = safe default
// (core UI ops granted), byte-identical to pre-M10 behavior.
if let grants = ProcessInfo.processInfo.environment["ND_ACL_GRANTS"] {
    grants.withCString { nd_set_acl(ctx, $0) }
}
if ProcessInfo.processInfo.environment["ND_PLUGINS"] == "1",
   let pluginPath = ProcessInfo.processInfo.environment["ND_PLUGIN_PATH"] {
    let rc = pluginPath.withCString { nd_load_plugin(ctx, $0) }
    if rc != 0 { FileHandle.standardError.write("ND_PLUGIN_LOAD_FAILED rc=\(rc)\n".data(using: .utf8)!) }
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
// TEMP probe: enter fullscreen after N seconds so the fullscreen layout can be
// inspected via automation getTree. Remove before finishing.
if let fsDelay = ProcessInfo.processInfo.environment["ND_TEST_FULLSCREEN"], let secs = Double(fsDelay) {
    func logFrames(_ tag: String) {
        let w = gWindow
        let ctrlView = w?.contentViewController?.view
        let split = (w?.contentViewController as? NSSplitViewController)?.splitView
        let cons = ctrlView?.constraints.map { "\($0.firstAttribute.rawValue)=\($0.constant)@\($0.priority.rawValue)" }.joined(separator: ",") ?? "nil"
        let msg = "ND_TEST_FS_FRAMES \(tag) window=\(w?.frame ?? .zero) screen=\(w?.screen?.frame ?? .zero) split=\(split?.frame ?? .zero) fs=\(w?.styleMask.contains(.fullScreen) ?? false) collBehav=\(w?.collectionBehavior.rawValue ?? 0) minSize=\(w?.contentMinSize ?? .zero) maxSize=\(w?.contentMaxSize ?? .zero) ctrlViewCons=[\(cons)]\n"
        FileHandle.standardError.write(msg.data(using: .utf8)!)
    }
    DispatchQueue.main.asyncAfter(deadline: .now() + secs - 0.5) { logFrames("WINDOWED") }
    DispatchQueue.main.asyncAfter(deadline: .now() + secs) {
        FileHandle.standardError.write("ND_TEST_FULLSCREEN toggling now isActive=\(NSApp.isActive)\n".data(using: .utf8)!)
        if let v = gWindow?.contentViewController?.view {
            for c in v.constraints {
                FileHandle.standardError.write("ND_TEST_CON attr=\(c.firstAttribute.rawValue) const=\(c.constant) prio=\(c.priority.rawValue) id=\(c.identifier ?? "nil") first=\(type(of: c.firstItem)) second=\(c.secondItem.map { String(describing: type(of: $0)) } ?? "nil")\n".data(using: .utf8)!)
            }
            if ProcessInfo.processInfo.environment["ND_TEST_RMCONS"] == "1" {
                let sizeCons = v.constraints.filter { ($0.firstAttribute == .width || $0.firstAttribute == .height) && $0.secondItem == nil }
                v.removeConstraints(sizeCons)
                FileHandle.standardError.write("ND_TEST_RMCONS removed \(sizeCons.count)\n".data(using: .utf8)!)
            }
        }
        gWindow?.toggleFullScreen(nil)
    }
    DispatchQueue.main.asyncAfter(deadline: .now() + secs + 2.0) { logFrames("FULLSCREEN") }
}
app.run()
