import Accelerate
import AVFoundation
import AppKit
import Foundation
import MediaToolbox

/// AppKit half of the `audio.*` method family on the system-capabilities seam
/// (routed from `NDSystem.handleRequest`). Playback rides an `AVPlayer` (one
/// code path for local files and remote URLs); an optional spectrum analyzer
/// hangs an `MTAudioProcessingTap` off the player item's audio mix, folds a
/// 1024-point vDSP FFT into 32 log-spaced bands on the realtime audio thread,
/// and a 15 Hz main-thread timer drains the latest frame out as an
/// `audio.spectrum` event. State transitions push as `audio.state` events. The
/// core pre-gates `core:audio` and marshals every request onto the main actor,
/// so the registry and every player live MainActor-isolated.
enum NDAudio {
    @MainActor private static var players: [String: NDAudioPlayer] = [:]
    @MainActor private static var counter: Int = 0

    // MARK: - dispatch

    @MainActor static func play(_ id: UInt32, _ p: [String: Any]) {
        let path = propStr(p, "path")
        let urlStr = propStr(p, "url")
        switch (path, urlStr) {
        case (.some, .some), (.none, .none):
            NDSystem.respondError(id, "audio.play requires exactly one of path or url")
            return
        default:
            break
        }

        let mediaURL: URL
        if let path {
            guard FileManager.default.fileExists(atPath: path) else {
                NDSystem.respondError(id, "audio file not found: \(path)")
                return
            }
            mediaURL = URL(fileURLWithPath: path)
        } else {
            guard let parsed = URL(string: urlStr!), parsed.scheme != nil else {
                NDSystem.respondError(id, "invalid audio url: \(urlStr ?? "")")
                return
            }
            mediaURL = parsed
        }

        let volume = clampVolume(propDouble(p, "volume") ?? 1.0)
        let spectrum = propBool(p, "spectrum") ?? false

        counter += 1
        let handle = "audio-\(counter)"
        let player = NDAudioPlayer(handle: handle, url: mediaURL, volume: volume, spectrum: spectrum)
        players[handle] = player
        player.start()
        NDSystem.respondResult(id, NDSystem.jsonFragment(handle))
    }

    @MainActor static func pause(_ id: UInt32, _ p: [String: Any]) {
        guard let player = resolve(id, p) else { return }
        player.pause()
        NDSystem.respondResult(id, "null")
    }

    @MainActor static func resume(_ id: UInt32, _ p: [String: Any]) {
        guard let player = resolve(id, p) else { return }
        player.resume()
        NDSystem.respondResult(id, "null")
    }

    @MainActor static func stop(_ id: UInt32, _ p: [String: Any]) {
        guard let handle = propStr(p, "handle"), let player = players[handle] else {
            NDSystem.respondError(id, "unknown audio handle")
            return
        }
        player.stop()
        players.removeValue(forKey: handle)
        NDSystem.respondResult(id, "null")
    }

    @MainActor static func seek(_ id: UInt32, _ p: [String: Any]) {
        guard let player = resolve(id, p) else { return }
        player.seek(toMs: propDouble(p, "position") ?? 0)
        NDSystem.respondResult(id, "null")
    }

    @MainActor static func setVolume(_ id: UInt32, _ p: [String: Any]) {
        guard let player = resolve(id, p) else { return }
        player.setVolume(clampVolume(propDouble(p, "volume") ?? 1.0))
        NDSystem.respondResult(id, "null")
    }

    /// Looks up the player named by `handle`, answering `ok=false "unknown
    /// audio handle"` (and returning nil) when the handle is absent/released.
    @MainActor private static func resolve(_ id: UInt32, _ p: [String: Any]) -> NDAudioPlayer? {
        guard let handle = propStr(p, "handle"), let player = players[handle] else {
            NDSystem.respondError(id, "unknown audio handle")
            return nil
        }
        return player
    }

    private static func clampVolume(_ v: Double) -> Float { Float(min(max(v, 0), 1)) }
}

/// One AVPlayer-backed sound. Owns its KVO/notification observers and, when
/// spectrum analysis is requested, the audio-mix tap + the 15 Hz drain timer.
@MainActor final class NDAudioPlayer {
    let handle: String

    private let asset: AVURLAsset
    private let item: AVPlayerItem
    private let player: AVPlayer
    private let wantSpectrum: Bool

    private var statusObs: NSKeyValueObservation?
    private var endObs: NSObjectProtocol?
    private var started = false
    private var duration: Int?

    private var spectrumTap: SpectrumTap?
    private var spectrumTimer: Timer?

    init(handle: String, url: URL, volume: Float, spectrum: Bool) {
        self.handle = handle
        self.wantSpectrum = spectrum
        asset = AVURLAsset(url: url)
        item = AVPlayerItem(asset: asset)
        player = AVPlayer(playerItem: item)
        player.volume = volume
    }

    func start() {
        statusObs = item.observe(\.status, options: [.new]) { [weak self] _, _ in
            DispatchQueue.main.async { MainActor.assumeIsolated { self?.statusChanged() } }
        }
        endObs = NotificationCenter.default.addObserver(
            forName: AVPlayerItem.didPlayToEndTimeNotification, object: item, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.ended() }
        }
        player.play()
        if wantSpectrum { setupSpectrum() }
    }

    func pause() {
        player.pause()
        emitState("paused")
    }

    func resume() {
        player.play()
        emitState("playing")
    }

    func stop() {
        player.pause()
        emitState("stopped")
        teardown()
    }

    func seek(toMs ms: Double) {
        let time = CMTime(value: CMTimeValue(ms.rounded()), timescale: 1000)
        player.seek(to: time, toleranceBefore: .zero, toleranceAfter: .zero)
    }

    func setVolume(_ v: Float) {
        player.volume = v
    }

    // MARK: - state

    private func statusChanged() {
        switch item.status {
        case .readyToPlay:
            updateDuration()
            if !started {
                started = true
                emitState("playing")
            }
        case .failed:
            stopSpectrum()
            emitState("error", error: item.error?.localizedDescription ?? "playback failed")
        default:
            break
        }
    }

    private func ended() {
        stopSpectrum()
        emitState("ended")
    }

    private func updateDuration() {
        let d = item.duration
        if d.isNumeric {
            duration = Int((CMTimeGetSeconds(d) * 1000).rounded())
        }
    }

    private func positionMs() -> Int {
        let seconds = CMTimeGetSeconds(player.currentTime())
        guard seconds.isFinite else { return 0 }
        return Int((seconds * 1000).rounded())
    }

    private func emitState(_ state: String, error: String? = nil) {
        let durationJson = duration.map(String.init) ?? "null"
        var json = "{\"handle\":\(NDSystem.jsonFragment(handle)),\"state\":\(NDSystem.jsonFragment(state))"
        json += ",\"position\":\(positionMs()),\"duration\":\(durationJson)"
        if let error {
            json += ",\"error\":\(NDSystem.jsonFragment(error))"
        }
        json += "}"
        NDSystem.emitEvent(channel: "audio.state", dataJson: json)
    }

    private func teardown() {
        stopSpectrum()
        statusObs?.invalidate()
        statusObs = nil
        if let endObs {
            NotificationCenter.default.removeObserver(endObs)
        }
        endObs = nil
    }

    // MARK: - spectrum

    private func setupSpectrum() {
        let tap = SpectrumTap()
        spectrumTap = tap
        // The audio track must be loaded before an input-parameters tap can bind
        // to it; loadTracks is async, so playback (already started) runs a beat
        // before the first spectrum frame — acceptable for a visualizer.
        Task { @MainActor in
            guard let tracks = try? await asset.loadTracks(withMediaType: .audio),
                  let track = tracks.first,
                  let processor = tap.makeProcessingTap()
            else { return }
            let params = AVMutableAudioMixInputParameters(track: track)
            params.audioTapProcessor = processor
            let mix = AVMutableAudioMix()
            mix.inputParameters = [params]
            item.audioMix = mix
            startSpectrumTimer()
        }
    }

    private func startSpectrumTimer() {
        spectrumTimer?.invalidate()
        let timer = Timer(timeInterval: 1.0 / 15.0, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.pushSpectrum() }
        }
        RunLoop.main.add(timer, forMode: .common)
        spectrumTimer = timer
    }

    private func pushSpectrum() {
        guard let bins = spectrumTap?.drainLatest() else { return }
        let list = bins.map { String(format: "%.4f", $0) }.joined(separator: ",")
        NDSystem.emitEvent(
            channel: "audio.spectrum",
            dataJson: "{\"handle\":\(NDSystem.jsonFragment(handle)),\"bins\":[\(list)]}")
    }

    private func stopSpectrum() {
        spectrumTimer?.invalidate()
        spectrumTimer = nil
        if spectrumTap != nil {
            item.audioMix = nil
            spectrumTap = nil
        }
    }
}

/// Realtime FFT analyzer driven by an `MTAudioProcessingTap`. The tap's
/// `process` callback fires on a realtime audio thread, so it must never
/// allocate or emit events: it computes the 32 band magnitudes into a
/// preallocated buffer and stashes the latest frame under a lock; the player's
/// main-thread timer drains it (`drainLatest`) and emits. `@unchecked Sendable`
/// because the lock guards the one field crossing threads (`latestBins`); the
/// FFT scratch buffers are touched only on the realtime thread.
final class SpectrumTap: @unchecked Sendable {
    private static let fftSize = 1024
    private static let log2n = vDSP_Length(10)
    private static let bandCount = 32
    private static let minHz = 50.0
    private static let maxHz = 16000.0
    private static let minDb: Float = -60
    private static let maxDb: Float = 0
    private static let decay: Float = 0.90  // per-frame release of the peak envelope

    private let fftSetup = vDSP_create_fftsetup(SpectrumTap.log2n, FFTRadix(kFFTRadix2))
    private let windowBuf = UnsafeMutablePointer<Float>.allocate(capacity: SpectrumTap.fftSize)
    private let inputBuf = UnsafeMutablePointer<Float>.allocate(capacity: SpectrumTap.fftSize)
    private let windowedBuf = UnsafeMutablePointer<Float>.allocate(capacity: SpectrumTap.fftSize)
    private let realp = UnsafeMutablePointer<Float>.allocate(capacity: SpectrumTap.fftSize / 2)
    private let imagp = UnsafeMutablePointer<Float>.allocate(capacity: SpectrumTap.fftSize / 2)
    private let magsBuf = UnsafeMutablePointer<Float>.allocate(capacity: SpectrumTap.fftSize / 2)

    private var bandRanges: [(lo: Int, hi: Int)] = []
    private var envelope = [Float](repeating: 0, count: SpectrumTap.bandCount)

    private let lock = NSLock()
    private var latestBins = [Float](repeating: 0, count: SpectrumTap.bandCount)
    private var hasNew = false

    init() {
        vDSP_hann_window(windowBuf, vDSP_Length(SpectrumTap.fftSize), Int32(vDSP_HANN_NORM))
    }

    deinit {
        if let fftSetup { vDSP_destroy_fftsetup(fftSetup) }
        windowBuf.deallocate()
        inputBuf.deallocate()
        windowedBuf.deallocate()
        realp.deallocate()
        imagp.deallocate()
        magsBuf.deallocate()
    }

    /// Builds the tap, handing it a retained reference to `self` as its client
    /// info (released in the finalize callback). Returns nil on failure, having
    /// balanced that retain so `self` is not leaked.
    func makeProcessingTap() -> MTAudioProcessingTap? {
        var callbacks = MTAudioProcessingTapCallbacks(
            version: kMTAudioProcessingTapCallbacksVersion_0,
            clientInfo: UnsafeMutableRawPointer(Unmanaged.passRetained(self).toOpaque()),
            init: tapInit,
            finalize: tapFinalize,
            prepare: tapPrepare,
            unprepare: tapUnprepare,
            process: tapProcess)
        var tap: MTAudioProcessingTap?
        let status = MTAudioProcessingTapCreate(
            kCFAllocatorDefault, &callbacks, kMTAudioProcessingTapCreationFlag_PostEffects, &tap)
        guard status == noErr, let tap else {
            if let clientInfo = callbacks.clientInfo {
                Unmanaged<SpectrumTap>.fromOpaque(clientInfo).release()
            }
            return nil
        }
        return tap
    }

    func prepare(sampleRate: Double) {
        var ranges: [(lo: Int, hi: Int)] = []
        let n = Double(SpectrumTap.fftSize)
        let maxBin = SpectrumTap.fftSize / 2 - 1
        let ratio = SpectrumTap.maxHz / SpectrumTap.minHz
        for i in 0..<SpectrumTap.bandCount {
            let loHz = SpectrumTap.minHz * pow(ratio, Double(i) / Double(SpectrumTap.bandCount))
            let hiHz = SpectrumTap.minHz * pow(ratio, Double(i + 1) / Double(SpectrumTap.bandCount))
            let rawLo = Int((loHz * n / sampleRate).rounded(.down))
            let rawHi = Int((hiHz * n / sampleRate).rounded(.up))
            // Clamp both ends into [1, maxBin] so bands whose centers exceed the
            // signal's Nyquist collapse to the top bin rather than invert the range.
            let lo = min(max(1, rawLo), maxBin)
            let hi = min(max(lo, rawHi), maxBin)
            ranges.append((lo, hi))
        }
        bandRanges = ranges
    }

    /// Realtime audio thread — no allocations, no event emission.
    func process(_ bufferList: UnsafeMutablePointer<AudioBufferList>, frames: Int) {
        guard let fftSetup, frames > 0, !bandRanges.isEmpty else { return }
        let abl = UnsafeMutableAudioBufferListPointer(bufferList)
        guard let first = abl.first, let raw = first.mData else { return }

        let fftSize = SpectrumTap.fftSize
        let channels = Int(first.mNumberChannels)
        // One buffer holding N channels means interleaved; N buffers means each
        // is one planar channel. Either way, read channel 0.
        let stride = (abl.count == 1 && channels > 1) ? channels : 1
        let available = min(frames, fftSize)
        let src = raw.assumingMemoryBound(to: Float.self)
        if stride == 1 {
            inputBuf.update(from: src, count: available)
        } else {
            for i in 0..<available { inputBuf[i] = src[i * stride] }
        }
        if available < fftSize {
            vDSP_vclr(inputBuf + available, 1, vDSP_Length(fftSize - available))
        }

        vDSP_vmul(inputBuf, 1, windowBuf, 1, windowedBuf, 1, vDSP_Length(fftSize))
        var split = DSPSplitComplex(realp: realp, imagp: imagp)
        windowedBuf.withMemoryRebound(to: DSPComplex.self, capacity: fftSize / 2) { complex in
            vDSP_ctoz(complex, 2, &split, 1, vDSP_Length(fftSize / 2))
        }
        vDSP_fft_zrip(fftSetup, &split, 1, SpectrumTap.log2n, FFTDirection(FFT_FORWARD))
        vDSP_zvabs(&split, 1, magsBuf, 1, vDSP_Length(fftSize / 2))

        // zrip magnitudes run ~2x an amplitude spectrum; 1/N brings a
        // full-scale tone to roughly unity before the dB map.
        let scale = 1.0 / Float(fftSize)
        let range = SpectrumTap.maxDb - SpectrumTap.minDb
        for (i, band) in bandRanges.enumerated() {
            var sum: Float = 0
            for k in band.lo...band.hi { sum += magsBuf[k] }
            let mag = (sum / Float(band.hi - band.lo + 1)) * scale
            let db = 20 * log10f(mag + 1e-9)
            let norm = min(max((db - SpectrumTap.minDb) / range, 0), 1)
            // Fast attack, slow release — steadies the bars without smearing peaks.
            envelope[i] = norm > envelope[i] ? norm : envelope[i] * SpectrumTap.decay
        }

        lock.lock()
        latestBins = envelope
        hasNew = true
        lock.unlock()
    }

    /// Main thread — returns the latest frame once, or nil if none since the
    /// last drain (so a paused/idle tap emits nothing).
    func drainLatest() -> [Float]? {
        lock.lock()
        defer { lock.unlock() }
        guard hasNew else { return nil }
        hasNew = false
        return latestBins
    }
}

// MARK: - MTAudioProcessingTap C callbacks (no captures — plain functions)

private func tapInit(
    _ tap: MTAudioProcessingTap, _ clientInfo: UnsafeMutableRawPointer?,
    _ tapStorageOut: UnsafeMutablePointer<UnsafeMutableRawPointer?>
) {
    tapStorageOut.pointee = clientInfo
}

private func tapFinalize(_ tap: MTAudioProcessingTap) {
    Unmanaged<SpectrumTap>.fromOpaque(MTAudioProcessingTapGetStorage(tap)).release()
}

private func tapPrepare(
    _ tap: MTAudioProcessingTap, _ maxFrames: CMItemCount,
    _ processingFormat: UnsafePointer<AudioStreamBasicDescription>
) {
    let analyzer = Unmanaged<SpectrumTap>.fromOpaque(MTAudioProcessingTapGetStorage(tap)).takeUnretainedValue()
    analyzer.prepare(sampleRate: processingFormat.pointee.mSampleRate)
}

private func tapUnprepare(_ tap: MTAudioProcessingTap) {}

private func tapProcess(
    _ tap: MTAudioProcessingTap, _ numberFrames: CMItemCount, _ flags: MTAudioProcessingTapFlags,
    _ bufferListInOut: UnsafeMutablePointer<AudioBufferList>,
    _ numberFramesOut: UnsafeMutablePointer<CMItemCount>,
    _ flagsOut: UnsafeMutablePointer<MTAudioProcessingTapFlags>
) {
    let status = MTAudioProcessingTapGetSourceAudio(
        tap, numberFrames, bufferListInOut, flagsOut, nil, numberFramesOut)
    guard status == noErr else { return }
    let analyzer = Unmanaged<SpectrumTap>.fromOpaque(MTAudioProcessingTapGetStorage(tap)).takeUnretainedValue()
    analyzer.process(bufferListInOut, frames: Int(numberFramesOut.pointee))
}
