//! MPRIS as `org.mpris.MediaPlayer2.<app id>`, so media keys, the desktop's media widget and
//! `playerctl` reach the app. `mpris-server`'s player is not `Send`; it lives on one thread
//! that takes updates over a channel. Clearing the session drops the player off the bus.

use std::future::poll_fn;
use std::pin::pin;
use std::sync::{Mutex, OnceLock};
use std::task::Poll;

use futures_channel::mpsc;
use futures_core::Stream;
use mpris_server::{Metadata, PlaybackStatus, Player, Time, TrackId};

use crate::backend::{Emitter, Reply};
use crate::{AppIdentity, Error, MediaCommand, NowPlaying, PlaybackState, Result, SystemEvent};

type Update = (Option<NowPlaying>, Reply<()>);

pub struct Mpris {
    app: AppIdentity,
    events: OnceLock<Emitter>,
    tx: Mutex<Option<mpsc::UnboundedSender<Update>>>,
}

impl Mpris {
    pub fn new(app: &AppIdentity) -> Self {
        Self { app: app.clone(), events: OnceLock::new(), tx: Mutex::new(None) }
    }

    pub fn set_emitter(&self, events: Emitter) {
        let _ = self.events.set(events);
    }

    pub fn set(&self, now: Option<NowPlaying>, reply: Reply<()>) {
        let mut tx = self.tx.lock().expect("mpris");
        let sender = tx.get_or_insert_with(|| {
            let (tx, rx) = mpsc::unbounded();
            let app = self.app.clone();
            let events = self.events.get().cloned().unwrap_or_else(Emitter::detached);
            std::thread::Builder::new()
                .name("nd-mpris".into())
                .spawn(move || futures_executor::block_on(serve(app, events, rx)))
                .expect("mpris thread");
            tx
        });
        if let Err(e) = sender.unbounded_send((now, reply)) {
            e.into_inner().1.err(Error::Platform("MPRIS thread is gone".into()));
        }
    }
}

/// The bus name suffix: the app id with what a D-Bus name element cannot hold replaced.
fn bus_suffix(app: &AppIdentity) -> String {
    app.id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' { c } else { '_' }).collect()
}

type Connect = fn(&Player, Box<dyn Fn(&Player)>);

fn connect(player: &Player, events: &Emitter) {
    let commands: [(MediaCommand, Connect); 6] = [
        (MediaCommand::Play, |p, f| p.connect_play(f)),
        (MediaCommand::Pause, |p, f| p.connect_pause(f)),
        (MediaCommand::Toggle, |p, f| p.connect_play_pause(f)),
        (MediaCommand::Stop, |p, f| p.connect_stop(f)),
        (MediaCommand::Next, |p, f| p.connect_next(f)),
        (MediaCommand::Previous, |p, f| p.connect_previous(f)),
    ];
    for (command, connect) in commands {
        let events = events.clone();
        connect(player, Box::new(move |_| events.emit(SystemEvent::Media(command))));
    }
    let e = events.clone();
    player.connect_seek(move |_, offset| e.emit(SystemEvent::Media(MediaCommand::SeekBy(offset.as_millis()))));
    let e = events.clone();
    player.connect_set_position(move |_, _, at| {
        let at = std::time::Duration::from_micros(at.as_micros().max(0) as u64);
        e.emit(SystemEvent::Media(MediaCommand::SetPosition(at)));
    });
}

fn metadata(now: &NowPlaying) -> Metadata {
    let mut m = Metadata::builder()
        .title(now.title.clone())
        .trackid(TrackId::try_from("/org/nd/track/current").unwrap_or(TrackId::NO_TRACK));
    if let Some(artist) = &now.artist {
        m = m.artist([artist.clone()]);
    }
    if let Some(album) = &now.album {
        m = m.album(album.clone());
    }
    if let Some(d) = now.duration {
        m = m.length(Time::from_micros(d.as_micros() as i64));
    }
    if let Some(art) = &now.artwork_url {
        m = m.art_url(art.clone());
    }
    m.build()
}

async fn apply(player: &Player, now: &NowPlaying) -> mpris_server::zbus::Result<()> {
    player.set_metadata(metadata(now)).await?;
    player
        .set_playback_status(match now.state {
            PlaybackState::Playing => PlaybackStatus::Playing,
            PlaybackState::Paused => PlaybackStatus::Paused,
            PlaybackState::Stopped => PlaybackStatus::Stopped,
        })
        .await?;
    player.set_can_go_next(now.can_next).await?;
    player.set_can_go_previous(now.can_previous).await?;
    player.set_can_seek(now.can_seek).await?;
    if let Some(at) = now.position {
        player.set_position(Time::from_micros(at.as_micros() as i64));
    }
    Ok(())
}

async fn build(app: &AppIdentity, events: &Emitter) -> Result<Player> {
    let player = Player::builder(&bus_suffix(app))
        .identity(app.display_name.clone())
        .desktop_entry(app.id.clone())
        .can_play(true)
        .can_pause(true)
        .can_control(true)
        .build()
        .await
        .map_err(Error::platform)?;
    connect(&player, events);
    Ok(player)
}

/// Owns the player: applies updates as they come and runs its server in between.
async fn serve(app: AppIdentity, events: Emitter, mut rx: mpsc::UnboundedReceiver<Update>) {
    let mut player: Option<Player> = None;
    loop {
        let update = match &player {
            Some(p) => {
                let mut run = pin!(p.run());
                poll_fn(|cx| match std::pin::Pin::new(&mut rx).poll_next(cx) {
                    Poll::Ready(update) => Poll::Ready(update),
                    // The server task never finishes while the player lives.
                    Poll::Pending => run.as_mut().poll(cx).map(|()| None),
                })
                .await
            }
            None => poll_fn(|cx| std::pin::Pin::new(&mut rx).poll_next(cx)).await,
        };
        let Some((now, reply)) = update else { return };
        match now {
            None => {
                player = None;
                reply.ok(());
            }
            Some(now) => {
                if player.is_none() {
                    match build(&app, &events).await {
                        Ok(p) => player = Some(p),
                        Err(e) => {
                            reply.err(e);
                            continue;
                        }
                    }
                }
                let p = player.as_ref().expect("built above");
                reply.send(apply(p, &now).await.map_err(Error::platform));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bus_suffix_is_a_name_element() {
        let app =
            crate::AppIdentity { id: "dev.formalsnake.my-app".into(), name: "my-app".into(), display_name: "x".into() };
        assert_eq!(super::bus_suffix(&app), "dev.formalsnake.my_app");
    }
}
