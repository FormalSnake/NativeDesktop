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
app.run()
