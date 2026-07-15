import AppKit

/// ToastOverlay: fully host-implemented on the Mac (AppKit has no native
/// toast). The tracked handle is a plain container
/// the single React child fills; toasts are non-activating borderless
/// NSPanels attached as CHILD WINDOWS of the node's OWN window (multi-window
/// correct: resolved via view.window, never a global), bottom-center,
/// slide/fade via NSAnimationContext. Chrome: NSGlassEffectView (macOS 26)
/// with an NSVisualEffectView `.hudWindow` fallback. Queue contract follows
/// AdwToastOverlay: one visible toast, FIFO, HIGH priority interrupts (the
/// interrupted toast is dismissed and its `toastDismissed` fires).
/// Caller-supplied ids are echoed back in `toastButtonClicked` /
/// `toastDismissed` payloads (the executeJavaScript correlation pattern);
/// `timeoutSeconds` 0 persists until dismissed (default 5, AdwToast parity).

struct NDToastRequest {
    let id: String
    let title: String
    let buttonLabel: String?
    let timeoutSeconds: Double
    let highPriority: Bool
}

final class NDToastOverlayView: NSView {
    var nodeID: UInt32 = 0
    private var queue: [NDToastRequest] = []
    private var current: (request: NDToastRequest, panel: NSPanel)?
    private var timer: Timer?
    private var resizeObservedWindow: NSWindow?

    override var isFlipped: Bool { true }

    /// Single-child slot (generated structural ToastOverlay arms).
    func setChild(_ child: NSView) {
        subviews.forEach { $0.removeFromSuperview() }
        child.translatesAutoresizingMaskIntoConstraints = false
        addSubview(child)
        NSLayoutConstraint.activate([
            child.leadingAnchor.constraint(equalTo: leadingAnchor),
            child.trailingAnchor.constraint(equalTo: trailingAnchor),
            child.topAnchor.constraint(equalTo: topAnchor),
            child.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    func clearChild(_ child: NSView) {
        if child.superview === self { child.removeFromSuperview() }
    }

    // MARK: queue

    func show(_ request: NDToastRequest) {
        if current == nil {
            present(request)
        } else if request.highPriority {
            queue.insert(request, at: 0)
            dismissCurrent() // interrupt: the high-priority toast presents next
        } else {
            queue.append(request)
        }
    }

    func dismiss(id: String) {
        if let current, current.request.id == id {
            dismissCurrent()
            return
        }
        if let idx = queue.firstIndex(where: { $0.id == id }) {
            let removed = queue.remove(at: idx)
            emitWithId("toastDismissed", removed.id) // never shown, still correlated
        }
    }

    private func presentNext() {
        guard current == nil, !queue.isEmpty else { return }
        present(queue.removeFirst())
    }

    private func present(_ request: NDToastRequest) {
        guard let host = window else {
            // Overlay not in a window yet (commit-order race): retry next turn.
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                if self.window != nil { self.present(request) }
                else { FileHandle.standardError.write("ND_WARN showToast before ToastOverlay entered a window; dropped\n".data(using: .utf8)!) }
            }
            return
        }
        let panel = buildPanel(for: request)
        current = (request, panel)

        let size = panel.contentView?.fittingSize ?? NSSize(width: 240, height: 44)
        panel.setContentSize(size)
        let frame = host.frame
        let origin = NSPoint(x: frame.midX - size.width / 2, y: frame.minY + 28)
        panel.setFrameOrigin(origin)
        panel.alphaValue = 0
        host.addChildWindow(panel, ordered: .above)
        observeResize(of: host)

        // Slide up + fade in.
        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = 0.22
            ctx.timingFunction = CAMediaTimingFunction(name: .easeOut)
            panel.animator().alphaValue = 1
            panel.animator().setFrameOrigin(NSPoint(x: origin.x, y: origin.y + 10))
        }

        timer?.invalidate()
        if request.timeoutSeconds > 0 {
            timer = Timer.scheduledTimer(withTimeInterval: request.timeoutSeconds, repeats: false) { [weak self] _ in
                self?.dismissCurrent()
            }
        }
    }

    fileprivate func dismissCurrent() {
        guard let (request, panel) = current else { return }
        current = nil
        timer?.invalidate()
        timer = nil
        NSAnimationContext.runAnimationGroup({ ctx in
            ctx.duration = 0.18
            panel.animator().alphaValue = 0
        }, completionHandler: { [weak self] in
            panel.parent?.removeChildWindow(panel)
            panel.orderOut(nil)
            guard let self else { return }
            self.emitWithId("toastDismissed", request.id)
            self.presentNext()
        })
    }

    @objc private func toastButtonPressed(_ sender: Any?) {
        guard let (request, _) = current else { return }
        emitWithId("toastButtonClicked", request.id)
        dismissCurrent()
    }

    @objc private func toastBodyClicked(_ sender: Any?) {
        dismissCurrent()
    }

    private func emitWithId(_ name: String, _ id: String) {
        ndEmitEvent(nodeID, name, "{\"data\":{\"id\":\(ndJsonString(id))}}")
    }

    // MARK: chrome

    private func buildPanel(for request: NDToastRequest) -> NSPanel {
        let stack = NSStackView()
        stack.orientation = .horizontal
        stack.spacing = 12
        stack.alignment = .centerY

        let label = NSTextField(labelWithString: request.title)
        label.lineBreakMode = .byTruncatingTail
        stack.addArrangedSubview(label)
        if let buttonLabel = request.buttonLabel, !buttonLabel.isEmpty {
            let button = NSButton(title: buttonLabel, target: self, action: #selector(toastButtonPressed(_:)))
            button.bezelStyle = .rounded
            button.controlSize = .small
            stack.addArrangedSubview(button)
        }

        // Explicit constraint padding (NSStackView edgeInsets don't reach a
        // panel's fittingSize reliably); `content.fittingSize` is what sizes
        // the panel below.
        let content = NSView()
        stack.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 16),
            stack.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: request.buttonLabel == nil ? -16 : -10),
            stack.topAnchor.constraint(equalTo: content.topAnchor, constant: 10),
            stack.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -10),
        ])
        // Click anywhere on the toast surface (outside the button) dismisses.
        content.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(toastBodyClicked(_:))))

        let chrome: NSView
        if #available(macOS 26.0, *) {
            let glass = NSGlassEffectView()
            glass.cornerRadius = 14
            glass.contentView = content
            chrome = glass
        } else {
            let effect = NSVisualEffectView()
            effect.material = .hudWindow
            effect.blendingMode = .behindWindow
            effect.state = .active
            effect.wantsLayer = true
            effect.layer?.cornerRadius = 14
            effect.layer?.masksToBounds = true
            content.translatesAutoresizingMaskIntoConstraints = false
            effect.addSubview(content)
            NSLayoutConstraint.activate([
                content.leadingAnchor.constraint(equalTo: effect.leadingAnchor),
                content.trailingAnchor.constraint(equalTo: effect.trailingAnchor),
                content.topAnchor.constraint(equalTo: effect.topAnchor),
                content.bottomAnchor.constraint(equalTo: effect.bottomAnchor),
            ])
            chrome = effect
        }

        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 240, height: 44),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered, defer: false)
        panel.isFloatingPanel = true
        panel.becomesKeyOnlyIfNeeded = true
        panel.isReleasedWhenClosed = false
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = true
        panel.contentView = chrome
        return panel
    }

    // MARK: keep bottom-center through host resizes (moves ride child-window
    // attachment automatically; resizes don't).

    private func observeResize(of host: NSWindow) {
        guard resizeObservedWindow !== host else { return }
        if let old = resizeObservedWindow {
            NotificationCenter.default.removeObserver(self, name: NSWindow.didResizeNotification, object: old)
        }
        resizeObservedWindow = host
        NotificationCenter.default.addObserver(
            self, selector: #selector(hostResized(_:)),
            name: NSWindow.didResizeNotification, object: host)
    }

    @objc private func hostResized(_ note: Notification) {
        guard let (_, panel) = current, let host = resizeObservedWindow else { return }
        let frame = host.frame
        let size = panel.frame.size
        panel.setFrameOrigin(NSPoint(x: frame.midX - size.width / 2, y: frame.minY + 38))
    }

    deinit {
        if let old = resizeObservedWindow {
            NotificationCenter.default.removeObserver(self, name: NSWindow.didResizeNotification, object: old)
        }
    }
}

/// `ndCreate`'s ToastOverlay arm (generated) calls this.
func makeToastOverlay(_ props: [String: Any]) -> NSView {
    _ = props
    return NDToastOverlayView()
}

/// Generated ndConnectEvents ToastOverlay arm (one call wires both events).
func ndToastOverlayConnect(_ view: NSView, nodeID: UInt32) {
    (view as? NDToastOverlayView)?.nodeID = nodeID
}

/// Generated ndWidgetCommand ToastOverlay arm.
/// showToast arg: { id, title, buttonLabel?, timeoutSeconds?, priority? };
/// dismissToast arg: { id } (bare string accepted too, GTK parity).
func ndToastOverlayCommand(_ view: NSView, _ command: String, _ argJson: String) {
    guard let overlay = view as? NDToastOverlayView else { return }
    switch command {
    case "showToast":
        let arg = parseProps(argJson)
        guard let id = propStr(arg, "id") else {
            FileHandle.standardError.write("ND_WARN ToastOverlay showToast: missing id\n".data(using: .utf8)!)
            return
        }
        overlay.show(NDToastRequest(
            id: id,
            title: propStr(arg, "title") ?? "",
            buttonLabel: propStr(arg, "buttonLabel"),
            timeoutSeconds: propDouble(arg, "timeoutSeconds") ?? 5,
            highPriority: (propStr(arg, "priority") ?? "normal") == "high"
        ))
    case "dismissToast":
        let arg = parseProps(argJson)
        if let id = propStr(arg, "id") {
            overlay.dismiss(id: id)
        } else if let data = argJson.data(using: .utf8),
                  let bare = (try? JSONSerialization.jsonObject(with: data, options: [.allowFragments])) as? String {
            overlay.dismiss(id: bare)
        } else {
            FileHandle.standardError.write("ND_WARN ToastOverlay dismissToast: malformed arg (expected {id})\n".data(using: .utf8)!)
        }
    default:
        FileHandle.standardError.write("ND_WARN unknown ToastOverlay command \(command)\n".data(using: .utf8)!)
    }
}

/// Generated structural ToastOverlay arms.
func ndToastOverlaySetChild(_ parent: NSView, _ child: NSView) {
    (parent as? NDToastOverlayView)?.setChild(child)
}

func ndToastOverlayClearChild(_ parent: NSView, _ child: NSView) {
    (parent as? NDToastOverlayView)?.clearChild(child)
}
