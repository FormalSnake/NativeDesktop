//! Now Playing (Control Center, the menu bar's media item) and the remote commands media keys
//! and AirPods send. Command handlers are registered with the first session and stay; clearing
//! the session empties the info and disables the commands.

use std::cell::Cell;
use std::ptr::NonNull;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{MainThreadMarker, NSDictionary, NSNumber, NSString};
use objc2_media_player::{
    MPChangePlaybackPositionCommandEvent, MPMediaItemPropertyAlbumTitle, MPMediaItemPropertyArtist,
    MPMediaItemPropertyPlaybackDuration, MPMediaItemPropertyTitle, MPNowPlayingInfoCenter,
    MPNowPlayingInfoPropertyElapsedPlaybackTime, MPNowPlayingInfoPropertyPlaybackRate, MPNowPlayingPlaybackState,
    MPRemoteCommand, MPRemoteCommandCenter, MPRemoteCommandEvent, MPRemoteCommandHandlerStatus,
    MPSkipIntervalCommandEvent,
};

use crate::{MediaCommand, NowPlaying, PlaybackState, Result, SystemEvent};

thread_local! {
    static REGISTERED: Cell<bool> = const { Cell::new(false) };
}

fn handle(command: &MPRemoteCommand, f: impl Fn(&MPRemoteCommandEvent) -> Option<MediaCommand> + 'static) {
    let block = RcBlock::new(move |event: NonNull<MPRemoteCommandEvent>| {
        // SAFETY: MediaPlayer passes a live event for the duration of the call.
        match f(unsafe { event.as_ref() }) {
            Some(command) => {
                super::emitter().emit(SystemEvent::Media(command));
                MPRemoteCommandHandlerStatus::Success
            }
            None => MPRemoteCommandHandlerStatus::CommandFailed,
        }
    });
    // SAFETY: the returned target token is retained by the command for the life of the process.
    let _token = unsafe { command.addTargetWithHandler(&block) };
}

/// Every command nd forwards, each with whether the session enables it.
fn commands(center: &MPRemoteCommandCenter, now: Option<&NowPlaying>) -> Vec<(Retained<MPRemoteCommand>, bool)> {
    let on = now.is_some();
    // SAFETY: plain getters on the shared command center.
    unsafe {
        vec![
            (center.playCommand(), on),
            (center.pauseCommand(), on),
            (center.togglePlayPauseCommand(), on),
            (center.stopCommand(), on),
            (center.nextTrackCommand(), now.is_some_and(|n| n.can_next)),
            (center.previousTrackCommand(), now.is_some_and(|n| n.can_previous)),
            (Retained::into_super(center.changePlaybackPositionCommand()), now.is_some_and(|n| n.can_seek)),
            (Retained::into_super(center.skipForwardCommand()), now.is_some_and(|n| n.can_seek)),
            (Retained::into_super(center.skipBackwardCommand()), now.is_some_and(|n| n.can_seek)),
        ]
    }
}

fn register(center: &MPRemoteCommandCenter) {
    if REGISTERED.replace(true) {
        return;
    }
    // SAFETY: plain getters on the shared command center; the downcasts name the event class
    // each command documents.
    unsafe {
        handle(&center.playCommand(), |_| Some(MediaCommand::Play));
        handle(&center.pauseCommand(), |_| Some(MediaCommand::Pause));
        handle(&center.togglePlayPauseCommand(), |_| Some(MediaCommand::Toggle));
        handle(&center.stopCommand(), |_| Some(MediaCommand::Stop));
        handle(&center.nextTrackCommand(), |_| Some(MediaCommand::Next));
        handle(&center.previousTrackCommand(), |_| Some(MediaCommand::Previous));
        handle(&center.changePlaybackPositionCommand(), |e| {
            let e = &*(e as *const MPRemoteCommandEvent).cast::<MPChangePlaybackPositionCommandEvent>();
            Some(MediaCommand::SetPosition(Duration::from_secs_f64(e.positionTime().max(0.))))
        });
        handle(&center.skipForwardCommand(), |e| {
            let e = &*(e as *const MPRemoteCommandEvent).cast::<MPSkipIntervalCommandEvent>();
            Some(MediaCommand::SeekBy((e.interval() * 1000.) as i64))
        });
        handle(&center.skipBackwardCommand(), |e| {
            let e = &*(e as *const MPRemoteCommandEvent).cast::<MPSkipIntervalCommandEvent>();
            Some(MediaCommand::SeekBy(-(e.interval() * 1000.) as i64))
        });
    }
}

fn info(now: &NowPlaying) -> Retained<NSDictionary<NSString, AnyObject>> {
    let mut keys: Vec<&NSString> = Vec::new();
    let mut values: Vec<Retained<AnyObject>> = Vec::new();
    let text = |s: &str| -> Retained<AnyObject> { NSString::from_str(s).into() };
    let number = |v: f64| -> Retained<AnyObject> { NSNumber::new_f64(v).into() };
    // SAFETY: framework constants, valid for the life of the process.
    unsafe {
        keys.push(MPMediaItemPropertyTitle);
        values.push(text(&now.title));
        if let Some(artist) = &now.artist {
            keys.push(MPMediaItemPropertyArtist);
            values.push(text(artist));
        }
        if let Some(album) = &now.album {
            keys.push(MPMediaItemPropertyAlbumTitle);
            values.push(text(album));
        }
        if let Some(duration) = now.duration {
            keys.push(MPMediaItemPropertyPlaybackDuration);
            values.push(number(duration.as_secs_f64()));
        }
        if let Some(position) = now.position {
            keys.push(MPNowPlayingInfoPropertyElapsedPlaybackTime);
            values.push(number(position.as_secs_f64()));
        }
        keys.push(MPNowPlayingInfoPropertyPlaybackRate);
        values.push(number(if now.state == PlaybackState::Playing { 1. } else { 0. }));
    }
    let values: Vec<&AnyObject> = values.iter().map(|v| &**v).collect();
    NSDictionary::from_slices(&keys, &values)
}

pub fn set(_mtm: MainThreadMarker, now: Option<NowPlaying>) -> Result<()> {
    // SAFETY: the shared center and command center, used on the main thread.
    unsafe {
        let center = MPNowPlayingInfoCenter::defaultCenter();
        let commands_center = MPRemoteCommandCenter::sharedCommandCenter();
        register(&commands_center);
        for (command, enabled) in commands(&commands_center, now.as_ref()) {
            command.setEnabled(enabled);
        }
        match &now {
            Some(now) => {
                center.setNowPlayingInfo(Some(&info(now)));
                center.setPlaybackState(match now.state {
                    PlaybackState::Playing => MPNowPlayingPlaybackState::Playing,
                    PlaybackState::Paused => MPNowPlayingPlaybackState::Paused,
                    PlaybackState::Stopped => MPNowPlayingPlaybackState::Stopped,
                });
            }
            None => {
                center.setNowPlayingInfo(None);
                center.setPlaybackState(MPNowPlayingPlaybackState::Stopped);
            }
        }
    }
    Ok(())
}

/// The title and playback state Now Playing holds, read back off MediaPlayer.
pub fn probe(_mtm: MainThreadMarker) -> (Option<String>, isize) {
    // SAFETY: as in `set`.
    unsafe {
        let center = MPNowPlayingInfoCenter::defaultCenter();
        let title = center
            .nowPlayingInfo()
            .and_then(|info| info.objectForKey(MPMediaItemPropertyTitle))
            .and_then(|v| v.downcast::<NSString>().ok())
            .map(|s| s.to_string());
        (title, center.playbackState().0 as isize)
    }
}
