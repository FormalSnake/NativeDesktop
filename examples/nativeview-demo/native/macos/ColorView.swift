import AppKit
import Foundation
import CNdPlugin

private final class ColorView: NSView, NativeDesktopView {
    private var color: NSColor = .systemBlue
    private var emit: ((String, String) -> Void)?

    init(propsJSON: String) {
        super.init(frame: .zero)
        apply(propsJSON: propsJSON)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

    override var isFlipped: Bool { true }
    override var intrinsicContentSize: NSSize { NSSize(width: 320, height: 200) }
    override func draw(_ dirtyRect: NSRect) { color.setFill(); dirtyRect.fill() }
    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        emit?("pressed", "{\"source\":\"appkit\",\"x\":\(Int(point.x)),\"y\":\(Int(point.y))}")
    }

    func apply(propsJSON: String) {
        if let data = propsJSON.data(using: .utf8), let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           let hex = object["color"] as? String { color = NSColor(hex: hex) ?? .systemBlue; needsDisplay = true }
    }
    func connect(nodeID: UInt32, emit: @escaping (String, String) -> Void) { self.emit = emit }
    func command(_ name: String, argJSON: String) {
        guard name == "reset" else { return }
        color = .systemBlue
        needsDisplay = true
    }
}

private extension NSColor {
    convenience init?(hex: String) {
        let value = UInt64(hex.trimmingCharacters(in: CharacterSet(charactersIn: "#")), radix: 16) ?? 0x3b82f6
        self.init(red: CGFloat((value >> 16) & 255) / 255, green: CGFloat((value >> 8) & 255) / 255, blue: CGFloat(value & 255) / 255, alpha: 1)
    }
}

private let plugin = NativeDesktopPlugin.descriptor(name: "app-colorview") { registry in
    guard let registry else { return -1 }
    NativeDesktopPlugin.registerView(registry, kind: "app.colorview") { ColorView(propsJSON: $0) }
    return 0
}

@_cdecl("nd_plugin_entry") public func entry() -> UnsafePointer<nd_plugin_v1> { plugin }
