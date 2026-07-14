import AppKit
import SwiftUI
import CNdPlugin

/// Protocol implemented by app-owned AppKit views hosted by NativeDesktop.
public protocol NativeDesktopView: AnyObject {
    func apply(propsJSON: String)
    func connect(nodeID: UInt32, emit: @escaping (_ name: String, _ payloadJSON: String) -> Void)
    func command(_ name: String, argJSON: String)
    func destroy()
}

public extension NativeDesktopView {
    func connect(nodeID: UInt32, emit: @escaping (String, String) -> Void) {}
    func command(_ name: String, argJSON: String) {}
    func destroy() {}
}

/// C-ABI glue for plugins written against NativeDesktopView. nd_view_impl
/// callbacks carry no context pointer, so registerView supports one view kind
/// per plugin; hand-roll an nd_view_impl to register more.
public enum NativeDesktopPlugin {
    private static var registry: UnsafeMutablePointer<nd_plugin_registry>?
    private static var factory: ((String) -> NSView & NativeDesktopView)?

    /// Allocates an nd_plugin_v1 with the stable lifetime the ABI requires;
    /// return the result from @_cdecl("nd_plugin_entry").
    public static func descriptor(
        name: String,
        capabilities: [String] = [],
        initialize: @escaping @convention(c) (UnsafeMutablePointer<nd_plugin_registry>?) -> Int32,
        deinitialize: @escaping @convention(c) () -> Void = {}
    ) -> UnsafePointer<nd_plugin_v1> {
        let caps = UnsafeMutablePointer<UnsafePointer<CChar>?>.allocate(capacity: capabilities.count + 1)
        for (index, capability) in capabilities.enumerated() { caps[index] = UnsafePointer(strdup(capability)) }
        caps[capabilities.count] = nil
        let plugin = UnsafeMutablePointer<nd_plugin_v1>.allocate(capacity: 1)
        plugin.initialize(to: nd_plugin_v1(abi_version: UInt32(ND_PLUGIN_ABI_VERSION), name: UnsafePointer(strdup(name)), capabilities: UnsafePointer(caps), init: initialize, deinit: deinitialize))
        return UnsafePointer(plugin)
    }

    /// Call from the descriptor's initialize callback.
    public static func registerView(
        _ registry: UnsafeMutablePointer<nd_plugin_registry>,
        kind: String,
        factory: @escaping (String) -> NSView & NativeDesktopView
    ) {
        self.registry = registry
        self.factory = factory
        var impl = nd_view_impl(
            create: { props in
                guard let view = NativeDesktopPlugin.factory?(NativeDesktopPlugin.string(props)) else { return nil }
                return Unmanaged.passRetained(view as NSView).toOpaque()
            },
            apply_props: { raw, props in
                NativeDesktopPlugin.view(raw)?.apply(propsJSON: NativeDesktopPlugin.string(props))
            },
            command: { raw, name, arg in
                guard let name else { return }
                NativeDesktopPlugin.view(raw)?.command(String(cString: name), argJSON: NativeDesktopPlugin.string(arg))
            },
            destroy: { raw in
                guard let raw else { return }
                NativeDesktopPlugin.view(raw)?.destroy()
                Unmanaged<NSView>.fromOpaque(raw).release()
            },
            connect: { raw, nodeID in
                NativeDesktopPlugin.view(raw)?.connect(nodeID: nodeID) { name, payload in
                    guard let registry = NativeDesktopPlugin.registry, let emit = registry.pointee.emit_event else { return }
                    name.withCString { n in payload.withCString { p in emit(registry, nodeID, n, p) } }
                }
            }
        )
        registry.pointee.register_view(registry, kind, &impl)
    }

    private static func view(_ raw: UnsafeMutableRawPointer?) -> (NSView & NativeDesktopView)? {
        raw.flatMap { Unmanaged<NSView>.fromOpaque($0).takeUnretainedValue() as? NSView & NativeDesktopView }
    }

    private static func string(_ pointer: UnsafePointer<CChar>?) -> String {
        pointer.map { String(cString: $0) } ?? "{}"
    }
}

/// AppKit host for a SwiftUI component. Updating `rootView` preserves the
/// ordinary NSView identity expected by NativeDesktop's retained tree.
public final class NativeDesktopSwiftUIView<Content: View>: NSHostingView<Content>, NativeDesktopView {
    private let updateRoot: (String) -> Content

    public init(initialPropsJSON: String, content: @escaping (String) -> Content) {
        self.updateRoot = content
        super.init(rootView: content(initialPropsJSON))
    }

    @available(*, unavailable)
    required init(rootView: Content) { fatalError("use init(initialPropsJSON:content:)") }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

    public func apply(propsJSON: String) { rootView = updateRoot(propsJSON) }
}
