import AppKit
import AVKit

/// Video (M15): AVKit's AVPlayerView — the system player, per HIG. `src`
/// takes a plain path or a URL (the same "://" sniff as GTK's ndVideoSetSrc);
/// `controls` maps to controlsStyle (.inline / .none); `loop` replays on the
/// item's end notification (AVPlayerLooper needs an AVQueuePlayer — a plain
/// seek-and-play keeps the single-player shape).
final class NDVideoView: AVPlayerView {
    var looping = false
    private var observedItem: AVPlayerItem?

    func setSrc(_ src: String) {
        if let observedItem {
            NotificationCenter.default.removeObserver(self, name: AVPlayerItem.didPlayToEndTimeNotification, object: observedItem)
            self.observedItem = nil
        }
        guard !src.isEmpty else {
            player = nil
            return
        }
        let url: URL? = src.contains("://") ? URL(string: src) : URL(fileURLWithPath: src)
        guard let url else {
            FileHandle.standardError.write("ND_WARN Video: unparsable src \(src)\n".data(using: .utf8)!)
            return
        }
        let item = AVPlayerItem(url: url)
        player = AVPlayer(playerItem: item)
        observedItem = item
        NotificationCenter.default.addObserver(
            self, selector: #selector(didPlayToEnd(_:)),
            name: AVPlayerItem.didPlayToEndTimeNotification, object: item)
    }

    @objc private func didPlayToEnd(_ note: Notification) {
        guard looping else { return }
        player?.seek(to: .zero)
        player?.play()
    }

    deinit {
        if let observedItem {
            NotificationCenter.default.removeObserver(self, name: AVPlayerItem.didPlayToEndTimeNotification, object: observedItem)
        }
    }
}

/// `ndCreate`'s Video arm (generated) calls this.
func makeVideoView(_ props: [String: Any]) -> NSView {
    let v = NDVideoView()
    v.controlsStyle = (propBool(props, "controls") ?? true) ? .inline : .none
    v.looping = propBool(props, "loop") ?? false
    if let src = propStr(props, "src"), !src.isEmpty {
        v.setSrc(src)
        if propBool(props, "autoplay") ?? false { v.player?.play() }
    }
    return v
}

/// Generated ndApplyProps Video.src arm.
func ndVideoSetSrc(_ view: NSView, _ src: String) {
    (view as? NDVideoView)?.setSrc(src)
}

/// Generated ndApplyProps Video.loop arm.
func ndVideoSetLoop(_ view: NSView, _ loop: Bool) {
    (view as? NDVideoView)?.looping = loop
}
