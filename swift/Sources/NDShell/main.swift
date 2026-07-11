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
// T1 stub: the window opened by the stub `create` on first Window widget, so
// the handshake + first commit can present *something*. T3 replaces this
// with the real backend's window tracking.
nonisolated(unsafe) var gWindow: NSWindow? = nil

func buildStubVTable() -> nd_backend {
    var vt = nd_backend()
    // T3 fills the rest from NDGen.Widgets; T1 stubs create to a bare NSView
    // so the handshake + first commit can present *something*.
    vt.create = { _, kind, _ in
        // Decode the C string *before* crossing into the isolated closure:
        // `UnsafePointer<CChar>` capture into a @MainActor closure trips
        // strict-concurrency's sending-risk check even though the pointer
        // never actually escapes to another thread here.
        // The NDP create op carries the schema widget name, capitalized:
        // {"op":"create","id":7,"widget":"Window",...}
        let isWindow = kind.map { String(cString: $0) == "Window" } ?? false
        // The ABI contract guarantees `create` arrives on the embedder's UI
        // thread (the core marshals); `@convention(c)` closures carry no
        // actor isolation for the compiler to see that, so assert it here.
        // The isolated closure returns `Int` (not the raw pointer) because
        // `UnsafeMutableRawPointer`'s `Sendable` conformance is unavailable.
        let bits: Int = MainActor.assumeIsolated {
            let v = NSView(frame: NSRect(x: 0, y: 0, width: 100, height: 24))
            if isWindow {
                let window = NSWindow(
                    contentRect: NSRect(x: 0, y: 0, width: 480, height: 320),
                    styleMask: [.titled, .closable, .miniaturizable, .resizable],
                    backing: .buffered,
                    defer: false
                )
                window.title = "NativeDesktop"
                window.contentView = v
                window.center()
                window.makeKeyAndOrderFront(nil)
                gWindow = window
            }
            return Int(bitPattern: Unmanaged.passRetained(v).toOpaque())
        }
        return UnsafeMutableRawPointer(bitPattern: bits)
    }
    vt.marshal_async = { _, fn, data in
        // dispatch_async_f: C fn ptr onto the main queue. NEVER dispatch_sync.
        // `data` is sent across as `Int` bit pattern: `UnsafeMutableRawPointer`
        // capture into the async closure trips strict-concurrency's
        // sending-risk check even though it never touches another thread.
        guard let fn else { return }
        let bits = Int(bitPattern: data)
        DispatchQueue.main.async { fn(UnsafeMutableRawPointer(bitPattern: bits)) }
    }
    vt.get_window = { _ in
        let bits: Int? = MainActor.assumeIsolated {
            guard let gWindow else { return nil }
            return Int(bitPattern: Unmanaged.passUnretained(gWindow).toOpaque())
        }
        guard let bits else { return nil }
        return UnsafeMutableRawPointer(bitPattern: bits)
    }
    // Every field the core's `Tree.apply` calls unconditionally on a commit
    // (src/abi_backend.zig: apply_props/append_child/set_text/apply_style/
    // connect_events/has_parent/unparent, even for the stub's bare NSViews)
    // must be non-null — a null fn-ptr call segfaults. T1 wires just enough
    // to not crash and to actually parent widgets so something presents;
    // T3 replaces the struct wholesale with the real AppKit backend.
    vt.apply_props = { _, _, _, _ in }
    vt.append_child = { _, parent, _, child, _ in
        // Raw pointer params are converted to `Int` bit patterns *before*
        // crossing into the @MainActor closure — capturing the raw pointers
        // themselves trips strict-concurrency's sending-risk check (same
        // issue as `create`/`marshal_async` above). `Int(bitPattern:)` on an
        // `UnsafeMutableRawPointer?` is non-failable (nil -> 0), so rebuild
        // the pointer with `UnsafeMutableRawPointer(bitPattern:)` instead of
        // `guard let` on the Int itself.
        let parentBits = Int(bitPattern: parent)
        let childBits = Int(bitPattern: child)
        MainActor.assumeIsolated {
            guard let parentPtr = UnsafeMutableRawPointer(bitPattern: parentBits),
                  let childPtr = UnsafeMutableRawPointer(bitPattern: childBits) else { return }
            let parentView = Unmanaged<NSView>.fromOpaque(parentPtr).takeUnretainedValue()
            let childView = Unmanaged<NSView>.fromOpaque(childPtr).takeUnretainedValue()
            parentView.addSubview(childView)
        }
    }
    vt.insert_before = { _, parent, _, child, _, _ in
        let parentBits = Int(bitPattern: parent)
        let childBits = Int(bitPattern: child)
        MainActor.assumeIsolated {
            guard let parentPtr = UnsafeMutableRawPointer(bitPattern: parentBits),
                  let childPtr = UnsafeMutableRawPointer(bitPattern: childBits) else { return }
            let parentView = Unmanaged<NSView>.fromOpaque(parentPtr).takeUnretainedValue()
            let childView = Unmanaged<NSView>.fromOpaque(childPtr).takeUnretainedValue()
            parentView.addSubview(childView)
        }
    }
    vt.remove_child = { _, _, _, child in
        let childBits = Int(bitPattern: child)
        MainActor.assumeIsolated {
            guard let childPtr = UnsafeMutableRawPointer(bitPattern: childBits) else { return }
            Unmanaged<NSView>.fromOpaque(childPtr).takeUnretainedValue().removeFromSuperview()
        }
    }
    vt.set_text = { _, _, _ in }
    vt.set_visible = { _, widget, visible in
        let widgetBits = Int(bitPattern: widget)
        MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return }
            Unmanaged<NSView>.fromOpaque(widgetPtr).takeUnretainedValue().isHidden = !visible
        }
    }
    vt.apply_style = { _, _, _, _ in }
    vt.connect_events = { _, _, _, _ in }
    vt.has_parent = { _, widget in
        let widgetBits = Int(bitPattern: widget)
        return MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return false }
            return Unmanaged<NSView>.fromOpaque(widgetPtr).takeUnretainedValue().superview != nil
        }
    }
    vt.unparent = { _, widget in
        let widgetBits = Int(bitPattern: widget)
        MainActor.assumeIsolated {
            guard let widgetPtr = UnsafeMutableRawPointer(bitPattern: widgetBits) else { return }
            Unmanaged<NSView>.fromOpaque(widgetPtr).takeUnretainedValue().removeFromSuperview()
        }
    }
    vt.show_overlay = { _, _ in }
    vt.node_visible = { _, _ in false }
    vt.node_bounds = { _, _, _ in false }
    vt.snapshot = { _, _ in false }
    vt.semantic_action = { _, _, _, _, _, _, _ in -32601 } // JSON-RPC "method not found"
    return vt
}

let app = NSApplication.shared
app.setActivationPolicy(.regular)

guard let ctx = nd_init() else {
    FileHandle.standardError.write("ND_RUNTIME_ERROR nd_init failed\n".data(using: .utf8)!)
    exit(1)
}
gCtx = ctx
gVTable = buildStubVTable()
nd_register_backend(ctx, &gVTable)

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
