//! Real playback through the native backend: a generated tone plays, its spectrum lights the
//! tone's band, the controls answer, it ends, and a stopped player is gone. `harness = false`
//! because AVPlayer delivers on the main run loop, which the test spins on the main thread.
//!
//! On Linux without GStreamer, play must fail with "audio unavailable"; `ND_AUDIO_REQUIRE=1`
//! turns that into a failure, for a run where GStreamer is known to be present.

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn main() {
    real::run();
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() {}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod real {
    use std::f64::consts::PI;
    use std::path::Path;
    use std::time::{Duration, Instant};

    use futures_executor::block_on;
    use nd_system::spectrum::{BINS, HIGH_HZ, LOW_HZ};
    use nd_system::{AudioEvent, AudioState, Error, Events, PlayOptions, PlayerId, System, SystemEvent};

    const RATE: u32 = 44_100;
    const TONE_HZ: f64 = 1_000.;
    const SECONDS: f64 = 1.5;
    /// Silent: both backends analyze the sound before the player's volume applies.
    const VOLUME: f32 = 0.;

    pub fn run() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tone = dir.path().join("tone.wav");
        write_tone(&tone);

        let system = System::native();
        let mut events = system.events();
        let player = match block_on(system.audio().play(PlayOptions::path(&tone).volume(VOLUME).spectrum())) {
            Ok(player) => player,
            Err(Error::Platform(message))
                if cfg!(target_os = "linux")
                    && message.starts_with("audio unavailable")
                    && std::env::var_os("ND_AUDIO_REQUIRE").is_none() =>
            {
                println!("audio_playback: {message}, the clean degrade");
                return;
            }
            Err(e) => panic!("play failed: {e}"),
        };
        let id = player.id();

        let playing = wait_state(&mut events, id, Duration::from_secs(5));
        assert_eq!(playing.0, AudioState::Playing, "first transition");
        assert_eq!(playing.2.map(|d| (d.as_secs_f64() * 10.).round() / 10.), Some(SECONDS), "duration");

        let expected = band_of(TONE_HZ);
        let mut lit = 0;
        let deadline = Instant::now() + Duration::from_secs(3);
        while lit < 5 {
            assert!(Instant::now() < deadline, "only {lit} lit spectrum frames in 3 s");
            if let Some(AudioEvent::Spectrum(bins)) = next_for(&mut events, id, deadline) {
                assert_eq!(bins.len(), BINS);
                let (loudest, level) = loudest(&bins);
                if level > 0.1 {
                    assert!(
                        loudest.abs_diff(expected) <= 1,
                        "{TONE_HZ} Hz lit bin {loudest}, expected {expected}: {bins:?}"
                    );
                    lit += 1;
                }
            }
        }

        block_on(player.pause()).expect("pause");
        assert_eq!(wait_state(&mut events, id, Duration::from_secs(2)).0, AudioState::Paused);
        pump(Duration::from_millis(300));
        while events.try_next().is_some() {}
        pump(Duration::from_millis(400));
        while let Some(event) = events.try_next() {
            assert!(
                !matches!(event, SystemEvent::Audio { event: AudioEvent::Spectrum(_), .. }),
                "spectrum while paused"
            );
        }

        block_on(player.set_volume(0.)).expect("set_volume");
        block_on(player.seek(Duration::from_millis(500))).expect("seek");
        block_on(player.resume()).expect("resume");
        assert_eq!(wait_state(&mut events, id, Duration::from_secs(2)).0, AudioState::Playing);
        let ended = wait_state(&mut events, id, Duration::from_secs(5));
        assert_eq!(ended.0, AudioState::Ended);
        assert!(ended.1 >= Duration::from_millis(1_400), "ended at {:?}", ended.1);

        block_on(player.stop()).expect("stop");
        assert_eq!(wait_state(&mut events, id, Duration::from_secs(2)).0, AudioState::Stopped);
        assert!(matches!(block_on(player.pause()), Err(Error::NotFound)), "a stopped player is gone");
        assert!(matches!(block_on(player.stop()), Err(Error::NotFound)));

        let junk = dir.path().join("junk.wav");
        std::fs::write(&junk, b"RIFF this is not audio at all, just bytes").expect("write junk");
        let bad = block_on(system.audio().play(PlayOptions::path(&junk).volume(0.))).expect("play junk");
        let failed = wait_state(&mut events, bad.id(), Duration::from_secs(5));
        assert_eq!(failed.0, AudioState::Error);
        assert!(failed.3.is_some_and(|m| !m.is_empty()), "error carries a message");
        block_on(bad.stop()).expect("stop the failed player");

        let missing = block_on(system.audio().play(PlayOptions::path(dir.path().join("missing.wav"))));
        assert!(matches!(missing, Err(Error::Invalid(_))), "missing file: {:?}", missing.err());

        println!("audio_playback: ok");
    }

    /// Spins the main run loop (macOS) or sleeps (Linux, where a bus thread delivers).
    fn pump(for_: Duration) {
        #[cfg(target_os = "macos")]
        {
            use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSRunLoop};
            let until = Instant::now() + for_;
            while Instant::now() < until {
                let date = NSDate::dateWithTimeIntervalSinceNow(0.01);
                // SAFETY: a framework constant.
                NSRunLoop::mainRunLoop().runMode_beforeDate(unsafe { NSDefaultRunLoopMode }, &date);
            }
        }
        #[cfg(target_os = "linux")]
        std::thread::sleep(for_);
    }

    fn next_for(events: &mut Events, id: PlayerId, deadline: Instant) -> Option<AudioEvent> {
        while Instant::now() < deadline {
            match events.try_next() {
                Some(SystemEvent::Audio { player, event }) if player == id => return Some(event),
                Some(_) => {}
                None => pump(Duration::from_millis(10)),
            }
        }
        None
    }

    /// The next state transition for `id`, skipping spectrum frames.
    fn wait_state(
        events: &mut Events,
        id: PlayerId,
        timeout: Duration,
    ) -> (AudioState, Duration, Option<Duration>, Option<String>) {
        let deadline = Instant::now() + timeout;
        loop {
            match next_for(events, id, deadline) {
                Some(AudioEvent::State { state, position, duration, error }) => {
                    return (state, position, duration, error);
                }
                Some(AudioEvent::Spectrum(_)) => {}
                None => panic!("no state change for {id:?} in {timeout:?}"),
            }
        }
    }

    fn band_of(hz: f64) -> usize {
        ((hz / LOW_HZ).ln() / (HIGH_HZ / LOW_HZ).ln() * BINS as f64).floor() as usize
    }

    fn loudest(bins: &[f32]) -> (usize, f32) {
        bins.iter().copied().enumerate().max_by(|a, b| a.1.total_cmp(&b.1)).expect("bins")
    }

    /// A 16-bit mono WAV of a half-scale sine.
    fn write_tone(path: &Path) {
        let frames = (RATE as f64 * SECONDS) as u32;
        let mut wav = Vec::with_capacity(44 + frames as usize * 2);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + frames * 2).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&RATE.to_le_bytes());
        wav.extend_from_slice(&(RATE * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(frames * 2).to_le_bytes());
        for i in 0..frames {
            let s = 0.5 * (2. * PI * TONE_HZ * i as f64 / RATE as f64).sin();
            wav.extend_from_slice(&((s * i16::MAX as f64) as i16).to_le_bytes());
        }
        std::fs::write(path, wav).expect("write tone");
    }
}
