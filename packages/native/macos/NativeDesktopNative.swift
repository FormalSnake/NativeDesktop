import AppKit
import SwiftUI

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
