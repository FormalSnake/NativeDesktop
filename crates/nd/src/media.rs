//! The media components wired to `nd::system`'s audio and media session: a player's state
//! events become the [`PlaybackClock`] a `player_bar`, `seek_bar` or `lyrics_view` reads, the
//! OS media keys become the same [`TransportCommand`]s the bar sends, and [`NowPlaying`] becomes
//! what the OS shows as Now Playing. Apps keep one handler for the bar and the media keys.

use nd_components::media::{NowPlaying, PlaybackClock, TransportCommand};
use nd_core::time::{Duration, Instant};
use nd_system::{AudioEvent, AudioState, MediaCommand, PlaybackState};

/// The clock a player's state event reports, or `None` for a spectrum frame. A player that
/// ended stands at its end; one that stopped or failed, at its last position.
pub fn clock_from(event: &AudioEvent) -> Option<(PlaybackClock, Option<Duration>)> {
    let AudioEvent::State { state, position, duration, .. } = event else { return None };
    let clock = match state {
        AudioState::Playing => PlaybackClock::playing(*position),
        AudioState::Ended => PlaybackClock::paused(duration.unwrap_or(*position)),
        AudioState::Paused | AudioState::Stopped | AudioState::Error => PlaybackClock::paused(*position),
    };
    Some((clock.reported_at(Instant::now()), *duration))
}

/// What a press of the OS media controls asks the bar's handler to do. `position` is where
/// playback stands now, for relative seeks.
pub fn command_from(command: MediaCommand, playing: bool, position: Duration) -> Option<TransportCommand> {
    Some(match command {
        MediaCommand::Toggle => TransportCommand::TogglePlay,
        MediaCommand::Play if !playing => TransportCommand::TogglePlay,
        MediaCommand::Pause | MediaCommand::Stop if playing => TransportCommand::TogglePlay,
        MediaCommand::Play | MediaCommand::Pause | MediaCommand::Stop => return None,
        MediaCommand::Next => TransportCommand::Next,
        MediaCommand::Previous => TransportCommand::Previous,
        MediaCommand::SeekBy(ms) => {
            let delta = Duration::from_millis(ms.unsigned_abs());
            TransportCommand::Seek(if ms < 0 { position.saturating_sub(delta) } else { position + delta })
        }
        MediaCommand::SetPosition(at) => TransportCommand::Seek(at),
    })
}

/// What the OS Now Playing shows for the bar's [`NowPlaying`]. Artwork crosses only as a URL,
/// so bytes and paths the bar shows are left out unless `artwork_url` names them.
pub fn now_playing(now: &NowPlaying, artwork_url: Option<String>) -> nd_system::NowPlaying {
    nd_system::NowPlaying {
        title: now.title.to_string(),
        artist: now.subtitle.as_ref().map(|s| s.to_string()),
        album: None,
        duration: now.duration,
        position: Some(now.clock.now(now.duration)),
        artwork_url,
        state: if now.playing() { PlaybackState::Playing } else { PlaybackState::Paused },
        can_next: now.can_next,
        can_previous: now.can_previous,
        can_seek: now.duration.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_events_become_clocks() {
        let state = |state, secs| AudioEvent::State {
            state,
            position: Duration::from_secs(secs),
            duration: Some(Duration::from_secs(80)),
            error: None,
        };
        let (clock, duration) = clock_from(&state(AudioState::Playing, 12)).unwrap();
        assert!(clock.playing && clock.position == Duration::from_secs(12));
        assert_eq!(duration, Some(Duration::from_secs(80)));
        let (ended, _) = clock_from(&state(AudioState::Ended, 79)).unwrap();
        assert!(!ended.playing && ended.position == Duration::from_secs(80), "an ended player stands at its end");
        assert!(clock_from(&AudioEvent::Spectrum(vec![0.; 4])).is_none());
    }

    #[test]
    fn media_keys_become_bar_commands() {
        let at = Duration::from_secs(10);
        assert_eq!(command_from(MediaCommand::Toggle, true, at), Some(TransportCommand::TogglePlay));
        assert_eq!(command_from(MediaCommand::Play, true, at), None, "already playing");
        assert_eq!(command_from(MediaCommand::Pause, true, at), Some(TransportCommand::TogglePlay));
        assert_eq!(command_from(MediaCommand::SeekBy(-15_000), true, at), Some(TransportCommand::Seek(Duration::ZERO)));
        assert_eq!(
            command_from(MediaCommand::SeekBy(5_000), true, at),
            Some(TransportCommand::Seek(Duration::from_secs(15)))
        );
        let now = NowPlaying::new("Tone").subtitle("Gallery").duration(Duration::from_secs(80));
        let os = now_playing(&now, None);
        assert_eq!(
            (os.title.as_str(), os.artist.as_deref(), os.state),
            ("Tone", Some("Gallery"), PlaybackState::Paused)
        );
        assert!(os.can_seek);
    }
}
