import AppKit
import Foundation
import CNdPlugin

private var registry: UnsafeMutablePointer<nd_plugin_registry>?

private final class ColorView: NSView {
    var nodeID: UInt32 = 0
    var color: NSColor = .systemBlue
    override var isFlipped: Bool { true }
    override var intrinsicContentSize: NSSize { NSSize(width: 320, height: 200) }
    override func draw(_ dirtyRect: NSRect) { color.setFill(); dirtyRect.fill() }
    override func mouseDown(with event: NSEvent) {
        guard let registry, let emit = registry.pointee.emit_event else { return }
        "pressed".withCString { name in "{\"source\":\"appkit\"}".withCString { payload in emit(registry, nodeID, name, payload) } }
    }
    func apply(_ json: String) {
        if let data = json.data(using: .utf8), let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           let hex = object["color"] as? String { color = NSColor(hex: hex) ?? .systemBlue; needsDisplay = true }
    }
}

private extension NSColor {
    convenience init?(hex: String) {
        let value = UInt64(hex.trimmingCharacters(in: CharacterSet(charactersIn: "#")), radix: 16) ?? 0x3b82f6
        self.init(red: CGFloat((value >> 16) & 255) / 255, green: CGFloat((value >> 8) & 255) / 255, blue: CGFloat(value & 255) / 255, alpha: 1)
    }
}

@_cdecl("color_create") private func create(_ props: UnsafePointer<CChar>?) -> UnsafeMutableRawPointer? {
    let view = ColorView(frame: .zero); view.apply(String(cString: props!)); return Unmanaged.passRetained(view).toOpaque()
}
@_cdecl("color_apply") private func apply(_ raw: UnsafeMutableRawPointer?, _ props: UnsafePointer<CChar>?) { Unmanaged<ColorView>.fromOpaque(raw!).takeUnretainedValue().apply(String(cString: props!)) }
@_cdecl("color_connect") private func connect(_ raw: UnsafeMutableRawPointer?, _ nodeID: UInt32) { Unmanaged<ColorView>.fromOpaque(raw!).takeUnretainedValue().nodeID = nodeID }
@_cdecl("color_command") private func command(_ raw: UnsafeMutableRawPointer?, _ name: UnsafePointer<CChar>?, _ arg: UnsafePointer<CChar>?) {
    guard let raw, let name, String(cString: name) == "reset" else { return }
    let view = Unmanaged<ColorView>.fromOpaque(raw).takeUnretainedValue()
    view.color = .systemBlue
    view.needsDisplay = true
}
@_cdecl("color_destroy") private func destroy(_ raw: UnsafeMutableRawPointer?) { Unmanaged<ColorView>.fromOpaque(raw!).release() }

private var implementation = nd_view_impl(create: create, apply_props: apply, command: command, destroy: destroy, connect: connect)
@_cdecl("color_init") private func initialize(_ host: UnsafeMutablePointer<nd_plugin_registry>?) -> Int32 { registry = host; host!.pointee.register_view(host, "app.colorview", &implementation); return 0 }
@_cdecl("color_deinit") private func deinitialize() {}
private let pluginName = strdup("app-colorview")!
private let capabilities = UnsafeMutablePointer<UnsafePointer<CChar>?>.allocate(capacity: 1)
private var plugin: nd_plugin_v1 = {
    capabilities.initialize(to: nil)
    return nd_plugin_v1(abi_version: UInt32(ND_PLUGIN_ABI_VERSION), name: pluginName, capabilities: UnsafePointer(capabilities), init: initialize, deinit: deinitialize)
}()

@_cdecl("nd_plugin_entry") public func entry() -> UnsafePointer<nd_plugin_v1> {
    withUnsafePointer(to: &plugin) { $0 }
}
