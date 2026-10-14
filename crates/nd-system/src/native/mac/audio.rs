//! `AVPlayer` playback. A spectrum player hangs an `MTAudioProcessingTap` off its item's audio
//! mix: the tap runs [`Analyzer`] on the realtime thread and the player's 15 Hz main-thread timer
//! drains the latest frame. The same timer watches the item's status, which saves a KVO class.

use std::cell::{RefCell, UnsafeCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::{self, NonNull};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2_av_foundation::{
    AVAssetTrack, AVMediaTypeAudio, AVMutableAudioMix, AVMutableAudioMixInputParameters, AVPlayer, AVPlayerItem,
    AVPlayerItemDidPlayToEndTimeNotification, AVPlayerItemStatus, AVURLAsset,
};
use objc2_core_audio_types::{AudioBufferList, AudioStreamBasicDescription, kAudioFormatFlagIsFloat};
use objc2_core_foundation::CFRetained;
use objc2_core_media::{CMItemCount, CMTime, CMTimeFlags, kCMTimeZero};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSError, NSNotification, NSNotificationCenter, NSOperationQueue, NSRunLoop,
    NSRunLoopCommonModes, NSString, NSTimer, NSURL,
};
use objc2_media_toolbox::{
    MTAudioProcessingTap, MTAudioProcessingTapCallbacks, MTAudioProcessingTapFlags,
    kMTAudioProcessingTapCallbacksVersion_0, kMTAudioProcessingTapCreationFlag_PostEffects,
};

use crate::backend::AudioControl;
use crate::spectrum::{Analyzer, BINS};
use crate::{AudioEvent, AudioSource, AudioState, Error, PlayOptions, PlayerId, Result, SystemEvent};

const DRAIN_INTERVAL: f64 = 1. / 15.;

thread_local! {
    static PLAYERS: RefCell<HashMap<PlayerId, Player>> = RefCell::new(HashMap::new());
}

struct Player {
    player: Retained<AVPlayer>,
    item: Retained<AVPlayerItem>,
    timer: Retained<NSTimer>,
    end_observer: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    spectrum: Option<Spectrum>,
    /// Set once Playing (or a control) has been reported, so readiness reports it only once.
    started: bool,
    failed: bool,
}

struct Spectrum {
    shared: Arc<TapShared>,
    tap: CFRetained<MTAudioProcessingTap>,
}

pub fn play(mtm: MainThreadMarker, id: PlayerId, options: PlayOptions) -> Result<()> {
    let url = match &options.source {
        AudioSource::Path(path) => {
            if !path.exists() {
                return Err(Error::Invalid(format!("audio file not found: {}", path.display())));
            }
            super::file_url(path)
        }
        AudioSource::Url(url) => NSURL::URLWithString(&NSString::from_str(url))
            .filter(|u| u.scheme().is_some())
            .ok_or_else(|| Error::Invalid(format!("invalid audio url: {url}")))?,
    };

    // SAFETY: no options dictionary; the item and player are created on the main thread.
    let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&url, None) };
    let item = unsafe { AVPlayerItem::playerItemWithAsset(&asset, mtm) };
    let player = unsafe { AVPlayer::playerWithPlayerItem(Some(&item), mtm) };
    unsafe { player.setVolume(options.volume.clamp(0., 1.)) };

    let on_end = RcBlock::new(move |_: NonNull<NSNotification>| ended(id));
    // SAFETY: the observer is removed in `release`; the main queue runs the block on the main
    // thread, where the player map lives.
    let end_observer = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(AVPlayerItemDidPlayToEndTimeNotification),
            Some(&item as &AnyObject),
            Some(&NSOperationQueue::mainQueue()),
            &on_end,
        )
    };

    let on_tick = RcBlock::new(move |_: NonNull<NSTimer>| tick(id));
    // SAFETY: the timer is invalidated in `release`, and fires on the main run loop.
    let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(DRAIN_INTERVAL, true, &on_tick) };
    unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };

    let spectrum = if options.spectrum { make_tap() } else { None };
    if spectrum.is_some()
        && let Some(audio) = unsafe { AVMediaTypeAudio }
    {
        let loaded = RcBlock::new(move |tracks: *mut NSArray<AVAssetTrack>, _: *mut NSError| {
            // SAFETY: AVFoundation passes a valid array or null.
            let Some(track) = unsafe { tracks.as_ref() }.and_then(|t| t.firstObject()) else { return };
            let track = SendTrack(track);
            super::on_main(move |_| attach_tap(id, track.into_inner()));
        });
        // SAFETY: the block captures only a `PlayerId` and posts its work to the main thread.
        unsafe { asset.loadTracksWithMediaType_completionHandler(audio, &loaded) };
    }

    unsafe { player.play() };
    PLAYERS.with_borrow_mut(|players| {
        players.insert(id, Player { player, item, timer, end_observer, spectrum, started: false, failed: false })
    });
    Ok(())
}

pub fn control(_mtm: MainThreadMarker, id: PlayerId, control: AudioControl) -> Result<()> {
    let event = PLAYERS.with_borrow_mut(|players| {
        let p = players.get_mut(&id).ok_or(Error::NotFound)?;
        Ok::<_, Error>(match control {
            AudioControl::Pause => {
                unsafe { p.player.pause() };
                p.started = true;
                Some(p.state(AudioState::Paused, None))
            }
            AudioControl::Resume => {
                unsafe { p.player.play() };
                p.started = true;
                Some(p.state(AudioState::Playing, None))
            }
            AudioControl::Stop => {
                let p = players.remove(&id).expect("looked up above");
                unsafe { p.player.pause() };
                let event = p.state(AudioState::Stopped, None);
                p.release();
                Some(event)
            }
            AudioControl::Seek(to) => {
                let time = unsafe { CMTime::new(to.as_millis().min(i64::MAX as u128) as i64, 1000) };
                unsafe { p.player.seekToTime_toleranceBefore_toleranceAfter(time, kCMTimeZero, kCMTimeZero) };
                None
            }
            AudioControl::SetVolume(volume) => {
                unsafe { p.player.setVolume(volume.clamp(0., 1.)) };
                None
            }
        })
    })?;
    if let Some(event) = event {
        emit(id, event);
    }
    Ok(())
}

impl Player {
    fn state(&self, state: AudioState, error: Option<String>) -> AudioEvent {
        let position = seconds(unsafe { self.player.currentTime() }).unwrap_or_default();
        let duration = seconds(unsafe { self.item.duration() });
        AudioEvent::State { state, position, duration, error }
    }

    fn release(self) {
        self.timer.invalidate();
        // SAFETY: the observer came from this center's block API.
        unsafe { NSNotificationCenter::defaultCenter().removeObserver(self.end_observer.as_ref()) };
        if self.spectrum.is_some() {
            unsafe { self.item.setAudioMix(None) };
        }
    }
}

fn seconds(time: CMTime) -> Option<Duration> {
    if !time.flags.contains(CMTimeFlags::Valid) || time.flags.intersects(CMTimeFlags::ImpliedValueFlagsMask) {
        return None;
    }
    let s = unsafe { time.seconds() };
    (s.is_finite() && s >= 0.).then(|| Duration::from_secs_f64(s))
}

fn emit(player: PlayerId, event: AudioEvent) {
    super::emitter().emit(SystemEvent::Audio { player, event });
}

fn tick(id: PlayerId) {
    let mut events = Vec::new();
    PLAYERS.with_borrow_mut(|players| {
        let Some(p) = players.get_mut(&id) else { return };
        match unsafe { p.item.status() } {
            AVPlayerItemStatus::ReadyToPlay if !p.started => {
                p.started = true;
                events.push(p.state(AudioState::Playing, None));
            }
            AVPlayerItemStatus::Failed if !p.failed => {
                p.failed = true;
                let message = unsafe { p.item.error() }
                    .map(|e| e.localizedDescription().to_string())
                    .unwrap_or_else(|| "playback failed".into());
                events.push(p.state(AudioState::Error, Some(message)));
            }
            _ => {}
        }
        // A paused player's tap still renders silence; those frames are not the sound's.
        if let Some(bins) = p.spectrum.as_ref().and_then(|s| s.shared.drain())
            && unsafe { p.player.rate() } != 0.
        {
            events.push(AudioEvent::Spectrum(bins.to_vec()));
        }
    });
    for event in events {
        emit(id, event);
    }
}

fn ended(id: PlayerId) {
    let event = PLAYERS.with_borrow(|players| {
        let p = players.get(&id)?;
        let mut event = p.state(AudioState::Ended, None);
        if let AudioEvent::State { position, duration: Some(duration), .. } = &mut event {
            *position = *duration;
        }
        Some(event)
    });
    if let Some(event) = event {
        emit(id, event);
    }
}

fn attach_tap(id: PlayerId, track: Retained<AVAssetTrack>) {
    PLAYERS.with_borrow(|players| {
        let Some(p) = players.get(&id) else { return };
        let Some(spectrum) = &p.spectrum else { return };
        // SAFETY: plain object construction and setters on the main thread; the mix copies its
        // input parameters and retains the tap.
        unsafe {
            let params = AVMutableAudioMixInputParameters::audioMixInputParametersWithTrack(Some(&track));
            params.setAudioTapProcessor(Some(&spectrum.tap));
            let mix = AVMutableAudioMix::audioMix();
            mix.setInputParameters(&NSArray::from_retained_slice(&[Retained::into_super(params)]));
            p.item.setAudioMix(Some(&mix));
        }
    });
}

/// An audio track from `loadTracksWithMediaType:completionHandler:`, handed to the main thread.
struct SendTrack(Retained<AVAssetTrack>);

// SAFETY: AVAssetTrack is immutable once loaded and documented as Sendable; the completion
// handler gives up its reference.
unsafe impl Send for SendTrack {}

impl SendTrack {
    fn into_inner(self) -> Retained<AVAssetTrack> {
        self.0
    }
}

/// What the tap's realtime thread shares with the main thread.
struct TapShared {
    /// Only `prepare` and `process` touch it, and MediaToolbox never runs them concurrently.
    analyzer: UnsafeCell<Option<Analyzer>>,
    latest: Mutex<Latest>,
}

struct Latest {
    bins: [f32; BINS],
    fresh: bool,
}

// SAFETY: `analyzer` is confined to the tap's callbacks as documented on the field.
unsafe impl Sync for TapShared {}

impl TapShared {
    fn drain(&self) -> Option<[f32; BINS]> {
        let mut latest = self.latest.lock().ok()?;
        std::mem::take(&mut latest.fresh).then_some(latest.bins)
    }
}

fn make_tap() -> Option<Spectrum> {
    let shared = Arc::new(TapShared {
        analyzer: UnsafeCell::new(None),
        latest: Mutex::new(Latest { bins: [0.; BINS], fresh: false }),
    });
    let client = Arc::into_raw(shared.clone()) as *mut c_void;
    let mut callbacks = MTAudioProcessingTapCallbacks {
        version: kMTAudioProcessingTapCallbacksVersion_0,
        clientInfo: client,
        init: Some(tap_init),
        finalize: Some(tap_finalize),
        prepare: Some(tap_prepare),
        unprepare: None,
        process: Some(tap_process),
    };
    let mut out: *const MTAudioProcessingTap = ptr::null();
    // SAFETY: the callbacks match MediaToolbox's signatures; `finalize` balances `into_raw`.
    let status = unsafe {
        MTAudioProcessingTap::create(
            None,
            NonNull::from(&mut callbacks),
            kMTAudioProcessingTapCreationFlag_PostEffects,
            NonNull::from(&mut out),
        )
    };
    match NonNull::new(out.cast_mut()) {
        // SAFETY: the create rule hands us one reference.
        Some(tap) if status == 0 => Some(Spectrum { shared, tap: unsafe { CFRetained::from_raw(tap) } }),
        _ => {
            // SAFETY: no tap exists to finalize, so the reference from `into_raw` is still ours.
            drop(unsafe { Arc::from_raw(client as *const TapShared) });
            None
        }
    }
}

unsafe fn shared<'a>(tap: NonNull<MTAudioProcessingTap>) -> &'a TapShared {
    unsafe { tap.as_ref().storage().cast::<TapShared>().as_ref() }
}

unsafe extern "C-unwind" fn tap_init(
    _: NonNull<MTAudioProcessingTap>,
    client: *mut c_void,
    storage: NonNull<*mut c_void>,
) {
    unsafe { storage.write(client) };
}

unsafe extern "C-unwind" fn tap_finalize(tap: NonNull<MTAudioProcessingTap>) {
    drop(unsafe { Arc::from_raw(tap.as_ref().storage().as_ptr() as *const TapShared) });
}

unsafe extern "C-unwind" fn tap_prepare(
    tap: NonNull<MTAudioProcessingTap>,
    _max_frames: CMItemCount,
    format: NonNull<AudioStreamBasicDescription>,
) {
    let format = unsafe { format.as_ref() };
    let float32 = format.mFormatFlags & kAudioFormatFlagIsFloat != 0 && format.mBitsPerChannel == 32;
    let shared = unsafe { shared(tap) };
    unsafe { *shared.analyzer.get() = float32.then(|| Analyzer::new(format.mSampleRate)) };
}

/// The realtime thread: no allocation and no blocking, so a frame the main thread is draining
/// is skipped rather than waited for.
unsafe extern "C-unwind" fn tap_process(
    tap: NonNull<MTAudioProcessingTap>,
    frames: CMItemCount,
    _flags: MTAudioProcessingTapFlags,
    buffers: NonNull<AudioBufferList>,
    frames_out: NonNull<CMItemCount>,
    flags_out: NonNull<MTAudioProcessingTapFlags>,
) {
    let status =
        unsafe { tap.as_ref().source_audio(frames, buffers, flags_out.as_ptr(), ptr::null_mut(), frames_out.as_ptr()) };
    if status != 0 {
        return;
    }
    let shared = unsafe { shared(tap) };
    let Some(analyzer) = (unsafe { &mut *shared.analyzer.get() }) else { return };
    let list = unsafe { buffers.as_ref() };
    let first = list.mBuffers[0];
    if list.mNumberBuffers == 0 || first.mData.is_null() {
        return;
    }
    // One buffer of N channels is interleaved; N buffers are planar. Either way, channel 0.
    let stride = if list.mNumberBuffers == 1 { first.mNumberChannels.max(1) as usize } else { 1 };
    let in_buffer = first.mDataByteSize as usize / size_of::<f32>() / stride;
    let count = (unsafe { *frames_out.as_ptr() }).max(0) as usize;
    let count = count.min(in_buffer);
    let samples = first.mData as *const f32;
    let bins = analyzer.process((0..count).map(|i| unsafe { *samples.add(i * stride) }));
    if let Ok(mut latest) = shared.latest.try_lock() {
        latest.bins = bins;
        latest.fresh = true;
    }
}
