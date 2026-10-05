#if canImport(CCef)
import AppKit
import CCef
import QuartzCore

// A page under a sliding sidebar (SplitMotion.swift). Chromium resizing on
// every frame of the slide is the expensive way to move it: the anchor window
// is resized each frame and the renderer lays out and rasters each size, and
// its frames trail the layout they were drawn for. So the page is covered by
// a still of itself, taken as the slide is asked for, which is stretched over
// whatever rectangle the slide lays the view out at, and the web contents
// underneath keep their size. When the slide lands they take the new size
// once, and the still goes as soon as the page has painted at it.

/// How long a page that never reports a paint keeps its still.
private let ndMotionPaintPatience: TimeInterval = 0.4
/// From the renderer's frame at the new size to that frame on screen: the
/// browser's compositor takes the new frame a few display frames later.
private let ndMotionPresentLag: TimeInterval = 0.1

extension NDCefWebView: NDSlidingPage {
    /// Freezes the page and asks for its still; `ready` runs once the still is
    /// up, or at once when there is none to wait for. False when the page
    /// cannot be frozen and should follow the layout.
    func motionFreeze(ready: @escaping @MainActor () -> Void) -> Bool {
        if motionFrozen {
            ready()
            return true
        }
        // One web contents and nothing beside it: a docked inspector keeps
        // the live path.
        guard subviews.count == 1, devTools.isReady, let layer else { return false }
        motionFrozen = true
        motionGeneration += 1
        motionEndedAt = 0
        let generation = motionGeneration
        devTools.call("Page.captureScreenshot", ["format": "jpeg", "quality": 90, "optimizeForSpeed": true]) { [weak self] result, _ in
            MainActor.assumeIsolated {
                defer { ready() }
                guard let self, self.motionGeneration == generation, self.motionFrozen, self.motionCover == nil,
                      let encoded = result?["data"] as? String, let data = Data(base64Encoded: encoded),
                      let image = NSImage(data: data)?.cgImage(forProposedRect: nil, context: nil, hints: nil) else { return }
                let cover = CALayer()
                cover.contents = image
                cover.contentsGravity = .resize
                cover.zPosition = 1000
                cover.actions = ["bounds": NSNull(), "position": NSNull(), "frame": NSNull(), "contents": NSNull()]
                CATransaction.begin()
                CATransaction.setDisableActions(true)
                cover.frame = layer.bounds
                layer.addSublayer(cover)
                CATransaction.commit()
                self.motionCover = cover
            }
        }
        return true
    }

    /// Stretches the still over this view's current bounds, in whatever
    /// transaction laid the view out.
    func motionStretch() {
        motionCover?.frame = layer?.bounds ?? bounds
    }

    /// The slide is over and this view has its landing frame: the web
    /// contents take it under the still.
    func motionThaw() {
        guard motionFrozen else { return }
        motionFrozen = false
        motionEndedAt = CACurrentMediaTime()
        let generation = motionGeneration
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        motionStretch()
        for child in subviews where child.frame != bounds { child.frame = bounds }
        hostGeometryNeedsSync()
        CATransaction.commit()
        guard motionCover != nil else {
            motionTraceLanding(timedOut: false)
            return
        }
        let width = Int((bounds.width * (window?.backingScaleFactor ?? 2)).rounded())
        let probe = "new Promise(r=>{const w=\(width),t=performance.now(),f=()=>{if(Math.abs(innerWidth*devicePixelRatio-w)<3||performance.now()-t>\(Int(ndMotionPaintPatience * 1000)))requestAnimationFrame(()=>r(1));else requestAnimationFrame(f)};requestAnimationFrame(f)})"
        devTools.call("Runtime.evaluate", ["expression": probe, "awaitPromise": true, "returnByValue": true]) { [weak self] _, _ in
            DispatchQueue.main.asyncAfter(deadline: .now() + ndMotionPresentLag) {
                MainActor.assumeIsolated {
                    guard let self, self.motionGeneration == generation else { return }
                    self.motionUncover(timedOut: false)
                }
            }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + ndMotionPaintPatience + ndMotionPresentLag) { [weak self] in
            MainActor.assumeIsolated {
                guard let self, self.motionGeneration == generation else { return }
                self.motionUncover(timedOut: true)
            }
        }
    }

    private func motionUncover(timedOut: Bool) {
        guard !motionFrozen, let cover = motionCover else { return }
        motionCover = nil
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        cover.removeFromSuperlayer()
        CATransaction.commit()
        motionTraceLanding(timedOut: timedOut)
    }

    private func motionTraceLanding(timedOut: Bool) {
        guard ProcessInfo.processInfo.environment["ND_MOTION_TRACE"] != nil else { return }
        let after = motionEndedAt > 0 ? (CACurrentMediaTime() - motionEndedAt) * 1000 : 0
        FileHandle.standardError.write(String(format: "ND_PAGE_MOTION node=0 held_ms=0 after_end_ms=%.1f resizes=1 timed_out=%@\n", after, timedOut ? "true" : "false").data(using: .utf8)!)
    }
}
#endif
