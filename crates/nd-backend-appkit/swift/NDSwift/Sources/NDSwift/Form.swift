import AppKit
import SwiftUI

/// The rows of one `form()`, as the Rust side last described them, and the nd views each row
/// shows. Rust writes it through the C functions below; SwiftUI observes it.
@MainActor @Observable
final class NDFormModel {
    struct Row: Decodable, Identifiable {
        let id: Int64
        let label: String
        let help: String?
    }

    struct Section: Decodable, Identifiable {
        let id: Int64
        let title: String?
        let footer: String?
        let rows: [Row]
    }

    struct Page: Decodable {
        let title: String?
        let pinned: Bool
        let sections: [Section]
    }

    var title: String?
    /// The title stays above the scrolling rows instead of scrolling with them.
    var pinned = false
    var sections: [Section] = []
    var slots: [Int64: Slot] = [:]

    struct Slot {
        let view: NSView
        /// The size nd laid the control out at; zero leaves it to the control.
        let size: CGSize
    }
}

/// A grouped `Form`: the system's settings rows, so metrics, separators, and light and dark
/// follow System Settings.
struct NDFormBody: View {
    let model: NDFormModel

    var body: some View {
        if model.pinned, let title = model.title {
            VStack(alignment: .leading, spacing: 0) {
                Text(title).font(.largeTitle.weight(.semibold)).padding([.horizontal, .top], 20)
                form(title: nil)
            }
        } else {
            form(title: model.title)
        }
    }

    /// The grouped form. A title that scrolls with the rows goes above the first section's own
    /// header, inside it, so that header keeps the style every other section header has.
    private func form(title: String?) -> some View {
        Form {
            if let title, model.sections.isEmpty {
                Section {} header: { pageTitle(title) }
            }
            ForEach(model.sections) { section in
                Section {
                    ForEach(section.rows) { row in
                        LabeledContent {
                            if let slot = model.slots[row.id] {
                                NDSlot(view: slot.view, size: slot.size).id(ObjectIdentifier(slot.view))
                            }
                        } label: {
                            Text(row.label)
                            if let help = row.help {
                                Text(help)
                            }
                        }
                    }
                } header: {
                    if let title, section.id == model.sections.first?.id {
                        VStack(alignment: .leading, spacing: 12) {
                            pageTitle(title)
                            if let header = section.title {
                                Text(header)
                            }
                        }
                    } else if let header = section.title {
                        Text(header)
                    }
                } footer: {
                    if let footer = section.footer {
                        Text(footer)
                    }
                }
            }
        }
        .formStyle(.grouped)
    }

    private func pageTitle(_ title: String) -> some View {
        Text(title).font(.largeTitle.weight(.semibold)).foregroundStyle(.primary).textCase(nil)
            .padding(.bottom, 8)
    }
}

/// An nd control inside a row. The view stays nd's; SwiftUI places it.
struct NDSlot: NSViewRepresentable {
    let view: NSView
    let size: CGSize

    func makeNSView(context: Context) -> NSView { view }

    func updateNSView(_ nsView: NSView, context: Context) {}

    /// The app's own width or height where its style set one, the control's fitting size
    /// otherwise.
    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSView, context: Context) -> CGSize? {
        let fitting = nsView.fittingSize
        return CGSize(width: max(size.width, fitting.width), height: max(size.height, fitting.height))
    }
}

final class NDFormHost: NSHostingView<NDFormBody> {
    let model: NDFormModel

    init(model: NDFormModel) {
        self.model = model
        super.init(rootView: NDFormBody(model: model))
    }

    @MainActor required init(rootView: NDFormBody) {
        self.model = rootView.model
        super.init(rootView: rootView)
    }

    @MainActor @preconcurrency required init?(coder: NSCoder) {
        fatalError("NDFormHost is built in code")
    }
}

@MainActor
private func host(_ view: UnsafeMutableRawPointer) -> NDFormHost {
    Unmanaged<NDFormHost>.fromOpaque(view).takeUnretainedValue()
}

/// Returns a +1 retained `NSHostingView`; the caller releases it.
@MainActor @_cdecl("nd_swift_form_create")
public func ndSwiftFormCreate() -> UnsafeMutableRawPointer {
    Unmanaged.passRetained(NDFormHost(model: NDFormModel())).toOpaque()
}

/// Replaces the page from JSON: `{"title", "pinned", "sections": [{"id", "title", "footer",
/// "rows": [{"id", "label", "help"}]}]}`. Malformed JSON keeps the last page.
@MainActor @_cdecl("nd_swift_form_set")
public func ndSwiftFormSet(_ view: UnsafeMutableRawPointer, _ json: UnsafePointer<CChar>) {
    let data = Data(String(cString: json).utf8)
    guard let page = try? JSONDecoder().decode(NDFormModel.Page.self, from: data) else { return }
    let model = host(view).model
    model.title = page.title
    model.pinned = page.pinned
    model.sections = page.sections
}

/// Shows `child` (an `NSView`, not retained by the caller's call) in row `row` at least
/// `width` by `height`; null clears it.
@MainActor @_cdecl("nd_swift_form_set_slot")
public func ndSwiftFormSetSlot(
    _ view: UnsafeMutableRawPointer, _ row: Int64, _ child: UnsafeMutableRawPointer?, _ width: Double,
    _ height: Double
) {
    let model = host(view).model
    if let child {
        let nsView = Unmanaged<NSView>.fromOpaque(child).takeUnretainedValue()
        model.slots[row] = NDFormModel.Slot(view: nsView, size: CGSize(width: width, height: height))
    } else {
        model.slots[row] = nil
    }
}
